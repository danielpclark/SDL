// Rust translation of source/plutovg-canvas.c (and the canvas parts of
// source/plutovg-private.h) from PlutoVG (0.0.12, as SDL_ttf's
// external/plutovg pins it).
// Copyright (c) 2020-2024 Samuel Ugochukwu <sammycageagle@gmail.com>
// This is an altered (translated) version of the original software; it is
// used under the MIT License (see PLUTOVG-LICENSE and LICENSE.txt).

//! The canvas: its state stack, paint, stroke, transformation, path, and
//! fill, stroke, clip and paint operations. (The font and text functions
//! are not translated: see the module documentation.)

use std::rc::Rc;

use super::blend::plutovg_blend;
use super::matrix::*;
use super::paint::*;
use super::path::*;
use super::rasterize::*;
use super::surface::Surface;
use super::*;

/// `PLUTOVG_DEFAULT_STROKE_STYLE`
const PLUTOVG_DEFAULT_STROKE_STYLE: StrokeStyle = StrokeStyle {
    width: 1.0,
    cap: LineCap::Butt,
    join: LineJoin::Miter,
    miter_limit: 10.0,
};

/// `plutovg_state_t` (the stack of saved states is the canvas's vector)
#[derive(Debug, Clone)]
pub struct State {
    pub paint: Option<Rc<Paint>>,
    pub color: Color,
    pub matrix: Matrix,
    pub stroke: StrokeData,
    pub clip_spans: SpanBuffer,
    pub winding: FillRule,
    pub op: Operator,
    pub font_size: f32,
    pub opacity: f32,
    pub clipping: bool,
}

/// `plutovg_state_create`
fn plutovg_state_create() -> State {
    State {
        paint: None,
        color: PLUTOVG_BLACK_COLOR,
        matrix: PLUTOVG_IDENTITY_MATRIX,
        stroke: StrokeData {
            style: PLUTOVG_DEFAULT_STROKE_STYLE,
            dash: StrokeDash {
                offset: 0.0,
                array: Vec::new(),
            },
        },
        clip_spans: plutovg_span_buffer_init(),
        winding: FillRule::NonZero,
        op: Operator::SrcOver,
        font_size: 12.0,
        opacity: 1.0,
        clipping: false,
    }
}

/// `struct plutovg_canvas`
#[derive(Debug)]
pub struct Canvas {
    pub surface: Surface,
    pub path: Path,
    /// the current state (C's `state`) is the last one
    pub states: Vec<State>,
    pub clip_rect: Rect,
    pub clip_spans: SpanBuffer,
    pub fill_spans: SpanBuffer,
}

impl Canvas {
    /// `canvas->state`
    #[inline]
    pub fn state(&self) -> &State {
        self.states.last().expect("a canvas always has a state")
    }

    /// `canvas->state`
    #[inline]
    pub fn state_mut(&mut self) -> &mut State {
        self.states.last_mut().expect("a canvas always has a state")
    }
}

/// Creates a drawing context on a surface (taking it: see
/// [`plutovg_canvas_into_surface`]).
pub fn plutovg_canvas_create(surface: Surface) -> Canvas {
    let clip_rect = plutovg_make_rect(0.0, 0.0, surface.width as f32, surface.height as f32);
    Canvas {
        surface,
        path: plutovg_path_create(),
        states: vec![plutovg_state_create()],
        clip_rect,
        clip_spans: plutovg_span_buffer_init(),
        fill_spans: plutovg_span_buffer_init(),
    }
}

/// The surface of the canvas (`plutovg_canvas_destroy`, giving back the
/// surface the canvas took).
pub fn plutovg_canvas_into_surface(canvas: Canvas) -> Surface {
    canvas.surface
}

/// Gets the surface associated with the canvas.
pub fn plutovg_canvas_get_surface(canvas: &Canvas) -> &Surface {
    &canvas.surface
}

/// Saves the current state of the canvas.
pub fn plutovg_canvas_save(canvas: &mut Canvas) {
    let new_state = canvas.state().clone();
    canvas.states.push(new_state);
}

/// Restores the canvas to the most recently saved state.
pub fn plutovg_canvas_restore(canvas: &mut Canvas) {
    if canvas.states.len() <= 1 {
        return;
    }
    canvas.states.pop();
}

/// Sets the current paint to a solid color (alpha 1).
pub fn plutovg_canvas_set_rgb(canvas: &mut Canvas, r: f32, g: f32, b: f32) {
    plutovg_canvas_set_rgba(canvas, r, g, b, 1.0);
}

/// Sets the current paint to a solid color.
pub fn plutovg_canvas_set_rgba(canvas: &mut Canvas, r: f32, g: f32, b: f32, a: f32) {
    plutovg_color_init_rgba(&mut canvas.state_mut().color, r, g, b, a);
    plutovg_canvas_set_paint(canvas, None);
}

/// Sets the current paint to a solid color.
pub fn plutovg_canvas_set_color(canvas: &mut Canvas, color: &Color) {
    plutovg_canvas_set_rgba(canvas, color.r, color.g, color.b, color.a);
}

/// Sets the current paint to a linear gradient.
pub fn plutovg_canvas_set_linear_gradient(
    canvas: &mut Canvas,
    x1: f32,
    y1: f32,
    x2: f32,
    y2: f32,
    spread: SpreadMethod,
    stops: &[GradientStop],
    matrix: Option<&Matrix>,
) {
    let paint = plutovg_paint_create_linear_gradient(x1, y1, x2, y2, spread, stops, matrix);
    plutovg_canvas_set_paint(canvas, Some(paint));
}

/// Sets the current paint to a radial gradient.
pub fn plutovg_canvas_set_radial_gradient(
    canvas: &mut Canvas,
    cx: f32,
    cy: f32,
    cr: f32,
    fx: f32,
    fy: f32,
    fr: f32,
    spread: SpreadMethod,
    stops: &[GradientStop],
    matrix: Option<&Matrix>,
) {
    let paint = plutovg_paint_create_radial_gradient(cx, cy, cr, fx, fy, fr, spread, stops, matrix);
    plutovg_canvas_set_paint(canvas, Some(paint));
}

/// Sets the current paint to a texture.
pub fn plutovg_canvas_set_texture(
    canvas: &mut Canvas,
    surface: Option<Rc<Surface>>,
    type_: TextureType,
    opacity: f32,
    matrix: Option<&Matrix>,
) {
    let paint = plutovg_paint_create_texture(surface, type_, opacity, matrix);
    plutovg_canvas_set_paint(canvas, Some(paint));
}

/// Sets the current paint.
pub fn plutovg_canvas_set_paint(canvas: &mut Canvas, paint: Option<Rc<Paint>>) {
    canvas.state_mut().paint = paint;
}

/// Gets the current paint (and the current color).
pub fn plutovg_canvas_get_paint(canvas: &Canvas) -> (Option<Rc<Paint>>, Color) {
    (canvas.state().paint.clone(), canvas.state().color)
}

/// Sets the fill rule.
pub fn plutovg_canvas_set_fill_rule(canvas: &mut Canvas, winding: FillRule) {
    canvas.state_mut().winding = winding;
}

/// Gets the current fill rule.
pub fn plutovg_canvas_get_fill_rule(canvas: &Canvas) -> FillRule {
    canvas.state().winding
}

/// Sets the compositing operator.
pub fn plutovg_canvas_set_operator(canvas: &mut Canvas, op: Operator) {
    canvas.state_mut().op = op;
}

/// Gets the compositing operator.
pub fn plutovg_canvas_get_operator(canvas: &Canvas) -> Operator {
    canvas.state().op
}

/// Sets the global opacity.
pub fn plutovg_canvas_set_opacity(canvas: &mut Canvas, opacity: f32) {
    canvas.state_mut().opacity = plutovg_clamp(opacity, 0.0, 1.0);
}

/// Gets the current global opacity.
pub fn plutovg_canvas_get_opacity(canvas: &Canvas) -> f32 {
    canvas.state().opacity
}

/// Sets the line width.
pub fn plutovg_canvas_set_line_width(canvas: &mut Canvas, line_width: f32) {
    canvas.state_mut().stroke.style.width = line_width;
}

/// Gets the current line width.
pub fn plutovg_canvas_get_line_width(canvas: &Canvas) -> f32 {
    canvas.state().stroke.style.width
}

/// Sets the line cap style.
pub fn plutovg_canvas_set_line_cap(canvas: &mut Canvas, line_cap: LineCap) {
    canvas.state_mut().stroke.style.cap = line_cap;
}

/// Gets the current line cap style.
pub fn plutovg_canvas_get_line_cap(canvas: &Canvas) -> LineCap {
    canvas.state().stroke.style.cap
}

/// Sets the line join style.
pub fn plutovg_canvas_set_line_join(canvas: &mut Canvas, line_join: LineJoin) {
    canvas.state_mut().stroke.style.join = line_join;
}

/// Gets the current line join style.
pub fn plutovg_canvas_get_line_join(canvas: &Canvas) -> LineJoin {
    canvas.state().stroke.style.join
}

/// Sets the miter limit.
pub fn plutovg_canvas_set_miter_limit(canvas: &mut Canvas, miter_limit: f32) {
    canvas.state_mut().stroke.style.miter_limit = miter_limit;
}

/// Gets the current miter limit.
pub fn plutovg_canvas_get_miter_limit(canvas: &Canvas) -> f32 {
    canvas.state().stroke.style.miter_limit
}

/// Sets the dash pattern.
pub fn plutovg_canvas_set_dash(canvas: &mut Canvas, offset: f32, dashes: &[f32]) {
    plutovg_canvas_set_dash_offset(canvas, offset);
    plutovg_canvas_set_dash_array(canvas, dashes);
}

/// Sets the dash offset.
pub fn plutovg_canvas_set_dash_offset(canvas: &mut Canvas, offset: f32) {
    canvas.state_mut().stroke.dash.offset = offset;
}

/// Gets the current dash offset.
pub fn plutovg_canvas_get_dash_offset(canvas: &Canvas) -> f32 {
    canvas.state().stroke.dash.offset
}

/// Sets the dash array.
pub fn plutovg_canvas_set_dash_array(canvas: &mut Canvas, dashes: &[f32]) {
    let array = &mut canvas.state_mut().stroke.dash.array;
    array.clear();
    array.extend_from_slice(dashes);
}

/// Gets the current dash array.
pub fn plutovg_canvas_get_dash_array(canvas: &Canvas) -> &[f32] {
    &canvas.state().stroke.dash.array
}

/// Translates the current transformation matrix.
pub fn plutovg_canvas_translate(canvas: &mut Canvas, tx: f32, ty: f32) {
    plutovg_matrix_translate(&mut canvas.state_mut().matrix, tx, ty);
}

/// Scales the current transformation matrix.
pub fn plutovg_canvas_scale(canvas: &mut Canvas, sx: f32, sy: f32) {
    plutovg_matrix_scale(&mut canvas.state_mut().matrix, sx, sy);
}

/// Shears the current transformation matrix.
pub fn plutovg_canvas_shear(canvas: &mut Canvas, shx: f32, shy: f32) {
    plutovg_matrix_shear(&mut canvas.state_mut().matrix, shx, shy);
}

/// Rotates the current transformation matrix.
pub fn plutovg_canvas_rotate(canvas: &mut Canvas, angle: f32) {
    plutovg_matrix_rotate(&mut canvas.state_mut().matrix, angle);
}

/// Multiplies the current transformation matrix by `matrix`.
pub fn plutovg_canvas_transform(canvas: &mut Canvas, matrix: &Matrix) {
    let right = canvas.state().matrix;
    plutovg_matrix_multiply(&mut canvas.state_mut().matrix, matrix, &right);
}

/// Resets the current transformation matrix to the identity matrix.
pub fn plutovg_canvas_reset_matrix(canvas: &mut Canvas) {
    plutovg_matrix_init_identity(&mut canvas.state_mut().matrix);
}

/// Sets the current transformation matrix.
pub fn plutovg_canvas_set_matrix(canvas: &mut Canvas, matrix: Option<&Matrix>) {
    canvas.state_mut().matrix = matrix.copied().unwrap_or(PLUTOVG_IDENTITY_MATRIX);
}

/// Gets the current transformation matrix.
pub fn plutovg_canvas_get_matrix(canvas: &Canvas) -> Matrix {
    canvas.state().matrix
}

/// Transforms a point using the current transformation matrix.
pub fn plutovg_canvas_map(canvas: &Canvas, x: f32, y: f32) -> (f32, f32) {
    plutovg_matrix_map(&canvas.state().matrix, x, y)
}

/// Transforms a point using the current transformation matrix.
pub fn plutovg_canvas_map_point(canvas: &Canvas, src: &Point) -> Point {
    plutovg_matrix_map_point(&canvas.state().matrix, src)
}

/// Transforms a rectangle using the current transformation matrix.
pub fn plutovg_canvas_map_rect(canvas: &Canvas, src: &Rect) -> Rect {
    plutovg_matrix_map_rect(&canvas.state().matrix, src)
}

/// Moves the current point to a new position.
pub fn plutovg_canvas_move_to(canvas: &mut Canvas, x: f32, y: f32) {
    plutovg_path_move_to(&mut canvas.path, x, y);
}

/// Adds a straight line segment.
pub fn plutovg_canvas_line_to(canvas: &mut Canvas, x: f32, y: f32) {
    plutovg_path_line_to(&mut canvas.path, x, y);
}

/// Adds a quadratic Bézier curve.
pub fn plutovg_canvas_quad_to(canvas: &mut Canvas, x1: f32, y1: f32, x2: f32, y2: f32) {
    plutovg_path_quad_to(&mut canvas.path, x1, y1, x2, y2);
}

/// Adds a cubic Bézier curve.
pub fn plutovg_canvas_cubic_to(
    canvas: &mut Canvas,
    x1: f32,
    y1: f32,
    x2: f32,
    y2: f32,
    x3: f32,
    y3: f32,
) {
    plutovg_path_cubic_to(&mut canvas.path, x1, y1, x2, y2, x3, y3);
}

/// Adds an elliptical arc.
pub fn plutovg_canvas_arc_to(
    canvas: &mut Canvas,
    rx: f32,
    ry: f32,
    angle: f32,
    large_arc_flag: bool,
    sweep_flag: bool,
    x: f32,
    y: f32,
) {
    plutovg_path_arc_to(
        &mut canvas.path,
        rx,
        ry,
        angle,
        large_arc_flag,
        sweep_flag,
        x,
        y,
    );
}

/// Adds a rectangle.
pub fn plutovg_canvas_rect(canvas: &mut Canvas, x: f32, y: f32, w: f32, h: f32) {
    plutovg_path_add_rect(&mut canvas.path, x, y, w, h);
}

/// Adds a rounded rectangle.
pub fn plutovg_canvas_round_rect(
    canvas: &mut Canvas,
    x: f32,
    y: f32,
    w: f32,
    h: f32,
    rx: f32,
    ry: f32,
) {
    plutovg_path_add_round_rect(&mut canvas.path, x, y, w, h, rx, ry);
}

/// Adds an ellipse.
pub fn plutovg_canvas_ellipse(canvas: &mut Canvas, cx: f32, cy: f32, rx: f32, ry: f32) {
    plutovg_path_add_ellipse(&mut canvas.path, cx, cy, rx, ry);
}

/// Adds a circle.
pub fn plutovg_canvas_circle(canvas: &mut Canvas, cx: f32, cy: f32, r: f32) {
    plutovg_path_add_circle(&mut canvas.path, cx, cy, r);
}

/// Adds an arc.
pub fn plutovg_canvas_arc(
    canvas: &mut Canvas,
    cx: f32,
    cy: f32,
    r: f32,
    a0: f32,
    a1: f32,
    ccw: bool,
) {
    plutovg_path_add_arc(&mut canvas.path, cx, cy, r, a0, a1, ccw);
}

/// Adds a path to the current path.
pub fn plutovg_canvas_add_path(canvas: &mut Canvas, path: &Path) {
    plutovg_path_add_path(&mut canvas.path, path, None);
}

/// Starts a new path.
pub fn plutovg_canvas_new_path(canvas: &mut Canvas) {
    plutovg_path_reset(&mut canvas.path);
}

/// Closes the current path.
pub fn plutovg_canvas_close_path(canvas: &mut Canvas) {
    plutovg_path_close(&mut canvas.path);
}

/// Gets the current point.
pub fn plutovg_canvas_get_current_point(canvas: &Canvas) -> (f32, f32) {
    plutovg_path_get_current_point(&canvas.path)
}

/// Gets the current path.
pub fn plutovg_canvas_get_path(canvas: &Canvas) -> &Path {
    &canvas.path
}

/// Computes the bounding box of the area to be filled.
pub fn plutovg_canvas_fill_extents(canvas: &Canvas, extents: &mut Rect) {
    plutovg_path_extents(&canvas.path, Some(extents), true);
    *extents = plutovg_canvas_map_rect(canvas, extents);
}

/// Computes the bounding box of the area to be stroked.
pub fn plutovg_canvas_stroke_extents(canvas: &Canvas, extents: &mut Rect) {
    let stroke = &canvas.state().stroke;
    let mut cap_limit = stroke.style.width / 2.0;
    if stroke.style.cap == LineCap::Square {
        cap_limit *= PLUTOVG_SQRT2;
    }
    let mut join_limit = stroke.style.width / 2.0;
    if stroke.style.join == LineJoin::Miter {
        join_limit *= stroke.style.miter_limit;
    }

    let delta = plutovg_max(cap_limit, join_limit);
    plutovg_path_extents(&canvas.path, Some(extents), true);
    extents.x -= delta;
    extents.y -= delta;
    extents.w += delta * 2.0;
    extents.h += delta * 2.0;
    *extents = plutovg_canvas_map_rect(canvas, extents);
}

/// Gets the bounding box of the current clip region.
pub fn plutovg_canvas_clip_extents(canvas: &mut Canvas, extents: &mut Rect) {
    if canvas.state().clipping {
        plutovg_span_buffer_extents(&mut canvas.state_mut().clip_spans, extents);
    } else {
        extents.x = canvas.clip_rect.x;
        extents.y = canvas.clip_rect.y;
        extents.w = canvas.clip_rect.w;
        extents.h = canvas.clip_rect.h;
    }
}

/// Fills the current path and starts a new one.
pub fn plutovg_canvas_fill(canvas: &mut Canvas) {
    plutovg_canvas_fill_preserve(canvas);
    plutovg_canvas_new_path(canvas);
}

/// Strokes the current path and starts a new one.
pub fn plutovg_canvas_stroke(canvas: &mut Canvas) {
    plutovg_canvas_stroke_preserve(canvas);
    plutovg_canvas_new_path(canvas);
}

/// Intersects the clip region with the current path and starts a new one.
pub fn plutovg_canvas_clip(canvas: &mut Canvas) {
    plutovg_canvas_clip_preserve(canvas);
    plutovg_canvas_new_path(canvas);
}

/// Paints the clip region with the current paint.
pub fn plutovg_canvas_paint(canvas: &mut Canvas) {
    if canvas.state().clipping {
        let spans = canvas.state().clip_spans.clone();
        plutovg_blend(canvas, &spans);
    } else {
        let (w, h) = (canvas.surface.width, canvas.surface.height);
        plutovg_span_buffer_init_rect(&mut canvas.clip_spans, 0, 0, w, h);
        let spans = std::mem::take(&mut canvas.clip_spans);
        plutovg_blend(canvas, &spans);
        canvas.clip_spans = spans;
    }
}

/// Fills the current path (preserving it).
pub fn plutovg_canvas_fill_preserve(canvas: &mut Canvas) {
    let mut fill_spans = std::mem::take(&mut canvas.fill_spans);
    let clip_rect = canvas.clip_rect;
    let state = canvas.state();
    plutovg_rasterize(
        &mut fill_spans,
        &canvas.path,
        &state.matrix,
        Some(&clip_rect),
        None,
        state.winding,
    );
    if canvas.state().clipping {
        let mut clip_spans = std::mem::take(&mut canvas.clip_spans);
        plutovg_span_buffer_intersect(&mut clip_spans, &fill_spans, &canvas.state().clip_spans);
        plutovg_blend(canvas, &clip_spans);
        canvas.clip_spans = clip_spans;
    } else {
        plutovg_blend(canvas, &fill_spans);
    }
    canvas.fill_spans = fill_spans;
}

/// Strokes the current path (preserving it).
pub fn plutovg_canvas_stroke_preserve(canvas: &mut Canvas) {
    let mut fill_spans = std::mem::take(&mut canvas.fill_spans);
    let clip_rect = canvas.clip_rect;
    let state = canvas.state();
    plutovg_rasterize(
        &mut fill_spans,
        &canvas.path,
        &state.matrix,
        Some(&clip_rect),
        Some(&state.stroke),
        FillRule::NonZero,
    );
    if canvas.state().clipping {
        let mut clip_spans = std::mem::take(&mut canvas.clip_spans);
        plutovg_span_buffer_intersect(&mut clip_spans, &fill_spans, &canvas.state().clip_spans);
        plutovg_blend(canvas, &clip_spans);
        canvas.clip_spans = clip_spans;
    } else {
        plutovg_blend(canvas, &fill_spans);
    }
    canvas.fill_spans = fill_spans;
}

/// Intersects the clip region with the current path (preserving it).
pub fn plutovg_canvas_clip_preserve(canvas: &mut Canvas) {
    let clip_rect = canvas.clip_rect;
    if canvas.state().clipping {
        let mut fill_spans = std::mem::take(&mut canvas.fill_spans);
        let state = canvas.state();
        plutovg_rasterize(
            &mut fill_spans,
            &canvas.path,
            &state.matrix,
            Some(&clip_rect),
            None,
            state.winding,
        );
        let mut clip_spans = std::mem::take(&mut canvas.clip_spans);
        plutovg_span_buffer_intersect(&mut clip_spans, &fill_spans, &canvas.state().clip_spans);
        plutovg_span_buffer_copy(&mut canvas.state_mut().clip_spans, &clip_spans);
        canvas.clip_spans = clip_spans;
        canvas.fill_spans = fill_spans;
    } else {
        let mut state_clip = std::mem::take(&mut canvas.state_mut().clip_spans);
        let state = canvas.state();
        plutovg_rasterize(
            &mut state_clip,
            &canvas.path,
            &state.matrix,
            Some(&clip_rect),
            None,
            state.winding,
        );
        let state = canvas.state_mut();
        state.clip_spans = state_clip;
        state.clipping = true;
    }
}

/// Fills a rectangle.
pub fn plutovg_canvas_fill_rect(canvas: &mut Canvas, x: f32, y: f32, w: f32, h: f32) {
    plutovg_canvas_new_path(canvas);
    plutovg_canvas_rect(canvas, x, y, w, h);
    plutovg_canvas_fill(canvas);
}

/// Fills a path.
pub fn plutovg_canvas_fill_path(canvas: &mut Canvas, path: &Path) {
    plutovg_canvas_new_path(canvas);
    plutovg_canvas_add_path(canvas, path);
    plutovg_canvas_fill(canvas);
}

/// Strokes a rectangle.
pub fn plutovg_canvas_stroke_rect(canvas: &mut Canvas, x: f32, y: f32, w: f32, h: f32) {
    plutovg_canvas_new_path(canvas);
    plutovg_canvas_rect(canvas, x, y, w, h);
    plutovg_canvas_stroke(canvas);
}

/// Strokes a path.
pub fn plutovg_canvas_stroke_path(canvas: &mut Canvas, path: &Path) {
    plutovg_canvas_new_path(canvas);
    plutovg_canvas_add_path(canvas, path);
    plutovg_canvas_stroke(canvas);
}

/// Intersects the clip region with a rectangle.
pub fn plutovg_canvas_clip_rect(canvas: &mut Canvas, x: f32, y: f32, w: f32, h: f32) {
    plutovg_canvas_new_path(canvas);
    plutovg_canvas_rect(canvas, x, y, w, h);
    plutovg_canvas_clip(canvas);
}

/// Intersects the clip region with a path.
pub fn plutovg_canvas_clip_path(canvas: &mut Canvas, path: &Path) {
    plutovg_canvas_new_path(canvas);
    plutovg_canvas_add_path(canvas, path);
    plutovg_canvas_clip(canvas);
}
