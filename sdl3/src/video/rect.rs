// Rust translation of src/video/SDL_rect.c, src/video/SDL_rect_impl.h and the
// inline helpers of include/SDL3/SDL_rect.h from Simple DirectMedia Layer.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! Rectangles and 2D points, in integer ([`Rect`], [`Point`]) and floating
//! point ([`FRect`], [`FPoint`]) versions.
//!
//! Upstream implements the two versions by `#include`-ing `SDL_rect_impl.h`
//! twice with different macros; here a `macro_rules!` plays the same role.

use crate::err;
use crate::error::Result;

/// A point, using integers. Translation of `SDL_Point`.
#[repr(C)]
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Default)]
pub struct Point {
    pub x: i32,
    pub y: i32,
}

/// A point, using floating point values. Translation of `SDL_FPoint`.
#[repr(C)]
#[derive(Clone, Copy, PartialEq, Debug, Default)]
pub struct FPoint {
    pub x: f32,
    pub y: f32,
}

/// A rectangle with the origin at the upper left, using integers. Translation of `SDL_Rect`.
#[repr(C)]
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Default)]
pub struct Rect {
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
}

/// A rectangle with the origin at the upper left, using floating point values. Translation of `SDL_FRect`.
#[repr(C)]
#[derive(Clone, Copy, PartialEq, Debug, Default)]
pub struct FRect {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

impl Point {
    pub const fn new(x: i32, y: i32) -> Self {
        Point { x, y }
    }
}

impl FPoint {
    pub const fn new(x: f32, y: f32) -> Self {
        FPoint { x, y }
    }
}

impl From<Point> for FPoint {
    fn from(p: Point) -> FPoint {
        FPoint {
            x: p.x as f32,
            y: p.y as f32,
        }
    }
}

impl From<(i32, i32)> for Point {
    fn from((x, y): (i32, i32)) -> Point {
        Point { x, y }
    }
}

impl From<(f32, f32)> for FPoint {
    fn from((x, y): (f32, f32)) -> FPoint {
        FPoint { x, y }
    }
}

impl From<Rect> for FRect {
    /// Translation of `SDL_RectToFRect()`.
    fn from(r: Rect) -> FRect {
        FRect {
            x: r.x as f32,
            y: r.y as f32,
            w: r.w as f32,
            h: r.h as f32,
        }
    }
}

impl Rect {
    pub const fn new(x: i32, y: i32, w: i32, h: i32) -> Self {
        Rect { x, y, w, h }
    }

    /// Whether the rectangle has no area (width and/or height ≤ 0).
    /// Translation of `SDL_RectEmpty()`.
    #[inline]
    pub const fn is_empty(&self) -> bool {
        self.w <= 0 || self.h <= 0
    }

    /// Whether `p` is inside: x and y ≥ the top left corner and < `x+w`/`y+h`,
    /// so a 1x1 rectangle contains (0,0) but not (0,1).
    /// Translation of `SDL_PointInRect()`.
    #[inline]
    pub const fn contains(&self, p: Point) -> bool {
        p.x >= self.x && p.x < self.x + self.w && p.y >= self.y && p.y < self.y + self.h
    }

    /// Translation of `SDL_RectToFRect()`.
    #[inline]
    pub const fn to_frect(self) -> FRect {
        FRect {
            x: self.x as f32,
            y: self.y as f32,
            w: self.w as f32,
            h: self.h as f32,
        }
    }
}

impl FRect {
    pub const fn new(x: f32, y: f32, w: f32, h: f32) -> Self {
        FRect { x, y, w, h }
    }

    /// Whether the rectangle can contain no point (width and/or height < 0).
    /// Translation of `SDL_RectEmptyFloat()`.
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.w < 0.0 || self.h < 0.0
    }

    /// Whether `p` is inside: x and y ≥ the top left corner and ≤ `x+w`/`y+h`,
    /// so a 1x1 rectangle contains (0,0) and (0,1) but not (0,2).
    /// Translation of `SDL_PointInRectFloat()`.
    #[inline]
    pub fn contains(&self, p: FPoint) -> bool {
        p.x >= self.x && p.x <= self.x + self.w && p.y >= self.y && p.y <= self.y + self.h
    }

    /// Equality within `epsilon` per component. Translation of `SDL_RectsEqualEpsilon()`.
    #[inline]
    pub fn approx_eq(&self, other: &FRect, epsilon: f32) -> bool {
        std::ptr::eq(self, other)
            || ((self.x - other.x).abs() <= epsilon
                && (self.y - other.y).abs() <= epsilon
                && (self.w - other.w).abs() <= epsilon
                && (self.h - other.h).abs() <= epsilon)
    }

    /// Equality within `f32::EPSILON`. Translation of `SDL_RectsEqualFloat()`.
    #[inline]
    pub fn approx_eq_default(&self, other: &FRect) -> bool {
        self.approx_eq(other, f32::EPSILON)
    }
}

/// Minimal rectangle covering the vertical spans of `rects`, clipped to
/// `width`×`height`. There's no float version of this at the moment, because
/// it's not a public API and internally we only need the int version.
/// Translation of the internal `SDL_GetSpanEnclosingRect()`.
#[allow(dead_code)] // used by the surface/blit code in the next phase
pub(crate) fn span_enclosing_rect(width: i32, height: i32, rects: &[Rect]) -> Option<Rect> {
    if width < 1 || height < 1 || rects.is_empty() {
        return None;
    }

    // Initialize to empty rect
    let mut span_y1 = height;
    let mut span_y2 = 0;

    for rect in rects {
        let rect_y1 = rect.y;
        let rect_y2 = rect_y1 + rect.h;

        // Clip out of bounds rectangles, and expand span rect
        if rect_y1 < 0 {
            span_y1 = 0;
        } else if rect_y1 < span_y1 {
            span_y1 = rect_y1;
        }
        if rect_y2 > height {
            span_y2 = height;
        } else if rect_y2 > span_y2 {
            span_y2 = rect_y2;
        }
    }
    (span_y2 > span_y1).then(|| Rect::new(0, span_y1, width, span_y2 - span_y1))
}

// For use with the Cohen-Sutherland algorithm for line clipping
const CODE_BOTTOM: i32 = 1;
const CODE_TOP: i32 = 2;
const CODE_LEFT: i32 = 4;
const CODE_RIGHT: i32 = 8;

/// Same code twice, for float and int versions (translation of `SDL_rect_impl.h`).
macro_rules! rect_impl {
    (
        $RECTTYPE:ident, $POINTTYPE:ident, $SCALARTYPE:ty, $BIGSCALARTYPE:ty, $EPSILON:expr,
        $c_has:literal, $c_intersect:literal, $c_union:literal, $c_enclose:literal, $c_line:literal
    ) => {
        impl $RECTTYPE {
            /// Translation of `SDL_RectCanOverflow()`: coordinates near the
            /// `i32` limits make the `+ w`/`+ h` arithmetic below unsafe.
            pub(crate) fn can_overflow(&self) -> bool {
                self.x <= (i32::MIN / 2) as $SCALARTYPE
                    || self.x >= (i32::MAX / 2) as $SCALARTYPE
                    || self.y <= (i32::MIN / 2) as $SCALARTYPE
                    || self.y >= (i32::MAX / 2) as $SCALARTYPE
                    || self.w >= (i32::MAX / 2) as $SCALARTYPE
                    || self.h >= (i32::MAX / 2) as $SCALARTYPE
            }

            #[doc = concat!("Whether two rectangles intersect. Rectangles whose coordinates could overflow the arithmetic (see `SDL_RectCanOverflow`) never intersect.\n\nTranslation of `", $c_has, "()`.")]
            pub fn intersects(&self, other: &$RECTTYPE) -> bool {
                if self.can_overflow() || other.can_overflow() {
                    return false; // "Potential rect math overflow"
                }
                let (a, b) = (self, other);

                // Horizontal intersection
                let mut amin = a.x;
                let mut amax = amin + a.w;
                let mut bmin = b.x;
                let mut bmax = bmin + b.w;
                if bmin > amin {
                    amin = bmin;
                }
                if bmax < amax {
                    amax = bmax;
                }
                if (amax - $EPSILON) < amin {
                    return false;
                }

                // Vertical intersection
                amin = a.y;
                amax = amin + a.h;
                bmin = b.y;
                bmax = bmin + b.h;
                if bmin > amin {
                    amin = bmin;
                }
                if bmax < amax {
                    amax = bmax;
                }
                if (amax - $EPSILON) < amin {
                    return false;
                }
                true
            }

            #[doc = concat!("The intersection of two rectangles, or `None` if they don't intersect (or could overflow, see [`intersects`](Self::intersects)).\n\nTranslation of `", $c_intersect, "()`.")]
            pub fn intersection(&self, other: &$RECTTYPE) -> Option<$RECTTYPE> {
                if self.can_overflow() || other.can_overflow() {
                    return None; // "Potential rect math overflow"
                }
                let (a, b) = (self, other);
                let mut result = $RECTTYPE::default();

                // Horizontal intersection
                let mut amin = a.x;
                let mut amax = amin + a.w;
                let mut bmin = b.x;
                let mut bmax = bmin + b.w;
                if bmin > amin {
                    amin = bmin;
                }
                result.x = amin;
                if bmax < amax {
                    amax = bmax;
                }
                result.w = amax - amin;

                // Vertical intersection
                amin = a.y;
                amax = amin + a.h;
                bmin = b.y;
                bmax = bmin + b.h;
                if bmin > amin {
                    amin = bmin;
                }
                result.y = amin;
                if bmax < amax {
                    amax = bmax;
                }
                result.h = amax - amin;

                (!result.is_empty()).then_some(result)
            }

            /// `SDL_GetRectIntersection()` with its output-parameter
            /// behaviour: `result` is written even when the rectangles don't
            /// intersect (with an empty rectangle), and left alone only when
            /// the arithmetic could overflow.
            #[allow(dead_code)] // only the integer variant is used so far
            pub(crate) fn intersect_into(&self, other: &$RECTTYPE, result: &mut $RECTTYPE) -> bool {
                if self.can_overflow() || other.can_overflow() {
                    return false; // "Potential rect math overflow"
                }
                let (a, b) = (self, other);

                // Horizontal intersection
                let mut amin = a.x;
                let mut amax = amin + a.w;
                let bmin = b.x;
                let bmax = bmin + b.w;
                if bmin > amin {
                    amin = bmin;
                }
                result.x = amin;
                if bmax < amax {
                    amax = bmax;
                }
                result.w = amax - amin;

                // Vertical intersection
                let mut amin = a.y;
                let mut amax = amin + a.h;
                let bmin = b.y;
                let bmax = bmin + b.h;
                if bmin > amin {
                    amin = bmin;
                }
                result.y = amin;
                if bmax < amax {
                    amax = bmax;
                }
                result.h = amax - amin;

                !result.is_empty()
            }

            #[doc = concat!("The smallest rectangle containing both. Empty rectangles are ignored. Fails only when the arithmetic could overflow.\n\nTranslation of `", $c_union, "()`.")]
            pub fn union(&self, other: &$RECTTYPE) -> Result<$RECTTYPE> {
                if self.can_overflow() || other.can_overflow() {
                    return Err(err!("Potential rect math overflow"));
                }
                let (a, b) = (self, other);

                if a.is_empty() {
                    // Special cases for empty Rects
                    return Ok(if b.is_empty() { $RECTTYPE::default() } else { *b });
                } else if b.is_empty() {
                    // A not empty, B empty
                    return Ok(*a);
                }
                let mut result = $RECTTYPE::default();

                // Horizontal union
                let mut amin = a.x;
                let mut amax = amin + a.w;
                let mut bmin = b.x;
                let mut bmax = bmin + b.w;
                if bmin < amin {
                    amin = bmin;
                }
                result.x = amin;
                if bmax > amax {
                    amax = bmax;
                }
                result.w = amax - amin;

                // Vertical union
                amin = a.y;
                amax = amin + a.h;
                bmin = b.y;
                bmax = bmin + b.h;
                if bmin < amin {
                    amin = bmin;
                }
                result.y = amin;
                if bmax > amax {
                    amax = bmax;
                }
                result.h = amax - amin;
                Ok(result)
            }

            #[doc = concat!("The minimal rectangle enclosing `points`, considering only points inside `clip` when given. `None` if there are no points, or none inside `clip`.\n\nTranslation of `", $c_enclose, "()`.")]
            pub fn enclosing_points(points: &[$POINTTYPE], clip: Option<&$RECTTYPE>) -> Option<$RECTTYPE> {
                if points.is_empty() {
                    return None; // invalid parameter "count"
                }
                let (mut minx, mut miny, mut maxx, mut maxy);

                if let Some(clip) = clip {
                    let clip_minx = clip.x;
                    let clip_miny = clip.y;
                    let clip_maxx = clip.x + clip.w - $EPSILON;
                    let clip_maxy = clip.y + clip.h - $EPSILON;

                    // Special case for empty rectangle
                    if clip.is_empty() {
                        return None;
                    }

                    let mut inside = points
                        .iter()
                        .filter(|p| !(p.x < clip_minx || p.x > clip_maxx || p.y < clip_miny || p.y > clip_maxy));
                    // First point added
                    let first = inside.next()?;
                    minx = first.x;
                    maxx = first.x;
                    miny = first.y;
                    maxy = first.y;
                    for p in inside {
                        if p.x < minx {
                            minx = p.x;
                        } else if p.x > maxx {
                            maxx = p.x;
                        }
                        if p.y < miny {
                            miny = p.y;
                        } else if p.y > maxy {
                            maxy = p.y;
                        }
                    }
                } else {
                    // No clipping, always add the first point
                    minx = points[0].x;
                    maxx = points[0].x;
                    miny = points[0].y;
                    maxy = points[0].y;
                    for p in &points[1..] {
                        if p.x < minx {
                            minx = p.x;
                        } else if p.x > maxx {
                            maxx = p.x;
                        }
                        if p.y < miny {
                            miny = p.y;
                        } else if p.y > maxy {
                            maxy = p.y;
                        }
                    }
                }

                Some($RECTTYPE { x: minx, y: miny, w: (maxx - minx) + $EPSILON, h: (maxy - miny) + $EPSILON })
            }

            // Use the Cohen-Sutherland algorithm for line clipping
            fn compute_outcode(&self, x: $SCALARTYPE, y: $SCALARTYPE) -> i32 {
                let mut code = 0;
                if y < self.y {
                    code |= CODE_TOP;
                } else if y > (self.y + self.h - $EPSILON) {
                    code |= CODE_BOTTOM;
                }
                if x < self.x {
                    code |= CODE_LEFT;
                } else if x > (self.x + self.w - $EPSILON) {
                    code |= CODE_RIGHT;
                }
                code
            }

            #[doc = concat!("Clip the line segment `a`–`b` to the rectangle. Returns the (possibly shortened) segment, or `None` if it lies entirely outside (or the rectangle is empty / could overflow).\n\nTranslation of `", $c_line, "()`.")]
            pub fn clip_line(&self, a: $POINTTYPE, b: $POINTTYPE) -> Option<($POINTTYPE, $POINTTYPE)> {
                let rect = self;
                let mut x: $SCALARTYPE = Default::default();
                let mut y: $SCALARTYPE = Default::default();

                if rect.can_overflow() {
                    return None; // "Potential rect math overflow"
                }
                if rect.is_empty() {
                    return None; // Special case for empty rect
                }

                let (mut x1, mut y1, mut x2, mut y2) = (a.x, a.y, b.x, b.y);
                let rectx1 = rect.x;
                let recty1 = rect.y;
                let rectx2 = rect.x + rect.w - $EPSILON;
                let recty2 = rect.y + rect.h - $EPSILON;

                // Check to see if entire line is inside rect
                if x1 >= rectx1
                    && x1 <= rectx2
                    && x2 >= rectx1
                    && x2 <= rectx2
                    && y1 >= recty1
                    && y1 <= recty2
                    && y2 >= recty1
                    && y2 <= recty2
                {
                    return Some((a, b));
                }

                // Check to see if entire line is to one side of rect
                if (x1 < rectx1 && x2 < rectx1)
                    || (x1 > rectx2 && x2 > rectx2)
                    || (y1 < recty1 && y2 < recty1)
                    || (y1 > recty2 && y2 > recty2)
                {
                    return None;
                }

                if y1 == y2 {
                    // Horizontal line, easy to clip
                    let cx = |v: $SCALARTYPE| if v < rectx1 { rectx1 } else if v > rectx2 { rectx2 } else { v };
                    return Some(($POINTTYPE { x: cx(x1), y: y1 }, $POINTTYPE { x: cx(x2), y: y2 }));
                }

                if x1 == x2 {
                    // Vertical line, easy to clip
                    let cy = |v: $SCALARTYPE| if v < recty1 { recty1 } else if v > recty2 { recty2 } else { v };
                    return Some(($POINTTYPE { x: x1, y: cy(y1) }, $POINTTYPE { x: x2, y: cy(y2) }));
                }

                // More complicated Cohen-Sutherland algorithm
                let mut outcode1 = rect.compute_outcode(x1, y1);
                let mut outcode2 = rect.compute_outcode(x2, y2);
                while outcode1 != 0 || outcode2 != 0 {
                    if (outcode1 & outcode2) != 0 {
                        return None;
                    }

                    if outcode1 != 0 {
                        if (outcode1 & CODE_TOP) != 0 {
                            y = recty1;
                            x = (x1 as $BIGSCALARTYPE
                                + ((x2 - x1) as $BIGSCALARTYPE * (y - y1) as $BIGSCALARTYPE)
                                    / (y2 - y1) as $BIGSCALARTYPE) as $SCALARTYPE;
                        } else if (outcode1 & CODE_BOTTOM) != 0 {
                            y = recty2;
                            x = (x1 as $BIGSCALARTYPE
                                + ((x2 - x1) as $BIGSCALARTYPE * (y - y1) as $BIGSCALARTYPE)
                                    / (y2 - y1) as $BIGSCALARTYPE) as $SCALARTYPE;
                        } else if (outcode1 & CODE_LEFT) != 0 {
                            x = rectx1;
                            y = (y1 as $BIGSCALARTYPE
                                + ((y2 - y1) as $BIGSCALARTYPE * (x - x1) as $BIGSCALARTYPE)
                                    / (x2 - x1) as $BIGSCALARTYPE) as $SCALARTYPE;
                        } else if (outcode1 & CODE_RIGHT) != 0 {
                            x = rectx2;
                            y = (y1 as $BIGSCALARTYPE
                                + ((y2 - y1) as $BIGSCALARTYPE * (x - x1) as $BIGSCALARTYPE)
                                    / (x2 - x1) as $BIGSCALARTYPE) as $SCALARTYPE;
                        }
                        x1 = x;
                        y1 = y;
                        outcode1 = rect.compute_outcode(x, y);
                    } else {
                        if (outcode2 & CODE_TOP) != 0 {
                            debug_assert!(y2 != y1); // if equal: division by zero.
                            y = recty1;
                            x = (x1 as $BIGSCALARTYPE
                                + ((x2 - x1) as $BIGSCALARTYPE * (y - y1) as $BIGSCALARTYPE)
                                    / (y2 - y1) as $BIGSCALARTYPE) as $SCALARTYPE;
                        } else if (outcode2 & CODE_BOTTOM) != 0 {
                            debug_assert!(y2 != y1); // if equal: division by zero.
                            y = recty2;
                            x = (x1 as $BIGSCALARTYPE
                                + ((x2 - x1) as $BIGSCALARTYPE * (y - y1) as $BIGSCALARTYPE)
                                    / (y2 - y1) as $BIGSCALARTYPE) as $SCALARTYPE;
                        } else if (outcode2 & CODE_LEFT) != 0 {
                            debug_assert!(x2 != x1); // if equal: division by zero.
                            x = rectx1;
                            y = (y1 as $BIGSCALARTYPE
                                + ((y2 - y1) as $BIGSCALARTYPE * (x - x1) as $BIGSCALARTYPE)
                                    / (x2 - x1) as $BIGSCALARTYPE) as $SCALARTYPE;
                        } else if (outcode2 & CODE_RIGHT) != 0 {
                            debug_assert!(x2 != x1); // if equal: division by zero.
                            x = rectx2;
                            y = (y1 as $BIGSCALARTYPE
                                + ((y2 - y1) as $BIGSCALARTYPE * (x - x1) as $BIGSCALARTYPE)
                                    / (x2 - x1) as $BIGSCALARTYPE) as $SCALARTYPE;
                        }
                        x2 = x;
                        y2 = y;
                        outcode2 = rect.compute_outcode(x, y);
                    }
                }
                Some(($POINTTYPE { x: x1, y: y1 }, $POINTTYPE { x: x2, y: y2 }))
            }
        }
    };
}

rect_impl!(
    Rect,
    Point,
    i32,
    i64,
    1,
    "SDL_HasRectIntersection",
    "SDL_GetRectIntersection",
    "SDL_GetRectUnion",
    "SDL_GetRectEnclosingPoints",
    "SDL_GetRectAndLineIntersection"
);

rect_impl!(
    FRect,
    FPoint,
    f32,
    f64,
    0.0f32,
    "SDL_HasRectIntersectionFloat",
    "SDL_GetRectIntersectionFloat",
    "SDL_GetRectUnionFloat",
    "SDL_GetRectEnclosingPointsFloat",
    "SDL_GetRectAndLineIntersectionFloat"
);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn inline_helpers() {
        let r = Rect::new(0, 0, 1, 1);
        assert!(r.contains(Point::new(0, 0)));
        assert!(!r.contains(Point::new(0, 1)));
        assert!(Rect::new(0, 0, 0, 5).is_empty());
        assert!(!r.is_empty());
        let fr = FRect::new(0.0, 0.0, 1.0, 1.0);
        assert!(fr.contains(FPoint::new(0.0, 1.0)));
        assert!(!fr.contains(FPoint::new(0.0, 2.0)));
        assert!(!FRect::new(0.0, 0.0, 0.0, 0.0).is_empty());
        assert!(FRect::new(0.0, 0.0, -1.0, 0.0).is_empty());
        assert!(fr.approx_eq_default(&FRect::from(r)));
        assert_eq!(r.to_frect(), fr);
        assert!(fr.approx_eq(&FRect::new(0.05, 0.0, 1.0, 1.0), 0.1));
        assert_eq!(FPoint::from(Point::new(1, 2)), FPoint::new(1.0, 2.0));
        assert_eq!(Point::from((3, 4)), Point::new(3, 4));
    }

    #[test]
    fn intersection_and_union() {
        let a = Rect::new(0, 0, 10, 10);
        let b = Rect::new(5, 5, 10, 10);
        let c = Rect::new(20, 20, 1, 1);
        assert!(a.intersects(&b));
        assert!(!a.intersects(&c));
        assert_eq!(a.intersection(&b), Some(Rect::new(5, 5, 5, 5)));
        assert_eq!(a.intersection(&c), None);
        assert_eq!(a.union(&b), Ok(Rect::new(0, 0, 15, 15)));
        assert_eq!(Rect::default().union(&b), Ok(b));
        assert_eq!(Rect::default().union(&Rect::default()), Ok(Rect::default()));
        // Adjacent (touching) rects don't intersect in the int version...
        assert!(!a.intersects(&Rect::new(10, 0, 5, 5)));
        // ...but do in the float version (epsilon is 0).
        assert!(FRect::from(a).intersects(&FRect::new(10.0, 0.0, 5.0, 5.0)));
        // Overflow guard
        let huge = Rect::new(i32::MAX / 2, 0, 1, 1);
        assert!(!huge.intersects(&a));
        assert_eq!(huge.intersection(&a), None);
        assert!(huge.union(&a).unwrap_err().message().contains("overflow"));
        assert_eq!(
            FRect::new(0.0, 0.0, 2.5, 2.5).intersection(&FRect::new(1.0, 1.0, 5.0, 5.0)),
            Some(FRect::new(1.0, 1.0, 1.5, 1.5))
        );
    }

    #[test]
    fn enclosing_points() {
        let pts = [Point::new(1, 2), Point::new(5, 7), Point::new(-3, 4)];
        assert_eq!(
            Rect::enclosing_points(&pts, None),
            Some(Rect::new(-3, 2, 9, 6))
        );
        let clip = Rect::new(0, 0, 10, 10);
        assert_eq!(
            Rect::enclosing_points(&pts, Some(&clip)),
            Some(Rect::new(1, 2, 5, 6))
        );
        assert_eq!(
            Rect::enclosing_points(&pts, Some(&Rect::new(100, 100, 5, 5))),
            None
        );
        assert_eq!(
            Rect::enclosing_points(&pts, Some(&Rect::new(0, 0, 0, 0))),
            None
        );
        assert_eq!(Rect::enclosing_points(&[], None), None);
        let fpts = [FPoint::new(1.0, 2.0), FPoint::new(5.0, 7.0)];
        assert_eq!(
            FRect::enclosing_points(&fpts, None),
            Some(FRect::new(1.0, 2.0, 4.0, 5.0))
        );
    }

    #[test]
    fn line_clipping() {
        let r = Rect::new(0, 0, 10, 10);
        let p = Point::new;
        // fully inside
        assert_eq!(r.clip_line(p(1, 1), p(5, 5)), Some((p(1, 1), p(5, 5))));
        // fully outside
        assert_eq!(r.clip_line(p(20, 20), p(30, 30)), None);
        // horizontal crossing
        assert_eq!(r.clip_line(p(-5, 5), p(50, 5)), Some((p(0, 5), p(9, 5))));
        // vertical crossing
        assert_eq!(r.clip_line(p(3, -5), p(3, 50)), Some((p(3, 0), p(3, 9))));
        // diagonal crossing the whole rect
        assert_eq!(
            r.clip_line(p(-10, -10), p(20, 20)),
            Some((p(0, 0), p(9, 9)))
        );
        // diagonal that misses the corner
        assert_eq!(r.clip_line(p(-10, 5), p(5, 20)), None);
        // empty rect
        assert_eq!(Rect::new(0, 0, 0, 0).clip_line(p(1, 1), p(2, 2)), None);
        // float version
        let fr = FRect::new(0.0, 0.0, 10.0, 10.0);
        assert_eq!(
            fr.clip_line(FPoint::new(-10.0, -10.0), FPoint::new(20.0, 20.0)),
            Some((FPoint::new(0.0, 0.0), FPoint::new(10.0, 10.0)))
        );
    }

    #[test]
    fn span_enclosing() {
        let rects = [Rect::new(0, -5, 1, 10), Rect::new(0, 50, 1, 100)];
        assert_eq!(
            span_enclosing_rect(64, 64, &rects),
            Some(Rect::new(0, 0, 64, 64))
        );
        assert_eq!(
            span_enclosing_rect(64, 64, &[Rect::new(3, 10, 1, 5)]),
            Some(Rect::new(0, 10, 64, 5))
        );
        assert_eq!(span_enclosing_rect(64, 64, &[]), None);
        assert_eq!(span_enclosing_rect(0, 64, &rects), None);
        assert_eq!(
            span_enclosing_rect(64, 64, &[Rect::new(0, 100, 1, 5)]),
            None
        );
    }
}
