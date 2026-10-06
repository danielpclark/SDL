// Rust translation of src/test/SDL_test_fuzzer.c and include/SDL3/SDL_test_fuzzer.h
// from Simple DirectMedia Layer.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! Fuzzer functions of SDL test framework.
//!
//! Data generators for fuzzing test data in a reproducible way: after
//! [`init`] with an execution key, the generators return the same sequence
//! as upstream's for that key. The harness initializes the fuzzer before
//! every test with a key made from the run seed and the test's name.
//!
//! Based on GSOC code by Markus Kauppila <markus.kauppila@gmail.com>
//!
//! Note: The fuzzer implementation uses a static instance of random context
//! internally, shared by all threads. Each draw is atomic here, so threads
//! can't corrupt it, but threads drawing at once interleave their sequences.

use std::sync::atomic::{AtomicI32, AtomicU64, Ordering};

use sdl3::stdlib::Rng;
use sdl3::{Error, Result};

/// Counter for fuzzer invocations
static FUZZER_INVOCATION_COUNTER: AtomicI32 = AtomicI32::new(0);

/// Context for shared random number generator
static RND_CONTEXT: AtomicU64 = AtomicU64::new(0);

fn count_invocation() {
    FUZZER_INVOCATION_COUNTER.fetch_add(1, Ordering::Relaxed);
}

/// Run `f` on the shared generator as one atomic update of its state.
/// (Lock-free, so the memory tracker can draw from inside the allocator.)
fn with_rng<R>(f: impl Fn(&mut Rng) -> R) -> R {
    let mut state = RND_CONTEXT.load(Ordering::Acquire);
    loop {
        let mut rng = Rng::from_state(state);
        let result = f(&mut rng);
        match RND_CONTEXT.compare_exchange_weak(
            state,
            rng.state(),
            Ordering::AcqRel,
            Ordering::Acquire,
        ) {
            Ok(_) => return result,
            Err(current) => state = current,
        }
    }
}

/// `SDL_rand_bits_r(&rndContext)`.
fn rand_bits() -> u32 {
    with_rng(|rng| rng.next_u32())
}

/// Initializes the fuzzer for a test.
///
/// `exec_key` is the execution "Key" that initializes the random number
/// generator uniquely for the test. Translation of `SDLTest_FuzzerInit()`.
pub fn init(exec_key: u64) {
    RND_CONTEXT.store(exec_key, Ordering::Release);
    FUZZER_INVOCATION_COUNTER.store(0, Ordering::Relaxed);
}

/// Get the invocation count for the fuzzer since last [`init`].
/// Translation of `SDLTest_GetFuzzerInvocationCount()`.
pub fn invocation_count() -> i32 {
    FUZZER_INVOCATION_COUNTER.load(Ordering::Relaxed)
}

/// Returns a random Uint8. Translation of `SDLTest_RandomUint8()`.
pub fn random_uint8() -> u8 {
    count_invocation();

    (rand_bits() >> 24) as u8
}

/// Returns a random Sint8. Translation of `SDLTest_RandomSint8()`.
pub fn random_sint8() -> i8 {
    count_invocation();

    (rand_bits() >> 24) as i8
}

/// Returns a random Uint16. Translation of `SDLTest_RandomUint16()`.
pub fn random_uint16() -> u16 {
    count_invocation();

    (rand_bits() >> 16) as u16
}

/// Returns a random Sint16. Translation of `SDLTest_RandomSint16()`.
pub fn random_sint16() -> i16 {
    count_invocation();

    (rand_bits() >> 16) as i16
}

/// Returns a random positive integer. Translation of `SDLTest_RandomUint32()`.
pub fn random_uint32() -> u32 {
    count_invocation();

    rand_bits()
}

/// Returns a random integer. Translation of `SDLTest_RandomSint32()`.
pub fn random_sint32() -> i32 {
    count_invocation();

    rand_bits() as i32
}

/// Returns random Uint64: two Uint32s, the first in the low half (on a
/// little-endian machine, as upstream's union). It counts as three
/// invocations, as upstream's. Translation of `SDLTest_RandomUint64()`.
pub fn random_uint64() -> u64 {
    count_invocation();

    let v32_0 = random_uint32();
    let v32_1 = random_uint32();

    union_u64(v32_0, v32_1)
}

/// Returns random Sint64 (made as [`random_uint64`]). Translation of
/// `SDLTest_RandomSint64()`.
pub fn random_sint64() -> i64 {
    count_invocation();

    let v32_0 = random_uint32();
    let v32_1 = random_uint32();

    union_u64(v32_0, v32_1) as i64
}

/// The `Uint64` of upstream's `union { Uint64 v64; Uint32 v32[2]; }` with
/// `v32[0]` and `v32[1]` set.
fn union_u64(v32_0: u32, v32_1: u32) -> u64 {
    let mut bytes = [0u8; 8];
    bytes[..4].copy_from_slice(&v32_0.to_ne_bytes());
    bytes[4..].copy_from_slice(&v32_1.to_ne_bytes());
    u64::from_ne_bytes(bytes)
}

/// Returns integer in range [min, max] (inclusive). Min and max values can
/// be negative values. If Max in smaller than min, then the values are
/// swapped. Min and max are the same value, that value will be returned.
/// Translation of `SDLTest_RandomIntegerInRange()`.
pub fn random_integer_in_range(mut min: i32, mut max: i32) -> i32 {
    count_invocation();

    if min == max {
        return min;
    }

    if min > max {
        std::mem::swap(&mut min, &mut max);
    }

    let range = (max as i64 - min as i64) as u64;
    if range < i32::MAX as u64 {
        min + with_rng(|rng| rng.below(range as i32 + 1))
    } else {
        let add = with_rng(|rng| {
            let low = rng.next_u32() as u64;
            low | ((rng.next_u32() as u64) << 32)
        });
        (min as i64 + (add % (range + 1)) as i64) as i32
    }
}

/// Generates a unsigned boundary value between the given boundaries.
/// Boundary values are inclusive. See the examples below.
/// If boundary2 < boundary1, the values are swapped.
/// If boundary1 == boundary2, value of boundary1 will be returned
///
/// Generating boundary values for Uint8:
/// ```text
/// BoundaryValues(UINT8_MAX, 10, 20, True) -> [10,11,19,20]
/// BoundaryValues(UINT8_MAX, 10, 20, False) -> [9,21]
/// BoundaryValues(UINT8_MAX, 0, 15, True) -> [0, 1, 14, 15]
/// BoundaryValues(UINT8_MAX, 0, 15, False) -> [16]
/// BoundaryValues(UINT8_MAX, 0, 0xFF, False) -> [0], error set
/// ```
///
/// Generator works the same for other types of unsigned integers.
///
/// `max_value` is the biggest value that is acceptable for this data type
/// (for instance, for Uint8 -> 255, Uint16 -> 65536 etc.), `boundary1` and
/// `boundary2` define the lower and upper boundaries, `valid_domain`
/// generates only for the valid domain (for the data type).
///
/// Returns a random boundary value for the domain, or the error "That
/// operation is not supported" where there is none. Translation of
/// `SDLTest_GenerateUnsignedBoundaryValues()`.
fn generate_unsigned_boundary_values(
    max_value: u64,
    boundary1: u64,
    boundary2: u64,
    valid_domain: bool,
) -> Result<u64> {
    let mut temp_buf = [0u64; 4];

    /* Maybe swap */
    let (b1, b2) = if boundary1 > boundary2 {
        (boundary2, boundary1)
    } else {
        (boundary1, boundary2)
    };

    let mut index: u8 = 0;
    if valid_domain {
        if b1 == b2 {
            return Ok(b1);
        }

        /* Generate up to 4 values within bounds */
        let delta = b2 - b1;
        if delta < 4 {
            // (so b2 itself is never among them; upstream's quirk)
            loop {
                temp_buf[index as usize] = b1 + index as u64;
                index += 1;
                if (index as u64) >= delta {
                    break;
                }
            }
        } else {
            temp_buf[index as usize] = b1;
            index += 1;
            temp_buf[index as usize] = b1 + 1;
            index += 1;
            temp_buf[index as usize] = b2 - 1;
            index += 1;
            temp_buf[index as usize] = b2;
            index += 1;
        }
    } else {
        /* Generate up to 2 values outside of bounds */
        if b1 > 0 {
            temp_buf[index as usize] = b1 - 1;
            index += 1;
        }

        if b2 < max_value {
            temp_buf[index as usize] = b2 + 1;
            index += 1;
        }
    }

    if index == 0 {
        /* There are no valid boundaries */
        return Err(Error::unsupported());
    }

    Ok(temp_buf[(random_uint8() % index) as usize])
}

/// Returns a random boundary value for Uint8 within the given boundaries.
/// Boundaries are inclusive, see the usage examples below. If `valid_domain`
/// is true, the function will only return valid boundaries, otherwise
/// non-valid boundaries are also possible. If boundary1 > boundary2, the
/// values are swapped.
///
/// Usage examples:
/// ```text
/// RandomUint8BoundaryValue(10, 20, true) returns 10, 11, 19 or 20
/// RandomUint8BoundaryValue(1, 20, false) returns 0 or 21
/// RandomUint8BoundaryValue(0, 99, false) returns 100
/// RandomUint8BoundaryValue(0, 255, false) returns 0 (error set)
/// ```
///
/// Returns the error "That operation is not supported" where there is no
/// boundary value. Translation of `SDLTest_RandomUint8BoundaryValue()`.
pub fn random_uint8_boundary_value(boundary1: u8, boundary2: u8, valid_domain: bool) -> Result<u8> {
    /* max value for Uint8 */
    let max_value = u8::MAX as u64;
    generate_unsigned_boundary_values(max_value, boundary1 as u64, boundary2 as u64, valid_domain)
        .map(|v| v as u8)
}

/// Returns a random boundary value for Uint16 within the given boundaries,
/// as [`random_uint8_boundary_value`]. Translation of
/// `SDLTest_RandomUint16BoundaryValue()`.
pub fn random_uint16_boundary_value(
    boundary1: u16,
    boundary2: u16,
    valid_domain: bool,
) -> Result<u16> {
    /* max value for Uint16 */
    let max_value = u16::MAX as u64;
    generate_unsigned_boundary_values(max_value, boundary1 as u64, boundary2 as u64, valid_domain)
        .map(|v| v as u16)
}

/// Returns a random boundary value for Uint32 within the given boundaries,
/// as [`random_uint8_boundary_value`]. Translation of
/// `SDLTest_RandomUint32BoundaryValue()`.
pub fn random_uint32_boundary_value(
    boundary1: u32,
    boundary2: u32,
    valid_domain: bool,
) -> Result<u32> {
    /* max value for Uint32 */
    let max_value = u32::MAX as u64;
    generate_unsigned_boundary_values(max_value, boundary1 as u64, boundary2 as u64, valid_domain)
        .map(|v| v as u32)
}

/// Returns a random boundary value for Uint64 within the given boundaries,
/// as [`random_uint8_boundary_value`]. Translation of
/// `SDLTest_RandomUint64BoundaryValue()`.
pub fn random_uint64_boundary_value(
    boundary1: u64,
    boundary2: u64,
    valid_domain: bool,
) -> Result<u64> {
    /* max value for Uint64 */
    let max_value = u64::MAX;
    generate_unsigned_boundary_values(max_value, boundary1, boundary2, valid_domain)
}

/// Generates a signed boundary value between the given boundaries.
/// Boundary values are inclusive. See the examples below.
/// If boundary2 < boundary1, the values are swapped.
/// If boundary1 == boundary2, value of boundary1 will be returned
///
/// Generating boundary values for Sint8:
/// ```text
/// SignedBoundaryValues(SCHAR_MIN, SCHAR_MAX, -10, 20, True) -> [-10,-9,19,20]
/// SignedBoundaryValues(SCHAR_MIN, SCHAR_MAX, -10, 20, False) -> [-11,21]
/// SignedBoundaryValues(SCHAR_MIN, SCHAR_MAX, -30, -15, True) -> [-30, -29, -16, -15]
/// SignedBoundaryValues(SCHAR_MIN, SCHAR_MAX, -127, 15, False) -> [16]
/// SignedBoundaryValues(SCHAR_MIN, SCHAR_MAX, -127, 127, False) -> [0], error set
/// ```
///
/// Generator works the same for other types of signed integers.
///
/// `min_value` and `max_value` are the smallest and biggest values that are
/// acceptable for this data type, `boundary1` and `boundary2` define the
/// lower and upper boundaries, `valid_domain` generates only for the valid
/// domain (for the data type).
///
/// Returns a random boundary value for the domain, or the error "That
/// operation is not supported" where there is none (where upstream returns
/// `min_value`). Translation of `SDLTest_GenerateSignedBoundaryValues()`.
fn generate_signed_boundary_values(
    min_value: i64,
    max_value: i64,
    boundary1: i64,
    boundary2: i64,
    valid_domain: bool,
) -> Result<i64> {
    let mut temp_buf = [0i64; 4];

    /* Maybe swap */
    let (b1, b2) = if boundary1 > boundary2 {
        (boundary2, boundary1)
    } else {
        (boundary1, boundary2)
    };

    let mut index: u8 = 0;
    if valid_domain {
        if b1 == b2 {
            return Ok(b1);
        }

        /* Generate up to 4 values within bounds */
        // (as in C, the difference wraps when the bounds are further apart
        // than an Sint64 can say, as from INT64_MIN to INT64_MAX)
        let delta = b2.wrapping_sub(b1);
        if delta < 4 {
            // (so b2 itself is never among them; upstream's quirk)
            loop {
                temp_buf[index as usize] = b1 + index as i64;
                index += 1;
                if (index as i64) >= delta {
                    break;
                }
            }
        } else {
            temp_buf[index as usize] = b1;
            index += 1;
            temp_buf[index as usize] = b1 + 1;
            index += 1;
            temp_buf[index as usize] = b2 - 1;
            index += 1;
            temp_buf[index as usize] = b2;
            index += 1;
        }
    } else {
        /* Generate up to 2 values outside of bounds */
        if b1 > min_value {
            temp_buf[index as usize] = b1 - 1;
            index += 1;
        }

        if b2 < max_value {
            temp_buf[index as usize] = b2 + 1;
            index += 1;
        }
    }

    if index == 0 {
        /* There are no valid boundaries */
        return Err(Error::unsupported());
    }

    Ok(temp_buf[(random_uint8() % index) as usize])
}

/// Returns a random boundary value for Sint8 within the given boundaries.
/// Boundaries are inclusive, see the usage examples below. If `valid_domain`
/// is true, the function will only return valid boundaries, otherwise
/// non-valid boundaries are also possible. If boundary1 > boundary2, the
/// values are swapped.
///
/// Usage examples:
/// ```text
/// RandomSint8BoundaryValue(-10, 20, true) returns -11, -10, 19 or 20
/// RandomSint8BoundaryValue(-100, -10, false) returns -101 or -9
/// RandomSint8BoundaryValue(SINT8_MIN, 99, false) returns 100
/// RandomSint8BoundaryValue(SINT8_MIN, SINT8_MAX, false) returns SINT8_MIN (== error value) with error set
/// ```
///
/// Returns the error "That operation is not supported" where there is no
/// boundary value. Translation of `SDLTest_RandomSint8BoundaryValue()`.
pub fn random_sint8_boundary_value(boundary1: i8, boundary2: i8, valid_domain: bool) -> Result<i8> {
    /* min & max values for Sint8 */
    let max_value = i8::MAX as i64;
    let min_value = i8::MIN as i64;
    generate_signed_boundary_values(
        min_value,
        max_value,
        boundary1 as i64,
        boundary2 as i64,
        valid_domain,
    )
    .map(|v| v as i8)
}

/// Returns a random boundary value for Sint16 within the given boundaries,
/// as [`random_sint8_boundary_value`]. Translation of
/// `SDLTest_RandomSint16BoundaryValue()`.
pub fn random_sint16_boundary_value(
    boundary1: i16,
    boundary2: i16,
    valid_domain: bool,
) -> Result<i16> {
    /* min & max values for Sint16 */
    let max_value = i16::MAX as i64;
    let min_value = i16::MIN as i64;
    generate_signed_boundary_values(
        min_value,
        max_value,
        boundary1 as i64,
        boundary2 as i64,
        valid_domain,
    )
    .map(|v| v as i16)
}

/// Returns a random boundary value for Sint32 within the given boundaries,
/// as [`random_sint8_boundary_value`]. Translation of
/// `SDLTest_RandomSint32BoundaryValue()`.
pub fn random_sint32_boundary_value(
    boundary1: i32,
    boundary2: i32,
    valid_domain: bool,
) -> Result<i32> {
    /* min & max values for Sint32 */
    let max_value = i32::MAX as i64;
    let min_value = i32::MIN as i64;
    generate_signed_boundary_values(
        min_value,
        max_value,
        boundary1 as i64,
        boundary2 as i64,
        valid_domain,
    )
    .map(|v| v as i32)
}

/// Returns a random boundary value for Sint64 within the given boundaries,
/// as [`random_sint8_boundary_value`]. Translation of
/// `SDLTest_RandomSint64BoundaryValue()`.
pub fn random_sint64_boundary_value(
    boundary1: i64,
    boundary2: i64,
    valid_domain: bool,
) -> Result<i64> {
    /* min & max values for Sint64 */
    let max_value = i64::MAX;
    let min_value = i64::MIN;
    generate_signed_boundary_values(min_value, max_value, boundary1, boundary2, valid_domain)
}

/// Returns a random float in range [0.0 - 1.0[. It doesn't count as an
/// invocation (as upstream's). Translation of `SDLTest_RandomUnitFloat()`.
pub fn random_unit_float() -> f32 {
    with_rng(|rng| rng.next_f32())
}

/// Returns a random float: the bits of a random Uint32, drawn again while
/// they are a NaN or an infinity. Translation of `SDLTest_RandomFloat()`.
pub fn random_float() -> f32 {
    loop {
        let value = f32::from_bits(random_uint32());
        if !(value.is_nan() || value.is_infinite()) {
            return value;
        }
    }
}

/// Returns a random double in range [0.0 - 1.0[. Translation of
/// `SDLTest_RandomUnitDouble()`.
pub fn random_unit_double() -> f64 {
    (random_uint64() >> (64 - 53)) as f64 * f64::from_bits(0x3CA0000000000000) // 0x1.0p-53
}

/// Returns a random double: the bits of a random Uint64, drawn again while
/// they are a NaN or an infinity. Translation of `SDLTest_RandomDouble()`.
pub fn random_double() -> f64 {
    loop {
        let value = f64::from_bits(random_uint64());
        if !(value.is_nan() || value.is_infinite()) {
            return value;
        }
    }
}

/// Generates random string. The minimum length for the string is 1
/// character, maximum length for the string is 255 characters and it can
/// contain ASCII characters from 32 to 126. Translation of
/// `SDLTest_RandomAsciiString()`.
pub fn random_ascii_string() -> String {
    // (255 is a valid length)
    random_ascii_string_with_maximum_length(255).unwrap_or_default()
}

/// Generates random string. The maximum length for the string is defined
/// by the `max_length` parameter. String can contain ASCII characters from
/// 32 to 126.
///
/// Returns the error "Parameter 'maxLength' is invalid" if `max_length` is
/// less than 1. Translation of `SDLTest_RandomAsciiStringWithMaximumLength()`.
pub fn random_ascii_string_with_maximum_length(max_length: i32) -> Result<String> {
    if max_length < 1 {
        return Err(Error::invalid_param("maxLength"));
    }

    let mut size = (random_uint32() % (max_length.wrapping_add(1) as u32)) as i32;
    if size == 0 {
        size = 1;
    }
    random_ascii_string_of_size(size)
}

/// Generates random string. The length for the string is defined by the
/// `size` parameter. String can contain ASCII characters from 32 to 126.
///
/// Returns the error "Parameter 'size' is invalid" if `size` is less
/// than 1. Translation of `SDLTest_RandomAsciiStringOfSize()`.
pub fn random_ascii_string_of_size(size: i32) -> Result<String> {
    if size < 1 {
        return Err(Error::invalid_param("size"));
    }

    let mut string = String::with_capacity(size as usize);

    for _ in 0..size {
        string.push(random_integer_in_range(32, 126) as u8 as char);
    }

    count_invocation();

    Ok(string)
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::test_support::test_lock;

    /// Upstream's sequences for an execution key, from `SDL_test_fuzzer.c`
    /// compiled with SDL's `SDL_rand_*_r()`.
    #[test]
    fn sequences_for_a_seed() {
        let _l = test_lock();
        init(0x0123456789ABCDEF);
        let u8s: Vec<u8> = (0..8).map(|_| random_uint8()).collect();
        let s8s: Vec<i8> = (0..4).map(|_| random_sint8()).collect();
        let u16s: Vec<u16> = (0..4).map(|_| random_uint16()).collect();
        let s16s: Vec<i16> = (0..4).map(|_| random_sint16()).collect();
        let u32s: Vec<u32> = (0..4).map(|_| random_uint32()).collect();
        let s32s: Vec<i32> = (0..4).map(|_| random_sint32()).collect();
        let u64s: Vec<u64> = (0..2).map(|_| random_uint64()).collect();
        let s64s: Vec<i64> = (0..2).map(|_| random_sint64()).collect();
        let ranges: Vec<i32> = [
            (0, 10),
            (-5, 5),
            (10, -10),
            (7, 7),
            (i32::MIN, i32::MAX),
            (i32::MIN, 0),
            (-1, i32::MAX),
        ]
        .iter()
        .map(|&(a, b)| random_integer_in_range(a, b))
        .collect();
        let unit_float = random_unit_float();
        let float = random_float();
        let unit_double = random_unit_double();
        let double = random_double();
        let string = random_ascii_string_of_size(20).unwrap();
        let count = invocation_count();

        let expected = include_str!("testdata/fuzzer_sequences.txt");
        let actual = format!(
            "u8 {u8s:?}\ns8 {s8s:?}\nu16 {u16s:?}\ns16 {s16s:?}\nu32 {u32s:?}\ns32 {s32s:?}\n\
             u64 {u64s:?}\ns64 {s64s:?}\nrange {ranges:?}\nunitf {:08x}\nf {:08x}\n\
             unitd {:016x}\nd {:016x}\nstring {string}\ncount {count}\n",
            unit_float.to_bits(),
            float.to_bits(),
            unit_double.to_bits(),
            double.to_bits(),
        );
        assert_eq!(actual, expected);
    }

    /// Upstream's boundary values and strings for an execution key, from
    /// `SDL_test_fuzzer.c` compiled with SDL's `SDL_rand_*_r()`.
    #[test]
    fn boundary_sequences_for_a_seed() {
        let _l = test_lock();
        init(12345);
        let line = |values: Vec<String>| values.join(" ") + "\n";
        let mut actual = String::new();
        actual += &line(
            (0..16)
                .map(|_| {
                    random_uint8_boundary_value(10, 20, true)
                        .unwrap()
                        .to_string()
                })
                .collect(),
        );
        actual += &line(
            (0..16)
                .map(|_| {
                    random_sint16_boundary_value(-100, 100, false)
                        .unwrap()
                        .to_string()
                })
                .collect(),
        );
        actual += &line(
            (0..8)
                .map(|_| {
                    random_uint64_boundary_value(5, 3000000000000, true)
                        .unwrap()
                        .to_string()
                })
                .collect(),
        );
        actual += &line(
            (0..4)
                .map(|_| format!("[{}]", random_ascii_string_with_maximum_length(12).unwrap()))
                .collect(),
        );
        actual += &format!("[{}]\n", random_ascii_string());
        actual += &format!("count {}\n", invocation_count());
        assert_eq!(actual, include_str!("testdata/fuzzer_boundaries.txt"));
    }

    #[test]
    fn invocation_counts() {
        let _l = test_lock();
        init(1);
        assert_eq!(invocation_count(), 0);
        random_uint8();
        assert_eq!(invocation_count(), 1);
        // a Uint64 counts once for itself and once for each Uint32
        random_uint64();
        assert_eq!(invocation_count(), 4);
        // the unit float doesn't count
        random_unit_float();
        assert_eq!(invocation_count(), 4);
        // a string counts each character's range and once itself
        random_ascii_string_of_size(5).unwrap();
        assert_eq!(invocation_count(), 10);
        init(1);
        assert_eq!(invocation_count(), 0);
    }

    #[test]
    fn boundary_values() {
        let _l = test_lock();
        init(42);
        for _ in 0..100 {
            assert_eq!(random_uint8_boundary_value(10, 10, true).unwrap(), 10);
            assert!([10, 11, 19, 20].contains(&random_uint8_boundary_value(10, 20, true).unwrap()));
            assert!([10, 11, 19, 20].contains(&random_uint8_boundary_value(20, 10, true).unwrap()));
            assert!([0, 21].contains(&random_uint8_boundary_value(1, 20, false).unwrap()));
            assert_eq!(random_uint8_boundary_value(0, 99, false).unwrap(), 100);
            assert_eq!(random_uint8_boundary_value(1, 255, false).unwrap(), 0);
            assert_eq!(random_uint8_boundary_value(0, 254, false).unwrap(), 255);
            // Fewer than 4 apart: b1 up to b2 - 1 only (upstream's quirk).
            assert_eq!(random_uint8_boundary_value(10, 11, true).unwrap(), 10);
            assert!([10, 11].contains(&random_uint8_boundary_value(10, 12, true).unwrap()));
            assert!([10, 11, 12].contains(&random_uint8_boundary_value(10, 13, true).unwrap()));
            assert!(
                [-10, -9, 19, 20].contains(&random_sint8_boundary_value(-10, 20, true).unwrap())
            );
            assert!([-11, 21].contains(&random_sint8_boundary_value(-10, 20, false).unwrap()));
            assert_eq!(random_sint8_boundary_value(-128, 99, false).unwrap(), 100);
            assert!([i64::MIN, i64::MIN + 1, i64::MAX - 1, i64::MAX]
                .contains(&random_sint64_boundary_value(i64::MIN, i64::MAX, true).unwrap()));
            assert!([0, u64::MAX]
                .contains(&random_uint64_boundary_value(1, u64::MAX - 1, false).unwrap()));
        }
        let error = random_uint8_boundary_value(0, 255, false).unwrap_err();
        assert_eq!(error.message(), "That operation is not supported");
        assert!(random_uint16_boundary_value(0, u16::MAX, false).is_err());
        assert!(random_uint32_boundary_value(0, u32::MAX, false).is_err());
        assert!(random_uint64_boundary_value(0, u64::MAX, false).is_err());
        assert!(random_sint8_boundary_value(i8::MIN, i8::MAX, false).is_err());
        assert!(random_sint16_boundary_value(i16::MIN, i16::MAX, false).is_err());
        assert!(random_sint32_boundary_value(i32::MIN, i32::MAX, false).is_err());
        assert!(random_sint64_boundary_value(i64::MIN, i64::MAX, false).is_err());
    }

    #[test]
    fn strings() {
        let _l = test_lock();
        init(7);
        for _ in 0..50 {
            let s = random_ascii_string();
            assert!((1..=255).contains(&s.len()));
            assert!(s.bytes().all(|b| (32..=126).contains(&b)));
            let s = random_ascii_string_with_maximum_length(10).unwrap();
            assert!((1..=10).contains(&s.len()));
        }
        assert_eq!(
            random_ascii_string_with_maximum_length(0)
                .unwrap_err()
                .message(),
            "Parameter 'maxLength' is invalid"
        );
        assert_eq!(
            random_ascii_string_of_size(0).unwrap_err().message(),
            "Parameter 'size' is invalid"
        );
        assert_eq!(random_ascii_string_of_size(33).unwrap().len(), 33);
    }

    #[test]
    fn ranges_and_floats() {
        let _l = test_lock();
        init(99);
        for _ in 0..1000 {
            let v = random_integer_in_range(-3, 3);
            assert!((-3..=3).contains(&v));
            let v = random_integer_in_range(i32::MAX - 2, i32::MAX);
            assert!(v >= i32::MAX - 2);
            assert!((0.0..1.0).contains(&random_unit_float()));
            assert!((0.0..1.0).contains(&random_unit_double()));
            assert!(random_float().is_finite());
            assert!(random_double().is_finite());
        }
    }
}
