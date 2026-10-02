// Tests for the fdlibm translation.
//
// The hashes below were produced by compiling upstream's src/libm/*.c
// (SDL 3.5.0) with gcc 13 -O2 on x86-64 and running the same input
// generator (an LCG + mixer, with special values every 97th input) over
// 20000 inputs per function, FNV-1a hashing the output bits (any NaN hashed
// as the canonical NaN; -O0 and -O2 builds of the C code agree on these).
// Matching them means the translation is bit-identical to the C code on
// every one of those inputs.

use super::*;

struct Gen {
    st: u64,
}

const SPECIALS: [u64; 20] = [
    0x0,
    0x8000000000000000,
    0x7ff0000000000000,
    0xfff0000000000000,
    0x7ff8000000000000,
    0x7ff4000000000001,
    0x3ff0000000000000,
    0xbff0000000000000,
    0x3fe0000000000000,
    0x4000000000000000,
    0x0000000000000001,
    0x000fffffffffffff,
    0x7fefffffffffffff,
    0x400921fb54442d18,
    0x3ff921fb54442d18,
    0x3fe921fb54442d18,
    0x4086232bdd7abcd2,
    0xc0874910d52d3051,
    0x40862e42fefa39ef,
    0x4330000000000000,
];

impl Gen {
    fn new() -> Self {
        Gen {
            st: 0x853c49e6748fea9b,
        }
    }
    fn next(&mut self) -> u64 {
        self.st = self
            .st
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        let mut x = self.st;
        x ^= x >> 33;
        x = x.wrapping_mul(0xff51afd7ed558ccd);
        x ^= x >> 33;
        x
    }
    fn gen(&mut self, i: i32) -> f64 {
        let r = self.next();
        if i % 97 == 0 {
            return f64::from_bits(SPECIALS[(r % SPECIALS.len() as u64) as usize]);
        }
        let unit = ((r >> 11) as i64) as f64 / 9007199254740992.0;
        match i % 6 {
            0 => f64::from_bits(r),
            1 => unit * 20.0 - 10.0,
            2 => unit * 2e6 - 1e6,
            3 => unit * 4.0 - 2.0,
            4 => f64::from_bits((r & 0x800fffffffffffff) | ((0x3c0 + (r >> 52) % 0x80) << 52)),
            _ => f64::from_bits(r & 0x800fffffffffffff),
        }
    }
}

struct Fnv(u64);
impl Fnv {
    fn mix(&mut self, mut v: u64) {
        // NaN payloads are not part of the comparison: neither C nor Rust
        // compilers preserve them through `x * 1.0` (gcc and LLVM fold it at
        // -O2 and keep it at -O0), so every NaN hashes as the canonical one.
        if ((v >> 52) & 0x7ff) == 0x7ff && (v & 0xfffffffffffff) != 0 {
            v = 0x7ff8000000000000;
        }
        for k in 0..8 {
            self.0 ^= (v >> (8 * k)) & 0xff;
            self.0 = self.0.wrapping_mul(0x100000001b3);
        }
    }
}

const N: i32 = 20000;

fn hash_one(f: impl Fn(&mut Gen, &mut Fnv, f64) -> f64) -> u64 {
    let mut g = Gen::new();
    let mut h = Fnv(0xcbf29ce484222325);
    for i in 0..N {
        let x = g.gen(i);
        let r = f(&mut g, &mut h, x);
        h.mix(r.to_bits());
    }
    h.0
}

fn hash_two(f: impl Fn(f64, f64) -> f64) -> u64 {
    let mut g = Gen::new();
    let mut h = Fnv(0xcbf29ce484222325);
    for i in 0..N {
        let x = g.gen(i);
        let y = g.gen(i + 1);
        h.mix(f(x, y).to_bits());
    }
    h.0
}

#[test]
fn bit_identical_to_upstream_libm() {
    let checks: [(&str, u64, u64); 16] = [
        ("sin", hash_one(|_, _, x| trig::sin(x)), 0x6131dd3c18670920),
        ("cos", hash_one(|_, _, x| trig::cos(x)), 0xbddee5b5d5805e4b),
        ("tan", hash_one(|_, _, x| trig::tan(x)), 0x6d56a4dc90d359d7),
        (
            "atan",
            hash_one(|_, _, x| trig::atan(x)),
            0xbdfdbb99ff464548,
        ),
        ("atan2", hash_two(trig::atan2), 0x4fcf764323daee25),
        (
            "exp",
            hash_one(|_, _, x| exp_log::exp(x)),
            0x07170f06e2282ff8,
        ),
        (
            "log",
            hash_one(|_, _, x| exp_log::log(x)),
            0x14ab08ea918a4960,
        ),
        (
            "log10",
            hash_one(|_, _, x| exp_log::log10(x)),
            0xd0a3ee6e17d32903,
        ),
        ("pow", hash_two(exp_log::pow), 0x094f50dcdc28f8b2),
        ("fmod", hash_two(misc::fmod), 0xa15a3f777dcaf829),
        (
            "sqrt",
            hash_one(|_, _, x| misc::sqrt(x)),
            0xefbc0e0ada140f63,
        ),
        (
            "floor",
            hash_one(|_, _, x| misc::floor(x)),
            0x2e6fcfd43007a547,
        ),
        (
            "scalbn",
            hash_one(|g, _, x| misc::scalbn(x, (g.next() % 4200) as i32 - 2100)),
            0x0174471e2abb10ed,
        ),
        (
            "modf",
            hash_one(|_, h, x| {
                let (f, ip) = misc::modf(x);
                h.mix(ip.to_bits());
                f
            }),
            0x774281b9c435e2e6,
        ),
        (
            "isinf",
            hash_one(|_, _, x| misc::isinf(x) as f64),
            0xc185eb7cd0bef065,
        ),
        (
            "isnan",
            hash_one(|_, _, x| misc::isnan(x) as f64),
            0x1f5198b622095605,
        ),
    ];
    let mismatches: Vec<_> = checks
        .iter()
        .filter(|(_, got, want)| got != want)
        .map(|(name, got, want)| format!("{name}: got {got:016x}, want {want:016x}"))
        .collect();
    assert!(mismatches.is_empty(), "{mismatches:#?}");
}

#[test]
fn spot_values() {
    assert_eq!(sin(1e22).to_bits(), 0xbfeb453ab76bf397);
    assert_eq!(cos(1e300).to_bits(), 0xbfe2699022adc4c1);
    assert_eq!(pow(2.0, 0.5).to_bits(), 0x3ff6a09e667f3bcd);
    assert_eq!(exp(1.0).to_bits(), 0x4005bf0a8b14576a);
    assert_eq!(log(10.0).to_bits(), 0x40026bb1bbb55516);
}

#[test]
fn constants_match_their_hex_comments() {
    // (Every fdlibm literal is covered by the bit-identity test above.)
    assert_eq!(PI_D.to_bits(), 0x400921FB54442D18);
    assert_eq!(PI_F.to_bits(), 0x40490FDB);
}

#[test]
fn front_end_semantics() {
    assert_eq!(ceil(1.2), 2.0);
    assert_eq!(ceil(-1.2), -1.0);
    assert_eq!(trunc(-1.7), -1.0);
    assert_eq!(round(2.5), 3.0);
    assert_eq!(round(-2.5), -3.0);
    assert_eq!(lround(-2.5), -3);
    assert_eq!(roundf(0.49999997), 0.0);
    assert_eq!(modf(-3.75), (-0.75, -3.0));
    assert_eq!(modff(2.5), (0.5, 2.0));
    assert!(isinf(f64::NEG_INFINITY) && !isinf(1.0));
    assert!(isinff(f32::INFINITY) && !isinff(f32::NAN));
    assert!(isnan(f64::NAN) && !isnan(f64::INFINITY));
    assert!(isnanf(f32::NAN) && !isnanf(0.0));
    assert_eq!(acos(-1.0), PI_D);
    assert_eq!(asin(-1.0), -PI_D / 2.0);
    assert!((acos(0.5) - std::f64::consts::FRAC_PI_3).abs() < 1e-15);
    assert!((asin(0.5) - std::f64::consts::FRAC_PI_6).abs() < 1e-15);
    assert_eq!(copysign(3.0, -0.0), -3.0);
    assert_eq!(fabs(-0.0).to_bits(), 0);
    assert_eq!(scalbn(1.0, -1074), f64::from_bits(1));
    assert_eq!(sqrt(2.0), 2f64.sqrt());
    assert_eq!(sqrtf(2.0), 2f32.sqrt());
    assert!((sinf(1.0) - 1f32.sin()).abs() <= f32::EPSILON);
    assert_eq!(fmod(-7.5, 2.0), -1.5);
    assert_eq!(atan2f(1.0, 1.0), std::f32::consts::FRAC_PI_4);
}
