// Rust translation of test/testautomation_sdltest.c from Simple DirectMedia Layer:
// upstream's test suite of the test library, run through the harness.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

/*
 * SDL_test test suite
 */
use std::sync::{Arc, Mutex};

use sdl3::init::InitFlags;
use sdl3_test::common::CommonState;
use sdl3_test::fuzzer::*;
use sdl3_test::harness::{
    generate_run_seed, TestCase, TestData, TestStatus, TestSuite, TestSuiteRunner,
};
use sdl3_test::{assert_check, assert_pass, log_error};

const EXPECTED_UNSUPPORTED: &str = "That operation is not supported";

/// `SDL_GetError()` checks: the error of a boundary value call, or that
/// there was none.
fn check_no_error<T>(result: &sdl3::Result<T>) {
    assert_pass!("SDL_GetError()");
    assert_check!(result.is_ok(), "Validate no error message was set");
}

fn check_error<T>(result: &sdl3::Result<T>, expected_error: &str) {
    assert_pass!("SDL_GetError()");
    let last_error = result.as_ref().err().map(|e| e.message().to_owned());
    assert_check!(
        last_error.as_deref() == Some(expected_error),
        "SDL_GetError(): expected message '{}', was message: '{}'",
        expected_error,
        last_error.as_deref().unwrap_or("(null)")
    );
}

/* Test case functions */

/*
 * Calls to SDLTest_GenerateRunSeed()
 */
fn sdltest_generate_run_seed(_: &mut TestData) -> TestStatus {
    for i in (1..=10).step_by(3) {
        let result = generate_run_seed(i);
        assert_pass!("Call to SDLTest_GenerateRunSeed(<buf>, {})", i);
        assert_check!(result.is_some(), "Verify returned value is not NULL");
        if let Some(result) = result {
            let l = result.len();
            assert_check!(
                l == i as usize,
                "Verify length of returned value is {}, got: {}",
                i,
                l
            );
        }
    }

    // (no buffer is impossible here: SDLTest_GenerateRunSeed(NULL, 10))

    /* Negative cases */
    for j in -2..=0 {
        let result = generate_run_seed(j);
        assert_pass!("Call to SDLTest_GenerateRunSeed(<buf>, {})", j);
        assert_check!(result.is_none(), "Verify returned value is NULL");
    }

    TestStatus::Completed
}

/*
 * Calls to SDLTest_GetFuzzerInvocationCount()
 */
fn sdltest_get_fuzzer_invocation_count(_: &mut TestData) -> TestStatus {
    let fuzzer_count1 = invocation_count();
    assert_pass!("Call to SDLTest_GetFuzzerInvocationCount()");
    assert_check!(
        fuzzer_count1 >= 0,
        "Verify returned value, expected: >=0, got: {}",
        fuzzer_count1
    );

    let result = random_uint8();
    assert_pass!("Call to SDLTest_RandomUint8(), returned {}", result);

    let fuzzer_count2 = invocation_count();
    assert_pass!("Call to SDLTest_GetFuzzerInvocationCount()");
    assert_check!(
        fuzzer_count2 > fuzzer_count1,
        "Verify returned value, expected: >{}, got: {}",
        fuzzer_count1,
        fuzzer_count2
    );

    TestStatus::Completed
}

/*
 * Calls to random number generators
 */
fn sdltest_random_number(_: &mut TestData) -> TestStatus {
    let result = random_uint8() as i64;
    let umax = (1u64 << 8) - 1;
    assert_pass!("Call to SDLTest_RandomUint8");
    assert_check!(
        result >= 0 && result <= umax as i64,
        "Verify result value, expected: [0,{}], got: {}",
        umax,
        result
    );

    let result = random_sint8() as i64;
    let (min, max) = (-(1i64 << 7), (1i64 << 7) - 1);
    assert_pass!("Call to SDLTest_RandomSint8");
    assert_check!(
        result >= min && result <= max,
        "Verify result value, expected: [{},{}], got: {}",
        min,
        max,
        result
    );

    let result = random_uint16() as i64;
    let umax = (1u64 << 16) - 1;
    assert_pass!("Call to SDLTest_RandomUint16");
    assert_check!(
        result >= 0 && result <= umax as i64,
        "Verify result value, expected: [0,{}], got: {}",
        umax,
        result
    );

    let result = random_sint16() as i64;
    let (min, max) = (-(1i64 << 15), (1i64 << 15) - 1);
    assert_pass!("Call to SDLTest_RandomSint16");
    assert_check!(
        result >= min && result <= max,
        "Verify result value, expected: [{},{}], got: {}",
        min,
        max,
        result
    );

    let result = random_uint32() as i64;
    let umax = (1u64 << 32) - 1;
    assert_pass!("Call to SDLTest_RandomUint32");
    assert_check!(
        result >= 0 && result <= umax as i64,
        "Verify result value, expected: [0,{}], got: {}",
        umax,
        result
    );

    let result = random_sint32() as i64;
    let (min, max) = (-(1i64 << 31), (1i64 << 31) - 1);
    assert_pass!("Call to SDLTest_RandomSint32");
    assert_check!(
        result >= min && result <= max,
        "Verify result value, expected: [{},{}], got: {}",
        min,
        max,
        result
    );

    random_uint64();
    assert_pass!("Call to SDLTest_RandomUint64");

    random_sint64();
    assert_pass!("Call to SDLTest_RandomSint64");

    let dresult = random_unit_float() as f64;
    assert_pass!("Call to SDLTest_RandomUnitFloat");
    assert_check!(
        (0.0..1.0).contains(&dresult),
        "Verify result value, expected: [0.0,1.0[, got: {:e}",
        dresult
    );

    let dresult = random_float() as f64;
    assert_pass!("Call to SDLTest_RandomFloat");
    assert_check!(
        dresult >= -(f32::MAX as f64) && dresult <= f32::MAX as f64,
        "Verify result value, expected: [{:e},{:e}], got: {:e}",
        -(f32::MAX as f64),
        f32::MAX as f64,
        dresult
    );

    let dresult = random_unit_double();
    assert_pass!("Call to SDLTest_RandomUnitDouble");
    assert_check!(
        (0.0..1.0).contains(&dresult),
        "Verify result value, expected: [0.0,1.0[, got: {:e}",
        dresult
    );

    random_double();
    assert_pass!("Call to SDLTest_RandomDouble");

    TestStatus::Completed
}

/// The test of random boundary number generators for an unsigned type
/// (`sdltest_randomBoundaryNumberUintX`, the same for each type).
macro_rules! unsigned_boundary_test {
    ($name:ident, $generator:ident, $call:literal, $max:expr, $max_text:literal) => {
        fn $name(_: &mut TestData) -> TestStatus {
            let max = $max;

            /* Clean error messages */
            assert_pass!("SDL_ClearError()");

            /* RandomUintXBoundaryValue(10, 10, true) returns 10 */
            let uresult = $generator(10, 10, true).unwrap_or(0) as u64;
            assert_pass!("Call to {}", $call);
            assert_check!(
                uresult == 10,
                "Validate result value for parameters (10,10,true); expected: 10, got: {}",
                uresult
            );

            /* RandomUintXBoundaryValue(10, 11, true) returns 10, 11 */
            let uresult = $generator(10, 11, true).unwrap_or(0) as u64;
            assert_pass!("Call to {}", $call);
            assert_check!(
                uresult == 10 || uresult == 11,
                "Validate result value for parameters (10,11,true); expected: 10|11, got: {}",
                uresult
            );

            /* RandomUintXBoundaryValue(10, 12, true) returns 10, 11, 12 */
            let uresult = $generator(10, 12, true).unwrap_or(0) as u64;
            assert_pass!("Call to {}", $call);
            assert_check!(
                uresult == 10 || uresult == 11 || uresult == 12,
                "Validate result value for parameters (10,12,true); expected: 10|11|12, got: {}",
                uresult
            );

            /* RandomUintXBoundaryValue(10, 13, true) returns 10, 11, 12, 13 */
            let uresult = $generator(10, 13, true).unwrap_or(0) as u64;
            assert_pass!("Call to {}", $call);
            assert_check!(
                uresult == 10 || uresult == 11 || uresult == 12 || uresult == 13,
                "Validate result value for parameters (10,13,true); expected: 10|11|12|13, got: {}",
                uresult
            );

            /* RandomUintXBoundaryValue(10, 20, true) returns 10, 11, 19 or 20 */
            let uresult = $generator(10, 20, true).unwrap_or(0) as u64;
            assert_pass!("Call to {}", $call);
            assert_check!(
                uresult == 10 || uresult == 11 || uresult == 19 || uresult == 20,
                "Validate result value for parameters (10,20,true); expected: 10|11|19|20, got: {}",
                uresult
            );

            /* RandomUintXBoundaryValue(20, 10, true) returns 10, 11, 19 or 20 */
            let uresult = $generator(20, 10, true).unwrap_or(0) as u64;
            assert_pass!("Call to {}", $call);
            assert_check!(
                uresult == 10 || uresult == 11 || uresult == 19 || uresult == 20,
                "Validate result value for parameters (20,10,true); expected: 10|11|19|20, got: {}",
                uresult
            );

            /* RandomUintXBoundaryValue(1, 20, false) returns 0, 21 */
            let uresult = $generator(1, 20, false).unwrap_or(0) as u64;
            assert_pass!("Call to {}", $call);
            assert_check!(
                uresult == 0 || uresult == 21,
                "Validate result value for parameters (1,20,false); expected: 0|21, got: {}",
                uresult
            );

            /* RandomUintXBoundaryValue(0, 99, false) returns 100 */
            let uresult = $generator(0, 99, false).unwrap_or(0) as u64;
            assert_pass!("Call to {}", $call);
            assert_check!(
                uresult == 100,
                "Validate result value for parameters (0,99,false); expected: 100, got: {}",
                uresult
            );

            /* RandomUintXBoundaryValue(1, max, false) returns 0 (no error) */
            let result = $generator(1, max, false);
            let uresult = *result.as_ref().unwrap_or(&0) as u64;
            assert_pass!("Call to {}", $call);
            assert_check!(
                uresult == 0,
                "Validate result value for parameters (1,{},false); expected: 0, got: {}",
                $max_text,
                uresult
            );
            check_no_error(&result);

            /* RandomUintXBoundaryValue(0, max - 1, false) returns max (no error) */
            let result = $generator(0, max - 1, false);
            let uresult = *result.as_ref().unwrap_or(&0) as u64;
            assert_pass!("Call to {}", $call);
            assert_check!(
                uresult == max as u64,
                "Validate result value for parameters (0,{} - 1,false); expected: {}, got: {}",
                $max_text,
                $max_text,
                uresult
            );
            check_no_error(&result);

            /* RandomUintXBoundaryValue(0, max, false) returns 0 (sets error) */
            let result = $generator(0, max, false);
            let uresult = *result.as_ref().unwrap_or(&0) as u64;
            assert_pass!("Call to {}", $call);
            assert_check!(
                uresult == 0,
                "Validate result value for parameters(0,{},false); expected: 0, got: {}",
                $max_text,
                uresult
            );
            check_error(&result, EXPECTED_UNSUPPORTED);

            /* Clear error messages */
            assert_pass!("SDL_ClearError()");

            TestStatus::Completed
        }
    };
}

/// The test of random boundary number generators for a signed type
/// (`sdltest_randomBoundaryNumberSintX`, the same for each type).
macro_rules! signed_boundary_test {
    ($name:ident, $generator:ident, $call:literal, $ty:ty) => {
        fn $name(_: &mut TestData) -> TestStatus {
            let (min, max) = (<$ty>::MIN, <$ty>::MAX);

            /* Clean error messages */
            assert_pass!("SDL_ClearError()");

            /* RandomSintXBoundaryValue(10, 10, true) returns 10 */
            let sresult = $generator(10, 10, true).unwrap_or(min) as i64;
            assert_pass!("Call to {}", $call);
            assert_check!(
                sresult == 10,
                "Validate result value for parameters (10,10,true); expected: 10, got: {}",
                sresult
            );

            /* RandomSintXBoundaryValue(10, 11, true) returns 10, 11 */
            let sresult = $generator(10, 11, true).unwrap_or(min) as i64;
            assert_pass!("Call to {}", $call);
            assert_check!(
                sresult == 10 || sresult == 11,
                "Validate result value for parameters (10,11,true); expected: 10|11, got: {}",
                sresult
            );

            /* RandomSintXBoundaryValue(10, 12, true) returns 10, 11, 12 */
            let sresult = $generator(10, 12, true).unwrap_or(min) as i64;
            assert_pass!("Call to {}", $call);
            assert_check!(
                sresult == 10 || sresult == 11 || sresult == 12,
                "Validate result value for parameters (10,12,true); expected: 10|11|12, got: {}",
                sresult
            );

            /* RandomSintXBoundaryValue(10, 13, true) returns 10, 11, 12, 13 */
            let sresult = $generator(10, 13, true).unwrap_or(min) as i64;
            assert_pass!("Call to {}", $call);
            assert_check!(
                sresult == 10 || sresult == 11 || sresult == 12 || sresult == 13,
                "Validate result value for parameters (10,13,true); expected: 10|11|12|13, got: {}",
                sresult
            );

            /* RandomSintXBoundaryValue(10, 20, true) returns 10, 11, 19 or 20 */
            let sresult = $generator(10, 20, true).unwrap_or(min) as i64;
            assert_pass!("Call to {}", $call);
            assert_check!(
                sresult == 10 || sresult == 11 || sresult == 19 || sresult == 20,
                "Validate result value for parameters (10,20,true); expected: 10|11|19|20, got: {}",
                sresult
            );

            /* RandomSintXBoundaryValue(20, 10, true) returns 10, 11, 19 or 20 */
            let sresult = $generator(20, 10, true).unwrap_or(min) as i64;
            assert_pass!("Call to {}", $call);
            assert_check!(
                sresult == 10 || sresult == 11 || sresult == 19 || sresult == 20,
                "Validate result value for parameters (20,10,true); expected: 10|11|19|20, got: {}",
                sresult
            );

            /* RandomSintXBoundaryValue(1, 20, false) returns 0, 21 */
            let sresult = $generator(1, 20, false).unwrap_or(min) as i64;
            assert_pass!("Call to {}", $call);
            assert_check!(
                sresult == 0 || sresult == 21,
                "Validate result value for parameters (1,20,false); expected: 0|21, got: {}",
                sresult
            );

            /* RandomSintXBoundaryValue(MIN, 99, false) returns 100 */
            let sresult = $generator(min, 99, false).unwrap_or(min) as i64;
            assert_pass!("Call to {}", $call);
            assert_check!(
                sresult == 100,
                "Validate result value for parameters (MIN,99,false); expected: 100, got: {}",
                sresult
            );

            /* RandomSintXBoundaryValue(MIN + 1, MAX, false) returns MIN (no error) */
            let result = $generator(min + 1, max, false);
            let sresult = *result.as_ref().unwrap_or(&max) as i64;
            assert_pass!("Call to {}", $call);
            assert_check!(
                sresult == min as i64,
                "Validate result value for parameters (MIN + 1,MAX,false); expected: {}, got: {}",
                min,
                sresult
            );
            check_no_error(&result);

            /* RandomSintXBoundaryValue(MIN, MAX - 1, false) returns MAX (no error) */
            let result = $generator(min, max - 1, false);
            let sresult = *result.as_ref().unwrap_or(&min) as i64;
            assert_pass!("Call to {}", $call);
            assert_check!(
                sresult == max as i64,
                "Validate result value for parameters (MIN,MAX - 1,false); expected: {}, got: {}",
                max,
                sresult
            );
            check_no_error(&result);

            /* RandomSintXBoundaryValue(MIN, MAX, false) returns MIN (sets error) */
            let result = $generator(min, max, false);
            let sresult = *result.as_ref().unwrap_or(&min) as i64;
            assert_pass!("Call to {}", $call);
            assert_check!(
                sresult == min as i64,
                "Validate result value for parameters(MIN,MAX,false); expected: {}, got: {}",
                min,
                sresult
            );
            check_error(&result, EXPECTED_UNSUPPORTED);

            /* Clear error messages */
            assert_pass!("SDL_ClearError()");

            TestStatus::Completed
        }
    };
}

/*
 * Calls to random boundary number generators for Uint8
 */
unsigned_boundary_test!(
    sdltest_random_boundary_number_uint8,
    random_uint8_boundary_value,
    "SDLTest_RandomUint8BoundaryValue",
    u8::MAX,
    "0xff"
);

/*
 * Calls to random boundary number generators for Uint16
 */
unsigned_boundary_test!(
    sdltest_random_boundary_number_uint16,
    random_uint16_boundary_value,
    "SDLTest_RandomUint16BoundaryValue",
    u16::MAX,
    "0xffff"
);

/*
 * Calls to random boundary number generators for Uint32
 */
unsigned_boundary_test!(
    sdltest_random_boundary_number_uint32,
    random_uint32_boundary_value,
    "SDLTest_RandomUint32BoundaryValue",
    u32::MAX,
    "0xffffffff"
);

/*
 * Calls to random boundary number generators for Uint64
 */
unsigned_boundary_test!(
    sdltest_random_boundary_number_uint64,
    random_uint64_boundary_value,
    "SDLTest_RandomUint64BoundaryValue",
    u64::MAX,
    "0xffffffffffffffff"
);

/*
 * Calls to random boundary number generators for Sint8
 */
signed_boundary_test!(
    sdltest_random_boundary_number_sint8,
    random_sint8_boundary_value,
    "SDLTest_RandomSint8BoundaryValue",
    i8
);

/*
 * Calls to random boundary number generators for Sint16
 */
signed_boundary_test!(
    sdltest_random_boundary_number_sint16,
    random_sint16_boundary_value,
    "SDLTest_RandomSint16BoundaryValue",
    i16
);

/*
 * Calls to random boundary number generators for Sint32
 */
signed_boundary_test!(
    sdltest_random_boundary_number_sint32,
    random_sint32_boundary_value,
    "SDLTest_RandomSint32BoundaryValue",
    i32
);

/*
 * Calls to random boundary number generators for Sint64
 */
signed_boundary_test!(
    sdltest_random_boundary_number_sint64,
    random_sint64_boundary_value,
    "SDLTest_RandomSint64BoundaryValue",
    i64
);

/*
 * Calls to SDLTest_RandomIntegerInRange
 */
fn sdltest_random_integer_in_range(_: &mut TestData) -> TestStatus {
    let long_min = i32::MIN;
    let long_max = i32::MAX;

    /* Standard range */
    let min = random_sint16() as i32;
    let max = min + random_uint8() as i32 + 2;
    let result = random_integer_in_range(min, max);
    assert_pass!("Call to SDLTest_RandomIntegerInRange(min,max)");
    assert_check!(
        min <= result && result <= max,
        "Validated returned value; expected: [{},{}], got: {}",
        min,
        max,
        result
    );

    /* One Range */
    let min = random_sint16() as i32;
    let max = min + 1;
    let result = random_integer_in_range(min, max);
    assert_pass!("Call to SDLTest_RandomIntegerInRange(min,min+1)");
    assert_check!(
        min <= result && result <= max,
        "Validated returned value; expected: [{},{}], got: {}",
        min,
        max,
        result
    );

    /* Zero range */
    let min = random_sint16() as i32;
    let max = min;
    let result = random_integer_in_range(min, max);
    assert_pass!("Call to SDLTest_RandomIntegerInRange(min,min)");
    assert_check!(
        min == result,
        "Validated returned value; expected: {}, got: {}",
        min,
        result
    );

    /* Zero range at zero */
    let result = random_integer_in_range(0, 0);
    assert_pass!("Call to SDLTest_RandomIntegerInRange(0,0)");
    assert_check!(
        result == 0,
        "Validated returned value; expected: 0, got: {}",
        result
    );

    /* Swapped min-max */
    let min = random_sint16() as i32;
    let max = min + random_uint8() as i32 + 2;
    let result = random_integer_in_range(max, min);
    assert_pass!("Call to SDLTest_RandomIntegerInRange(max,min)");
    assert_check!(
        min <= result && result <= max,
        "Validated returned value; expected: [{},{}], got: {}",
        min,
        max,
        result
    );

    /* Range with min at integer limit */
    let min = long_min;
    let max = long_min + random_uint16() as i32;
    let result = random_integer_in_range(min, max);
    assert_pass!("Call to SDLTest_RandomIntegerInRange(SINT32_MIN,...)");
    assert_check!(
        min <= result && result <= max,
        "Validated returned value; expected: [{},{}], got: {}",
        min,
        max,
        result
    );

    /* Range with max at integer limit */
    let min = long_max - random_uint16() as i32;
    let max = long_max;
    let result = random_integer_in_range(min, max);
    assert_pass!("Call to SDLTest_RandomIntegerInRange(...,SINT32_MAX)");
    assert_check!(
        min <= result && result <= max,
        "Validated returned value; expected: [{},{}], got: {}",
        min,
        max,
        result
    );

    /* Full integer range */
    let min = long_min;
    let max = long_max;
    let result = random_integer_in_range(min, max);
    assert_pass!("Call to SDLTest_RandomIntegerInRange(SINT32_MIN,SINT32_MAX)");
    assert_check!(
        min <= result && result <= max,
        "Validated returned value; expected: [{},{}], got: {}",
        min,
        max,
        result
    );

    TestStatus::Completed
}

/// The characters `SDL_iscntrl()` is true for.
fn count_control_characters(s: &str) -> usize {
    s.bytes().filter(|&c| c < 0x20 || c == 0x7F).count()
}

/*
 * Calls to SDLTest_RandomAsciiString
 */
fn sdltest_random_ascii_string(_: &mut TestData) -> TestStatus {
    let result = random_ascii_string();
    assert_pass!("Call to SDLTest_RandomAsciiString()");
    let len = result.len();
    assert_check!(
        (1..=255).contains(&len),
        "Validate that result length; expected: len=[1,255], got: {}",
        len
    );
    let non_ascii_characters = count_control_characters(&result);
    assert_check!(
        non_ascii_characters == 0,
        "Validate that result does not contain non-Ascii characters, got: {}",
        non_ascii_characters
    );
    if non_ascii_characters != 0 {
        log_error!("Invalid result from generator: '{}'", result);
    }

    TestStatus::Completed
}

/*
 * Calls to SDLTest_RandomAsciiStringWithMaximumLength
 */
fn sdltest_random_ascii_string_with_maximum_length(_: &mut TestData) -> TestStatus {
    let expected_error = "Parameter 'maxLength' is invalid";

    let target_len = 16 + random_uint8() as usize;
    let result = random_ascii_string_with_maximum_length(target_len as i32);
    assert_pass!(
        "Call to SDLTest_RandomAsciiStringWithMaximumLength({})",
        target_len
    );
    assert_check!(result.is_ok(), "Validate that result is not NULL");
    if let Ok(result) = &result {
        let len = result.len();
        assert_check!(
            len >= 1 && len <= target_len,
            "Validate that result length; expected: len=[1,{}], got: {}",
            target_len,
            len
        );
        let non_ascii_characters = count_control_characters(result);
        assert_check!(
            non_ascii_characters == 0,
            "Validate that result does not contain non-Ascii characters, got: {}",
            non_ascii_characters
        );
        if non_ascii_characters != 0 {
            log_error!("Invalid result from generator: '{}'", result);
        }
    }

    /* Negative test */
    let target_len = 0;
    let result = random_ascii_string_with_maximum_length(target_len);
    assert_pass!(
        "Call to SDLTest_RandomAsciiStringWithMaximumLength({})",
        target_len
    );
    assert_check!(result.is_err(), "Validate that result is NULL");
    check_error(&result, expected_error);

    /* Clear error messages */
    assert_pass!("SDL_ClearError()");

    TestStatus::Completed
}

/*
 * Calls to SDLTest_RandomAsciiStringOfSize
 */
fn sdltest_random_ascii_string_of_size(_: &mut TestData) -> TestStatus {
    let expected_error = "Parameter 'size' is invalid";

    /* Positive test */
    let target_len = 16 + random_uint8() as usize;
    let result = random_ascii_string_of_size(target_len as i32);
    assert_pass!("Call to SDLTest_RandomAsciiStringOfSize({})", target_len);
    assert_check!(result.is_ok(), "Validate that result is not NULL");
    if let Ok(result) = &result {
        let len = result.len();
        assert_check!(
            len == target_len,
            "Validate that result length; expected: len={}, got: {}",
            target_len,
            len
        );
        let non_ascii_characters = count_control_characters(result);
        assert_check!(
            non_ascii_characters == 0,
            "Validate that result does not contain non-ASCII characters, got: {}",
            non_ascii_characters
        );
        if non_ascii_characters != 0 {
            log_error!("Invalid result from generator: '{}'", result);
        }
    }

    /* Negative test */
    let target_len = 0;
    let result = random_ascii_string_of_size(target_len);
    assert_pass!("Call to SDLTest_RandomAsciiStringOfSize({})", target_len);
    assert_check!(result.is_err(), "Validate that result is NULL");
    check_error(&result, expected_error);

    /* Clear error messages */
    assert_pass!("SDL_ClearError()");

    TestStatus::Completed
}

/* ================= Test References ================== */

/* SDL_test test cases */
static SDLTEST_TEST1: TestCase = TestCase {
    test_case: sdltest_get_fuzzer_invocation_count,
    name: "sdltest_getFuzzerInvocationCount",
    description: "Call to sdltest_GetFuzzerInvocationCount",
    enabled: true,
};

static SDLTEST_TEST2: TestCase = TestCase {
    test_case: sdltest_random_number,
    name: "sdltest_randomNumber",
    description: "Calls to random number generators",
    enabled: true,
};

static SDLTEST_TEST3: TestCase = TestCase {
    test_case: sdltest_random_boundary_number_uint8,
    name: "sdltest_randomBoundaryNumberUint8",
    description: "Calls to random boundary number generators for Uint8",
    enabled: true,
};

static SDLTEST_TEST4: TestCase = TestCase {
    test_case: sdltest_random_boundary_number_uint16,
    name: "sdltest_randomBoundaryNumberUint16",
    description: "Calls to random boundary number generators for Uint16",
    enabled: true,
};

static SDLTEST_TEST5: TestCase = TestCase {
    test_case: sdltest_random_boundary_number_uint32,
    name: "sdltest_randomBoundaryNumberUint32",
    description: "Calls to random boundary number generators for Uint32",
    enabled: true,
};

static SDLTEST_TEST6: TestCase = TestCase {
    test_case: sdltest_random_boundary_number_uint64,
    name: "sdltest_randomBoundaryNumberUint64",
    description: "Calls to random boundary number generators for Uint64",
    enabled: true,
};

static SDLTEST_TEST7: TestCase = TestCase {
    test_case: sdltest_random_boundary_number_sint8,
    name: "sdltest_randomBoundaryNumberSint8",
    description: "Calls to random boundary number generators for Sint8",
    enabled: true,
};

static SDLTEST_TEST8: TestCase = TestCase {
    test_case: sdltest_random_boundary_number_sint16,
    name: "sdltest_randomBoundaryNumberSint16",
    description: "Calls to random boundary number generators for Sint16",
    enabled: true,
};

static SDLTEST_TEST9: TestCase = TestCase {
    test_case: sdltest_random_boundary_number_sint32,
    name: "sdltest_randomBoundaryNumberSint32",
    description: "Calls to random boundary number generators for Sint32",
    enabled: true,
};

static SDLTEST_TEST10: TestCase = TestCase {
    test_case: sdltest_random_boundary_number_sint64,
    name: "sdltest_randomBoundaryNumberSint64",
    description: "Calls to random boundary number generators for Sint64",
    enabled: true,
};

static SDLTEST_TEST11: TestCase = TestCase {
    test_case: sdltest_random_integer_in_range,
    name: "sdltest_randomIntegerInRange",
    description: "Calls to ranged random number generator",
    enabled: true,
};

static SDLTEST_TEST12: TestCase = TestCase {
    test_case: sdltest_random_ascii_string,
    name: "sdltest_randomAsciiString",
    description: "Calls to default ASCII string generator",
    enabled: true,
};

static SDLTEST_TEST13: TestCase = TestCase {
    test_case: sdltest_random_ascii_string_with_maximum_length,
    name: "sdltest_randomAsciiStringWithMaximumLength",
    description: "Calls to random maximum length ASCII string generator",
    enabled: true,
};

static SDLTEST_TEST14: TestCase = TestCase {
    test_case: sdltest_random_ascii_string_of_size,
    name: "sdltest_randomAsciiStringOfSize",
    description: "Calls to fixed size ASCII string generator",
    enabled: true,
};

static SDLTEST_TEST15: TestCase = TestCase {
    test_case: sdltest_generate_run_seed,
    name: "sdltest_generateRunSeed",
    description: "Checks internal harness function SDLTest_GenerateRunSeed",
    enabled: true,
};

/* SDL_test test suite (global) */
static SDLTEST_TEST_SUITE: TestSuite = TestSuite {
    name: "SDLtest",
    test_set_up: None,
    test_cases: &[
        &SDLTEST_TEST1,
        &SDLTEST_TEST2,
        &SDLTEST_TEST3,
        &SDLTEST_TEST4,
        &SDLTEST_TEST5,
        &SDLTEST_TEST6,
        &SDLTEST_TEST7,
        &SDLTEST_TEST8,
        &SDLTEST_TEST9,
        &SDLTEST_TEST10,
        &SDLTEST_TEST11,
        &SDLTEST_TEST12,
        &SDLTEST_TEST13,
        &SDLTEST_TEST14,
        &SDLTEST_TEST15,
    ],
    test_tear_down: None,
};

/// Run the suite as testautomation does: a common state for the options,
/// the runner registered with it.
fn run(args: &[&str]) -> (i32, Vec<String>) {
    let lines: Arc<Mutex<Vec<String>>> = Arc::default();
    let sink = lines.clone();
    sdl3::log::set_priority(sdl3::log::Category::Test, sdl3::log::Priority::Info);
    sdl3::log::set_output(move |record| sink.lock().unwrap().push(record.message.to_owned()));

    let mut state = CommonState::new(
        args.iter().map(|a| (*a).to_owned()).collect(),
        InitFlags::NONE,
    );
    let suites = [&SDLTEST_TEST_SUITE];
    let mut runner = TestSuiteRunner::new(&mut state, &suites);
    assert!(state.default_args());
    let result = runner.execute();
    drop(runner);
    drop(state);

    sdl3::log::reset_output();
    let lines = lines.lock().unwrap().clone();
    (result, lines)
}

#[test]
fn sdltest_suite() {
    let (result, lines) = run(&[
        "testautomation",
        "--no-color",
        "--no-time",
        "--iterations",
        "3",
    ]);
    let failures: Vec<&String> = lines.iter().filter(|l| l.contains("Failed")).collect();
    assert_eq!(result, 0, "{failures:#?}");
    // (15 tests, 3 iterations each)
    assert!(
        lines.contains(&" : Run Summary: Total=45 Passed=45 Failed=0 Skipped=0".to_owned()),
        "{lines:#?}"
    );
    assert_eq!(
        lines
            .iter()
            .filter(|l| l.starts_with(" : >>> Test '"))
            .count(),
        15
    );

    // One test, with a seed: the same run twice.
    let args = [
        "testautomation",
        "--no-color",
        "--no-time",
        "--seed",
        "SEEDSEEDSEEDSEED",
        "--filter",
        "sdltest_randomNumber",
    ];
    let (result, first) = run(&args);
    assert_eq!(result, 0);
    let (_, second) = run(&args);
    let keep = |lines: &[String]| -> Vec<String> {
        lines
            .iter()
            .filter(|l| !l.contains("untime"))
            .cloned()
            .collect()
    };
    assert_eq!(keep(&first), keep(&second));
    assert!(first
        .iter()
        .any(|l| l
            .ends_with("Filtering: running only test 'sdltest_randomNumber' in suite 'SDLtest'")));
}
