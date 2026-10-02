// Rust translation of src/libm/k_sin.c, k_cos.c and k_tan.c from Simple DirectMedia Layer.
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

//! fdlibm trigonometric kernels on [-pi/4, pi/4].

use super::misc::fabs;
use super::words::{from_words, hi, lo, with_lo};

const HALF: f64 = 5.00000000000000000000e-01; /* 0x3FE00000, 0x00000000 */
const S1: f64 = -1.66666666666666324348e-01; /* 0xBFC55555, 0x55555549 */
const S2: f64 = 8.33333333332248946124e-03; /* 0x3F811111, 0x1110F8A6 */
const S3: f64 = -1.98412698298579493134e-04; /* 0xBF2A01A0, 0x19C161D5 */
const S4: f64 = 2.75573137070700676789e-06; /* 0x3EC71DE3, 0x57B1FE7D */
const S5: f64 = -2.50507602534068634195e-08; /* 0xBE5AE5E6, 0x8A2B9CEB */
const S6: f64 = 1.58969099521155010221e-10; /* 0x3DE5D93A, 0x5ACFD57C */

/// __kernel_sin( x, y, iy)
/// kernel sin function on [-pi/4, pi/4], pi/4 ~ 0.7854
/// Input x is assumed to be bounded by ~pi/4 in magnitude.
/// Input y is the tail of x.
/// Input iy indicates whether y is 0. (if iy=0, y assume to be 0).
///
/// Algorithm
///  1. Since sin(-x) = -sin(x), we need only to consider positive x.
///  2. if x < 2^-27 (hx<0x3e400000 0), return x with inexact if x!=0.
///  3. sin(x) is approximated by a polynomial of degree 13 on [0,pi/4]
///     sin(x) ~ x + S1*x^3 + ... + S6*x^13, with
///     |sin(x)/x - (1+S1*x^2+S2*x^4+S3*x^6+S4*x^8+S5*x^10+S6*x^12)| <= 2^-58
///  4. sin(x+y) = sin(x) + sin'(x')*y ~ sin(x) + (1-x*x/2)*y.
///     For better accuracy, let r = x^3*(S2+x^2*(S3+x^2*(S4+x^2*(S5+x^2*S6))))
///     then sin(x) = x + (S1*x^3 + (x^2*(r-y/2)+y))
pub(super) fn kernel_sin(x: f64, y: f64, iy: i32) -> f64 {
    let ix = (hi(x) & 0x7fffffff) as i32; /* high word of x */
    if ix < 0x3e400000 {
        /* |x| < 2**-27 */
        if (x as i32) == 0 {
            return x; /* generate inexact */
        }
    }
    let z = x * x;
    let v = z * x;
    let r = S2 + z * (S3 + z * (S4 + z * (S5 + z * S6)));
    if iy == 0 {
        x + v * (S1 + z * r)
    } else {
        x - ((z * (HALF * y - v * r) - y) - v * S1)
    }
}

const ONE: f64 = 1.00000000000000000000e+00; /* 0x3FF00000, 0x00000000 */
const C1: f64 = 4.16666666666666019037e-02; /* 0x3FA55555, 0x5555554C */
const C2: f64 = -1.38888888888741095749e-03; /* 0xBF56C16C, 0x16C15177 */
const C3: f64 = 2.48015872894767294178e-05; /* 0x3EFA01A0, 0x19CB1590 */
const C4: f64 = -2.75573143513906633035e-07; /* 0xBE927E4F, 0x809C52AD */
const C5: f64 = 2.08757232129817482790e-09; /* 0x3E21EE9E, 0xBDB4B1C4 */
const C6: f64 = -1.13596475577881948265e-11; /* 0xBDA8FAE9, 0xBE8838D4 */

/// __kernel_cos( x,  y )
/// kernel cos function on [-pi/4, pi/4], pi/4 ~ 0.785398164
/// Input x is assumed to be bounded by ~pi/4 in magnitude.
/// Input y is the tail of x.
///
/// Algorithm
///  1. Since cos(-x) = cos(x), we need only to consider positive x.
///  2. if x < 2^-27 (hx<0x3e400000 0), return 1 with inexact if x!=0.
///  3. cos(x) is approximated by a polynomial of degree 14 on [0,pi/4]
///     cos(x) ~ 1 - x*x/2 + C1*x^4 + ... + C6*x^14 (remez error <= 2^-58)
///  4. let r = C1*x^4+C2*x^6+C3*x^8+C4*x^10+C5*x^12+C6*x^14, then
///     cos(x) = 1 - x*x/2 + r; since cos(x+y) ~ cos(x) - sin(x)*y ~ cos(x) - x*y,
///     a correction term is necessary in cos(x) and hence
///     cos(x+y) = 1 - (x*x/2 - (r - x*y))
///     For better accuracy when x > 0.3, let qx = |x|/4 with
///     the last 32 bits mask off, and if x > 0.78125, let qx = 0.28125.
///     Then cos(x+y) = (1-qx) - ((x*x/2-qx) - (r-x*y)).
///     Note that 1-qx and (x*x/2-qx) is EXACT here, and the
///     magnitude of the latter is at least a quarter of x*x/2,
///     thus, reducing the rounding error in the subtraction.
pub(super) fn kernel_cos(x: f64, y: f64) -> f64 {
    let ix = (hi(x) & 0x7fffffff) as i32; /* ix = |x|'s high word*/
    if ix < 0x3e400000 {
        /* if x < 2**27 */
        if (x as i32) == 0 {
            return ONE; /* generate inexact */
        }
    }
    let z = x * x;
    let r = z * (C1 + z * (C2 + z * (C3 + z * (C4 + z * (C5 + z * C6)))));
    if ix < 0x3FD33333 {
        /* if |x| < 0.3 */
        ONE - (0.5 * z - (z * r - x * y))
    } else {
        let qx = if ix > 0x3fe90000 {
            /* x > 0.78125 */
            0.28125
        } else {
            from_words((ix - 0x00200000) as u32, 0) /* x/4 */
        };
        let hz = 0.5 * z - qx;
        let a = ONE - qx;
        a - (hz - (z * r - x * y))
    }
}

const PIO4: f64 = 7.85398163397448278999e-01; /* 0x3FE921FB, 0x54442D18 */
const PIO4LO: f64 = 3.06161699786838301793e-17; /* 0x3C81A626, 0x33145C07 */
const T: [f64; 13] = [
    3.33333333333334091986e-01,  /* 0x3FD55555, 0x55555563 */
    1.33333333333201242699e-01,  /* 0x3FC11111, 0x1110FE7A */
    5.39682539762260521377e-02,  /* 0x3FABA1BA, 0x1BB341FE */
    2.18694882948595424599e-02,  /* 0x3F9664F4, 0x8406D637 */
    8.86323982359930005737e-03,  /* 0x3F8226E3, 0xE96E8493 */
    3.59207910759131235356e-03,  /* 0x3F6D6D22, 0xC9560328 */
    1.45620945432529025516e-03,  /* 0x3F57DBC8, 0xFEE08315 */
    5.88041240820264096874e-04,  /* 0x3F4344D8, 0xF2F26501 */
    2.46463134818469906812e-04,  /* 0x3F3026F7, 0x1A8D1068 */
    7.81794442939557092300e-05,  /* 0x3F147E88, 0xA03792A6 */
    7.14072491382608190305e-05,  /* 0x3F12B80F, 0x32F0A7E9 */
    -1.85586374855275456654e-05, /* 0xBEF375CB, 0xDB605373 */
    2.59073051863633712884e-05,  /* 0x3EFB2A70, 0x74BF7AD4 */
];

/// __kernel_tan( x, y, k )
/// kernel tan function on [-pi/4, pi/4], pi/4 ~ 0.7854
/// Input x is assumed to be bounded by ~pi/4 in magnitude.
/// Input y is the tail of x.
/// Input k indicates whether tan (if k=1) or -1/tan (if k= -1) is returned.
///
/// Algorithm
///  1. Since tan(-x) = -tan(x), we need only to consider positive x.
///  2. if x < 2^-28 (hx<0x3e300000 0), return x with inexact if x!=0.
///  3. tan(x) is approximated by a odd polynomial of degree 27 on [0,0.67434]
///     tan(x) ~ x + T1*x^3 + ... + T13*x^27, error <= 2^-59.2.
///     Note: tan(x+y) = tan(x) + tan'(x)*y ~ tan(x) + (1+x*x)*y
///     Therefore, for better accuracy in computing tan(x+y), let
///     r = x^3*(T2+x^2*(T3+x^2*(...+x^2*(T12+x^2*T13))))
///     then tan(x+y) = x + (T1*x^3 + (x^2*(r+y)+y))
///  4. For x in [0.67434,pi/4],  let y = pi/4 - x, then
///     tan(x) = tan(pi/4-y) = (1-tan(y))/(1+tan(y))
///            = 1 - 2*(tan(y) - (tan(y)^2)/(1+tan(y)))
pub(super) fn kernel_tan(mut x: f64, mut y: f64, iy: i32) -> f64 {
    let hx = hi(x) as i32;
    let ix = hx & 0x7fffffff; /* high word of |x| */
    if ix < 0x3e300000 {
        /* x < 2**-28 */
        if (x as i32) == 0 {
            /* generate inexact */
            let low = lo(x);
            if ((ix as u32 | low) | (iy + 1) as u32) == 0 {
                return ONE / fabs(x);
            } else {
                return if iy == 1 { x } else { -ONE / x };
            }
        }
    }
    if ix >= 0x3FE59428 {
        /* |x|>=0.6744 */
        if hx < 0 {
            x = -x;
            y = -y;
        }
        let z = PIO4 - x;
        let w = PIO4LO - y;
        x = z + w;
        y = 0.0;
    }
    let z = x * x;
    let w = z * z;
    /* Break x^5*(T[1]+x^2*T[2]+...) into
     *    x^5(T[1]+x^4*T[3]+...+x^20*T[11]) +
     *    x^5(x^2*(T[2]+x^4*T[4]+...+x^22*[T12]))
     */
    let mut r = T[1] + w * (T[3] + w * (T[5] + w * (T[7] + w * (T[9] + w * T[11]))));
    let mut v = z * (T[2] + w * (T[4] + w * (T[6] + w * (T[8] + w * (T[10] + w * T[12])))));
    let mut s = z * x;
    r = y + z * (s * (r + v) + y);
    r += T[0] * s;
    let w = x + r;
    if ix >= 0x3FE59428 {
        v = iy as f64;
        return (1 - ((hx >> 30) & 2)) as f64 * (v - 2.0 * (x - (w * w / (w + v) - r)));
    }
    if iy == 1 {
        w
    } else {
        /* if allow error up to 2 ulp,
        simply return -1.0/(x+r) here */
        /*  compute -1.0/(x+r) accurately */
        let z = with_lo(w, 0);
        v = r - (z - x); /* z+v = r+x */
        let a = -1.0 / w; /* a = -1.0/w */
        let t = with_lo(a, 0);
        s = 1.0 + t * z;
        t + a * (s + t * v)
    }
}
