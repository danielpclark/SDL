// Rust translation of lib/jxl/splines.h and lib/jxl/splines.cc from libjxl
// (https://github.com/libjxl/libjxl, at the revision SDL_image's
// external/libjxl pins: libjxl 0.7.3 with SDL's patches).
// Copyright (c) the JPEG XL Project Authors. All rights reserved.
// SPDX-License-Identifier: BSD-3-Clause (see LICENSE.txt)
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! Splines: decoding and rendering (the encoder-side quantization is not
//! translated).

use super::base::{jxl_failure, unpack_signed, Status, K_PI};
use super::dec_ans::{decode_histograms, AnsCode, AnsSymbolReader};
use super::dec_bit_reader::BitReader;
use super::math::{fast_cosf, fast_erff, hypotf, logf, roundf};

const K_DESIRED_RENDERING_DISTANCE: f32 = 1.0;

// (dct_scales.h)
const K_SQRT2: f32 = 1.41421356237f32;
const K_SQRT0_5: f32 = 0.70710678118f32;

const K_QUANTIZATION_ADJUSTMENT_CONTEXT: usize = 0;
const K_STARTING_POSITION_CONTEXT: usize = 1;
const K_NUM_SPLINES_CONTEXT: usize = 2;
const K_NUM_CONTROL_POINTS_CONTEXT: usize = 3;
const K_CONTROL_POINTS_CONTEXT: usize = 4;
const K_DCT_CONTEXT: usize = 5;
const K_NUM_SPLINE_CONTEXTS: usize = 6;

/// Translation of `Spline::Point`.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct Point {
    pub x: f32,
    pub y: f32,
}

impl PartialEq for Point {
    fn eq(&self, other: &Point) -> bool {
        (self.x - other.x).abs() < 1e-3f32 && (self.y - other.y).abs() < 1e-3f32
    }
}

/// Translation of `Spline`.
#[derive(Clone, Debug)]
struct Spline {
    control_points: Vec<Point>,
    color_dct: [[f32; 32]; 3],
    sigma_dct: [f32; 32],
}

/// Translation of `QuantizedSpline`.
#[derive(Clone, Debug, Default)]
pub(crate) struct QuantizedSpline {
    control_points: Vec<(i64, i64)>, // Double delta-encoded.
    color_dct: [[i32; 32]; 3],
    sigma_dct: [i32; 32],
}

/// Translation of `SplineSegment`.
#[derive(Clone, Copy, Debug, Default)]
struct SplineSegment {
    center_x: f32,
    center_y: f32,
    maximum_distance: f32,
    inv_sigma: f32,
    sigma_over_4_times_intensity: f32,
    color: [f32; 3],
}

/// Translation of `Splines`.
#[derive(Clone, Debug, Default)]
pub(crate) struct Splines {
    quantization_adjustment: i32,
    splines: Vec<QuantizedSpline>,
    starting_points: Vec<Point>,
    segments: Vec<SplineSegment>,
    segment_indices: Vec<usize>,
    segment_y_start: Vec<usize>,
}

// Given a set of DCT coefficients, this returns the result of performing cosine
// interpolation on the original samples.
/// Translation of `ContinuousIDCT()`.
fn continuous_idct(dct: &[f32; 32], t: f32) -> f32 {
    // We compute here the DCT-3 of the `dct` vector, rescaled by a factor of
    // sqrt(32). This is such that an input vector vector {x, 0, ..., 0} produces
    // a constant result of x. dct[0] was scaled in Dequantize() to allow uniform
    // treatment of all the coefficients.
    let mut result = 0.0f32;
    let tandhalf = t + 0.5f32;
    for (i, &d) in dct.iter().enumerate() {
        let multiplier = (K_PI / 32.0 * i as f64) as f32;
        let cos_arg = multiplier * tandhalf;
        let cos = fast_cosf(cos_arg);
        let local_res = d * cos;
        result = K_SQRT2 * local_res + result;
    }
    result
}

/// Translation of `DrawSegment()` (one pixel).
#[inline]
fn draw_segment_pixel(segment: &SplineSegment, add: bool, y: usize, x: i64, rows: &mut [&mut [f32]; 3], row_x0: i64) {
    let inv_sigma = segment.inv_sigma;
    let half = 0.5f32;
    let one_over_2s2 = 0.353553391f32;
    let sigma_over_4_times_intensity = segment.sigma_over_4_times_intensity;
    let dx = (x as i32) as f32 - segment.center_x;
    let dy = y as f32 - segment.center_y;
    let sqd = dx * dx + dy * dy;
    let distance = sqd.sqrt();
    let one_dimensional_factor = fast_erff((distance * half + one_over_2s2) * inv_sigma)
        - fast_erff((distance * half - one_over_2s2) * inv_sigma);
    let local_intensity = sigma_over_4_times_intensity * (one_dimensional_factor * one_dimensional_factor);
    for c in 0..3 {
        let cm = if add { segment.color[c] } else { -segment.color[c] };
        let idx = (x - row_x0) as usize;
        let inp = rows[c][idx];
        rows[c][idx] = cm * local_intensity + inp;
    }
}

/// Translation of `DrawSegment()` (a range).
fn draw_segment(segment: &SplineSegment, add: bool, y: usize, x0: i64, x1: i64, rows: &mut [&mut [f32]; 3]) {
    let mut x = x0.max((segment.center_x - segment.maximum_distance + 0.5f32) as i64);
    // one-past-the-end
    let x1 = x1.min((segment.center_x + segment.maximum_distance + 1.5f32) as i64);
    while x < x1 {
        draw_segment_pixel(segment, add, y, x, rows, x0);
        x += 1;
    }
}

/// Translation of `ComputeSegments()`.
fn compute_segments(
    center: &Point,
    intensity: f32,
    color: &[f32; 3],
    sigma: f32,
    segments: &mut Vec<SplineSegment>,
    segments_by_y: &mut Vec<(usize, usize)>,
    pixel_limit: &mut usize,
) {
    // In worst case zero-sized dot spans over 2 rows / columns.
    const K_THIN_DOT_SPAN: f32 = 2.0;
    // Sanity check sigma, inverse sigma and intensity
    if !(sigma.is_finite() && sigma != 0.0 && (1.0f32 / sigma).is_finite() && intensity.is_finite()) {
        // Even no-draw should still be accounted.
        *pixel_limit -= (*pixel_limit).min((K_THIN_DOT_SPAN * K_THIN_DOT_SPAN) as usize);
        return;
    }
    // (JXL_HIGH_PRECISION)
    const K_DISTANCE_EXP: f32 = 5.0;
    // We cap from below colors to at least 0.01.
    let mut max_color = 0.01f32;
    for &c in color {
        let v = (c * intensity).abs();
        if max_color < v {
            max_color = v;
        }
    }
    // Distance beyond which max_color*intensity*exp(-d^2 / (2 * sigma^2)) drops
    // below 10^-kDistanceExp.
    const LOG_0_1: f64 = -2.302585092994045684; // std::log(0.1)
    let maximum_distance = ((-2.0f32 * sigma * sigma) as f64
        * (LOG_0_1 * K_DISTANCE_EXP as f64 - logf(max_color) as f64))
        .sqrt() as f32;
    let segment = SplineSegment {
        center_y: center.y,
        center_x: center.x,
        color: *color,
        inv_sigma: 1.0f32 / sigma,
        sigma_over_4_times_intensity: 0.25f32 * sigma * intensity,
        maximum_distance,
    };
    let cost = 2.0f32 * maximum_distance + K_THIN_DOT_SPAN;
    // Check cost^2 fits size_t.
    if cost >= (1 << 15) as f32 {
        // Too much to rasterize.
        *pixel_limit = 0;
        return;
    }
    let area_cost = (cost * cost) as usize;
    if area_cost > *pixel_limit {
        *pixel_limit = 0;
        return;
    }
    // TODO(eustas): perhaps we should charge less: (y1 - y0) <= cost
    *pixel_limit -= area_cost;
    let y0 = (center.y - maximum_distance + 0.5f32) as i64;
    let y1 = (center.y + maximum_distance + 1.5f32) as i64; // one-past-the-end
    let mut y = y0.max(0);
    while y < y1 {
        segments_by_y.push((y as usize, segments.len()));
        y += 1;
    }
    segments.push(segment);
}

/// Translation of `SegmentsFromPoints()`.
fn segments_from_points(
    spline: &Spline,
    points_to_draw: &[(Point, f32)],
    arc_length: f32,
    segments: &mut Vec<SplineSegment>,
    segments_by_y: &mut Vec<(usize, usize)>,
    pixel_limit: &mut usize,
) {
    let inv_arc_length = 1.0f32 / arc_length;
    let mut k: i32 = 0;
    for (point, multiplier) in points_to_draw {
        let v = (k as f32 * K_DESIRED_RENDERING_DISTANCE) * inv_arc_length;
        let progress_along_arc = if v < 1.0f32 { v } else { 1.0f32 };
        k += 1;
        let mut color = [0f32; 3];
        for c in 0..3 {
            color[c] = continuous_idct(&spline.color_dct[c], (32 - 1) as f32 * progress_along_arc);
        }
        let sigma = continuous_idct(&spline.sigma_dct, (32 - 1) as f32 * progress_along_arc);
        compute_segments(point, *multiplier, &color, sigma, segments, segments_by_y, pixel_limit);
        if *pixel_limit == 0 {
            return;
        }
    }
}

// It is not in spec, but reasonable limit to avoid overflows.
fn validate_spline_point_pos_i64(x: i64, y: i64) -> Status {
    const K_SPLINE_POS_LIMIT: i64 = 1 << 23;
    if x >= K_SPLINE_POS_LIMIT || x <= -K_SPLINE_POS_LIMIT || y >= K_SPLINE_POS_LIMIT || y <= -K_SPLINE_POS_LIMIT {
        return jxl_failure!("Spline coordinates out of bounds");
    }
    Ok(())
}

fn validate_spline_point_pos_f32(x: f32, y: f32) -> Status {
    const K_SPLINE_POS_LIMIT: f32 = (1u32 << 23) as f32;
    if x >= K_SPLINE_POS_LIMIT || x <= -K_SPLINE_POS_LIMIT || y >= K_SPLINE_POS_LIMIT || y <= -K_SPLINE_POS_LIMIT {
        return jxl_failure!("Spline coordinates out of bounds");
    }
    Ok(())
}

// Maximum number of spline control points per frame is
//   std::min(kMaxNumControlPoints, xsize * ysize / 2)
const K_MAX_NUM_CONTROL_POINTS: usize = 1 << 20;
const K_MAX_NUM_CONTROL_POINTS_PER_PIXEL_RATIO: usize = 2;

fn inv_adjusted_quant(adjustment: i32) -> f32 {
    if adjustment >= 0 {
        1.0f32 / (1.0f32 + 0.125f32 * adjustment as f32)
    } else {
        1.0f32 - 0.125f32 * adjustment as f32
    }
}

// X, Y, B, sigma.
const K_CHANNEL_WEIGHT: [f32; 4] = [0.0042, 0.075, 0.07, 0.3333];

/// Translation of `DecodeAllStartingPoints()`.
fn decode_all_starting_points(
    points: &mut Vec<Point>,
    br: &mut BitReader<'_>,
    reader: &mut AnsSymbolReader<'_>,
    context_map: &[u8],
    num_splines: usize,
) -> Status {
    points.clear();
    if points.try_reserve(num_splines).is_err() {
        return jxl_failure!("out of memory");
    }
    let mut last_x: i64 = 0;
    let mut last_y: i64 = 0;
    for i in 0..num_splines {
        let mut x = reader.read_hybrid_uint(K_STARTING_POSITION_CONTEXT, br, context_map) as i64;
        let mut y = reader.read_hybrid_uint(K_STARTING_POSITION_CONTEXT, br, context_map) as i64;
        if i != 0 {
            x = (unpack_signed(x as usize) as i64).wrapping_add(last_x);
            y = (unpack_signed(y as usize) as i64).wrapping_add(last_y);
        }
        validate_spline_point_pos_i64(x, y)?;
        points.push(Point { x: x as f32, y: y as f32 });
        last_x = x;
        last_y = y;
    }
    Ok(())
}

impl Point {
    fn sub(&self, b: &Point) -> (f32, f32) {
        (self.x - b.x, self.y - b.y)
    }
    fn add_scaled(&self, k: f32, v: (f32, f32)) -> Point {
        Point {
            x: self.x + k * v.0,
            y: self.y + k * v.1,
        }
    }
}

// TODO(eustas): avoid making a copy of "points".
/// Translation of `DrawCentripetalCatmullRomSpline()`.
fn draw_centripetal_catmull_rom_spline(mut points: Vec<Point>, result: &mut Vec<Point>) {
    if points.is_empty() {
        return;
    }
    if points.len() == 1 {
        result.push(points[0]);
        return;
    }
    // Number of points to compute between each control point.
    const K_NUM_POINTS: i32 = 16;
    result.reserve((points.len() - 1) * K_NUM_POINTS as usize + 1);
    let first = points[0].add_scaled(1.0, points[0].sub(&points[1]));
    points.insert(0, first);
    let n = points.len();
    let last = points[n - 1].add_scaled(1.0, points[n - 1].sub(&points[n - 2]));
    points.push(last);
    // points has at least 4 elements at this point.
    for start in 0..points.len() - 3 {
        // 4 of them are used, and we draw from p[1] to p[2].
        let p = &points[start..start + 4];
        result.push(p[1]);
        let mut d = [0f32; 3];
        let mut t = [0f32; 4];
        t[0] = 0.0;
        for k in 0..3 {
            // TODO(eustas): for each segment delta is calculated 3 times...
            // TODO(eustas): restrict d[k] with reasonable limit and spec it.
            d[k] = hypotf(p[k + 1].x - p[k].x, p[k + 1].y - p[k].y).sqrt();
            t[k + 1] = t[k] + d[k];
        }
        for i in 1..K_NUM_POINTS {
            let tt = d[0] + (i as f32 / K_NUM_POINTS as f32) * d[1];
            let mut a = [Point::default(); 3];
            for k in 0..3 {
                // TODO(eustas): reciprocal multiplication would be faster.
                a[k] = p[k].add_scaled((tt - t[k]) / d[k], p[k + 1].sub(&p[k]));
            }
            let mut b = [Point::default(); 2];
            for k in 0..2 {
                b[k] = a[k].add_scaled((tt - t[k]) / (d[k] + d[k + 1]), a[k + 1].sub(&a[k]));
            }
            result.push(b[0].add_scaled((tt - t[1]) / d[1], b[1].sub(&b[0])));
        }
    }
    result.push(points[points.len() - 2]);
}

// Move along the line segments defined by `points`, `kDesiredRenderingDistance`
// pixels at a time, and call `functor` with each point and the actual distance
// to the previous point (which will always be kDesiredRenderingDistance except
// possibly for the very last point).
// TODO(eustas): this method always adds the last point, but never the first
//               (unless those are one); I believe both ends matter.
/// Translation of `ForEachEquallySpacedPoint()`.
fn for_each_equally_spaced_point(points: &[Point], functor: &mut dyn FnMut(&Point, f32) -> bool) -> bool {
    debug_assert!(!points.is_empty());
    let mut current = points[0];
    functor(&current, K_DESIRED_RENDERING_DISTANCE);
    let mut next = 0usize;
    while next != points.len() {
        let mut previous = current;
        let mut arclength_from_previous = 0.0f32;
        loop {
            if next == points.len() {
                return functor(&previous, arclength_from_previous);
            }
            let dv = points[next].sub(&previous);
            let arclength_to_next = (dv.0 * dv.0 + dv.1 * dv.1).sqrt();
            if arclength_from_previous + arclength_to_next >= K_DESIRED_RENDERING_DISTANCE {
                current = previous.add_scaled(
                    (K_DESIRED_RENDERING_DISTANCE - arclength_from_previous) / arclength_to_next,
                    points[next].sub(&previous),
                );
                if !functor(&current, K_DESIRED_RENDERING_DISTANCE) {
                    return false;
                }
                break;
            }
            arclength_from_previous += arclength_to_next;
            previous = points[next];
            next += 1;
        }
    }
    true
}

impl QuantizedSpline {
    /// Translation of `QuantizedSpline::Dequantize()`.
    fn dequantize(
        &self,
        starting_point: &Point,
        quantization_adjustment: i32,
        y_to_x: f32,
        y_to_b: f32,
        result: &mut Spline,
    ) -> Status {
        result.control_points.clear();
        if result.control_points.try_reserve(self.control_points.len() + 1).is_err() {
            return jxl_failure!("out of memory");
        }
        let px = roundf(starting_point.x);
        let py = roundf(starting_point.y);
        validate_spline_point_pos_f32(px, py)?;
        let mut current_x = px as i32;
        let mut current_y = py as i32;
        result.control_points.push(Point {
            x: current_x as f32,
            y: current_y as f32,
        });
        let mut current_delta_x: i32 = 0;
        let mut current_delta_y: i32 = 0;
        for point in &self.control_points {
            current_delta_x = (current_delta_x as i64).wrapping_add(point.0) as i32;
            current_delta_y = (current_delta_y as i64).wrapping_add(point.1) as i32;
            validate_spline_point_pos_i64(current_delta_x as i64, current_delta_y as i64)?;
            current_x += current_delta_x;
            current_y += current_delta_y;
            validate_spline_point_pos_i64(current_x as i64, current_y as i64)?;
            result.control_points.push(Point {
                x: current_x as f32,
                y: current_y as f32,
            });
        }

        let inv_quant = inv_adjusted_quant(quantization_adjustment);
        for c in 0..3 {
            for i in 0..32 {
                let inv_dct_factor = if i == 0 { K_SQRT0_5 } else { 1.0f32 };
                result.color_dct[c][i] =
                    self.color_dct[c][i] as f32 * inv_dct_factor * K_CHANNEL_WEIGHT[c] * inv_quant;
            }
        }
        for i in 0..32 {
            result.color_dct[0][i] += y_to_x * result.color_dct[1][i];
            result.color_dct[2][i] += y_to_b * result.color_dct[1][i];
        }
        for i in 0..32 {
            let inv_dct_factor = if i == 0 { K_SQRT0_5 } else { 1.0f32 };
            result.sigma_dct[i] = self.sigma_dct[i] as f32 * inv_dct_factor * K_CHANNEL_WEIGHT[3] * inv_quant;
        }

        Ok(())
    }

    /// Translation of `QuantizedSpline::Decode()`.
    fn decode(
        &mut self,
        context_map: &[u8],
        decoder: &mut AnsSymbolReader<'_>,
        br: &mut BitReader<'_>,
        max_control_points: usize,
        total_num_control_points: &mut usize,
    ) -> Status {
        let num_control_points = decoder.read_hybrid_uint(K_NUM_CONTROL_POINTS_CONTEXT, br, context_map);
        *total_num_control_points = total_num_control_points.wrapping_add(num_control_points);
        if *total_num_control_points > max_control_points {
            return jxl_failure!("Too many control points");
        }
        self.control_points.clear();
        if self.control_points.try_reserve(num_control_points).is_err() {
            return jxl_failure!("out of memory");
        }
        self.control_points.resize(num_control_points, (0, 0));
        // Maximal image dimension.
        const K_DELTA_LIMIT: i64 = 1 << 30;
        for control_point in self.control_points.iter_mut() {
            control_point.0 = unpack_signed(decoder.read_hybrid_uint(K_CONTROL_POINTS_CONTEXT, br, context_map)) as i64;
            control_point.1 = unpack_signed(decoder.read_hybrid_uint(K_CONTROL_POINTS_CONTEXT, br, context_map)) as i64;
            // Check delta-deltas are not outrageous; it is not in spec, but there is
            // no reason to allow larger values.
            if control_point.0 >= K_DELTA_LIMIT
                || control_point.0 <= -K_DELTA_LIMIT
                || control_point.1 >= K_DELTA_LIMIT
                || control_point.1 <= -K_DELTA_LIMIT
            {
                return jxl_failure!("Spline delta-delta is out of bounds");
            }
        }

        let mut decode_dct = |dct: &mut [i32; 32], br: &mut BitReader<'_>| {
            for d in dct.iter_mut() {
                *d = unpack_signed(decoder.read_hybrid_uint(K_DCT_CONTEXT, br, context_map)) as i32;
            }
        };
        for c in 0..3 {
            decode_dct(&mut self.color_dct[c], br);
        }
        decode_dct(&mut self.sigma_dct, br);
        Ok(())
    }
}

impl Splines {
    #[allow(dead_code)]
    pub(crate) fn has_any(&self) -> bool {
        !self.splines.is_empty()
    }

    /// Translation of `Splines::Clear()`.
    pub(crate) fn clear(&mut self) {
        self.quantization_adjustment = 0;
        self.splines.clear();
        self.starting_points.clear();
        self.segments.clear();
        self.segment_indices.clear();
        self.segment_y_start.clear();
    }

    /// Translation of `Splines::Decode()`.
    pub(crate) fn decode(&mut self, br: &mut BitReader<'_>, num_pixels: usize) -> Status {
        let mut context_map: Vec<u8> = Vec::new();
        let mut code = AnsCode::default();
        decode_histograms(br, K_NUM_SPLINE_CONTEXTS, &mut code, &mut context_map, false)?;
        let mut decoder = AnsSymbolReader::new(&code, br, 0);
        let num_splines = decoder
            .read_hybrid_uint(K_NUM_SPLINES_CONTEXT, br, &context_map)
            .wrapping_add(1);
        let max_control_points = K_MAX_NUM_CONTROL_POINTS.min(num_pixels / K_MAX_NUM_CONTROL_POINTS_PER_PIXEL_RATIO);
        if num_splines > max_control_points {
            return jxl_failure!("Too many splines");
        }
        decode_all_starting_points(&mut self.starting_points, br, &mut decoder, &context_map, num_splines)?;

        self.quantization_adjustment =
            unpack_signed(decoder.read_hybrid_uint(K_QUANTIZATION_ADJUSTMENT_CONTEXT, br, &context_map)) as i32;

        self.splines.clear();
        if self.splines.try_reserve(num_splines).is_err() {
            return jxl_failure!("out of memory");
        }
        let mut num_control_points = num_splines;
        for _ in 0..num_splines {
            let mut spline = QuantizedSpline::default();
            spline.decode(&context_map, &mut decoder, br, max_control_points, &mut num_control_points)?;
            self.splines.push(spline);
        }

        if !decoder.check_ans_final_state() {
            return jxl_failure!("ANS checksum failure");
        }

        if !self.has_any() {
            return jxl_failure!("Decoded splines but got none");
        }

        Ok(())
    }

    /// Translation of `Splines::AddToRow()` (the rows hold the pixels of
    /// `image_row`).
    pub(crate) fn add_to_row(&self, rows: &mut [&mut [f32]; 3], x0: usize, y: usize, xsize: usize) {
        self.apply_to_row(true, rows, x0, y, xsize);
    }

    /// Translation of `Splines::InitializeDrawCache()` (with the color
    /// correlation map's `YtoXRatio(0)` and `YtoBRatio(0)`).
    pub(crate) fn initialize_draw_cache(
        &mut self,
        image_xsize: usize,
        image_ysize: usize,
        y_to_x: f32,
        y_to_b: f32,
    ) -> Status {
        // TODO(veluca): avoid storing segments that are entirely outside image
        // boundaries.
        self.segments.clear();
        self.segment_indices.clear();
        self.segment_y_start.clear();
        let mut segments_by_y: Vec<(usize, usize)> = Vec::new();
        let mut spline = Spline {
            control_points: Vec::new(),
            color_dct: [[0.0; 32]; 3],
            sigma_dct: [0.0; 32],
        };
        // TODO(eustas): not in the spec; limit spline pixels with image area.
        let pixel_limit = 16.0f32 * image_xsize as f32 * image_ysize as f32 + (1 << 16) as f32;
        // Apply some extra cap to avoid overflows.
        const K_HARD_PIXEL_LIMIT: usize = 1 << 30;
        let mut px_limit = if pixel_limit < K_HARD_PIXEL_LIMIT as f32 {
            pixel_limit as usize
        } else {
            K_HARD_PIXEL_LIMIT
        };
        let mut intermediate_points: Vec<Point> = Vec::new();
        for i in 0..self.splines.len() {
            self.splines[i].dequantize(
                &self.starting_points[i],
                self.quantization_adjustment,
                y_to_x,
                y_to_b,
                &mut spline,
            )?;
            if spline.control_points.windows(2).any(|w| w[0] == w[1]) {
                // Otherwise division by zero might occur. Once control points coincide,
                // the direction of curve is undefined...
                return jxl_failure!("identical successive control points in spline");
            }
            let mut points_to_draw: Vec<(Point, f32)> = Vec::new();
            let mut add_point = |point: &Point, multiplier: f32| -> bool {
                points_to_draw.push((*point, multiplier));
                points_to_draw.len() <= px_limit
            };
            intermediate_points.clear();
            draw_centripetal_catmull_rom_spline(spline.control_points.clone(), &mut intermediate_points);
            if !for_each_equally_spaced_point(&intermediate_points, &mut add_point) {
                return jxl_failure!("Too many pixels covered with splines");
            }
            let arc_length = (points_to_draw.len().wrapping_sub(2)) as f32 * K_DESIRED_RENDERING_DISTANCE
                + points_to_draw.last().map_or(0.0, |p| p.1);
            if arc_length <= 0.0f32 {
                // This spline wouldn't have any effect.
                continue;
            }
            segments_from_points(
                &spline,
                &points_to_draw,
                arc_length,
                &mut self.segments,
                &mut segments_by_y,
                &mut px_limit,
            );
            if px_limit == 0 {
                return jxl_failure!("Too many pixels covered with splines");
            }
        }
        // TODO(eustas): consider linear sorting here.
        segments_by_y.sort_unstable();
        self.segment_indices.resize(segments_by_y.len(), 0);
        self.segment_y_start.resize(image_ysize + 1, 0);
        for (i, sy) in segments_by_y.iter().enumerate() {
            self.segment_indices[i] = sy.1;
            let y = sy.0;
            if y < image_ysize {
                self.segment_y_start[y + 1] += 1;
            }
        }
        for y in 0..image_ysize {
            self.segment_y_start[y + 1] += self.segment_y_start[y];
        }
        Ok(())
    }

    /// Translation of `Splines::ApplyToRow()` with `DrawSegments()`.
    fn apply_to_row(&self, add: bool, rows: &mut [&mut [f32]; 3], x0: usize, y: usize, xsize: usize) {
        if self.segments.is_empty() {
            return;
        }
        for i in self.segment_y_start[y]..self.segment_y_start[y + 1] {
            draw_segment(
                &self.segments[self.segment_indices[i]],
                add,
                y,
                x0 as i64,
                (x0 + xsize) as i64,
                rows,
            );
        }
    }
}
