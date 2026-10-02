// Rust translation of src/stdlib/SDL_random.c from Simple DirectMedia Layer.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! SDL's portable pseudo-random number generator.
//!
//! This is a small, fast linear congruential generator. It is **not**
//! cryptographically secure. [`Rng`] is an explicit-state generator
//! (translation of the `SDL_rand_*_r()` family); the module-level functions
//! use a process-global instance (`SDL_rand()`, `SDL_randf()`, ...).

use std::sync::Mutex;

/// An explicit-state SDL random number generator. Translation of the
/// `Uint64 *state` used by `SDL_rand_r()` and friends.
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub struct Rng {
    state: u64,
}

impl Rng {
    /// Create a generator from a seed. A seed of 0 is replaced by the current
    /// performance counter (as `SDL_srand(0)` does), so two generators created
    /// with `Rng::new(0)` differ.
    pub fn new(seed: u64) -> Self {
        let seed = if seed == 0 {
            crate::timer::performance_counter()
        } else {
            seed
        };
        Rng { state: seed }
    }

    /// Generate 32 pseudo-random bits. Translation of `SDL_rand_bits_r()`.
    pub fn next_u32(&mut self) -> u32 {
        // The C and A parameters of this LCG have been chosen based on hundreds
        // of core-hours of testing with PractRand and TestU01's Crush.
        // Using a 32-bit A improves performance on 32-bit architectures.
        // C can be any odd number, but < 256 generates smaller code on ARM32
        // These values perform as well as a full 64-bit implementation against
        // Crush and PractRand. Plus, their worst-case performance is better
        // than common 64-bit constants when tested against PractRand using seeds
        // with only a single bit set.

        // We tested all 32-bit and 33-bit A with all C < 256 from a v2 of:
        // Steele GL, Vigna S. Computationally easy, spectrally good multipliers
        // for congruential pseudorandom number generators.
        // Softw Pract Exper. 2022;52(2):443-458. doi: 10.1002/spe.3030
        // https://arxiv.org/abs/2001.05304v2
        self.state = self.state.wrapping_mul(0xff1cd035).wrapping_add(0x05);

        // Only return top 32 bits because they have a longer period
        (self.state >> 32) as u32
    }

    /// Generate a pseudo-random number in `0..n` for positive `n` (0 for
    /// `n <= 0`). Translation of `SDL_rand_r()`.
    ///
    /// The method used is faster and of better quality than `next_u32() % n`.
    /// Odds are roughly 99.9% even for `n = 1 million`; evenness is better
    /// for smaller `n`, and much worse as `n` gets bigger.
    pub fn below(&mut self, n: i32) -> i32 {
        // Algorithm: get 32 bits from rand_bits() and treat it as a 0.32 bit
        // fixed point number. Multiply by the 31.0 bit n to get a 31.32 bit
        // result. Shift right by 32 to get the 31 bit integer that we want.
        if n < 0 {
            // The algorithm looks like it works for numbers < 0 but it has an
            // infinitesimal chance of returning a value out of range.
            // Returning -rand(abs(n)) blows up at INT_MIN instead.
            // It's easier to just say no.
            return 0;
        }
        // On 32-bit arch, the compiler will optimize to a single 32-bit multiply
        let val = (self.next_u32() as u64) * (n as u64);
        (val >> 32) as i32
    }

    /// Generate a uniform pseudo-random `f32` in `0.0..1.0`. Translation of `SDL_randf_r()`.
    pub fn next_f32(&mut self) -> f32 {
        // Note: its using 24 bits because float has 23 bits significand + 1 implicit bit
        ((self.next_u32() >> (32 - 24)) as f32) * f32::from_bits(0x33800000) // 0x1p-24f
    }

    /// The raw generator state.
    pub fn state(&self) -> u64 {
        self.state
    }
}

static GLOBAL: Mutex<Option<Rng>> = Mutex::new(None);

fn with_global<R>(f: impl FnOnce(&mut Rng) -> R) -> R {
    let mut g = GLOBAL.lock().unwrap_or_else(|e| e.into_inner());
    f(g.get_or_insert_with(|| Rng::new(0)))
}

/// Seed the global generator. Translation of `SDL_srand()`.
///
/// Reusing the seed will cause [`below`]/[`next_f32`]/[`next_u32`] to repeat
/// the same stream of "random" numbers, which is often useful for debugging.
/// A seed of 0 uses the current performance counter.
pub fn seed(seed: u64) {
    *GLOBAL.lock().unwrap_or_else(|e| e.into_inner()) = Some(Rng::new(seed));
}

/// Global-generator version of [`Rng::below`]. Translation of `SDL_rand()`.
pub fn below(n: i32) -> i32 {
    with_global(|r| r.below(n))
}

/// Global-generator version of [`Rng::next_f32`]. Translation of `SDL_randf()`.
pub fn next_f32() -> f32 {
    with_global(|r| r.next_f32())
}

/// Global-generator version of [`Rng::next_u32`]. Translation of `SDL_rand_bits()`.
pub fn next_u32() -> u32 {
    with_global(|r| r.next_u32())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deterministic_sequence() {
        let mut a = Rng::new(12345);
        let mut b = Rng::new(12345);
        for _ in 0..100 {
            assert_eq!(a.next_u32(), b.next_u32());
        }
        // First outputs of the LCG from a known seed (matches the C implementation).
        let mut r = Rng::new(1);
        assert_eq!(r.state(), 1);
        assert_eq!(
            r.next_u32(),
            (1u64.wrapping_mul(0xff1cd035).wrapping_add(5) >> 32) as u32
        );
    }

    #[test]
    fn ranges() {
        let mut s = Rng::new(1);
        for _ in 0..10_000 {
            assert!((0..10).contains(&s.below(10)));
            assert!((0.0..1.0).contains(&s.next_f32()));
        }
        assert_eq!(s.below(-5), 0);
        assert_eq!(s.below(0), 0);
    }

    #[test]
    fn global_generator() {
        seed(42);
        let x = below(100);
        seed(42);
        assert_eq!(below(100), x);
        let _ = next_f32();
        let _ = next_u32();
        assert_ne!(Rng::new(0).state(), 0);
    }
}
