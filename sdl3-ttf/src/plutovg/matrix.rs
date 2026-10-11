// Rust translation of source/plutovg-matrix.c from PlutoVG (0.0.12, as
// SDL_ttf's external/plutovg pins it).
// Copyright (c) 2020-2024 Samuel Ugochukwu <sammycageagle@gmail.com>
// This is an altered (translated) version of the original software; it is
// used under the MIT License (see PLUTOVG-LICENSE and LICENSE.txt).

//! 2D transformation matrices.

use super::libm::{cosf, sinf, tanf};
use super::*;

/// Initializes a 2D transformation matrix.
pub fn plutovg_matrix_init(matrix: &mut Matrix, a: f32, b: f32, c: f32, d: f32, e: f32, f: f32) {
    matrix.a = a;
    matrix.b = b;
    matrix.c = c;
    matrix.d = d;
    matrix.e = e;
    matrix.f = f;
}

/// Initializes a 2D transformation matrix to the identity matrix.
pub fn plutovg_matrix_init_identity(matrix: &mut Matrix) {
    matrix.a = 1.0;
    matrix.b = 0.0;
    matrix.c = 0.0;
    matrix.d = 1.0;
    matrix.e = 0.0;
    matrix.f = 0.0;
}

/// Initializes a 2D transformation matrix for translation.
pub fn plutovg_matrix_init_translate(matrix: &mut Matrix, tx: f32, ty: f32) {
    plutovg_matrix_init(matrix, 1.0, 0.0, 0.0, 1.0, tx, ty);
}

/// Initializes a 2D transformation matrix for scaling.
pub fn plutovg_matrix_init_scale(matrix: &mut Matrix, sx: f32, sy: f32) {
    plutovg_matrix_init(matrix, sx, 0.0, 0.0, sy, 0.0, 0.0);
}

/// Initializes a 2D transformation matrix for rotation.
pub fn plutovg_matrix_init_rotate(matrix: &mut Matrix, angle: f32) {
    let c = cosf(angle);
    let s = sinf(angle);
    plutovg_matrix_init(matrix, c, s, -s, c, 0.0, 0.0);
}

/// Initializes a 2D transformation matrix for shearing.
pub fn plutovg_matrix_init_shear(matrix: &mut Matrix, shx: f32, shy: f32) {
    plutovg_matrix_init(matrix, 1.0, tanf(shy), tanf(shx), 1.0, 0.0, 0.0);
}

/// Adds a translation with offsets `tx` and `ty` to the matrix.
pub fn plutovg_matrix_translate(matrix: &mut Matrix, tx: f32, ty: f32) {
    let mut m = Matrix::default();
    plutovg_matrix_init_translate(&mut m, tx, ty);
    let right = *matrix;
    plutovg_matrix_multiply(matrix, &m, &right);
}

/// Scales the matrix by factors `sx` and `sy`.
pub fn plutovg_matrix_scale(matrix: &mut Matrix, sx: f32, sy: f32) {
    let mut m = Matrix::default();
    plutovg_matrix_init_scale(&mut m, sx, sy);
    let right = *matrix;
    plutovg_matrix_multiply(matrix, &m, &right);
}

/// Rotates the matrix by the specified angle (in radians).
pub fn plutovg_matrix_rotate(matrix: &mut Matrix, angle: f32) {
    let mut m = Matrix::default();
    plutovg_matrix_init_rotate(&mut m, angle);
    let right = *matrix;
    plutovg_matrix_multiply(matrix, &m, &right);
}

/// Shears the matrix by the specified factors.
pub fn plutovg_matrix_shear(matrix: &mut Matrix, shx: f32, shy: f32) {
    let mut m = Matrix::default();
    plutovg_matrix_init_shear(&mut m, shx, shy);
    let right = *matrix;
    plutovg_matrix_multiply(matrix, &m, &right);
}

/// Multiplies `left` and `right` matrices and stores the result in
/// `matrix`.
pub fn plutovg_matrix_multiply(matrix: &mut Matrix, left: &Matrix, right: &Matrix) {
    let a = left.a * right.a + left.b * right.c;
    let b = left.a * right.b + left.b * right.d;
    let c = left.c * right.a + left.d * right.c;
    let d = left.c * right.b + left.d * right.d;
    let e = left.e * right.a + left.f * right.c + right.e;
    let f = left.e * right.b + left.f * right.d + right.f;
    plutovg_matrix_init(matrix, a, b, c, d, e, f);
}

/// Calculates the inverse of `matrix` and stores it in `inverse`.
/// Returns `false` if the matrix is not invertible.
pub fn plutovg_matrix_invert(matrix: &Matrix, inverse: Option<&mut Matrix>) -> bool {
    let det = matrix.a * matrix.d - matrix.b * matrix.c;
    if det == 0.0 {
        return false;
    }
    if let Some(inverse) = inverse {
        let inv_det = 1.0 / det;
        let a = matrix.a * inv_det;
        let b = matrix.b * inv_det;
        let c = matrix.c * inv_det;
        let d = matrix.d * inv_det;
        let e = (matrix.c * matrix.f - matrix.d * matrix.e) * inv_det;
        let f = (matrix.b * matrix.e - matrix.a * matrix.f) * inv_det;
        plutovg_matrix_init(inverse, d, -b, -c, a, e, f);
    }

    true
}

/// Transforms the point `(x, y)` using `matrix`.
#[inline]
pub fn plutovg_matrix_map(matrix: &Matrix, x: f32, y: f32) -> (f32, f32) {
    (
        x * matrix.a + y * matrix.c + matrix.e,
        x * matrix.b + y * matrix.d + matrix.f,
    )
}

/// Transforms the `src` point using `matrix`.
#[inline]
pub fn plutovg_matrix_map_point(matrix: &Matrix, src: &Point) -> Point {
    let (x, y) = plutovg_matrix_map(matrix, src.x, src.y);
    Point { x, y }
}

/// Transforms an array of points using `matrix`.
pub fn plutovg_matrix_map_points(matrix: &Matrix, points: &mut [Point]) {
    for p in points.iter_mut() {
        *p = plutovg_matrix_map_point(matrix, p);
    }
}

/// Transforms the `src` rectangle using `matrix` (its bounding box).
pub fn plutovg_matrix_map_rect(matrix: &Matrix, src: &Rect) -> Rect {
    let mut p = [
        Point { x: src.x, y: src.y },
        Point {
            x: src.x + src.w,
            y: src.y,
        },
        Point {
            x: src.x + src.w,
            y: src.y + src.h,
        },
        Point {
            x: src.x,
            y: src.y + src.h,
        },
    ];
    plutovg_matrix_map_points(matrix, &mut p);

    let mut l = p[0].x;
    let mut t = p[0].y;
    let mut r = p[0].x;
    let mut b = p[0].y;

    for q in &p[1..] {
        if q.x < l {
            l = q.x;
        }
        if q.x > r {
            r = q.x;
        }
        if q.y < t {
            t = q.y;
        }
        if q.y > b {
            b = q.y;
        }
    }

    Rect {
        x: l,
        y: t,
        w: r - l,
        h: b - t,
    }
}

/// `parse_matrix_parameters`
fn parse_matrix_parameters(
    data: &[u8],
    begin: &mut usize,
    end: usize,
    values: &mut [f32; 6],
    required: i32,
    optional: i32,
) -> i32 {
    if !plutovg_skip_ws_and_delim(data, begin, end, b'(') {
        return 0;
    }
    let mut count = 0;
    let max_count = required + optional;
    let mut has_trailing_comma = false;
    while count < max_count {
        if !plutovg_parse_number(data, begin, end, &mut values[count as usize]) {
            break;
        }
        plutovg_skip_ws_or_comma(data, begin, end, Some(&mut has_trailing_comma));
        count += 1;
    }

    if !has_trailing_comma
        && (count == required || count == max_count)
        && plutovg_skip_delim(data, begin, end, b')')
    {
        return count;
    }

    0
}

/// Parses an SVG transform string into a matrix. Returns `true` on
/// success, `false` on failure.
pub fn plutovg_matrix_parse(matrix: &mut Matrix, data: &[u8]) -> bool {
    let mut values = [0.0f32; 6];
    plutovg_matrix_init_identity(matrix);
    let mut it = 0;
    let end = data.len();
    let mut has_trailing_comma = false;
    plutovg_skip_ws(data, &mut it, end);
    while it < end {
        if plutovg_skip_string(data, &mut it, end, b"matrix") {
            let count = parse_matrix_parameters(data, &mut it, end, &mut values, 6, 0);
            if count == 0 {
                return false;
            }
            let m = Matrix {
                a: values[0],
                b: values[1],
                c: values[2],
                d: values[3],
                e: values[4],
                f: values[5],
            };
            let right = *matrix;
            plutovg_matrix_multiply(matrix, &m, &right);
        } else if plutovg_skip_string(data, &mut it, end, b"translate") {
            let count = parse_matrix_parameters(data, &mut it, end, &mut values, 1, 1);
            if count == 0 {
                return false;
            }
            if count == 1 {
                plutovg_matrix_translate(matrix, values[0], 0.0);
            } else {
                plutovg_matrix_translate(matrix, values[0], values[1]);
            }
        } else if plutovg_skip_string(data, &mut it, end, b"scale") {
            let count = parse_matrix_parameters(data, &mut it, end, &mut values, 1, 1);
            if count == 0 {
                return false;
            }
            if count == 1 {
                plutovg_matrix_scale(matrix, values[0], values[0]);
            } else {
                plutovg_matrix_scale(matrix, values[0], values[1]);
            }
        } else if plutovg_skip_string(data, &mut it, end, b"rotate") {
            let count = parse_matrix_parameters(data, &mut it, end, &mut values, 1, 2);
            if count == 0 {
                return false;
            }
            if count == 3 {
                plutovg_matrix_translate(matrix, values[1], values[2]);
            }
            plutovg_matrix_rotate(matrix, plutovg_deg2rad(values[0]));
            if count == 3 {
                plutovg_matrix_translate(matrix, -values[1], -values[2]);
            }
        } else if plutovg_skip_string(data, &mut it, end, b"skewX") {
            let count = parse_matrix_parameters(data, &mut it, end, &mut values, 1, 0);
            if count == 0 {
                return false;
            }
            plutovg_matrix_shear(matrix, plutovg_deg2rad(values[0]), 0.0);
        } else if plutovg_skip_string(data, &mut it, end, b"skewY") {
            let count = parse_matrix_parameters(data, &mut it, end, &mut values, 1, 0);
            if count == 0 {
                return false;
            }
            plutovg_matrix_shear(matrix, 0.0, plutovg_deg2rad(values[0]));
        } else {
            return false;
        }

        plutovg_skip_ws_or_comma(data, &mut it, end, Some(&mut has_trailing_comma));
    }

    !has_trailing_comma
}
