// Rust translation of src/nanosvgrast.h from SDL_image (NanoSVG's
// rasterizer, https://github.com/memononen/nanosvg).
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
// The polygon rasterization is heavily based on stb_truetype rasterizer
// by Sean Barrett - http://nothings.org/

//! NanoSVG's rasterizer: shapes to RGBA pixels (non-premultiplied alpha).
//!
//! The active edges live in a vector instead of upstream's memory pages
//! (and their linked lists are indices into it); the integer arithmetic
//! wraps, as it does on the machines upstream runs on, and float to `int`
//! conversions out of range give `INT_MIN`, as x86's do.

// The translation keeps nanosvg's shapes: index loops, long parameter
// lists, its float comparisons.
#![allow(
    clippy::needless_range_loop,
    clippy::too_many_arguments,
    clippy::excessive_precision,
    clippy::approx_constant,
    clippy::assign_op_pattern,
    clippy::manual_clamp
)]

use std::cmp::Ordering;

use sdl3::stdlib::math::{acosf, atan2f, ceilf, cosf, floorf, fmodf, sinf, sqrtf};

use crate::nanosvg::{
    NsvgImage, NsvgPaint, NsvgShape, NSVG_CAP_BUTT, NSVG_CAP_ROUND, NSVG_CAP_SQUARE,
    NSVG_FILLRULE_EVENODD, NSVG_FILLRULE_NONZERO, NSVG_FLAGS_VISIBLE, NSVG_JOIN_BEVEL,
    NSVG_JOIN_ROUND, NSVG_PAINT_COLOR, NSVG_PAINT_LINEAR_GRADIENT, NSVG_PAINT_NONE,
    NSVG_PAINT_RADIAL_GRADIENT,
};
use crate::qsort::sdl_qsort;
use crate::util::c_f32_to_i32;

const NSVG_SUBSAMPLES: i32 = 5;
const NSVG_FIXSHIFT: i32 = 10;
const NSVG_FIX: i32 = 1 << NSVG_FIXSHIFT;
const NSVG_FIXMASK: i32 = NSVG_FIX - 1;

/// Translation of `NSVGedge`.
#[derive(Clone, Copy, Debug, Default)]
struct NsvgEdge {
    x0: f32,
    y0: f32,
    x1: f32,
    y1: f32,
    dir: i32,
}

/// Translation of `NSVGpoint`.
#[derive(Clone, Copy, Debug, Default)]
struct NsvgPoint {
    x: f32,
    y: f32,
    dx: f32,
    dy: f32,
    len: f32,
    dmx: f32,
    dmy: f32,
    flags: u8,
}

/// Translation of `NSVGactiveEdge` (`next` an index into the edges).
#[derive(Clone, Copy, Debug, Default)]
struct NsvgActiveEdge {
    x: i32,
    dx: i32,
    ey: f32,
    dir: i32,
    next: Option<usize>,
}

/// Translation of `NSVGcachedPaint`.
#[derive(Clone, Debug)]
struct NsvgCachedPaint {
    type_: i8,
    _spread: i8,
    xform: [f32; 6],
    colors: [u32; 256],
}

/// Translation of `struct NSVGrasterizer`.
#[derive(Debug, Default)]
pub(crate) struct NsvgRasterizer {
    tess_tol: f32,
    dist_tol: f32,

    edges: Vec<NsvgEdge>,

    points: Vec<NsvgPoint>,
    points2: Vec<NsvgPoint>,

    /// The active edges (upstream's memory pages).
    active_edges: Vec<NsvgActiveEdge>,
    freelist: Option<usize>,

    scanline: Vec<u8>,

    width: i32,
    height: i32,
    stride: i32,
}

/// Allocated rasterizer context. Translation of `nsvgCreateRasterizer()`.
pub(crate) fn nsvg_create_rasterizer() -> NsvgRasterizer {
    NsvgRasterizer {
        tess_tol: 0.25,
        dist_tol: 0.01,
        ..NsvgRasterizer::default()
    }
}

/// Translation of `nsvg__ptEquals()`.
fn nsvg_pt_equals(x1: f32, y1: f32, x2: f32, y2: f32, tol: f32) -> bool {
    let dx = x2 - x1;
    let dy = y2 - y1;
    dx * dx + dy * dy < tol * tol
}

impl NsvgRasterizer {
    /// Translation of `nsvg__resetPool()`.
    fn reset_pool(&mut self) {
        self.active_edges.clear();
    }

    /// Translation of `nsvg__addPathPoint()`.
    fn add_path_point(&mut self, x: f32, y: f32, flags: u8) {
        if let Some(pt) = self.points.last_mut() {
            if nsvg_pt_equals(pt.x, pt.y, x, y, self.dist_tol) {
                pt.flags |= flags;
                return;
            }
        }

        self.points.push(NsvgPoint {
            x,
            y,
            flags,
            ..NsvgPoint::default()
        });
    }

    /// Translation of `nsvg__appendPathPoint()`.
    fn append_path_point(&mut self, pt: NsvgPoint) {
        self.points.push(pt);
    }

    /// Translation of `nsvg__duplicatePoints()`.
    fn duplicate_points(&mut self) {
        self.points2.clear();
        self.points2.extend_from_slice(&self.points);
    }

    /// Translation of `nsvg__addEdge()`.
    fn add_edge(&mut self, x0: f32, y0: f32, x1: f32, y1: f32) {
        // Skip horizontal edges
        if y0 == y1 {
            return;
        }

        self.edges.push(if y0 < y1 {
            NsvgEdge {
                x0,
                y0,
                x1,
                y1,
                dir: 1,
            }
        } else {
            NsvgEdge {
                x0: x1,
                y0: y1,
                x1: x0,
                y1: y0,
                dir: -1,
            }
        });
    }
}

/// Translation of `nsvg__normalize()`.
fn nsvg_normalize(x: &mut f32, y: &mut f32) -> f32 {
    let d = sqrtf((*x) * (*x) + (*y) * (*y));
    if d > 1e-6 {
        let id = 1.0 / d;
        *x *= id;
        *y *= id;
    }
    d
}

/// Translation of `nsvg__absf()`.
fn nsvg_absf(x: f32) -> f32 {
    if x < 0.0 {
        -x
    } else {
        x
    }
}

/// Translation of `nsvg__roundf()`.
fn nsvg_roundf(x: f32) -> f32 {
    if x >= 0.0 {
        floorf(x + 0.5)
    } else {
        ceilf(x - 0.5)
    }
}

impl NsvgRasterizer {
    /// Translation of `nsvg__flattenCubicBez()`.
    fn flatten_cubic_bez(
        &mut self,
        x1: f32,
        y1: f32,
        x2: f32,
        y2: f32,
        x3: f32,
        y3: f32,
        x4: f32,
        y4: f32,
        mut level: i32,
        type_: u8,
    ) {
        const MAX_LEVEL: i32 = 10;

        if level > MAX_LEVEL {
            return;
        }

        let x12 = (x1 + x2) * 0.5;
        let y12 = (y1 + y2) * 0.5;
        let x23 = (x2 + x3) * 0.5;
        let y23 = (y2 + y3) * 0.5;
        let x34 = (x3 + x4) * 0.5;
        let y34 = (y3 + y4) * 0.5;
        let x123 = (x12 + x23) * 0.5;
        let y123 = (y12 + y23) * 0.5;

        let dx = x4 - x1;
        let dy = y4 - y1;
        let d2 = nsvg_absf((x2 - x4) * dy - (y2 - y4) * dx);
        let d3 = nsvg_absf((x3 - x4) * dy - (y3 - y4) * dx);

        if (d2 + d3) * (d2 + d3) < self.tess_tol * (dx * dx + dy * dy) {
            self.add_path_point(x4, y4, type_);
            return;
        }
        level += 1;
        if level > MAX_LEVEL {
            return;
        }

        let x234 = (x23 + x34) * 0.5;
        let y234 = (y23 + y34) * 0.5;
        let x1234 = (x123 + x234) * 0.5;
        let y1234 = (y123 + y234) * 0.5;

        self.flatten_cubic_bez(x1, y1, x12, y12, x123, y123, x1234, y1234, level, 0);
        self.flatten_cubic_bez(x1234, y1234, x234, y234, x34, y34, x4, y4, level, type_);
    }

    /// Translation of `nsvg__flattenShape()`.
    fn flatten_shape(&mut self, shape: &NsvgShape, scale: f32) {
        for path in &shape.paths {
            self.points.clear();
            // Flatten path
            self.add_path_point(path.pts[0] * scale, path.pts[1] * scale, 0);
            let mut i = 0;
            while i < path.npts - 1 {
                let p = &path.pts[i as usize * 2..];
                self.flatten_cubic_bez(
                    p[0] * scale,
                    p[1] * scale,
                    p[2] * scale,
                    p[3] * scale,
                    p[4] * scale,
                    p[5] * scale,
                    p[6] * scale,
                    p[7] * scale,
                    0,
                    0,
                );
                i += 3;
            }
            // Close path
            self.add_path_point(path.pts[0] * scale, path.pts[1] * scale, 0);
            // Build edges
            let n = self.points.len();
            let mut j = n.wrapping_sub(1);
            for i in 0..n {
                let (a, b) = (self.points[j], self.points[i]);
                self.add_edge(a.x, a.y, b.x, b.y);
                j = i;
            }
        }
    }
}

// NSVGpointFlags
const NSVG_PT_CORNER: u8 = 0x01;
const NSVG_PT_BEVEL: u8 = 0x02;
const NSVG_PT_LEFT: u8 = 0x04;

/// Translation of `nsvg__initClosed()`.
fn nsvg_init_closed(
    left: &mut NsvgPoint,
    right: &mut NsvgPoint,
    p0: &NsvgPoint,
    p1: &NsvgPoint,
    line_width: f32,
) {
    let w = line_width * 0.5;
    let mut dx = p1.x - p0.x;
    let mut dy = p1.y - p0.y;
    let len = nsvg_normalize(&mut dx, &mut dy);
    let px = p0.x + dx * len * 0.5;
    let py = p0.y + dy * len * 0.5;
    let dlx = dy;
    let dly = -dx;
    let lx = px - dlx * w;
    let ly = py - dly * w;
    let rx = px + dlx * w;
    let ry = py + dly * w;
    left.x = lx;
    left.y = ly;
    right.x = rx;
    right.y = ry;
}

impl NsvgRasterizer {
    /// Translation of `nsvg__buttCap()`.
    fn butt_cap(
        &mut self,
        left: &mut NsvgPoint,
        right: &mut NsvgPoint,
        p: &NsvgPoint,
        dx: f32,
        dy: f32,
        line_width: f32,
        connect: bool,
    ) {
        let w = line_width * 0.5;
        let px = p.x;
        let py = p.y;
        let dlx = dy;
        let dly = -dx;
        let lx = px - dlx * w;
        let ly = py - dly * w;
        let rx = px + dlx * w;
        let ry = py + dly * w;

        self.add_edge(lx, ly, rx, ry);

        if connect {
            self.add_edge(left.x, left.y, lx, ly);
            self.add_edge(rx, ry, right.x, right.y);
        }
        left.x = lx;
        left.y = ly;
        right.x = rx;
        right.y = ry;
    }

    /// Translation of `nsvg__squareCap()`.
    fn square_cap(
        &mut self,
        left: &mut NsvgPoint,
        right: &mut NsvgPoint,
        p: &NsvgPoint,
        dx: f32,
        dy: f32,
        line_width: f32,
        connect: bool,
    ) {
        let w = line_width * 0.5;
        let px = p.x - dx * w;
        let py = p.y - dy * w;
        let dlx = dy;
        let dly = -dx;
        let lx = px - dlx * w;
        let ly = py - dly * w;
        let rx = px + dlx * w;
        let ry = py + dly * w;

        self.add_edge(lx, ly, rx, ry);

        if connect {
            self.add_edge(left.x, left.y, lx, ly);
            self.add_edge(rx, ry, right.x, right.y);
        }
        left.x = lx;
        left.y = ly;
        right.x = rx;
        right.y = ry;
    }
}

const NSVG_PI: f32 = 3.14159265358979323846264338327;

impl NsvgRasterizer {
    /// Translation of `nsvg__roundCap()`.
    fn round_cap(
        &mut self,
        left: &mut NsvgPoint,
        right: &mut NsvgPoint,
        p: &NsvgPoint,
        dx: f32,
        dy: f32,
        line_width: f32,
        ncap: i32,
        connect: bool,
    ) {
        let w = line_width * 0.5;
        let px = p.x;
        let py = p.y;
        let dlx = dy;
        let dly = -dx;
        let (mut lx, mut ly, mut rx, mut ry, mut prevx, mut prevy) =
            (0.0f32, 0.0, 0.0, 0.0, 0.0, 0.0);

        for i in 0..ncap {
            let a = i as f32 / (ncap - 1) as f32 * NSVG_PI;
            let ax = cosf(a) * w;
            let ay = sinf(a) * w;
            let x = px - dlx * ax - dx * ay;
            let y = py - dly * ax - dy * ay;

            if i > 0 {
                self.add_edge(prevx, prevy, x, y);
            }

            prevx = x;
            prevy = y;

            if i == 0 {
                lx = x;
                ly = y;
            } else if i == ncap - 1 {
                rx = x;
                ry = y;
            }
        }

        if connect {
            self.add_edge(left.x, left.y, lx, ly);
            self.add_edge(rx, ry, right.x, right.y);
        }

        left.x = lx;
        left.y = ly;
        right.x = rx;
        right.y = ry;
    }

    /// Translation of `nsvg__bevelJoin()`.
    fn bevel_join(
        &mut self,
        left: &mut NsvgPoint,
        right: &mut NsvgPoint,
        p0: &NsvgPoint,
        p1: &NsvgPoint,
        line_width: f32,
    ) {
        let w = line_width * 0.5;
        let dlx0 = p0.dy;
        let dly0 = -p0.dx;
        let dlx1 = p1.dy;
        let dly1 = -p1.dx;
        let lx0 = p1.x - (dlx0 * w);
        let ly0 = p1.y - (dly0 * w);
        let rx0 = p1.x + (dlx0 * w);
        let ry0 = p1.y + (dly0 * w);
        let lx1 = p1.x - (dlx1 * w);
        let ly1 = p1.y - (dly1 * w);
        let rx1 = p1.x + (dlx1 * w);
        let ry1 = p1.y + (dly1 * w);

        self.add_edge(lx0, ly0, left.x, left.y);
        self.add_edge(lx1, ly1, lx0, ly0);

        self.add_edge(right.x, right.y, rx0, ry0);
        self.add_edge(rx0, ry0, rx1, ry1);

        left.x = lx1;
        left.y = ly1;
        right.x = rx1;
        right.y = ry1;
    }

    /// Translation of `nsvg__miterJoin()`.
    fn miter_join(
        &mut self,
        left: &mut NsvgPoint,
        right: &mut NsvgPoint,
        p0: &NsvgPoint,
        p1: &NsvgPoint,
        line_width: f32,
    ) {
        let w = line_width * 0.5;
        let dlx0 = p0.dy;
        let dly0 = -p0.dx;
        let dlx1 = p1.dy;
        let dly1 = -p1.dx;
        let (lx1, ly1, rx1, ry1);

        if p1.flags & NSVG_PT_LEFT != 0 {
            lx1 = p1.x - p1.dmx * w;
            ly1 = p1.y - p1.dmy * w;
            self.add_edge(lx1, ly1, left.x, left.y);

            let rx0 = p1.x + (dlx0 * w);
            let ry0 = p1.y + (dly0 * w);
            rx1 = p1.x + (dlx1 * w);
            ry1 = p1.y + (dly1 * w);
            self.add_edge(right.x, right.y, rx0, ry0);
            self.add_edge(rx0, ry0, rx1, ry1);
        } else {
            let lx0 = p1.x - (dlx0 * w);
            let ly0 = p1.y - (dly0 * w);
            lx1 = p1.x - (dlx1 * w);
            ly1 = p1.y - (dly1 * w);
            self.add_edge(lx0, ly0, left.x, left.y);
            self.add_edge(lx1, ly1, lx0, ly0);

            rx1 = p1.x + p1.dmx * w;
            ry1 = p1.y + p1.dmy * w;
            self.add_edge(right.x, right.y, rx1, ry1);
        }

        left.x = lx1;
        left.y = ly1;
        right.x = rx1;
        right.y = ry1;
    }

    /// Translation of `nsvg__roundJoin()`.
    fn round_join(
        &mut self,
        left: &mut NsvgPoint,
        right: &mut NsvgPoint,
        p0: &NsvgPoint,
        p1: &NsvgPoint,
        line_width: f32,
        ncap: i32,
    ) {
        let w = line_width * 0.5;
        let dlx0 = p0.dy;
        let dly0 = -p0.dx;
        let dlx1 = p1.dy;
        let dly1 = -p1.dx;
        let a0 = atan2f(dly0, dlx0);
        let a1 = atan2f(dly1, dlx1);
        let mut da = a1 - a0;

        if da < NSVG_PI {
            da += NSVG_PI * 2.0;
        }
        if da > NSVG_PI {
            da -= NSVG_PI * 2.0;
        }

        let mut n = c_f32_to_i32(ceilf((nsvg_absf(da) / NSVG_PI) * ncap as f32));
        if n < 2 {
            n = 2;
        }
        if n > ncap {
            n = ncap;
        }

        let mut lx = left.x;
        let mut ly = left.y;
        let mut rx = right.x;
        let mut ry = right.y;

        for i in 0..n {
            let u = i as f32 / (n - 1) as f32;
            let a = a0 + u * da;
            let ax = cosf(a) * w;
            let ay = sinf(a) * w;
            let lx1 = p1.x - ax;
            let ly1 = p1.y - ay;
            let rx1 = p1.x + ax;
            let ry1 = p1.y + ay;

            self.add_edge(lx1, ly1, lx, ly);
            self.add_edge(rx, ry, rx1, ry1);

            lx = lx1;
            ly = ly1;
            rx = rx1;
            ry = ry1;
        }

        left.x = lx;
        left.y = ly;
        right.x = rx;
        right.y = ry;
    }

    /// Translation of `nsvg__straightJoin()`.
    fn straight_join(
        &mut self,
        left: &mut NsvgPoint,
        right: &mut NsvgPoint,
        p1: &NsvgPoint,
        line_width: f32,
    ) {
        let w = line_width * 0.5;
        let lx = p1.x - (p1.dmx * w);
        let ly = p1.y - (p1.dmy * w);
        let rx = p1.x + (p1.dmx * w);
        let ry = p1.y + (p1.dmy * w);

        self.add_edge(lx, ly, left.x, left.y);
        self.add_edge(right.x, right.y, rx, ry);

        left.x = lx;
        left.y = ly;
        right.x = rx;
        right.y = ry;
    }
}

/// Translation of `nsvg__curveDivs()`.
fn nsvg_curve_divs(r: f32, arc: f32, tol: f32) -> i32 {
    let da = acosf(r / (r + tol)) * 2.0;
    let mut divs = c_f32_to_i32(ceilf(arc / da));
    if divs < 2 {
        divs = 2;
    }
    divs
}

impl NsvgRasterizer {
    /// Translation of `nsvg__expandStroke()`: the points are
    /// `self.points`.
    fn expand_stroke(&mut self, closed: bool, line_join: i8, line_cap: i8, line_width: f32) {
        let points = std::mem::take(&mut self.points);
        let npoints = points.len();
        let ncap = nsvg_curve_divs(line_width * 0.5, NSVG_PI, self.tess_tol); // Calculate divisions per half circle.
        let mut left = NsvgPoint::default();
        let mut right = NsvgPoint::default();
        let mut first_left = NsvgPoint::default();
        let mut first_right = NsvgPoint::default();
        let (mut p0, mut p1, s, e);

        // Build stroke edges
        if closed {
            // Looping
            p0 = npoints - 1;
            p1 = 0;
            s = 0;
            e = npoints;
        } else {
            // Add cap
            p0 = 0;
            p1 = 1;
            s = 1;
            e = npoints - 1;
        }

        if closed {
            nsvg_init_closed(&mut left, &mut right, &points[p0], &points[p1], line_width);
            first_left = left;
            first_right = right;
        } else {
            // Add cap
            let mut dx = points[p1].x - points[p0].x;
            let mut dy = points[p1].y - points[p0].y;
            nsvg_normalize(&mut dx, &mut dy);
            if line_cap == NSVG_CAP_BUTT {
                self.butt_cap(
                    &mut left,
                    &mut right,
                    &points[p0],
                    dx,
                    dy,
                    line_width,
                    false,
                );
            } else if line_cap == NSVG_CAP_SQUARE {
                self.square_cap(
                    &mut left,
                    &mut right,
                    &points[p0],
                    dx,
                    dy,
                    line_width,
                    false,
                );
            } else if line_cap == NSVG_CAP_ROUND {
                self.round_cap(
                    &mut left,
                    &mut right,
                    &points[p0],
                    dx,
                    dy,
                    line_width,
                    ncap,
                    false,
                );
            }
        }

        for _j in s..e {
            let (a, b) = (points[p0], points[p1]);
            if b.flags & NSVG_PT_CORNER != 0 {
                if line_join == NSVG_JOIN_ROUND {
                    self.round_join(&mut left, &mut right, &a, &b, line_width, ncap);
                } else if line_join == NSVG_JOIN_BEVEL || (b.flags & NSVG_PT_BEVEL != 0) {
                    self.bevel_join(&mut left, &mut right, &a, &b, line_width);
                } else {
                    self.miter_join(&mut left, &mut right, &a, &b, line_width);
                }
            } else {
                self.straight_join(&mut left, &mut right, &b, line_width);
            }
            p0 = p1;
            p1 += 1;
        }

        if closed {
            // Loop it
            self.add_edge(first_left.x, first_left.y, left.x, left.y);
            self.add_edge(right.x, right.y, first_right.x, first_right.y);
        } else {
            // Add cap
            let mut dx = points[p1].x - points[p0].x;
            let mut dy = points[p1].y - points[p0].y;
            nsvg_normalize(&mut dx, &mut dy);
            if line_cap == NSVG_CAP_BUTT {
                self.butt_cap(
                    &mut right,
                    &mut left,
                    &points[p1],
                    -dx,
                    -dy,
                    line_width,
                    true,
                );
            } else if line_cap == NSVG_CAP_SQUARE {
                self.square_cap(
                    &mut right,
                    &mut left,
                    &points[p1],
                    -dx,
                    -dy,
                    line_width,
                    true,
                );
            } else if line_cap == NSVG_CAP_ROUND {
                self.round_cap(
                    &mut right,
                    &mut left,
                    &points[p1],
                    -dx,
                    -dy,
                    line_width,
                    ncap,
                    true,
                );
            }
        }
        self.points = points;
    }

    /// Translation of `nsvg__prepareStroke()`.
    fn prepare_stroke(&mut self, miter_limit: f32, line_join: i8) {
        let n = self.points.len();

        let mut p0 = n - 1;
        let mut p1 = 0;
        for _i in 0..n {
            // Calculate segment direction and length
            let (x0, y0) = (self.points[p0].x, self.points[p0].y);
            let (x1, y1) = (self.points[p1].x, self.points[p1].y);
            let p = &mut self.points[p0];
            p.dx = x1 - x0;
            p.dy = y1 - y0;
            p.len = nsvg_normalize(&mut p.dx, &mut p.dy);
            // Advance
            p0 = p1;
            p1 += 1;
        }

        // calculate joins
        p0 = n - 1;
        p1 = 0;
        for _j in 0..n {
            let a = self.points[p0];
            let b = &mut self.points[p1];
            let dlx0 = a.dy;
            let dly0 = -a.dx;
            let dlx1 = b.dy;
            let dly1 = -b.dx;
            // Calculate extrusions
            b.dmx = (dlx0 + dlx1) * 0.5;
            b.dmy = (dly0 + dly1) * 0.5;
            let dmr2 = b.dmx * b.dmx + b.dmy * b.dmy;
            if dmr2 > 0.000001 {
                let mut s2 = 1.0 / dmr2;
                if s2 > 600.0 {
                    s2 = 600.0;
                }
                b.dmx *= s2;
                b.dmy *= s2;
            }

            // Clear flags, but keep the corner.
            b.flags = if b.flags & NSVG_PT_CORNER != 0 {
                NSVG_PT_CORNER
            } else {
                0
            };

            // Keep track of left turns.
            let cross = b.dx * a.dy - a.dx * b.dy;
            if cross > 0.0 {
                b.flags |= NSVG_PT_LEFT;
            }

            // Check to see if the corner needs to be beveled.
            if b.flags & NSVG_PT_CORNER != 0
                && ((dmr2 * miter_limit * miter_limit) < 1.0
                    || line_join == NSVG_JOIN_BEVEL
                    || line_join == NSVG_JOIN_ROUND)
            {
                b.flags |= NSVG_PT_BEVEL;
            }

            p0 = p1;
            p1 += 1;
        }
    }

    /// Translation of `nsvg__flattenShapeStroke()`.
    fn flatten_shape_stroke(&mut self, shape: &NsvgShape, scale: f32) {
        let miter_limit = shape.miter_limit;
        let line_join = shape.stroke_line_join;
        let line_cap = shape.stroke_line_cap;
        let line_width = shape.stroke_width * scale;

        for path in &shape.paths {
            // Flatten path
            self.points.clear();
            self.add_path_point(path.pts[0] * scale, path.pts[1] * scale, NSVG_PT_CORNER);
            let mut i = 0;
            while i < path.npts - 1 {
                let p = &path.pts[i as usize * 2..];
                self.flatten_cubic_bez(
                    p[0] * scale,
                    p[1] * scale,
                    p[2] * scale,
                    p[3] * scale,
                    p[4] * scale,
                    p[5] * scale,
                    p[6] * scale,
                    p[7] * scale,
                    0,
                    NSVG_PT_CORNER,
                );
                i += 3;
            }
            if self.points.len() < 2 {
                continue;
            }

            let mut closed = path.closed;

            // If the first and last points are the same, remove the last, mark as closed path.
            let p0 = self.points[self.points.len() - 1];
            let p1 = self.points[0];
            if nsvg_pt_equals(p0.x, p0.y, p1.x, p1.y, self.dist_tol) {
                self.points.pop();
                closed = true;
            }

            if shape.stroke_dash_count > 0 {
                let mut idash = 0usize;
                let mut dash_state = true;
                let mut total_dist = 0.0f32;
                let count = shape.stroke_dash_count as usize;

                if closed {
                    let first = self.points[0];
                    self.append_path_point(first);
                }

                // Duplicate points -> points2.
                self.duplicate_points();

                self.points.clear();
                let mut cur = self.points2[0];
                self.append_path_point(cur);

                // Figure out dash offset.
                let mut all_dash_len = 0.0f32;
                for j in 0..count {
                    all_dash_len += shape.stroke_dash_array[j];
                }
                if count & 1 != 0 {
                    all_dash_len *= 2.0;
                }
                // Find location inside pattern
                let mut dash_offset = fmodf(shape.stroke_dash_offset, all_dash_len);
                if dash_offset < 0.0 {
                    dash_offset += all_dash_len;
                }

                // FIXME (upstream): for an odd dash count the pattern is
                // twice as long as the dashes, but the dashes are indexed
                // modulo their count; a NaN offset never ends the walk
                // through them there, and leaves it as it is here.
                while dash_offset > shape.stroke_dash_array[idash] {
                    dash_offset -= shape.stroke_dash_array[idash];
                    idash = (idash + 1) % count;
                }
                let mut dash_len = (shape.stroke_dash_array[idash] - dash_offset) * scale;

                let mut j = 1;
                while j < self.points2.len() {
                    let dx = self.points2[j].x - cur.x;
                    let dy = self.points2[j].y - cur.y;
                    let dist = sqrtf(dx * dx + dy * dy);

                    if (total_dist + dist) > dash_len {
                        // Calculate intermediate point
                        let d = (dash_len - total_dist) / dist;
                        let x = cur.x + dx * d;
                        let y = cur.y + dy * d;
                        self.add_path_point(x, y, NSVG_PT_CORNER);

                        // Stroke
                        if self.points.len() > 1 && dash_state {
                            self.prepare_stroke(miter_limit, line_join);
                            self.expand_stroke(false, line_join, line_cap, line_width);
                        }
                        // Advance dash pattern
                        dash_state = !dash_state;
                        idash = (idash + 1) % count;
                        dash_len = shape.stroke_dash_array[idash] * scale;
                        // Restart
                        cur.x = x;
                        cur.y = y;
                        cur.flags = NSVG_PT_CORNER;
                        total_dist = 0.0;
                        self.points.clear();
                        self.append_path_point(cur);
                    } else {
                        total_dist += dist;
                        cur = self.points2[j];
                        self.append_path_point(cur);
                        j += 1;
                    }
                }
                // Stroke any leftover path
                if self.points.len() > 1 && dash_state {
                    self.prepare_stroke(miter_limit, line_join);
                    self.expand_stroke(false, line_join, line_cap, line_width);
                }
            } else {
                self.prepare_stroke(miter_limit, line_join);
                self.expand_stroke(closed, line_join, line_cap, line_width);
            }
        }
    }
}

/// Translation of `nsvg__cmpEdge()`.
fn nsvg_cmp_edge(a: &NsvgEdge, b: &NsvgEdge) -> Ordering {
    if a.y0 < b.y0 {
        return Ordering::Less;
    }
    if a.y0 > b.y0 {
        return Ordering::Greater;
    }
    Ordering::Equal
}

impl NsvgRasterizer {
    /// Translation of `nsvg__addActive()`.
    fn add_active(&mut self, e: &NsvgEdge, start_point: f32) -> usize {
        let z = if let Some(z) = self.freelist {
            // Restore from freelist.
            self.freelist = self.active_edges[z].next;
            z
        } else {
            // Alloc new edge.
            self.active_edges.push(NsvgActiveEdge::default());
            self.active_edges.len() - 1
        };

        let dxdy = (e.x1 - e.x0) / (e.y1 - e.y0);
        //	STBTT_assert(e->y0 <= start_point);
        // round dx down to avoid going too far
        let dx = if dxdy < 0.0 {
            c_f32_to_i32(-nsvg_roundf(NSVG_FIX as f32 * -dxdy))
        } else {
            c_f32_to_i32(nsvg_roundf(NSVG_FIX as f32 * dxdy))
        };
        let x = c_f32_to_i32(nsvg_roundf(
            NSVG_FIX as f32 * (e.x0 + dxdy * (start_point - e.y0)),
        ));
        //	z->x -= off_x * FIX;
        self.active_edges[z] = NsvgActiveEdge {
            x,
            dx,
            ey: e.y1,
            next: None,
            dir: e.dir,
        };

        z
    }

    /// Translation of `nsvg__freeActive()`.
    fn free_active(&mut self, z: usize) {
        self.active_edges[z].next = self.freelist;
        self.freelist = Some(z);
    }
}

/// Translation of `nsvg__fillScanline()`.
fn nsvg_fill_scanline(
    scanline: &mut [u8],
    len: i32,
    x0: i32,
    x1: i32,
    max_weight: i32,
    xmin: &mut i32,
    xmax: &mut i32,
) {
    let mut i = x0 >> NSVG_FIXSHIFT;
    let mut j = x1 >> NSVG_FIXSHIFT;
    if i < *xmin {
        *xmin = i;
    }
    if j > *xmax {
        *xmax = j;
    }
    let add = |s: &mut u8, v: i32| *s = (*s as i32).wrapping_add(v) as u8;
    if i < len && j >= 0 {
        if i == j {
            // x0,x1 are the same pixel, so compute combined coverage
            add(
                &mut scanline[i as usize],
                x1.wrapping_sub(x0).wrapping_mul(max_weight) >> NSVG_FIXSHIFT,
            );
        } else {
            if i >= 0 {
                // add antialiasing for x0
                add(
                    &mut scanline[i as usize],
                    ((NSVG_FIX - (x0 & NSVG_FIXMASK)) * max_weight) >> NSVG_FIXSHIFT,
                );
            } else {
                i = -1; // clip
            }

            if j < len {
                // add antialiasing for x1
                add(
                    &mut scanline[j as usize],
                    ((x1 & NSVG_FIXMASK) * max_weight) >> NSVG_FIXSHIFT,
                );
            } else {
                j = len; // clip
            }

            i += 1;
            while i < j {
                // fill pixels between x0 and x1
                add(&mut scanline[i as usize], max_weight);
                i += 1;
            }
        }
    }
}

impl NsvgRasterizer {
    // note: this routine clips fills that extend off the edges... ideally this
    // wouldn't happen, but it could happen if the truetype glyph bounding boxes
    // are wrong, or if the user supplies a too-small bitmap
    /// Translation of `nsvg__fillActiveEdges()`.
    fn fill_active_edges(
        &mut self,
        mut e: Option<usize>,
        max_weight: i32,
        xmin: &mut i32,
        xmax: &mut i32,
        fill_rule: i8,
    ) {
        // non-zero winding fill
        let mut x0 = 0;
        let mut w = 0i32;
        let len = self.width;

        if fill_rule == NSVG_FILLRULE_NONZERO {
            // Non-zero
            while let Some(ei) = e {
                let edge = self.active_edges[ei];
                if w == 0 {
                    // if we're currently at zero, we need to record the edge start point
                    x0 = edge.x;
                    w = w.wrapping_add(edge.dir);
                } else {
                    let x1 = edge.x;
                    w = w.wrapping_add(edge.dir);
                    // if we went to zero, we need to draw
                    if w == 0 {
                        nsvg_fill_scanline(&mut self.scanline, len, x0, x1, max_weight, xmin, xmax);
                    }
                }
                e = edge.next;
            }
        } else if fill_rule == NSVG_FILLRULE_EVENODD {
            // Even-odd
            while let Some(ei) = e {
                let edge = self.active_edges[ei];
                if w == 0 {
                    // if we're currently at zero, we need to record the edge start point
                    x0 = edge.x;
                    w = 1;
                } else {
                    let x1 = edge.x;
                    w = 0;
                    nsvg_fill_scanline(&mut self.scanline, len, x0, x1, max_weight, xmin, xmax);
                }
                e = edge.next;
            }
        }
    }
}

/// Translation of `nsvg__clampf()`.
fn nsvg_clampf(a: f32, mn: f32, mx: f32) -> f32 {
    if a.is_nan() {
        return mn;
    }
    if a < mn {
        mn
    } else if a > mx {
        mx
    } else {
        a
    }
}

/// Translation of `nsvg__RGBA()`.
fn nsvg_rgba(r: u8, g: u8, b: u8, a: u8) -> u32 {
    (r as u32) | ((g as u32) << 8) | ((b as u32) << 16) | ((a as u32) << 24)
}

/// Translation of `nsvg__lerpRGBA()`.
fn nsvg_lerp_rgba(c0: u32, c1: u32, u: f32) -> u32 {
    let iu = (nsvg_clampf(u, 0.0, 1.0) * 256.0) as i32;
    let ch = |c: u32, s: u32| ((c >> s) & 0xff) as i32;
    let r = (ch(c0, 0) * (256 - iu) + (ch(c1, 0) * iu)) >> 8;
    let g = (ch(c0, 8) * (256 - iu) + (ch(c1, 8) * iu)) >> 8;
    let b = (ch(c0, 16) * (256 - iu) + (ch(c1, 16) * iu)) >> 8;
    let a = (ch(c0, 24) * (256 - iu) + (ch(c1, 24) * iu)) >> 8;
    nsvg_rgba(r as u8, g as u8, b as u8, a as u8)
}

/// Translation of `nsvg__applyOpacity()`.
fn nsvg_apply_opacity(c: u32, u: f32) -> u32 {
    let iu = (nsvg_clampf(u, 0.0, 1.0) * 256.0) as i32;
    let r = (c & 0xff) as i32;
    let g = ((c >> 8) & 0xff) as i32;
    let b = ((c >> 16) & 0xff) as i32;
    let a = ((((c >> 24) & 0xff) as i32) * iu) >> 8;
    nsvg_rgba(r as u8, g as u8, b as u8, a as u8)
}

/// Translation of `nsvg__div255()`.
fn nsvg_div255(x: i32) -> i32 {
    ((x + 1) * 257) >> 16
}

/// Blend one pixel of a color with its coverage over `dst`.
fn nsvg_blend(dst: &mut [u8], cover: u8, c: u32) {
    let cr = (c & 0xff) as i32;
    let cg = ((c >> 8) & 0xff) as i32;
    let cb = ((c >> 16) & 0xff) as i32;
    let ca = ((c >> 24) & 0xff) as i32;

    let mut a = nsvg_div255(cover as i32 * ca);
    let ia = 255 - a;

    // Premultiply
    let mut r = nsvg_div255(cr * a);
    let mut g = nsvg_div255(cg * a);
    let mut b = nsvg_div255(cb * a);

    // Blend over
    r += nsvg_div255(ia * dst[0] as i32);
    g += nsvg_div255(ia * dst[1] as i32);
    b += nsvg_div255(ia * dst[2] as i32);
    a += nsvg_div255(ia * dst[3] as i32);

    dst[0] = r as u8;
    dst[1] = g as u8;
    dst[2] = b as u8;
    dst[3] = a as u8;
}

/// Translation of `nsvg__scanlineSolid()`.
fn nsvg_scanline_solid(
    dst: &mut [u8],
    count: i32,
    cover: &[u8],
    x: i32,
    y: i32,
    tx: f32,
    ty: f32,
    scale: f32,
    cache: &NsvgCachedPaint,
) {
    if cache.type_ == NSVG_PAINT_COLOR {
        for i in 0..count as usize {
            nsvg_blend(&mut dst[i * 4..i * 4 + 4], cover[i], cache.colors[0]);
        }
    } else if cache.type_ == NSVG_PAINT_LINEAR_GRADIENT {
        // TODO: spread modes.
        // TODO: plenty of opportunities to optimize.
        let t = &cache.xform;

        let mut fx = (x as f32 - tx) / scale;
        let fy = (y as f32 - ty) / scale;
        let dx = 1.0 / scale;

        for i in 0..count as usize {
            let gy = fx * t[1] + fy * t[3] + t[5];
            let c = cache.colors[nsvg_clampf(gy * 255.0, 0.0, 255.0) as i32 as usize];
            nsvg_blend(&mut dst[i * 4..i * 4 + 4], cover[i], c);
            fx += dx;
        }
    } else if cache.type_ == NSVG_PAINT_RADIAL_GRADIENT {
        // TODO: spread modes.
        // TODO: plenty of opportunities to optimize.
        // TODO: focus (fx,fy)
        let t = &cache.xform;

        let mut fx = (x as f32 - tx) / scale;
        let fy = (y as f32 - ty) / scale;
        let dx = 1.0 / scale;

        for i in 0..count as usize {
            let gx = fx * t[0] + fy * t[2] + t[4];
            let gy = fx * t[1] + fy * t[3] + t[5];
            let gd = sqrtf(gx * gx + gy * gy);
            let c = cache.colors[nsvg_clampf(gd * 255.0, 0.0, 255.0) as i32 as usize];
            nsvg_blend(&mut dst[i * 4..i * 4 + 4], cover[i], c);
            fx += dx;
        }
    }
}

/// Where an active edge's link is: the list head, or an edge's `next`.
#[derive(Clone, Copy)]
enum Link {
    Head,
    Next(usize),
}

impl NsvgRasterizer {
    fn link(&self, active: Option<usize>, l: Link) -> Option<usize> {
        match l {
            Link::Head => active,
            Link::Next(i) => self.active_edges[i].next,
        }
    }

    fn set_link(&mut self, active: &mut Option<usize>, l: Link, v: Option<usize>) {
        match l {
            Link::Head => *active = v,
            Link::Next(i) => self.active_edges[i].next = v,
        }
    }

    /// Translation of `nsvg__rasterizeSortedEdges()`.
    fn rasterize_sorted_edges(
        &mut self,
        bitmap: &mut [u8],
        tx: f32,
        ty: f32,
        scale: f32,
        cache: &NsvgCachedPaint,
        fill_rule: i8,
    ) {
        let mut active: Option<usize> = None;
        let mut e = 0usize;
        let max_weight = 255 / NSVG_SUBSAMPLES; // weight per vertical scanline

        for y in 0..self.height {
            self.scanline[..self.width as usize].fill(0);
            let mut xmin = self.width;
            let mut xmax = 0;
            for s in 0..NSVG_SUBSAMPLES {
                // find center of pixel for this scanline
                let scany = (y * NSVG_SUBSAMPLES + s) as f32 + 0.5;
                let mut step = Link::Head;

                // update all active edges;
                // remove all active edges that terminate before the center of this scanline
                while let Some(z) = self.link(active, step) {
                    if self.active_edges[z].ey <= scany {
                        let next = self.active_edges[z].next;
                        self.set_link(&mut active, step, next); // delete from list
                                                                //					NSVG__assert(z->valid);
                        self.free_active(z);
                    } else {
                        let edge = &mut self.active_edges[z];
                        edge.x = edge.x.wrapping_add(edge.dx); // advance to position for current scanline
                        step = Link::Next(z); // advance through list
                    }
                }

                // resort the list if needed
                loop {
                    let mut changed = false;
                    step = Link::Head;
                    while let Some(t) = self.link(active, step) {
                        let Some(q) = self.active_edges[t].next else {
                            break;
                        };
                        if self.active_edges[t].x > self.active_edges[q].x {
                            self.active_edges[t].next = self.active_edges[q].next;
                            self.active_edges[q].next = Some(t);
                            self.set_link(&mut active, step, Some(q));
                            changed = true;
                        }
                        step = Link::Next(self.link(active, step).unwrap_or(t));
                    }
                    if !changed {
                        break;
                    }
                }

                // insert all edges that start before the center of this scanline -- omit ones that also end on this scanline
                while e < self.edges.len() && self.edges[e].y0 <= scany {
                    if self.edges[e].y1 > scany {
                        let edge = self.edges[e];
                        let z = self.add_active(&edge, scany);
                        let zx = self.active_edges[z].x;
                        // find insertion point
                        match active {
                            None => active = Some(z),
                            Some(head) if zx < self.active_edges[head].x => {
                                // insert at front
                                self.active_edges[z].next = Some(head);
                                active = Some(z);
                            }
                            Some(head) => {
                                // find thing to insert AFTER
                                let mut p = head;
                                while let Some(n) = self.active_edges[p].next {
                                    if self.active_edges[n].x < zx {
                                        p = n;
                                    } else {
                                        break;
                                    }
                                }
                                // at this point, p->next->x is NOT < z->x
                                self.active_edges[z].next = self.active_edges[p].next;
                                self.active_edges[p].next = Some(z);
                            }
                        }
                    }
                    e += 1;
                }

                // now process all active edges in non-zero fashion
                if active.is_some() {
                    self.fill_active_edges(active, max_weight, &mut xmin, &mut xmax, fill_rule);
                }
            }
            // Blit
            if xmin < 0 {
                xmin = 0;
            }
            if xmax > self.width - 1 {
                xmax = self.width - 1;
            }
            if xmin <= xmax {
                let row = y as usize * self.stride as usize + xmin as usize * 4;
                nsvg_scanline_solid(
                    &mut bitmap[row..],
                    xmax - xmin + 1,
                    &self.scanline[xmin as usize..],
                    xmin,
                    y,
                    tx,
                    ty,
                    scale,
                    cache,
                );
            }
        }
    }
}

/// Translation of `nsvg__unpremultiplyAlpha()`.
fn nsvg_unpremultiply_alpha(image: &mut [u8], w: i32, h: i32, stride: i32) {
    let stride = stride as usize;

    // Unpremultiply
    for y in 0..h as usize {
        for x in 0..w as usize {
            let row = y * stride + x * 4;
            let (r, g, b, a) = (
                image[row] as i32,
                image[row + 1] as i32,
                image[row + 2] as i32,
                image[row + 3] as i32,
            );
            if a != 0 {
                image[row] = (r * 255 / a) as u8;
                image[row + 1] = (g * 255 / a) as u8;
                image[row + 2] = (b * 255 / a) as u8;
            }
        }
    }

    // Defringe
    for y in 0..h {
        for x in 0..w {
            let row = y as usize * stride + x as usize * 4;
            let (mut r, mut g, mut b, a, mut n) = (0i32, 0i32, 0i32, image[row + 3], 0i32);
            if a == 0 {
                if x - 1 > 0 && image[row - 1] != 0 {
                    r += image[row - 4] as i32;
                    g += image[row - 3] as i32;
                    b += image[row - 2] as i32;
                    n += 1;
                }
                if x + 1 < w && image[row + 7] != 0 {
                    r += image[row + 4] as i32;
                    g += image[row + 5] as i32;
                    b += image[row + 6] as i32;
                    n += 1;
                }
                if y - 1 > 0 && image[row - stride + 3] != 0 {
                    r += image[row - stride] as i32;
                    g += image[row - stride + 1] as i32;
                    b += image[row - stride + 2] as i32;
                    n += 1;
                }
                if y + 1 < h && image[row + stride + 3] != 0 {
                    r += image[row + stride] as i32;
                    g += image[row + stride + 1] as i32;
                    b += image[row + stride + 2] as i32;
                    n += 1;
                }
                if n > 0 {
                    image[row] = (r / n) as u8;
                    image[row + 1] = (g / n) as u8;
                    image[row + 2] = (b / n) as u8;
                }
            }
        }
    }
}

/// Translation of `nsvg__initPaint()`.
fn nsvg_init_paint(cache: &mut NsvgCachedPaint, paint: &NsvgPaint, opacity: f32) {
    cache.type_ = paint.type_;

    if paint.type_ == NSVG_PAINT_COLOR {
        cache.colors[0] = nsvg_apply_opacity(paint.color, opacity);
        return;
    }

    let Some(grad) = paint.gradient.as_deref() else {
        return;
    };

    cache._spread = grad.spread;
    cache.xform = grad.xform;

    let nstops = grad.stops.len();
    if nstops == 0 {
        cache.colors = [0; 256];
    } else if nstops == 1 {
        let color = nsvg_apply_opacity(grad.stops[0].color, opacity);
        cache.colors = [color; 256];
    } else {
        let mut cb = 0;

        let mut ca = nsvg_apply_opacity(grad.stops[0].color, opacity);
        let ua = nsvg_clampf(grad.stops[0].offset, 0.0, 1.0);
        let ub = nsvg_clampf(grad.stops[nstops - 1].offset, ua, 1.0);
        let mut ia = (ua * 255.0) as i32;
        let mut ib = (ub * 255.0) as i32;
        for i in 0..ia.max(0) as usize {
            cache.colors[i] = ca;
        }

        for i in 0..nstops - 1 {
            ca = nsvg_apply_opacity(grad.stops[i].color, opacity);
            cb = nsvg_apply_opacity(grad.stops[i + 1].color, opacity);
            let ua = nsvg_clampf(grad.stops[i].offset, 0.0, 1.0);
            let ub = nsvg_clampf(grad.stops[i + 1].offset, 0.0, 1.0);
            ia = (ua * 255.0) as i32;
            ib = (ub * 255.0) as i32;
            let count = ib - ia;
            if count <= 0 {
                continue;
            }
            let mut u = 0.0f32;
            let du = 1.0 / count as f32;
            for j in 0..count {
                cache.colors[(ia + j) as usize] = nsvg_lerp_rgba(ca, cb, u);
                u += du;
            }
        }

        for i in ib.max(0) as usize..256 {
            cache.colors[i] = cb;
        }
    }
}

/// Rasterizes SVG image, returns RGBA image (non-premultiplied alpha)
///   r - pointer to rasterizer context
///   image - pointer to image to rasterize
///   tx,ty - image offset (applied after scaling)
///   scale - image scale
///   dst - pointer to destination image data, 4 bytes per pixel (RGBA)
///   w - width of the image to render
///   h - height of the image to render
///   stride - number of bytes per scaleline in the destination buffer
///
/// Translation of `nsvgRasterize()`.
pub(crate) fn nsvg_rasterize(
    r: &mut NsvgRasterizer,
    image: &NsvgImage,
    tx: f32,
    ty: f32,
    scale: f32,
    dst: &mut [u8],
    w: i32,
    h: i32,
    stride: i32,
) {
    // (uninitialized there; every entry a paint uses is set before it is used)
    let mut cache = NsvgCachedPaint {
        type_: 0,
        _spread: 0,
        xform: [0.0; 6],
        colors: [0; 256],
    };

    r.width = w;
    r.height = h;
    r.stride = stride;

    if w as usize > r.scanline.len() {
        r.scanline.resize(w as usize, 0);
    }

    for i in 0..h as usize {
        let row = i * stride as usize;
        dst[row..row + w as usize * 4].fill(0);
    }

    for shape in &image.shapes {
        if shape.flags & NSVG_FLAGS_VISIBLE == 0 {
            continue;
        }

        if shape.fill.type_ != NSVG_PAINT_NONE {
            r.reset_pool();
            r.freelist = None;
            r.edges.clear();

            r.flatten_shape(shape, scale);

            // Scale and translate edges
            for e in &mut r.edges {
                e.x0 = tx + e.x0;
                e.y0 = (ty + e.y0) * NSVG_SUBSAMPLES as f32;
                e.x1 = tx + e.x1;
                e.y1 = (ty + e.y1) * NSVG_SUBSAMPLES as f32;
            }

            // Rasterize edges
            if !r.edges.is_empty() {
                sdl_qsort(&mut r.edges, nsvg_cmp_edge);
            }

            // now, traverse the scanlines and find the intersections on each scanline, use non-zero rule
            nsvg_init_paint(&mut cache, &shape.fill, shape.opacity);

            r.rasterize_sorted_edges(dst, tx, ty, scale, &cache, shape.fill_rule);
        }
        if shape.stroke.type_ != NSVG_PAINT_NONE && (shape.stroke_width * scale) > 0.01 {
            r.reset_pool();
            r.freelist = None;
            r.edges.clear();

            r.flatten_shape_stroke(shape, scale);

            //			dumpEdges(r, "edge.svg");

            // Scale and translate edges
            for e in &mut r.edges {
                e.x0 = tx + e.x0;
                e.y0 = (ty + e.y0) * NSVG_SUBSAMPLES as f32;
                e.x1 = tx + e.x1;
                e.y1 = (ty + e.y1) * NSVG_SUBSAMPLES as f32;
            }

            // Rasterize edges
            if !r.edges.is_empty() {
                sdl_qsort(&mut r.edges, nsvg_cmp_edge);
            }

            // now, traverse the scanlines and find the intersections on each scanline, use non-zero rule
            nsvg_init_paint(&mut cache, &shape.stroke, shape.opacity);

            r.rasterize_sorted_edges(dst, tx, ty, scale, &cache, NSVG_FILLRULE_NONZERO);
        }
    }

    nsvg_unpremultiply_alpha(dst, w, h, stride);

    r.width = 0;
    r.height = 0;
    r.stride = 0;
}
