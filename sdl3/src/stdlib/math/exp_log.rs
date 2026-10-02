// Rust translation of src/libm/e_exp.c, e_log.c, e_log10.c and e_pow.c
// from Simple DirectMedia Layer.
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

//! exp, log, log10, pow.

use super::misc::{fabs, scalbn, sqrt};
use super::words::{from_words, hi, lo, with_hi, with_lo, words};

const ONE: f64 = 1.0;
const HALF_SIGNED: [f64; 2] = [0.5, -0.5];
const HUGE: f64 = 1.0e+300;
const TWOM1000: f64 = 9.33263618503218878990e-302; /* 2**-1000=0x01700000,0*/
const O_THRESHOLD: f64 = 7.09782712893383973096e+02; /* 0x40862E42, 0xFEFA39EF */
const U_THRESHOLD: f64 = -7.45133219101941108420e+02; /* 0xc0874910, 0xD52D3051 */
const LN2HI: [f64; 2] = [
    6.93147180369123816490e-01,  /* 0x3fe62e42, 0xfee00000 */
    -6.93147180369123816490e-01, /* 0xbfe62e42, 0xfee00000 */
];
const LN2LO: [f64; 2] = [
    1.90821492927058770002e-10,  /* 0x3dea39ef, 0x35793c76 */
    -1.90821492927058770002e-10, /* 0xbdea39ef, 0x35793c76 */
];
const INVLN2: f64 = 1.44269504088896338700e+00; /* 0x3ff71547, 0x652b82fe */
const P1: f64 = 1.66666666666666019037e-01; /* 0x3FC55555, 0x5555553E */
const P2: f64 = -2.77777777770155933842e-03; /* 0xBF66C16C, 0x16BEBD93 */
const P3: f64 = 6.61375632143793436117e-05; /* 0x3F11566A, 0xAF25DE2C */
const P4: f64 = -1.65339022054652515390e-06; /* 0xBEBBBD41, 0xC5D26BF1 */
const P5: f64 = 4.13813679705723846039e-08; /* 0x3E663769, 0x72BEA4D0 */

/// Binary representation of a 64-bit infinite double (sign=0, exponent=2047, mantissa=0).
/// Translation of `inf_union`.
const INF: f64 = f64::from_bits(0x7ff0000000000000);

/// exp(x): returns the exponential of x.
///
/// ```text
/// Method
///   1. Argument reduction:
///      Reduce x to an r so that |r| <= 0.5*ln2 ~ 0.34658.
///      Given x, find r and integer k such that
///
///               x = k*ln2 + r,  |r| <= 0.5*ln2.
///
///      Here r will be represented as r = hi-lo for better accuracy.
///
///   2. Approximation of exp(r) by a special rational function on
///      the interval [0,0.34658]:
///      R(r**2) = r*(exp(r)+1)/(exp(r)-1) = 2 + r*r/6 - r**4/360 + ...
///      We use a special Remes algorithm on [0,0.34658] to generate
///      a polynomial of degree 5 to approximate R. The maximum error
///      of this polynomial approximation is bounded by 2**-59.
///
///   3. Scale back to obtain exp(x):
///      From step 1, we have exp(x) = 2^k * exp(r)
/// ```
///
/// Translation of `__ieee754_exp()` (`SDL_uclibc_exp`).
pub(super) fn exp(mut x: f64) -> f64 {
    let y: f64;
    let mut hi_ = 0.0;
    let mut lo_ = 0.0;
    let mut k: i32 = 0;

    let mut hx = hi(x);
    let xsb = ((hx >> 31) & 1) as usize; /* sign bit of x */
    hx &= 0x7fffffff; /* high word of |x| */

    /* filter out non-finite argument */
    if hx >= 0x40862E42 {
        /* if |x|>=709.78... */
        if hx >= 0x7ff00000 {
            let lx = lo(x);
            if ((hx & 0xfffff) | lx) != 0 {
                return x + x; /* NaN */
            } else {
                return if xsb == 0 { x } else { 0.0 }; /* exp(+-inf)={inf,0} */
            }
        }
        if x > O_THRESHOLD {
            return INF; /* overflow */
        }
        if x < U_THRESHOLD {
            return TWOM1000 * TWOM1000; /* underflow */
        }
    }

    /* argument reduction */
    if hx > 0x3fd62e42 {
        /* if  |x| > 0.5 ln2 */
        if hx < 0x3FF0A2B2 {
            /* and |x| < 1.5 ln2 */
            hi_ = x - LN2HI[xsb];
            lo_ = LN2LO[xsb];
            k = 1 - xsb as i32 - xsb as i32;
        } else {
            k = (INVLN2 * x + HALF_SIGNED[xsb]) as i32;
            let t = k as f64;
            hi_ = x - t * LN2HI[0]; /* t*ln2HI is exact here */
            lo_ = t * LN2LO[0];
        }
        x = hi_ - lo_;
    } else if hx < 0x3e300000 {
        /* when |x|<2**-28 */
        if HUGE + x > ONE {
            return ONE + x; /* trigger inexact */
        }
    } else {
        k = 0;
    }

    /* x is now in primary range */
    let t = x * x;
    let c = x - t * (P1 + t * (P2 + t * (P3 + t * (P4 + t * P5))));
    if k == 0 {
        return ONE - ((x * c) / (c - 2.0) - x);
    } else {
        y = ONE - ((lo_ - (x * c) / (2.0 - c)) - hi_);
    }
    if k >= -1021 {
        let hy = hi(y);
        with_hi(y, hy.wrapping_add((k << 20) as u32)) /* add k to y's exponent */
    } else {
        let hy = hi(y);
        let y = with_hi(y, hy.wrapping_add(((k + 1000) << 20) as u32)); /* add k to y's exponent */
        y * TWOM1000
    }
}

const LN2_HI: f64 = 6.93147180369123816490e-01; /* 3fe62e42 fee00000 */
const LN2_LO: f64 = 1.90821492927058770002e-10; /* 3dea39ef 35793c76 */
const TWO54: f64 = 1.80143985094819840000e+16; /* 43500000 00000000 */
const LG1: f64 = 6.666666666666735130e-01; /* 3FE55555 55555593 */
const LG2: f64 = 3.999999999940941908e-01; /* 3FD99999 9997FA04 */
const LG3: f64 = 2.857142874366239149e-01; /* 3FD24924 94229359 */
const LG4: f64 = 2.222219843214978396e-01; /* 3FCC71C5 1D8E78AF */
const LG5: f64 = 1.818357216161805012e-01; /* 3FC74664 96CB03DE */
const LG6: f64 = 1.531383769920937332e-01; /* 3FC39A09 D078C69F */
const LG7: f64 = 1.479819860511658591e-01; /* 3FC2F112 DF3E5244 */
const ZERO: f64 = 0.0;

/// log(x): return the logarithm of x.
///
/// Method :
///   1. Argument Reduction: find k and f such that
///          x = 2^k * (1+f),
///      where  sqrt(2)/2 < 1+f < sqrt(2) .
///
///   2. Approximation of log(1+f).
///      Let s = f/(2+f) ; based on log(1+f) = log(1+s) - log(1-s)
///          = 2s + 2/3 s**3 + 2/5 s**5 + .....,
///          = 2s + s*R
///      We use a special Reme algorithm on [0,0.1716] to generate
///      a polynomial of degree 14 to approximate R The maximum error
///      of this polynomial approximation is bounded by 2**-58.45.
///
///   3. Finally,  log(x) = k*ln2 + log(1+f).
///                       = k*ln2_hi+(f-(hfsq-(s*(hfsq+R)+k*ln2_lo)))
///
/// Translation of `__ieee754_log()` (`SDL_uclibc_log`).
pub(super) fn log(mut x: f64) -> f64 {
    let (mut hx, lx) = words(x);

    let mut k: i32 = 0;
    if hx < 0x00100000 {
        /* x < 2**-1022  */
        if ((hx & 0x7fffffff) as u32 | lx) == 0 {
            return -TWO54 / ZERO; /* log(+-0)=-inf */
        }
        if hx < 0 {
            return (x - x) / ZERO; /* log(-#) = NaN */
        }
        k -= 54;
        x *= TWO54; /* subnormal number, scale up x */
        hx = hi(x) as i32;
    }
    if hx >= 0x7ff00000 {
        return x + x;
    }
    k += (hx >> 20) - 1023;
    hx &= 0x000fffff;
    let mut i = (hx + 0x95f64) & 0x100000;
    x = with_hi(x, (hx | (i ^ 0x3ff00000)) as u32); /* normalize x or x/2 */
    k += i >> 20;
    let f = x - 1.0;
    if (0x000fffff & (2 + hx)) < 3 {
        /* |f| < 2**-20 */
        if f == ZERO {
            if k == 0 {
                return ZERO;
            } else {
                let dk = k as f64;
                return dk * LN2_HI + dk * LN2_LO;
            }
        }
        let r = f * f * (0.5 - 0.33333333333333333 * f);
        if k == 0 {
            return f - r;
        } else {
            let dk = k as f64;
            return dk * LN2_HI - ((r - dk * LN2_LO) - f);
        }
    }
    let s = f / (2.0 + f);
    let dk = k as f64;
    let z = s * s;
    i = hx - 0x6147a;
    let w = z * z;
    let j = 0x6b851 - hx;
    let t1 = w * (LG2 + w * (LG4 + w * LG6));
    let t2 = z * (LG1 + w * (LG3 + w * (LG5 + w * LG7)));
    i |= j;
    let r = t2 + t1;
    if i > 0 {
        let hfsq = 0.5 * f * f;
        if k == 0 {
            f - (hfsq - s * (hfsq + r))
        } else {
            dk * LN2_HI - ((hfsq - (s * (hfsq + r) + dk * LN2_LO)) - f)
        }
    } else if k == 0 {
        f - s * (f - r)
    } else {
        dk * LN2_HI - ((s * (f - r) - dk * LN2_LO) - f)
    }
}

const IVLN10: f64 = 4.34294481903251816668e-01; /* 0x3FDBCB7B, 0x1526E50E */
const LOG10_2HI: f64 = 3.01029995663611771306e-01; /* 0x3FD34413, 0x509F6000 */
const LOG10_2LO: f64 = 3.69423907715893078616e-13; /* 0x3D59FEF3, 0x11F12B36 */

/// log10(x): return the base 10 logarithm of x.
///
/// Method :
///   Let log10_2hi = leading 40 bits of log10(2) and
///       log10_2lo = log10(2) - log10_2hi,
///       ivln10   = 1/log(10) rounded.
///   Then
///       n = ilogb(x),
///       if(n<0)  n = n+1;
///       x = scalbn(x,-n);
///       log10(x) := n*log10_2hi + (n*log10_2lo + ivln10*log(x))
///
/// Translation of `__ieee754_log10()` (`SDL_uclibc_log10`).
pub(super) fn log10(mut x: f64) -> f64 {
    let (mut hx, lx) = words(x);

    let mut k: i32 = 0;
    if hx < 0x00100000 {
        /* x < 2**-1022  */
        if ((hx & 0x7fffffff) as u32 | lx) == 0 {
            return -TWO54 / ZERO; /* log(+-0)=-inf */
        }
        if hx < 0 {
            return (x - x) / ZERO; /* log(-#) = NaN */
        }
        k -= 54;
        x *= TWO54; /* subnormal number, scale up x */
        hx = hi(x) as i32;
    }
    if hx >= 0x7ff00000 {
        return x + x;
    }
    k += (hx >> 20) - 1023;
    let i = ((k as u32) & 0x80000000) >> 31;
    hx = (hx & 0x000fffff) | ((0x3ff - i as i32) << 20);
    let y = (k + i as i32) as f64;
    x = with_hi(x, hx as u32);
    let z = y * LOG10_2LO + IVLN10 * log(x);
    z + y * LOG10_2HI
}

const BP: [f64; 2] = [1.0, 1.5];
const DP_H: [f64; 2] = [0.0, 5.84962487220764160156e-01]; /* 0x3FE2B803, 0x40000000 */
const DP_L: [f64; 2] = [0.0, 1.35003920212974897128e-08]; /* 0x3E4CFDEB, 0x43CFD006 */
const TWO: f64 = 2.0;
const TWO53: f64 = 9007199254740992.0; /* 0x43400000, 0x00000000 */
const TINY: f64 = 1.0e-300;
/* poly coefs for (3/2)*(log(x)-2s-2/3*s**3 */
const L1: f64 = 5.99999999999994648725e-01; /* 0x3FE33333, 0x33333303 */
const L2: f64 = 4.28571428578550184252e-01; /* 0x3FDB6DB6, 0xDB6FABFF */
const L3: f64 = 3.33333329818377432918e-01; /* 0x3FD55555, 0x518F264D */
const L4: f64 = 2.72728123808534006489e-01; /* 0x3FD17460, 0xA91D4101 */
const L5: f64 = 2.30660745775561754067e-01; /* 0x3FCD864A, 0x93C9DB65 */
const L6: f64 = 2.06975017800338417784e-01; /* 0x3FCA7E28, 0x4A454EEF */
const LG2_: f64 = 6.93147180559945286227e-01; /* 0x3FE62E42, 0xFEFA39EF */
const LG2_H: f64 = 6.93147182464599609375e-01; /* 0x3FE62E43, 0x00000000 */
const LG2_L: f64 = -1.90465429995776804525e-09; /* 0xBE205C61, 0x0CA86C39 */
const OVT: f64 = 8.0085662595372944372e-0017; /* -(1024-log2(ovfl+.5ulp)) */
const CP: f64 = 9.61796693925975554329e-01; /* 0x3FEEC709, 0xDC3A03FD =2/(3ln2) */
const CP_H: f64 = 9.61796700954437255859e-01; /* 0x3FEEC709, 0xE0000000 =(float)cp */
const CP_L: f64 = -7.02846165095275826516e-09; /* 0xBE3E2FE0, 0x145B01F5 =tail of cp_h*/
const IVLN2: f64 = 1.44269504088896338700e+00; /* 0x3FF71547, 0x652B82FE =1/ln2 */
const IVLN2_H: f64 = 1.44269502162933349609e+00; /* 0x3FF71547, 0x60000000 =24b 1/ln2*/
const IVLN2_L: f64 = 1.92596299112661746887e-08; /* 0x3E54AE0B, 0xF85DDF44 =1/ln2 tail*/

/// pow(x,y) return x**y
///
/// Method:  Let x =  2   * (1+f)
///   1. Compute and return log2(x) in two pieces:
///          log2(x) = w1 + w2,
///      where w1 has 53-24 = 29 bit trailing zeros.
///   2. Perform y*log2(x) = n+y' by simulating multi-precision
///      arithmetic, where |y'|<=0.5.
///   3. Return x**y = 2**n*exp(y'*log2)
///
/// Special cases:
///   1.  (anything) ** 0  is 1
///   2.  (anything) ** 1  is itself
///   3.  (anything) ** NAN is NAN
///   4.  NAN ** (anything except 0) is NAN
///   5.  +-(|x| > 1) **  +INF is +INF
///   6.  +-(|x| > 1) **  -INF is +0
///   7.  +-(|x| < 1) **  +INF is +0
///   8.  +-(|x| < 1) **  -INF is +INF
///   9.  +-1         ** +-INF is NAN
///   10. +0 ** (+anything except 0, NAN)               is +0
///   11. -0 ** (+anything except 0, NAN, odd integer)  is +0
///   12. +0 ** (-anything except 0, NAN)               is +INF
///   13. -0 ** (-anything except 0, NAN, odd integer)  is +INF
///   14. -0 ** (odd integer) = -( +0 ** (odd integer) )
///   15. +INF ** (+anything except 0,NAN) is +INF
///   16. +INF ** (-anything except 0,NAN) is +0
///   17. -INF ** (anything)  = -0 ** (-anything)
///   18. (-anything) ** (integer) is (-1)**(integer)*(+anything**integer)
///   19. (-anything except 0 and inf) ** (non-integer) is NAN
///
/// (Note: this uClibc version returns 1 for `1 ** NaN`, see below.)
///
/// Translation of `__ieee754_pow()` (`SDL_uclibc_pow`).
#[allow(clippy::many_single_char_names)]
pub(super) fn pow(x: f64, y: f64) -> f64 {
    let (hx, lx) = words(x);

    /* x==1: 1**y = 1 (even if y is NaN) */
    if hx == 0x3ff00000 && lx == 0 {
        return x;
    }
    let mut ix = hx & 0x7fffffff;

    let (hy, ly) = words(y);
    let iy = hy & 0x7fffffff;

    /* y==zero: x**0 = 1 */
    if (iy as u32 | ly) == 0 {
        return ONE;
    }

    /* +-NaN return x+y */
    if ix > 0x7ff00000
        || (ix == 0x7ff00000 && lx != 0)
        || iy > 0x7ff00000
        || (iy == 0x7ff00000 && ly != 0)
    {
        return x + y;
    }

    /* determine if y is an odd int when x < 0
     * yisint = 0   ... y is not an integer
     * yisint = 1   ... y is an odd int
     * yisint = 2   ... y is an even int
     */
    let mut yisint: i32 = 0;
    if hx < 0 {
        if iy >= 0x43400000 {
            yisint = 2; /* even integer y */
        } else if iy >= 0x3ff00000 {
            let k = (iy >> 20) - 0x3ff; /* exponent */
            if k > 20 {
                let j = (ly >> (52 - k)) as i32;
                if ((j as u32) << (52 - k)) == ly {
                    yisint = 2 - (j & 1);
                }
            } else if ly == 0 {
                let j = iy >> (20 - k);
                if (j << (20 - k)) == iy {
                    yisint = 2 - (j & 1);
                }
            }
        }
    }

    /* special value of y */
    if ly == 0 {
        if iy == 0x7ff00000 {
            /* y is +-inf */
            if (ix.wrapping_sub(0x3ff00000) as u32 | lx) == 0 {
                return ONE; /* +-1**+-inf is 1 (yes, weird rule) */
            }
            if ix >= 0x3ff00000 {
                /* (|x|>1)**+-inf = inf,0 */
                return if hy >= 0 { y } else { ZERO };
            }
            /* (|x|<1)**-,+inf = inf,0 */
            return if hy < 0 { -y } else { ZERO };
        }
        if iy == 0x3ff00000 {
            /* y is  +-1 */
            return if hy < 0 { ONE / x } else { x };
        }
        if hy == 0x40000000 {
            return x * x; /* y is  2 */
        }
        if hy == 0x3fe00000 {
            /* y is  0.5 */
            if hx >= 0 {
                /* x >= +0 */
                return sqrt(x);
            }
        }
    }

    let mut ax = fabs(x);
    /* special value of x */
    if lx == 0 && (ix == 0x7ff00000 || ix == 0 || ix == 0x3ff00000) {
        let mut z = ax; /*x is +-0,+-inf,+-1*/
        if hy < 0 {
            z = ONE / z; /* z = (1/|x|) */
        }
        if hx < 0 {
            if ((ix - 0x3ff00000) | yisint) == 0 {
                z = (z - z) / (z - z); /* (-1)**non-int is NaN */
            } else if yisint == 1 {
                z = -z; /* (x<0)**odd = -(|x|**odd) */
            }
        }
        return z;
    }

    /* (x<0)**(non-int) is NaN */
    if (((hx as u32) >> 31).wrapping_sub(1) | yisint as u32) == 0 {
        return (x - x) / (x - x);
    }

    let t1: f64;
    let t2: f64;

    /* |y| is huge */
    if iy > 0x41e00000 {
        /* if |y| > 2**31 */
        if iy > 0x43f00000 {
            /* if |y| > 2**64, must o/uflow */
            if ix <= 0x3fefffff {
                return if hy < 0 { HUGE * HUGE } else { TINY * TINY };
            }
            if ix >= 0x3ff00000 {
                return if hy > 0 { HUGE * HUGE } else { TINY * TINY };
            }
        }
        /* over/underflow if x is not close to one */
        if ix < 0x3fefffff {
            return if hy < 0 { HUGE * HUGE } else { TINY * TINY };
        }
        if ix > 0x3ff00000 {
            return if hy > 0 { HUGE * HUGE } else { TINY * TINY };
        }
        /* now |1-x| is tiny <= 2**-20, suffice to compute
        log(x) by x-x^2/2+x^3/3-x^4/4 */
        let t = x - 1.0; /* t has 20 trailing zeros */
        let w = (t * t) * (0.5 - t * (0.3333333333333333333333 - t * 0.25));
        let u = IVLN2_H * t; /* ivln2_h has 21 sig. bits */
        let v = t * IVLN2_L - w * IVLN2;
        let mut tt1 = u + v;
        tt1 = with_lo(tt1, 0);
        t1 = tt1;
        t2 = v - (t1 - u);
    } else {
        let mut n: i32 = 0;
        /* take care subnormal number */
        if ix < 0x00100000 {
            ax *= TWO53;
            n -= 53;
            ix = hi(ax) as i32;
        }
        n += (ix >> 20) - 0x3ff;
        let j = ix & 0x000fffff;
        /* determine interval */
        ix = j | 0x3ff00000; /* normalize ix */
        let k: usize;
        if j <= 0x3988E {
            k = 0; /* |x|<sqrt(3/2) */
        } else if j < 0xBB67A {
            k = 1; /* |x|<sqrt(3)   */
        } else {
            k = 0;
            n += 1;
            ix -= 0x00100000;
        }
        ax = with_hi(ax, ix as u32);

        /* compute s = s_h+s_l = (x-1)/(x+1) or (x-1.5)/(x+1.5) */
        let mut u = ax - BP[k]; /* bp[0]=1.0, bp[1]=1.5 */
        let mut v = ONE / (ax + BP[k]);
        let s = u * v;
        let s_h = with_lo(s, 0);
        /* t_h=ax+bp[k] High */
        let mut t_h = from_words(
            (((ix >> 1) | 0x20000000) + 0x00080000 + ((k as i32) << 18)) as u32,
            0,
        );
        let mut t_l = ax - (t_h - BP[k]);
        let s_l = v * ((u - s_h * t_h) - s_h * t_l);
        /* compute log(ax) */
        let mut s2 = s * s;
        let mut r = s2 * s2 * (L1 + s2 * (L2 + s2 * (L3 + s2 * (L4 + s2 * (L5 + s2 * L6)))));
        r += s_l * (s_h + s);
        s2 = s_h * s_h;
        t_h = 3.0 + s2 + r;
        t_h = with_lo(t_h, 0);
        t_l = r - ((t_h - 3.0) - s2);
        /* u+v = s*(1+...) */
        u = s_h * t_h;
        v = s_l * t_h + t_l * s;
        /* 2/(3log2)*(s+...) */
        let mut p_h = u + v;
        p_h = with_lo(p_h, 0);
        let p_l = v - (p_h - u);
        let z_h = CP_H * p_h; /* cp_h+cp_l = 2/(3*log2) */
        let z_l = CP_L * p_h + p_l * CP + DP_L[k];
        /* log2(ax) = (s+..)*2/(3*log2) = n + dp_h + z_h + z_l */
        let t = n as f64;
        let mut tt1 = ((z_h + z_l) + DP_H[k]) + t;
        tt1 = with_lo(tt1, 0);
        t1 = tt1;
        t2 = z_l - (((t1 - t) - DP_H[k]) - z_h);
    }

    let mut s = ONE; /* s (sign of result -ve**odd) = -1 else = 1 */
    if (((hx as u32) >> 31).wrapping_sub(1) | (yisint - 1) as u32) == 0 {
        s = -ONE; /* (-ve)**(odd int) */
    }

    /* split up y into y1+y2 and compute (y1+y2)*(t1+t2) */
    let y1 = with_lo(y, 0);
    let p_l = (y - y1) * t1 + y * t2;
    let mut p_h = y1 * t1;
    let mut z = p_l + p_h;
    let (mut j, i) = words(z);
    if j >= 0x40900000 {
        /* z >= 1024 */
        if ((j - 0x40900000) as u32 | i) != 0 {
            /* if z > 1024 */
            return s * HUGE * HUGE; /* overflow */
        } else if p_l + OVT > z - p_h {
            return s * HUGE * HUGE; /* overflow */
        }
    } else if (j & 0x7fffffff) >= 0x4090cc00 {
        /* z <= -1075 */
        if ((j as u32).wrapping_sub(0xc090cc00) | i) != 0 {
            /* z < -1075 */
            return s * TINY * TINY; /* underflow */
        } else if p_l <= z - p_h {
            return s * TINY * TINY; /* underflow */
        }
    }
    /*
     * compute 2**(p_h+p_l)
     */
    let i = j & 0x7fffffff;
    let mut k = (i >> 20) - 0x3ff;
    let mut n: i32 = 0;
    if i > 0x3fe00000 {
        /* if |z| > 0.5, set n = [z+0.5] */
        n = j + (0x00100000 >> (k + 1));
        k = ((n & 0x7fffffff) >> 20) - 0x3ff; /* new k for n */
        let t = from_words((n & !(0x000fffff >> k)) as u32, 0);
        n = ((n & 0x000fffff) | 0x00100000) >> (20 - k);
        if j < 0 {
            n = -n;
        }
        p_h -= t;
    }
    let mut t = p_l + p_h;
    t = with_lo(t, 0);
    let u = t * LG2_H;
    let v = (p_l - (t - p_h)) * LG2_ + t * LG2_L;
    z = u + v;
    let w = v - (z - u);
    t = z * z;
    let t1 = z - t * (P1 + t * (P2 + t * (P3 + t * (P4 + t * P5))));
    let r = (z * t1) / (t1 - TWO) - (w + z * w);
    z = ONE - (r - z);
    j = hi(z) as i32;
    j = j.wrapping_add(n << 20);
    if (j >> 20) <= 0 {
        z = scalbn(z, n); /* subnormal output */
    } else {
        z = with_hi(z, j as u32);
    }
    s * z
}
