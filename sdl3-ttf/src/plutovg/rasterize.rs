// Rust translation of source/plutovg-rasterize.c (and the span and stroke
// parts of source/plutovg-private.h) from PlutoVG (0.0.12, as SDL_ttf's
// external/plutovg pins it).
// Copyright (c) 2020-2024 Samuel Ugochukwu <sammycageagle@gmail.com>
// This is an altered (translated) version of the original software; it is
// used under the MIT License (see PLUTOVG-LICENSE and LICENSE.txt).

//! Span buffers, and the rasterization of paths (filled, or stroked with
//! dashes) into spans.

use super::ft_math::*;
use super::ft_raster::*;
use super::ft_stroker::*;
use super::libm::hypot;
use super::path::*;
use super::*;

/// `plutovg_span_t`
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Span {
    pub x: i32,
    pub len: i32,
    pub y: i32,
    pub coverage: u8,
}

/// `plutovg_span_buffer_t`
#[derive(Debug, Clone, Default)]
pub struct SpanBuffer {
    pub spans: Vec<Span>,

    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
}

/// `plutovg_stroke_dash_t`
#[derive(Debug, Clone, Default)]
pub struct StrokeDash {
    pub offset: f32,
    pub array: Vec<f32>,
}

/// `plutovg_stroke_style_t`
#[derive(Debug, Clone, Copy)]
pub struct StrokeStyle {
    pub width: f32,
    pub cap: LineCap,
    pub join: LineJoin,
    pub miter_limit: f32,
}

/// `plutovg_stroke_data_t`
#[derive(Debug, Clone)]
pub struct StrokeData {
    pub style: StrokeStyle,
    pub dash: StrokeDash,
}

/// `plutovg_span_buffer_init`
pub fn plutovg_span_buffer_init() -> SpanBuffer {
    let mut span_buffer = SpanBuffer::default();
    plutovg_span_buffer_reset(&mut span_buffer);
    span_buffer
}

/// `plutovg_span_buffer_init_rect`
pub fn plutovg_span_buffer_init_rect(
    span_buffer: &mut SpanBuffer,
    x: i32,
    y: i32,
    width: i32,
    height: i32,
) {
    span_buffer.spans.clear();
    for i in 0..height {
        span_buffer.spans.push(Span {
            x,
            y: y + i,
            len: width,
            coverage: 255,
        });
    }

    span_buffer.x = x;
    span_buffer.y = y;
    span_buffer.w = width;
    span_buffer.h = height;
}

/// `plutovg_span_buffer_reset`
pub fn plutovg_span_buffer_reset(span_buffer: &mut SpanBuffer) {
    span_buffer.spans.clear();
    span_buffer.x = 0;
    span_buffer.y = 0;
    span_buffer.w = -1;
    span_buffer.h = -1;
}

/// `plutovg_span_buffer_copy`
pub fn plutovg_span_buffer_copy(span_buffer: &mut SpanBuffer, source: &SpanBuffer) {
    span_buffer.spans.clear();
    span_buffer.spans.extend_from_slice(&source.spans);
    span_buffer.x = source.x;
    span_buffer.y = source.y;
    span_buffer.w = source.w;
    span_buffer.h = source.h;
}

/// `plutovg_span_buffer_update_extents`
fn plutovg_span_buffer_update_extents(span_buffer: &mut SpanBuffer) {
    if span_buffer.w != -1 && span_buffer.h != -1 {
        return;
    }
    if span_buffer.spans.is_empty() {
        span_buffer.x = 0;
        span_buffer.y = 0;
        span_buffer.w = 0;
        span_buffer.h = 0;
        return;
    }

    let spans = &span_buffer.spans;
    let mut x1 = i32::MAX;
    let y1 = spans[0].y;
    let mut x2 = 0;
    let y2 = spans[spans.len() - 1].y;
    for s in spans {
        if s.x < x1 {
            x1 = s.x;
        }
        if s.x.wrapping_add(s.len) > x2 {
            x2 = s.x.wrapping_add(s.len);
        }
    }

    span_buffer.x = x1;
    span_buffer.y = y1;
    span_buffer.w = x2.wrapping_sub(x1);
    span_buffer.h = y2.wrapping_sub(y1).wrapping_add(1);
}

/// `plutovg_span_buffer_extents`
pub fn plutovg_span_buffer_extents(span_buffer: &mut SpanBuffer, extents: &mut Rect) {
    plutovg_span_buffer_update_extents(span_buffer);
    extents.x = span_buffer.x as f32;
    extents.y = span_buffer.y as f32;
    extents.w = span_buffer.w as f32;
    extents.h = span_buffer.h as f32;
}

/// `plutovg_span_buffer_intersect`
pub fn plutovg_span_buffer_intersect(span_buffer: &mut SpanBuffer, a: &SpanBuffer, b: &SpanBuffer) {
    plutovg_span_buffer_reset(span_buffer);
    let _ = span_buffer
        .spans
        .try_reserve(plutovg_max(a.spans.len(), b.spans.len()));

    let mut ai = 0;
    let a_end = a.spans.len();

    let mut bi = 0;
    let b_end = b.spans.len();
    while ai < a_end && bi < b_end {
        let a_spans = a.spans[ai];
        let b_spans = b.spans[bi];
        if b_spans.y > a_spans.y {
            ai += 1;
            continue;
        }

        if a_spans.y != b_spans.y {
            bi += 1;
            continue;
        }

        let ax1 = a_spans.x;
        let ax2 = ax1.wrapping_add(a_spans.len);
        let bx1 = b_spans.x;
        let bx2 = bx1.wrapping_add(b_spans.len);
        if bx1 < ax1 && bx2 < ax1 {
            bi += 1;
            continue;
        }

        if ax1 < bx1 && ax2 < bx1 {
            ai += 1;
            continue;
        }

        let x = plutovg_max(ax1, bx1);
        let len = plutovg_min(ax2, bx2).wrapping_sub(x);
        if len != 0 {
            span_buffer.spans.push(Span {
                x,
                len,
                y: a_spans.y,
                coverage: plutovg_div255(a_spans.coverage as u32 * b_spans.coverage as u32) as u8,
            });
        }

        if ax2 < bx2 {
            ai += 1;
        } else {
            bi += 1;
        }
    }
}

/// `ft_outline_create`: an outline with room for `points + contours`
/// points and `contours` contours (`None` if it can't be allocated).
fn ft_outline_create(points: i32, contours: i32) -> Option<PvgFtOutline> {
    let npoints = (points.max(0) as usize).checked_add(contours.max(0) as usize)?;
    let ncontours = contours.max(0) as usize;
    let mut outline = PvgFtOutline::default();
    outline.points.try_reserve_exact(npoints).ok()?;
    outline.points.resize(npoints, PvgFtVector::default());
    outline.tags.try_reserve_exact(npoints).ok()?;
    outline.tags.resize(npoints, 0);
    outline.contours.try_reserve_exact(ncontours).ok()?;
    outline.contours.resize(ncontours, 0);
    outline.contours_flag.try_reserve_exact(ncontours).ok()?;
    outline.contours_flag.resize(ncontours, 0);
    outline.n_points = 0;
    outline.n_contours = 0;
    outline.flags = 0x0;
    Some(outline)
}

/// `FT_COORD`
#[inline]
fn ft_coord(x: f32) -> PvgFtPos {
    cvt_f32_i64(x * 64.0)
}

/// Stores `value` at `index` of `v` (C writes past the end where its
/// sizes are wrong; they never are here).
#[inline]
fn put<T>(v: &mut [T], index: i32, value: T) {
    if let Some(slot) = v.get_mut(index.max(0) as usize) {
        if index >= 0 {
            *slot = value;
        }
    }
}

/// `ft_outline_move_to`
fn ft_outline_move_to(ft: &mut PvgFtOutline, x: f32, y: f32) {
    put(
        &mut ft.points,
        ft.n_points,
        PvgFtVector {
            x: ft_coord(x),
            y: ft_coord(y),
        },
    );
    put(&mut ft.tags, ft.n_points, PVG_FT_CURVE_TAG_ON);
    if ft.n_points != 0 {
        put(&mut ft.contours, ft.n_contours, ft.n_points - 1);
        ft.n_contours += 1;
    }

    put(&mut ft.contours_flag, ft.n_contours, 1);
    ft.n_points += 1;
}

/// `ft_outline_line_to`
fn ft_outline_line_to(ft: &mut PvgFtOutline, x: f32, y: f32) {
    put(
        &mut ft.points,
        ft.n_points,
        PvgFtVector {
            x: ft_coord(x),
            y: ft_coord(y),
        },
    );
    put(&mut ft.tags, ft.n_points, PVG_FT_CURVE_TAG_ON);
    ft.n_points += 1;
}

/// `ft_outline_cubic_to`
fn ft_outline_cubic_to(
    ft: &mut PvgFtOutline,
    x1: f32,
    y1: f32,
    x2: f32,
    y2: f32,
    x3: f32,
    y3: f32,
) {
    put(
        &mut ft.points,
        ft.n_points,
        PvgFtVector {
            x: ft_coord(x1),
            y: ft_coord(y1),
        },
    );
    put(&mut ft.tags, ft.n_points, PVG_FT_CURVE_TAG_CUBIC);
    ft.n_points += 1;

    put(
        &mut ft.points,
        ft.n_points,
        PvgFtVector {
            x: ft_coord(x2),
            y: ft_coord(y2),
        },
    );
    put(&mut ft.tags, ft.n_points, PVG_FT_CURVE_TAG_CUBIC);
    ft.n_points += 1;

    put(
        &mut ft.points,
        ft.n_points,
        PvgFtVector {
            x: ft_coord(x3),
            y: ft_coord(y3),
        },
    );
    put(&mut ft.tags, ft.n_points, PVG_FT_CURVE_TAG_ON);
    ft.n_points += 1;
}

/// `ft_outline_close`
fn ft_outline_close(ft: &mut PvgFtOutline) {
    put(&mut ft.contours_flag, ft.n_contours, 0);
    let index = if ft.n_contours != 0 {
        ft.contours
            .get((ft.n_contours - 1) as usize)
            .copied()
            .unwrap_or(-1)
            + 1
    } else {
        0
    };
    if index == ft.n_points {
        return;
    }
    let p = ft
        .points
        .get(index.max(0) as usize)
        .copied()
        .unwrap_or_default();
    put(&mut ft.points, ft.n_points, p);
    put(&mut ft.tags, ft.n_points, PVG_FT_CURVE_TAG_ON);
    ft.n_points += 1;
}

/// `ft_outline_end`
fn ft_outline_end(ft: &mut PvgFtOutline) {
    if ft.n_points != 0 {
        put(&mut ft.contours, ft.n_contours, ft.n_points - 1);
        ft.n_contours += 1;
    }
}

/// `ft_outline_convert`
fn ft_outline_convert(
    path: &Path,
    matrix: &Matrix,
    stroke_data: Option<&StrokeData>,
) -> Option<PvgFtOutline> {
    if let Some(stroke_data) = stroke_data {
        return ft_outline_convert_stroke(path, matrix, stroke_data);
    }

    let mut it = plutovg_path_iterator_init(path);

    let mut points = [Point::default(); 3];
    let mut outline = ft_outline_create(path.num_points, path.num_contours)?;
    while plutovg_path_iterator_has_next(&it) {
        match plutovg_path_iterator_next(&mut it, &mut points) {
            PathCommand::MoveTo => {
                plutovg_matrix_map_points(matrix, &mut points[..1]);
                ft_outline_move_to(&mut outline, points[0].x, points[0].y);
            }
            PathCommand::LineTo => {
                plutovg_matrix_map_points(matrix, &mut points[..1]);
                ft_outline_line_to(&mut outline, points[0].x, points[0].y);
            }
            PathCommand::CubicTo => {
                plutovg_matrix_map_points(matrix, &mut points[..3]);
                ft_outline_cubic_to(
                    &mut outline,
                    points[0].x,
                    points[0].y,
                    points[1].x,
                    points[1].y,
                    points[2].x,
                    points[2].y,
                );
            }
            PathCommand::Close => {
                ft_outline_close(&mut outline);
            }
        }
    }

    ft_outline_end(&mut outline);
    Some(outline)
}

/// `ft_outline_convert_dash`
fn ft_outline_convert_dash(
    path: &Path,
    matrix: &Matrix,
    stroke_dash: &StrokeDash,
) -> Option<PvgFtOutline> {
    if stroke_dash.array.is_empty() {
        return ft_outline_convert(path, matrix, None);
    }
    let dashed = plutovg_path_clone_dashed(path, stroke_dash.offset, &stroke_dash.array);
    ft_outline_convert(&dashed, matrix, None)
}

/// `ft_outline_convert_stroke`
fn ft_outline_convert_stroke(
    path: &Path,
    matrix: &Matrix,
    stroke_data: &StrokeData,
) -> Option<PvgFtOutline> {
    let scale_x = ((matrix.a * matrix.a + matrix.b * matrix.b) as f64).sqrt();
    let scale_y = ((matrix.c * matrix.c + matrix.d * matrix.d) as f64).sqrt();

    let scale = hypot(scale_x, scale_y) / PLUTOVG_SQRT2 as f64;
    let width = stroke_data.style.width as f64 * scale;

    let ft_width: PvgFtFixed = cvt_f64_i64(width * 0.5 * (1 << 6) as f64);
    let ft_miter_limit: PvgFtFixed = cvt_f32_i64(stroke_data.style.miter_limit * (1 << 16) as f32);

    let ft_cap = match stroke_data.style.cap {
        LineCap::Square => PvgFtStrokerLineCap::Square,
        LineCap::Round => PvgFtStrokerLineCap::Round,
        _ => PvgFtStrokerLineCap::Butt,
    };

    let ft_join = match stroke_data.style.join {
        LineJoin::Bevel => PvgFtStrokerLineJoin::Bevel,
        LineJoin::Round => PvgFtStrokerLineJoin::Round,
        _ => PvgFtStrokerLineJoin::MiterFixed,
    };

    let mut stroker = pvg_ft_stroker_new();
    pvg_ft_stroker_set(&mut stroker, ft_width, ft_cap, ft_join, ft_miter_limit);

    let outline = ft_outline_convert_dash(path, matrix, &stroke_data.dash)?;
    pvg_ft_stroker_parse_outline(&mut stroker, &outline);

    let mut points: PvgFtUInt = 0;
    let mut contours: PvgFtUInt = 0;
    pvg_ft_stroker_get_counts(&mut stroker, &mut points, &mut contours);

    let mut stroke_outline = ft_outline_create(points as i32, contours as i32)?;
    pvg_ft_stroker_export(&stroker, &mut stroke_outline);

    Some(stroke_outline)
}

/// `plutovg_rasterize`
pub fn plutovg_rasterize(
    span_buffer: &mut SpanBuffer,
    path: &Path,
    matrix: &Matrix,
    clip_rect: Option<&Rect>,
    stroke_data: Option<&StrokeData>,
    winding: FillRule,
) {
    plutovg_span_buffer_reset(span_buffer);
    let Some(mut outline) = ft_outline_convert(path, matrix, stroke_data) else {
        return;
    };
    if stroke_data.is_some() {
        outline.flags = PVG_FT_OUTLINE_NONE;
    } else {
        outline.flags = match winding {
            FillRule::EvenOdd => PVG_FT_OUTLINE_EVEN_ODD_FILL,
            _ => PVG_FT_OUTLINE_NONE,
        };
    }

    let spans = &mut span_buffer.spans;
    /* spans_generation_callback */
    let mut callback = |s: &[PvgFtSpan]| {
        spans.extend(s.iter().map(|s| Span {
            x: s.x,
            len: s.len,
            y: s.y,
            coverage: s.coverage,
        }));
    };
    let mut params = PvgFtRasterParams {
        source: Some(&outline),
        flags: PVG_FT_RASTER_FLAG_DIRECT | PVG_FT_RASTER_FLAG_AA,
        gray_spans: Some(&mut callback),
        clip_box: PvgFtBBox::default(),
    };
    if let Some(clip_rect) = clip_rect {
        params.flags |= PVG_FT_RASTER_FLAG_CLIP;
        params.clip_box.x_min = cvt_f32_i64(clip_rect.x);
        params.clip_box.y_min = cvt_f32_i64(clip_rect.y);
        params.clip_box.x_max = cvt_f32_i64(clip_rect.x + clip_rect.w);
        params.clip_box.y_max = cvt_f32_i64(clip_rect.y + clip_rect.h);
    }

    pvg_ft_raster_render(&mut params);
}
