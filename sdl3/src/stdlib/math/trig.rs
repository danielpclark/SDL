// Rust translation of src/libm/s_sin.c, s_cos.c, s_tan.c, s_atan.c and
// e_atan2.c from Simple DirectMedia Layer.
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

//! sin, cos, tan, atan, atan2.
//!
//! Method (sin/cos/tan).
//!     Let S,C and T denote the sin, cos and tan respectively on
//!     [-PI/4, +PI/4]. Reduce the argument x to y1+y2 = x-k*pi/2
//!     in [-pi/4 , +pi/4], and let n = k mod 4.
//!     We have
//!
//! ```text
//!          n        sin(x)      cos(x)        tan(x)
//!     ----------------------------------------------------------
//!          0          S           C             T
//!          1          C          -S            -1/T
//!          2         -S          -C             T
//!          3         -C           S            -1/T
//!     ----------------------------------------------------------
//! ```
//!
//! Special cases:
//!     Let trig be any of sin, cos, or tan.
//!     trig(+-INF)  is NaN, with signals;
//!     trig(NaN)    is that NaN;
//!
//! Accuracy:
//!     TRIG(x) returns trig(x) nearly rounded

use super::kernels::{kernel_cos, kernel_sin, kernel_tan};
use super::misc::fabs;
use super::rem_pio2::ieee754_rem_pio2;
use super::words::{hi, lo, with_hi, words};

/// Return sine function of x. Translation of fdlibm `sin()` (`SDL_uclibc_sin`).
pub(super) fn sin(x: f64) -> f64 {
    let mut y = [0.0f64; 2];
    let z = 0.0;

    /* High word of x. */
    let ix = (hi(x) & 0x7fffffff) as i32;

    /* |x| ~< pi/4 */
    if ix <= 0x3fe921fb {
        kernel_sin(x, z, 0)
    }
    /* sin(Inf or NaN) is NaN */
    else if ix >= 0x7ff00000 {
        x - x
    }
    /* argument reduction needed */
    else {
        let n = ieee754_rem_pio2(x, &mut y);
        match n & 3 {
            0 => kernel_sin(y[0], y[1], 1),
            1 => kernel_cos(y[0], y[1]),
            2 => -kernel_sin(y[0], y[1], 1),
            _ => -kernel_cos(y[0], y[1]),
        }
    }
}

/// Return cosine function of x. Translation of fdlibm `cos()` (`SDL_uclibc_cos`).
pub(super) fn cos(x: f64) -> f64 {
    let mut y = [0.0f64; 2];
    let z = 0.0;

    /* High word of x. */
    let ix = (hi(x) & 0x7fffffff) as i32;

    /* |x| ~< pi/4 */
    if ix <= 0x3fe921fb {
        kernel_cos(x, z)
    }
    /* cos(Inf or NaN) is NaN */
    else if ix >= 0x7ff00000 {
        x - x
    }
    /* argument reduction needed */
    else {
        let n = ieee754_rem_pio2(x, &mut y);
        match n & 3 {
            0 => kernel_cos(y[0], y[1]),
            1 => -kernel_sin(y[0], y[1], 1),
            2 => -kernel_cos(y[0], y[1]),
            _ => kernel_sin(y[0], y[1], 1),
        }
    }
}

/// Return tangent function of x. Translation of fdlibm `tan()` (`SDL_uclibc_tan`).
pub(super) fn tan(x: f64) -> f64 {
    let mut y = [0.0f64; 2];
    let z = 0.0;

    /* High word of x. */
    let ix = (hi(x) & 0x7fffffff) as i32;

    /* |x| ~< pi/4 */
    if ix <= 0x3fe921fb {
        kernel_tan(x, z, 1)
    }
    /* tan(Inf or NaN) is NaN */
    else if ix >= 0x7ff00000 {
        x - x /* NaN */
    }
    /* argument reduction needed */
    else {
        let n = ieee754_rem_pio2(x, &mut y);
        kernel_tan(y[0], y[1], 1 - ((n & 1) << 1)) /*   1 -- n even
                                                   -1 -- n odd */
    }
}

static ATANHI: [f64; 4] = [
    4.63647609000806093515e-01, /* atan(0.5)hi 0x3FDDAC67, 0x0561BB4F */
    7.85398163397448278999e-01, /* atan(1.0)hi 0x3FE921FB, 0x54442D18 */
    9.82793723247329054082e-01, /* atan(1.5)hi 0x3FEF730B, 0xD281F69B */
    1.57079632679489655800e+00, /* atan(inf)hi 0x3FF921FB, 0x54442D18 */
];

static ATANLO: [f64; 4] = [
    2.26987774529616870924e-17, /* atan(0.5)lo 0x3C7A2B7F, 0x222F65E2 */
    3.06161699786838301793e-17, /* atan(1.0)lo 0x3C81A626, 0x33145C07 */
    1.39033110312309984516e-17, /* atan(1.5)lo 0x3C700788, 0x7AF0CBBD */
    6.12323399573676603587e-17, /* atan(inf)lo 0x3C91A626, 0x33145C07 */
];

static AT: [f64; 11] = [
    3.33333333333329318027e-01,  /* 0x3FD55555, 0x5555550D */
    -1.99999999998764832476e-01, /* 0xBFC99999, 0x9998EBC4 */
    1.42857142725034663711e-01,  /* 0x3FC24924, 0x920083FF */
    -1.11111104054623557880e-01, /* 0xBFBC71C6, 0xFE231671 */
    9.09088713343650656196e-02,  /* 0x3FB745CD, 0xC54C206E */
    -7.69187620504482999495e-02, /* 0xBFB3B0F2, 0xAF749A6D */
    6.66107313738753120669e-02,  /* 0x3FB10D66, 0xA0D03D51 */
    -5.83357013379057348645e-02, /* 0xBFADDE2D, 0x52DEFD9A */
    4.97687799461593236017e-02,  /* 0x3FA97B4B, 0x24760DEB */
    -3.65315727442169155270e-02, /* 0xBFA2B444, 0x2C6A6C2F */
    1.62858201153657823623e-02,  /* 0x3F90AD3A, 0xE322DA11 */
];

const ONE: f64 = 1.0;
const HUGE: f64 = 1.0e300;

/// atan(x)
/// Method
///   1. Reduce x to positive by atan(x) = -atan(-x).
///   2. According to the integer k=4t+0.25 chopped, t=x, the argument
///      is further reduced to one of the following intervals and the
///      arctangent of t is evaluated by the corresponding formula:
///
///      [0,7/16]      atan(x) = t-t^3*(a1+t^2*(a2+...(a10+t^2*a11)...)
///      [7/16,11/16]  atan(x) = atan(1/2) + atan( (t-0.5)/(1+t/2) )
///      [11/16.19/16] atan(x) = atan( 1 ) + atan( (t-1)/(1+t) )
///      [19/16,39/16] atan(x) = atan(3/2) + atan( (t-1.5)/(1+1.5t) )
///      [39/16,INF]   atan(x) = atan(INF) + atan( -1/t )
///
/// Translation of fdlibm `atan()` (`SDL_uclibc_atan`).
pub(super) fn atan(mut x: f64) -> f64 {
    let hx = hi(x) as i32;
    let ix = hx & 0x7fffffff;
    let id: i32;
    if ix >= 0x44100000 {
        /* if |x| >= 2^66 */
        let low = lo(x);
        if ix > 0x7ff00000 || (ix == 0x7ff00000 && low != 0) {
            return x + x; /* NaN */
        }
        if hx > 0 {
            return ATANHI[3] + ATANLO[3];
        } else {
            return -ATANHI[3] - ATANLO[3];
        }
    }
    if ix < 0x3fdc0000 {
        /* |x| < 0.4375 */
        if ix < 0x3e200000 {
            /* |x| < 2^-29 */
            if HUGE + x > ONE {
                return x; /* raise inexact */
            }
        }
        id = -1;
    } else {
        x = fabs(x);
        if ix < 0x3ff30000 {
            /* |x| < 1.1875 */
            if ix < 0x3fe60000 {
                /* 7/16 <=|x|<11/16 */
                id = 0;
                x = (2.0 * x - ONE) / (2.0 + x);
            } else {
                /* 11/16<=|x|< 19/16 */
                id = 1;
                x = (x - ONE) / (x + ONE);
            }
        } else if ix < 0x40038000 {
            /* |x| < 2.4375 */
            id = 2;
            x = (x - 1.5) / (ONE + 1.5 * x);
        } else {
            /* 2.4375 <= |x| < 2^66 */
            id = 3;
            x = -1.0 / x;
        }
    }
    /* end of argument reduction */
    let mut z = x * x;
    let w = z * z;
    /* break sum from i=0 to 10 aT[i]z**(i+1) into odd and even poly */
    let s1 = z * (AT[0] + w * (AT[2] + w * (AT[4] + w * (AT[6] + w * (AT[8] + w * AT[10])))));
    let s2 = w * (AT[1] + w * (AT[3] + w * (AT[5] + w * (AT[7] + w * AT[9]))));
    if id < 0 {
        x - x * (s1 + s2)
    } else {
        let id = id as usize;
        z = ATANHI[id] - ((x * (s1 + s2) - ATANLO[id]) - x);
        if hx < 0 {
            -z
        } else {
            z
        }
    }
}

const TINY: f64 = 1.0e-300;
const ZERO: f64 = 0.0;
const PI_O_4: f64 = 7.8539816339744827900E-01; /* 0x3FE921FB, 0x54442D18 */
const PI_O_2: f64 = 1.5707963267948965580E+00; /* 0x3FF921FB, 0x54442D18 */
const PI: f64 = 3.1415926535897931160E+00; /* 0x400921FB, 0x54442D18 */
const PI_LO: f64 = 1.2246467991473531772E-16; /* 0x3CA1A626, 0x33145C07 */

/// atan2(y,x)
/// Method :
///   1. Reduce y to positive by atan2(y,x)=-atan2(-y,x).
///   2. Reduce x to positive by (if x and y are unexceptional):
///      ARG (x+iy) = arctan(y/x)          ... if x > 0,
///      ARG (x+iy) = pi - arctan[y/(-x)]  ... if x < 0,
///
/// Special cases:
///
///   ATAN2((anything), NaN ) is NaN;
///   ATAN2(NAN , (anything) ) is NaN;
///   ATAN2(+-0, +(anything but NaN)) is +-0  ;
///   ATAN2(+-0, -(anything but NaN)) is +-pi ;
///   ATAN2(+-(anything but 0 and NaN), 0) is +-pi/2;
///   ATAN2(+-(anything but INF and NaN), +INF) is +-0 ;
///   ATAN2(+-(anything but INF and NaN), -INF) is +-pi;
///   ATAN2(+-INF,+INF ) is +-pi/4 ;
///   ATAN2(+-INF,-INF ) is +-3pi/4;
///   ATAN2(+-INF, (anything but,0,NaN, and INF)) is +-pi/2;
///
/// Translation of `__ieee754_atan2()` (`SDL_uclibc_atan2`).
pub(super) fn atan2(y: f64, x: f64) -> f64 {
    let (hx, lx) = words(x);
    let ix = hx & 0x7fffffff;
    let (hy, ly) = words(y);
    let iy = hy & 0x7fffffff;
    let nan_bits = |l: u32| (l | (l as i32).wrapping_neg() as u32) >> 31;
    if (ix as u32 | nan_bits(lx)) > 0x7ff00000 || (iy as u32 | nan_bits(ly)) > 0x7ff00000 {
        /* x or y is NaN */
        return x + y;
    }
    if ((hx.wrapping_sub(0x3ff00000)) as u32 | lx) == 0 {
        return atan(y); /* x=1.0 */
    }
    let m = ((hy >> 31) & 1) | ((hx >> 30) & 2); /* 2*sign(x)+sign(y) */

    /* when y = 0 */
    if (iy as u32 | ly) == 0 {
        match m {
            0 | 1 => return y,      /* atan(+-0,+anything)=+-0 */
            2 => return PI + TINY,  /* atan(+0,-anything) = pi */
            3 => return -PI - TINY, /* atan(-0,-anything) =-pi */
            _ => {}
        }
    }
    /* when x = 0 */
    if (ix as u32 | lx) == 0 {
        return if hy < 0 {
            -PI_O_2 - TINY
        } else {
            PI_O_2 + TINY
        };
    }

    /* when x is INF */
    if ix == 0x7ff00000 {
        if iy == 0x7ff00000 {
            match m {
                0 => return PI_O_4 + TINY,        /* atan(+INF,+INF) */
                1 => return -PI_O_4 - TINY,       /* atan(-INF,+INF) */
                2 => return 3.0 * PI_O_4 + TINY,  /*atan(+INF,-INF)*/
                3 => return -3.0 * PI_O_4 - TINY, /*atan(-INF,-INF)*/
                _ => {}
            }
        } else {
            match m {
                0 => return ZERO,       /* atan(+...,+INF) */
                1 => return -ZERO,      /* atan(-...,+INF) */
                2 => return PI + TINY,  /* atan(+...,-INF) */
                3 => return -PI - TINY, /* atan(-...,-INF) */
                _ => {}
            }
        }
    }
    /* when y is INF */
    if iy == 0x7ff00000 {
        return if hy < 0 {
            -PI_O_2 - TINY
        } else {
            PI_O_2 + TINY
        };
    }

    /* compute y/x */
    let k = (iy - ix) >> 20;
    let mut z = if k > 60 {
        PI_O_2 + 0.5 * PI_LO /* |y/x| >  2**60 */
    } else if hx < 0 && k < -60 {
        0.0 /* |y|/x < -2**60 */
    } else {
        atan(fabs(y / x)) /* safe to do y/x */
    };
    match m {
        0 => z, /* atan(+,+) */
        1 => {
            let zh = hi(z);
            z = with_hi(z, zh ^ 0x80000000);
            z /* atan(-,+) */
        }
        2 => PI - (z - PI_LO), /* atan(+,-) */
        _ => (z - PI_LO) - PI, /* atan(-,-) */
    }
}
