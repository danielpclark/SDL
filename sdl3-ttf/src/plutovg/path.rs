// Rust translation of source/plutovg-path.c (and the path parts of
// source/plutovg-private.h) from PlutoVG (0.0.12, as SDL_ttf's
// external/plutovg pins it).
// Copyright (c) 2020-2024 Samuel Ugochukwu <sammycageagle@gmail.com>
// This is an altered (translated) version of the original software; it is
// used under the MIT License (see PLUTOVG-LICENSE and LICENSE.txt).

//! Paths: building, transforming, traversing (flattened and dashed),
//! extents and the SVG path data parser.

use super::libm::{atan2f, cosf, hypotf, sinf, tanf};
use super::matrix::*;
use super::*;

/// `struct plutovg_path`
#[derive(Debug, Clone, Default)]
pub struct Path {
    pub num_points: i32,
    pub num_contours: i32,
    pub num_curves: i32,
    pub start_point: Point,
    pub elements: Vec<PathElement>,
}

/// `plutovg_path_iterator_t`
#[derive(Debug)]
pub struct PathIterator<'a> {
    /// Pointer to the array of path elements.
    pub elements: &'a [PathElement],
    /// Total number of elements in the array.
    pub size: i32,
    /// Current position in the array.
    pub index: i32,
}

/// Initializes the path iterator for a given path.
pub fn plutovg_path_iterator_init(path: &Path) -> PathIterator<'_> {
    PathIterator {
        elements: &path.elements,
        size: path.elements.len() as i32,
        index: 0,
    }
}

/// Checks if there are more elements to iterate over.
pub fn plutovg_path_iterator_has_next(it: &PathIterator<'_>) -> bool {
    it.index < it.size
}

/// Retrieves the current command and its associated points, and moves to
/// the next element.
pub fn plutovg_path_iterator_next(
    it: &mut PathIterator<'_>,
    points: &mut [Point; 3],
) -> PathCommand {
    let i = it.index as usize;
    let (command, length) = match it.elements[i] {
        PathElement::Header { command, length } => (command, length),
        PathElement::Point(_) => (PathCommand::MoveTo, 1),
    };
    match command {
        PathCommand::MoveTo | PathCommand::LineTo | PathCommand::Close => {
            points[0] = it.elements[i + 1].point();
        }
        PathCommand::CubicTo => {
            points[0] = it.elements[i + 1].point();
            points[1] = it.elements[i + 2].point();
            points[2] = it.elements[i + 3].point();
        }
    }

    it.index += length;
    command
}

/// Creates a new path object.
pub fn plutovg_path_create() -> Path {
    Path::default()
}

/// Retrieves the elements of the path.
pub fn plutovg_path_get_elements(path: &Path) -> &[PathElement] {
    &path.elements
}

/// `plutovg_path_add_command`: appends a command header and returns the
/// index of its first point.
fn plutovg_path_add_command(path: &mut Path, command: PathCommand, npoints: i32) -> usize {
    let length = npoints + 1;
    path.elements.push(PathElement::Header { command, length });
    let first = path.elements.len();
    for _ in 0..npoints {
        path.elements.push(PathElement::Point(Point::default()));
    }
    path.num_points += npoints;
    first
}

/// Moves the current point to a new position.
pub fn plutovg_path_move_to(path: &mut Path, x: f32, y: f32) {
    let e = plutovg_path_add_command(path, PathCommand::MoveTo, 1);
    path.elements[e] = PathElement::Point(Point { x, y });
    path.start_point = Point { x, y };
    path.num_contours += 1;
}

/// Adds a straight line segment to the path.
pub fn plutovg_path_line_to(path: &mut Path, x: f32, y: f32) {
    if path.elements.is_empty() {
        plutovg_path_move_to(path, 0.0, 0.0);
    }
    let e = plutovg_path_add_command(path, PathCommand::LineTo, 1);
    path.elements[e] = PathElement::Point(Point { x, y });
}

/// Adds a quadratic Bézier curve to the path.
pub fn plutovg_path_quad_to(path: &mut Path, x1: f32, y1: f32, x2: f32, y2: f32) {
    let (current_x, current_y) = plutovg_path_get_current_point(path);
    let cp1x = 2.0 / 3.0 * x1 + 1.0 / 3.0 * current_x;
    let cp1y = 2.0 / 3.0 * y1 + 1.0 / 3.0 * current_y;
    let cp2x = 2.0 / 3.0 * x1 + 1.0 / 3.0 * x2;
    let cp2y = 2.0 / 3.0 * y1 + 1.0 / 3.0 * y2;
    plutovg_path_cubic_to(path, cp1x, cp1y, cp2x, cp2y, x2, y2);
}

/// Adds a cubic Bézier curve to the path.
pub fn plutovg_path_cubic_to(
    path: &mut Path,
    x1: f32,
    y1: f32,
    x2: f32,
    y2: f32,
    x3: f32,
    y3: f32,
) {
    if path.elements.is_empty() {
        plutovg_path_move_to(path, 0.0, 0.0);
    }
    let e = plutovg_path_add_command(path, PathCommand::CubicTo, 3);
    path.elements[e] = PathElement::Point(Point { x: x1, y: y1 });
    path.elements[e + 1] = PathElement::Point(Point { x: x2, y: y2 });
    path.elements[e + 2] = PathElement::Point(Point { x: x3, y: y3 });
    path.num_curves += 1;
}

/// Adds an elliptical arc to the path.
pub fn plutovg_path_arc_to(
    path: &mut Path,
    mut rx: f32,
    mut ry: f32,
    angle: f32,
    large_arc_flag: bool,
    sweep_flag: bool,
    x: f32,
    y: f32,
) {
    let (current_x, current_y) = plutovg_path_get_current_point(path);
    if rx == 0.0 || ry == 0.0 || (current_x == x && current_y == y) {
        plutovg_path_line_to(path, x, y);
        return;
    }

    if rx < 0.0 {
        rx = -rx;
    }
    if ry < 0.0 {
        ry = -ry;
    }

    let dx = (current_x - x) * 0.5;
    let dy = (current_y - y) * 0.5;

    let mut matrix = Matrix::default();
    plutovg_matrix_init_rotate(&mut matrix, -angle);
    let (dx, dy) = plutovg_matrix_map(&matrix, dx, dy);

    let rxrx = rx * rx;
    let ryry = ry * ry;
    let dxdx = dx * dx;
    let dydy = dy * dy;
    let radius = dxdx / rxrx + dydy / ryry;
    if radius > 1.0 {
        rx *= radius.sqrt();
        ry *= radius.sqrt();
    }

    plutovg_matrix_init_scale(&mut matrix, 1.0 / rx, 1.0 / ry);
    plutovg_matrix_rotate(&mut matrix, -angle);

    let (x1, y1) = plutovg_matrix_map(&matrix, current_x, current_y);
    let (x2, y2) = plutovg_matrix_map(&matrix, x, y);

    let mut dx1 = x2 - x1;
    let mut dy1 = y2 - y1;
    let d = dx1 * dx1 + dy1 * dy1;
    let mut scale_sq = 1.0 / d - 0.25;
    if scale_sq < 0.0 {
        scale_sq = 0.0;
    }
    let mut scale = scale_sq.sqrt();
    if sweep_flag == large_arc_flag {
        scale = -scale;
    }
    dx1 *= scale;
    dy1 *= scale;

    let cx1 = 0.5 * (x1 + x2) - dy1;
    let cy1 = 0.5 * (y1 + y2) + dx1;

    let th1 = atan2f(y1 - cy1, x1 - cx1);
    let th2 = atan2f(y2 - cy1, x2 - cx1);
    let mut th_arc = th2 - th1;
    if th_arc < 0.0 && sweep_flag {
        th_arc += PLUTOVG_TWO_PI;
    } else if th_arc > 0.0 && !sweep_flag {
        th_arc -= PLUTOVG_TWO_PI;
    }
    plutovg_matrix_init_rotate(&mut matrix, angle);
    plutovg_matrix_scale(&mut matrix, rx, ry);
    let segments = cvt_f32_i32((th_arc / (PLUTOVG_HALF_PI + 0.001)).abs().ceil());
    for i in 0..segments {
        let th_start = th1 + i as f32 * th_arc / segments as f32;
        let th_end = th1 + (i + 1) as f32 * th_arc / segments as f32;
        let t = (8.0 / 6.0) * tanf(0.25 * (th_end - th_start));

        let x3 = cosf(th_end) + cx1;
        let y3 = sinf(th_end) + cy1;

        let cp2x = x3 + t * sinf(th_end);
        let cp2y = y3 - t * cosf(th_end);

        let mut cp1x = cosf(th_start) - t * sinf(th_start);
        let mut cp1y = sinf(th_start) + t * cosf(th_start);

        cp1x += cx1;
        cp1y += cy1;

        let (cp1x, cp1y) = plutovg_matrix_map(&matrix, cp1x, cp1y);
        let (cp2x, cp2y) = plutovg_matrix_map(&matrix, cp2x, cp2y);
        let (x3, y3) = plutovg_matrix_map(&matrix, x3, y3);

        plutovg_path_cubic_to(path, cp1x, cp1y, cp2x, cp2y, x3, y3);
    }
}

/// Closes the current sub-path by drawing a straight line back to the
/// start point.
pub fn plutovg_path_close(path: &mut Path) {
    if path.elements.is_empty() {
        return;
    }
    let e = plutovg_path_add_command(path, PathCommand::Close, 1);
    path.elements[e] = PathElement::Point(path.start_point);
}

/// Retrieves the current point of the path.
pub fn plutovg_path_get_current_point(path: &Path) -> (f32, f32) {
    let mut xx = 0.0;
    let mut yy = 0.0;
    if path.num_points > 0 {
        if let Some(e) = path.elements.last() {
            xx = e.point().x;
            yy = e.point().y;
        }
    }

    (xx, yy)
}

/// Reserves space for path elements.
pub fn plutovg_path_reserve(path: &mut Path, count: i32) {
    if count > 0 {
        let _ = path.elements.try_reserve(count as usize);
    }
}

/// Resets the path, clearing all path data.
pub fn plutovg_path_reset(path: &mut Path) {
    path.elements.clear();
    path.start_point = Point { x: 0.0, y: 0.0 };
    path.num_points = 0;
    path.num_contours = 0;
    path.num_curves = 0;
}

/// Adds a rectangle to the path.
pub fn plutovg_path_add_rect(path: &mut Path, x: f32, y: f32, w: f32, h: f32) {
    plutovg_path_reserve(path, 6 * 2);
    plutovg_path_move_to(path, x, y);
    plutovg_path_line_to(path, x + w, y);
    plutovg_path_line_to(path, x + w, y + h);
    plutovg_path_line_to(path, x, y + h);
    plutovg_path_line_to(path, x, y);
    plutovg_path_close(path);
}

/// Adds a rounded rectangle to the path.
pub fn plutovg_path_add_round_rect(
    path: &mut Path,
    x: f32,
    y: f32,
    w: f32,
    h: f32,
    rx: f32,
    ry: f32,
) {
    let rx = plutovg_min(rx, w * 0.5);
    let ry = plutovg_min(ry, h * 0.5);
    if rx == 0.0 && ry == 0.0 {
        plutovg_path_add_rect(path, x, y, w, h);
        return;
    }

    let right = x + w;
    let bottom = y + h;

    let cpx = rx * PLUTOVG_KAPPA;
    let cpy = ry * PLUTOVG_KAPPA;

    plutovg_path_reserve(path, 6 * 2 + 4 * 4);
    plutovg_path_move_to(path, x, y + ry);
    plutovg_path_cubic_to(path, x, y + ry - cpy, x + rx - cpx, y, x + rx, y);
    plutovg_path_line_to(path, right - rx, y);
    plutovg_path_cubic_to(
        path,
        right - rx + cpx,
        y,
        right,
        y + ry - cpy,
        right,
        y + ry,
    );
    plutovg_path_line_to(path, right, bottom - ry);
    plutovg_path_cubic_to(
        path,
        right,
        bottom - ry + cpy,
        right - rx + cpx,
        bottom,
        right - rx,
        bottom,
    );
    plutovg_path_line_to(path, x + rx, bottom);
    plutovg_path_cubic_to(
        path,
        x + rx - cpx,
        bottom,
        x,
        bottom - ry + cpy,
        x,
        bottom - ry,
    );
    plutovg_path_line_to(path, x, y + ry);
    plutovg_path_close(path);
}

/// Adds an ellipse to the path.
pub fn plutovg_path_add_ellipse(path: &mut Path, cx: f32, cy: f32, rx: f32, ry: f32) {
    let left = cx - rx;
    let top = cy - ry;
    let right = cx + rx;
    let bottom = cy + ry;

    let cpx = rx * PLUTOVG_KAPPA;
    let cpy = ry * PLUTOVG_KAPPA;

    plutovg_path_reserve(path, 2 * 2 + 4 * 4);
    plutovg_path_move_to(path, cx, top);
    plutovg_path_cubic_to(path, cx + cpx, top, right, cy - cpy, right, cy);
    plutovg_path_cubic_to(path, right, cy + cpy, cx + cpx, bottom, cx, bottom);
    plutovg_path_cubic_to(path, cx - cpx, bottom, left, cy + cpy, left, cy);
    plutovg_path_cubic_to(path, left, cy - cpy, cx - cpx, top, cx, top);
    plutovg_path_close(path);
}

/// Adds a circle to the path.
pub fn plutovg_path_add_circle(path: &mut Path, cx: f32, cy: f32, r: f32) {
    plutovg_path_add_ellipse(path, cx, cy, r, r);
}

/// Adds an arc to the path.
pub fn plutovg_path_add_arc(
    path: &mut Path,
    cx: f32,
    cy: f32,
    r: f32,
    a0: f32,
    a1: f32,
    ccw: bool,
) {
    let mut da = a1 - a0;
    if da.abs() > PLUTOVG_TWO_PI {
        da = PLUTOVG_TWO_PI;
    } else if da != 0.0 && ccw != (da < 0.0) {
        da += PLUTOVG_TWO_PI * (if ccw { -1.0 } else { 1.0 });
    }

    let seg_n = cvt_f32_i32((da.abs() / PLUTOVG_HALF_PI).ceil());
    if seg_n == 0 {
        return;
    }
    let mut a = a0;
    let mut ax = cx + cosf(a) * r;
    let mut ay = cy + sinf(a) * r;

    let seg_a = da / seg_n as f32;
    let d = (seg_a / PLUTOVG_HALF_PI) * PLUTOVG_KAPPA * r;
    let mut dx = -sinf(a) * d;
    let mut dy = cosf(a) * d;

    plutovg_path_reserve(path, 2 + 4 * seg_n);
    if path.elements.is_empty() {
        plutovg_path_move_to(path, ax, ay);
    } else {
        plutovg_path_line_to(path, ax, ay);
    }

    for _ in 0..seg_n {
        let cp1x = ax + dx;
        let cp1y = ay + dy;

        a += seg_a;
        ax = cx + cosf(a) * r;
        ay = cy + sinf(a) * r;

        dx = -sinf(a) * d;
        dy = cosf(a) * d;

        let cp2x = ax - dx;
        let cp2y = ay - dy;

        plutovg_path_cubic_to(path, cp1x, cp1y, cp2x, cp2y, ax, ay);
    }
}

/// Applies a transformation matrix to all points of the path.
pub fn plutovg_path_transform(path: &mut Path, matrix: &Matrix) {
    for e in path.elements.iter_mut() {
        if let PathElement::Point(p) = e {
            *p = plutovg_matrix_map_point(matrix, p);
        }
    }
}

/// Adds the commands of `source` to the path, transformed by `matrix` if
/// any.
pub fn plutovg_path_add_path(path: &mut Path, source: &Path, matrix: Option<&Matrix>) {
    let Some(matrix) = matrix else {
        path.elements.extend_from_slice(&source.elements);
        path.num_curves += source.num_curves;
        path.num_contours += source.num_contours;
        path.num_points += source.num_points;
        path.start_point = source.start_point;
        return;
    };

    let mut it = plutovg_path_iterator_init(source);

    let mut points = [Point::default(); 3];
    plutovg_path_reserve(path, source.elements.len() as i32);
    while plutovg_path_iterator_has_next(&it) {
        match plutovg_path_iterator_next(&mut it, &mut points) {
            PathCommand::MoveTo => {
                plutovg_matrix_map_points(matrix, &mut points[..1]);
                plutovg_path_move_to(path, points[0].x, points[0].y);
            }
            PathCommand::LineTo => {
                plutovg_matrix_map_points(matrix, &mut points[..1]);
                plutovg_path_line_to(path, points[0].x, points[0].y);
            }
            PathCommand::CubicTo => {
                plutovg_matrix_map_points(matrix, &mut points[..3]);
                plutovg_path_cubic_to(
                    path,
                    points[0].x,
                    points[0].y,
                    points[1].x,
                    points[1].y,
                    points[2].x,
                    points[2].y,
                );
            }
            PathCommand::Close => {
                plutovg_path_close(path);
            }
        }
    }
}

/// Traverses the path and calls `traverse_func` for each element.
pub fn plutovg_path_traverse(path: &Path, traverse_func: &mut dyn FnMut(PathCommand, &[Point])) {
    let mut it = plutovg_path_iterator_init(path);

    let mut points = [Point::default(); 3];
    while plutovg_path_iterator_has_next(&it) {
        match plutovg_path_iterator_next(&mut it, &mut points) {
            PathCommand::MoveTo => traverse_func(PathCommand::MoveTo, &points[..1]),
            PathCommand::LineTo => traverse_func(PathCommand::LineTo, &points[..1]),
            PathCommand::CubicTo => traverse_func(PathCommand::CubicTo, &points[..3]),
            PathCommand::Close => traverse_func(PathCommand::Close, &points[..1]),
        }
    }
}

/// `bezier_t`
#[derive(Clone, Copy, Default)]
struct Bezier {
    x1: f32,
    y1: f32,
    x2: f32,
    y2: f32,
    x3: f32,
    y3: f32,
    x4: f32,
    y4: f32,
}

/// `split_bezier`: splits `b` into `first` and `second`.
#[inline]
fn split_bezier(b: &Bezier) -> (Bezier, Bezier) {
    let mut first = Bezier::default();
    let mut second = Bezier::default();
    let mut c = (b.x2 + b.x3) * 0.5;
    first.x2 = (b.x1 + b.x2) * 0.5;
    second.x3 = (b.x3 + b.x4) * 0.5;
    first.x1 = b.x1;
    second.x4 = b.x4;
    first.x3 = (first.x2 + c) * 0.5;
    second.x2 = (second.x3 + c) * 0.5;
    first.x4 = (first.x3 + second.x2) * 0.5;
    second.x1 = first.x4;

    c = (b.y2 + b.y3) * 0.5;
    first.y2 = (b.y1 + b.y2) * 0.5;
    second.y3 = (b.y3 + b.y4) * 0.5;
    first.y1 = b.y1;
    second.y4 = b.y4;
    first.y3 = (first.y2 + c) * 0.5;
    second.y2 = (second.y3 + c) * 0.5;
    first.y4 = (first.y3 + second.y2) * 0.5;
    second.y1 = first.y4;
    (first, second)
}

/// Traverses the path with curves flattened into line segments.
pub fn plutovg_path_traverse_flatten(
    path: &Path,
    traverse_func: &mut dyn FnMut(PathCommand, &[Point]),
) {
    if path.num_curves == 0 {
        plutovg_path_traverse(path, traverse_func);
        return;
    }

    let threshold: f32 = 0.25;

    let mut it = plutovg_path_iterator_init(path);

    let mut beziers = [Bezier::default(); 32];
    let mut points = [Point::default(); 3];
    let mut current_point = Point { x: 0.0, y: 0.0 };
    while plutovg_path_iterator_has_next(&it) {
        let command = plutovg_path_iterator_next(&mut it, &mut points);
        match command {
            PathCommand::MoveTo | PathCommand::LineTo | PathCommand::Close => {
                traverse_func(command, &points[..1]);
                current_point = points[0];
            }
            PathCommand::CubicTo => {
                beziers[0].x1 = current_point.x;
                beziers[0].y1 = current_point.y;
                beziers[0].x2 = points[0].x;
                beziers[0].y2 = points[0].y;
                beziers[0].x3 = points[1].x;
                beziers[0].y3 = points[1].y;
                beziers[0].x4 = points[2].x;
                beziers[0].y4 = points[2].y;
                let mut b: isize = 0;
                while b >= 0 {
                    let bz = beziers[b as usize];
                    let y4y1 = bz.y4 - bz.y1;
                    let x4x1 = bz.x4 - bz.x1;
                    let mut l = x4x1.abs() + y4y1.abs();
                    let d;
                    if l > 1.0 {
                        d = ((x4x1) * (bz.y1 - bz.y2) - (y4y1) * (bz.x1 - bz.x2)).abs()
                            + ((x4x1) * (bz.y1 - bz.y3) - (y4y1) * (bz.x1 - bz.x3)).abs();
                    } else {
                        d = (bz.x1 - bz.x2).abs()
                            + (bz.y1 - bz.y2).abs()
                            + (bz.x1 - bz.x3).abs()
                            + (bz.y1 - bz.y3).abs();
                        l = 1.0;
                    }

                    if d < threshold * l || b == 31 {
                        let p = Point { x: bz.x4, y: bz.y4 };
                        traverse_func(PathCommand::LineTo, std::slice::from_ref(&p));
                        b -= 1;
                    } else {
                        let (first, second) = split_bezier(&bz);
                        beziers[b as usize + 1] = first;
                        beziers[b as usize] = second;
                        b += 1;
                    }
                }

                current_point = points[2];
            }
        }
    }
}

/// `dasher_t`
struct Dasher<'a, 'f> {
    dashes: &'a [f32],
    ndashes: i32,
    start_phase: f32,
    phase: f32,
    start_index: i32,
    index: i32,
    start_toggle: bool,
    toggle: bool,
    current_point: Point,
    traverse_func: &'f mut dyn FnMut(PathCommand, &[Point]),
}

impl Dasher<'_, '_> {
    #[inline]
    fn dash(&self, index: i32) -> f32 {
        self.dashes[(index % self.ndashes) as usize]
    }
}

/// `dash_traverse_func`
fn dash_traverse_func(dasher: &mut Dasher<'_, '_>, command: PathCommand, points: &[Point]) {
    if command == PathCommand::MoveTo {
        if dasher.start_toggle {
            (dasher.traverse_func)(PathCommand::MoveTo, points);
        }
        dasher.current_point = points[0];
        dasher.phase = dasher.start_phase;
        dasher.index = dasher.start_index;
        dasher.toggle = dasher.start_toggle;
        return;
    }

    /* assert(command == PLUTOVG_PATH_COMMAND_LINE_TO || command == PLUTOVG_PATH_COMMAND_CLOSE); */
    let p0 = dasher.current_point;
    let p1 = points[0];
    let dx = p1.x - p0.x;
    let dy = p1.y - p0.y;
    let dist0 = (dx * dx + dy * dy).sqrt();
    let mut dist1: f32 = 0.0;
    while dist0 - dist1 > dasher.dash(dasher.index) - dasher.phase {
        dist1 += dasher.dash(dasher.index) - dasher.phase;
        let a = dist1 / dist0;
        let p = Point {
            x: p0.x + a * dx,
            y: p0.y + a * dy,
        };
        if dasher.toggle {
            (dasher.traverse_func)(PathCommand::LineTo, std::slice::from_ref(&p));
        } else {
            (dasher.traverse_func)(PathCommand::MoveTo, std::slice::from_ref(&p));
        }

        dasher.phase = 0.0;
        dasher.toggle = !dasher.toggle;
        dasher.index = dasher.index.wrapping_add(1);
        if dasher.index < 0 {
            /* Note (upstream): C's int index overflows here (after 2^31
            dashes on a segment); it is kept non-negative here */
            dasher.index &= i32::MAX;
        }
    }

    if dasher.toggle {
        (dasher.traverse_func)(PathCommand::LineTo, std::slice::from_ref(&p1));
    }

    dasher.phase += dist0 - dist1;
    dasher.current_point = p1;
}

/// Traverses the path with a dashed pattern.
pub fn plutovg_path_traverse_dashed(
    path: &Path,
    offset: f32,
    dashes: &[f32],
    traverse_func: &mut dyn FnMut(PathCommand, &[Point]),
) {
    let ndashes = dashes.len() as i32;
    let mut dash_sum: f32 = 0.0;
    for d in dashes {
        dash_sum += *d;
    }
    if ndashes % 2 == 1 {
        dash_sum *= 2.0;
    }
    if dash_sum <= 0.0 {
        plutovg_path_traverse(path, traverse_func);
        return;
    }

    let mut dasher = Dasher {
        dashes,
        ndashes,
        start_phase: offset % dash_sum,
        phase: 0.0,
        start_index: 0,
        index: 0,
        start_toggle: true,
        toggle: true,
        current_point: Point::default(),
        traverse_func,
    };
    if dasher.start_phase < 0.0 {
        dasher.start_phase += dash_sum;
    }
    while dasher.start_phase > 0.0 && dasher.start_phase >= dasher.dash(dasher.start_index) {
        dasher.start_phase -= dashes[(dasher.start_index % dasher.ndashes) as usize];
        dasher.start_toggle = !dasher.start_toggle;
        dasher.start_index += 1;
    }

    dasher.phase = dasher.start_phase;
    dasher.index = dasher.start_index;
    dasher.toggle = dasher.start_toggle;
    dasher.current_point = Point { x: 0.0, y: 0.0 };
    plutovg_path_traverse_flatten(path, &mut |command, points| {
        dash_traverse_func(&mut dasher, command, points)
    });
}

/// Creates a copy of the path.
pub fn plutovg_path_clone(path: &Path) -> Path {
    path.clone()
}

/// `clone_traverse_func`
fn clone_traverse_func(path: &mut Path, command: PathCommand, points: &[Point]) {
    match command {
        PathCommand::MoveTo => plutovg_path_move_to(path, points[0].x, points[0].y),
        PathCommand::LineTo => plutovg_path_line_to(path, points[0].x, points[0].y),
        PathCommand::CubicTo => plutovg_path_cubic_to(
            path,
            points[0].x,
            points[0].y,
            points[1].x,
            points[1].y,
            points[2].x,
            points[2].y,
        ),
        PathCommand::Close => plutovg_path_close(path),
    }
}

/// Creates a copy of the path with curves flattened into line segments.
pub fn plutovg_path_clone_flatten(path: &Path) -> Path {
    let mut clone = plutovg_path_create();
    plutovg_path_reserve(
        &mut clone,
        (path.elements.len() as i32).saturating_add(path.num_curves.saturating_mul(32)),
    );
    plutovg_path_traverse_flatten(path, &mut |command, points| {
        clone_traverse_func(&mut clone, command, points)
    });
    clone
}

/// Creates a copy of the path with a dashed pattern applied.
pub fn plutovg_path_clone_dashed(path: &Path, offset: f32, dashes: &[f32]) -> Path {
    let mut clone = plutovg_path_create();
    plutovg_path_reserve(
        &mut clone,
        (path.elements.len() as i32).saturating_add(path.num_curves.saturating_mul(32)),
    );
    plutovg_path_traverse_dashed(path, offset, dashes, &mut |command, points| {
        clone_traverse_func(&mut clone, command, points)
    });
    clone
}

/// `extents_calculator_t`
struct ExtentsCalculator {
    current_point: Point,
    is_first_point: bool,
    length: f32,
    x1: f32,
    y1: f32,
    x2: f32,
    y2: f32,
}

/// `extents_traverse_func`
fn extents_traverse_func(
    calculator: &mut ExtentsCalculator,
    command: PathCommand,
    points: &[Point],
) {
    if calculator.is_first_point {
        /* assert(command == PLUTOVG_PATH_COMMAND_MOVE_TO); */
        calculator.is_first_point = false;
        calculator.current_point = points[0];
        calculator.x1 = points[0].x;
        calculator.y1 = points[0].y;
        calculator.x2 = points[0].x;
        calculator.y2 = points[0].y;
        calculator.length = 0.0;
        return;
    }

    for p in points {
        calculator.x1 = plutovg_min(calculator.x1, p.x);
        calculator.y1 = plutovg_min(calculator.y1, p.y);
        calculator.x2 = plutovg_max(calculator.x2, p.x);
        calculator.y2 = plutovg_max(calculator.y2, p.y);
        if command != PathCommand::MoveTo {
            calculator.length += hypotf(
                p.x - calculator.current_point.x,
                p.y - calculator.current_point.y,
            );
        }
        calculator.current_point = *p;
    }
}

/// Computes the bounding box and total length of the path (the length is
/// returned).
pub fn plutovg_path_extents(path: &Path, extents: Option<&mut Rect>, tight: bool) -> f32 {
    let mut calculator = ExtentsCalculator {
        current_point: Point { x: 0.0, y: 0.0 },
        is_first_point: true,
        length: 0.0,
        x1: 0.0,
        y1: 0.0,
        x2: 0.0,
        y2: 0.0,
    };
    if tight {
        plutovg_path_traverse_flatten(path, &mut |command, points| {
            extents_traverse_func(&mut calculator, command, points)
        });
    } else {
        plutovg_path_traverse(path, &mut |command, points| {
            extents_traverse_func(&mut calculator, command, points)
        });
    }

    if let Some(extents) = extents {
        extents.x = calculator.x1;
        extents.y = calculator.y1;
        extents.w = calculator.x2 - calculator.x1;
        extents.h = calculator.y2 - calculator.y1;
    }

    calculator.length
}

/// Calculates the total length of the path.
pub fn plutovg_path_length(path: &Path) -> f32 {
    plutovg_path_extents(path, None, true)
}

/// `parse_arc_flag`
#[inline]
fn parse_arc_flag(data: &[u8], begin: &mut usize, end: usize, flag: &mut bool) -> bool {
    if plutovg_skip_delim(data, begin, end, b'0') {
        *flag = false;
    } else if plutovg_skip_delim(data, begin, end, b'1') {
        *flag = true;
    } else {
        return false;
    }
    plutovg_skip_ws_or_comma(data, begin, end, None);
    true
}

/// `parse_path_coordinates`
#[inline]
fn parse_path_coordinates(
    data: &[u8],
    begin: &mut usize,
    end: usize,
    values: &mut [f32; 6],
    offset: usize,
    count: usize,
) -> bool {
    for i in 0..count {
        if !plutovg_parse_number(data, begin, end, &mut values[offset + i]) {
            return false;
        }
        plutovg_skip_ws_or_comma(data, begin, end, None);
    }

    true
}

/// Parses SVG path data into the path. Returns `true` on success.
pub fn plutovg_path_parse(path: &mut Path, data: &[u8]) -> bool {
    let mut it = 0;
    let end = data.len();

    let mut values = [0.0f32; 6];
    let mut flags = [false; 2];

    let mut start_x: f32 = 0.0;
    let mut start_y: f32 = 0.0;
    let mut current_x: f32 = 0.0;
    let mut current_y: f32 = 0.0;
    let mut last_control_x: f32 = 0.0;
    let mut last_control_y: f32 = 0.0;

    let mut command: u8 = 0;
    let mut last_command: u8 = 0;
    plutovg_skip_ws(data, &mut it, end);
    while it < end {
        if plutovg_is_alpha(data[it]) {
            command = data[it];
            it += 1;
            plutovg_skip_ws(data, &mut it, end);
        }

        if last_command == 0 && !(command == b'M' || command == b'm') {
            return false;
        }
        if command == b'M' || command == b'm' {
            if !parse_path_coordinates(data, &mut it, end, &mut values, 0, 2) {
                return false;
            }
            if command == b'm' {
                values[0] += current_x;
                values[1] += current_y;
            }

            plutovg_path_move_to(path, values[0], values[1]);
            start_x = values[0];
            current_x = start_x;
            start_y = values[1];
            current_y = start_y;
            command = if command == b'm' { b'l' } else { b'L' };
        } else if command == b'L' || command == b'l' {
            if !parse_path_coordinates(data, &mut it, end, &mut values, 0, 2) {
                return false;
            }
            if command == b'l' {
                values[0] += current_x;
                values[1] += current_y;
            }

            plutovg_path_line_to(path, values[0], values[1]);
            current_x = values[0];
            current_y = values[1];
        } else if command == b'H' || command == b'h' {
            if !parse_path_coordinates(data, &mut it, end, &mut values, 0, 1) {
                return false;
            }
            if command == b'h' {
                values[0] += current_x;
            }

            plutovg_path_line_to(path, values[0], current_y);
            current_x = values[0];
        } else if command == b'V' || command == b'v' {
            if !parse_path_coordinates(data, &mut it, end, &mut values, 1, 1) {
                return false;
            }
            if command == b'v' {
                values[1] += current_y;
            }

            plutovg_path_line_to(path, current_x, values[1]);
            current_y = values[1];
        } else if command == b'Q' || command == b'q' {
            if !parse_path_coordinates(data, &mut it, end, &mut values, 0, 4) {
                return false;
            }
            if command == b'q' {
                values[0] += current_x;
                values[1] += current_y;
                values[2] += current_x;
                values[3] += current_y;
            }

            plutovg_path_quad_to(path, values[0], values[1], values[2], values[3]);
            last_control_x = values[0];
            last_control_y = values[1];
            current_x = values[2];
            current_y = values[3];
        } else if command == b'C' || command == b'c' {
            if !parse_path_coordinates(data, &mut it, end, &mut values, 0, 6) {
                return false;
            }
            if command == b'c' {
                values[0] += current_x;
                values[1] += current_y;
                values[2] += current_x;
                values[3] += current_y;
                values[4] += current_x;
                values[5] += current_y;
            }

            plutovg_path_cubic_to(
                path, values[0], values[1], values[2], values[3], values[4], values[5],
            );
            last_control_x = values[2];
            last_control_y = values[3];
            current_x = values[4];
            current_y = values[5];
        } else if command == b'T' || command == b't' {
            if last_command != b'Q'
                && last_command != b'q'
                && last_command != b'T'
                && last_command != b't'
            {
                values[0] = current_x;
                values[1] = current_y;
            } else {
                values[0] = 2.0 * current_x - last_control_x;
                values[1] = 2.0 * current_y - last_control_y;
            }

            if !parse_path_coordinates(data, &mut it, end, &mut values, 2, 2) {
                return false;
            }
            if command == b't' {
                values[2] += current_x;
                values[3] += current_y;
            }

            plutovg_path_quad_to(path, values[0], values[1], values[2], values[3]);
            last_control_x = values[0];
            last_control_y = values[1];
            current_x = values[2];
            current_y = values[3];
        } else if command == b'S' || command == b's' {
            if last_command != b'C'
                && last_command != b'c'
                && last_command != b'S'
                && last_command != b's'
            {
                values[0] = current_x;
                values[1] = current_y;
            } else {
                values[0] = 2.0 * current_x - last_control_x;
                values[1] = 2.0 * current_y - last_control_y;
            }

            if !parse_path_coordinates(data, &mut it, end, &mut values, 2, 4) {
                return false;
            }
            if command == b's' {
                values[2] += current_x;
                values[3] += current_y;
                values[4] += current_x;
                values[5] += current_y;
            }

            plutovg_path_cubic_to(
                path, values[0], values[1], values[2], values[3], values[4], values[5],
            );
            last_control_x = values[2];
            last_control_y = values[3];
            current_x = values[4];
            current_y = values[5];
        } else if command == b'A' || command == b'a' {
            if !parse_path_coordinates(data, &mut it, end, &mut values, 0, 3)
                || !parse_arc_flag(data, &mut it, end, &mut flags[0])
                || !parse_arc_flag(data, &mut it, end, &mut flags[1])
                || !parse_path_coordinates(data, &mut it, end, &mut values, 3, 2)
            {
                return false;
            }

            if command == b'a' {
                values[3] += current_x;
                values[4] += current_y;
            }

            plutovg_path_arc_to(
                path,
                values[0],
                values[1],
                plutovg_deg2rad(values[2]),
                flags[0],
                flags[1],
                values[3],
                values[4],
            );
            current_x = values[3];
            current_y = values[4];
        } else if command == b'Z' || command == b'z' {
            if last_command == b'Z' || last_command == b'z' {
                return false;
            }
            plutovg_path_close(path);
            current_x = start_x;
            current_y = start_y;
        } else {
            return false;
        }

        last_command = command;
    }

    true
}
