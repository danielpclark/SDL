// Rust translation of src/libm/e_fmod.c, e_sqrt.c, s_floor.c, s_scalbn.c,
// s_modf.c, s_copysign.c, s_fabs.c, s_isinf.c, s_isinff.c, s_isnan.c and
// s_isnanf.c from Simple DirectMedia Layer.
//
// ====================================================
// Copyright (C) 1993 by Sun Microsystems, Inc. All rights reserved.
//
// Developed at SunPro, a Sun Microsystems, Inc. business.
// Permission to use, copy, modify, and distribute this
// software is freely granted, provided that this notice
// is preserved.
// ====================================================
//
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! fmod, sqrt, floor, scalbn, modf, copysign, fabs, isinf, isnan.

use super::words::{float_word, from_words, hi, with_hi, words};

const ONE: f64 = 1.0;
const ZERO_SIGNED: [f64; 2] = [0.0, -0.0];

/// Return x mod y in exact arithmetic.
/// Method: shift and subtract.
/// Translation of `__ieee754_fmod()` (`SDL_uclibc_fmod`).
pub(super) fn fmod(mut x: f64, y: f64) -> f64 {
    let (mut hx, mut lx) = words(x);
    let (mut hy, mut ly) = words(y);
    let sx = hx & (0x80000000u32 as i32); /* sign of x */
    hx ^= sx; /* |x| */
    hy &= 0x7fffffff; /* |y| */
    let sign_index = ((sx as u32) >> 31) as usize;

    /* purge off exception values */
    if (hy as u32 | ly) == 0
        || hx >= 0x7ff00000 /* y=0,or x not finite */
        || (hy as u32 | ((ly | (ly as i32).wrapping_neg() as u32) >> 31)) > 0x7ff00000
    {
        /* or y is NaN */
        return (x * y) / (x * y);
    }
    if hx <= hy {
        if hx < hy || lx < ly {
            return x; /* |x|<|y| return x */
        }
        if lx == ly {
            return ZERO_SIGNED[sign_index]; /* |x|=|y| return x*0*/
        }
    }

    /* determine ix = ilogb(x) */
    let mut ix: i32;
    if hx < 0x00100000 {
        /* subnormal x */
        if hx == 0 {
            ix = -1043;
            let mut i = lx as i32;
            while i > 0 {
                ix -= 1;
                i <<= 1;
            }
        } else {
            ix = -1022;
            let mut i = hx << 11;
            while i > 0 {
                ix -= 1;
                i <<= 1;
            }
        }
    } else {
        ix = (hx >> 20) - 1023;
    }

    /* determine iy = ilogb(y) */
    let mut iy: i32;
    if hy < 0x00100000 {
        /* subnormal y */
        if hy == 0 {
            iy = -1043;
            let mut i = ly as i32;
            while i > 0 {
                iy -= 1;
                i <<= 1;
            }
        } else {
            iy = -1022;
            let mut i = hy << 11;
            while i > 0 {
                iy -= 1;
                i <<= 1;
            }
        }
    } else {
        iy = (hy >> 20) - 1023;
    }

    /* set up {hx,lx}, {hy,ly} and align y to x */
    if ix >= -1022 {
        hx = 0x00100000 | (0x000fffff & hx);
    } else {
        /* subnormal x, shift x to normal */
        let n = -1022 - ix;
        if n <= 31 {
            hx = (hx << n) | (lx >> (32 - n)) as i32;
            lx <<= n;
        } else {
            hx = (lx << (n - 32)) as i32;
            lx = 0;
        }
    }
    if iy >= -1022 {
        hy = 0x00100000 | (0x000fffff & hy);
    } else {
        /* subnormal y, shift y to normal */
        let n = -1022 - iy;
        if n <= 31 {
            hy = (hy << n) | (ly >> (32 - n)) as i32;
            ly <<= n;
        } else {
            hy = (ly << (n - 32)) as i32;
            ly = 0;
        }
    }

    /* fix point fmod */
    let mut n = ix - iy;
    let mut hz: i32;
    let mut lz: u32;
    while n != 0 {
        n -= 1;
        hz = hx - hy;
        lz = lx.wrapping_sub(ly);
        if lx < ly {
            hz -= 1;
        }
        if hz < 0 {
            hx = hx + hx + (lx >> 31) as i32;
            lx = lx.wrapping_add(lx);
        } else {
            if (hz as u32 | lz) == 0 {
                /* return sign(x)*0 */
                return ZERO_SIGNED[sign_index];
            }
            hx = hz + hz + (lz >> 31) as i32;
            lx = lz.wrapping_add(lz);
        }
    }
    hz = hx - hy;
    lz = lx.wrapping_sub(ly);
    if lx < ly {
        hz -= 1;
    }
    if hz >= 0 {
        hx = hz;
        lx = lz;
    }

    /* convert back to floating value and restore the sign */
    if (hx as u32 | lx) == 0 {
        /* return sign(x)*0 */
        return ZERO_SIGNED[sign_index];
    }
    while hx < 0x00100000 {
        /* normalize x */
        hx = hx + hx + (lx >> 31) as i32;
        lx = lx.wrapping_add(lx);
        iy -= 1;
    }
    if iy >= -1022 {
        /* normalize output */
        hx = (hx - 0x00100000) | ((iy + 1023) << 20);
        x = from_words((hx | sx) as u32, lx);
    } else {
        /* subnormal output */
        let n = -1022 - iy;
        if n <= 20 {
            lx = (lx >> n) | ((hx as u32) << (32 - n));
            hx >>= n;
        } else if n <= 31 {
            lx = ((hx << (32 - n)) as u32) | (lx >> n);
            hx = sx;
        } else {
            lx = (hx >> (n - 32)) as u32;
            hx = sx;
        }
        x = from_words((hx | sx) as u32, lx);
        x *= ONE; /* create necessary signal */
    }
    x /* exact output */
}

const TINY: f64 = 1.0e-300;

/// Return correctly rounded sqrt.
///
/// Method:
///   Bit by bit method using integer arithmetic. (Slow, but portable)
///   1. Normalization
///      Scale x to y in [1,4) with even powers of 2:
///      find an integer k such that  1 <= (y=x*2^(2k)) < 4, then
///      sqrt(x) = 2^k * sqrt(y)
///   2. Bit by bit computation
///      Let q  = sqrt(y) truncated to i bit after binary point (q = 1),
///      s  = 2*q , and y  =  2   * ( y - q  ).
///      To compute q    from q , one checks whether
///      (q + 2^-(i+1))^2 <= y. (Kahan and Ng's paper on the other
///      methods is kept in upstream's e_sqrt.c.)
///   3. Final rounding
///      After generating the 53 bits result, we compute one more bit.
///      Together with the remainder, we can decide whether the
///      result is exact, bigger than 1/2ulp, or less than 1/2ulp
///      (it will never equal to 1/2ulp).
///
/// Translation of `__ieee754_sqrt()` (`SDL_uclibc_sqrt`).
pub(super) fn sqrt(x: f64) -> f64 {
    let mut z: f64;
    let sign: i32 = 0x80000000u32 as i32;
    let sign_u: u32 = 0x80000000;
    let mut s0: i32;
    let mut q: i32;
    let mut m: i32;
    let mut t: i32;
    let mut r: u32;
    let mut t1: u32;
    let mut s1: u32;
    let mut q1: u32;

    let (mut ix0, mut ix1) = words(x);

    /* take care of Inf and NaN */
    if (ix0 & 0x7ff00000) == 0x7ff00000 {
        return x * x + x; /* sqrt(NaN)=NaN, sqrt(+inf)=+inf
                          sqrt(-inf)=sNaN */
    }
    /* take care of zero */
    if ix0 <= 0 {
        if ((ix0 & !sign) as u32 | ix1) == 0 {
            return x; /* sqrt(+-0) = +-0 */
        } else if ix0 < 0 {
            return (x - x) / (x - x); /* sqrt(-ve) = sNaN */
        }
    }
    /* normalize x */
    m = ix0 >> 20;
    if m == 0 {
        /* subnormal x */
        while ix0 == 0 {
            m -= 21;
            ix0 |= (ix1 >> 11) as i32;
            ix1 <<= 21;
        }
        let mut i = 0;
        while (ix0 & 0x00100000) == 0 {
            ix0 <<= 1;
            i += 1;
        }
        m -= i - 1;
        // Upstream writes `ix1>>(32-i)`, which is undefined for i == 0 (a shift
        // by 32); the intent is "move the top i bits of ix1", i.e. nothing.
        if i != 0 {
            ix0 |= (ix1 >> (32 - i)) as i32;
        }
        ix1 <<= i;
    }
    m -= 1023; /* unbias exponent */
    ix0 = (ix0 & 0x000fffff) | 0x00100000;
    if (m & 1) != 0 {
        /* odd m, double x to make it even */
        ix0 += ix0 + ((ix1 & sign_u) >> 31) as i32;
        ix1 = ix1.wrapping_add(ix1);
    }
    m >>= 1; /* m = [m/2] */

    /* generate sqrt(x) bit by bit */
    ix0 += ix0 + ((ix1 & sign_u) >> 31) as i32;
    ix1 = ix1.wrapping_add(ix1);
    q = 0;
    q1 = 0;
    s0 = 0;
    s1 = 0; /* [q,q1] = sqrt(x) */
    r = 0x00200000; /* r = moving bit from right to left */

    while r != 0 {
        t = s0 + r as i32;
        if t <= ix0 {
            s0 = t + r as i32;
            ix0 -= t;
            q += r as i32;
        }
        ix0 += ix0 + ((ix1 & sign_u) >> 31) as i32;
        ix1 = ix1.wrapping_add(ix1);
        r >>= 1;
    }

    r = sign_u;
    while r != 0 {
        t1 = s1.wrapping_add(r);
        t = s0;
        if t < ix0 || (t == ix0 && t1 <= ix1) {
            s1 = t1.wrapping_add(r);
            if (t1 & sign_u) == sign_u && (s1 & sign_u) == 0 {
                s0 += 1;
            }
            ix0 -= t;
            if ix1 < t1 {
                ix0 -= 1;
            }
            ix1 = ix1.wrapping_sub(t1);
            q1 = q1.wrapping_add(r);
        }
        ix0 += ix0 + ((ix1 & sign_u) >> 31) as i32;
        ix1 = ix1.wrapping_add(ix1);
        r >>= 1;
    }

    /* use floating add to find out rounding direction */
    if (ix0 as u32 | ix1) != 0 {
        z = ONE - TINY; /* trigger inexact flag */
        if z >= ONE {
            z = ONE + TINY;
            if q1 == 0xffffffff {
                q1 = 0;
                q += 1;
            } else if z > ONE {
                if q1 == 0xfffffffe {
                    q += 1;
                }
                q1 = q1.wrapping_add(2);
            } else {
                q1 += q1 & 1;
            }
        }
    }
    ix0 = (q >> 1) + 0x3fe00000;
    ix1 = q1 >> 1;
    if (q & 1) == 1 {
        ix1 |= sign_u;
    }
    ix0 += m << 20;
    from_words(ix0 as u32, ix1)
}

const HUGE: f64 = 1.0e300;

/// Return x rounded toward -inf to integral value.
/// Method: Bit twiddling.
/// Exception: Inexact flag raised if x not equal to floor(x).
/// Translation of fdlibm `floor()` (`SDL_uclibc_floor`).
pub(super) fn floor(x: f64) -> f64 {
    let (mut i0, i1) = words(x);
    let mut i1 = i1 as i32;
    let j0 = ((i0 >> 20) & 0x7ff) - 0x3ff;
    if j0 < 20 {
        if j0 < 0 {
            /* raise inexact if x != 0 */
            if HUGE + x > 0.0 {
                /* return 0*sign(x) if |x|<1 */
                if i0 >= 0 {
                    i0 = 0;
                    i1 = 0;
                } else if ((i0 & 0x7fffffff) | i1) != 0 {
                    i0 = 0xbff00000u32 as i32;
                    i1 = 0;
                }
            }
        } else {
            let i = (0x000fffffu32 >> j0) as i32;
            if ((i0 & i) | i1) == 0 {
                return x; /* x is integral */
            }
            if HUGE + x > 0.0 {
                /* raise inexact flag */
                if i0 < 0 {
                    i0 += 0x00100000 >> j0;
                }
                i0 &= !i;
                i1 = 0;
            }
        }
    } else if j0 > 51 {
        if j0 == 0x400 {
            return x + x; /* inf or NaN */
        } else {
            return x; /* x is integral */
        }
    } else {
        let i = 0xffffffffu32 >> (j0 - 20);
        if (i1 as u32 & i) == 0 {
            return x; /* x is integral */
        }
        if HUGE + x > 0.0 {
            /* raise inexact flag */
            if i0 < 0 {
                if j0 == 20 {
                    i0 += 1;
                } else {
                    let j = (i1 as u32).wrapping_add(1u32 << (52 - j0));
                    if j < i1 as u32 {
                        i0 += 1; /* got a carry */
                    }
                    i1 = j as i32;
                }
            }
            i1 = (i1 as u32 & !i) as i32;
        }
    }
    from_words(i0 as u32, i1 as u32)
}

const TWO54: f64 = 1.80143985094819840000e+16; /* 0x43500000, 0x00000000 */
const TWOM54: f64 = 5.55111512312578270212e-17; /* 0x3C900000, 0x00000000 */

/// scalbln (double x, long n)
/// scalbln(x,n) returns x* 2**n  computed by  exponent
/// manipulation rather than by actually performing an
/// exponentiation or a multiplication.
/// Translation of fdlibm `scalbln()` (`SDL_uclibc_scalbln`); `long` is
/// taken as 64-bit, as on the LP64 platforms SDL is built for.
pub(super) fn scalbln(mut x: f64, n: i64) -> f64 {
    let (mut hx, lx) = words(x);
    let mut k = (hx & 0x7ff00000) >> 20; /* extract exponent */
    if k == 0 {
        /* 0 or subnormal x */
        if (lx | (hx & 0x7fffffff) as u32) == 0 {
            return x; /* +-0 */
        }
        x *= TWO54;
        hx = hi(x) as i32;
        k = ((hx & 0x7ff00000) >> 20) - 54;
    }
    if k == 0x7ff {
        return x + x; /* NaN or Inf */
    }
    k = (k as i64 + n) as i32;
    if k > 0x7fe {
        return HUGE * copysign(HUGE, x); /* overflow */
    }
    if n < -50000 {
        return TINY * copysign(TINY, x); /*underflow*/
    }
    if k > 0 {
        /* normal result */
        return with_hi(x, ((hx as u32) & 0x800fffff) | ((k as u32) << 20));
    }
    if k <= -54 {
        if n > 50000 {
            /* in case integer overflow in n+k */
            return HUGE * copysign(HUGE, x); /*overflow*/
        }
        return TINY * copysign(TINY, x); /*underflow*/
    }
    k += 54; /* subnormal result */
    x = with_hi(x, ((hx as u32) & 0x800fffff) | ((k as u32) << 20));
    x * TWOM54
}

/// x * 2**n. Translation of fdlibm `scalbn()` (`SDL_uclibc_scalbn`).
pub(super) fn scalbn(x: f64, n: i32) -> f64 {
    scalbln(x, n as i64)
}

/// modf(double x, double *iptr)
/// return fraction part of x, and return x's integral part in *iptr.
/// Method: Bit twiddling.
/// Translation of fdlibm `modf()` (`SDL_uclibc_modf`); returns (fraction, integral).
pub(super) fn modf(x: f64) -> (f64, f64) {
    let (i0, i1) = words(x);
    let j0 = ((i0 >> 20) & 0x7ff) - 0x3ff; /* exponent of x */
    if j0 < 20 {
        /* integer part in high x */
        if j0 < 0 {
            /* |x|<1 */
            let iptr = from_words(i0 as u32 & 0x80000000, 0); /* *iptr = +-0 */
            (x, iptr)
        } else {
            let i = 0x000fffffu32 >> j0;
            if ((i0 as u32 & i) | i1) == 0 {
                /* x is integral */
                let iptr = x;
                (from_words(i0 as u32 & 0x80000000, 0), iptr) /* return +-0 */
            } else {
                let iptr = from_words(i0 as u32 & !i, 0);
                (x - iptr, iptr)
            }
        }
    } else if j0 > 51 {
        /* no fraction part */
        let iptr = x * ONE;
        /* We must handle NaNs separately.  */
        if j0 == 0x400 && ((i0 as u32 & 0xfffff) | i1) != 0 {
            return (x * ONE, iptr);
        }
        (from_words(i0 as u32 & 0x80000000, 0), iptr) /* return +-0 */
    } else {
        /* fraction part in low x */
        let i = 0xffffffffu32 >> (j0 - 20);
        if (i1 & i) == 0 {
            /* x is integral */
            let iptr = x;
            (from_words(i0 as u32 & 0x80000000, 0), iptr) /* return +-0 */
        } else {
            let iptr = from_words(i0 as u32, i1 & !i);
            (x - iptr, iptr)
        }
    }
}

/// copysign(double x, double y)
/// copysign(x,y) returns a value with the magnitude of x and
/// with the sign bit of y.
/// Translation of fdlibm `copysign()` (`SDL_uclibc_copysign`).
pub(super) fn copysign(x: f64, y: f64) -> f64 {
    let hx = hi(x);
    let hy = hi(y);
    with_hi(x, (hx & 0x7fffffff) | (hy & 0x80000000))
}

/// fabs(x) returns the absolute value of x.
/// Translation of fdlibm `fabs()` (`SDL_uclibc_fabs`).
pub(super) fn fabs(x: f64) -> f64 {
    let high = hi(x);
    with_hi(x, high & 0x7fffffff)
}

/// isinf(x) returns 1 is x is inf, -1 if x is -inf, else 0;
/// no branching!
/// Translation of `__isinf()` (`SDL_uclibc_isinf`).
pub(super) fn isinf(x: f64) -> i32 {
    let (hx, lx) = words(x);
    let mut lx = lx as i32;
    lx |= (hx & 0x7fffffff) ^ 0x7ff00000;
    lx |= lx.wrapping_neg();
    !(lx >> 31) & (hx >> 30)
}

/// isinff(x) returns 1 is x is inf, -1 if x is -inf, else 0;
/// no branching!
/// Translation of `__isinff()` (`SDL_uclibc_isinff`).
pub(super) fn isinff(x: f32) -> i32 {
    let ix = float_word(x);
    let mut t = ix & 0x7fffffff;
    t ^= 0x7f800000;
    t |= t.wrapping_neg();
    !(t >> 31) & (ix >> 30)
}

/// isnan(x) returns 1 is x is nan, else 0;
/// no branching!
/// Translation of `__isnan()` (`SDL_uclibc_isnan`).
pub(super) fn isnan(x: f64) -> i32 {
    let (mut hx, lx) = words(x);
    let lx = lx as i32;
    hx &= 0x7fffffff;
    hx |= ((lx | lx.wrapping_neg()) as u32 >> 31) as i32;
    hx = 0x7ff00000 - hx;
    ((hx as u32) >> 31) as i32
}

/// isnanf(x) returns 1 is x is nan, else 0;
/// no branching!
/// Translation of `__isnanf()` (`SDL_uclibc_isnanf`).
pub(super) fn isnanf(x: f32) -> i32 {
    let mut ix = float_word(x);
    ix &= 0x7fffffff;
    ix = 0x7f800000 - ix;
    ((ix as u32) >> 31) as i32
}
