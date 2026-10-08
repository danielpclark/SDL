// Rust translation of src/smooth/ftgrays.c (and ftgrays.h) from FreeType
// (2.13.2, as SDL_ttf's external/freetype pins it).
// Copyright (C) 2000-2023 by David Turner, Robert Wilhelm, and Werner Lemberg.
// This is an altered (translated) version of the original software; it is
// used under the FreeType License (see FTL.TXT and LICENSE.txt).

//! A new `perfect' anti-aliasing renderer (body).
//!
//! This is a new anti-aliasing scan-converter for FreeType 2.  The
//! algorithm used here is _very_ different from the one in the standard
//! `ftraster' module.  Actually, `ftgrays' computes the _exact_ coverage
//! of the outline on each pixel cell by straight segments.
//!
//! It is based on ideas that I initially found in Raph Levien's excellent
//! LibArt graphics library (see <https://www.levien.com/libart> for more
//! information, though the web pages do not tell anything about the
//! renderer; you'll have to dive into the source code to understand how
//! it works).
//!
//! Note, however, that this is a _very_ different implementation compared
//! to Raph's.  Coverage information is stored in a very different way, and
//! I don't use sorted vector paths.  Also, it doesn't use floating point
//! values.
//!
//! Bézier segments are flattened by splitting them until their deviation
//! from straight line becomes much smaller than a pixel.  Therefore, the
//! pixel coverage by a Bézier curve is calculated approximately.  To
//! estimate the deviation, we use the distance from the control point to
//! the conic chord centre or the cubic chord trisection.  These distances
//! vanish fast after each split.  In the conic case, they vanish
//! predictably and the number of necessary splits can be calculated.
//!
//! This renderer has the following advantages:
//!
//! - It doesn't need an intermediate bitmap.  Instead, one can supply a
//!   callback function that will be called by the renderer to draw gray
//!   spans on any target surface.  You can thus do direct composition on
//!   any kind of bitmap, provided that you give the renderer the right
//!   callback.
//!
//! - A perfect anti-aliaser, i.e., it computes the _exact_ coverage on
//!   each pixel cell by straight segments.
//!
//! - It performs a single pass on the outline (the `standard' FT2
//!   renderer makes two passes).
//!
//! - It can easily be modified to render to _any_ number of gray levels
//!   cheaply.
//!
//! - For small (< 80) pixel sizes, it is faster than the standard
//!   renderer.
//!
//! Translation notes: the build is the 64-bit one SDL_ttf makes on x86-64
//! (`FT_INT64`, `BEZIER_USE_DDA` with `FT_SSE2`), so the line renderer is
//! the division-free `FT_INT64` variant and conic arcs are flattened with
//! the DDA (whose SSE2 and scalar versions compute the same values; the
//! shifts follow the SSE2 instructions, which give 0 for shift counts out
//! of range). The cell pool keeps C's layout on a 64-bit target: 16384
//! bytes of 24-byte cells, with the per-band list heads (8-byte pointers)
//! at its start. The `longjmp` out of a full pool becomes a flag that
//! stops the outline decomposition.

use super::super::base::ftoutln::{ft_outline_decompose, FtOutlineFuncs};
use super::super::ftimage::{FtRasterFuncs, FtRasterParams, FtRasterSource};
use super::super::fttypes::*;

/* must be at least 6 bits! */
const PIXEL_BITS: i32 = 8;

const ONE_PIXEL: i32 = 1 << PIXEL_BITS;

/// `TRUNC`
#[inline]
fn trunc(x: TPos) -> TCoord {
    (x >> PIXEL_BITS) as TCoord
}

/// `FRACT`
#[inline]
fn fract(x: TPos) -> TCoord {
    (x & (ONE_PIXEL as TPos - 1)) as TCoord
}

/// `UPSCALE` (PIXEL_BITS >= 6)
#[inline]
fn upscale(x: TPos) -> TPos {
    x.wrapping_mul((ONE_PIXEL >> 6) as TPos)
}

/// `FT_UDIVPREP`: Calculating coverages for a slanted line requires a
/// division each time the line crosses from cell to cell.  These macros
/// speed up the repetitive divisions by replacing them with
/// multiplications and right shifts so that at most two divisions are
/// performed for each slanted line.  Nevertheless, these divisions are
/// noticeable in the overall performance because flattened curves produce
/// a very large number of slanted lines.
///
/// The division results here are always within ONE_PIXEL.  Therefore the
/// shift magnitude should be at least PIXEL_BITS wider than the divisors
/// to provide sufficient accuracy of the multiply-shift.  It should not
/// exceed (64 - PIXEL_BITS) to prevent overflowing and leave enough room
/// for 64-bit unsigned multiplication however.
#[inline]
fn ft_udivprep(c: bool, b: TPos) -> i64 {
    if c {
        0xFFFFFFFFi64 / b
    } else {
        0
    }
}

/// `FT_UDIV` (with the prepared reciprocal `b_r`)
#[inline]
fn ft_udiv(a: i64, b_r: i64) -> TCoord {
    ((a as u64).wrapping_mul(b_r as u64) >> 32) as TCoord
}

/// `FT_FILL_RULE`: Scale area and apply fill rule to calculate the
/// coverage byte.  The top fill bit is used for the non-zero rule. The
/// eighth fill bit is used for the even-odd rule.  The higher coverage
/// bytes are either clamped for the non-zero-rule or discarded later for
/// the even-odd rule.
#[inline]
fn ft_fill_rule(area: TArea, fill: i32) -> i32 {
    let mut coverage = area >> (PIXEL_BITS * 2 + 1 - 8);
    if coverage & fill != 0 {
        coverage = !coverage;
    }
    if coverage > 255 && fill & i32::MIN != 0 {
        coverage = 255;
    }
    coverage
}

/*************************************************************************/
/*                                                                       */
/*   TYPE DEFINITIONS                                                    */
/*                                                                       */
/*************************************************************************/

/* don't change the following types to FT_Int or FT_Pos, since we might */
/* need to define them to "float" or "double" when experimenting with   */
/* new algorithms                                                       */

type TPos = i64; /* subpixel coordinate               */
type TCoord = i32; /* integer scanline/pixel coordinate */
type TArea = i32; /* cells areas, coordinate products  */

/// `TCell` (`next` is an index into the pool)
#[derive(Debug, Clone, Copy, Default)]
struct TCell {
    x: TCoord,     /* same with gray_TWorker.ex    */
    cover: TCoord, /* same with gray_TWorker.cover */
    area: TArea,
    next: usize,
}

/// `TPixmap`
#[derive(Debug, Clone, Copy, Default)]
struct TPixmap {
    origin: isize, /* pixmap origin at the bottom-left */
    pitch: isize,  /* pitch to go down one row */
}

/// `FT_RENDER_POOL_SIZE` (ftoption.h)
const FT_RENDER_POOL_SIZE: usize = 16384;

/// `sizeof ( TCell )` and `sizeof ( PCell )` on a 64-bit target
const SIZEOF_TCELL: usize = 24;
const SIZEOF_PCELL: usize = 8;

/* maximum number of gray cells in the buffer */
const FT_MAX_GRAY_POOL: usize = FT_RENDER_POOL_SIZE / SIZEOF_TCELL;

/* FT_Span buffer size for direct rendering only */
const FT_MAX_GRAY_SPANS: usize = 16;

/// `gray_TWorker`
struct GrayTWorker<'a, 'b> {
    /// set where C does `ft_longjmp( ras.jump_buffer, 1 )`
    jumped: bool,

    min_ex: TCoord,
    max_ex: TCoord, /* min and max integer pixel coordinates */
    min_ey: TCoord,
    max_ey: TCoord,
    count_ey: TCoord, /* same as (max_ey - min_ey) */

    cell: usize,      /* current cell                             */
    cell_free: usize, /* call allocation next free slot           */
    cell_null: usize, /* last cell, used as dumpster and limit    */

    /// the cell pool (`buffer`)
    cells: Vec<TCell>,
    /// `ycells`: array of cell linked-lists; one per vertical coordinate
    /// in the current band (C keeps it at the start of the pool)
    ycells: Vec<usize>,

    x: TPos,
    y: TPos, /* last point position */

    outline: &'a FtOutline, /* input outline */
    target: TPixmap,        /* target pixmap */
    target_buffer: Option<&'b mut [u8]>,

    render_span: Option<&'b mut dyn FnMut(i32, &[FtSpan])>,
}

/* The |x| value of the null cell.  Must be the largest possible */
/* integer value stored in a `TCell.x` field.                    */
const CELL_MAX_X_VALUE: TCoord = i32::MAX;

impl GrayTWorker<'_, '_> {
    /// `FT_INTEGRATE`
    #[inline]
    fn integrate(&mut self, a: TCoord, b: TCoord) {
        let c = &mut self.cells[self.cell];
        c.cover = c.cover.wrapping_add(a);
        c.area = c.area.wrapping_add(a.wrapping_mul(b));
    }

    /// `gray_set_cell`: Set the current cell to a new position.
    fn gray_set_cell(&mut self, ex: TCoord, ey: TCoord) {
        /* Move the cell pointer to a new position in the linked list. We use  */
        /* a dumpster null cell for everything outside of the clipping region  */
        /* during the render phase.  This means that:                          */
        /*                                                                     */
        /* . the new vertical position must be within min_ey..max_ey-1.        */
        /* . the new horizontal position must be strictly less than max_ex     */
        /*                                                                     */
        /* Note that if a cell is to the left of the clipping region, it is    */
        /* actually set to the (min_ex-1) horizontal position.                 */

        let ey_index = ey.wrapping_sub(self.min_ey);

        if self.jumped || ey_index < 0 || ey_index >= self.count_ey || ex >= self.max_ex {
            self.cell = self.cell_null;
        } else {
            let ex = std::cmp::max(ex, self.min_ex - 1);

            /* (`pcell` is the list head, or the `next` field of `prev`) */
            let mut prev: Option<usize> = None;
            let mut cell;

            loop {
                cell = match prev {
                    None => self.ycells[ey_index as usize],
                    Some(p) => self.cells[p].next,
                };
                if self.cells[cell].x > ex {
                    break;
                }

                if self.cells[cell].x == ex {
                    /* Found: */
                    self.cell = cell;
                    return;
                }

                prev = Some(cell);
            }

            /* insert new cell */
            let new = self.cell_free;
            self.cell_free += 1;
            if new >= self.cell_null {
                /* ft_longjmp( ras.jump_buffer, 1 ) */
                self.jumped = true;
                self.cell = self.cell_null;
                return;
            }

            self.cells[new] = TCell {
                x: ex,
                area: 0,
                cover: 0,
                next: cell,
            };
            match prev {
                None => self.ycells[ey_index as usize] = new,
                Some(p) => self.cells[p].next = new,
            }

            /* Found: */
            self.cell = new;
        }
    }

    /// `gray_render_line`: Render a straight line across multiple cells
    /// in any direction.
    fn gray_render_line(&mut self, to_x: TPos, to_y: TPos) {
        let ey1_ = trunc(self.y);
        let ey2 = trunc(to_y);

        /* perform vertical clipping */
        if (ey1_ >= self.max_ey && ey2 >= self.max_ey) || (ey1_ < self.min_ey && ey2 < self.min_ey)
        {
            /* End: */
            self.x = to_x;
            self.y = to_y;
            return;
        }

        let mut ey1 = ey1_;
        let mut ex1 = trunc(self.x);
        let ex2 = trunc(to_x);

        let mut fx1 = fract(self.x);
        let mut fy1 = fract(self.y);

        let dx = to_x.wrapping_sub(self.x);
        let dy = to_y.wrapping_sub(self.y);

        if ex1 == ex2 && ey1 == ey2 {
            /* inside one cell */
        } else if dy == 0 {
            /* ex1 != ex2 */
            /* any horizontal line */
            self.gray_set_cell(ex2, ey2);
            /* End: */
            self.x = to_x;
            self.y = to_y;
            return;
        } else if dx == 0 {
            if dy > 0 {
                /* vertical line up */
                loop {
                    let fy2 = ONE_PIXEL;
                    self.integrate(fy2 - fy1, fx1 * 2);
                    fy1 = 0;
                    ey1 = ey1.wrapping_add(1);
                    self.gray_set_cell(ex1, ey1);
                    if ey1 == ey2 {
                        break;
                    }
                }
            } else {
                /* vertical line down */
                loop {
                    let fy2 = 0;
                    self.integrate(fy2 - fy1, fx1 * 2);
                    fy1 = ONE_PIXEL;
                    ey1 = ey1.wrapping_sub(1);
                    self.gray_set_cell(ex1, ey1);
                    if ey1 == ey2 {
                        break;
                    }
                }
            }
        } else {
            /* any other line */
            let mut prod: i64 = dx
                .wrapping_mul(fy1 as i64)
                .wrapping_sub(dy.wrapping_mul(fx1 as i64));
            let dx_r = ft_udivprep(ex1 != ex2, dx);
            let dy_r = ft_udivprep(ey1 != ey2, dy);

            let one = ONE_PIXEL as i64;

            /* The fundamental value `prod' determines which side and the  */
            /* exact coordinate where the line exits current cell.  It is  */
            /* also easily updated when moving from one cell to the next.  */
            loop {
                let (fx2, fy2);
                if prod.wrapping_sub(dx.wrapping_mul(one)) > 0 && prod <= 0 {
                    /* left */
                    fx2 = 0;
                    fy2 = ft_udiv(prod.wrapping_neg(), dx_r.wrapping_neg());
                    prod = prod.wrapping_sub(dy.wrapping_mul(one));
                    self.integrate(fy2.wrapping_sub(fy1), fx1.wrapping_add(fx2));
                    fx1 = ONE_PIXEL;
                    fy1 = fy2;
                    ex1 = ex1.wrapping_sub(1);
                } else if prod
                    .wrapping_sub(dx.wrapping_mul(one))
                    .wrapping_add(dy.wrapping_mul(one))
                    > 0
                    && prod.wrapping_sub(dx.wrapping_mul(one)) <= 0
                {
                    /* up */
                    prod = prod.wrapping_sub(dx.wrapping_mul(one));
                    fx2 = ft_udiv(prod.wrapping_neg(), dy_r);
                    fy2 = ONE_PIXEL;
                    self.integrate(fy2.wrapping_sub(fy1), fx1.wrapping_add(fx2));
                    fx1 = fx2;
                    fy1 = 0;
                    ey1 = ey1.wrapping_add(1);
                } else if prod.wrapping_add(dy.wrapping_mul(one)) >= 0
                    && prod
                        .wrapping_sub(dx.wrapping_mul(one))
                        .wrapping_add(dy.wrapping_mul(one))
                        <= 0
                {
                    /* right */
                    prod = prod.wrapping_add(dy.wrapping_mul(one));
                    fx2 = ONE_PIXEL;
                    fy2 = ft_udiv(prod, dx_r);
                    self.integrate(fy2.wrapping_sub(fy1), fx1.wrapping_add(fx2));
                    fx1 = 0;
                    fy1 = fy2;
                    ex1 = ex1.wrapping_add(1);
                } else {
                    /* ( prod                                   >  0 &&
                    prod                  + dy * ONE_PIXEL <  0 )    down */
                    fx2 = ft_udiv(prod, dy_r.wrapping_neg());
                    fy2 = 0;
                    prod = prod.wrapping_add(dx.wrapping_mul(one));
                    self.integrate(fy2.wrapping_sub(fy1), fx1.wrapping_add(fx2));
                    fx1 = fx2;
                    fy1 = ONE_PIXEL;
                    ey1 = ey1.wrapping_sub(1);
                }

                self.gray_set_cell(ex1, ey1);

                if !(ex1 != ex2 || ey1 != ey2) {
                    break;
                }
            }
        }

        let fx2 = fract(to_x);
        let fy2 = fract(to_y);

        self.integrate(fy2.wrapping_sub(fy1), fx1.wrapping_add(fx2));

        /* End: */
        self.x = to_x;
        self.y = to_y;
    }

    /// `gray_render_conic` (BEZIER_USE_DDA)
    fn gray_render_conic(&mut self, control: &FtVector, to: &FtVector) {
        let p0 = FtVector {
            x: self.x,
            y: self.y,
        };
        let p1 = FtVector {
            x: upscale(control.x),
            y: upscale(control.y),
        };
        let p2 = FtVector {
            x: upscale(to.x),
            y: upscale(to.y),
        };

        /* short-cut the arc that crosses the current band */
        if (trunc(p0.y) >= self.max_ey && trunc(p1.y) >= self.max_ey && trunc(p2.y) >= self.max_ey)
            || (trunc(p0.y) < self.min_ey && trunc(p1.y) < self.min_ey && trunc(p2.y) < self.min_ey)
        {
            self.x = p2.x;
            self.y = p2.y;
            return;
        }

        let bx = p1.x.wrapping_sub(p0.x);
        let by = p1.y.wrapping_sub(p0.y);
        let ax = p2.x.wrapping_sub(p1.x).wrapping_sub(bx); /* p0.x + p2.x - 2 * p1.x */
        let ay = p2.y.wrapping_sub(p1.y).wrapping_sub(by); /* p0.y + p2.y - 2 * p1.y */

        let mut dx = ax.wrapping_abs();
        let dy = ay.wrapping_abs();
        if dx < dy {
            dx = dy;
        }

        if dx <= (ONE_PIXEL / 4) as TPos {
            self.gray_render_line(p2.x, p2.y);
            return;
        }

        /* We can calculate the number of necessary bisections because  */
        /* each bisection predictably reduces deviation exactly 4-fold. */
        /* Even 32-bit deviation would vanish after 16 bisections.      */
        let mut shift: i32 = 0;
        loop {
            dx >>= 2;
            shift += 1;

            if dx <= (ONE_PIXEL / 4) as TPos {
                break;
            }
        }

        /*
         * The (P0,P1,P2) arc equation, for t in [0,1] range:
         *
         * P(t) = P0*(1-t)^2 + P1*2*t*(1-t) + P2*t^2
         *
         * P(t) = P0 + 2*(P1-P0)*t + (P0+P2-2*P1)*t^2
         *      = P0 + 2*B*t + A*t^2
         *
         *    for A = P0 + P2 - 2*P1
         *    and B = P1 - P0
         *
         * Let's consider the difference when advancing by a small
         * parameter h:
         *
         *    Q(h,t) = P(t+h) - P(t) = 2*B*h + A*h^2 + 2*A*h*t
         *
         * And then its own difference:
         *
         *    R(h,t) = Q(h,t+h) - Q(h,t) = 2*A*h*h = R (constant)
         *
         * Since R is always a constant, it is possible to compute
         * successive positions with:
         *
         *     P = P0
         *     Q = Q(h,0) = 2*B*h + A*h*h
         *     R = 2*A*h*h
         *
         *   loop:
         *     P += Q
         *     Q += R
         *     EMIT(P)
         *
         * To ensure accurate results, perform computations on 64-bit
         * values, after scaling them by 2^32.
         *
         *           h = 1 / 2^N
         *
         *     R << 32 = 2 * A << (32 - N - N)
         *             = A << (33 - 2*N)
         *
         *     Q << 32 = (2 * B << (32 - N)) + (A << (32 - N - N))
         *             = (B << (33 - N)) + (A << (32 - 2*N))
         */

        /* (the SSE2 version, used for shift > 2, computes the same; */
        /* `left_shift' follows its out-of-range shift counts)       */

        let rx = left_shift(ax, 33 - 2 * shift);
        let ry = left_shift(ay, 33 - 2 * shift);

        let mut qx = left_shift(bx, 33 - shift).wrapping_add(left_shift(ax, 32 - 2 * shift));
        let mut qy = left_shift(by, 33 - shift).wrapping_add(left_shift(ay, 32 - 2 * shift));

        let mut px = left_shift(p0.x as i32 as i64, 32);
        let mut py = left_shift(p0.y as i32 as i64, 32);

        let mut count: u32 = 1u32 << shift;
        while count > 0 {
            px = px.wrapping_add(qx);
            py = py.wrapping_add(qy);
            qx = qx.wrapping_add(rx);
            qy = qy.wrapping_add(ry);

            self.gray_render_line(px >> 32, py >> 32);
            count -= 1;
        }
    }

    /// `gray_render_cubic`
    fn gray_render_cubic(&mut self, control1: &FtVector, control2: &FtVector, to: &FtVector) {
        let mut bez_stack = [FtVector::default(); 16 * 3 + 1]; /* enough to accommodate bisections */
        let mut arc = 0usize;

        bez_stack[0].x = upscale(to.x);
        bez_stack[0].y = upscale(to.y);
        bez_stack[1].x = upscale(control2.x);
        bez_stack[1].y = upscale(control2.y);
        bez_stack[2].x = upscale(control1.x);
        bez_stack[2].y = upscale(control1.y);
        bez_stack[3].x = self.x;
        bez_stack[3].y = self.y;

        /* short-cut the arc that crosses the current band */
        {
            let a = &bez_stack;
            if (trunc(a[0].y) >= self.max_ey
                && trunc(a[1].y) >= self.max_ey
                && trunc(a[2].y) >= self.max_ey
                && trunc(a[3].y) >= self.max_ey)
                || (trunc(a[0].y) < self.min_ey
                    && trunc(a[1].y) < self.min_ey
                    && trunc(a[2].y) < self.min_ey
                    && trunc(a[3].y) < self.min_ey)
            {
                self.x = a[0].x;
                self.y = a[0].y;
                return;
            }
        }

        let half = (ONE_PIXEL / 2) as TPos;
        loop {
            let a = &bez_stack[arc..];

            /* with each split, control points quickly converge towards  */
            /* chord trisection points and the vanishing distances below */
            /* indicate when the segment is flat enough to draw          */
            let split = (a[0]
                .x
                .wrapping_mul(2)
                .wrapping_sub(a[1].x.wrapping_mul(3))
                .wrapping_add(a[3].x))
            .wrapping_abs()
                > half
                || (a[0]
                    .y
                    .wrapping_mul(2)
                    .wrapping_sub(a[1].y.wrapping_mul(3))
                    .wrapping_add(a[3].y))
                .wrapping_abs()
                    > half
                || (a[0]
                    .x
                    .wrapping_sub(a[2].x.wrapping_mul(3))
                    .wrapping_add(a[3].x.wrapping_mul(2)))
                .wrapping_abs()
                    > half
                || (a[0]
                    .y
                    .wrapping_sub(a[2].y.wrapping_mul(3))
                    .wrapping_add(a[3].y.wrapping_mul(2)))
                .wrapping_abs()
                    > half;

            /* (C would split past the end of the stack for arcs that do */
            /* not flatten after 15 splits, which needs coordinates far  */
            /* beyond 32 bits; they are drawn as they are here)          */
            if split && arc + 6 < bez_stack.len() {
                /* Split: */
                gray_split_cubic(&mut bez_stack[arc..]);
                arc += 3;
                continue;
            }

            let (x, y) = (bez_stack[arc].x, bez_stack[arc].y);
            self.gray_render_line(x, y);

            if arc == 0 {
                return;
            }

            arc -= 3;
        }
    }

    /// `gray_sweep`
    fn gray_sweep(&mut self) {
        let fill: i32 = if self.outline.flags & FT_OUTLINE_EVEN_ODD_FILL != 0 {
            0x100
        } else {
            i32::MIN
        };

        let target = self.target;
        let buffer = self.target_buffer.as_deref_mut().unwrap();

        for y in self.min_ey..self.max_ey {
            let mut cell = self.ycells[(y - self.min_ey) as usize];
            let mut x = self.min_ex;
            let mut cover: TArea = 0;

            let line = target.origin - target.pitch * y as isize;

            while cell != self.cell_null {
                let c = self.cells[cell];

                if cover != 0 && c.x > x {
                    let coverage = ft_fill_rule(cover, fill);
                    let start = (line + x as isize) as usize;
                    buffer[start..start + (c.x - x) as usize].fill(coverage as u8);
                }

                cover = cover.wrapping_add(c.cover.wrapping_mul(ONE_PIXEL * 2));
                let area = cover.wrapping_sub(c.area);

                if area != 0 && c.x >= self.min_ex {
                    let coverage = ft_fill_rule(area, fill);
                    buffer[(line + c.x as isize) as usize] = coverage as u8;
                }

                x = c.x + 1;
                cell = c.next;
            }

            if cover != 0 {
                /* only if cropped */
                let coverage = ft_fill_rule(cover, fill);
                let start = (line + x as isize) as usize;
                buffer[start..start + (self.max_ex - x) as usize].fill(coverage as u8);
            }
        }
    }

    /// `gray_sweep_direct`
    fn gray_sweep_direct(&mut self) {
        let fill: i32 = if self.outline.flags & FT_OUTLINE_EVEN_ODD_FILL != 0 {
            0x100
        } else {
            i32::MIN
        };

        let mut span = [FtSpan::default(); FT_MAX_GRAY_SPANS];
        let mut n = 0usize;

        let render_span = self.render_span.as_deref_mut().unwrap();

        for y in self.min_ey..self.max_ey {
            let mut cell = self.ycells[(y - self.min_ey) as usize];
            let mut x = self.min_ex;
            let mut cover: TArea = 0;

            while cell != self.cell_null {
                let c = self.cells[cell];

                if cover != 0 && c.x > x {
                    let coverage = ft_fill_rule(cover, fill);

                    span[n].coverage = coverage as u8;
                    span[n].x = x as i16;
                    span[n].len = (c.x - x) as u16;

                    n += 1;
                    if n == FT_MAX_GRAY_SPANS {
                        /* flush the span buffer and reset the count */
                        render_span(y, &span[..n]);
                        n = 0;
                    }
                }

                cover = cover.wrapping_add(c.cover.wrapping_mul(ONE_PIXEL * 2));
                let area = cover.wrapping_sub(c.area);

                if area != 0 && c.x >= self.min_ex {
                    let coverage = ft_fill_rule(area, fill);

                    span[n].coverage = coverage as u8;
                    span[n].x = c.x as i16;
                    span[n].len = 1;

                    n += 1;
                    if n == FT_MAX_GRAY_SPANS {
                        /* flush the span buffer and reset the count */
                        render_span(y, &span[..n]);
                        n = 0;
                    }
                }

                x = c.x + 1;
                cell = c.next;
            }

            if cover != 0 {
                /* only if cropped */
                let coverage = ft_fill_rule(cover, fill);

                span[n].coverage = coverage as u8;
                span[n].x = x as i16;
                span[n].len = (self.max_ex - x) as u16;

                n += 1;
            }

            if n != 0 {
                /* flush the span buffer and reset the count */
                render_span(y, &span[..n]);
                n = 0;
            }
        }
    }
}

/// `LEFT_SHIFT`, as `_mm_slli_epi64` computes it (0 for shift counts out
/// of range)
#[inline]
fn left_shift(a: i64, b: i32) -> i64 {
    if !(0..=63).contains(&b) {
        0
    } else {
        ((a as u64) << b) as i64
    }
}

/// `gray_split_cubic`: For cubic Bézier, binary splits are still faster
/// than DDA because the splits are adaptive to how quickly each sub-arc
/// approaches their chord trisection points.
///
/// It might be useful to experiment with SSE2 to speed up
/// `gray_split_cubic`, though.
fn gray_split_cubic(base: &mut [FtVector]) {
    base[6].x = base[3].x;
    let mut a = base[0].x.wrapping_add(base[1].x);
    let b = base[1].x.wrapping_add(base[2].x);
    let mut c = base[2].x.wrapping_add(base[3].x);
    base[5].x = c >> 1;
    c = c.wrapping_add(b);
    base[4].x = c >> 2;
    base[1].x = a >> 1;
    a = a.wrapping_add(b);
    base[2].x = a >> 2;
    base[3].x = a.wrapping_add(c) >> 3;

    base[6].y = base[3].y;
    let mut a = base[0].y.wrapping_add(base[1].y);
    let b = base[1].y.wrapping_add(base[2].y);
    let mut c = base[2].y.wrapping_add(base[3].y);
    base[5].y = c >> 1;
    c = c.wrapping_add(b);
    base[4].y = c >> 2;
    base[1].y = a >> 1;
    a = a.wrapping_add(b);
    base[2].y = a >> 2;
    base[3].y = a.wrapping_add(c) >> 3;
}

/* the error with which the callbacks stop the decomposition after the */
/* pool overflowed (C's `longjmp')                                     */
const GRAY_JUMPED: FtError = -1;

/// `func_interface`: `gray_move_to`, `gray_line_to`, `gray_conic_to`, and
/// `gray_cubic_to`
impl FtOutlineFuncs for GrayTWorker<'_, '_> {
    /// `gray_move_to`
    fn move_to(&mut self, to: &FtVector) -> FtError {
        /* start to a new position */
        let x = upscale(to.x);
        let y = upscale(to.y);

        self.gray_set_cell(trunc(x), trunc(y));

        self.x = x;
        self.y = y;

        if self.jumped {
            GRAY_JUMPED
        } else {
            0
        }
    }

    /// `gray_line_to`
    fn line_to(&mut self, to: &FtVector) -> FtError {
        self.gray_render_line(upscale(to.x), upscale(to.y));

        if self.jumped {
            GRAY_JUMPED
        } else {
            0
        }
    }

    /// `gray_conic_to`
    fn conic_to(&mut self, control: &FtVector, to: &FtVector) -> FtError {
        self.gray_render_conic(control, to);

        if self.jumped {
            GRAY_JUMPED
        } else {
            0
        }
    }

    /// `gray_cubic_to`
    fn cubic_to(&mut self, control1: &FtVector, control2: &FtVector, to: &FtVector) -> FtError {
        self.gray_render_cubic(control1, control2, to);

        if self.jumped {
            GRAY_JUMPED
        } else {
            0
        }
    }
}

/// `gray_convert_glyph_inner`
fn gray_convert_glyph_inner(worker: &mut GrayTWorker<'_, '_>, _continued: bool) -> FtResult<()> {
    worker.jumped = false;

    let outline = worker.outline;
    let r = ft_outline_decompose(outline, worker, 0, 0);

    if worker.jumped {
        Err(FT_ERR_RASTER_OVERFLOW)
    } else {
        r
    }
}

/// `gray_convert_glyph`
fn gray_convert_glyph(worker: &mut GrayTWorker<'_, '_>) -> FtResult<()> {
    let y_min = worker.min_ey;
    let y_max = worker.max_ey;

    let mut height = (y_max - y_min) as usize;
    let mut n = FT_MAX_GRAY_POOL / 8;
    let mut bands: [TCoord; 32] = [0; 32]; /* enough to accommodate bisections */

    let mut continued = false;

    /* Initialize the null cell at the end of the poll. */
    worker.cells = vec![TCell::default(); FT_MAX_GRAY_POOL];
    worker.cell_null = FT_MAX_GRAY_POOL - 1;
    worker.cells[worker.cell_null] = TCell {
        x: CELL_MAX_X_VALUE,
        area: 0,
        cover: 0,
        next: usize::MAX,
    };

    /* set up vertical bands */
    /* (`ycells` is a separate array; the pool space C gives it is */
    /* skipped below)                                              */

    if height > n {
        /* two divisions rounded up */
        n = height.div_ceil(n);
        height = height.div_ceil(n);
    }

    let mut y = y_min;
    while y < y_max {
        worker.min_ey = y;
        y = y.wrapping_add(height as TCoord);
        worker.max_ey = std::cmp::min(y, y_max);

        let mut band: isize = 0;
        bands[1] = worker.min_ey;
        bands[0] = worker.max_ey;

        loop {
            let b = band as usize;
            let mut width = bands[b] - bands[b + 1];

            worker.ycells.clear();
            worker.ycells.resize(width as usize, worker.cell_null);

            /* memory management: skip ycells */
            n = (width as usize * SIZEOF_PCELL).div_ceil(SIZEOF_TCELL);

            worker.cell_free = n;
            worker.cell = worker.cell_null;
            worker.min_ey = bands[b + 1];
            worker.max_ey = bands[b];
            worker.count_ey = width;

            let error = gray_convert_glyph_inner(worker, continued);
            continued = true;

            match error {
                Ok(()) => {
                    if worker.render_span.is_some() {
                        /* for FT_RASTER_FLAG_DIRECT only */
                        worker.gray_sweep_direct();
                    } else {
                        worker.gray_sweep();
                    }
                    band -= 1;
                    if band < 0 {
                        break;
                    }
                    continue;
                }
                Err(e) if e != FT_ERR_RASTER_OVERFLOW => return Err(e),
                Err(_) => {}
            }

            /* render pool overflow; we will reduce the render band by half */
            width >>= 1;

            /* this should never happen even with tiny rendering pool */
            if width == 0 {
                return Err(FT_ERR_RASTER_OVERFLOW);
            }

            band += 1;
            let b = band as usize;
            bands[b + 1] = bands[b];
            bands[b] += width;
        }
    }

    Ok(())
}

/// `gray_raster_render`
fn gray_raster_render(source: FtRasterSource<'_>, params: &mut FtRasterParams<'_>) -> FtResult<()> {
    /* this version does not support monochrome rendering */
    if params.flags & FT_RASTER_FLAG_AA == 0 {
        return Err(FT_ERR_CANNOT_RENDER_GLYPH);
    }

    let outline = match source {
        FtRasterSource::Outline(o) => o,
        FtRasterSource::Bitmap(_) => return Err(FT_ERR_INVALID_OUTLINE),
    };

    /* return immediately if the outline is empty */
    if outline.n_points == 0 || outline.n_contours <= 0 {
        return Ok(());
    }

    if outline.contours.is_empty() || outline.points.is_empty() {
        return Err(FT_ERR_INVALID_OUTLINE);
    }

    if outline.n_points as i32 != outline.contours[outline.n_contours as usize - 1] as i32 + 1 {
        return Err(FT_ERR_INVALID_OUTLINE);
    }

    let mut worker = GrayTWorker {
        jumped: false,
        min_ex: 0,
        max_ex: 0,
        min_ey: 0,
        max_ey: 0,
        count_ey: 0,
        cell: 0,
        cell_free: 0,
        cell_null: 0,
        cells: Vec::new(),
        ycells: Vec::new(),
        x: 0,
        y: 0,
        outline,
        target: TPixmap::default(),
        target_buffer: None,
        render_span: None,
    };

    if params.flags & FT_RASTER_FLAG_DIRECT != 0 {
        let Some(gray_spans) = params.gray_spans.as_deref_mut() else {
            return Ok(());
        };

        worker.render_span = Some(gray_spans);

        worker.min_ex = params.clip_box.xMin as TCoord;
        worker.min_ey = params.clip_box.yMin as TCoord;
        worker.max_ex = params.clip_box.xMax as TCoord;
        worker.max_ey = params.clip_box.yMax as TCoord;
    } else {
        /* if direct mode is not set, we must have a target bitmap */
        let Some(target_map) = params.target.as_deref_mut() else {
            return Err(FT_ERR_INVALID_ARGUMENT);
        };

        /* nothing to do */
        if target_map.width == 0 || target_map.rows == 0 {
            return Ok(());
        }

        if target_map.buffer.is_empty() {
            return Err(FT_ERR_INVALID_ARGUMENT);
        }

        worker.target.origin = if target_map.pitch < 0 {
            0
        } else {
            (target_map.rows as isize - 1) * target_map.pitch as isize
        };

        worker.target.pitch = target_map.pitch as isize;

        worker.min_ex = 0;
        worker.min_ey = 0;
        worker.max_ex = target_map.width as TCoord;
        worker.max_ey = target_map.rows as TCoord;

        worker.target_buffer = Some(&mut target_map.buffer[..]);
    }

    /* exit if nothing to do */
    if worker.max_ex <= worker.min_ex || worker.max_ey <= worker.min_ey {
        return Ok(());
    }

    gray_convert_glyph(&mut worker)
}

/// `ft_grays_raster` (`gray_raster_new`, `gray_raster_reset`,
/// `gray_raster_set_mode`, and `gray_raster_done` have nothing to do)
pub static FT_GRAYS_RASTER: FtRasterFuncs = FtRasterFuncs {
    glyph_format: FT_GLYPH_FORMAT_OUTLINE,
    raster_render: gray_raster_render,
};
