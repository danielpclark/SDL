// Rust translation of src/raster/ftraster.c (and ftraster.h) from FreeType
// (2.13.2, as SDL_ttf's external/freetype pins it).
// Copyright (C) 1996-2023 by David Turner, Robert Wilhelm, and Werner Lemberg.
// This is an altered (translated) version of the original software; it is
// used under the FreeType License (see FTL.TXT and LICENSE.txt).

//! The FreeType glyph rasterizer (body).
//!
//! This is a rewrite of the FreeType 1.x scan-line converter
//!
//! A simple technical note on how the raster works
//! -----------------------------------------------
//!
//!   Converting an outline into a bitmap is achieved in several steps:
//!
//!   1 - Decomposing the outline into successive `profiles'.  Each
//!       profile is simply an array of scanline intersections on a given
//!       dimension.  A profile's main attributes are
//!
//!       o its scanline position boundaries, i.e. `Ymin' and `Ymax'
//!
//!       o an array of intersection coordinates for each scanline
//!         between `Ymin' and `Ymax'
//!
//!       o a direction, indicating whether it was built going `up' or
//!         `down', as this is very important for filling rules
//!
//!       o its drop-out mode
//!
//!   2 - Sweeping the target map's scanlines in order to compute segment
//!       `spans' which are then filled.  Additionally, this pass
//!       performs drop-out control.
//!
//!   The outline data is parsed during step 1 only.  The profiles are
//!   built from the bottom of the render pool, used as a stack.  The
//!   following graphics shows the profile list under construction:
//!
//! ```text
//!    __________________________________________________________ _ _
//!   |         |                 |         |                 |
//!   | profile | coordinates for | profile | coordinates for |-->
//!   |    1    |  profile 1      |    2    |  profile 2      |-->
//!   |_________|_________________|_________|_________________|__ _ _
//!
//!   ^                                                       ^
//!   |                                                       |
//! start of render pool                                      top
//! ```
//!
//!   The top of the profile stack is kept in the `top' variable.
//!
//!   As you can see, a profile record is pushed on top of the render
//!   pool, which is then followed by its coordinates/intersections.  If
//!   a change of direction is detected in the outline, a new profile is
//!   generated until the end of the outline.
//!
//!   Note that when all profiles have been generated, the function
//!   Finalize_Profile_Table() is used to record, for each profile, its
//!   bottom-most scanline as well as the scanline above its upmost
//!   boundary.  These positions are called `y-turns' because they (sort
//!   of) correspond to local extrema.  They are stored in a sorted list
//!   built from the top of the render pool as a downwards stack:
//!
//! ```text
//!     _ _ _______________________________________
//!                           |                    |
//!                        <--| sorted list of     |
//!                        <--|  extrema scanlines |
//!     _ _ __________________|____________________|
//!
//!                           ^                    ^
//!                           |                    |
//!                         maxBuff           sizeBuff = end of pool
//! ```
//!
//!   This list is later used during the sweep phase in order to
//!   optimize performance (see technical note on the sweep below).
//!
//!   Of course, the raster detects whether the two stacks collide and
//!   handles the situation properly.
//!
//! Translation notes: the render pool is C's (16384 bytes of `Long`s on a
//! 64-bit target, with each profile record taking `AlignProfileSize` = 8
//! of them), and pool pointers are indices into it. The profile records
//! themselves are kept in a side table indexed by their pool position, so
//! that the pool accounting (and with it sub-banding) is C's. The Bézier
//! stacks have C's sizes; an arc that would need more splits than they
//! hold (C would write past them) fails with a raster overflow.

use super::super::base::ftcalc::{ft_mul_div, ft_mul_div_no_round};
use super::super::ftimage::{FtRasterFuncs, FtRasterParams, FtRasterSource};
use super::super::fttypes::*;

/// `FT_RENDER_POOL_SIZE` (ftoption.h)
const FT_RENDER_POOL_SIZE: usize = 16384;

/// `FMulDiv` means `Fast MulDiv'; it is used in case where `b' is
/// typically a small value and the result of a*b is known to fit into 32
/// bits.  (A zero divisor gives 0; C would trap.)
#[inline]
fn fmul_div(a: Long, b: Long, c: Long) -> Long {
    if c == 0 {
        0
    } else {
        a.wrapping_mul(b).wrapping_div(c)
    }
}

/* On the other hand, SMulDiv means `Slow MulDiv', and is used typically */
/* for clipping computations.  It simply uses the FT_MulDiv() function   */
/* defined in `ftcalc.h'.                                                */
use ft_mul_div as smul_div;
use ft_mul_div_no_round as smul_div_no_round;

const SUCCESS: bool = false;
const FAILURE: bool = true;

const MAX_BEZIER: usize = 32; /* The maximum number of stacked Bezier curves. */
/* Setting this constant to more than 32 is a   */
/* pure waste of space.                         */

const PIXEL_BITS: Int = 6; /* fractional bits of *input* coordinates */

/* ********************************************************************* */
/*                                                                       */
/*   SIMPLE TYPE DECLARATIONS                                            */
/*                                                                       */
/* ********************************************************************* */

type Int = i32;
type Short = i16;
type UShort = u16;
type Long = i64;
type Byte = u8;

/// `TPoint`
#[derive(Debug, Clone, Copy, Default)]
struct TPoint {
    x: Long,
    y: Long,
}

/* values for the `flags' bit field */
const FLOW_UP: UShort = 0x08;
const OVERSHOOT_TOP: UShort = 0x10;
const OVERSHOOT_BOTTOM: UShort = 0x20;

/// `TStates`: States of each line, arc, and profile
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TStates {
    Unknown,
    Ascending,
    Descending,
    #[allow(dead_code)]
    Flat,
}

/// `TProfile` (`PProfile`s are pool positions)
#[derive(Debug, Clone, Copy, Default)]
struct TProfile {
    X: Long,             /* current coordinate during sweep          */
    link: Option<usize>, /* link to next profile (various purposes)  */
    offset: isize,       /* start of profile's data in render pool   */
    flags: UShort,       /* Bit 0-2: drop-out mode                   */
    /* Bit 3: profile orientation (up/down)     */
    /* Bit 4: is top profile?                   */
    /* Bit 5: is bottom profile?                */
    height: Long, /* profile's height in scanlines            */
    start: Long,  /* profile's starting scanline              */
    countL: Int,  /* number of lines to step before this      */
    /* profile becomes drawable                 */
    next: Option<usize>, /* next profile in same contour, used       */
                         /* during drop-out control                  */
}

/// `AlignProfileSize`: `( sizeof ( TProfile ) + sizeof ( Alignment ) - 1 )
/// / sizeof ( Long )` on a 64-bit target (a 64-byte record)
const ALIGN_PROFILE_SIZE: isize = (64 + 8 - 1) / 8;

/* FT_RENDER_POOL_SIZE > 2048 */
const FT_MAX_BLACK_POOL: usize = FT_RENDER_POOL_SIZE / std::mem::size_of::<Long>();

/// The sweep procedures (`Function_Sweep_Init` and friends), selected by
/// the pass.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Sweep {
    Vertical,
    Horizontal,
}

/// `black_TWorker`
struct BlackTWorker<'a> {
    precision_bits: Int, /* precision related variables         */
    precision: Int,
    precision_half: Int,
    precision_scale: Int,
    precision_step: Int,
    precision_jitter: Int,

    /// the render pool (`buff` is position 0, `sizeBuff` its length)
    pool: Vec<Long>,
    /// the profile records, by pool position
    profiles: Vec<TProfile>,
    maxBuff: isize, /* Profiles buffer size                */
    top: isize,     /* Current cursor in buffer            */

    error: FtError,

    numTurns: Int, /* number of Y-turns in outline        */

    dropOutControl: Byte, /* current drop_out control method     */

    bWidth: UShort, /* target bitmap width                 */
    bOrigin: isize, /* target bitmap bottom-left origin    */
    bLine: isize,   /* target bitmap current line          */

    lastX: Long,
    lastY: Long,
    minY: Long,
    maxY: Long,

    num_Profs: UShort, /* current number of profiles          */

    fresh: bool, /* signals a fresh new profile which   */
    /* `start' field must be completed     */
    joint: bool, /* signals that the last arc ended     */
    /* exactly on a scanline.  Allows      */
    /* removal of doublets                 */
    cProfile: usize,         /* current profile                     */
    fProfile: Option<usize>, /* head of linked list of profiles     */
    gProfile: Option<usize>, /* contour's first profile in case     */
    /* of impact                           */
    state: TStates, /* rendering state                     */

    /// `target` (description of target bit/pixmap)
    target_rows: u32,
    target_width: u32,
    target_pitch: isize,
    buffer: &'a mut [u8],

    outline: &'a FtOutline,

    /* dispatch variables */
    sweep: Sweep,
}

impl BlackTWorker<'_> {
    /// `FLOOR`
    #[inline]
    fn floor(&self, x: Long) -> Long {
        x & -(self.precision as Long)
    }

    /// `CEILING`
    #[inline]
    fn ceiling(&self, x: Long) -> Long {
        x.wrapping_add(self.precision as Long - 1) & -(self.precision as Long)
    }

    /// `TRUNC`
    #[inline]
    fn trunc(&self, x: Long) -> Long {
        x >> self.precision_bits
    }

    /// `FRAC`
    #[inline]
    fn frac(&self, x: Long) -> Long {
        x & (self.precision as Long - 1)
    }

    /// `SCALED`: scale and shift grid to pixel centers
    #[inline]
    fn scaled(&self, x: Long) -> Long {
        x.wrapping_mul(self.precision_scale as Long)
            .wrapping_sub(self.precision_half as Long)
    }

    /// `IS_BOTTOM_OVERSHOOT`
    #[inline]
    fn is_bottom_overshoot(&self, x: Long) -> bool {
        self.ceiling(x).wrapping_sub(x) >= self.precision_half as Long
    }

    /// `IS_TOP_OVERSHOOT`
    #[inline]
    fn is_top_overshoot(&self, x: Long) -> bool {
        x.wrapping_sub(self.floor(x)) >= self.precision_half as Long
    }

    /// `SMART`: Smart dropout rounding to find which pixel is closer to
    /// span ends.  To mimick Windows, symmetric cases break down
    /// indepenently of the precision.
    #[inline]
    fn smart(&self, p: Long, q: Long) -> Long {
        self.floor(
            (p.wrapping_add(q)
                .wrapping_add(self.precision as Long * 63 / 64))
                >> 1,
        )
    }

    /// `ras.sizeBuff[i]` for a negative `i`
    #[inline]
    fn size_buff(&self, i: isize) -> Long {
        self.pool[(FT_MAX_BLACK_POOL as isize + i) as usize]
    }

    /// The profile at a pool position.
    #[inline]
    fn prof(&mut self, p: usize) -> &mut TProfile {
        &mut self.profiles[p]
    }

    /* ********************************************************************* */
    /*                                                                       */
    /*   PROFILES COMPUTATION                                                */
    /*                                                                       */
    /* ********************************************************************* */

    /// `Set_High_Precision`: Set precision variables according to param
    /// flag.
    ///
    /// `High`: Set to True for high precision (typically for ppem < 24),
    /// false otherwise.
    fn set_high_precision(&mut self, high: bool) {
        /*
         * `precision_step' is used in `Bezier_Up' to decide when to split a
         * given y-monotonous Bezier arc that crosses a scanline before
         * approximating it as a straight segment.  The default value of 32 (for
         * low accuracy) corresponds to
         *
         *   32 / 64 == 0.5 pixels,
         *
         * while for the high accuracy case we have
         *
         *   256 / (1 << 12) = 0.0625 pixels.
         *
         * `precision_jitter' is an epsilon threshold used in
         * `Vertical_Sweep_Span' to deal with small imperfections in the Bezier
         * decomposition (after all, we are working with approximations only);
         * it avoids switching on additional pixels which would cause artifacts
         * otherwise.
         *
         * The value of `precision_jitter' has been determined heuristically.
         *
         */

        if high {
            self.precision_bits = 12;
            self.precision_step = 256;
            self.precision_jitter = 30;
        } else {
            self.precision_bits = 6;
            self.precision_step = 32;
            self.precision_jitter = 2;
        }

        self.precision = 1 << self.precision_bits;
        self.precision_half = self.precision >> 1;
        self.precision_scale = self.precision >> PIXEL_BITS;
    }

    /// `New_Profile`: Create a new profile in the render pool.
    ///
    /// `aState`: The state/orientation of the new profile.
    ///
    /// `overshoot`: Whether the profile's unrounded start position differs
    /// by at least a half pixel.
    ///
    /// Returns SUCCESS on success.  FAILURE in case of overflow or of
    /// incoherent profile.
    fn new_profile(&mut self, a_state: TStates, overshoot: bool) -> bool {
        if self.fProfile.is_none() {
            self.cProfile = self.top as usize;
            self.fProfile = Some(self.cProfile);
            self.top += ALIGN_PROFILE_SIZE;
        }

        if self.top >= self.maxBuff {
            self.error = FT_ERR_RASTER_OVERFLOW;
            return FAILURE;
        }

        let top = self.top;
        let drop_out_control = self.dropOutControl as UShort;
        let c = self.cProfile;
        {
            let p = self.prof(c);
            p.start = 0;
            p.height = 0;
            p.offset = top;
            p.link = None;
            p.next = None;
            p.flags = drop_out_control;
        }

        match a_state {
            TStates::Ascending => {
                self.prof(c).flags |= FLOW_UP;
                if overshoot {
                    self.prof(c).flags |= OVERSHOOT_BOTTOM;
                }
            }

            TStates::Descending => {
                if overshoot {
                    self.prof(c).flags |= OVERSHOOT_TOP;
                }
            }

            _ => {
                self.error = FT_ERR_INVALID_OUTLINE;
                return FAILURE;
            }
        }

        if self.gProfile.is_none() {
            self.gProfile = Some(c);
        }

        self.state = a_state;
        self.fresh = true;
        self.joint = false;

        SUCCESS
    }

    /// `End_Profile`: Finalize the current profile.
    ///
    /// `overshoot`: Whether the profile's unrounded end position differs
    /// by at least a half pixel.
    ///
    /// Returns SUCCESS on success.  FAILURE in case of overflow or
    /// incoherency.
    fn end_profile(&mut self, overshoot: bool) -> bool {
        let c = self.cProfile;
        let h = (self.top - self.profiles[c].offset) as Long;

        if h < 0 {
            self.error = FT_ERR_RASTER_NEGATIVE_HEIGHT;
            return FAILURE;
        }

        if h > 0 {
            self.prof(c).height = h;

            if overshoot {
                if self.profiles[c].flags & FLOW_UP != 0 {
                    self.prof(c).flags |= OVERSHOOT_TOP;
                } else {
                    self.prof(c).flags |= OVERSHOOT_BOTTOM;
                }
            }

            let old_profile = c;
            self.cProfile = self.top as usize;

            self.top += ALIGN_PROFILE_SIZE;

            let top = self.top;
            let nc = self.cProfile;
            self.prof(nc).height = 0;
            self.prof(nc).offset = top;

            self.prof(old_profile).next = Some(nc);
            self.num_Profs = self.num_Profs.wrapping_add(1);
        }

        if self.top >= self.maxBuff {
            self.error = FT_ERR_RASTER_OVERFLOW;
            return FAILURE;
        }

        self.joint = false;

        SUCCESS
    }

    /// `Insert_Y_Turn`: Insert a salient into the sorted list placed on
    /// top of the render pool.
    ///
    /// Returns SUCCESS on success.  FAILURE in case of overflow.
    fn insert_y_turn(&mut self, y: Int) -> bool {
        let mut y = y;
        let mut n = self.numTurns - 1;
        let y_turns = FT_MAX_BLACK_POOL as isize - self.numTurns as isize;

        /* look for first y value that is <= */
        while n >= 0 && (y as Long) < self.pool[(y_turns + n as isize) as usize] {
            n -= 1;
        }

        /* if it is <, simply insert it, ignore if == */
        if n >= 0 && y as Long > self.pool[(y_turns + n as isize) as usize] {
            loop {
                let i = (y_turns + n as isize) as usize;
                let y2 = self.pool[i] as Int;

                self.pool[i] = y as Long;
                y = y2;

                n -= 1;
                if n < 0 {
                    break;
                }
            }
        }

        if n < 0 {
            self.maxBuff -= 1;
            if self.maxBuff <= self.top {
                self.error = FT_ERR_RASTER_OVERFLOW;
                return FAILURE;
            }
            self.numTurns += 1;
            self.pool[FT_MAX_BLACK_POOL - self.numTurns as usize] = y as Long;
        }

        SUCCESS
    }

    /// `Finalize_Profile_Table`: Adjust all links in the profiles list.
    ///
    /// Returns SUCCESS on success.  FAILURE in case of overflow.
    fn finalize_profile_table(&mut self) -> bool {
        let mut n = self.num_Profs;
        let mut p = self.fProfile;

        if n > 1 && p.is_some() {
            loop {
                let pi = p.unwrap();
                let (bottom, top);

                if n > 1 {
                    let pr = self.profiles[pi];
                    self.prof(pi).link = Some((pr.offset + pr.height as isize) as usize);
                } else {
                    self.prof(pi).link = None;
                }

                let pr = self.profiles[pi];
                if pr.flags & FLOW_UP != 0 {
                    bottom = pr.start as Int;
                    top = (pr.start + pr.height - 1) as Int;
                } else {
                    bottom = (pr.start - pr.height + 1) as Int;
                    top = pr.start as Int;
                    self.prof(pi).start = bottom as Long;
                    self.prof(pi).offset += pr.height as isize - 1;
                }

                if self.insert_y_turn(bottom) || self.insert_y_turn(top.wrapping_add(1)) {
                    return FAILURE;
                }

                p = self.profiles[pi].link;

                n -= 1;
                if n == 0 {
                    break;
                }
            }
        } else {
            self.fProfile = None;
        }

        SUCCESS
    }

    /// `Line_Up`: Compute the x-coordinates of an ascending line segment
    /// and store them in the render pool.
    ///
    /// `miny`/`maxy`: A lower/upper vertical clipping bound value.
    ///
    /// Returns SUCCESS on success, FAILURE on render pool overflow.
    fn line_up(&mut self, x1: Long, y1: Long, x2: Long, y2: Long, miny: Long, maxy: Long) -> bool {
        let mut x1 = x1;
        let mut dx = x2.wrapping_sub(x1);
        let dy = y2.wrapping_sub(y1);

        if dy <= 0 || y2 < miny || y1 > maxy {
            return SUCCESS;
        }

        let (mut e1, f1): (Int, Int);
        if y1 < miny {
            /* Take care: miny-y1 can be a very large value; we use     */
            /*            a slow MulDiv function to avoid clipping bugs */
            x1 = x1.wrapping_add(smul_div(dx, miny - y1, dy));
            e1 = self.trunc(miny) as Int;
            f1 = 0;
        } else {
            e1 = self.trunc(y1) as Int;
            f1 = self.frac(y1) as Int;
        }

        let (e2, f2): (Int, Int);
        if y2 > maxy {
            /* x2 += FMulDiv( Dx, maxy - y2, Dy );  UNNECESSARY */
            e2 = self.trunc(maxy) as Int;
            f2 = 0;
        } else {
            e2 = self.trunc(y2) as Int;
            f2 = self.frac(y2) as Int;
        }

        if f1 > 0 {
            if e1 == e2 {
                return SUCCESS;
            } else {
                x1 = x1.wrapping_add(smul_div(dx, (self.precision - f1) as Long, dy));
                e1 += 1;
            }
        } else if self.joint {
            self.top -= 1;
            self.joint = false;
        }

        self.joint = f2 == 0;

        if self.fresh {
            let c = self.cProfile;
            self.prof(c).start = e1 as Long;
            self.fresh = false;
        }

        let mut size = e2.wrapping_sub(e1).wrapping_add(1);
        if self.top + size as isize >= self.maxBuff {
            self.error = FT_ERR_RASTER_OVERFLOW;
            return FAILURE;
        }

        let (ix, rx);
        if dx > 0 {
            ix = smul_div_no_round(self.precision as Long, dx, dy);
            rx = (self.precision as Long).wrapping_mul(dx) % dy;
            dx = 1;
        } else {
            ix = -smul_div_no_round(self.precision as Long, -dx, dy);
            rx = (self.precision as Long).wrapping_mul(-dx) % dy;
            dx = -1;
        }

        let mut ax = -dy;
        let mut top = self.top;

        while size > 0 {
            self.pool[top as usize] = x1;
            top += 1;

            x1 = x1.wrapping_add(ix);
            ax += rx;
            if ax >= 0 {
                ax -= dy;
                x1 = x1.wrapping_add(dx);
            }
            size -= 1;
        }

        self.top = top;
        SUCCESS
    }

    /// `Line_Down`: Compute the x-coordinates of an descending line
    /// segment and store them in the render pool.
    ///
    /// Returns SUCCESS on success, FAILURE on render pool overflow.
    fn line_down(
        &mut self,
        x1: Long,
        y1: Long,
        x2: Long,
        y2: Long,
        miny: Long,
        maxy: Long,
    ) -> bool {
        let fresh = self.fresh;

        let result = self.line_up(
            x1,
            y1.wrapping_neg(),
            x2,
            y2.wrapping_neg(),
            maxy.wrapping_neg(),
            miny.wrapping_neg(),
        );

        if fresh && !self.fresh {
            let c = self.cProfile;
            self.prof(c).start = -self.profiles[c].start;
        }

        result
    }

    /// `Bezier_Up`: Compute the x-coordinates of an ascending Bezier arc
    /// and store them in the render pool.
    ///
    /// `degree`: The degree of the Bezier arc (either 2 or 3).
    ///
    /// `splitter`: The function to split Bezier arcs.
    ///
    /// Returns SUCCESS on success, FAILURE on render pool overflow.
    fn bezier_up(
        &mut self,
        degree: usize,
        arcs: &mut [TPoint],
        arc: usize,
        splitter: fn(&mut [TPoint]),
        miny: Long,
        maxy: Long,
    ) -> bool {
        let mut arc = arc as isize;
        let mut y1 = arcs[arc as usize + degree].y;
        let mut y2 = arcs[arc as usize].y;
        let mut top = self.top;

        'fin: {
            if y2 < miny || y1 > maxy {
                break 'fin;
            }

            let mut e2 = self.floor(y2);

            if e2 > maxy {
                e2 = maxy;
            }

            let mut e0 = miny;
            let mut e;

            if y1 < miny {
                e = miny;
            } else {
                e = self.ceiling(y1);
                let f1 = self.frac(y1) as Short;
                e0 = e;

                if f1 == 0 {
                    if self.joint {
                        top -= 1;
                        self.joint = false;
                    }

                    self.pool[top as usize] = arcs[arc as usize + degree].x;
                    top += 1;

                    e += self.precision as Long;
                }
            }

            if self.fresh {
                let c = self.cProfile;
                self.prof(c).start = self.trunc(e0);
                self.fresh = false;
            }

            if e2 < e {
                break 'fin;
            }

            if top + self.trunc(e2 - e) as isize + 1 >= self.maxBuff {
                self.top = top;
                self.error = FT_ERR_RASTER_OVERFLOW;
                return FAILURE;
            }

            let start_arc = arc;

            loop {
                self.joint = false;

                let a = arc as usize;
                y2 = arcs[a].y;

                if y2 > e {
                    y1 = arcs[a + degree].y;
                    if y2 - y1 >= self.precision_step as Long {
                        /* (see the module documentation) */
                        if a + 2 * degree >= arcs.len() {
                            self.top = top;
                            self.error = FT_ERR_RASTER_OVERFLOW;
                            return FAILURE;
                        }

                        splitter(&mut arcs[a..]);
                        arc += degree as isize;
                    } else {
                        self.pool[top as usize] = arcs[a + degree].x.wrapping_add(fmul_div(
                            arcs[a].x.wrapping_sub(arcs[a + degree].x),
                            e - y1,
                            y2 - y1,
                        ));
                        top += 1;
                        arc -= degree as isize;
                        e += self.precision as Long;
                    }
                } else {
                    if y2 == e {
                        self.joint = true;
                        self.pool[top as usize] = arcs[a].x;
                        top += 1;

                        e += self.precision as Long;
                    }
                    arc -= degree as isize;
                }

                if !(arc >= start_arc && e <= e2) {
                    break;
                }
            }
        }

        /* Fin: */
        self.top = top;
        SUCCESS
    }

    /// `Bezier_Down`: Compute the x-coordinates of an descending Bezier
    /// arc and store them in the render pool.
    ///
    /// Returns SUCCESS on success, FAILURE on render pool overflow.
    fn bezier_down(
        &mut self,
        degree: usize,
        arcs: &mut [TPoint],
        arc: usize,
        splitter: fn(&mut [TPoint]),
        miny: Long,
        maxy: Long,
    ) -> bool {
        arcs[arc].y = arcs[arc].y.wrapping_neg();
        arcs[arc + 1].y = arcs[arc + 1].y.wrapping_neg();
        arcs[arc + 2].y = arcs[arc + 2].y.wrapping_neg();
        if degree > 2 {
            arcs[arc + 3].y = arcs[arc + 3].y.wrapping_neg();
        }

        let fresh = self.fresh;

        let result = self.bezier_up(
            degree,
            arcs,
            arc,
            splitter,
            maxy.wrapping_neg(),
            miny.wrapping_neg(),
        );

        if fresh && !self.fresh {
            let c = self.cProfile;
            self.prof(c).start = -self.profiles[c].start;
        }

        arcs[arc].y = arcs[arc].y.wrapping_neg();
        result
    }

    /// `Line_To`: Inject a new line segment and adjust the Profiles list.
    ///
    /// `x`/`y`: The coordinates of the segment's end point (its start
    /// point is stored in `lastX`/`lastY').
    ///
    /// Returns SUCCESS on success, FAILURE on render pool overflow or
    /// incorrect profile.
    fn line_to(&mut self, x: Long, y: Long) -> bool {
        /* First, detect a change of direction */

        match self.state {
            TStates::Unknown => {
                if y > self.lastY {
                    let o = self.is_bottom_overshoot(self.lastY);
                    if self.new_profile(TStates::Ascending, o) {
                        return FAILURE;
                    }
                } else if y < self.lastY {
                    let o = self.is_top_overshoot(self.lastY);
                    if self.new_profile(TStates::Descending, o) {
                        return FAILURE;
                    }
                }
            }

            TStates::Ascending => {
                if y < self.lastY {
                    let o = self.is_top_overshoot(self.lastY);
                    if self.end_profile(o) || self.new_profile(TStates::Descending, o) {
                        return FAILURE;
                    }
                }
            }

            TStates::Descending => {
                if y > self.lastY {
                    let o = self.is_bottom_overshoot(self.lastY);
                    if self.end_profile(o) || self.new_profile(TStates::Ascending, o) {
                        return FAILURE;
                    }
                }
            }

            _ => {}
        }

        /* Then compute the lines */

        match self.state {
            TStates::Ascending => {
                if self.line_up(self.lastX, self.lastY, x, y, self.minY, self.maxY) {
                    return FAILURE;
                }
            }

            TStates::Descending => {
                if self.line_down(self.lastX, self.lastY, x, y, self.minY, self.maxY) {
                    return FAILURE;
                }
            }

            _ => {}
        }

        self.lastX = x;
        self.lastY = y;

        SUCCESS
    }

    /// `Conic_To`: Inject a new conic arc and adjust the profile list.
    ///
    /// `cx`/`cy`: The coordinates of the arc's new control point.
    ///
    /// `x`/`y`: The coordinates of the arc's end point (its start point is
    /// stored in `lastX`/`lastY').
    ///
    /// Returns SUCCESS on success, FAILURE on render pool overflow or
    /// incorrect profile.
    fn conic_to(&mut self, cx: Long, cy: Long, x: Long, y: Long) -> bool {
        let mut arcs = [TPoint::default(); 2 * MAX_BEZIER + 1]; /* The Bezier stack           */
        let mut arc: isize = 0; /* current Bezier arc pointer */

        let (mut x3, mut y3) = (0, 0);

        arcs[2].x = self.lastX;
        arcs[2].y = self.lastY;
        arcs[1].x = cx;
        arcs[1].y = cy;
        arcs[0].x = x;
        arcs[0].y = y;

        loop {
            let a = arc as usize;
            let y1 = arcs[a + 2].y;
            let y2 = arcs[a + 1].y;
            y3 = arcs[a].y;
            x3 = arcs[a].x;

            /* first, categorize the Bezier arc */

            let (ymin, ymax) = if y1 <= y3 { (y1, y3) } else { (y3, y1) };

            if y2 < ymin || y2 > ymax {
                /* this arc has no given direction, split it! */
                /* (see the module documentation) */
                if a + 4 >= arcs.len() {
                    self.error = FT_ERR_RASTER_OVERFLOW;
                    return FAILURE;
                }
                split_conic(&mut arcs[a..]);
                arc += 2;
            } else if y1 == y3 {
                /* this arc is flat, ignore it and pop it from the Bezier stack */
                arc -= 2;
            } else {
                /* the arc is y-monotonous, either ascending or descending */
                /* detect a change of direction                            */
                let state_bez = if y1 < y3 {
                    TStates::Ascending
                } else {
                    TStates::Descending
                };
                if self.state != state_bez {
                    let o = if state_bez == TStates::Ascending {
                        self.is_bottom_overshoot(y1)
                    } else {
                        self.is_top_overshoot(y1)
                    };

                    /* finalize current profile if any */
                    if self.state != TStates::Unknown && self.end_profile(o) {
                        return FAILURE;
                    }

                    /* create a new profile */
                    if self.new_profile(state_bez, o) {
                        return FAILURE;
                    }
                }

                /* now call the appropriate routine */
                if state_bez == TStates::Ascending {
                    if self.bezier_up(2, &mut arcs, a, split_conic, self.minY, self.maxY) {
                        return FAILURE;
                    }
                } else if self.bezier_down(2, &mut arcs, a, split_conic, self.minY, self.maxY) {
                    return FAILURE;
                }
                arc -= 2;
            }

            if arc < 0 {
                break;
            }
        }

        self.lastX = x3;
        self.lastY = y3;

        SUCCESS
    }

    /// `Cubic_To`: Inject a new cubic arc and adjust the profile list.
    ///
    /// Returns SUCCESS on success, FAILURE on render pool overflow or
    /// incorrect profile.
    fn cubic_to(&mut self, cx1: Long, cy1: Long, cx2: Long, cy2: Long, x: Long, y: Long) -> bool {
        let mut arcs = [TPoint::default(); 3 * MAX_BEZIER + 1]; /* The Bezier stack           */
        let mut arc: isize = 0; /* current Bezier arc pointer */

        let (mut x4, mut y4) = (0, 0);

        arcs[3].x = self.lastX;
        arcs[3].y = self.lastY;
        arcs[2].x = cx1;
        arcs[2].y = cy1;
        arcs[1].x = cx2;
        arcs[1].y = cy2;
        arcs[0].x = x;
        arcs[0].y = y;

        loop {
            let a = arc as usize;
            let y1 = arcs[a + 3].y;
            let y2 = arcs[a + 2].y;
            let y3 = arcs[a + 1].y;
            y4 = arcs[a].y;
            x4 = arcs[a].x;

            /* first, categorize the Bezier arc */

            let (ymin1, ymax1) = if y1 <= y4 { (y1, y4) } else { (y4, y1) };

            let (ymin2, ymax2) = if y2 <= y3 { (y2, y3) } else { (y3, y2) };

            if ymin2 < ymin1 || ymax2 > ymax1 {
                /* this arc has no given direction, split it! */
                /* (see the module documentation) */
                if a + 6 >= arcs.len() {
                    self.error = FT_ERR_RASTER_OVERFLOW;
                    return FAILURE;
                }
                split_cubic(&mut arcs[a..]);
                arc += 3;
            } else if y1 == y4 {
                /* this arc is flat, ignore it and pop it from the Bezier stack */
                arc -= 3;
            } else {
                let state_bez = if y1 <= y4 {
                    TStates::Ascending
                } else {
                    TStates::Descending
                };

                /* detect a change of direction */
                if self.state != state_bez {
                    let o = if state_bez == TStates::Ascending {
                        self.is_bottom_overshoot(y1)
                    } else {
                        self.is_top_overshoot(y1)
                    };

                    /* finalize current profile if any */
                    if self.state != TStates::Unknown && self.end_profile(o) {
                        return FAILURE;
                    }

                    if self.new_profile(state_bez, o) {
                        return FAILURE;
                    }
                }

                /* compute intersections */
                if state_bez == TStates::Ascending {
                    if self.bezier_up(3, &mut arcs, a, split_cubic, self.minY, self.maxY) {
                        return FAILURE;
                    }
                } else if self.bezier_down(3, &mut arcs, a, split_cubic, self.minY, self.maxY) {
                    return FAILURE;
                }
                arc -= 3;
            }

            if arc < 0 {
                break;
            }
        }

        self.lastX = x4;
        self.lastY = y4;

        SUCCESS
    }

    /// `Decompose_Curve`: Scan the outline arrays in order to emit
    /// individual segments and Beziers by calling Line_To() and
    /// Bezier_To().  It handles all weird cases, like when the first point
    /// is off the curve, or when there are simply no `on' points in the
    /// contour!
    ///
    /// `first`/`last`: The index of the first/last point in the contour.
    ///
    /// `flipped`: If set, flip the direction of the curve.
    ///
    /// Returns SUCCESS on success, FAILURE on error.
    fn decompose_curve(&mut self, first: Int, last: Int, flipped: bool) -> bool {
        let outline = self.outline;
        let points = &outline.points;
        let tags = &outline.tags;

        macro_rules! swap_ {
            ($x:expr, $y:expr) => {
                std::mem::swap(&mut $x, &mut $y)
            };
        }

        let mut limit = last as isize;

        let mut v_start = FtVector {
            x: self.scaled(points[first as usize].x),
            y: self.scaled(points[first as usize].y),
        };
        let mut v_last = FtVector {
            x: self.scaled(points[last as usize].x),
            y: self.scaled(points[last as usize].y),
        };

        if flipped {
            swap_!(v_start.x, v_start.y);
            swap_!(v_last.x, v_last.y);
        }

        let mut v_control = v_start;

        let mut point = first as isize;

        /* set scan mode if necessary */
        if tags[first as usize] & FT_CURVE_TAG_HAS_SCANMODE != 0 {
            self.dropOutControl = tags[first as usize] >> 5;
        }

        let mut tag = ft_curve_tag(tags[first as usize]);

        /* A contour cannot start with a cubic control point! */
        if tag == FT_CURVE_TAG_CUBIC {
            /* Invalid_Outline: */
            self.error = FT_ERR_INVALID_OUTLINE;
            return FAILURE;
        }

        /* check first point to determine origin */
        if tag == FT_CURVE_TAG_CONIC {
            /* first point is conic control.  Yes, this happens. */
            if ft_curve_tag(tags[last as usize]) == FT_CURVE_TAG_ON {
                /* start at last point if it is on the curve */
                v_start = v_last;
                limit -= 1;
            } else {
                /* if both first and last points are conic,         */
                /* start at their middle and record its position    */
                /* for closure                                      */
                v_start.x = (v_start.x + v_last.x) / 2;
                v_start.y = (v_start.y + v_last.y) / 2;

                /* v_last = v_start; */
            }
            point -= 1;
        }

        self.lastX = v_start.x;
        self.lastY = v_start.y;

        let pt = |worker: &Self, i: isize| -> (Long, Long) {
            let p = points[i as usize];
            let (mut x, mut y) = (worker.scaled(p.x), worker.scaled(p.y));
            if flipped {
                swap_!(x, y);
            }
            (x, y)
        };

        while point < limit {
            point += 1;

            tag = ft_curve_tag(tags[point as usize]);

            match tag {
                FT_CURVE_TAG_ON => {
                    /* emit a single line_to */
                    let (x, y) = pt(self, point);

                    if self.line_to(x, y) {
                        return FAILURE;
                    }
                    continue;
                }

                FT_CURVE_TAG_CONIC => {
                    /* consume conic arcs */
                    let (cx, cy) = pt(self, point);
                    v_control.x = cx;
                    v_control.y = cy;

                    /* Do_Conic: */
                    loop {
                        if point < limit {
                            point += 1;
                            tag = ft_curve_tag(tags[point as usize]);

                            let (x, y) = pt(self, point);

                            if tag == FT_CURVE_TAG_ON {
                                if self.conic_to(v_control.x, v_control.y, x, y) {
                                    return FAILURE;
                                }
                                break;
                            }

                            if tag != FT_CURVE_TAG_CONIC {
                                /* Invalid_Outline: */
                                self.error = FT_ERR_INVALID_OUTLINE;
                                return FAILURE;
                            }

                            let v_middle = FtVector {
                                x: (v_control.x + x) / 2,
                                y: (v_control.y + y) / 2,
                            };

                            if self.conic_to(v_control.x, v_control.y, v_middle.x, v_middle.y) {
                                return FAILURE;
                            }

                            v_control.x = x;
                            v_control.y = y;

                            continue;
                        }

                        if self.conic_to(v_control.x, v_control.y, v_start.x, v_start.y) {
                            return FAILURE;
                        }

                        /* Close: */
                        return SUCCESS;
                    }
                    continue;
                }

                _ => {
                    /* FT_CURVE_TAG_CUBIC */
                    if point + 1 > limit
                        || ft_curve_tag(tags[point as usize + 1]) != FT_CURVE_TAG_CUBIC
                    {
                        /* Invalid_Outline: */
                        self.error = FT_ERR_INVALID_OUTLINE;
                        return FAILURE;
                    }

                    point += 2;

                    let (x1, y1) = pt(self, point - 2);
                    let (x2, y2) = pt(self, point - 1);

                    if point <= limit {
                        let (x3, y3) = pt(self, point);

                        if self.cubic_to(x1, y1, x2, y2, x3, y3) {
                            return FAILURE;
                        }
                        continue;
                    }

                    if self.cubic_to(x1, y1, x2, y2, v_start.x, v_start.y) {
                        return FAILURE;
                    }

                    /* Close: */
                    return SUCCESS;
                }
            }
        }

        /* close the contour with a line segment */
        if self.line_to(v_start.x, v_start.y) {
            return FAILURE;
        }

        /* Close: */
        SUCCESS
    }

    /// `Convert_Glyph`: Convert a glyph into a series of segments and arcs
    /// and make a profiles list with them.
    ///
    /// `flipped`: If set, flip the direction of curve.
    ///
    /// Returns SUCCESS on success, FAILURE if any error was encountered
    /// during rendering.
    fn convert_glyph(&mut self, flipped: bool) -> bool {
        self.fProfile = None;
        self.joint = false;
        self.fresh = false;

        self.maxBuff = FT_MAX_BLACK_POOL as isize - ALIGN_PROFILE_SIZE;

        self.numTurns = 0;

        self.cProfile = self.top as usize;
        let top = self.top;
        let c = self.cProfile;
        self.prof(c).offset = top;
        self.num_Profs = 0;

        let mut last: Int = -1;
        for i in 0..self.outline.n_contours as usize {
            self.state = TStates::Unknown;
            self.gProfile = None;

            let first = last + 1;
            last = self.outline.contours[i] as Int;

            if self.decompose_curve(first, last, flipped) {
                return FAILURE;
            }

            /* we must now check whether the extreme arcs join or not */
            if self.frac(self.lastY) == 0 && self.lastY >= self.minY && self.lastY <= self.maxY {
                if let Some(g) = self.gProfile {
                    if (self.profiles[g].flags & FLOW_UP)
                        == (self.profiles[self.cProfile].flags & FLOW_UP)
                    {
                        self.top -= 1;
                    }
                }
            }
            /* Note that ras.gProfile can be nil if the contour was too small */
            /* to be drawn.                                                   */

            let last_profile = self.cProfile;

            let o = if self.top != self.profiles[self.cProfile].offset
                && self.profiles[self.cProfile].flags & FLOW_UP != 0
            {
                self.is_top_overshoot(self.lastY)
            } else {
                self.is_bottom_overshoot(self.lastY)
            };
            if self.end_profile(o) {
                return FAILURE;
            }

            /* close the `next profile in contour' linked list */
            if let Some(g) = self.gProfile {
                self.prof(last_profile).next = Some(g);
            }
        }

        if self.finalize_profile_table() {
            return FAILURE;
        }

        if self.top < self.maxBuff {
            SUCCESS
        } else {
            FAILURE
        }
    }

    /* ********************************************************************* */
    /*                                                                       */
    /*   SCAN-LINE SWEEPS AND DRAWING                                        */
    /*                                                                       */
    /* ********************************************************************* */

    /// `InsNew`: Inserts a new profile in a linked list.
    fn ins_new(&mut self, list: &mut Option<usize>, profile: usize) {
        let x = self.profiles[profile].X;

        /* (`old' is the list head, or the `link' field of `prev') */
        let mut prev: Option<usize> = None;
        let mut current = *list;

        while let Some(c) = current {
            if x < self.profiles[c].X {
                break;
            }
            prev = Some(c);
            current = self.profiles[c].link;
        }

        self.prof(profile).link = current;
        match prev {
            None => *list = Some(profile),
            Some(p) => self.prof(p).link = Some(profile),
        }
    }

    /// `DelOld`: Removes an old profile from a linked list.
    fn del_old(&mut self, list: &mut Option<usize>, profile: usize) {
        let mut prev: Option<usize> = None;
        let mut current = *list;

        while let Some(c) = current {
            if c == profile {
                let next = self.profiles[c].link;
                match prev {
                    None => *list = next,
                    Some(p) => self.prof(p).link = next,
                }
                return;
            }

            prev = Some(c);
            current = self.profiles[c].link;
        }

        /* we should never get there, unless the profile was not part of */
        /* the list.                                                     */
    }

    /// `Sort`: Sorts a trace list.  In 95%, the list is already sorted.
    /// We need an algorithm which is fast in this case.  Bubble sort is
    /// enough and simple.
    fn sort(&mut self, list: &mut Option<usize>) {
        /* First, set the new X coordinate of each profile */
        let mut current = *list;
        while let Some(c) = current {
            let off = self.profiles[c].offset;
            let x = self.pool.get(off as usize).copied().unwrap_or(0);
            let p = self.prof(c);
            p.X = x;
            p.offset += if p.flags & FLOW_UP != 0 { 1 } else { -1 };
            p.height -= 1;
            current = p.link;
        }

        /* Then sort them */
        let mut prev: Option<usize> = None;
        let Some(mut current) = *list else {
            return;
        };

        let mut next = self.profiles[current].link;

        while let Some(n) = next {
            if self.profiles[current].X <= self.profiles[n].X {
                prev = Some(current);
                match self.profiles[current].link {
                    Some(c) => current = c,
                    None => return,
                }
            } else {
                match prev {
                    None => *list = Some(n),
                    Some(p) => self.prof(p).link = Some(n),
                }
                self.prof(current).link = self.profiles[n].link;
                self.prof(n).link = Some(current);

                prev = None;
                current = list.unwrap();
            }

            next = self.profiles[current].link;
        }
    }

    /*
     * Vertical Sweep Procedure Set
     *
     * These four routines are used during the vertical black/white sweep
     * phase by the generic Draw_Sweep() function.
     *
     */

    /// `Vertical_Sweep_Init`
    fn vertical_sweep_init(&mut self, min: Short, _max: Short) {
        self.bLine = self.bOrigin - min as isize * self.target_pitch;
    }

    /// `Vertical_Sweep_Span`
    fn vertical_sweep_span(
        &mut self,
        _y: Short,
        x1: FtF26Dot6,
        x2: FtF26Dot6,
        left: usize,
        _right: usize,
    ) {
        let drop_out_control = (self.profiles[left].flags & 7) as Int;

        /* Drop-out control */

        let mut e1 = self.ceiling(x1);
        let mut e2 = self.floor(x2);

        /* take care of the special case where both the left */
        /* and right contour lie exactly on pixel centers    */
        if drop_out_control != 2
            && x2.wrapping_sub(x1).wrapping_sub(self.precision as Long)
                <= self.precision_jitter as Long
            && e1 != x1
            && e2 != x2
        {
            e2 = e1;
        }

        e1 = self.trunc(e1);
        e2 = self.trunc(e2);

        if e2 >= 0 && e1 < self.bWidth as Long {
            if e1 < 0 {
                e1 = 0;
            }
            if e2 >= self.bWidth as Long {
                e2 = self.bWidth as Long - 1;
            }

            let c1 = (e1 >> 3) as Short as Int;
            let mut c2 = (e2 >> 3) as Short as Int;

            let f1 = (0xFF >> (e1 & 7)) as Byte;
            let f2 = !(0x7F >> (e2 & 7)) as Byte;

            let mut target = (self.bLine + c1 as isize) as usize;
            c2 -= c1;

            if c2 > 0 {
                self.buffer[target] |= f1;

                /* memset() is slower than the following code on many platforms. */
                /* This is due to the fact that, in the vast majority of cases,  */
                /* the span length in bytes is relatively small.                 */
                loop {
                    c2 -= 1;
                    if c2 <= 0 {
                        break;
                    }
                    target += 1;
                    self.buffer[target] = 0xFF;
                }
                self.buffer[target + 1] |= f2;
            } else {
                self.buffer[target] |= f1 & f2;
            }
        }
    }

    /// `Vertical_Sweep_Drop`
    fn vertical_sweep_drop(
        &mut self,
        y: Short,
        x1: FtF26Dot6,
        x2: FtF26Dot6,
        left: usize,
        right: usize,
    ) {
        /* Drop-out control */

        /*   e2            x2                    x1           e1   */
        /*                                                         */
        /*                 ^                     |                 */
        /*                 |                     |                 */
        /*   +-------------+---------------------+------------+    */
        /*                 |                     |                 */
        /*                 |                     v                 */
        /*                                                         */
        /* pixel         contour              contour       pixel  */
        /* center                                           center */

        /* drop-out mode    scan conversion rules (as defined in OpenType) */
        /* --------------------------------------------------------------- */
        /*  0                1, 2, 3                                       */
        /*  1                1, 2, 4                                       */
        /*  2                1, 2                                          */
        /*  3                same as mode 2                                */
        /*  4                1, 2, 5                                       */
        /*  5                1, 2, 6                                       */
        /*  6, 7             same as mode 2                                */

        let mut e1 = self.ceiling(x1);
        let e2 = self.floor(x2);
        let mut pxl = e1;

        if e1 > e2 {
            let l = self.profiles[left];
            let drop_out_control = (l.flags & 7) as Int;

            if e1 == e2 + self.precision as Long {
                match drop_out_control {
                    0 => {
                        /* simple drop-outs including stubs */
                        pxl = e2;
                    }

                    4 => {
                        /* smart drop-outs including stubs */
                        pxl = self.smart(x1, x2);
                    }

                    1 | 5 => {
                        /* simple/smart drop-outs excluding stubs */
                        /* Drop-out Control Rules #4 and #6 */

                        /* The specification neither provides an exact definition */
                        /* of a `stub' nor gives exact rules to exclude them.     */
                        /*                                                        */
                        /* Here the constraints we use to recognize a stub.       */
                        /*                                                        */
                        /*  upper stub:                                           */
                        /*                                                        */
                        /*   - P_Left and P_Right are in the same contour         */
                        /*   - P_Right is the successor of P_Left in that contour */
                        /*   - y is the top of P_Left and P_Right                 */
                        /*                                                        */
                        /*  lower stub:                                           */
                        /*                                                        */
                        /*   - P_Left and P_Right are in the same contour         */
                        /*   - P_Left is the successor of P_Right in that contour */
                        /*   - y is the bottom of P_Left                          */
                        /*                                                        */
                        /* We draw a stub if the following constraints are met.   */
                        /*                                                        */
                        /*   - for an upper or lower stub, there is top or bottom */
                        /*     overshoot, respectively                            */
                        /*   - the covered interval is greater or equal to a half */
                        /*     pixel                                              */

                        /* upper stub test */
                        if l.next == Some(right)
                            && l.height <= 0
                            && !(l.flags & OVERSHOOT_TOP != 0
                                && x2 - x1 >= self.precision_half as Long)
                        {
                            return;
                        }

                        /* lower stub test */
                        if self.profiles[right].next == Some(left)
                            && l.start == y as Long
                            && !(l.flags & OVERSHOOT_BOTTOM != 0
                                && x2 - x1 >= self.precision_half as Long)
                        {
                            return;
                        }

                        if drop_out_control == 1 {
                            pxl = e2;
                        } else {
                            pxl = self.smart(x1, x2);
                        }
                    }

                    _ => {
                        /* modes 2, 3, 6, 7 */
                        return; /* no drop-out control */
                    }
                }

                /* undocumented but confirmed: If the drop-out would result in a  */
                /* pixel outside of the bounding box, use the pixel inside of the */
                /* bounding box instead                                           */
                if pxl < 0 {
                    pxl = e1;
                } else if self.trunc(pxl) >= self.bWidth as Long {
                    pxl = e2;
                }

                /* check that the other pixel isn't set */
                e1 = if pxl == e1 { e2 } else { e1 };

                e1 = self.trunc(e1);

                let c1 = (e1 >> 3) as Short;
                let f1 = (e1 & 7) as Short;

                if e1 >= 0
                    && e1 < self.bWidth as Long
                    && self.buffer[(self.bLine + c1 as isize) as usize] & (0x80u8 >> f1) != 0
                {
                    return;
                }
            } else {
                return;
            }
        }

        e1 = self.trunc(pxl);

        if e1 >= 0 && e1 < self.bWidth as Long {
            let c1 = (e1 >> 3) as Short;
            let f1 = (e1 & 7) as Short;

            self.buffer[(self.bLine + c1 as isize) as usize] |= 0x80u8 >> f1;
        }

        /* Exit: */
    }

    /// `Vertical_Sweep_Step`
    fn vertical_sweep_step(&mut self) {
        self.bLine -= self.target_pitch;
    }

    /*
     * Horizontal Sweep Procedure Set
     *
     * These four routines are used during the horizontal black/white
     * sweep phase by the generic Draw_Sweep() function.
     *
     */

    /// `Horizontal_Sweep_Span`
    fn horizontal_sweep_span(
        &mut self,
        y: Short,
        x1: FtF26Dot6,
        x2: FtF26Dot6,
        _left: usize,
        _right: usize,
    ) {
        /* We should not need this procedure but the vertical sweep   */
        /* mishandles horizontal lines through pixel centers.  So we  */
        /* have to check perfectly aligned span edges here.           */
        /*                                                            */
        /* XXX: Can we handle horizontal lines better and drop this?  */

        let mut e1 = self.ceiling(x1);

        if x1 == e1 {
            e1 = self.trunc(e1);

            if e1 >= 0 && (e1 as u64) < self.target_rows as u64 {
                let bits = self.bOrigin + (y >> 3) as isize - e1 as isize * self.target_pitch;
                let f1 = (0x80 >> (y & 7)) as Byte;

                self.buffer[bits as usize] |= f1;
            }
        }

        let mut e2 = self.floor(x2);

        if x2 == e2 {
            e2 = self.trunc(e2);

            if e2 >= 0 && (e2 as u64) < self.target_rows as u64 {
                let bits = self.bOrigin + (y >> 3) as isize - e2 as isize * self.target_pitch;
                let f1 = (0x80 >> (y & 7)) as Byte;

                self.buffer[bits as usize] |= f1;
            }
        }
    }

    /// `Horizontal_Sweep_Drop`
    fn horizontal_sweep_drop(
        &mut self,
        y: Short,
        x1: FtF26Dot6,
        x2: FtF26Dot6,
        left: usize,
        right: usize,
    ) {
        /* During the horizontal sweep, we only take care of drop-outs */

        /* e1     +       <-- pixel center */
        /*        |                        */
        /* x1  ---+-->    <-- contour      */
        /*        |                        */
        /*        |                        */
        /* x2  <--+---    <-- contour      */
        /*        |                        */
        /*        |                        */
        /* e2     +       <-- pixel center */

        let mut e1 = self.ceiling(x1);
        let e2 = self.floor(x2);
        let mut pxl = e1;

        if e1 > e2 {
            let l = self.profiles[left];
            let drop_out_control = (l.flags & 7) as Int;

            if e1 == e2 + self.precision as Long {
                match drop_out_control {
                    0 => {
                        /* simple drop-outs including stubs */
                        pxl = e2;
                    }

                    4 => {
                        /* smart drop-outs including stubs */
                        pxl = self.smart(x1, x2);
                    }

                    1 | 5 => {
                        /* simple/smart drop-outs excluding stubs */
                        /* see Vertical_Sweep_Drop for details */

                        /* rightmost stub test */
                        if l.next == Some(right)
                            && l.height <= 0
                            && !(l.flags & OVERSHOOT_TOP != 0
                                && x2 - x1 >= self.precision_half as Long)
                        {
                            return;
                        }

                        /* leftmost stub test */
                        if self.profiles[right].next == Some(left)
                            && l.start == y as Long
                            && !(l.flags & OVERSHOOT_BOTTOM != 0
                                && x2 - x1 >= self.precision_half as Long)
                        {
                            return;
                        }

                        if drop_out_control == 1 {
                            pxl = e2;
                        } else {
                            pxl = self.smart(x1, x2);
                        }
                    }

                    _ => {
                        /* modes 2, 3, 6, 7 */
                        return; /* no drop-out control */
                    }
                }

                /* undocumented but confirmed: If the drop-out would result in a  */
                /* pixel outside of the bounding box, use the pixel inside of the */
                /* bounding box instead                                           */
                if pxl < 0 {
                    pxl = e1;
                } else if (self.trunc(pxl) as u64) >= self.target_rows as u64 {
                    pxl = e2;
                }

                /* check that the other pixel isn't set */
                e1 = if pxl == e1 { e2 } else { e1 };

                e1 = self.trunc(e1);

                let bits = self.bOrigin + (y >> 3) as isize - e1 as isize * self.target_pitch;
                let f1 = (0x80 >> (y & 7)) as Byte;

                if e1 >= 0
                    && (e1 as u64) < self.target_rows as u64
                    && self.buffer[bits as usize] & f1 != 0
                {
                    return;
                }
            } else {
                return;
            }
        }

        e1 = self.trunc(pxl);

        if e1 >= 0 && (e1 as u64) < self.target_rows as u64 {
            let bits = self.bOrigin + (y >> 3) as isize - e1 as isize * self.target_pitch;
            let f1 = (0x80 >> (y & 7)) as Byte;

            self.buffer[bits as usize] |= f1;
        }

        /* Exit: */
    }

    /// `ras.Proc_Sweep_Init`
    fn proc_sweep_init(&mut self, min: Short, max: Short) {
        match self.sweep {
            Sweep::Vertical => self.vertical_sweep_init(min, max),
            Sweep::Horizontal => { /* `Horizontal_Sweep_Init': nothing, really */ }
        }
    }

    /// `ras.Proc_Sweep_Span`
    fn proc_sweep_span(
        &mut self,
        y: Short,
        x1: FtF26Dot6,
        x2: FtF26Dot6,
        left: usize,
        right: usize,
    ) {
        match self.sweep {
            Sweep::Vertical => self.vertical_sweep_span(y, x1, x2, left, right),
            Sweep::Horizontal => self.horizontal_sweep_span(y, x1, x2, left, right),
        }
    }

    /// `ras.Proc_Sweep_Drop`
    fn proc_sweep_drop(
        &mut self,
        y: Short,
        x1: FtF26Dot6,
        x2: FtF26Dot6,
        left: usize,
        right: usize,
    ) {
        match self.sweep {
            Sweep::Vertical => self.vertical_sweep_drop(y, x1, x2, left, right),
            Sweep::Horizontal => self.horizontal_sweep_drop(y, x1, x2, left, right),
        }
    }

    /// `ras.Proc_Sweep_Step`
    fn proc_sweep_step(&mut self) {
        match self.sweep {
            Sweep::Vertical => self.vertical_sweep_step(),
            Sweep::Horizontal => { /* `Horizontal_Sweep_Step': Nothing, really */ }
        }
    }

    /// `Draw_Sweep`: Generic Sweep Drawing routine
    fn draw_sweep(&mut self) -> bool {
        /* initialize empty linked lists */

        let mut waiting: Option<usize> = None;
        let mut draw_left: Option<usize> = None;
        let mut draw_right: Option<usize> = None;

        /* first, compute min and max Y */

        let mut p = self.fProfile;
        let mut max_y = self.trunc(self.minY) as Short;
        let mut min_y = self.trunc(self.maxY) as Short;

        while let Some(pi) = p {
            let q = self.profiles[pi].link;

            let bottom = self.profiles[pi].start as Short;
            let top = (self.profiles[pi].start + self.profiles[pi].height - 1) as Short;

            if min_y > bottom {
                min_y = bottom;
            }
            if max_y < top {
                max_y = top;
            }

            self.prof(pi).X = 0;
            self.ins_new(&mut waiting, pi);

            p = q;
        }

        /* check the Y-turns */
        if self.numTurns == 0 {
            self.error = FT_ERR_INVALID_OUTLINE;
            return FAILURE;
        }

        /* now initialize the sweep */

        self.proc_sweep_init(min_y, max_y);

        /* then compute the distance of each profile from min_Y */

        let mut p = waiting;

        while let Some(pi) = p {
            let pr = self.prof(pi);
            pr.countL = (pr.start - min_y as Long) as Int;
            p = pr.link;
        }

        /* let's go */

        let mut y = min_y;
        let mut y_height: Short = 0;

        if self.numTurns > 0 && self.size_buff(-(self.numTurns as isize)) == min_y as Long {
            self.numTurns -= 1;
        }

        while self.numTurns > 0 {
            /* check waiting list for new activations */

            let mut p = waiting;

            while let Some(pi) = p {
                let q = self.profiles[pi].link;
                self.prof(pi).countL -= y_height as Int;
                if self.profiles[pi].countL == 0 {
                    self.del_old(&mut waiting, pi);

                    if self.profiles[pi].flags & FLOW_UP != 0 {
                        self.ins_new(&mut draw_left, pi);
                    } else {
                        self.ins_new(&mut draw_right, pi);
                    }
                }

                p = q;
            }

            /* sort the drawing lists */

            self.sort(&mut draw_left);
            self.sort(&mut draw_right);

            let y_change = self.size_buff(-(self.numTurns as isize)) as Short;
            self.numTurns -= 1;
            y_height = y_change.wrapping_sub(y);

            while y < y_change {
                /* let's trace */

                let mut dropouts = 0;

                let mut p_left = draw_left;
                let mut p_right = draw_right;

                while let (Some(l), Some(r)) = (p_left, p_right) {
                    let mut x1 = self.profiles[l].X;
                    let mut x2 = self.profiles[r].X;

                    if x1 > x2 {
                        std::mem::swap(&mut x1, &mut x2);
                    }

                    let e1 = self.floor(x1);
                    let e2 = self.ceiling(x2);

                    let mut skip = false;
                    if x2 - x1 <= self.precision as Long && e1 != x1 && e2 != x2 {
                        if e1 > e2 || e2 == e1 + self.precision as Long {
                            let drop_out_control = (self.profiles[l].flags & 7) as Int;

                            if drop_out_control != 2 {
                                /* a drop-out was detected */

                                self.prof(l).X = x1;
                                self.prof(r).X = x2;

                                /* mark profile for drop-out processing */
                                self.prof(l).countL = 1;
                                dropouts += 1;
                            }

                            skip = true;
                        }
                    }

                    if !skip {
                        self.proc_sweep_span(y, x1, x2, l, r);
                    }

                    /* Skip_To_Next: */

                    p_left = self.profiles[l].link;
                    p_right = self.profiles[r].link;
                }

                /* handle drop-outs _after_ the span drawing --       */
                /* drop-out processing has been moved out of the loop */
                /* for performance tuning                             */
                if dropouts > 0 {
                    /* Scan_DropOuts: */

                    let mut p_left = draw_left;
                    let mut p_right = draw_right;

                    while let (Some(l), Some(r)) = (p_left, p_right) {
                        if self.profiles[l].countL != 0 {
                            self.prof(l).countL = 0;
                            /* dropouts--;  -- this is useful when debugging only */
                            let (xl, xr) = (self.profiles[l].X, self.profiles[r].X);
                            self.proc_sweep_drop(y, xl, xr, l, r);
                        }

                        p_left = self.profiles[l].link;
                        p_right = self.profiles[r].link;
                    }
                }

                /* Next_Line: */
                self.proc_sweep_step();

                y = y.wrapping_add(1);

                if y < y_change {
                    self.sort(&mut draw_left);
                    self.sort(&mut draw_right);
                }
            }

            /* now finalize the profiles that need it */

            let mut p = draw_left;
            while let Some(pi) = p {
                let q = self.profiles[pi].link;
                if self.profiles[pi].height == 0 {
                    self.del_old(&mut draw_left, pi);
                }
                p = q;
            }

            let mut p = draw_right;
            while let Some(pi) = p {
                let q = self.profiles[pi].link;
                if self.profiles[pi].height == 0 {
                    self.del_old(&mut draw_right, pi);
                }
                p = q;
            }
        }

        /* for gray-scaling, flush the bitmap scanline cache */
        while y <= max_y {
            self.proc_sweep_step();
            y = y.wrapping_add(1);
            if y == Short::MIN {
                break;
            }
        }

        SUCCESS
    }

    /// `Render_Single_Pass`: Perform one sweep with sub-banding.
    ///
    /// `flipped`: If set, flip the direction of the outline.
    ///
    /// Returns renderer error code.
    fn render_single_pass(&mut self, flipped: bool, y_min: Int, y_max: Int) -> FtError {
        let (mut y_min, mut y_max) = (y_min, y_max);
        let mut band_top: usize = 0;
        let mut band_stack: [Int; 32] = [0; 32]; /* enough to bisect 32-bit int bands */

        loop {
            self.minY = y_min as Long * self.precision as Long;
            self.maxY = y_max as Long * self.precision as Long;

            self.top = 0;

            self.error = FT_ERR_OK;

            if self.convert_glyph(flipped) {
                if self.error != FT_ERR_RASTER_OVERFLOW {
                    return self.error;
                }

                /* sub-banding */

                if y_min == y_max {
                    return self.error; /* still Raster_Overflow */
                }

                let y_mid = (y_min + y_max) >> 1;

                band_stack[band_top] = y_min;
                band_top += 1;
                y_min = y_mid + 1;
            } else {
                if self.fProfile.is_some() && self.draw_sweep() {
                    return self.error;
                }

                if band_top == 0 {
                    break;
                }
                band_top -= 1;

                y_max = y_min - 1;
                y_min = band_stack[band_top];
            }
        }

        FT_ERR_OK
    }

    /// `Render_Glyph`: Render a glyph in a bitmap.  Sub-banding if needed.
    ///
    /// Returns FreeType error code.  0 means success.
    fn render_glyph(&mut self) -> FtError {
        self.set_high_precision(self.outline.flags & FT_OUTLINE_HIGH_PRECISION != 0);

        if self.outline.flags & FT_OUTLINE_IGNORE_DROPOUTS != 0 {
            self.dropOutControl = 2;
        } else {
            if self.outline.flags & FT_OUTLINE_SMART_DROPOUTS != 0 {
                self.dropOutControl = 4;
            } else {
                self.dropOutControl = 0;
            }

            if self.outline.flags & FT_OUTLINE_INCLUDE_STUBS == 0 {
                self.dropOutControl += 1;
            }
        }

        /* Vertical Sweep */
        self.sweep = Sweep::Vertical;

        self.bWidth = self.target_width as UShort;
        self.bOrigin = 0;

        if self.target_pitch > 0 {
            self.bOrigin += (self.target_rows as isize - 1) * self.target_pitch;
        }

        let error = self.render_single_pass(false, 0, self.target_rows as Int - 1);
        if error != 0 {
            return error;
        }

        /* Horizontal Sweep */
        if self.outline.flags & FT_OUTLINE_SINGLE_PASS == 0 {
            self.sweep = Sweep::Horizontal;

            let error = self.render_single_pass(true, 0, self.target_width as Int - 1);
            if error != 0 {
                return error;
            }
        }

        FT_ERR_OK
    }
}

/// `Split_Conic`: Subdivide one conic Bezier into two joint sub-arcs in
/// the Bezier stack.
///
/// This routine is the `beef' of this component.  It is  _the_ inner loop
/// that should be optimized to hell to get the best performance.
fn split_conic(base: &mut [TPoint]) {
    base[4].x = base[2].x;
    let a = base[0].x.wrapping_add(base[1].x);
    let b = base[1].x.wrapping_add(base[2].x);
    base[3].x = b >> 1;
    base[2].x = a.wrapping_add(b) >> 2;
    base[1].x = a >> 1;

    base[4].y = base[2].y;
    let a = base[0].y.wrapping_add(base[1].y);
    let b = base[1].y.wrapping_add(base[2].y);
    base[3].y = b >> 1;
    base[2].y = a.wrapping_add(b) >> 2;
    base[1].y = a >> 1;

    /* hand optimized.  gcc doesn't seem to be too good at common      */
    /* expression substitution and instruction scheduling ;-)          */
}

/// `Split_Cubic`: Subdivide a third-order Bezier arc into two joint
/// sub-arcs in the Bezier stack.
///
/// This routine is the `beef' of the component.  It is one of _the_ inner
/// loops that should be optimized like hell to get the best performance.
fn split_cubic(base: &mut [TPoint]) {
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

/// `ft_black_render`
fn ft_black_render(source: FtRasterSource<'_>, params: &mut FtRasterParams<'_>) -> FtResult<()> {
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

    if outline.n_points as Int != outline.contours[outline.n_contours as usize - 1] as Int + 1 {
        return Err(FT_ERR_INVALID_OUTLINE);
    }

    /* this version of the raster does not support direct rendering, sorry */
    if params.flags & FT_RASTER_FLAG_DIRECT != 0 || params.flags & FT_RASTER_FLAG_AA != 0 {
        return Err(FT_ERR_CANNOT_RENDER_GLYPH);
    }

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

    let mut worker = BlackTWorker {
        precision_bits: 0,
        precision: 0,
        precision_half: 0,
        precision_scale: 0,
        precision_step: 0,
        precision_jitter: 0,
        pool: vec![0; FT_MAX_BLACK_POOL],
        profiles: vec![TProfile::default(); FT_MAX_BLACK_POOL],
        maxBuff: 0,
        top: 0,
        error: 0,
        numTurns: 0,
        dropOutControl: 0,
        bWidth: 0,
        bOrigin: 0,
        bLine: 0,
        lastX: 0,
        lastY: 0,
        minY: 0,
        maxY: 0,
        num_Profs: 0,
        fresh: false,
        joint: false,
        cProfile: 0,
        fProfile: None,
        gProfile: None,
        state: TStates::Unknown,
        target_rows: target_map.rows,
        target_width: target_map.width,
        target_pitch: target_map.pitch as isize,
        buffer: &mut target_map.buffer[..],
        outline,
        sweep: Sweep::Vertical,
    };

    match worker.render_glyph() {
        0 => Ok(()),
        e => Err(e),
    }
}

/// `ft_standard_raster` (`ft_black_new`, `ft_black_reset`,
/// `ft_black_set_mode`, and `ft_black_done` have nothing to do)
pub static FT_STANDARD_RASTER: FtRasterFuncs = FtRasterFuncs {
    glyph_format: FT_GLYPH_FORMAT_OUTLINE,
    raster_render: ft_black_render,
};
