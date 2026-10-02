// Rust translation of src/SDL_utils.c from Simple DirectMedia Layer.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! Small numeric helpers shared across subsystems.

use std::sync::atomic::{AtomicU32, Ordering};

/// Translation of `SDL_CalculateGCD()`.
pub fn gcd(a: u32, b: u32) -> u32 {
    if b == 0 {
        return a;
    }
    gcd(b, a % b)
}

/// Best rational approximation of `x` with numerator and denominator ≤ 1000,
/// returned as `(numerator, denominator)`.
///
/// Algorithm adapted with thanks from John Cook's blog post:
/// <http://www.johndcook.com/blog/2010/10/20/best-rational-approximation>
///
/// Translation of `SDL_CalculateFraction()`. SDL uses this to express
/// refresh rates and pixel densities as fractions.
pub fn approximate_fraction(x: f32) -> (i32, i32) {
    const N: i32 = 1000;
    let (mut a, mut b) = (0i32, 1i32);
    let (mut c, mut d) = (1i32, 0i32);

    while b <= N && d <= N {
        let mediant = (a + c) as f32 / (b + d) as f32;
        if x == mediant {
            if b + d <= N {
                return (a + c, b + d);
            } else if d > b {
                return (c, d);
            } else {
                return (a, b);
            }
        } else if x > mediant {
            a += c;
            b += d;
        } else {
            c += a;
            d += b;
        }
    }
    if b > N {
        (c, d)
    } else {
        (a, b)
    }
}

static LAST_OBJECT_ID: AtomicU32 = AtomicU32::new(0);

/// Next unique, non-zero object id (translation of `SDL_GetNextObjectID()`).
pub(crate) fn next_object_id() -> u32 {
    let mut id = LAST_OBJECT_ID
        .fetch_add(1, Ordering::Relaxed)
        .wrapping_add(1);
    if id == 0 {
        id = LAST_OBJECT_ID
            .fetch_add(1, Ordering::Relaxed)
            .wrapping_add(1);
    }
    id
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gcd_and_fraction() {
        assert_eq!(gcd(1_000_000_000, 1000), 1000);
        assert_eq!(gcd(12, 18), 6);
        assert_eq!(approximate_fraction(0.5), (1, 2));
        assert_eq!(approximate_fraction(0.75), (3, 4));
        let (n, d) = approximate_fraction(59.94);
        assert!(((n as f32 / d as f32) - 59.94).abs() < 0.01);
    }

    #[test]
    fn ids_unique_nonzero() {
        let a = next_object_id();
        let b = next_object_id();
        assert_ne!(a, 0);
        assert_ne!(a, b);
    }
}
