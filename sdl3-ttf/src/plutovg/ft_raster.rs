// Rust translation of source/plutovg-ft-raster.c and plutovg-ft-raster.h
// from PlutoVG (0.0.12, as SDL_ttf's external/plutovg pins it): PlutoVG's
// copy of FreeType's ftgrays.c and ftimage.h.
// Copyright 2000-2003, 2005-2014 by David Turner, Robert Wilhelm, and
// Werner Lemberg (ftgrays.c; ftimage.h: Copyright 1996-2010, 2013).
// This file is part of the FreeType project, and may only be used,
// modified, and distributed under the terms of the FreeType project
// license, FTL.TXT. This is an altered (translated) version of the
// original software; it is used under the FreeType License (see
// sdl3-ttf/FTL.TXT and LICENSE.txt).

//! A new `perfect' anti-aliasing renderer (body).
//!
//! This is a new anti-aliasing scan-converter for FreeType 2.  The
//! algorithm used here is _very_ different from the one in the standard
//! `ftraster' module.  Actually, `ftgrays' computes the _exact_ coverage
//! of the outline on each pixel cell.
//!
//! It is based on ideas that I initially found in Raph Levien's excellent
//! LibArt graphics library (see <http://www.levien.com/libart> for more
//! information, though the web pages do not tell anything about the
//! renderer; you'll have to dive into the source code to understand how
//! it works).
//!
//! Note, however, that this is a _very_ different implementation compared
//! to Raph's.  Coverage information is stored in a very different way,
//! and I don't use sorted vector paths.  Also, it doesn't use floating
//! point values.
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
//!   each pixel cell.
//!
//! - It performs a single pass on the outline (the `standard' FT2
//!   renderer makes two passes).
//!
//! - It can easily be modified to render to _any_ number of gray levels
//!   cheaply.
//!
//! - For small (< 20) pixel sizes, it is faster than the standard
//!   renderer.
//!
//! Translation notes: the render pool is C's byte buffer, laid out as on
//! x86-64 (eight-byte cell pointers, 24-byte cells), as indices; the
//! longjmp out of a full pool is an `Err`.

use super::ft_math::*;

/// `PVG_FT_BBox`
#[derive(Debug, Clone, Copy, Default)]
pub struct PvgFtBBox {
    pub x_min: PvgFtPos,
    pub y_min: PvgFtPos,
    pub x_max: PvgFtPos,
    pub y_max: PvgFtPos,
}

/// `PVG_FT_Outline`: the arrays have the sizes they were created with
/// (`ft_outline_create`); `n_points` and `n_contours` count their entries
/// in use.
#[derive(Debug, Clone, Default)]
pub struct PvgFtOutline {
    pub n_contours: i32,          /* number of contours in glyph        */
    pub n_points: i32,            /* number of points in the glyph      */
    pub points: Vec<PvgFtVector>, /* the outline's points               */
    pub tags: Vec<u8>,            /* the points flags                   */
    pub contours: Vec<i32>,       /* the contour end points             */
    pub contours_flag: Vec<u8>,   /* the contour open flags             */
    pub flags: i32,               /* outline masks                      */
}

pub const PVG_FT_OUTLINE_NONE: i32 = 0x0;
pub const PVG_FT_OUTLINE_OWNER: i32 = 0x1;
pub const PVG_FT_OUTLINE_EVEN_ODD_FILL: i32 = 0x2;
pub const PVG_FT_OUTLINE_REVERSE_FILL: i32 = 0x4;

/// `PVG_FT_CURVE_TAG`
#[inline]
pub fn pvg_ft_curve_tag(flag: u8) -> u8 {
    flag & 3
}

pub const PVG_FT_CURVE_TAG_ON: u8 = 1;
pub const PVG_FT_CURVE_TAG_CONIC: u8 = 0;
pub const PVG_FT_CURVE_TAG_CUBIC: u8 = 2;

/// `PVG_FT_Span`: a single span of gray pixels.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct PvgFtSpan {
    pub x: i32,
    pub len: i32,
    pub y: i32,
    pub coverage: u8,
}

pub const PVG_FT_RASTER_FLAG_DEFAULT: i32 = 0x0;
pub const PVG_FT_RASTER_FLAG_AA: i32 = 0x1;
pub const PVG_FT_RASTER_FLAG_DIRECT: i32 = 0x2;
pub const PVG_FT_RASTER_FLAG_CLIP: i32 = 0x4;

/// The `gray_spans` callback (with its `user` data).
pub type PvgFtSpanFunc<'a> = dyn FnMut(&[PvgFtSpan]) + 'a;

/// `PVG_FT_Raster_Params`
#[allow(missing_debug_implementations)]
pub struct PvgFtRasterParams<'a> {
    pub source: Option<&'a PvgFtOutline>,
    pub flags: i32,
    /// `gray_spans` (with its `user` data)
    pub gray_spans: Option<&'a mut PvgFtSpanFunc<'a>>,
    pub clip_box: PvgFtBBox,
}

const ERR_RASTER_INVALID_MODE: i32 = -2;
const ERR_RASTER_INVALID_OUTLINE: i32 = -1;
#[allow(dead_code)]
const ERR_RASTER_INVALID_ARGUMENT: i32 = -3;
const ERR_RASTER_MEMORY_OVERFLOW: i32 = -4;
const ERR_RASTER_OUT_OF_MEMORY: i32 = -6;

const PVG_FT_MINIMUM_POOL_SIZE: i64 = 8192;

/* must be at least 6 bits! */
const PIXEL_BITS: i64 = 8;

const ONE_PIXEL: i64 = 1 << PIXEL_BITS;

/// `TRUNC`
#[inline]
fn trunc(x: i64) -> TCoord {
    x >> PIXEL_BITS
}

/// `FRACT`
#[inline]
fn fract(x: i64) -> TCoord {
    x & (ONE_PIXEL - 1)
}

/// `UPSCALE`
#[inline]
fn upscale(x: i64) -> i64 {
    x.wrapping_mul(ONE_PIXEL >> 6)
}

/// `PVG_FT_DIV_MOD`: compute `dividend / divisor' and return both its
/// quotient and remainder.  This also ensures that the remainder is
/// always positive.
#[inline]
fn pvg_ft_div_mod(dividend: i64, divisor: i64) -> (i64, i64) {
    if divisor == 0 {
        /* (never: the callers' divisors are non-zero) */
        return (0, 0);
    }
    let mut quotient = dividend.wrapping_div(divisor);
    let mut remainder = dividend.wrapping_rem(divisor);
    if remainder < 0 {
        quotient = quotient.wrapping_sub(1);
        remainder = remainder.wrapping_add(divisor);
    }
    (quotient, remainder)
}

/* don't change the following types to PVG_FT_Int or PVG_FT_Pos, since we might */
/* need to define them to "float" or "double" when experimenting with   */
/* new algorithms                                                       */

type TCoord = i64; /* integer scanline/pixel coordinate */
type TPos = i64; /* sub-pixel coordinate              */
type TArea = i64; /* cell areas, coordinate products   */

/* maximal number of gray spans in a call to the span callback */
const PVG_FT_MAX_GRAY_SPANS: usize = 256;

/// `TCell` (`next` is an index into the cells)
#[derive(Debug, Clone, Copy, Default)]
struct TCell {
    x: i32,
    cover: i32,
    area: TArea,
    next: Option<u32>,
}

/// The sizes of C's `PCell` and `TCell` on x86-64.
const SIZEOF_PCELL: i64 = 8;
const SIZEOF_TCELL: i64 = 24;

/// The render pool overflowed (C's `longjmp`).
struct PoolOverflow;

/// `TWorker`
struct TWorker<'a, 'f> {
    ex: TCoord,
    ey: TCoord,
    min_ex: TPos,
    max_ex: TPos,
    min_ey: TPos,
    max_ey: TPos,
    count_ex: TPos,
    count_ey: TPos,

    area: TArea,
    cover: i32,
    invalid: bool,

    cells: Vec<TCell>,
    max_cells: i64,

    x: TPos,
    y: TPos,

    outline: &'a PvgFtOutline,
    clip_box: PvgFtBBox,

    gray_spans: [PvgFtSpan; PVG_FT_MAX_GRAY_SPANS],
    num_gray_spans: i32,
    skip_spans: i32,

    render_span: Option<&'f mut PvgFtSpanFunc<'f>>,

    band_size: i32,
    band_shoot: i32,

    buffer_size: i64,

    ycells: Vec<Option<u32>>,
    ycount: TPos,
}

impl TWorker<'_, '_> {
    /// `gray_init_cells`: initialize the cells table.
    fn gray_init_cells(&mut self, byte_size: i64) {
        self.buffer_size = byte_size;

        self.ycells.clear();
        self.cells.clear();
        self.max_cells = 0;
        self.area = 0;
        self.cover = 0;
        self.invalid = true;
    }

    /// `gray_compute_cbox`: compute the outline bounding box.
    fn gray_compute_cbox(&mut self) {
        let outline = self.outline;

        if outline.n_points <= 0 {
            self.min_ex = 0;
            self.max_ex = 0;
            self.min_ey = 0;
            self.max_ey = 0;
            return;
        }

        let points = &outline.points[..outline.n_points as usize];
        self.min_ex = points[0].x;
        self.max_ex = points[0].x;
        self.min_ey = points[0].y;
        self.max_ey = points[0].y;

        for vec in &points[1..] {
            let x = vec.x;
            let y = vec.y;

            if x < self.min_ex {
                self.min_ex = x;
            }
            if x > self.max_ex {
                self.max_ex = x;
            }
            if y < self.min_ey {
                self.min_ey = y;
            }
            if y > self.max_ey {
                self.max_ey = y;
            }
        }

        /* truncate the bounding box to integer pixels */
        self.min_ex >>= 6;
        self.min_ey >>= 6;
        self.max_ex = self.max_ex.wrapping_add(63) >> 6;
        self.max_ey = self.max_ey.wrapping_add(63) >> 6;
    }

    /// `gray_find_cell`: record the current cell in the table.
    fn gray_find_cell(&mut self) -> Result<usize, PoolOverflow> {
        let mut x = self.ex;

        if x > self.count_ex {
            x = self.count_ex;
        }

        /* (ey is within 0..ycount: the current cell is valid) */
        let row = self.ey as usize;
        let mut prev: Option<usize> = None;
        let mut cur = self.ycells.get(row).copied().flatten();
        while let Some(c) = cur {
            let cell = self.cells[c as usize];
            if cell.x as i64 > x {
                break;
            }

            if cell.x as i64 == x {
                return Ok(c as usize);
            }

            prev = Some(c as usize);
            cur = cell.next;
        }

        if self.cells.len() as i64 >= self.max_cells {
            return Err(PoolOverflow);
        }

        let index = self.cells.len();
        self.cells.push(TCell {
            x: x as i32,
            area: 0,
            cover: 0,
            next: cur,
        });
        match prev {
            Some(p) => self.cells[p].next = Some(index as u32),
            None => {
                if let Some(slot) = self.ycells.get_mut(row) {
                    *slot = Some(index as u32);
                }
            }
        }

        Ok(index)
    }

    /// `gray_record_cell`
    fn gray_record_cell(&mut self) -> Result<(), PoolOverflow> {
        if (self.area | self.cover as i64) != 0 {
            let cell = self.gray_find_cell()?;

            let c = &mut self.cells[cell];
            c.area = c.area.wrapping_add(self.area);
            c.cover = c.cover.wrapping_add(self.cover);
        }
        Ok(())
    }

    /// `gray_set_cell`: set the current cell to a new position.
    fn gray_set_cell(&mut self, mut ex: TCoord, mut ey: TCoord) -> Result<(), PoolOverflow> {
        /* Move the cell pointer to a new position.  We set the `invalid'      */
        /* flag to indicate that the cell isn't part of those we're interested */
        /* in during the render phase.  This means that:                       */
        /*                                                                     */
        /* . the new vertical position must be within min_ey..max_ey-1.        */
        /* . the new horizontal position must be strictly less than max_ex     */
        /*                                                                     */
        /* Note that if a cell is to the left of the clipping region, it is    */
        /* actually set to the (min_ex-1) horizontal position.                 */

        /* All cells that are on the left of the clipping region go to the */
        /* min_ex - 1 horizontal position.                                 */
        ey = ey.wrapping_sub(self.min_ey);

        if ex > self.max_ex {
            ex = self.max_ex;
        }

        ex = ex.wrapping_sub(self.min_ex);
        if ex < 0 {
            ex = -1;
        }

        /* are we moving to a different cell ? */
        if ex != self.ex || ey != self.ey {
            /* record the current one if it is valid */
            if !self.invalid {
                self.gray_record_cell()?;
            }

            self.area = 0;
            self.cover = 0;
            self.ex = ex;
            self.ey = ey;
        }

        self.invalid = (ey as u32) >= (self.count_ey as u32) || ex >= self.count_ex;
        Ok(())
    }

    /// `gray_start_cell`: start a new contour at a given cell.
    fn gray_start_cell(&mut self, mut ex: TCoord, ey: TCoord) -> Result<(), PoolOverflow> {
        if ex > self.max_ex {
            ex = self.max_ex;
        }

        if ex < self.min_ex {
            ex = self.min_ex - 1;
        }

        self.area = 0;
        self.cover = 0;
        self.ex = ex.wrapping_sub(self.min_ex);
        self.ey = ey.wrapping_sub(self.min_ey);
        self.invalid = false;

        self.gray_set_cell(ex, ey)
    }

    /// `gray_render_scanline`: render a scanline as one or more cells.
    fn gray_render_scanline(
        &mut self,
        ey: TCoord,
        x1: TPos,
        mut y1: TCoord,
        x2: TPos,
        y2: TCoord,
    ) -> Result<(), PoolOverflow> {
        let mut ex1 = trunc(x1);
        let ex2 = trunc(x2);

        /* trivial case.  Happens often */
        if y1 == y2 {
            return self.gray_set_cell(ex2, ey);
        }

        let mut fx1 = fract(x1);
        let fx2 = fract(x2);

        /* everything is located in a single cell.  That is easy! */
        /*                                                        */
        if ex1 != ex2 {
            /* ok, we'll have to render a run of adjacent cells on the same */
            /* scanline...                                                  */
            /*                                                              */
            let mut dx = x2.wrapping_sub(x1);
            let dy = y2.wrapping_sub(y1);

            let p;
            let first;
            let incr: i64;
            if dx > 0 {
                p = (ONE_PIXEL - fx1).wrapping_mul(dy);
                first = ONE_PIXEL;
                incr = 1;
            } else {
                p = fx1.wrapping_mul(dy);
                first = 0;
                incr = -1;
                dx = dx.wrapping_neg();
            }

            let (delta, mut modv) = pvg_ft_div_mod(p, dx);

            self.area = self.area.wrapping_add((fx1 + first).wrapping_mul(delta));
            self.cover = self.cover.wrapping_add(delta as i32);
            y1 = y1.wrapping_add(delta);
            ex1 = ex1.wrapping_add(incr);
            self.gray_set_cell(ex1, ey)?;

            if ex1 != ex2 {
                let p = ONE_PIXEL.wrapping_mul(dy);
                let (lift, rem) = pvg_ft_div_mod(p, dx);

                loop {
                    let mut delta = lift;
                    modv = modv.wrapping_add(rem);
                    if modv >= dx {
                        modv = modv.wrapping_sub(dx);
                        delta = delta.wrapping_add(1);
                    }

                    self.area = self.area.wrapping_add(ONE_PIXEL.wrapping_mul(delta));
                    self.cover = self.cover.wrapping_add(delta as i32);
                    y1 = y1.wrapping_add(delta);
                    ex1 = ex1.wrapping_add(incr);
                    self.gray_set_cell(ex1, ey)?;
                    if ex1 == ex2 {
                        break;
                    }
                }
            }
            fx1 = ONE_PIXEL - first;
        }

        /* End: */
        let dy = y2.wrapping_sub(y1);

        self.area = self
            .area
            .wrapping_add((fx1.wrapping_add(fx2)).wrapping_mul(dy));
        self.cover = self.cover.wrapping_add(dy as i32);
        Ok(())
    }

    /// `gray_render_line`: render a given line as a series of scanlines.
    fn gray_render_line(&mut self, to_x: TPos, to_y: TPos) -> Result<(), PoolOverflow> {
        let mut ey1 = trunc(self.y);
        let ey2 = trunc(to_y); /* if (ey2 >= ras.max_ey) ey2 = ras.max_ey-1; */

        /* perform vertical clipping */
        if !((ey1 >= self.max_ey && ey2 >= self.max_ey) || (ey1 < self.min_ey && ey2 < self.min_ey))
        {
            let fy1 = fract(self.y);
            let fy2 = fract(to_y);

            /* everything is on a single scanline */
            if ey1 == ey2 {
                self.gray_render_scanline(ey1, self.x, fy1, to_x, fy2)?;
            } else {
                let dx = to_x.wrapping_sub(self.x);
                let mut dy = to_y.wrapping_sub(self.y);

                /* vertical line - avoid calling gray_render_scanline */
                if dx == 0 {
                    let ex = trunc(self.x);
                    let two_fx = fract(self.x) << 1;

                    let first = if dy > 0 { ONE_PIXEL } else { 0 };

                    let mut delta = first - fy1;
                    self.area = self.area.wrapping_add(two_fx.wrapping_mul(delta));
                    self.cover = self.cover.wrapping_add(delta as i32);

                    delta = first + first - ONE_PIXEL;
                    let area = two_fx.wrapping_mul(delta);
                    let max_ey1 = self.count_ey.wrapping_add(self.min_ey);
                    if dy < 0 {
                        if ey1 > max_ey1 {
                            ey1 = if max_ey1 > ey2 { max_ey1 } else { ey2 };
                            self.gray_set_cell(ex, ey1)?;
                        } else {
                            ey1 -= 1;
                            self.gray_set_cell(ex, ey1)?;
                        }
                        while ey1 > ey2 && ey1 >= self.min_ey {
                            self.area = self.area.wrapping_add(area);
                            self.cover = self.cover.wrapping_add(delta as i32);
                            ey1 -= 1;

                            self.gray_set_cell(ex, ey1)?;
                        }
                        if ey1 != ey2 {
                            ey1 = ey2;
                            self.gray_set_cell(ex, ey1)?;
                        }
                    } else {
                        if ey1 < self.min_ey {
                            ey1 = if self.min_ey < ey2 { self.min_ey } else { ey2 };
                            self.gray_set_cell(ex, ey1)?;
                        } else {
                            ey1 += 1;
                            self.gray_set_cell(ex, ey1)?;
                        }
                        while ey1 < ey2 && ey1 < max_ey1 {
                            self.area = self.area.wrapping_add(area);
                            self.cover = self.cover.wrapping_add(delta as i32);
                            ey1 += 1;

                            self.gray_set_cell(ex, ey1)?;
                        }
                        if ey1 != ey2 {
                            ey1 = ey2;
                            self.gray_set_cell(ex, ey1)?;
                        }
                    }

                    let delta = (fy2 - ONE_PIXEL + first) as i32;
                    self.area = self.area.wrapping_add(two_fx.wrapping_mul(delta as i64));
                    self.cover = self.cover.wrapping_add(delta);
                } else {
                    /* ok, we have to render several scanlines */
                    let p;
                    let first;
                    let incr: i64;
                    if dy > 0 {
                        p = (ONE_PIXEL - fy1).wrapping_mul(dx);
                        first = ONE_PIXEL;
                        incr = 1;
                    } else {
                        p = fy1.wrapping_mul(dx);
                        first = 0;
                        incr = -1;
                        dy = dy.wrapping_neg();
                    }

                    /* the fractional part of x-delta is mod/dy. It is essential to */
                    /* keep track of its accumulation for accurate rendering.       */
                    let (delta, mut modv) = pvg_ft_div_mod(p, dy);

                    let mut x = self.x.wrapping_add(delta);
                    self.gray_render_scanline(ey1, self.x, fy1, x, first)?;

                    ey1 = ey1.wrapping_add(incr);
                    self.gray_set_cell(trunc(x), ey1)?;

                    if ey1 != ey2 {
                        let p = ONE_PIXEL.wrapping_mul(dx);
                        let (lift, rem) = pvg_ft_div_mod(p, dy);

                        loop {
                            let mut delta = lift;
                            modv = modv.wrapping_add(rem);
                            if modv >= dy {
                                modv = modv.wrapping_sub(dy);
                                delta = delta.wrapping_add(1);
                            }

                            let x2 = x.wrapping_add(delta);
                            self.gray_render_scanline(ey1, x, ONE_PIXEL - first, x2, first)?;
                            x = x2;

                            ey1 = ey1.wrapping_add(incr);
                            self.gray_set_cell(trunc(x), ey1)?;
                            if ey1 == ey2 {
                                break;
                            }
                        }
                    }

                    self.gray_render_scanline(ey1, x, ONE_PIXEL - first, to_x, fy2)?;
                }
            }
        }

        /* End: */
        self.x = to_x;
        self.y = to_y;
        Ok(())
    }

    /// `gray_render_conic`
    fn gray_render_conic(
        &mut self,
        control: &PvgFtVector,
        to: &PvgFtVector,
    ) -> Result<(), PoolOverflow> {
        let mut bez_stack = [PvgFtVector::default(); 16 * 2 + 1]; /* enough to accommodate bisections */
        let mut arc = 0usize;

        bez_stack[0].x = upscale(to.x);
        bez_stack[0].y = upscale(to.y);
        bez_stack[1].x = upscale(control.x);
        bez_stack[1].y = upscale(control.y);
        bez_stack[2].x = self.x;
        bez_stack[2].y = self.y;

        /* short-cut the arc that crosses the current band */
        if (trunc(bez_stack[0].y) >= self.max_ey
            && trunc(bez_stack[1].y) >= self.max_ey
            && trunc(bez_stack[2].y) >= self.max_ey)
            || (trunc(bez_stack[0].y) < self.min_ey
                && trunc(bez_stack[1].y) < self.min_ey
                && trunc(bez_stack[2].y) < self.min_ey)
        {
            self.x = bez_stack[0].x;
            self.y = bez_stack[0].y;
            return Ok(());
        }

        let a = &bez_stack;
        let mut dx = pvg_ft_abs(
            a[2].x
                .wrapping_add(a[0].x)
                .wrapping_sub(a[1].x.wrapping_mul(2)),
        );
        let dy = pvg_ft_abs(
            a[2].y
                .wrapping_add(a[0].y)
                .wrapping_sub(a[1].y.wrapping_mul(2)),
        );
        if dx < dy {
            dx = dy;
        }

        /* We can calculate the number of necessary bisections because  */
        /* each bisection predictably reduces deviation exactly 4-fold. */
        /* Even 32-bit deviation would vanish after 16 bisections.      */
        let mut draw: i32 = 1;
        while dx > ONE_PIXEL / 4 {
            dx >>= 2;
            draw <<= 1;
        }

        /* We use decrement counter to count the total number of segments */
        /* to draw starting from 2^level. Before each draw we split as    */
        /* many times as there are trailing zeros in the counter.         */
        loop {
            let mut split: i32 = 1;
            while (draw & split) == 0 {
                if arc + 4 >= bez_stack.len() {
                    /* Note (upstream): C overflows its stack of arcs here,
                    for deviations beyond 32 bits (huge coordinates); here it
                    stops */
                    return Ok(());
                }
                gray_split_conic(&mut bez_stack[arc..]);
                arc += 2;
                split = split.wrapping_shl(1);
                if split == 0 {
                    break;
                }
            }

            self.gray_render_line(bez_stack[arc].x, bez_stack[arc].y)?;
            arc = arc.wrapping_sub(2);

            draw = draw.wrapping_sub(1);
            if draw == 0 {
                break;
            }
        }
        Ok(())
    }

    /// `gray_render_cubic`
    fn gray_render_cubic(
        &mut self,
        control1: &PvgFtVector,
        control2: &PvgFtVector,
        to: &PvgFtVector,
    ) -> Result<(), PoolOverflow> {
        let mut bez_stack = [PvgFtVector::default(); 16 * 3 + 1]; /* enough to accommodate bisections */
        let mut arc = 0usize;
        let limit = 45usize;

        bez_stack[0].x = upscale(to.x);
        bez_stack[0].y = upscale(to.y);
        bez_stack[1].x = upscale(control2.x);
        bez_stack[1].y = upscale(control2.y);
        bez_stack[2].x = upscale(control1.x);
        bez_stack[2].y = upscale(control1.y);
        bez_stack[3].x = self.x;
        bez_stack[3].y = self.y;

        /* short-cut the arc that crosses the current band */
        if (trunc(bez_stack[0].y) >= self.max_ey
            && trunc(bez_stack[1].y) >= self.max_ey
            && trunc(bez_stack[2].y) >= self.max_ey
            && trunc(bez_stack[3].y) >= self.max_ey)
            || (trunc(bez_stack[0].y) < self.min_ey
                && trunc(bez_stack[1].y) < self.min_ey
                && trunc(bez_stack[2].y) < self.min_ey
                && trunc(bez_stack[3].y) < self.min_ey)
        {
            self.x = bez_stack[0].x;
            self.y = bez_stack[0].y;
            return Ok(());
        }

        loop {
            /* Decide whether to split or draw. See `Rapid Termination          */
            /* Evaluation for Recursive Subdivision of Bezier Curves' by Thomas */
            /* F. Hain, at                                                      */
            /* http://www.cis.southalabama.edu/~hain/general/Publications/Bezier/Camera-ready%20CISST02%202.pdf */

            let a = &bez_stack[arc..];
            let split = 'decide: {
                /* dx and dy are x and y components of the P0-P3 chord vector. */
                let dx = a[3].x.wrapping_sub(a[0].x);
                let dy = a[3].y.wrapping_sub(a[0].y);
                let mut dx_ = dx;
                let mut dy_ = dy;

                let l = pvg_ft_hypot(&mut dx_, &mut dy_);

                /* Avoid possible arithmetic overflow below by splitting. */
                if l >= (1 << 23) {
                    break 'decide true;
                }

                /* Max deviation may be as much as (s/L) * 3/4 (if Hain's v = 1). */
                let s_limit = l.wrapping_mul(ONE_PIXEL / 6);

                /* s is L * the perpendicular distance from P1 to the line P0-P3. */
                let dx1 = a[1].x.wrapping_sub(a[0].x);
                let dy1 = a[1].y.wrapping_sub(a[0].y);
                let s = pvg_ft_abs(dy.wrapping_mul(dx1).wrapping_sub(dx.wrapping_mul(dy1)));

                if s > s_limit {
                    break 'decide true;
                }

                /* s is L * the perpendicular distance from P2 to the line P0-P3. */
                let dx2 = a[2].x.wrapping_sub(a[0].x);
                let dy2 = a[2].y.wrapping_sub(a[0].y);
                let s = pvg_ft_abs(dy.wrapping_mul(dx2).wrapping_sub(dx.wrapping_mul(dy2)));

                if s > s_limit {
                    break 'decide true;
                }

                /* Split super curvy segments where the off points are so far
                from the chord that the angles P0-P1-P3 or P0-P2-P3 become
                acute as detected by appropriate dot products. */
                if dx1
                    .wrapping_mul(dx1.wrapping_sub(dx))
                    .wrapping_add(dy1.wrapping_mul(dy1.wrapping_sub(dy)))
                    > 0
                    || dx2
                        .wrapping_mul(dx2.wrapping_sub(dx))
                        .wrapping_add(dy2.wrapping_mul(dy2.wrapping_sub(dy)))
                        > 0
                {
                    break 'decide true;
                }
                false
            };

            if !split {
                self.gray_render_line(bez_stack[arc].x, bez_stack[arc].y)?;

                if arc == 0 {
                    return Ok(());
                }

                arc -= 3;
                continue;
            }

            /* Split: */
            if arc == limit {
                return Ok(());
            }
            gray_split_cubic(&mut bez_stack[arc..]);
            arc += 3;
        }
    }

    /// `gray_move_to`
    fn gray_move_to(&mut self, to: &PvgFtVector) -> Result<i32, PoolOverflow> {
        /* record current cell, if any */
        if !self.invalid {
            self.gray_record_cell()?;
        }

        /* start to a new position */
        let x = upscale(to.x);
        let y = upscale(to.y);

        self.gray_start_cell(trunc(x), trunc(y))?;

        self.x = x;
        self.y = y;
        Ok(0)
    }

    /// `gray_hline`
    fn gray_hline(&mut self, mut x: TCoord, mut y: TCoord, area: TPos, acount: i32) {
        /* compute the coverage line's coverage, depending on the    */
        /* outline fill rule                                         */
        /*                                                           */
        /* the coverage percentage is area/(PIXEL_BITS*PIXEL_BITS*2) */
        /*                                                           */
        let mut coverage = (area >> (PIXEL_BITS * 2 + 1 - 8)) as i32;
        /* use range 0..256 */
        if coverage < 0 {
            coverage = coverage.wrapping_neg();
        }

        if self.outline.flags & PVG_FT_OUTLINE_EVEN_ODD_FILL != 0 {
            coverage &= 511;

            if coverage > 256 {
                coverage = 512 - coverage;
            } else if coverage == 256 {
                coverage = 255;
            }
        } else {
            /* normal non-zero winding rule */
            if coverage >= 256 {
                coverage = 255;
            }
        }

        y = y.wrapping_add(self.min_ey);
        x = x.wrapping_add(self.min_ex);

        /* PVG_FT_Span.x is an int, so limit our coordinates appropriately */
        if x >= (1 << 23) {
            x = (1 << 23) - 1;
        }

        /* PVG_FT_Span.y is an int, so limit our coordinates appropriately */
        if y >= (1 << 23) {
            y = (1 << 23) - 1;
        }

        if coverage != 0 {
            /* see whether we can add this span to the current list */
            let count = self.num_gray_spans;
            if count > 0 {
                let span = &mut self.gray_spans[(count - 1) as usize];
                if span.y as i64 == y
                    && span.x.wrapping_add(span.len) as i64 == x
                    && span.coverage as i32 == coverage
                {
                    span.len = span.len.wrapping_add(acount);
                    return;
                }
            }

            let index;
            if count as usize >= PVG_FT_MAX_GRAY_SPANS {
                if count > self.skip_spans {
                    let skip = if self.skip_spans > 0 {
                        self.skip_spans
                    } else {
                        0
                    };
                    if let Some(render_span) = self.render_span.as_mut() {
                        render_span(&self.gray_spans[skip as usize..self.num_gray_spans as usize]);
                    }
                }

                self.skip_spans -= self.num_gray_spans;
                /* ras.render_span( span->y, ras.gray_spans, count ); */
                self.num_gray_spans = 0;

                index = 0;
            } else {
                index = count as usize;
            }

            /* add a gray span to the current list */
            let span = &mut self.gray_spans[index];
            span.x = x as i32;
            span.len = acount;
            span.y = y as i32;
            span.coverage = coverage as u8;

            self.num_gray_spans += 1;
        }
    }

    /// `gray_sweep`
    fn gray_sweep(&mut self) {
        if self.cells.is_empty() {
            return;
        }

        for yindex in 0..self.ycount {
            let mut cell = self.ycells.get(yindex as usize).copied().flatten();
            let mut cover: TCoord = 0;
            let mut x: TCoord = 0;

            while let Some(c) = cell {
                let cc = self.cells[c as usize];

                if cc.x as i64 > x && cover != 0 {
                    self.gray_hline(
                        x,
                        yindex,
                        cover.wrapping_mul(ONE_PIXEL * 2),
                        (cc.x as i64 - x) as i32,
                    );
                }

                cover = cover.wrapping_add(cc.cover as i64);
                let area = cover.wrapping_mul(ONE_PIXEL * 2).wrapping_sub(cc.area);

                if area != 0 && cc.x >= 0 {
                    self.gray_hline(cc.x as i64, yindex, area, 1);
                }

                x = cc.x as i64 + 1;
                cell = cc.next;
            }

            if self.count_ex > x && cover != 0 {
                self.gray_hline(
                    x,
                    yindex,
                    cover.wrapping_mul(ONE_PIXEL * 2),
                    (self.count_ex - x) as i32,
                );
            }
        }
    }
}

/// `gray_split_conic`
fn gray_split_conic(base: &mut [PvgFtVector]) {
    base[4].x = base[2].x;
    let mut b = base[1].x;
    base[3].x = (base[2].x.wrapping_add(b)) / 2;
    let a = base[3].x;
    base[1].x = (base[0].x.wrapping_add(b)) / 2;
    b = base[1].x;
    base[2].x = (a.wrapping_add(b)) / 2;

    base[4].y = base[2].y;
    let mut b = base[1].y;
    base[3].y = (base[2].y.wrapping_add(b)) / 2;
    let a = base[3].y;
    base[1].y = (base[0].y.wrapping_add(b)) / 2;
    b = base[1].y;
    base[2].y = (a.wrapping_add(b)) / 2;
}

/// `gray_split_cubic`
fn gray_split_cubic(base: &mut [PvgFtVector]) {
    base[6].x = base[3].x;
    let mut c = base[1].x;
    let d = base[2].x;
    let mut a = (base[0].x.wrapping_add(c)) / 2;
    base[1].x = a;
    let mut b = (base[3].x.wrapping_add(d)) / 2;
    base[5].x = b;
    c = (c.wrapping_add(d)) / 2;
    a = (a.wrapping_add(c)) / 2;
    base[2].x = a;
    b = (b.wrapping_add(c)) / 2;
    base[4].x = b;
    base[3].x = (a.wrapping_add(b)) / 2;

    base[6].y = base[3].y;
    let mut c = base[1].y;
    let d = base[2].y;
    let mut a = (base[0].y.wrapping_add(c)) / 2;
    base[1].y = a;
    let mut b = (base[3].y.wrapping_add(d)) / 2;
    base[5].y = b;
    c = (c.wrapping_add(d)) / 2;
    a = (a.wrapping_add(c)) / 2;
    base[2].y = a;
    b = (b.wrapping_add(c)) / 2;
    base[4].y = b;
    base[3].y = (a.wrapping_add(b)) / 2;
}

/// `PVG_FT_Outline_Check`
pub fn pvg_ft_outline_check(outline: Option<&PvgFtOutline>) -> PvgFtError {
    if let Some(outline) = outline {
        let n_points = outline.n_points;
        let n_contours = outline.n_contours;

        /* empty glyph? */
        if n_points == 0 && n_contours == 0 {
            return 0;
        }

        /* check point and contour counts */
        if n_points <= 0 || n_contours <= 0 {
            return ERR_RASTER_INVALID_OUTLINE;
        }

        let mut end0 = -1;
        let mut end = -1;
        for n in 0..n_contours as usize {
            end = outline.contours.get(n).copied().unwrap_or(-1);

            /* note that we don't accept empty contours */
            if end <= end0 || end >= n_points {
                return ERR_RASTER_INVALID_OUTLINE;
            }

            end0 = end;
        }

        if end != n_points - 1 {
            return ERR_RASTER_INVALID_OUTLINE;
        }

        /* XXX: check the tags array */
        return 0;
    }

    ERR_RASTER_INVALID_OUTLINE
}

/// `PVG_FT_Outline_Get_CBox`
pub fn pvg_ft_outline_get_cbox(outline: &PvgFtOutline) -> PvgFtBBox {
    let (x_min, y_min, x_max, y_max);

    if outline.n_points == 0 {
        x_min = 0;
        y_min = 0;
        x_max = 0;
        y_max = 0;
    } else {
        let points = &outline.points[..outline.n_points as usize];
        let (mut xmin, mut xmax) = (points[0].x, points[0].x);
        let (mut ymin, mut ymax) = (points[0].y, points[0].y);

        for vec in &points[1..] {
            let x = vec.x;
            if x < xmin {
                xmin = x;
            }
            if x > xmax {
                xmax = x;
            }

            let y = vec.y;
            if y < ymin {
                ymin = y;
            }
            if y > ymax {
                ymax = y;
            }
        }
        x_min = xmin;
        y_min = ymin;
        x_max = xmax;
        y_max = ymax;
    }
    PvgFtBBox {
        x_min,
        x_max,
        y_min,
        y_max,
    }
}

/// The outcome of `PVG_FT_Outline_Decompose`.
enum DecomposeError {
    Invalid,
    Overflow,
}

impl From<PoolOverflow> for DecomposeError {
    fn from(_: PoolOverflow) -> Self {
        DecomposeError::Overflow
    }
}

/// `PVG_FT_Outline_Decompose`: walks over an outline's structure to
/// decompose it into individual segments and Bezier arcs.
fn pvg_ft_outline_decompose(
    outline: &PvgFtOutline,
    user: &mut TWorker<'_, '_>,
) -> Result<(), DecomposeError> {
    let points = &outline.points;
    let tags = &outline.tags;
    let pt = |i: usize| points.get(i).copied().unwrap_or_default();
    let tg = |i: usize| tags.get(i).copied().unwrap_or(0);

    let mut first: i64 = 0;

    for n in 0..outline.n_contours.max(0) as usize {
        let last = outline.contours.get(n).copied().unwrap_or(-1) as i64; /* index of last point in contour */
        if last < 0 {
            return Err(DecomposeError::Invalid);
        }
        let mut limit = last;

        let mut v_start = pt(first as usize);
        let mut v_last = pt(last as usize);

        let mut v_control;

        let mut point = first;
        let mut tagi = first;
        let mut tag = pvg_ft_curve_tag(tg(tagi as usize));

        /* A contour cannot start with a cubic control point! */
        if tag == PVG_FT_CURVE_TAG_CUBIC {
            return Err(DecomposeError::Invalid);
        }

        /* check first point to determine origin */
        if tag == PVG_FT_CURVE_TAG_CONIC {
            /* first point is conic control.  Yes, this happens. */
            if pvg_ft_curve_tag(tg(last as usize)) == PVG_FT_CURVE_TAG_ON {
                /* start at last point if it is on the curve */
                v_start = v_last;
                limit -= 1;
            } else {
                /* if both first and last points are conic,         */
                /* start at their middle and record its position    */
                /* for closure                                      */
                v_start.x = (v_start.x.wrapping_add(v_last.x)) / 2;
                v_start.y = (v_start.y.wrapping_add(v_last.y)) / 2;

                v_last = v_start;
            }
            point -= 1;
            tagi -= 1;
        }
        let _ = v_last;

        user.gray_move_to(&v_start)?;

        let mut closed = false;
        'contour: while point < limit {
            point += 1;
            tagi += 1;

            tag = pvg_ft_curve_tag(tg(tagi as usize));
            match tag {
                PVG_FT_CURVE_TAG_ON => {
                    /* emit a single line_to */
                    let vec = pt(point as usize);

                    user.gray_render_line(upscale(vec.x), upscale(vec.y))?;
                    continue;
                }

                PVG_FT_CURVE_TAG_CONIC => {
                    /* consume conic arcs */
                    v_control = pt(point as usize);

                    /* Do_Conic: */
                    loop {
                        if point < limit {
                            point += 1;
                            tagi += 1;
                            tag = pvg_ft_curve_tag(tg(tagi as usize));

                            let vec = pt(point as usize);

                            if tag == PVG_FT_CURVE_TAG_ON {
                                user.gray_render_conic(&v_control, &vec)?;
                                continue 'contour;
                            }

                            if tag != PVG_FT_CURVE_TAG_CONIC {
                                return Err(DecomposeError::Invalid);
                            }

                            let v_middle = PvgFtVector {
                                x: (v_control.x.wrapping_add(vec.x)) / 2,
                                y: (v_control.y.wrapping_add(vec.y)) / 2,
                            };

                            user.gray_render_conic(&v_control, &v_middle)?;

                            v_control = vec;
                            continue;
                        }

                        user.gray_render_conic(&v_control, &v_start)?;
                        closed = true;
                        break 'contour;
                    }
                }

                _ => {
                    /* PVG_FT_CURVE_TAG_CUBIC */
                    if point + 1 > limit
                        || pvg_ft_curve_tag(tg(tagi as usize + 1)) != PVG_FT_CURVE_TAG_CUBIC
                    {
                        return Err(DecomposeError::Invalid);
                    }

                    point += 2;
                    tagi += 2;

                    let vec1 = pt(point as usize - 2);
                    let vec2 = pt(point as usize - 1);

                    if point <= limit {
                        let vec = pt(point as usize);

                        user.gray_render_cubic(&vec1, &vec2, &vec)?;
                        continue;
                    }

                    user.gray_render_cubic(&vec1, &vec2, &v_start)?;
                    closed = true;
                    break 'contour;
                }
            }
        }

        if !closed {
            /* close the contour with a line segment */
            user.gray_render_line(upscale(v_start.x), upscale(v_start.y))?;
        }

        /* Close: */
        first = last + 1;
    }

    Ok(())
}

/// `TBand`
#[derive(Clone, Copy, Default)]
struct TBand {
    min: TPos,
    max: TPos,
}

impl TWorker<'_, '_> {
    /// `gray_convert_glyph_inner`
    fn gray_convert_glyph_inner(&mut self) -> i32 {
        let outline = self.outline;
        match pvg_ft_outline_decompose(outline, self) {
            Ok(()) => {
                if !self.invalid && self.gray_record_cell().is_err() {
                    return ERR_RASTER_MEMORY_OVERFLOW;
                }
                0
            }
            Err(DecomposeError::Invalid) => ERR_RASTER_INVALID_OUTLINE,
            Err(DecomposeError::Overflow) => ERR_RASTER_MEMORY_OVERFLOW,
        }
    }

    /// `gray_convert_glyph`
    fn gray_convert_glyph(&mut self) -> i32 {
        let mut bands = [TBand::default(); 40];

        self.num_gray_spans = 0;

        /* Set up state in the raster object */
        self.gray_compute_cbox();

        /* clip to target bitmap, exit if nothing to do */
        let clip = self.clip_box;

        if self.max_ex <= clip.x_min
            || self.min_ex >= clip.x_max
            || self.max_ey <= clip.y_min
            || self.min_ey >= clip.y_max
        {
            return 0;
        }

        if self.min_ex < clip.x_min {
            self.min_ex = clip.x_min;
        }
        if self.min_ey < clip.y_min {
            self.min_ey = clip.y_min;
        }

        if self.max_ex > clip.x_max {
            self.max_ex = clip.x_max;
        }
        if self.max_ey > clip.y_max {
            self.max_ey = clip.y_max;
        }

        self.count_ex = self.max_ex - self.min_ex;
        self.count_ey = self.max_ey - self.min_ey;

        /* set up vertical bands */
        let mut num_bands = ((self.max_ey - self.min_ey) / self.band_size as i64) as i32;
        if num_bands == 0 {
            num_bands = 1;
        }
        if num_bands >= 39 {
            num_bands = 39;
        }

        self.band_shoot = 0;

        let mut min = self.min_ey;
        let max_y = self.max_ey;

        let mut n = 0;
        while n < num_bands {
            let mut max = min + self.band_size as i64;
            if n == num_bands - 1 || max > max_y {
                max = max_y;
            }

            bands[0].min = min;
            bands[0].max = max;
            let mut band: isize = 0;

            while band >= 0 {
                let b = bands[band as usize];
                let mut reduce = false;

                {
                    self.ycount = b.max - b.min;

                    let mut cell_start = SIZEOF_PCELL * self.ycount;
                    let cell_mod = cell_start % SIZEOF_TCELL;
                    if cell_mod > 0 {
                        cell_start += SIZEOF_TCELL - cell_mod;
                    }

                    let mut cell_end = self.buffer_size;
                    cell_end -= cell_end % SIZEOF_TCELL;

                    if cell_start >= cell_end {
                        reduce = true;
                    } else {
                        self.max_cells = (cell_end - cell_start) / SIZEOF_TCELL;
                        if self.max_cells < 2 {
                            reduce = true;
                        } else {
                            self.ycells.clear();
                            self.ycells.resize(self.ycount as usize, None);
                        }
                    }
                }

                if !reduce {
                    self.cells.clear();
                    self.invalid = true;
                    self.min_ey = b.min;
                    self.max_ey = b.max;
                    self.count_ey = b.max - b.min;

                    let error = self.gray_convert_glyph_inner();

                    if error == 0 {
                        self.gray_sweep();
                        band -= 1;
                        continue;
                    } else if error != ERR_RASTER_MEMORY_OVERFLOW {
                        return 1;
                    }
                }

                /* ReduceBands: */
                /* render pool overflow; we will reduce the render band by half */
                let bottom = b.min;
                let top = b.max;
                let middle = bottom + ((top - bottom) >> 1);

                /* This is too complex for a single scanline; there must */
                /* be some problems.                                     */
                if middle == bottom {
                    return ERR_RASTER_OUT_OF_MEMORY;
                }

                if bottom - top >= self.band_size as i64 {
                    self.band_shoot += 1;
                }

                if band as usize + 1 >= bands.len() {
                    /* (never: bands are at most 2^24 lines high) */
                    return ERR_RASTER_OUT_OF_MEMORY;
                }

                bands[band as usize + 1].min = bottom;
                bands[band as usize + 1].max = middle;
                bands[band as usize].min = middle;
                bands[band as usize].max = top;
                band += 1;
            }

            n += 1;
            min = max;
        }

        if self.num_gray_spans > self.skip_spans {
            let skip = if self.skip_spans > 0 {
                self.skip_spans
            } else {
                0
            };
            if let Some(render_span) = self.render_span.as_mut() {
                render_span(&self.gray_spans[skip as usize..self.num_gray_spans as usize]);
            }
        }

        self.skip_spans -= self.num_gray_spans;

        if self.band_shoot > 8 && self.band_size > 16 {
            self.band_size /= 2;
        }

        0
    }
}

/// `gray_raster_render`
fn gray_raster_render(
    worker: &mut TWorker<'_, '_>,
    buffer_size: i64,
    params: &PvgFtRasterParams<'_>,
) -> i32 {
    let outline = worker.outline;

    /* return immediately if the outline is empty */
    if outline.n_points == 0 || outline.n_contours <= 0 {
        return 0;
    }

    let last = outline
        .contours
        .get(outline.n_contours as usize - 1)
        .copied();
    if last.is_none() || outline.points.is_empty() {
        return ERR_RASTER_INVALID_OUTLINE;
    }

    if Some(outline.n_points) != last.map(|l| l.wrapping_add(1)) {
        return ERR_RASTER_INVALID_OUTLINE;
    }

    /* this version does not support monochrome rendering */
    if params.flags & PVG_FT_RASTER_FLAG_AA == 0 {
        return ERR_RASTER_INVALID_MODE;
    }

    if params.flags & PVG_FT_RASTER_FLAG_DIRECT == 0 {
        return ERR_RASTER_INVALID_MODE;
    }

    /* compute clipping box */
    if params.flags & PVG_FT_RASTER_FLAG_CLIP != 0 {
        worker.clip_box = params.clip_box;
    } else {
        worker.clip_box.x_min = -(1 << 23);
        worker.clip_box.y_min = -(1 << 23);
        worker.clip_box.x_max = (1 << 23) - 1;
        worker.clip_box.y_max = (1 << 23) - 1;
    }

    worker.gray_init_cells(buffer_size);

    worker.cells.clear();
    worker.invalid = true;
    worker.band_size = (buffer_size / (SIZEOF_TCELL * 8)) as i32;

    worker.gray_convert_glyph()
}

/// `PVG_FT_Raster_Render`
pub fn pvg_ft_raster_render(params: &mut PvgFtRasterParams<'_>) {
    let Some(outline) = params.source else {
        /* ErrRaster_Invalid_Outline */
        return;
    };
    let mut length = PVG_FT_MINIMUM_POOL_SIZE;

    let mut worker = TWorker {
        ex: 0,
        ey: 0,
        min_ex: 0,
        max_ex: 0,
        min_ey: 0,
        max_ey: 0,
        count_ex: 0,
        count_ey: 0,
        area: 0,
        cover: 0,
        invalid: true,
        cells: Vec::new(),
        max_cells: 0,
        x: 0,
        y: 0,
        outline,
        clip_box: PvgFtBBox::default(),
        gray_spans: [PvgFtSpan::default(); PVG_FT_MAX_GRAY_SPANS],
        num_gray_spans: 0,
        skip_spans: 0,
        render_span: params.gray_spans.take(),
        band_size: 0,
        band_shoot: 0,
        buffer_size: 0,
        ycells: Vec::new(),
        ycount: 0,
    };
    let mut rendered_spans: i32 = 0;
    let mut error = gray_raster_render(&mut worker, length, params);
    while error == ERR_RASTER_OUT_OF_MEMORY {
        if worker.skip_spans < 0 {
            rendered_spans += -worker.skip_spans;
        }
        worker.skip_spans = rendered_spans;
        length = length.saturating_mul(2);
        /* (C mallocs the pool; the cells are allocated as they are used,
        stopping where the allocation fails) */
        if length > (1i64 << 40) {
            break;
        }
        error = gray_raster_render(&mut worker, length, params);
    }
    params.gray_spans = worker.render_span.take();
}
