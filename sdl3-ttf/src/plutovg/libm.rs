// The C library math functions PlutoVG and PlutoSVG call (cosf, sinf,
// tanf, atan2f, powf, hypotf and hypot), computed as the C library of the
// reference build (glibc 2.39 on x86-64, which runs its FMA variants on
// CPUs that have FMA) computes them, from permissively licensed sources:
//
// * sinf and cosf: translated from ARM's optimized-routines (math/sinf.c,
//   cosf.c, sincosf.h and sincosf_data.c), which glibc bundles;
//   Copyright (c) 2018-2024, Arm Limited; SPDX-License-Identifier: MIT OR
//   Apache-2.0 WITH LLVM-exception (used here under the MIT license, see
//   LICENSE.txt). The multiply-adds the FMA build fuses are fused here
//   (`fma`, below).
// * atanf, atan2f and tanf: translated from newlib's float versions of
//   fdlibm (libm/math/sf_atan.c, ef_atan2.c, sf_tan.c, kf_tan.c and
//   ef_rem_pio2.c, "Conversion to float by Ian Lance Taylor, Cygnus
//   Support"), which carry Sun's notice:
//
//   ====================================================
//   Copyright (C) 1993 by Sun Microsystems, Inc. All rights reserved.
//
//   Developed at SunPro, a Sun Microsystems, Inc. business.
//   Permission to use, copy, modify, and distribute this
//   software is freely granted, provided that this notice
//   is preserved.
//   ====================================================
//
// * hypot: Borges' corrected algorithm ("An Improved Algorithm for
//   hypot(a,b)", C. F. Borges, 2019), written for this crate; hypotf is
//   the square root of the sum of squares in double, powf of 10 and an
//   integer exponent its correctly rounded value, and `fma` an exact fused
//   multiply-add written for this crate.

//! The C library math functions of PlutoVG and PlutoSVG.
//!
//! sinf, cosf, atanf and atan2f give glibc's results for every input the
//! comparisons ran (20 million each); tanf gives them for |x| <= pi/4
//! except within about 2^-13 of pi/4, and for most larger arguments (not
//! all: glibc's float tangent differs from newlib's there); hypot gives
//! glibc's results for the scale factors of the stroker.

#![allow(clippy::excessive_precision, clippy::approx_constant)]

/* ---- fma -------------------------------------------------------------- */

/// The exact fused multiply-add `a * b + c`, rounded once (to nearest,
/// ties to even), for finite operands whose result is a normal number;
/// other cases (zeros, infinities, NaNs, results out of the normal range)
/// fall back to the unfused `a * b + c`.
pub fn fma(a: f64, b: f64, c: f64) -> f64 {
    if !a.is_finite() || !b.is_finite() || !c.is_finite() || a == 0.0 || b == 0.0 || c == 0.0 {
        return a * b + c;
    }

    /* sign, exponent of the least significant bit, integer mantissa */
    fn decompose(x: f64) -> (bool, i32, u64) {
        let bits = x.to_bits();
        let sign = bits >> 63 != 0;
        let e = ((bits >> 52) & 0x7ff) as i32;
        let f = bits & ((1u64 << 52) - 1);
        if e == 0 {
            (sign, -1074, f)
        } else {
            (sign, e - 1075, f | (1u64 << 52))
        }
    }

    let (sa, ea, ma) = decompose(a);
    let (sb, eb, mb) = decompose(b);
    let (sc, ec, mc) = decompose(c);
    if ma == 0 || mb == 0 || mc == 0 {
        return a * b + c;
    }

    /* the exact product, and c, normalized to bit 125 */
    let mut pm = ma as u128 * mb as u128;
    let mut pe = ea + eb;
    let ps = sa ^ sb;
    let sh = 125 - (127 - pm.leading_zeros() as i32);
    pm <<= sh;
    pe -= sh;
    let mut cm = mc as u128;
    let mut ce = ec;
    let sh = 125 - (127 - cm.leading_zeros() as i32);
    cm <<= sh;
    ce -= sh;

    /* align the smaller one, folding the bits shifted out into a sticky bit */
    fn shift_sticky(m: u128, d: i32) -> u128 {
        if d <= 0 {
            m
        } else if d >= 127 {
            (m != 0) as u128
        } else {
            let lost = m & ((1u128 << d) - 1);
            (m >> d) | (lost != 0) as u128
        }
    }
    let e = pe.max(ce);
    let pm = shift_sticky(pm, e - pe);
    let cm = shift_sticky(cm, e - ce);

    let (mut r, rs) = if ps == sc {
        (pm + cm, ps)
    } else if pm >= cm {
        (pm - cm, ps)
    } else {
        (cm - pm, sc)
    };
    if r == 0 {
        return 0.0;
    }

    /* round to 53 bits */
    let msb = 127 - r.leading_zeros() as i32;
    let mut exp = e;
    if msb > 52 {
        let d = msb - 52;
        let half = 1u128 << (d - 1);
        let rem = r & ((1u128 << d) - 1);
        r >>= d;
        exp += d;
        if rem > half || (rem == half && (r & 1) != 0) {
            r += 1;
            if r == 1u128 << 53 {
                r >>= 1;
                exp += 1;
            }
        }
    } else {
        r <<= 52 - msb;
        exp -= 52 - msb;
    }
    let biased = exp + 1075;
    if !(1..=2046).contains(&biased) {
        return a * b + c;
    }
    let bits = ((rs as u64) << 63) | ((biased as u64) << 52) | (r as u64 & ((1u64 << 52) - 1));
    f64::from_bits(bits)
}

/* ---- sinf and cosf (optimized-routines) -------------------------------- */

/// `sincos_t`: the constants and polynomials for sine and cosine.
struct SinCos {
    sign: [f64; 4], /* Sign of sine in quadrants 0..3.  */
    hpi_inv: f64,   /* 2 / PI ( * 2^24 if !TOINT_INTRINSICS).  */
    hpi: f64,       /* PI / 2.  */
    c0: f64,        /* Cosine polynomial.  */
    c1: f64,
    c2: f64,
    c3: f64,
    c4: f64,
    s1: f64, /* Sine polynomial.  */
    s2: f64,
    s3: f64,
}

/* 2PI * 2^-64.  */
const PI63: f64 = f64::from_bits(0x3C1921FB54442D18);
/* PI / 4.  */
const PIO4F: f32 = f32::from_bits(0x3F490FDB);

/// `__sincosf_table`: the constants and polynomials for sine and cosine.
/// The 2nd entry computes -cos (x) rather than cos (x) to get negation for
/// free.
static SINCOSF_TABLE: [SinCos; 2] = [
    SinCos {
        sign: [1.0, -1.0, -1.0, 1.0],
        hpi_inv: f64::from_bits(0x41645F306DC9C883),
        hpi: f64::from_bits(0x3FF921FB54442D18),
        c0: 1.0,
        c1: f64::from_bits(0xBFDFFFFFFD0C621C),
        c2: f64::from_bits(0x3FA55553E1068F19),
        c3: f64::from_bits(0xBF56C087E89A359D),
        c4: f64::from_bits(0x3EF99343027BF8C3),
        s1: f64::from_bits(0xBFC555545995A603),
        s2: f64::from_bits(0x3F81107605230BC4),
        s3: f64::from_bits(0xBF2994EB3774CF24),
    },
    SinCos {
        sign: [1.0, -1.0, -1.0, 1.0],
        hpi_inv: f64::from_bits(0x41645F306DC9C883),
        hpi: f64::from_bits(0x3FF921FB54442D18),
        c0: -1.0,
        c1: f64::from_bits(0x3FDFFFFFFD0C621C),
        c2: f64::from_bits(0xBFA55553E1068F19),
        c3: f64::from_bits(0x3F56C087E89A359D),
        c4: f64::from_bits(0xBEF99343027BF8C3),
        s1: f64::from_bits(0xBFC555545995A603),
        s2: f64::from_bits(0x3F81107605230BC4),
        s3: f64::from_bits(0xBF2994EB3774CF24),
    },
];

/// `__inv_pio4`: table with 4/PI to 192 bit precision.  To avoid unaligned
/// accesses only 8 new bits are added per entry, making the table 4 times
/// larger.
static INV_PIO4: [u32; 24] = [
    0xa2, 0xa2f9, 0xa2f983, 0xa2f9836e, 0xf9836e4e, 0x836e4e44, 0x6e4e4415, 0x4e441529, 0x441529fc,
    0x1529fc27, 0x29fc2757, 0xfc2757d1, 0x2757d1f5, 0x57d1f534, 0xd1f534dd, 0xf534ddc0, 0x34ddc0db,
    0xddc0db62, 0xc0db6295, 0xdb629599, 0x6295993c, 0x95993c43, 0x993c4390, 0x3c439041,
];

/// `abstop12`: top 12 bits of the float representation with the sign bit
/// cleared.
#[inline]
fn abstop12(x: f32) -> u32 {
    (x.to_bits() >> 20) & 0x7ff
}

/// `sinf_poly`: return the sine of inputs X and X2 (X squared) using the
/// polynomial P. N is the quadrant, and if odd the cosine polynomial is
/// used.
#[inline]
fn sinf_poly(x: f64, x2: f64, p: &SinCos, n: i32) -> f32 {
    if (n & 1) == 0 {
        let x3 = x * x2;
        let s1 = fma(x2, p.s3, p.s2);

        let x7 = x3 * x2;
        let s = fma(x3, p.s1, x);

        fma(x7, s1, s) as f32
    } else {
        let x4 = x2 * x2;
        let c2 = fma(x2, p.c4, p.c3);
        let c1 = fma(x2, p.c1, p.c0);

        let x6 = x4 * x2;
        let c = fma(x4, p.c2, c1);

        fma(x6, c2, c) as f32
    }
}

/// `reduce_fast`: fast range reduction using single multiply-subtract.
/// Return the modulo of X as a value between -PI/4 and PI/4 and store the
/// quadrant in NP. (Scaled float to int conversion with explicit rounding:
/// hpi_inv is prescaled by 2^24 so the quadrant ends up in bits 24..31.)
#[inline]
fn reduce_fast(x: f64, p: &SinCos, np: &mut i32) -> f64 {
    let r = x * p.hpi_inv;
    let n = (r as i32).wrapping_add(0x800000) >> 24;
    *np = n;
    fma(-(n as f64), p.hpi, x)
}

/// `reduce_large`: reduce the range of XI to a multiple of PI/2 using fast
/// integer arithmetic. XI is a reinterpreted float and must be >= 2.0f
/// (the sign bit is ignored). Return the modulo between -PI/4 and PI/4 and
/// store the quadrant in NP.
#[inline]
fn reduce_large(mut xi: u32, np: &mut i32) -> f64 {
    let arr = &INV_PIO4[((xi >> 26) & 15) as usize..];
    let shift = (xi >> 23) & 7;

    xi = (xi & 0xffffff) | 0x800000;
    xi <<= shift;

    let mut res0 = xi.wrapping_mul(arr[0]) as u64;
    let res1 = xi as u64 * arr[4] as u64;
    let res2 = xi as u64 * arr[8] as u64;
    res0 = (res2 >> 32) | (res0 << 32);
    res0 = res0.wrapping_add(res1);

    let n = res0.wrapping_add(1u64 << 61) >> 62;
    res0 = res0.wrapping_sub(n << 62);
    let x = res0 as i64 as f64;
    *np = n as i32;
    x * PI63
}

/// `sinf`
#[allow(clippy::eq_op)]
pub fn sinf(y: f32) -> f32 {
    let mut x = y as f64;
    let s;
    let mut n = 0;
    let mut p = &SINCOSF_TABLE[0];

    if abstop12(y) < abstop12(PIO4F) {
        s = x * x;

        if abstop12(y) < abstop12(f32::from_bits(0x39800000) /* 0x1p-12f */) {
            return y;
        }

        sinf_poly(x, s, p, 0)
    } else if abstop12(y) < abstop12(120.0) {
        x = reduce_fast(x, p, &mut n);

        /* Setup the signs for sin and cos.  */
        let s = p.sign[(n & 3) as usize];

        if n & 2 != 0 {
            p = &SINCOSF_TABLE[1];
        }

        sinf_poly(x * s, x * x, p, n)
    } else if abstop12(y) < abstop12(f32::INFINITY) {
        let xi = y.to_bits();
        let sign = (xi >> 31) as i32;

        x = reduce_large(xi, &mut n);

        /* Setup signs for sin and cos - include original sign.  */
        let s = p.sign[((n + sign) & 3) as usize];

        if (n + sign) & 2 != 0 {
            p = &SINCOSF_TABLE[1];
        }

        sinf_poly(x * s, x * x, p, n)
    } else {
        (y - y) / (y - y)
    }
}

/// `cosf`
#[allow(clippy::eq_op)]
pub fn cosf(y: f32) -> f32 {
    let mut x = y as f64;
    let mut n = 0;
    let mut p = &SINCOSF_TABLE[0];

    if abstop12(y) < abstop12(PIO4F) {
        let x2 = x * x;

        if abstop12(y) < abstop12(f32::from_bits(0x39800000) /* 0x1p-12f */) {
            return 1.0;
        }

        sinf_poly(x, x2, p, 1)
    } else if abstop12(y) < abstop12(120.0) {
        x = reduce_fast(x, p, &mut n);

        /* Setup the signs for sin and cos.  */
        let s = p.sign[(n & 3) as usize];

        if n & 2 != 0 {
            p = &SINCOSF_TABLE[1];
        }

        sinf_poly(x * s, x * x, p, n ^ 1)
    } else if abstop12(y) < abstop12(f32::INFINITY) {
        let xi = y.to_bits();
        let sign = (xi >> 31) as i32;

        x = reduce_large(xi, &mut n);

        /* Setup signs for sin and cos - include original sign.  */
        let s = p.sign[((n + sign) & 3) as usize];

        if (n + sign) & 2 != 0 {
            p = &SINCOSF_TABLE[1];
        }

        sinf_poly(x * s, x * x, p, n ^ 1)
    } else {
        (y - y) / (y - y)
    }
}

/* ---- atanf and atan2f (newlib sf_atan.c, ef_atan2.c) ------------------- */

static ATANHI: [f32; 4] = [
    4.6364760399e-01, /* atan(0.5)hi 0x3eed6338 */
    7.8539812565e-01, /* atan(1.0)hi 0x3f490fda */
    9.8279368877e-01, /* atan(1.5)hi 0x3f7b985e */
    1.5707962513e+00, /* atan(inf)hi 0x3fc90fda */
];

static ATANLO: [f32; 4] = [
    5.0121582440e-09, /* atan(0.5)lo 0x31ac3769 */
    3.7748947079e-08, /* atan(1.0)lo 0x33222168 */
    3.4473217170e-08, /* atan(1.5)lo 0x33140fb4 */
    7.5497894159e-08, /* atan(inf)lo 0x33a22168 */
];

static AT: [f32; 11] = [
    3.3333334327e-01,  /* 0x3eaaaaaa */
    -2.0000000298e-01, /* 0xbe4ccccd */
    1.4285714924e-01,  /* 0x3e124925 */
    -1.1111110449e-01, /* 0xbde38e38 */
    9.0908870101e-02,  /* 0x3dba2e6e */
    -7.6918758452e-02, /* 0xbd9d8795 */
    6.6610731184e-02,  /* 0x3d886b35 */
    -5.8335702866e-02, /* 0xbd6ef16b */
    4.9768779427e-02,  /* 0x3d4bda59 */
    -3.6531571299e-02, /* 0xbd15a221 */
    1.6285819933e-02,  /* 0x3c8569d7 */
];

/// `atanf`
pub fn atanf(mut x: f32) -> f32 {
    const ONE: f32 = 1.0;
    const HUGE: f32 = 1.0e30;
    let id: i32;

    let hx = x.to_bits() as i32;
    let ix = hx & 0x7fffffff;
    if ix >= 0x50800000 {
        /* if |x| >= 2^34 */
        if ix > 0x7f800000 {
            return x + x; /* NaN */
        }
        if hx > 0 {
            return ATANHI[3] + ATANLO[3];
        } else {
            return -ATANHI[3] - ATANLO[3];
        }
    }
    if ix < 0x3ee00000 {
        /* |x| < 0.4375 */
        if ix < 0x31000000 {
            /* |x| < 2^-29 */
            if HUGE + x > ONE {
                return x; /* raise inexact */
            }
        }
        id = -1;
    } else {
        x = x.abs();
        if ix < 0x3f980000 {
            /* |x| < 1.1875 */
            if ix < 0x3f300000 {
                /* 7/16 <=|x|<11/16 */
                id = 0;
                x = (2.0 * x - ONE) / (2.0 + x);
            } else {
                /* 11/16<=|x|< 19/16 */
                id = 1;
                x = (x - ONE) / (x + ONE);
            }
        } else if ix < 0x401c0000 {
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
    let z = x * x;
    let w = z * z;
    /* break sum from i=0 to 10 aT[i]z**(i+1) into odd and even poly */
    let s1 = z * (AT[0] + w * (AT[2] + w * (AT[4] + w * (AT[6] + w * (AT[8] + w * AT[10])))));
    let s2 = w * (AT[1] + w * (AT[3] + w * (AT[5] + w * (AT[7] + w * AT[9]))));
    if id < 0 {
        x - x * (s1 + s2)
    } else {
        let id = id as usize;
        let z = ATANHI[id] - ((x * (s1 + s2) - ATANLO[id]) - x);
        if hx < 0 {
            -z
        } else {
            z
        }
    }
}

/// `__ieee754_atan2f` (`atan2f`)
pub fn atan2f(y: f32, x: f32) -> f32 {
    const TINY: f32 = 1.0e-30;
    const ZERO: f32 = 0.0;
    const PI_O_4: f32 = 7.8539818525e-01; /* 0x3f490fdb */
    const PI_O_2: f32 = 1.5707963705e+00; /* 0x3fc90fdb */
    const PI: f32 = 3.1415927410e+00; /* 0x40490fdb */
    const PI_LO: f32 = -8.7422776573e-08; /* 0xb3bbbd2e */

    let hx = x.to_bits() as i32;
    let ix = hx & 0x7fffffff;
    let hy = y.to_bits() as i32;
    let iy = hy & 0x7fffffff;
    if ix > 0x7f800000 || iy > 0x7f800000 {
        /* x or y is NaN */
        return x + y;
    }
    if hx == 0x3f800000 {
        return atanf(y); /* x=1.0 */
    }
    let m = ((hy >> 31) & 1) | ((hx >> 30) & 2); /* 2*sign(x)+sign(y) */

    /* when y = 0 */
    if iy == 0 {
        match m {
            0 | 1 => return y,      /* atan(+-0,+anything)=+-0 */
            2 => return PI + TINY,  /* atan(+0,-anything) = pi */
            _ => return -PI - TINY, /* atan(-0,-anything) =-pi */
        }
    }
    /* when x = 0 */
    if ix == 0 {
        return if hy < 0 {
            -PI_O_2 - TINY
        } else {
            PI_O_2 + TINY
        };
    }

    /* when x is INF */
    if ix == 0x7f800000 {
        if iy == 0x7f800000 {
            match m {
                0 => return PI_O_4 + TINY,        /* atan(+INF,+INF) */
                1 => return -PI_O_4 - TINY,       /* atan(-INF,+INF) */
                2 => return 3.0 * PI_O_4 + TINY,  /*atan(+INF,-INF)*/
                _ => return -3.0 * PI_O_4 - TINY, /*atan(-INF,-INF)*/
            }
        } else {
            match m {
                0 => return ZERO,       /* atan(+...,+INF) */
                1 => return -ZERO,      /* atan(-...,+INF) */
                2 => return PI + TINY,  /* atan(+...,-INF) */
                _ => return -PI - TINY, /* atan(-...,-INF) */
            }
        }
    }
    /* when y is INF */
    if iy == 0x7f800000 {
        return if hy < 0 {
            -PI_O_2 - TINY
        } else {
            PI_O_2 + TINY
        };
    }

    /* compute y/x */
    let k = (iy - ix) >> 23;
    let z = if k > 60 {
        PI_O_2 + 0.5 * PI_LO /* |y/x| >  2**60 */
    } else if hx < 0 && k < -60 {
        0.0 /* |y|/x < -2**60 */
    } else {
        atanf((y / x).abs()) /* safe to do y/x */
    };
    match m {
        0 => z,                                        /* atan(+,+) */
        1 => f32::from_bits(z.to_bits() ^ 0x80000000), /* atan(-,+) */
        2 => PI - (z - PI_LO),                         /* atan(+,-) */
        _ => (z - PI_LO) - PI,                         /* atan(-,-) */
    }
}

/* ---- tanf (newlib sf_tan.c, kf_tan.c, ef_rem_pio2.c) ------------------- */

static T: [f32; 13] = [
    3.3333334327e-01,  /* 0x3eaaaaab */
    1.3333334029e-01,  /* 0x3e088889 */
    5.3968254477e-02,  /* 0x3d5d0dd1 */
    2.1869488060e-02,  /* 0x3cb327a4 */
    8.8632395491e-03,  /* 0x3c11371f */
    3.5920790397e-03,  /* 0x3b6b6916 */
    1.4562094584e-03,  /* 0x3abede48 */
    5.8804126456e-04,  /* 0x3a1a26c8 */
    2.4646313977e-04,  /* 0x398137b9 */
    7.8179444245e-05,  /* 0x38a3f445 */
    7.1407252108e-05,  /* 0x3895c07a */
    -1.8558637748e-05, /* 0xb79bae5f */
    2.5907305826e-05,  /* 0x37d95384 */
];

/// `__kernel_tanf`
fn kernel_tanf(mut x: f32, mut y: f32, iy: i32) -> f32 {
    const ONE: f32 = 1.0000000000e+00; /* 0x3f800000 */
    const PIO4: f32 = 7.8539812565e-01; /* 0x3f490fda */
    const PIO4LO: f32 = 3.7748947079e-08; /* 0x33222168 */

    let hx = x.to_bits() as i32;
    let ix = hx & 0x7fffffff; /* high word of |x| */
    if ix < 0x31800000 {
        /* x < 2**-28 */
        if x as i32 == 0 {
            /* generate inexact */
            if (ix | (iy + 1)) == 0 {
                return ONE / x.abs();
            } else {
                return if iy == 1 { x } else { -ONE / x };
            }
        }
    }
    if ix >= 0x3f2ca140 {
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
    let s = z * x;
    r = y + z * (s * (r + v) + y);
    r += T[0] * s;
    let w = x + r;
    if ix >= 0x3f2ca140 {
        v = iy as f32;
        return (1 - ((hx >> 30) & 2)) as f32 * (v - 2.0 * (x - (w * w / (w + v) - r)));
    }
    if iy == 1 {
        w
    } else {
        /* if allow error up to 2 ulp,
        simply return -1.0/(x+r) here */
        /*  compute -1.0/(x+r) accurately */
        let z = f32::from_bits(w.to_bits() & 0xfffff000);
        let v = r - (z - x); /* z+v = r+x */
        let a = -1.0 / w; /* a = -1.0/w */
        let t = f32::from_bits(a.to_bits() & 0xfffff000);
        let s = 1.0 + t * z;
        t + a * (s + t * v)
    }
}

/// `npio2_hw`: like the array in e_rem_pio2.c, but the numbers are single
/// precision and the last 8 bits are forced to 0.
static NPIO2_HW: [i32; 32] = [
    0x3fc90f00, 0x40490f00, 0x4096cb00, 0x40c90f00, 0x40fb5300, 0x4116cb00, 0x412fed00, 0x41490f00,
    0x41623100, 0x417b5300, 0x418a3a00, 0x4196cb00, 0x41a35c00, 0x41afed00, 0x41bc7e00, 0x41c90f00,
    0x41d5a000, 0x41e23100, 0x41eec200, 0x41fb5300, 0x4203f200, 0x420a3a00, 0x42108300, 0x4216cb00,
    0x421d1400, 0x42235c00, 0x4229a500, 0x422fed00, 0x42363600, 0x423c7e00, 0x4242c700, 0x42490f00,
];

/// `__ieee754_rem_pio2f`, for |x| <= 2^7*(pi/2); `None` for larger
/// arguments (`__kernel_rem_pio2f` is not translated: see `tanf`).
fn rem_pio2f(x: f32, y: &mut [f32; 2]) -> Option<i32> {
    const HALF: f32 = 5.0000000000e-01; /* 0x3f000000 */
    const INVPIO2: f32 = 6.3661980629e-01; /* 0x3f22f984 */
    const PIO2_1: f32 = 1.5707855225e+00; /* 0x3fc90f80 */
    const PIO2_1T: f32 = 1.0804334124e-05; /* 0x37354443 */
    const PIO2_2: f32 = 1.0804273188e-05; /* 0x37354400 */
    const PIO2_2T: f32 = 6.0770999344e-11; /* 0x2e85a308 */
    const PIO2_3: f32 = 6.0770943833e-11; /* 0x2e85a300 */
    const PIO2_3T: f32 = 6.1232342629e-17; /* 0x248d3132 */

    let hx = x.to_bits() as i32;
    let ix = hx & 0x7fffffff;
    if ix <= 0x3f490fd8 {
        /* |x| ~<= pi/4 , no need for reduction */
        y[0] = x;
        y[1] = 0.0;
        return Some(0);
    }
    if ix < 0x4016cbe4 {
        /* |x| < 3pi/4, special case with n=+-1 */
        if hx > 0 {
            let mut z = x - PIO2_1;
            if (ix & 0xfffffff0u32 as i32) != 0x3fc90fd0 {
                /* 24+24 bit pi OK */
                y[0] = z - PIO2_1T;
                y[1] = (z - y[0]) - PIO2_1T;
            } else {
                /* near pi/2, use 24+24+24 bit pi */
                z -= PIO2_2;
                y[0] = z - PIO2_2T;
                y[1] = (z - y[0]) - PIO2_2T;
            }
            return Some(1);
        } else {
            /* negative x */
            let mut z = x + PIO2_1;
            if (ix & 0xfffffff0u32 as i32) != 0x3fc90fd0 {
                /* 24+24 bit pi OK */
                y[0] = z + PIO2_1T;
                y[1] = (z - y[0]) + PIO2_1T;
            } else {
                /* near pi/2, use 24+24+24 bit pi */
                z += PIO2_2;
                y[0] = z + PIO2_2T;
                y[1] = (z - y[0]) + PIO2_2T;
            }
            return Some(-1);
        }
    }
    if ix <= 0x43490f80 {
        /* |x| ~<= 2^7*(pi/2), medium size */
        let t = x.abs();
        let n = (t * INVPIO2 + HALF) as i32;
        let fn_ = n as f32;
        let mut r = t - fn_ * PIO2_1;
        let mut w = fn_ * PIO2_1T; /* 1st round good to 40 bit */
        if n < 32 && (ix & 0xffffff00u32 as i32) != NPIO2_HW[(n - 1) as usize] {
            y[0] = r - w; /* quick check no cancellation */
        } else {
            let j = ix >> 23;
            y[0] = r - w;
            let high = y[0].to_bits();
            let i = j - ((high >> 23) & 0xff) as i32;
            if i > 8 {
                /* 2nd iteration needed, good to 57 */
                let t = r;
                w = fn_ * PIO2_2;
                r = t - w;
                w = fn_ * PIO2_2T - ((t - r) - w);
                y[0] = r - w;
                let high = y[0].to_bits();
                let i = j - ((high >> 23) & 0xff) as i32;
                if i > 25 {
                    /* 3rd iteration need, 74 bits acc */
                    let t = r; /* will cover all possible cases */
                    w = fn_ * PIO2_3;
                    r = t - w;
                    w = fn_ * PIO2_3T - ((t - r) - w);
                    y[0] = r - w;
                }
            }
        }
        y[1] = (r - y[0]) - w;
        if hx < 0 {
            y[0] = -y[0];
            y[1] = -y[1];
            return Some(-n);
        } else {
            return Some(n);
        }
    }
    None
}

/// `tanf`. The arguments beyond 2^7*(pi/2), whose reduction newlib does
/// with `__kernel_rem_pio2f`, are computed in double (SDL's fdlibm) and
/// rounded instead.
#[allow(clippy::eq_op)]
pub fn tanf(x: f32) -> f32 {
    let mut y = [0.0f32; 2];
    let z = 0.0;

    let ix = x.to_bits() as i32 & 0x7fffffff;

    /* |x| ~< pi/4 */
    if ix <= 0x3f490fda {
        kernel_tanf(x, z, 1)
    }
    /* tan(Inf or NaN) is NaN */
    else if ix >= 0x7f800000 {
        x - x /* NaN */
    }
    /* argument reduction needed */
    else {
        match rem_pio2f(x, &mut y) {
            Some(n) => kernel_tanf(y[0], y[1], 1 - ((n & 1) << 1)), /*   1 -- n even
            -1 -- n odd */
            None => sdl3::stdlib::math::tan(x as f64) as f32,
        }
    }
}

/* ---- powf, hypotf, hypot ------------------------------------------------ */

/// `powf`, as PlutoVG and PlutoSVG call it: for 10 and an integer
/// exponent (the exponents of numbers in SVG data), the correctly rounded
/// power of ten (glibc's result); other arguments are computed in double
/// (SDL's fdlibm) and rounded.
pub fn powf(x: f32, y: f32) -> f32 {
    if x == 10.0 && y == y.trunc() && y.is_finite() {
        if y > 38.0 {
            return f32::INFINITY;
        }
        if y < -46.0 {
            return 0.0;
        }
        let k = y as i32;
        /* 10^k is exact in double for 0 <= k <= 22; otherwise compute it
        as a correctly rounded float from an exact decimal string */
        let s = format!("1e{k}");
        return s.parse::<f32>().unwrap_or(0.0);
    }
    sdl3::stdlib::math::pow(x as f64, y as f64) as f32
}

/// `hypotf`: the square root of the sum of the squares in double.
pub fn hypotf(x: f32, y: f32) -> f32 {
    let (x, y) = (x as f64, y as f64);
    (x * x + y * y).sqrt() as f32
}

/// `hypot`: Borges' corrected algorithm (without fused multiply-adds),
/// for finite arguments without overflow; infinities and NaNs as C's.
pub fn hypot(x: f64, y: f64) -> f64 {
    if x.is_infinite() || y.is_infinite() {
        return f64::INFINITY;
    }
    if x.is_nan() || y.is_nan() {
        return x + y;
    }
    let (mut ax, mut ay) = (x.abs(), y.abs());
    if ax < ay {
        std::mem::swap(&mut ax, &mut ay);
    }
    if ay == 0.0 {
        return ax;
    }
    let mut h = (ax * ax + ay * ay).sqrt();
    let (t1, t2);
    if h <= 2.0 * ay {
        let delta = h - ay;
        t1 = ax * (2.0 * delta - ax);
        t2 = (delta - 2.0 * (ax - ay)) * delta;
    } else {
        let delta = h - ax;
        t1 = 2.0 * delta * (ax - 2.0 * ay);
        t2 = (4.0 * delta - ay) * ay + delta * delta;
    }
    h -= (t1 + t2) / (2.0 * h);
    h
}
