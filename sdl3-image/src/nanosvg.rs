// Rust translation of src/nanosvg.h from SDL_image (NanoSVG,
// https://github.com/memononen/nanosvg), as IMG_svg.c builds it.
// Copyright (c) 2013-14 Mikko Mononen memon@inside.org
// This is an altered (translated) version of the original software; see LICENSE.txt.
//
// This software is provided 'as-is', without any express or implied
// warranty.  In no event will the authors be held liable for any damages
// arising from the use of this software.
//
// Permission is granted to anyone to use this software for any purpose,
// including commercial applications, and to alter it and redistribute it
// freely, subject to the following restrictions:
//
// 1. The origin of this software must not be misrepresented; you must not
// claim that you wrote the original software. If you use this software
// in a product, an acknowledgment in the product documentation would be
// appreciated but is not required.
// 2. Altered source versions must be plainly marked as such, and must not be
// misrepresented as being the original software.
// 3. This notice may not be removed or altered from any source distribution.
//
// The SVG parser is based on Anti-Grain Geometry 2.4 SVG example
// Copyright (C) 2002-2004 Maxim Shemanarev (McSeem) (http://www.antigrain.com/)
//
// Arc calculation code based on canvg (https://code.google.com/p/canvg/)
//
// Bounding box calculation based on http://blog.hackers-cafe.net/2009/06/how-to-calculate-bezier-curves-bounding.html

//! NanoSVG is a simple stupid single-header-file SVG parse. The output of
//! the parser is a list of cubic bezier shapes.
//!
//! The library suits well for anything from rendering scalable icons in
//! your editor application to prototyping a game.
//!
//! NanoSVG supports a wide range of SVG features, but something may be
//! missing, feel free to create a pull request!
//!
//! The shapes in the SVG images are transformed by the viewBox and
//! converted to specified units. That is, you should get the same looking
//! data as your designed in your favorite app.
//!
//! NanoSVG can return the paths in few different units. For example if you
//! want to render an image, you may choose to get the paths in pixels, or
//! if you are feeding the data into a CNC-cutter, you may want to use
//! millimeters.
//!
//! The units passed to NanoSVG should be one of: 'px', 'pt', 'pc' 'mm',
//! 'cm', or 'in'. DPI (dots-per-inch) controls how the unit conversion is
//! done.
//!
//! If you don't know or care about the units stuff, "px" and 96 should get
//! you going.
//!
//! As IMG_svg.c builds it: the C runtime functions are SDL's (`SDL_sscanf`,
//! `SDL_strtol`, `SDL_sinf`, ... here the `sdl3` crate's), only the basic
//! color keywords are known (no `NANOSVG_ALL_COLOR_KEYWORDS`), and there is
//! no `nsvgParseFromFile()`. Strings are C strings in byte slices: they end
//! at the slice's end or at a NUL, whichever comes first.

// The translation keeps nanosvg's shapes: its index loops over parallel
// arrays, its float comparisons and its long parameter lists.
#![allow(
    clippy::needless_range_loop,
    clippy::too_many_arguments,
    clippy::collapsible_else_if,
    clippy::excessive_precision,
    clippy::if_same_then_else,
    clippy::approx_constant,
    clippy::byte_char_slices,
    clippy::type_complexity,
    clippy::manual_clamp
)]

use sdl3::stdlib::math::{acosf, cosf, fabs, fabsf, pow, roundf, sinf, sqrt, sqrtf, tanf};
use sdl3::stdlib::string::{strtol, strtoll, strtoul};

use crate::util::{c_f32_to_i32, c_f32_to_u32};

pub(crate) const NSVG_PAINT_UNDEF: i8 = -1;
pub(crate) const NSVG_PAINT_NONE: i8 = 0;
pub(crate) const NSVG_PAINT_COLOR: i8 = 1;
pub(crate) const NSVG_PAINT_LINEAR_GRADIENT: i8 = 2;
pub(crate) const NSVG_PAINT_RADIAL_GRADIENT: i8 = 3;

pub(crate) const NSVG_SPREAD_PAD: i8 = 0;
pub(crate) const NSVG_SPREAD_REFLECT: i8 = 1;
pub(crate) const NSVG_SPREAD_REPEAT: i8 = 2;

pub(crate) const NSVG_JOIN_MITER: i8 = 0;
pub(crate) const NSVG_JOIN_ROUND: i8 = 1;
pub(crate) const NSVG_JOIN_BEVEL: i8 = 2;

pub(crate) const NSVG_CAP_BUTT: i8 = 0;
pub(crate) const NSVG_CAP_ROUND: i8 = 1;
pub(crate) const NSVG_CAP_SQUARE: i8 = 2;

pub(crate) const NSVG_FILLRULE_NONZERO: i8 = 0;
pub(crate) const NSVG_FILLRULE_EVENODD: i8 = 1;

pub(crate) const NSVG_FLAGS_VISIBLE: u8 = 0x01;

/// A `char[64]` C string.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Name(pub(crate) [u8; 64]);

impl Default for Name {
    fn default() -> Name {
        Name([0; 64])
    }
}

/// Translation of `NSVGgradientStop`.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct NsvgGradientStop {
    pub(crate) color: u32,
    pub(crate) offset: f32,
}

/// Translation of `NSVGgradient` (`nstops` is the length of `stops`).
#[derive(Clone, Debug, Default)]
pub(crate) struct NsvgGradient {
    pub(crate) xform: [f32; 6],
    pub(crate) spread: i8,
    pub(crate) fx: f32,
    pub(crate) fy: f32,
    pub(crate) stops: Vec<NsvgGradientStop>,
}

/// Translation of `NSVGpaint`: the union's color and gradient side by
/// side.
#[derive(Clone, Debug, Default)]
pub(crate) struct NsvgPaint {
    pub(crate) type_: i8,
    pub(crate) color: u32,
    pub(crate) gradient: Option<Box<NsvgGradient>>,
}

/// Translation of `NSVGpath`.
#[derive(Clone, Debug, Default)]
pub(crate) struct NsvgPath {
    pub(crate) pts: Vec<f32>, // Cubic bezier points: x0,y0, [cpx1,cpx1,cpx2,cpy2,x1,y1], ...
    pub(crate) npts: i32,     // Total number of bezier points.
    pub(crate) closed: bool,  // Flag indicating if shapes should be treated as closed.
    pub(crate) bounds: [f32; 4], // Tight bounding box of the shape [minx,miny,maxx,maxy].
}

/// Translation of `NSVGshape` (`paths` in the order of the C list).
#[derive(Clone, Debug, Default)]
pub(crate) struct NsvgShape {
    #[allow(dead_code)] // (kept as upstream has it)
    pub(crate) id: Name, // Optional 'id' attr of the shape or its group
    pub(crate) fill: NsvgPaint,             // Fill paint
    pub(crate) stroke: NsvgPaint,           // Stroke paint
    pub(crate) opacity: f32,                // Opacity of the shape.
    pub(crate) stroke_width: f32,           // Stroke width (scaled).
    pub(crate) stroke_dash_offset: f32,     // Stroke dash offset (scaled).
    pub(crate) stroke_dash_array: [f32; 8], // Stroke dash array (scaled).
    pub(crate) stroke_dash_count: i8,       // Number of dash values in dash array.
    pub(crate) stroke_line_join: i8,        // Stroke join type.
    pub(crate) stroke_line_cap: i8,         // Stroke cap type.
    pub(crate) miter_limit: f32,            // Miter limit
    pub(crate) fill_rule: i8,               // Fill rule, see NSVGfillRule.
    pub(crate) flags: u8,                   // Logical or of NSVG_FLAGS_* flags
    pub(crate) bounds: [f32; 4], // Tight bounding box of the shape [minx,miny,maxx,maxy].
    pub(crate) fill_gradient: Name, // Optional 'id' of fill gradient
    pub(crate) stroke_gradient: Name, // Optional 'id' of stroke gradient
    pub(crate) xform: [f32; 6],  // Root transformation for fill/stroke gradient
    pub(crate) paths: Vec<NsvgPath>, // Linked list of paths in the image.
}

/// Translation of `NSVGimage`.
#[derive(Clone, Debug, Default)]
pub(crate) struct NsvgImage {
    pub(crate) width: f32,             // Width of the image.
    pub(crate) height: f32,            // Height of the image.
    pub(crate) shapes: Vec<NsvgShape>, // Linked list of shapes in the image.
}

const NSVG_PI: f32 = 3.14159265358979323846264338327;
const NSVG_KAPPA90: f32 = 0.5522847493; // Length proportional to radius of a cubic bezier handle for 90deg arcs.

const NSVG_ALIGN_MIN: i32 = 0;
const NSVG_ALIGN_MID: i32 = 1;
const NSVG_ALIGN_MAX: i32 = 2;
const NSVG_ALIGN_NONE: i32 = 0;
const NSVG_ALIGN_MEET: i32 = 1;
const NSVG_ALIGN_SLICE: i32 = 2;

/// Translation of `NSVG_RGB()`.
fn nsvg_rgb(r: u32, g: u32, b: u32) -> u32 {
    r | (g << 8) | (b << 16)
}

/// The byte at `i` of a C string (NUL past its end).
fn at(s: &[u8], i: usize) -> u8 {
    s.get(i).copied().unwrap_or(0)
}

/// A C string: the slice up to its first NUL.
fn cstr(s: &[u8]) -> &[u8] {
    &s[..s.iter().position(|&b| b == 0).unwrap_or(s.len())]
}

/// `strlcpy(dst, src, size)` (`SDL_strlcpy()`, which nanosvg's `strncpy`
/// is defined as there) into a fixed array.
fn strlcpy(dst: &mut [u8], src: &[u8], size: usize) {
    let src = cstr(src);
    if size > 0 {
        let n = src.len().min(size - 1);
        dst[..n].copy_from_slice(&src[..n]);
        dst[n] = 0;
    }
}

/// Translation of `nsvg__isspace()`: `strchr(" \t\n\v\f\r", c) != NULL`,
/// which finds the terminator for a NUL too.
fn nsvg_isspace(c: u8) -> bool {
    matches!(c, b' ' | b'\t' | b'\n' | 0x0b | 0x0c | b'\r' | 0)
}

/// Translation of `nsvg__isdigit()`.
fn nsvg_isdigit(c: u8) -> bool {
    c.is_ascii_digit()
}

/// Translation of `nsvg__minf()`.
fn nsvg_minf(a: f32, b: f32) -> f32 {
    if a < b {
        a
    } else {
        b
    }
}

/// Translation of `nsvg__maxf()`.
fn nsvg_maxf(a: f32, b: f32) -> f32 {
    if a > b {
        a
    } else {
        b
    }
}

// Simple XML parser

const NSVG_XML_TAG: i32 = 1;
const NSVG_XML_CONTENT: i32 = 2;
const NSVG_XML_MAX_ATTRIBS: usize = 256;

/// Translation of `nsvg__parseContent()`: `s` is the content's offset in
/// the buffer.
fn nsvg_parse_content(buf: &[u8], mut s: usize, p: &mut NsvgParser) {
    // Trim start white spaces
    while buf[s] != 0 && nsvg_isspace(buf[s]) {
        s += 1;
    }
    if buf[s] == 0 {
        return;
    }

    p.content(cstr(&buf[s..]));
}

/// Translation of `nsvg__parseElement()`: `s` is the tag's offset in the
/// buffer, which is cut into the name and attributes with NULs.
fn nsvg_parse_element(buf: &mut [u8], mut s: usize, p: &mut NsvgParser) {
    let mut attr: Vec<(usize, usize)> = Vec::new();
    let mut nattr = 0;
    let mut start = false;
    let mut end = false;

    // Skip white space after the '<'
    while buf[s] != 0 && nsvg_isspace(buf[s]) {
        s += 1;
    }

    // Check if the tag is end tag
    if buf[s] == b'/' {
        s += 1;
        end = true;
    } else {
        start = true;
    }

    // Skip comments, data and preprocessor stuff.
    if buf[s] == 0 || buf[s] == b'?' || buf[s] == b'!' {
        return;
    }

    // Get tag name
    let name = s;
    while buf[s] != 0 && !nsvg_isspace(buf[s]) {
        s += 1;
    }
    if buf[s] != 0 {
        buf[s] = 0;
        s += 1;
    }

    // Get attribs
    while !end && buf[s] != 0 && nattr < NSVG_XML_MAX_ATTRIBS - 3 {
        // Skip white space before the attrib name
        while buf[s] != 0 && nsvg_isspace(buf[s]) {
            s += 1;
        }
        if buf[s] == 0 {
            break;
        }
        if buf[s] == b'/' {
            end = true;
            break;
        }
        let attr_name = s;
        // Find end of the attrib name.
        while buf[s] != 0 && !nsvg_isspace(buf[s]) && buf[s] != b'=' {
            s += 1;
        }
        if buf[s] != 0 {
            buf[s] = 0;
            s += 1;
        }
        // Skip until the beginning of the value.
        while buf[s] != 0 && buf[s] != b'"' && buf[s] != b'\'' {
            s += 1;
        }
        if buf[s] == 0 {
            break;
        }
        let quote = buf[s];
        s += 1;
        // Store value and find the end of it.
        let attr_value = s;
        while buf[s] != 0 && buf[s] != quote {
            s += 1;
        }
        if buf[s] != 0 {
            buf[s] = 0;
            s += 1;
        }

        // Store only well formed attributes
        attr.push((attr_name, attr_value));
        nattr += 2;
    }

    // List terminator
    let buf: &[u8] = buf;
    let name = cstr(&buf[name..]);
    let attr: Vec<(&[u8], &[u8])> = attr
        .iter()
        .map(|&(n, v)| (cstr(&buf[n..]), cstr(&buf[v..])))
        .collect();

    // Call callbacks.
    if start {
        p.start_element(name, &attr);
    }
    if end {
        p.end_element(name);
    }
}

/// Translation of `nsvg__parseXML()`: `input` ends with a NUL.
fn nsvg_parse_xml(input: &mut [u8], p: &mut NsvgParser) {
    let mut s = 0;
    let mut mark = s;
    let mut state = NSVG_XML_CONTENT;
    while input[s] != 0 {
        if input[s] == b'<' && state == NSVG_XML_CONTENT {
            // Start of a tag
            input[s] = 0;
            s += 1;
            nsvg_parse_content(input, mark, p);
            mark = s;
            state = NSVG_XML_TAG;
        } else if input[s] == b'>' && state == NSVG_XML_TAG {
            // Start of a content or new tag.
            input[s] = 0;
            s += 1;
            nsvg_parse_element(input, mark, p);
            mark = s;
            state = NSVG_XML_CONTENT;
        } else {
            s += 1;
        }
    }
}

/* Simple SVG parser. */

const NSVG_MAX_ATTR: usize = 128;

// NSVGgradientUnits
const NSVG_USER_SPACE: i8 = 0;
const NSVG_OBJECT_SPACE: i8 = 1;

const NSVG_MAX_DASHES: usize = 8;
const NSVG_MAX_CLASSES: usize = 32;

// NSVGunits
const NSVG_UNITS_USER: i32 = 0;
const NSVG_UNITS_PX: i32 = 1;
const NSVG_UNITS_PT: i32 = 2;
const NSVG_UNITS_PC: i32 = 3;
const NSVG_UNITS_MM: i32 = 4;
const NSVG_UNITS_CM: i32 = 5;
const NSVG_UNITS_IN: i32 = 6;
const NSVG_UNITS_PERCENT: i32 = 7;
const NSVG_UNITS_EM: i32 = 8;
const NSVG_UNITS_EX: i32 = 9;

/// Translation of `NSVGcoordinate`.
#[derive(Clone, Copy, Debug, Default)]
struct NsvgCoordinate {
    value: f32,
    units: i32,
}

/// Translation of `NSVGgradientData`. The union of `NSVGlinearData` and
/// `NSVGradialData` is the five coordinates they share: `x1`/`cx`,
/// `y1`/`cy`, `x2`/`r`, `y2`/`fx` and `fy`.
#[derive(Clone, Debug, Default)]
struct NsvgGradientData {
    id: Name,
    ref_: Name,
    type_: i8,
    coords: [NsvgCoordinate; 5],
    spread: i8,
    units: i8,
    xform: [f32; 6],
    stops: Vec<NsvgGradientStop>,
}

// (the union's members)
const X1: usize = 0;
const Y1: usize = 1;
const X2: usize = 2;
const Y2: usize = 3;
const CX: usize = 0;
const CY: usize = 1;
const R: usize = 2;
const FX: usize = 3;
const FY: usize = 4;

/// Translation of `NSVGattrib`.
#[derive(Clone, Debug, Default)]
struct NsvgAttrib {
    id: Name,
    xform: [f32; 6],
    fill_color: u32,
    stroke_color: u32,
    opacity: f32,
    fill_opacity: f32,
    stroke_opacity: f32,
    fill_gradient: Name,
    stroke_gradient: Name,
    stroke_width: f32,
    stroke_dash_offset: f32,
    stroke_dash_array: [f32; NSVG_MAX_DASHES],
    stroke_dash_count: i32,
    stroke_line_join: i8,
    stroke_line_cap: i8,
    miter_limit: f32,
    fill_rule: i8,
    font_size: f32,
    stop_color: u32,
    stop_opacity: f32,
    stop_offset: f32,
    has_fill: i8,
    has_stroke: i8,
    visible: bool,
}

/// Translation of `NSVGstyleDeclaration`.
#[derive(Clone, Debug)]
struct NsvgStyleDeclaration {
    class_name: Vec<u8>,
    properties_text: Vec<u8>,
}

/// Translation of `NSVGparser`. Its lists are vectors in the order the
/// elements were made: the C code puts paths, gradients and styles at the
/// head of theirs, so those are walked backwards.
struct NsvgParser {
    attr: Vec<NsvgAttrib>,
    attr_head: usize,
    pts: Vec<f32>,
    npts: i32,
    plist: Vec<NsvgPath>,
    image: NsvgImage,
    styles: Vec<NsvgStyleDeclaration>,
    gradients: Vec<NsvgGradientData>,
    view_minx: f32,
    view_miny: f32,
    view_width: f32,
    view_height: f32,
    align_x: i32,
    align_y: i32,
    align_type: i32,
    dpi: f32,
    path_flag: bool,
    defs_flag: bool,
    style_flag: bool,
}

/// Translation of `nsvg__xformIdentity()`.
fn nsvg_xform_identity(t: &mut [f32; 6]) {
    *t = [1.0, 0.0, 0.0, 1.0, 0.0, 0.0];
}

/// Translation of `nsvg__xformSetTranslation()`.
fn nsvg_xform_set_translation(t: &mut [f32; 6], tx: f32, ty: f32) {
    *t = [1.0, 0.0, 0.0, 1.0, tx, ty];
}

/// Translation of `nsvg__xformSetScale()`.
fn nsvg_xform_set_scale(t: &mut [f32; 6], sx: f32, sy: f32) {
    *t = [sx, 0.0, 0.0, sy, 0.0, 0.0];
}

/// Translation of `nsvg__xformSetSkewX()`.
fn nsvg_xform_set_skew_x(t: &mut [f32; 6], a: f32) {
    *t = [1.0, 0.0, tanf(a), 1.0, 0.0, 0.0];
}

/// Translation of `nsvg__xformSetSkewY()`.
fn nsvg_xform_set_skew_y(t: &mut [f32; 6], a: f32) {
    *t = [1.0, tanf(a), 0.0, 1.0, 0.0, 0.0];
}

/// Translation of `nsvg__xformSetRotation()`.
fn nsvg_xform_set_rotation(t: &mut [f32; 6], a: f32) {
    let cs = cosf(a);
    let sn = sinf(a);
    *t = [cs, sn, -sn, cs, 0.0, 0.0];
}

/// Translation of `nsvg__xformMultiply()`.
fn nsvg_xform_multiply(t: &mut [f32; 6], s: &[f32; 6]) {
    let t0 = t[0] * s[0] + t[1] * s[2];
    let t2 = t[2] * s[0] + t[3] * s[2];
    let t4 = t[4] * s[0] + t[5] * s[2] + s[4];
    t[1] = t[0] * s[1] + t[1] * s[3];
    t[3] = t[2] * s[1] + t[3] * s[3];
    t[5] = t[4] * s[1] + t[5] * s[3] + s[5];
    t[0] = t0;
    t[2] = t2;
    t[4] = t4;
}

/// Translation of `nsvg__xformInverse()`.
fn nsvg_xform_inverse(inv: &mut [f32; 6], t: &mut [f32; 6]) {
    let det = t[0] as f64 * t[3] as f64 - t[2] as f64 * t[1] as f64;
    if det > -1e-6 && det < 1e-6 {
        // FIXME (upstream): for a singular matrix the source is reset to
        // the identity and the inverse left as it was.
        nsvg_xform_identity(t);
        return;
    }
    let invdet = 1.0 / det;
    inv[0] = (t[3] as f64 * invdet) as f32;
    inv[2] = (-t[2] as f64 * invdet) as f32;
    inv[4] = ((t[2] as f64 * t[5] as f64 - t[3] as f64 * t[4] as f64) * invdet) as f32;
    inv[1] = (-t[1] as f64 * invdet) as f32;
    inv[3] = (t[0] as f64 * invdet) as f32;
    inv[5] = ((t[1] as f64 * t[4] as f64 - t[0] as f64 * t[5] as f64) * invdet) as f32;
}

/// Translation of `nsvg__xformPremultiply()`.
fn nsvg_xform_premultiply(t: &mut [f32; 6], s: &[f32; 6]) {
    let mut s2 = *s;
    nsvg_xform_multiply(&mut s2, t);
    *t = s2;
}

/// Translation of `nsvg__xformPoint()`.
fn nsvg_xform_point(x: f32, y: f32, t: &[f32; 6]) -> (f32, f32) {
    (x * t[0] + y * t[2] + t[4], x * t[1] + y * t[3] + t[5])
}

/// Translation of `nsvg__xformVec()`.
fn nsvg_xform_vec(x: f32, y: f32, t: &[f32; 6]) -> (f32, f32) {
    (x * t[0] + y * t[2], x * t[1] + y * t[3])
}

const NSVG_EPSILON: f64 = 1e-12;

/// Translation of `nsvg__ptInBounds()`.
fn nsvg_pt_in_bounds(pt: &[f32], bounds: &[f32; 4]) -> bool {
    pt[0] >= bounds[0] && pt[0] <= bounds[2] && pt[1] >= bounds[1] && pt[1] <= bounds[3]
}

/// Translation of `nsvg__evalBezier()`.
fn nsvg_eval_bezier(t: f64, p0: f64, p1: f64, p2: f64, p3: f64) -> f64 {
    let it = 1.0 - t;
    it * it * it * p0 + 3.0 * it * it * t * p1 + 3.0 * it * t * t * p2 + t * t * t * p3
}

/// Translation of `nsvg__curveBounds()`: `curve` is 4 points.
fn nsvg_curve_bounds(bounds: &mut [f32; 4], curve: &[f32]) {
    let v0 = &curve[0..2];
    let v1 = &curve[2..4];
    let v2 = &curve[4..6];
    let v3 = &curve[6..8];

    // Start the bounding box by end points
    bounds[0] = nsvg_minf(v0[0], v3[0]);
    bounds[1] = nsvg_minf(v0[1], v3[1]);
    bounds[2] = nsvg_maxf(v0[0], v3[0]);
    bounds[3] = nsvg_maxf(v0[1], v3[1]);

    // Bezier curve fits inside the convex hull of it's control points.
    // If control points are inside the bounds, we're done.
    if nsvg_pt_in_bounds(v1, bounds) && nsvg_pt_in_bounds(v2, bounds) {
        return;
    }

    // Add bezier curve inflection points in X and Y.
    for i in 0..2 {
        let mut roots = [0.0f64; 2];
        let a = -3.0 * v0[i] as f64 + 9.0 * v1[i] as f64 - 9.0 * v2[i] as f64 + 3.0 * v3[i] as f64;
        let b = 6.0 * v0[i] as f64 - 12.0 * v1[i] as f64 + 6.0 * v2[i] as f64;
        let c = 3.0 * v1[i] as f64 - 3.0 * v0[i] as f64;
        let mut count = 0;
        if fabs(a) < NSVG_EPSILON {
            if fabs(b) > NSVG_EPSILON {
                let t = -c / b;
                if t > NSVG_EPSILON && t < 1.0 - NSVG_EPSILON {
                    roots[count] = t;
                    count += 1;
                }
            }
        } else {
            let b2ac = b * b - 4.0 * c * a;
            if b2ac > NSVG_EPSILON {
                let t = (-b + sqrt(b2ac)) / (2.0 * a);
                if t > NSVG_EPSILON && t < 1.0 - NSVG_EPSILON {
                    roots[count] = t;
                    count += 1;
                }
                let t = (-b - sqrt(b2ac)) / (2.0 * a);
                if t > NSVG_EPSILON && t < 1.0 - NSVG_EPSILON {
                    roots[count] = t;
                    count += 1;
                }
            }
        }
        for j in 0..count {
            let v = nsvg_eval_bezier(
                roots[j],
                v0[i] as f64,
                v1[i] as f64,
                v2[i] as f64,
                v3[i] as f64,
            );
            bounds[i] = nsvg_minf(bounds[i], v as f32);
            bounds[2 + i] = nsvg_maxf(bounds[2 + i], v as f32);
        }
    }
}

impl NsvgParser {
    /// Translation of `nsvg__createParser()`.
    fn new() -> NsvgParser {
        let mut attr = vec![NsvgAttrib::default(); NSVG_MAX_ATTR];

        // Init style
        let a = &mut attr[0];
        nsvg_xform_identity(&mut a.xform);
        a.id = Name::default();
        a.fill_color = nsvg_rgb(0, 0, 0);
        a.stroke_color = nsvg_rgb(0, 0, 0);
        a.opacity = 1.0;
        a.fill_opacity = 1.0;
        a.stroke_opacity = 1.0;
        a.stop_opacity = 1.0;
        a.stroke_width = 1.0;
        a.stroke_line_join = NSVG_JOIN_MITER;
        a.stroke_line_cap = NSVG_CAP_BUTT;
        a.miter_limit = 4.0;
        a.fill_rule = NSVG_FILLRULE_NONZERO;
        a.has_fill = 1;
        a.visible = true;

        NsvgParser {
            attr,
            attr_head: 0,
            pts: Vec::new(),
            npts: 0,
            plist: Vec::new(),
            image: NsvgImage::default(),
            styles: Vec::new(),
            gradients: Vec::new(),
            view_minx: 0.0,
            view_miny: 0.0,
            view_width: 0.0,
            view_height: 0.0,
            align_x: 0,
            align_y: 0,
            align_type: 0,
            dpi: 0.0,
            path_flag: false,
            defs_flag: false,
            style_flag: false,
        }
    }

    /// Translation of `nsvg__resetPath()`.
    fn reset_path(&mut self) {
        self.npts = 0;
        self.pts.clear();
    }

    /// Translation of `nsvg__addPoint()`.
    fn add_point(&mut self, x: f32, y: f32) {
        self.pts.push(x);
        self.pts.push(y);
        self.npts += 1;
    }

    /// Translation of `nsvg__moveTo()`.
    fn move_to(&mut self, x: f32, y: f32) {
        if self.npts > 0 {
            let n = self.npts as usize;
            self.pts[(n - 1) * 2] = x;
            self.pts[(n - 1) * 2 + 1] = y;
        } else {
            self.add_point(x, y);
        }
    }

    /// Translation of `nsvg__lineTo()`.
    fn line_to(&mut self, x: f32, y: f32) {
        if self.npts > 0 {
            let n = self.npts as usize;
            let px = self.pts[(n - 1) * 2];
            let py = self.pts[(n - 1) * 2 + 1];
            let dx = x - px;
            let dy = y - py;
            self.add_point(px + dx / 3.0, py + dy / 3.0);
            self.add_point(x - dx / 3.0, y - dy / 3.0);
            self.add_point(x, y);
        }
    }

    /// Translation of `nsvg__cubicBezTo()`.
    fn cubic_bez_to(&mut self, cpx1: f32, cpy1: f32, cpx2: f32, cpy2: f32, x: f32, y: f32) {
        if self.npts > 0 {
            self.add_point(cpx1, cpy1);
            self.add_point(cpx2, cpy2);
            self.add_point(x, y);
        }
    }

    /// Translation of `nsvg__getAttr()`.
    fn get_attr(&mut self) -> &mut NsvgAttrib {
        &mut self.attr[self.attr_head]
    }

    /// Translation of `nsvg__pushAttr()`.
    fn push_attr(&mut self) {
        if self.attr_head < NSVG_MAX_ATTR - 1 {
            self.attr_head += 1;
            self.attr[self.attr_head] = self.attr[self.attr_head - 1].clone();
        }
    }

    /// Translation of `nsvg__popAttr()`.
    fn pop_attr(&mut self) {
        if self.attr_head > 0 {
            self.attr_head -= 1;
        }
    }

    /// Translation of `nsvg__actualOrigX()`.
    fn actual_orig_x(&self) -> f32 {
        self.view_minx
    }

    /// Translation of `nsvg__actualOrigY()`.
    fn actual_orig_y(&self) -> f32 {
        self.view_miny
    }

    /// Translation of `nsvg__actualWidth()`.
    fn actual_width(&self) -> f32 {
        self.view_width
    }

    /// Translation of `nsvg__actualHeight()`.
    fn actual_height(&self) -> f32 {
        self.view_height
    }

    /// Translation of `nsvg__actualLength()`.
    fn actual_length(&self) -> f32 {
        let w = self.actual_width();
        let h = self.actual_height();
        sqrtf(w * w + h * h) / sqrtf(2.0)
    }

    /// Translation of `nsvg__convertToPixels()`.
    fn convert_to_pixels(&self, c: NsvgCoordinate, orig: f32, length: f32) -> f32 {
        let attr = &self.attr[self.attr_head];
        match c.units {
            NSVG_UNITS_USER => c.value,
            NSVG_UNITS_PX => c.value,
            NSVG_UNITS_PT => c.value / 72.0 * self.dpi,
            NSVG_UNITS_PC => c.value / 6.0 * self.dpi,
            NSVG_UNITS_MM => c.value / 25.4 * self.dpi,
            NSVG_UNITS_CM => c.value / 2.54 * self.dpi,
            NSVG_UNITS_IN => c.value * self.dpi,
            NSVG_UNITS_EM => c.value * attr.font_size,
            NSVG_UNITS_EX => c.value * attr.font_size * 0.52, // x-height of Helvetica.
            NSVG_UNITS_PERCENT => orig + c.value / 100.0 * length,
            _ => c.value,
        }
        /*return c.value; UNREACHABLE CODE */
    }

    /// Translation of `nsvg__findGradientData()`: the index of the newest
    /// gradient with the id.
    fn find_gradient_data(&self, id: &[u8]) -> Option<usize> {
        let id = cstr(id);
        if id.is_empty() {
            return None;
        }
        (0..self.gradients.len())
            .rev()
            .find(|&i| cstr(&self.gradients[i].id.0) == id)
    }

    /// Translation of `nsvg__createGradient()`.
    fn create_gradient(
        &self,
        id: &[u8],
        local_bounds: &[f32; 4],
        xform: &[f32; 6],
        paint_type: &mut i8,
    ) -> Option<Box<NsvgGradient>> {
        let data = self.find_gradient_data(id)?;

        // TODO: use ref to fill in all unset values too.
        let mut stops: Option<&Vec<NsvgGradientStop>> = None;
        let mut ref_ = Some(data);
        let mut ref_iter = 0;
        while let Some(r) = ref_ {
            let rd = &self.gradients[r];
            if stops.is_none() && !rd.stops.is_empty() {
                stops = Some(&rd.stops);
                break;
            }
            let next_ref = self.find_gradient_data(&rd.ref_.0);
            if next_ref == Some(r) {
                break; // prevent infite loops on malformed data
            }
            ref_ = next_ref;
            ref_iter += 1;
            if ref_iter > 32 {
                break; // prevent infite loops on malformed data
            }
        }
        let stops = stops?;

        let data = &self.gradients[data];
        let mut grad = Box::new(NsvgGradient::default());

        // The shape width and height.
        let (ox, oy, sw, sh);
        if data.units == NSVG_OBJECT_SPACE {
            ox = local_bounds[0];
            oy = local_bounds[1];
            sw = local_bounds[2] - local_bounds[0];
            sh = local_bounds[3] - local_bounds[1];
        } else {
            ox = self.actual_orig_x();
            oy = self.actual_orig_y();
            sw = self.actual_width();
            sh = self.actual_height();
        }
        let sl = sqrtf(sw * sw + sh * sh) / sqrtf(2.0);

        if data.type_ == NSVG_PAINT_LINEAR_GRADIENT {
            let x1 = self.convert_to_pixels(data.coords[X1], ox, sw);
            let y1 = self.convert_to_pixels(data.coords[Y1], oy, sh);
            let x2 = self.convert_to_pixels(data.coords[X2], ox, sw);
            let y2 = self.convert_to_pixels(data.coords[Y2], oy, sh);
            // Calculate transform aligned to the line
            let dx = x2 - x1;
            let dy = y2 - y1;
            grad.xform = [dy, -dx, dx, dy, x1, y1];
        } else {
            let cx = self.convert_to_pixels(data.coords[CX], ox, sw);
            let cy = self.convert_to_pixels(data.coords[CY], oy, sh);
            let fx = self.convert_to_pixels(data.coords[FX], ox, sw);
            let fy = self.convert_to_pixels(data.coords[FY], oy, sh);
            let r = self.convert_to_pixels(data.coords[R], 0.0, sl);
            // Calculate transform aligned to the circle
            grad.xform = [r, 0.0, 0.0, r, cx, cy];
            grad.fx = fx / r;
            grad.fy = fy / r;
        }

        nsvg_xform_multiply(&mut grad.xform, &data.xform);
        nsvg_xform_multiply(&mut grad.xform, xform);

        grad.spread = data.spread;
        grad.stops = stops.clone();

        *paint_type = data.type_;

        Some(grad)
    }
}

/// Translation of `nsvg__getAverageScale()`.
fn nsvg_get_average_scale(t: &[f32; 6]) -> f32 {
    let sx = sqrtf(t[0] * t[0] + t[2] * t[2]);
    let sy = sqrtf(t[1] * t[1] + t[3] * t[3]);
    (sx + sy) * 0.5
}

/// Translation of `nsvg__getLocalBounds()`.
fn nsvg_get_local_bounds(bounds: &mut [f32; 4], shape: &NsvgShape, xform: &[f32; 6]) {
    let mut curve = [0.0f32; 4 * 2];
    let mut curve_bounds = [0.0f32; 4];
    let mut first = true;
    for path in &shape.paths {
        (curve[0], curve[1]) = nsvg_xform_point(path.pts[0], path.pts[1], xform);
        let mut i = 0;
        while i < path.npts - 1 {
            let i_ = i as usize;
            (curve[2], curve[3]) =
                nsvg_xform_point(path.pts[(i_ + 1) * 2], path.pts[(i_ + 1) * 2 + 1], xform);
            (curve[4], curve[5]) =
                nsvg_xform_point(path.pts[(i_ + 2) * 2], path.pts[(i_ + 2) * 2 + 1], xform);
            (curve[6], curve[7]) =
                nsvg_xform_point(path.pts[(i_ + 3) * 2], path.pts[(i_ + 3) * 2 + 1], xform);
            nsvg_curve_bounds(&mut curve_bounds, &curve);
            if first {
                *bounds = curve_bounds;
                first = false;
            } else {
                bounds[0] = nsvg_minf(bounds[0], curve_bounds[0]);
                bounds[1] = nsvg_minf(bounds[1], curve_bounds[1]);
                bounds[2] = nsvg_maxf(bounds[2], curve_bounds[2]);
                bounds[3] = nsvg_maxf(bounds[3], curve_bounds[3]);
            }
            curve[0] = curve[6];
            curve[1] = curve[7];
            i += 3;
        }
    }
}

impl NsvgParser {
    /// Translation of `nsvg__addShape()`.
    fn add_shape(&mut self) {
        if self.plist.is_empty() {
            return;
        }

        let attr = self.attr[self.attr_head].clone();
        let mut shape = NsvgShape {
            id: attr.id,
            fill_gradient: attr.fill_gradient,
            stroke_gradient: attr.stroke_gradient,
            xform: attr.xform,
            ..NsvgShape::default()
        };
        let scale = nsvg_get_average_scale(&attr.xform);
        shape.stroke_width = attr.stroke_width * scale;
        shape.stroke_dash_offset = attr.stroke_dash_offset * scale;
        shape.stroke_dash_count = attr.stroke_dash_count as i8;
        for i in 0..attr.stroke_dash_count as usize {
            shape.stroke_dash_array[i] = attr.stroke_dash_array[i] * scale;
        }
        shape.stroke_line_join = attr.stroke_line_join;
        shape.stroke_line_cap = attr.stroke_line_cap;
        shape.miter_limit = attr.miter_limit;
        shape.fill_rule = attr.fill_rule;
        shape.opacity = attr.opacity;

        // (the C list has the newest path first)
        shape.paths = std::mem::take(&mut self.plist);
        shape.paths.reverse();

        // Calculate shape bounds
        shape.bounds = shape.paths[0].bounds;
        for path in &shape.paths[1..] {
            shape.bounds[0] = nsvg_minf(shape.bounds[0], path.bounds[0]);
            shape.bounds[1] = nsvg_minf(shape.bounds[1], path.bounds[1]);
            shape.bounds[2] = nsvg_maxf(shape.bounds[2], path.bounds[2]);
            shape.bounds[3] = nsvg_maxf(shape.bounds[3], path.bounds[3]);
        }

        // Set fill
        if attr.has_fill == 0 {
            shape.fill.type_ = NSVG_PAINT_NONE;
        } else if attr.has_fill == 1 {
            shape.fill.type_ = NSVG_PAINT_COLOR;
            shape.fill.color = attr.fill_color;
            shape.fill.color |= c_f32_to_u32(attr.fill_opacity * 255.0) << 24;
        } else if attr.has_fill == 2 {
            shape.fill.type_ = NSVG_PAINT_UNDEF;
        }

        // Set stroke
        if attr.has_stroke == 0 {
            shape.stroke.type_ = NSVG_PAINT_NONE;
        } else if attr.has_stroke == 1 {
            shape.stroke.type_ = NSVG_PAINT_COLOR;
            shape.stroke.color = attr.stroke_color;
            shape.stroke.color |= c_f32_to_u32(attr.stroke_opacity * 255.0) << 24;
        } else if attr.has_stroke == 2 {
            shape.stroke.type_ = NSVG_PAINT_UNDEF;
        }

        // Set flags
        shape.flags = if attr.visible {
            NSVG_FLAGS_VISIBLE
        } else {
            0x00
        };

        // Add to tail
        self.image.shapes.push(shape);
    }

    /// Translation of `nsvg__addPath()`.
    fn add_path(&mut self, closed: bool) {
        let mut bounds = [0.0f32; 4];

        if self.npts < 4 {
            return;
        }

        if closed {
            let (x, y) = (self.pts[0], self.pts[1]);
            self.line_to(x, y);
        }

        // Expect 1 + N*3 points (N = number of cubic bezier segments).
        if (self.npts % 3) != 1 {
            return;
        }

        let xform = self.attr[self.attr_head].xform;
        let mut path = NsvgPath {
            pts: vec![0.0; self.npts as usize * 2],
            closed,
            npts: self.npts,
            bounds: [0.0; 4],
        };

        // Transform path.
        for i in 0..self.npts as usize {
            (path.pts[i * 2], path.pts[i * 2 + 1]) =
                nsvg_xform_point(self.pts[i * 2], self.pts[i * 2 + 1], &xform);
        }

        // Find bounds
        let mut i = 0;
        while i < path.npts - 1 {
            let curve = &path.pts[i as usize * 2..];
            nsvg_curve_bounds(&mut bounds, curve);
            if i == 0 {
                path.bounds = bounds;
            } else {
                path.bounds[0] = nsvg_minf(path.bounds[0], bounds[0]);
                path.bounds[1] = nsvg_minf(path.bounds[1], bounds[1]);
                path.bounds[2] = nsvg_maxf(path.bounds[2], bounds[2]);
                path.bounds[3] = nsvg_maxf(path.bounds[3], bounds[3]);
            }
            i += 3;
        }

        self.plist.push(path);
    }
}

// We roll our own string to float because the std library one uses locale and messes things up.
/// Translation of `nsvg__atof()`.
fn nsvg_atof(s: &[u8]) -> f32 {
    let s = cstr(s);
    let mut cur = 0usize;
    let mut res = 0.0f64;
    let mut sign = 1.0f64;
    let mut has_int_part = false;
    let mut has_frac_part = false;

    // Parse optional sign
    if at(s, cur) == b'+' {
        cur += 1;
    } else if at(s, cur) == b'-' {
        sign = -1.0;
        cur += 1;
    }

    // Parse integer part
    if nsvg_isdigit(at(s, cur)) {
        // Parse digit sequence
        let (int_part, len) = strtoll(&s[cur..], 10);
        if len != 0 {
            res = int_part as f64;
            has_int_part = true;
            cur += len;
        }
    }

    // Parse fractional part.
    if at(s, cur) == b'.' {
        cur += 1; // Skip '.'
        if nsvg_isdigit(at(s, cur)) {
            // Parse digit sequence
            let (frac_part, len) = strtoll(&s[cur..], 10);
            if len != 0 {
                res += frac_part as f64 / pow(10.0, len as f64);
                has_frac_part = true;
                cur += len;
            }
        }
    }

    // A valid number should have integer or fractional part.
    if !has_int_part && !has_frac_part {
        return 0.0;
    }

    // Parse optional exponent
    if at(s, cur) == b'e' || at(s, cur) == b'E' {
        cur += 1; // skip 'E'
        let (exp_part, len) = strtol(s.get(cur..).unwrap_or(&[]), 10); // Parse digit sequence with sign
        if len != 0 {
            res *= pow(10.0, exp_part as f64);
        }
    }

    (res * sign) as f32
}

/// Translation of `nsvg__parseNumber()`: the number's characters (at most
/// `size - 1`) go to `it`; returns the rest of `s`.
fn nsvg_parse_number<'a>(s: &'a [u8], it: &mut Vec<u8>, size: usize) -> &'a [u8] {
    let last = size - 1;
    let mut s = cstr(s);
    it.clear();
    let put = |it: &mut Vec<u8>, c: u8| {
        if it.len() < last {
            it.push(c);
        }
    };

    // sign
    if at(s, 0) == b'-' || at(s, 0) == b'+' {
        put(it, s[0]);
        s = &s[1..];
    }
    // integer part
    while nsvg_isdigit(at(s, 0)) {
        put(it, s[0]);
        s = &s[1..];
    }
    if at(s, 0) == b'.' {
        // decimal point
        put(it, s[0]);
        s = &s[1..];
        // fraction part
        while nsvg_isdigit(at(s, 0)) {
            put(it, s[0]);
            s = &s[1..];
        }
    }
    // exponent
    if (at(s, 0) == b'e' || at(s, 0) == b'E') && (at(s, 1) != b'm' && at(s, 1) != b'x') {
        put(it, s[0]);
        s = &s[1..];
        if at(s, 0) == b'-' || at(s, 0) == b'+' {
            put(it, s[0]);
            s = &s[1..];
        }
        while nsvg_isdigit(at(s, 0)) {
            put(it, s[0]);
            s = &s[1..];
        }
    }

    s
}

/// Translation of `nsvg__getNextPathItemWhenArcFlag()`.
fn nsvg_get_next_path_item_when_arc_flag<'a>(s: &'a [u8], it: &mut Vec<u8>) -> &'a [u8] {
    let mut s = cstr(s);
    it.clear();
    while !s.is_empty() && (nsvg_isspace(s[0]) || s[0] == b',') {
        s = &s[1..];
    }
    if s.is_empty() {
        return s;
    }
    if s[0] == b'0' || s[0] == b'1' {
        it.push(s[0]);
        return &s[1..];
    }
    s
}

/// Translation of `nsvg__getNextPathItem()`.
fn nsvg_get_next_path_item<'a>(s: &'a [u8], it: &mut Vec<u8>) -> &'a [u8] {
    let mut s = cstr(s);
    it.clear();
    // Skip white spaces and commas
    while !s.is_empty() && (nsvg_isspace(s[0]) || s[0] == b',') {
        s = &s[1..];
    }
    if s.is_empty() {
        return s;
    }
    if s[0] == b'-' || s[0] == b'+' || s[0] == b'.' || nsvg_isdigit(s[0]) {
        s = nsvg_parse_number(s, it, 64);
    } else {
        // Parse command
        it.push(s[0]);
        return &s[1..];
    }

    s
}

/// glibc's `scanf` conversion of an integer of at most `width` characters
/// (0 for no limit) in `base` 10 or 16, after white space: an optional
/// sign, then for base 16 an optional `0x`, then digits. Returns the value
/// (as `strtoul` makes it) and the rest of the input, or `None` for a
/// matching failure.
fn scan_integer(s: &[u8], width: usize, base: u32) -> Option<(u64, &[u8])> {
    let mut i = 0;
    while crate::util::isspace(at(s, i)) {
        i += 1;
    }
    let mut width = if width == 0 { usize::MAX } else { width };
    let mut text: Vec<u8> = Vec::new();
    let mut c = at(s, i);
    if width != 0 && (c == b'-' || c == b'+') {
        text.push(c);
        width -= 1;
        i += 1;
        c = at(s, i);
    }
    let mut digits = false;
    if base == 16 && width != 0 && c == b'0' {
        width -= 1;
        text.push(b'0');
        digits = true;
        i += 1;
        c = at(s, i);
        if width != 0 && (c == b'x' || c == b'X') {
            width -= 1;
            i += 1;
            c = at(s, i);
        }
    }
    while width != 0 && c != 0 && (c as char).is_digit(base) {
        text.push(c);
        digits = true;
        width -= 1;
        i += 1;
        c = at(s, i);
    }
    if !digits {
        return None;
    }
    Some((strtoul(&text, base).0, s.get(i..).unwrap_or(&[])))
}

/// Translation of `nsvg__parseColorHex()` (its `sscanf()`s being glibc's).
fn nsvg_parse_color_hex(s: &[u8]) -> u32 {
    let scan3 = |width: usize| -> Option<[u32; 3]> {
        let mut rest = s.strip_prefix(b"#")?;
        let mut v = [0u32; 3];
        for x in &mut v {
            let (value, r) = scan_integer(rest, width, 16)?;
            *x = value as u32;
            rest = r;
        }
        Some(v)
    };
    if let Some([r, g, b]) = scan3(2) {
        // 2 digit hex
        return nsvg_rgb(r, g, b);
    }
    if let Some([r, g, b]) = scan3(1) {
        // 1 digit hex, e.g. #abc -> 0xccbbaa
        return nsvg_rgb(r.wrapping_mul(17), g.wrapping_mul(17), b.wrapping_mul(17));
        // same effect as (r<<4|r), (g<<4|g), ..
    }
    nsvg_rgb(128, 128, 128)
}

/// `sscanf(str, "rgb(%u, %u, %u)", ...)` (glibc's): the three numbers, or
/// `None` when fewer convert.
fn scan_rgb_integers(s: &[u8]) -> Option<[u32; 3]> {
    let mut rest = s.strip_prefix(b"rgb(")?;
    let mut v = [0u32; 3];
    for (i, x) in v.iter_mut().enumerate() {
        if i > 0 {
            // ", ": a comma, then any white space
            rest = rest.strip_prefix(b",")?;
        }
        let (value, r) = scan_integer(rest, 0, 10)?;
        *x = value as u32;
        rest = r;
    }
    Some(v)
}

// Parse rgb color. The pointer 'str' must point at "rgb(" (4+ characters).
// This function returns gray (rgb(128, 128, 128) == '#808080') on parse errors
// for backwards compatibility. Note: other image viewers return black instead.

/// Translation of `nsvg__parseColorRGB()`.
fn nsvg_parse_color_rgb(s: &[u8]) -> u32 {
    let mut rgbi = [0u32; 3];
    let mut rgbf = [0.0f32; 3];
    // try decimal integers first
    if let Some(v) = scan_rgb_integers(s) {
        rgbi = v;
    } else {
        // integers failed, try percent values (float, locale independent)
        let delimiter = [b',', b',', b')'];
        let mut str = &s[4..]; // skip "rgb("
        let mut i = 0;
        while i < 3 {
            while !str.is_empty() && nsvg_isspace(str[0]) {
                str = &str[1..]; // skip leading spaces
            }
            if at(str, 0) == b'+' {
                str = &str[1..]; // skip '+' (don't allow '-')
            }
            if str.is_empty() {
                break;
            }
            rgbf[i] = nsvg_atof(str);

            // Note 1: it would be great if nsvg__atof() returned how many
            // bytes it consumed but it doesn't. We need to skip the number,
            // the '%' character, spaces, and the delimiter ',' or ')'.

            // Note 2: The following code does not allow values like "33.%",
            // i.e. a decimal point w/o fractional part, but this is consistent
            // with other image viewers, e.g. firefox, chrome, eog, gimp.

            while nsvg_isdigit(at(str, 0)) {
                str = &str[1..]; // skip integer part
            }
            if at(str, 0) == b'.' {
                str = &str[1..];
                if !nsvg_isdigit(at(str, 0)) {
                    break; // error: no digit after '.'
                }
                while nsvg_isdigit(at(str, 0)) {
                    str = &str[1..]; // skip fractional part
                }
            }
            if at(str, 0) == b'%' {
                str = &str[1..];
            } else {
                break;
            }
            while !str.is_empty() && nsvg_isspace(str[0]) {
                str = &str[1..];
            }
            if at(str, 0) == delimiter[i] {
                str = &str[1..];
            } else {
                break;
            }
            i += 1;
        }
        if i == 3 {
            rgbi[0] = c_f32_to_u32(roundf(rgbf[0] * 2.55));
            rgbi[1] = c_f32_to_u32(roundf(rgbf[1] * 2.55));
            rgbi[2] = c_f32_to_u32(roundf(rgbf[2] * 2.55));
        } else {
            rgbi = [128; 3];
        }
    }
    // clip values as the CSS spec requires
    for v in &mut rgbi {
        if *v > 255 {
            *v = 255;
        }
    }
    nsvg_rgb(rgbi[0], rgbi[1], rgbi[2])
}

/// Translation of `nsvg__colors[]` (without `NANOSVG_ALL_COLOR_KEYWORDS`,
/// which IMG_svg.c doesn't define).
static NSVG_COLORS: &[(&[u8], (u32, u32, u32))] = &[
    (b"red", (255, 0, 0)),
    (b"green", (0, 128, 0)),
    (b"blue", (0, 0, 255)),
    (b"yellow", (255, 255, 0)),
    (b"cyan", (0, 255, 255)),
    (b"magenta", (255, 0, 255)),
    (b"black", (0, 0, 0)),
    (b"grey", (128, 128, 128)),
    (b"gray", (128, 128, 128)),
    (b"white", (255, 255, 255)),
];

/// Translation of `nsvg__parseColorName()`.
fn nsvg_parse_color_name(s: &[u8]) -> u32 {
    for &(name, (r, g, b)) in NSVG_COLORS {
        if name == s {
            return nsvg_rgb(r, g, b);
        }
    }

    nsvg_rgb(128, 128, 128)
}

/// Translation of `nsvg__parseColor()`.
fn nsvg_parse_color(s: &[u8]) -> u32 {
    let mut s = cstr(s);
    while at(s, 0) == b' ' {
        s = &s[1..];
    }
    let len = s.len();
    if len >= 1 && s[0] == b'#' {
        return nsvg_parse_color_hex(s);
    } else if len >= 4 && s.starts_with(b"rgb(") {
        return nsvg_parse_color_rgb(s);
    }
    nsvg_parse_color_name(s)
}

/// Translation of `nsvg__parseOpacity()`.
fn nsvg_parse_opacity(s: &[u8]) -> f32 {
    let mut val = nsvg_atof(s);
    if val < 0.0 {
        val = 0.0;
    }
    if val > 1.0 {
        val = 1.0;
    }
    val
}

/// Translation of `nsvg__parseMiterLimit()`.
fn nsvg_parse_miter_limit(s: &[u8]) -> f32 {
    let mut val = nsvg_atof(s);
    if val < 0.0 {
        val = 0.0;
    }
    val
}

/// Translation of `nsvg__parseUnits()`.
fn nsvg_parse_units(units: &[u8]) -> i32 {
    let (u0, u1) = (at(units, 0), at(units, 1));
    if u0 == b'p' && u1 == b'x' {
        NSVG_UNITS_PX
    } else if u0 == b'p' && u1 == b't' {
        NSVG_UNITS_PT
    } else if u0 == b'p' && u1 == b'c' {
        NSVG_UNITS_PC
    } else if u0 == b'm' && u1 == b'm' {
        NSVG_UNITS_MM
    } else if u0 == b'c' && u1 == b'm' {
        NSVG_UNITS_CM
    } else if u0 == b'i' && u1 == b'n' {
        NSVG_UNITS_IN
    } else if u0 == b'%' {
        NSVG_UNITS_PERCENT
    } else if u0 == b'e' && u1 == b'm' {
        NSVG_UNITS_EM
    } else if u0 == b'e' && u1 == b'x' {
        NSVG_UNITS_EX
    } else {
        NSVG_UNITS_USER
    }
}

/// Translation of `nsvg__isCoordinate()`.
fn nsvg_is_coordinate(s: &[u8]) -> bool {
    let mut i = 0;
    // optional sign
    if at(s, 0) == b'-' || at(s, 0) == b'+' {
        i += 1;
    }
    // must have at least one digit, or start by a dot
    nsvg_isdigit(at(s, i)) || at(s, i) == b'.'
}

/// Translation of `nsvg__parseCoordinateRaw()`.
fn nsvg_parse_coordinate_raw(s: &[u8]) -> NsvgCoordinate {
    let mut buf = Vec::new();
    let units = nsvg_parse_units(nsvg_parse_number(s, &mut buf, 64));
    NsvgCoordinate {
        value: nsvg_atof(&buf),
        units,
    }
}

/// Translation of `nsvg__coord()`.
fn nsvg_coord(v: f32, units: i32) -> NsvgCoordinate {
    NsvgCoordinate { value: v, units }
}

impl NsvgParser {
    /// Translation of `nsvg__parseCoordinate()`.
    fn parse_coordinate(&self, s: &[u8], orig: f32, length: f32) -> f32 {
        let coord = nsvg_parse_coordinate_raw(s);
        self.convert_to_pixels(coord, orig, length)
    }
}

/// Translation of `nsvg__parseTransformArgs()`: returns the length parsed
/// (0 for an error), with the arguments in `args` and their count in `na`.
fn nsvg_parse_transform_args(s: &[u8], args: &mut [f32], max_na: usize, na: &mut usize) -> usize {
    let s = cstr(s);
    let mut it = Vec::new();

    *na = 0;
    let Some(mut ptr) = s.iter().position(|&c| c == b'(') else {
        return 1;
    };
    let Some(end) = s[ptr..].iter().position(|&c| c == b')').map(|e| ptr + e) else {
        return 1;
    };

    while ptr < end {
        let c = s[ptr];
        if c == b'-' || c == b'+' || c == b'.' || nsvg_isdigit(c) {
            if *na >= max_na {
                return 0;
            }
            let rest = nsvg_parse_number(&s[ptr..], &mut it, 64);
            ptr = s.len() - rest.len();
            args[*na] = nsvg_atof(&it);
            *na += 1;
        } else {
            ptr += 1;
        }
    }
    end
}

/// Translation of `nsvg__parseMatrix()`.
fn nsvg_parse_matrix(xform: &mut [f32; 6], s: &[u8]) -> usize {
    let mut t = [0.0f32; 6];
    let mut na = 0;
    let len = nsvg_parse_transform_args(s, &mut t, 6, &mut na);
    if na != 6 {
        return len;
    }
    *xform = t;
    len
}

// FIXME (upstream): the argument arrays below are uninitialized there, so
// arguments that are missing take stack garbage; they are 0 here.

/// Translation of `nsvg__parseTranslate()`.
fn nsvg_parse_translate(xform: &mut [f32; 6], s: &[u8]) -> usize {
    let mut args = [0.0f32; 2];
    let mut t = [0.0f32; 6];
    let mut na = 0;
    let len = nsvg_parse_transform_args(s, &mut args, 2, &mut na);
    if na == 1 {
        args[1] = 0.0;
    }

    nsvg_xform_set_translation(&mut t, args[0], args[1]);
    *xform = t;
    len
}

/// Translation of `nsvg__parseScale()`.
fn nsvg_parse_scale(xform: &mut [f32; 6], s: &[u8]) -> usize {
    let mut args = [0.0f32; 2];
    let mut na = 0;
    let mut t = [0.0f32; 6];
    let len = nsvg_parse_transform_args(s, &mut args, 2, &mut na);
    if na == 1 {
        args[1] = args[0];
    }
    nsvg_xform_set_scale(&mut t, args[0], args[1]);
    *xform = t;
    len
}

/// Translation of `nsvg__parseSkewX()`.
fn nsvg_parse_skew_x(xform: &mut [f32; 6], s: &[u8]) -> usize {
    let mut args = [0.0f32; 1];
    let mut na = 0;
    let mut t = [0.0f32; 6];
    let len = nsvg_parse_transform_args(s, &mut args, 1, &mut na);
    nsvg_xform_set_skew_x(&mut t, args[0] / 180.0 * NSVG_PI);
    *xform = t;
    len
}

/// Translation of `nsvg__parseSkewY()`.
fn nsvg_parse_skew_y(xform: &mut [f32; 6], s: &[u8]) -> usize {
    let mut args = [0.0f32; 1];
    let mut na = 0;
    let mut t = [0.0f32; 6];
    let len = nsvg_parse_transform_args(s, &mut args, 1, &mut na);
    nsvg_xform_set_skew_y(&mut t, args[0] / 180.0 * NSVG_PI);
    *xform = t;
    len
}

/// Translation of `nsvg__parseRotate()`.
fn nsvg_parse_rotate(xform: &mut [f32; 6], s: &[u8]) -> usize {
    let mut args = [0.0f32; 3];
    let mut na = 0;
    let mut m = [0.0f32; 6];
    let mut t = [0.0f32; 6];
    let len = nsvg_parse_transform_args(s, &mut args, 3, &mut na);
    if na == 1 {
        args[1] = 0.0;
        args[2] = 0.0;
    }
    nsvg_xform_identity(&mut m);

    if na > 1 {
        nsvg_xform_set_translation(&mut t, -args[1], -args[2]);
        nsvg_xform_multiply(&mut m, &t);
    }

    nsvg_xform_set_rotation(&mut t, args[0] / 180.0 * NSVG_PI);
    nsvg_xform_multiply(&mut m, &t);

    if na > 1 {
        nsvg_xform_set_translation(&mut t, args[1], args[2]);
        nsvg_xform_multiply(&mut m, &t);
    }

    *xform = m;

    len
}

/// Translation of `nsvg__parseTransform()`.
fn nsvg_parse_transform(xform: &mut [f32; 6], s: &[u8]) {
    let mut t = [0.0f32; 6];
    let mut s = cstr(s);
    nsvg_xform_identity(xform);
    while !s.is_empty() {
        let len = if s.starts_with(b"matrix") {
            nsvg_parse_matrix(&mut t, s)
        } else if s.starts_with(b"translate") {
            nsvg_parse_translate(&mut t, s)
        } else if s.starts_with(b"scale") {
            nsvg_parse_scale(&mut t, s)
        } else if s.starts_with(b"rotate") {
            nsvg_parse_rotate(&mut t, s)
        } else if s.starts_with(b"skewX") {
            nsvg_parse_skew_x(&mut t, s)
        } else if s.starts_with(b"skewY") {
            nsvg_parse_skew_y(&mut t, s)
        } else {
            s = &s[1..];
            continue;
        };
        if len != 0 {
            s = s.get(len..).unwrap_or(&[]);
        } else {
            s = &s[1..];
            continue;
        }

        nsvg_xform_premultiply(xform, &t);
    }
}

/// Translation of `nsvg__parseUrl()`: `s` starts with `url(`.
fn nsvg_parse_url(id: &mut [u8; 64], s: &[u8]) {
    let mut i = 0;
    let mut s = &cstr(s)[4..]; // "url(";
    if at(s, 0) == b'#' {
        s = &s[1..];
    }
    while i < 63 && !s.is_empty() && s[0] != b')' {
        id[i] = s[0];
        s = &s[1..];
        i += 1;
    }
    id[i] = 0;
}

/// Translation of `nsvg__parseLineCap()`.
fn nsvg_parse_line_cap(s: &[u8]) -> i8 {
    match s {
        b"butt" => NSVG_CAP_BUTT,
        b"round" => NSVG_CAP_ROUND,
        b"square" => NSVG_CAP_SQUARE,
        // TODO: handle inherit.
        _ => NSVG_CAP_BUTT,
    }
}

/// Translation of `nsvg__parseLineJoin()`.
fn nsvg_parse_line_join(s: &[u8]) -> i8 {
    match s {
        b"miter" => NSVG_JOIN_MITER,
        b"round" => NSVG_JOIN_ROUND,
        b"bevel" => NSVG_JOIN_BEVEL,
        // TODO: handle inherit.
        _ => NSVG_JOIN_MITER,
    }
}

/// Translation of `nsvg__parseFillRule()`.
fn nsvg_parse_fill_rule(s: &[u8]) -> i8 {
    match s {
        b"nonzero" => NSVG_FILLRULE_NONZERO,
        b"evenodd" => NSVG_FILLRULE_EVENODD,
        // TODO: handle inherit.
        _ => NSVG_FILLRULE_NONZERO,
    }
}

/// Translation of `nsvg__getNextDashItem()`.
fn nsvg_get_next_dash_item<'a>(s: &'a [u8], it: &mut Vec<u8>) -> &'a [u8] {
    let mut s = cstr(s);
    it.clear();
    // Skip white spaces and commas
    while !s.is_empty() && (nsvg_isspace(s[0]) || s[0] == b',') {
        s = &s[1..];
    }
    // Advance until whitespace, comma or end.
    while !s.is_empty() && (!nsvg_isspace(s[0]) && s[0] != b',') {
        if it.len() < 63 {
            it.push(s[0]);
        }
        s = &s[1..];
    }
    s
}

impl NsvgParser {
    /// Translation of `nsvg__parseStrokeDashArray()`.
    fn parse_stroke_dash_array(
        &self,
        s: &[u8],
        stroke_dash_array: &mut [f32; NSVG_MAX_DASHES],
    ) -> i32 {
        let mut item = Vec::new();
        let mut count = 0;
        let mut sum = 0.0f32;
        let mut s = cstr(s);

        // Handle "none"
        if at(s, 0) == b'n' {
            return 0;
        }

        // Parse dashes
        while !s.is_empty() {
            s = nsvg_get_next_dash_item(s, &mut item);
            if item.is_empty() {
                break;
            }
            if count < NSVG_MAX_DASHES {
                stroke_dash_array[count] =
                    fabsf(self.parse_coordinate(&item, 0.0, self.actual_length()));
                count += 1;
            }
        }

        for i in 0..count {
            sum += stroke_dash_array[i];
        }
        if sum <= 1e-6 {
            count = 0;
        }

        count as i32
    }

    // Apply any matching class styles for a "class" attribute value. We support only simple class
    // selectors. The class attribute may contain multiple space-separated class names.
    /// Translation of `nsvg__applyClassStyles()`.
    fn apply_class_styles(&mut self, value: &[u8]) {
        let mut cur = cstr(value);
        while !cur.is_empty() {
            while !cur.is_empty() && nsvg_isspace(cur[0]) {
                cur = &cur[1..];
            }
            if cur.is_empty() {
                break;
            }

            let class_len = cur
                .iter()
                .position(|&c| nsvg_isspace(c))
                .unwrap_or(cur.len());
            let class = &cur[..class_len];
            cur = &cur[class_len..];

            // (newest first, as the C list)
            for i in (0..self.styles.len()).rev() {
                if self.styles[i].class_name == class {
                    let text = self.styles[i].properties_text.clone();
                    self.parse_style(&text);
                }
            }
        }
    }

    /// Translation of `nsvg__parseAttr()` (`false` for 0).
    fn parse_attr(&mut self, name: &[u8], value: &[u8]) -> bool {
        let mut xform = [0.0f32; 6];
        let name = cstr(name);
        let value = cstr(value);

        if name == b"style" {
            self.parse_style(value);
        } else if name == b"display" {
            if value == b"none" {
                self.get_attr().visible = false;
            }
            // Don't reset ->visible on display:inline, one display:none hides the whole subtree
        } else if name == b"fill" {
            if value == b"none" {
                self.get_attr().has_fill = 0;
            } else if value.starts_with(b"url(") {
                let attr = self.get_attr();
                attr.has_fill = 2;
                nsvg_parse_url(&mut attr.fill_gradient.0, value);
            } else {
                let attr = self.get_attr();
                attr.has_fill = 1;
                attr.fill_color = nsvg_parse_color(value);
            }
        } else if name == b"opacity" {
            self.get_attr().opacity = nsvg_parse_opacity(value);
        } else if name == b"fill-opacity" {
            self.get_attr().fill_opacity = nsvg_parse_opacity(value);
        } else if name == b"stroke" {
            if value == b"none" {
                self.get_attr().has_stroke = 0;
            } else if value.starts_with(b"url(") {
                let attr = self.get_attr();
                attr.has_stroke = 2;
                nsvg_parse_url(&mut attr.stroke_gradient.0, value);
            } else {
                let attr = self.get_attr();
                attr.has_stroke = 1;
                attr.stroke_color = nsvg_parse_color(value);
            }
        } else if name == b"stroke-width" {
            let v = self.parse_coordinate(value, 0.0, self.actual_length());
            self.get_attr().stroke_width = v;
        } else if name == b"stroke-dasharray" {
            let mut dashes = self.attr[self.attr_head].stroke_dash_array;
            let count = self.parse_stroke_dash_array(value, &mut dashes);
            let attr = self.get_attr();
            attr.stroke_dash_array = dashes;
            attr.stroke_dash_count = count;
        } else if name == b"stroke-dashoffset" {
            let v = self.parse_coordinate(value, 0.0, self.actual_length());
            self.get_attr().stroke_dash_offset = v;
        } else if name == b"stroke-opacity" {
            self.get_attr().stroke_opacity = nsvg_parse_opacity(value);
        } else if name == b"stroke-linecap" {
            self.get_attr().stroke_line_cap = nsvg_parse_line_cap(value);
        } else if name == b"stroke-linejoin" {
            self.get_attr().stroke_line_join = nsvg_parse_line_join(value);
        } else if name == b"stroke-miterlimit" {
            self.get_attr().miter_limit = nsvg_parse_miter_limit(value);
        } else if name == b"fill-rule" {
            self.get_attr().fill_rule = nsvg_parse_fill_rule(value);
        } else if name == b"font-size" {
            let v = self.parse_coordinate(value, 0.0, self.actual_length());
            self.get_attr().font_size = v;
        } else if name == b"transform" {
            nsvg_parse_transform(&mut xform, value);
            nsvg_xform_premultiply(&mut self.get_attr().xform, &xform);
        } else if name == b"stop-color" {
            self.get_attr().stop_color = nsvg_parse_color(value);
        } else if name == b"stop-opacity" {
            self.get_attr().stop_opacity = nsvg_parse_opacity(value);
        } else if name == b"offset" {
            let v = self.parse_coordinate(value, 0.0, 1.0);
            self.get_attr().stop_offset = v;
        } else if name == b"id" {
            let attr = self.get_attr();
            strlcpy(&mut attr.id.0, value, 63);
            attr.id.0[63] = 0;
        } else if name == b"class" {
            self.apply_class_styles(value);
        } else {
            return false;
        }
        true
    }

    /// Translation of `nsvg__parseNameValue()`: `s` holds the declaration
    /// from `start` to `end` (it may be read past `end`, to its NUL).
    fn parse_name_value(&mut self, s: &[u8], start: usize, end: usize) -> bool {
        let mut str = start;
        while str < end && at(s, str) != b':' {
            str += 1;
        }

        let mut val = str;

        // Right Trim
        while str > start && (at(s, str) == b':' || nsvg_isspace(at(s, str))) {
            str -= 1;
        }
        str += 1;

        let n = (str - start).min(511);
        let mut name = vec![0u8; n];
        for (i, c) in name.iter_mut().enumerate() {
            *c = at(s, start + i);
        }

        while val < end && (at(s, val) == b':' || nsvg_isspace(at(s, val))) {
            val += 1;
        }

        let n = end.saturating_sub(val).min(511);
        let mut value = vec![0u8; n];
        for (i, c) in value.iter_mut().enumerate() {
            *c = at(s, val + i);
        }

        self.parse_attr(&name, &value)
    }

    /// Translation of `nsvg__parseStyle()`.
    fn parse_style(&mut self, s: &[u8]) {
        let s = cstr(s);
        let mut str = 0;

        while str < s.len() {
            // Left Trim
            while str < s.len() && nsvg_isspace(s[str]) {
                str += 1;
            }
            let start = str;
            while str < s.len() && s[str] != b';' {
                str += 1;
            }
            let mut end = str;

            // Right Trim
            while end > start && (at(s, end) == b';' || nsvg_isspace(at(s, end))) {
                end -= 1;
            }
            end += 1;

            self.parse_name_value(s, start, end);
            if str < s.len() {
                str += 1;
            }
        }
    }

    /// Translation of `nsvg__parseAttribs()`.
    fn parse_attribs(&mut self, attr: &[(&[u8], &[u8])]) {
        for &(name, value) in attr {
            if name == b"style" {
                self.parse_style(value);
            } else {
                self.parse_attr(name, value);
            }
        }
    }
}

/// Translation of `nsvg__getArgsPerElement()`.
fn nsvg_get_args_per_element(cmd: u8) -> i32 {
    match cmd {
        b'v' | b'V' | b'h' | b'H' => 1,
        b'm' | b'M' | b'l' | b'L' | b't' | b'T' => 2,
        b'q' | b'Q' | b's' | b'S' => 4,
        b'c' | b'C' => 6,
        b'a' | b'A' => 7,
        b'z' | b'Z' => 0,
        _ => -1,
    }
}

/// The current point and the last control point of a path:
/// `cpx`, `cpy`, `cpx2`, `cpy2`.
#[derive(Clone, Copy, Default)]
struct PathCursor {
    cpx: f32,
    cpy: f32,
    cpx2: f32,
    cpy2: f32,
}

impl NsvgParser {
    /// Translation of `nsvg__pathMoveTo()`.
    fn path_move_to(&mut self, c: &mut PathCursor, args: &[f32], rel: bool) {
        if rel {
            c.cpx += args[0];
            c.cpy += args[1];
        } else {
            c.cpx = args[0];
            c.cpy = args[1];
        }
        self.move_to(c.cpx, c.cpy);
    }

    /// Translation of `nsvg__pathLineTo()`.
    fn path_line_to(&mut self, c: &mut PathCursor, args: &[f32], rel: bool) {
        if rel {
            c.cpx += args[0];
            c.cpy += args[1];
        } else {
            c.cpx = args[0];
            c.cpy = args[1];
        }
        self.line_to(c.cpx, c.cpy);
    }

    /// Translation of `nsvg__pathHLineTo()`.
    fn path_h_line_to(&mut self, c: &mut PathCursor, args: &[f32], rel: bool) {
        if rel {
            c.cpx += args[0];
        } else {
            c.cpx = args[0];
        }
        self.line_to(c.cpx, c.cpy);
    }

    /// Translation of `nsvg__pathVLineTo()`.
    fn path_v_line_to(&mut self, c: &mut PathCursor, args: &[f32], rel: bool) {
        if rel {
            c.cpy += args[0];
        } else {
            c.cpy = args[0];
        }
        self.line_to(c.cpx, c.cpy);
    }

    /// Translation of `nsvg__pathCubicBezTo()`.
    fn path_cubic_bez_to(&mut self, c: &mut PathCursor, args: &[f32], rel: bool) {
        let (x2, y2, cx1, cy1, cx2, cy2);

        if rel {
            cx1 = c.cpx + args[0];
            cy1 = c.cpy + args[1];
            cx2 = c.cpx + args[2];
            cy2 = c.cpy + args[3];
            x2 = c.cpx + args[4];
            y2 = c.cpy + args[5];
        } else {
            cx1 = args[0];
            cy1 = args[1];
            cx2 = args[2];
            cy2 = args[3];
            x2 = args[4];
            y2 = args[5];
        }

        self.cubic_bez_to(cx1, cy1, cx2, cy2, x2, y2);

        c.cpx2 = cx2;
        c.cpy2 = cy2;
        c.cpx = x2;
        c.cpy = y2;
    }

    /// Translation of `nsvg__pathCubicBezShortTo()`.
    fn path_cubic_bez_short_to(&mut self, c: &mut PathCursor, args: &[f32], rel: bool) {
        let (x2, y2, cx2, cy2);

        let x1 = c.cpx;
        let y1 = c.cpy;
        if rel {
            cx2 = c.cpx + args[0];
            cy2 = c.cpy + args[1];
            x2 = c.cpx + args[2];
            y2 = c.cpy + args[3];
        } else {
            cx2 = args[0];
            cy2 = args[1];
            x2 = args[2];
            y2 = args[3];
        }

        let cx1 = 2.0 * x1 - c.cpx2;
        let cy1 = 2.0 * y1 - c.cpy2;

        self.cubic_bez_to(cx1, cy1, cx2, cy2, x2, y2);

        c.cpx2 = cx2;
        c.cpy2 = cy2;
        c.cpx = x2;
        c.cpy = y2;
    }

    /// Translation of `nsvg__pathQuadBezTo()`.
    fn path_quad_bez_to(&mut self, c: &mut PathCursor, args: &[f32], rel: bool) {
        let (x2, y2, cx, cy);

        let x1 = c.cpx;
        let y1 = c.cpy;
        if rel {
            cx = c.cpx + args[0];
            cy = c.cpy + args[1];
            x2 = c.cpx + args[2];
            y2 = c.cpy + args[3];
        } else {
            cx = args[0];
            cy = args[1];
            x2 = args[2];
            y2 = args[3];
        }

        // Convert to cubic bezier
        let cx1 = x1 + 2.0 / 3.0 * (cx - x1);
        let cy1 = y1 + 2.0 / 3.0 * (cy - y1);
        let cx2 = x2 + 2.0 / 3.0 * (cx - x2);
        let cy2 = y2 + 2.0 / 3.0 * (cy - y2);

        self.cubic_bez_to(cx1, cy1, cx2, cy2, x2, y2);

        c.cpx2 = cx;
        c.cpy2 = cy;
        c.cpx = x2;
        c.cpy = y2;
    }

    /// Translation of `nsvg__pathQuadBezShortTo()`.
    fn path_quad_bez_short_to(&mut self, c: &mut PathCursor, args: &[f32], rel: bool) {
        let (x2, y2);

        let x1 = c.cpx;
        let y1 = c.cpy;
        if rel {
            x2 = c.cpx + args[0];
            y2 = c.cpy + args[1];
        } else {
            x2 = args[0];
            y2 = args[1];
        }

        let cx = 2.0 * x1 - c.cpx2;
        let cy = 2.0 * y1 - c.cpy2;

        // Convert to cubix bezier
        let cx1 = x1 + 2.0 / 3.0 * (cx - x1);
        let cy1 = y1 + 2.0 / 3.0 * (cy - y1);
        let cx2 = x2 + 2.0 / 3.0 * (cx - x2);
        let cy2 = y2 + 2.0 / 3.0 * (cy - y2);

        self.cubic_bez_to(cx1, cy1, cx2, cy2, x2, y2);

        c.cpx2 = cx;
        c.cpy2 = cy;
        c.cpx = x2;
        c.cpy = y2;
    }
}

/// Translation of `nsvg__sqr()`.
fn nsvg_sqr(x: f32) -> f32 {
    x * x
}

/// Translation of `nsvg__vmag()`.
fn nsvg_vmag(x: f32, y: f32) -> f32 {
    sqrtf(x * x + y * y)
}

/// Translation of `nsvg__vecrat()`.
fn nsvg_vecrat(ux: f32, uy: f32, vx: f32, vy: f32) -> f32 {
    (ux * vx + uy * vy) / (nsvg_vmag(ux, uy) * nsvg_vmag(vx, vy))
}

/// Translation of `nsvg__vecang()`.
fn nsvg_vecang(ux: f32, uy: f32, vx: f32, vy: f32) -> f32 {
    let mut r = nsvg_vecrat(ux, uy, vx, vy);
    if r < -1.0 {
        r = -1.0;
    }
    if r > 1.0 {
        r = 1.0;
    }
    (if ux * vy < uy * vx { -1.0 } else { 1.0 }) * acosf(r)
}

impl NsvgParser {
    /// Translation of `nsvg__pathArcTo()`.
    fn path_arc_to(&mut self, c: &mut PathCursor, args: &[f32], rel: bool) {
        // Ported from canvg (https://code.google.com/p/canvg/)
        let (mut px, mut py, mut ptanx, mut ptany) = (0.0f32, 0.0f32, 0.0f32, 0.0f32);

        let mut rx = fabsf(args[0]); // y radius
        let mut ry = fabsf(args[1]); // x radius
        let rotx = args[2] / 180.0 * NSVG_PI; // x rotation angle
        let fa = (fabsf(args[3]) as f64) > 1e-6; // Large arc
        let fs = (fabsf(args[4]) as f64) > 1e-6; // Sweep direction
        let x1 = c.cpx; // start point
        let y1 = c.cpy;
        let (x2, y2);
        if rel {
            // end point
            x2 = c.cpx + args[5];
            y2 = c.cpy + args[6];
        } else {
            x2 = args[5];
            y2 = args[6];
        }

        let mut dx = x1 - x2;
        let mut dy = y1 - y2;
        let mut d = sqrtf(dx * dx + dy * dy);
        if d < 1e-6 || rx < 1e-6 || ry < 1e-6 {
            // The arc degenerates to a line
            self.line_to(x2, y2);
            c.cpx = x2;
            c.cpy = y2;
            return;
        }

        let sinrx = sinf(rotx);
        let cosrx = cosf(rotx);

        // Convert to center point parameterization.
        // http://www.w3.org/TR/SVG11/implnote.html#ArcImplementationNotes
        // 1) Compute x1', y1'
        let x1p = cosrx * dx / 2.0 + sinrx * dy / 2.0;
        let y1p = -sinrx * dx / 2.0 + cosrx * dy / 2.0;
        d = nsvg_sqr(x1p) / nsvg_sqr(rx) + nsvg_sqr(y1p) / nsvg_sqr(ry);
        if d > 1.0 {
            d = sqrtf(d);
            rx *= d;
            ry *= d;
        }
        // 2) Compute cx', cy'
        let mut s = 0.0f32;
        let mut sa = nsvg_sqr(rx) * nsvg_sqr(ry)
            - nsvg_sqr(rx) * nsvg_sqr(y1p)
            - nsvg_sqr(ry) * nsvg_sqr(x1p);
        let sb = nsvg_sqr(rx) * nsvg_sqr(y1p) + nsvg_sqr(ry) * nsvg_sqr(x1p);
        if sa < 0.0 {
            sa = 0.0;
        }
        if sb > 0.0 {
            s = sqrtf(sa / sb);
        }
        if fa == fs {
            s = -s;
        }
        let cxp = s * rx * y1p / ry;
        let cyp = s * -ry * x1p / rx;

        // 3) Compute cx,cy from cx',cy'
        let cx = (x1 + x2) / 2.0 + cosrx * cxp - sinrx * cyp;
        let cy = (y1 + y2) / 2.0 + sinrx * cxp + cosrx * cyp;

        // 4) Calculate theta1, and delta theta.
        let ux = (x1p - cxp) / rx;
        let uy = (y1p - cyp) / ry;
        let vx = (-x1p - cxp) / rx;
        let vy = (-y1p - cyp) / ry;
        let a1 = nsvg_vecang(1.0, 0.0, ux, uy); // Initial angle
        let mut da = nsvg_vecang(ux, uy, vx, vy); // Delta angle

        //	if (vecrat(ux,uy,vx,vy) <= -1.0f) da = NSVG_PI;
        //	if (vecrat(ux,uy,vx,vy) >= 1.0f) da = 0;

        if !fs && da > 0.0 {
            da -= 2.0 * NSVG_PI;
        } else if fs && da < 0.0 {
            da += 2.0 * NSVG_PI;
        }

        // Approximate the arc using cubic spline segments.
        let t = [cosrx, sinrx, -sinrx, cosrx, cx, cy];

        // Split arc into max 90 degree segments.
        // The loop assumes an iteration per end point (including start and end), this +1.
        let ndivs = c_f32_to_i32(fabsf(da) / (NSVG_PI * 0.5) + 1.0);
        let mut hda = (da / ndivs as f32) / 2.0;
        // Fix for ticket #179: division by 0: avoid cotangens around 0 (infinite)
        if (hda < 1e-3) && (hda > -1e-3) {
            hda *= 0.5;
        } else {
            hda = (1.0 - cosf(hda)) / sinf(hda);
        }
        let mut kappa = fabsf(4.0 / 3.0 * hda);
        if da < 0.0 {
            kappa = -kappa;
        }

        let mut i = 0;
        while i <= ndivs {
            let a = a1 + da * (i as f32 / ndivs as f32);
            dx = cosf(a);
            dy = sinf(a);
            let (x, y) = nsvg_xform_point(dx * rx, dy * ry, &t); // position
            let (tanx, tany) = nsvg_xform_vec(-dy * rx * kappa, dx * ry * kappa, &t); // tangent
            if i > 0 {
                self.cubic_bez_to(px + ptanx, py + ptany, x - tanx, y - tany, x, y);
            }
            px = x;
            py = y;
            ptanx = tanx;
            ptany = tany;
            i += 1;
        }

        c.cpx = x2;
        c.cpy = y2;
    }

    /// Translation of `nsvg__parsePath()`.
    fn parse_path(&mut self, attr: &[(&[u8], &[u8])]) {
        let mut s: Option<&[u8]> = None;
        let mut cmd = 0u8;
        let mut args = [0.0f32; 10];
        let mut nargs: usize;
        let mut rargs = 0i32;
        let mut init_point: bool;
        let mut closed_flag: bool;
        let mut item = Vec::new();

        for &(name, value) in attr {
            if name == b"d" {
                s = Some(value);
            } else {
                self.parse_attribs(&[(name, value)]);
            }
        }

        if let Some(mut s) = s {
            self.reset_path();
            let mut c = PathCursor::default();
            init_point = false;
            closed_flag = false;
            nargs = 0;

            while !s.is_empty() {
                item.clear();
                if (cmd == b'A' || cmd == b'a') && (nargs == 3 || nargs == 4) {
                    s = nsvg_get_next_path_item_when_arc_flag(s, &mut item);
                }
                if item.is_empty() {
                    s = nsvg_get_next_path_item(s, &mut item);
                }
                if item.is_empty() {
                    break;
                }
                if cmd != 0 && nsvg_is_coordinate(&item) {
                    if nargs < 10 {
                        args[nargs] = nsvg_atof(&item);
                        nargs += 1;
                    }
                    if nargs as i32 >= rargs {
                        match cmd {
                            b'm' | b'M' => {
                                self.path_move_to(&mut c, &args, cmd == b'm');
                                // Moveto can be followed by multiple coordinate pairs,
                                // which should be treated as linetos.
                                cmd = if cmd == b'm' { b'l' } else { b'L' };
                                rargs = nsvg_get_args_per_element(cmd);
                                c.cpx2 = c.cpx;
                                c.cpy2 = c.cpy;
                                init_point = true;
                            }
                            b'l' | b'L' => {
                                self.path_line_to(&mut c, &args, cmd == b'l');
                                c.cpx2 = c.cpx;
                                c.cpy2 = c.cpy;
                            }
                            b'H' | b'h' => {
                                self.path_h_line_to(&mut c, &args, cmd == b'h');
                                c.cpx2 = c.cpx;
                                c.cpy2 = c.cpy;
                            }
                            b'V' | b'v' => {
                                self.path_v_line_to(&mut c, &args, cmd == b'v');
                                c.cpx2 = c.cpx;
                                c.cpy2 = c.cpy;
                            }
                            b'C' | b'c' => {
                                self.path_cubic_bez_to(&mut c, &args, cmd == b'c');
                            }
                            b'S' | b's' => {
                                self.path_cubic_bez_short_to(&mut c, &args, cmd == b's');
                            }
                            b'Q' | b'q' => {
                                self.path_quad_bez_to(&mut c, &args, cmd == b'q');
                            }
                            b'T' | b't' => {
                                self.path_quad_bez_short_to(&mut c, &args, cmd == b't');
                            }
                            b'A' | b'a' => {
                                self.path_arc_to(&mut c, &args, cmd == b'a');
                                c.cpx2 = c.cpx;
                                c.cpy2 = c.cpy;
                            }
                            _ => {
                                if nargs >= 2 {
                                    c.cpx = args[nargs - 2];
                                    c.cpy = args[nargs - 1];
                                    c.cpx2 = c.cpx;
                                    c.cpy2 = c.cpy;
                                }
                            }
                        }
                        nargs = 0;
                    }
                } else {
                    cmd = item[0];
                    if cmd == b'M' || cmd == b'm' {
                        // Commit path.
                        if self.npts > 0 {
                            self.add_path(closed_flag);
                        }
                        // Start new subpath.
                        self.reset_path();
                        closed_flag = false;
                        nargs = 0;
                    } else if !init_point {
                        // Do not allow other commands until initial point has been set (moveTo called once).
                        cmd = 0;
                    }
                    if cmd == b'Z' || cmd == b'z' {
                        closed_flag = true;
                        // Commit path.
                        if self.npts > 0 {
                            // Move current point to first point
                            c.cpx = self.pts[0];
                            c.cpy = self.pts[1];
                            c.cpx2 = c.cpx;
                            c.cpy2 = c.cpy;
                            self.add_path(closed_flag);
                        }
                        // Start new subpath.
                        self.reset_path();
                        self.move_to(c.cpx, c.cpy);
                        closed_flag = false;
                        nargs = 0;
                    }
                    rargs = nsvg_get_args_per_element(cmd);
                    if rargs == -1 {
                        // Command not recognized
                        cmd = 0;
                        rargs = 0;
                    }
                }
            }
            // Commit path.
            if self.npts != 0 {
                self.add_path(closed_flag);
            }
        }

        self.add_shape();
    }

    /// Translation of `nsvg__parseRect()`.
    fn parse_rect(&mut self, attr: &[(&[u8], &[u8])]) {
        let mut x = 0.0f32;
        let mut y = 0.0f32;
        let mut w = 0.0f32;
        let mut h = 0.0f32;
        let mut rx = -1.0f32; // marks not set
        let mut ry = -1.0f32;

        for &(name, value) in attr {
            if !self.parse_attr(name, value) {
                if name == b"x" {
                    x = self.parse_coordinate(value, self.actual_orig_x(), self.actual_width());
                }
                if name == b"y" {
                    y = self.parse_coordinate(value, self.actual_orig_y(), self.actual_height());
                }
                if name == b"width" {
                    w = self.parse_coordinate(value, 0.0, self.actual_width());
                }
                if name == b"height" {
                    h = self.parse_coordinate(value, 0.0, self.actual_height());
                }
                if name == b"rx" {
                    rx = fabsf(self.parse_coordinate(value, 0.0, self.actual_width()));
                }
                if name == b"ry" {
                    ry = fabsf(self.parse_coordinate(value, 0.0, self.actual_height()));
                }
            }
        }

        if rx < 0.0 && ry > 0.0 {
            rx = ry;
        }
        if ry < 0.0 && rx > 0.0 {
            ry = rx;
        }
        if rx < 0.0 {
            rx = 0.0;
        }
        if ry < 0.0 {
            ry = 0.0;
        }
        if rx > w / 2.0 {
            rx = w / 2.0;
        }
        if ry > h / 2.0 {
            ry = h / 2.0;
        }

        if w != 0.0 && h != 0.0 {
            self.reset_path();

            if rx < 0.00001 || ry < 0.0001 {
                self.move_to(x, y);
                self.line_to(x + w, y);
                self.line_to(x + w, y + h);
                self.line_to(x, y + h);
            } else {
                // Rounded rectangle
                let k = 1.0 - NSVG_KAPPA90;
                self.move_to(x + rx, y);
                self.line_to(x + w - rx, y);
                self.cubic_bez_to(x + w - rx * k, y, x + w, y + ry * k, x + w, y + ry);
                self.line_to(x + w, y + h - ry);
                self.cubic_bez_to(
                    x + w,
                    y + h - ry * k,
                    x + w - rx * k,
                    y + h,
                    x + w - rx,
                    y + h,
                );
                self.line_to(x + rx, y + h);
                self.cubic_bez_to(x + rx * k, y + h, x, y + h - ry * k, x, y + h - ry);
                self.line_to(x, y + ry);
                self.cubic_bez_to(x, y + ry * k, x + rx * k, y, x + rx, y);
            }

            self.add_path(true);

            self.add_shape();
        }
    }

    /// Translation of `nsvg__parseCircle()`.
    fn parse_circle(&mut self, attr: &[(&[u8], &[u8])]) {
        let mut cx = 0.0f32;
        let mut cy = 0.0f32;
        let mut r = 0.0f32;

        for &(name, value) in attr {
            if !self.parse_attr(name, value) {
                if name == b"cx" {
                    cx = self.parse_coordinate(value, self.actual_orig_x(), self.actual_width());
                }
                if name == b"cy" {
                    cy = self.parse_coordinate(value, self.actual_orig_y(), self.actual_height());
                }
                if name == b"r" {
                    r = fabsf(self.parse_coordinate(value, 0.0, self.actual_length()));
                }
            }
        }

        if r > 0.0 {
            self.reset_path();

            let k = NSVG_KAPPA90;
            self.move_to(cx + r, cy);
            self.cubic_bez_to(cx + r, cy + r * k, cx + r * k, cy + r, cx, cy + r);
            self.cubic_bez_to(cx - r * k, cy + r, cx - r, cy + r * k, cx - r, cy);
            self.cubic_bez_to(cx - r, cy - r * k, cx - r * k, cy - r, cx, cy - r);
            self.cubic_bez_to(cx + r * k, cy - r, cx + r, cy - r * k, cx + r, cy);

            self.add_path(true);

            self.add_shape();
        }
    }

    /// Translation of `nsvg__parseEllipse()`.
    fn parse_ellipse(&mut self, attr: &[(&[u8], &[u8])]) {
        let mut cx = 0.0f32;
        let mut cy = 0.0f32;
        let mut rx = 0.0f32;
        let mut ry = 0.0f32;

        for &(name, value) in attr {
            if !self.parse_attr(name, value) {
                if name == b"cx" {
                    cx = self.parse_coordinate(value, self.actual_orig_x(), self.actual_width());
                }
                if name == b"cy" {
                    cy = self.parse_coordinate(value, self.actual_orig_y(), self.actual_height());
                }
                if name == b"rx" {
                    rx = fabsf(self.parse_coordinate(value, 0.0, self.actual_width()));
                }
                if name == b"ry" {
                    ry = fabsf(self.parse_coordinate(value, 0.0, self.actual_height()));
                }
            }
        }

        if rx > 0.0 && ry > 0.0 {
            self.reset_path();

            let k = NSVG_KAPPA90;
            self.move_to(cx + rx, cy);
            self.cubic_bez_to(cx + rx, cy + ry * k, cx + rx * k, cy + ry, cx, cy + ry);
            self.cubic_bez_to(cx - rx * k, cy + ry, cx - rx, cy + ry * k, cx - rx, cy);
            self.cubic_bez_to(cx - rx, cy - ry * k, cx - rx * k, cy - ry, cx, cy - ry);
            self.cubic_bez_to(cx + rx * k, cy - ry, cx + rx, cy - ry * k, cx + rx, cy);

            self.add_path(true);

            self.add_shape();
        }
    }

    /// Translation of `nsvg__parseLine()`.
    fn parse_line(&mut self, attr: &[(&[u8], &[u8])]) {
        let mut x1 = 0.0f32;
        let mut y1 = 0.0f32;
        let mut x2 = 0.0f32;
        let mut y2 = 0.0f32;

        for &(name, value) in attr {
            if !self.parse_attr(name, value) {
                if name == b"x1" {
                    x1 = self.parse_coordinate(value, self.actual_orig_x(), self.actual_width());
                }
                if name == b"y1" {
                    y1 = self.parse_coordinate(value, self.actual_orig_y(), self.actual_height());
                }
                if name == b"x2" {
                    x2 = self.parse_coordinate(value, self.actual_orig_x(), self.actual_width());
                }
                if name == b"y2" {
                    y2 = self.parse_coordinate(value, self.actual_orig_y(), self.actual_height());
                }
            }
        }

        self.reset_path();

        self.move_to(x1, y1);
        self.line_to(x2, y2);

        self.add_path(false);

        self.add_shape();
    }

    /// Translation of `nsvg__parsePoly()`.
    fn parse_poly(&mut self, attr: &[(&[u8], &[u8])], close_flag: bool) {
        let mut args = [0.0f32; 2];
        let mut npts = 0;
        let mut item = Vec::new();

        self.reset_path();

        for &(name, value) in attr {
            if !self.parse_attr(name, value) && name == b"points" {
                let mut s = value;
                let mut nargs = 0;
                while !s.is_empty() {
                    s = nsvg_get_next_path_item(s, &mut item);
                    args[nargs] = nsvg_atof(&item);
                    nargs += 1;
                    if nargs >= 2 {
                        if npts == 0 {
                            self.move_to(args[0], args[1]);
                        } else {
                            self.line_to(args[0], args[1]);
                        }
                        nargs = 0;
                        npts += 1;
                    }
                }
            }
        }

        self.add_path(close_flag);

        self.add_shape();
    }

    /// Translation of `nsvg__parseSVG()`.
    fn parse_svg(&mut self, attr: &[(&[u8], &[u8])]) {
        for &(name, value) in attr {
            if !self.parse_attr(name, value) {
                if name == b"width" {
                    self.image.width = self.parse_coordinate(value, 0.0, 0.0);
                } else if name == b"height" {
                    self.image.height = self.parse_coordinate(value, 0.0, 0.0);
                } else if name == b"viewBox" {
                    let mut buf = Vec::new();
                    fn skip(mut s: &[u8]) -> &[u8] {
                        while !s.is_empty() && (nsvg_isspace(s[0]) || s[0] == b'%' || s[0] == b',')
                        {
                            s = &s[1..];
                        }
                        s
                    }
                    let mut s = nsvg_parse_number(value, &mut buf, 64);
                    self.view_minx = nsvg_atof(&buf);
                    s = skip(s);
                    if s.is_empty() {
                        return;
                    }
                    s = nsvg_parse_number(s, &mut buf, 64);
                    self.view_miny = nsvg_atof(&buf);
                    s = skip(s);
                    if s.is_empty() {
                        return;
                    }
                    s = nsvg_parse_number(s, &mut buf, 64);
                    self.view_width = nsvg_atof(&buf);
                    s = skip(s);
                    if s.is_empty() {
                        return;
                    }
                    nsvg_parse_number(s, &mut buf, 64);
                    self.view_height = nsvg_atof(&buf);
                } else if name == b"preserveAspectRatio" {
                    let has = |needle: &[u8]| value.windows(needle.len()).any(|w| w == needle);
                    if has(b"none") {
                        // No uniform scaling
                        self.align_type = NSVG_ALIGN_NONE;
                    } else {
                        // Parse X align
                        if has(b"xMin") {
                            self.align_x = NSVG_ALIGN_MIN;
                        } else if has(b"xMid") {
                            self.align_x = NSVG_ALIGN_MID;
                        } else if has(b"xMax") {
                            self.align_x = NSVG_ALIGN_MAX;
                        }
                        // Parse X align
                        if has(b"yMin") {
                            self.align_y = NSVG_ALIGN_MIN;
                        } else if has(b"yMid") {
                            self.align_y = NSVG_ALIGN_MID;
                        } else if has(b"yMax") {
                            self.align_y = NSVG_ALIGN_MAX;
                        }
                        // Parse meet/slice
                        self.align_type = NSVG_ALIGN_MEET;
                        if has(b"slice") {
                            self.align_type = NSVG_ALIGN_SLICE;
                        }
                    }
                }
            }
        }
    }

    /// Translation of `nsvg__parseGradient()`.
    fn parse_gradient(&mut self, attr: &[(&[u8], &[u8])], type_: i8) {
        let mut grad = NsvgGradientData {
            units: NSVG_OBJECT_SPACE,
            type_,
            ..NsvgGradientData::default()
        };
        if grad.type_ == NSVG_PAINT_LINEAR_GRADIENT {
            grad.coords[X1] = nsvg_coord(0.0, NSVG_UNITS_PERCENT);
            grad.coords[Y1] = nsvg_coord(0.0, NSVG_UNITS_PERCENT);
            grad.coords[X2] = nsvg_coord(100.0, NSVG_UNITS_PERCENT);
            grad.coords[Y2] = nsvg_coord(0.0, NSVG_UNITS_PERCENT);
        } else if grad.type_ == NSVG_PAINT_RADIAL_GRADIENT {
            grad.coords[CX] = nsvg_coord(50.0, NSVG_UNITS_PERCENT);
            grad.coords[CY] = nsvg_coord(50.0, NSVG_UNITS_PERCENT);
            grad.coords[R] = nsvg_coord(50.0, NSVG_UNITS_PERCENT);
        }

        nsvg_xform_identity(&mut grad.xform);

        for &(name, value) in attr {
            if name == b"id" {
                strlcpy(&mut grad.id.0, value, 63);
                grad.id.0[63] = 0;
            } else if !self.parse_attr(name, value) {
                if name == b"gradientUnits" {
                    if value == b"objectBoundingBox" {
                        grad.units = NSVG_OBJECT_SPACE;
                    } else {
                        grad.units = NSVG_USER_SPACE;
                    }
                } else if name == b"gradientTransform" {
                    nsvg_parse_transform(&mut grad.xform, value);
                } else if name == b"cx" {
                    grad.coords[CX] = nsvg_parse_coordinate_raw(value);
                } else if name == b"cy" {
                    grad.coords[CY] = nsvg_parse_coordinate_raw(value);
                } else if name == b"r" {
                    grad.coords[R] = nsvg_parse_coordinate_raw(value);
                } else if name == b"fx" {
                    grad.coords[FX] = nsvg_parse_coordinate_raw(value);
                } else if name == b"fy" {
                    grad.coords[FY] = nsvg_parse_coordinate_raw(value);
                } else if name == b"x1" {
                    grad.coords[X1] = nsvg_parse_coordinate_raw(value);
                } else if name == b"y1" {
                    grad.coords[Y1] = nsvg_parse_coordinate_raw(value);
                } else if name == b"x2" {
                    grad.coords[X2] = nsvg_parse_coordinate_raw(value);
                } else if name == b"y2" {
                    grad.coords[Y2] = nsvg_parse_coordinate_raw(value);
                } else if name == b"spreadMethod" {
                    if value == b"pad" {
                        grad.spread = NSVG_SPREAD_PAD;
                    } else if value == b"reflect" {
                        grad.spread = NSVG_SPREAD_REFLECT;
                    } else if value == b"repeat" {
                        grad.spread = NSVG_SPREAD_REPEAT;
                    }
                } else if name == b"xlink:href" {
                    // FIXME (upstream): an empty reference is read past its
                    // end there; it is empty here.
                    let href = value.get(1..).unwrap_or(&[]);
                    strlcpy(&mut grad.ref_.0, href, 62);
                    grad.ref_.0[62] = 0;
                }
            }
        }

        self.gradients.push(grad);
    }

    /// Translation of `nsvg__parseGradientStop()`.
    fn parse_gradient_stop(&mut self, attr: &[(&[u8], &[u8])]) {
        {
            let cur_attr = self.get_attr();
            cur_attr.stop_offset = 0.0;
            cur_attr.stop_color = 0;
            cur_attr.stop_opacity = 1.0;
        }

        for &(name, value) in attr {
            self.parse_attr(name, value);
        }

        // Add stop to the last gradient.
        let cur_attr = self.attr[self.attr_head].clone();
        let Some(grad) = self.gradients.last_mut() else {
            return;
        };

        let nstops = grad.stops.len() + 1;
        grad.stops.push(NsvgGradientStop::default());

        // Insert
        let mut idx = nstops - 1;
        for i in 0..nstops - 1 {
            if cur_attr.stop_offset < grad.stops[i].offset {
                idx = i;
                break;
            }
        }
        if idx != nstops - 1 {
            for i in (idx + 1..nstops).rev() {
                grad.stops[i] = grad.stops[i - 1];
            }
        }

        let stop = &mut grad.stops[idx];
        stop.color = cur_attr.stop_color;
        stop.color |= c_f32_to_u32(cur_attr.stop_opacity * 255.0) << 24;
        stop.offset = cur_attr.stop_offset;
    }

    /// Translation of `nsvg__startElement()`.
    fn start_element(&mut self, el: &[u8], attr: &[(&[u8], &[u8])]) {
        if self.defs_flag {
            // Skip everything but gradients and styles in defs
            if el == b"linearGradient" {
                self.parse_gradient(attr, NSVG_PAINT_LINEAR_GRADIENT);
            } else if el == b"radialGradient" {
                self.parse_gradient(attr, NSVG_PAINT_RADIAL_GRADIENT);
            } else if el == b"stop" {
                self.parse_gradient_stop(attr);
            } else if el == b"style" {
                self.style_flag = true;
            }
            return;
        }

        if el == b"g" {
            self.push_attr();
            self.parse_attribs(attr);
        } else if el == b"path" {
            if self.path_flag {
                // Do not allow nested paths.
                return;
            }
            self.push_attr();
            self.parse_path(attr);
            self.pop_attr();
        } else if el == b"rect" {
            self.push_attr();
            self.parse_rect(attr);
            self.pop_attr();
        } else if el == b"circle" {
            self.push_attr();
            self.parse_circle(attr);
            self.pop_attr();
        } else if el == b"ellipse" {
            self.push_attr();
            self.parse_ellipse(attr);
            self.pop_attr();
        } else if el == b"line" {
            self.push_attr();
            self.parse_line(attr);
            self.pop_attr();
        } else if el == b"polyline" {
            self.push_attr();
            self.parse_poly(attr, false);
            self.pop_attr();
        } else if el == b"polygon" {
            self.push_attr();
            self.parse_poly(attr, true);
            self.pop_attr();
        } else if el == b"linearGradient" {
            self.parse_gradient(attr, NSVG_PAINT_LINEAR_GRADIENT);
        } else if el == b"radialGradient" {
            self.parse_gradient(attr, NSVG_PAINT_RADIAL_GRADIENT);
        } else if el == b"stop" {
            self.parse_gradient_stop(attr);
        } else if el == b"defs" {
            self.defs_flag = true;
        } else if el == b"svg" {
            self.parse_svg(attr);
        } else if el == b"style" {
            self.style_flag = true;
        }
    }

    /// Translation of `nsvg__endElement()`.
    fn end_element(&mut self, el: &[u8]) {
        if el == b"g" {
            self.pop_attr();
        } else if el == b"path" {
            self.path_flag = false;
        } else if el == b"defs" {
            self.defs_flag = false;
        } else if el == b"style" {
            self.style_flag = false;
        }
    }

    /// Translation of `nsvg__content()`.
    fn content(&mut self, s: &[u8]) {
        if !self.style_flag {
            return;
        }

        // Parse all the styles inside the style block. Each style's content will be later processed using nsvg__parseStyle().
        // Note: We only support selector lists of simple class selectors (e.g. ".foo, .bar { ... }").
        let mut s = s;
        while !s.is_empty() {
            let mut styles: Vec<Vec<u8>> = Vec::new();

            // 1) Parse the selector list up to '{'. For each simple class selector ('.name'),
            //    allocate a new NSVGstyleDeclaration into the local staging array. Styles are
            //    only committed to p->styles in step 3 below, once their propertiesText has
            //    also been allocated successfully.
            while !s.is_empty() && s[0] != b'{' {
                while !s.is_empty() && (nsvg_isspace(s[0]) || s[0] == b',') {
                    s = &s[1..];
                }
                if s.is_empty() || s[0] == b'{' {
                    break;
                }

                let sel_len = s
                    .iter()
                    .position(|&c| nsvg_isspace(c) || c == b',' || c == b'{')
                    .unwrap_or(s.len());
                let sel = &s[..sel_len];
                s = &s[sel_len..];

                if sel[0] != b'.' || styles.len() >= NSVG_MAX_CLASSES {
                    continue; // unsupported selector, or staging array full
                }
                styles.push(sel[1..].to_vec()); // strip leading '.'
            }
            if s.is_empty() {
                // No '{' found - discard pending styles and stop.
                break;
            }
            s = &s[1..]; // advance past '{'

            // 2) Find the end of the properties block (up to '}').
            let props_len = s.iter().position(|&c| c == b'}').unwrap_or(s.len());
            let props = &s[..props_len];
            s = &s[props_len..];

            // 3) Allocate propertiesText for each pending style. Commit successful ones to
            //    p->styles (head-insertion); free any whose allocation fails.
            for class_name in styles {
                self.styles.push(NsvgStyleDeclaration {
                    class_name,
                    properties_text: props.to_vec(),
                });
            }

            if !s.is_empty() {
                s = &s[1..]; // advance past '}'
            }
        }
    }

    /// Translation of `nsvg__imageBounds()`.
    fn image_bounds(&self, bounds: &mut [f32; 4]) {
        let Some((first, rest)) = self.image.shapes.split_first() else {
            *bounds = [0.0; 4];
            return;
        };
        *bounds = first.bounds;
        for shape in rest {
            bounds[0] = nsvg_minf(bounds[0], shape.bounds[0]);
            bounds[1] = nsvg_minf(bounds[1], shape.bounds[1]);
            bounds[2] = nsvg_maxf(bounds[2], shape.bounds[2]);
            bounds[3] = nsvg_maxf(bounds[3], shape.bounds[3]);
        }
    }
}

/// Translation of `nsvg__viewAlign()`.
fn nsvg_view_align(content: f32, container: f32, type_: i32) -> f32 {
    if type_ == NSVG_ALIGN_MIN {
        0.0
    } else if type_ == NSVG_ALIGN_MAX {
        container - content
    } else {
        // mid
        (container - content) * 0.5
    }
}

/// Translation of `nsvg__scaleGradient()`.
fn nsvg_scale_gradient(grad: &mut NsvgGradient, tx: f32, ty: f32, sx: f32, sy: f32) {
    let mut t = [0.0f32; 6];
    nsvg_xform_set_translation(&mut t, tx, ty);
    nsvg_xform_multiply(&mut grad.xform, &t);

    nsvg_xform_set_scale(&mut t, sx, sy);
    nsvg_xform_multiply(&mut grad.xform, &t);
}

impl NsvgParser {
    /// Translation of `nsvg__scaleToViewbox()`.
    fn scale_to_viewbox(&mut self, units: &[u8]) {
        let mut bounds = [0.0f32; 4];

        // Guess image size if not set completely.
        self.image_bounds(&mut bounds);

        if self.view_width == 0.0 {
            if self.image.width > 0.0 {
                self.view_width = self.image.width;
            } else {
                self.view_minx = bounds[0];
                self.view_width = bounds[2] - bounds[0];
            }
        }
        if self.view_height == 0.0 {
            if self.image.height > 0.0 {
                self.view_height = self.image.height;
            } else {
                self.view_miny = bounds[1];
                self.view_height = bounds[3] - bounds[1];
            }
        }
        if self.image.width == 0.0 {
            self.image.width = self.view_width;
        }
        if self.image.height == 0.0 {
            self.image.height = self.view_height;
        }

        let mut tx = -self.view_minx;
        let mut ty = -self.view_miny;
        let mut sx = if self.view_width > 0.0 {
            self.image.width / self.view_width
        } else {
            0.0
        };
        let mut sy = if self.view_height > 0.0 {
            self.image.height / self.view_height
        } else {
            0.0
        };
        // Unit scaling
        let us = 1.0 / self.convert_to_pixels(nsvg_coord(1.0, nsvg_parse_units(units)), 0.0, 1.0);

        // Fix aspect ratio
        if self.align_type == NSVG_ALIGN_MEET {
            // fit whole image into viewbox
            sx = nsvg_minf(sx, sy);
            sy = sx;
            tx += nsvg_view_align(self.view_width * sx, self.image.width, self.align_x) / sx;
            ty += nsvg_view_align(self.view_height * sy, self.image.height, self.align_y) / sy;
        } else if self.align_type == NSVG_ALIGN_SLICE {
            // fill whole viewbox with image
            sx = nsvg_maxf(sx, sy);
            sy = sx;
            tx += nsvg_view_align(self.view_width * sx, self.image.width, self.align_x) / sx;
            ty += nsvg_view_align(self.view_height * sy, self.image.height, self.align_y) / sy;
        }

        // Transform
        sx *= us;
        sy *= us;
        let avgs = (sx + sy) / 2.0;
        for shape in &mut self.image.shapes {
            shape.bounds[0] = (shape.bounds[0] + tx) * sx;
            shape.bounds[1] = (shape.bounds[1] + ty) * sy;
            shape.bounds[2] = (shape.bounds[2] + tx) * sx;
            shape.bounds[3] = (shape.bounds[3] + ty) * sy;
            for path in &mut shape.paths {
                path.bounds[0] = (path.bounds[0] + tx) * sx;
                path.bounds[1] = (path.bounds[1] + ty) * sy;
                path.bounds[2] = (path.bounds[2] + tx) * sx;
                path.bounds[3] = (path.bounds[3] + ty) * sy;
                for i in 0..path.npts as usize {
                    let pt = &mut path.pts[i * 2..i * 2 + 2];
                    pt[0] = (pt[0] + tx) * sx;
                    pt[1] = (pt[1] + ty) * sy;
                }
            }

            for paint in [&mut shape.fill, &mut shape.stroke] {
                if paint.type_ == NSVG_PAINT_LINEAR_GRADIENT
                    || paint.type_ == NSVG_PAINT_RADIAL_GRADIENT
                {
                    if let Some(gradient) = paint.gradient.as_deref_mut() {
                        nsvg_scale_gradient(gradient, tx, ty, sx, sy);
                        let mut t = gradient.xform;
                        nsvg_xform_inverse(&mut gradient.xform, &mut t);
                    }
                }
            }

            shape.stroke_width *= avgs;
            shape.stroke_dash_offset *= avgs;
            for i in 0..shape.stroke_dash_count.max(0) as usize {
                shape.stroke_dash_array[i] *= avgs;
            }
        }
    }

    /// Translation of `nsvg__createGradients()`.
    fn create_gradients(&mut self) {
        for i in 0..self.image.shapes.len() {
            for stroke in [false, true] {
                let shape = &mut self.image.shapes[i];
                let (paint, gradient_id) = if stroke {
                    (&shape.stroke, shape.stroke_gradient)
                } else {
                    (&shape.fill, shape.fill_gradient)
                };
                if paint.type_ != NSVG_PAINT_UNDEF {
                    continue;
                }
                let mut type_ = paint.type_;
                let mut gradient = None;
                if gradient_id.0[0] != 0 {
                    // FIXME (upstream): for a singular transform the
                    // inverse is left uninitialized there (and the shape's
                    // transform reset); it is zeros here.
                    let mut inv = [0.0f32; 6];
                    let mut local_bounds = [0.0f32; 4];
                    nsvg_xform_inverse(&mut inv, &mut shape.xform);
                    nsvg_get_local_bounds(&mut local_bounds, shape, &inv);
                    let xform = shape.xform;
                    gradient =
                        self.create_gradient(&gradient_id.0, &local_bounds, &xform, &mut type_);
                }
                let shape = &mut self.image.shapes[i];
                let paint = if stroke {
                    &mut shape.stroke
                } else {
                    &mut shape.fill
                };
                paint.type_ = type_;
                paint.gradient = gradient;
                if paint.type_ == NSVG_PAINT_UNDEF {
                    paint.type_ = NSVG_PAINT_NONE;
                }
            }
        }
    }
}

/// Parses SVG file from a null terminated string, returns SVG image as
/// paths. Important note: changes the string. Translation of
/// `nsvgParse()`: `input` ends with a NUL.
pub(crate) fn nsvg_parse(input: &mut [u8], units: &[u8], dpi: f32) -> NsvgImage {
    let mut p = NsvgParser::new();
    p.dpi = dpi;

    nsvg_parse_xml(input, &mut p);

    // Create gradients after all definitions have been parsed
    p.create_gradients();

    // Scale to viewBox
    p.scale_to_viewbox(units);

    p.image
}
