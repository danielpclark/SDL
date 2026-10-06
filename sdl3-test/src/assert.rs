// Rust translation of src/test/SDL_test_assert.c and include/SDL3/SDL_test_assert.h
// from Simple DirectMedia Layer.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! Assertion functions of SDL test framework.
//!
//! Assert API for test code and test cases: every assertion is logged and
//! counted, and the harness turns the counts into the test's result. The
//! [`assert_check!`](crate::assert_check), [`assert_pass!`](crate::assert_pass)
//! and [`test_assert!`](crate::test_assert) macros take a format string for
//! the description.
//!
//! Used by the test framework and test cases.

use std::fmt;
use std::sync::atomic::{AtomicI32, Ordering};

use sdl3::log::Priority;

use crate::harness::TestResult;
use crate::internal::{color_end, color_green, color_red, truncate};
use crate::log::log_message;
use crate::MAX_LOGMESSAGE_LENGTH;

/// Fails the assert. Translation of `ASSERT_FAIL`.
pub const ASSERT_FAIL: bool = false;

/// Passes the assert. Translation of `ASSERT_PASS`.
pub const ASSERT_PASS: bool = true;

/* ! counts the failed asserts */
static ASSERTS_FAILED: AtomicI32 = AtomicI32::new(0);

/* ! counts the passed asserts */
static ASSERTS_PASSED: AtomicI32 = AtomicI32::new(0);

/// Translation of `SDLTest_LogAssertMessage()`.
fn log_assert_message(success: bool, assertion: &str) {
    let (priority, color, message) = if success {
        (Priority::Info, color_green(), "Passed")
    } else {
        (Priority::Error, color_red(), "Failed")
    };
    log_message(
        priority,
        &format!("Assert '{assertion}': {color}{message}{}", color_end()),
    );
}

/// Print assert description into a buffer.
fn description(args: fmt::Arguments<'_>) -> String {
    truncate(fmt::format(args), MAX_LOGMESSAGE_LENGTH)
}

/// Assert that logs and break execution flow on failures (i.e. for harness
/// errors): a failure is counted, logged, and then raised as an SDL
/// assertion (`SDL_assert`). Translation of `SDLTest_Assert()` (see
/// [`test_assert!`](crate::test_assert) for a formatted description).
pub fn assert(assert_condition: bool, assert_description: &str) {
    __assert(assert_condition, format_args!("{assert_description}"));
}

/// Assert that logs but does not break execution flow on failures (i.e. for
/// test cases). Updates assertion counters. Returns `assert_condition`, so
/// it can be used externally to break execution flow if desired.
/// Translation of `SDLTest_AssertCheck()` (see
/// [`assert_check!`](crate::assert_check) for a formatted description).
pub fn check(assert_condition: bool, assert_description: &str) -> bool {
    __check(assert_condition, format_args!("{assert_description}"))
}

/// Explicitly passing Assert that logs (i.e. for test cases), without
/// checking an assertion condition. Updates assertion counter. Translation of
/// `SDLTest_AssertPass()` (see [`assert_pass!`](crate::assert_pass) for a
/// formatted description).
pub fn pass(assert_description: &str) {
    __pass(format_args!("{assert_description}"));
}

#[doc(hidden)]
pub fn __assert(assert_condition: bool, args: fmt::Arguments<'_>) {
    /* Print assert description into a buffer */
    let log_message = description(args);

    /* Log, then assert and break on failure */
    let checked = __check(assert_condition, format_args!("{log_message}"));
    sdl3::sdl_assert!(checked);
}

#[doc(hidden)]
pub fn __check(assert_condition: bool, args: fmt::Arguments<'_>) -> bool {
    /* Print assert description into a buffer */
    let log_message = description(args);

    /* Log pass or fail message */
    if assert_condition == ASSERT_FAIL {
        ASSERTS_FAILED.fetch_add(1, Ordering::Relaxed);
        log_assert_message(false, &log_message);
    } else {
        ASSERTS_PASSED.fetch_add(1, Ordering::Relaxed);
        log_assert_message(true, &log_message);
    }

    assert_condition
}

#[doc(hidden)]
pub fn __pass(args: fmt::Arguments<'_>) {
    /* Print assert description into a buffer */
    let log_message = description(args);

    /* Log pass message */
    ASSERTS_PASSED.fetch_add(1, Ordering::Relaxed);
    log_assert_message(true, &log_message);
}

/// `test_assert!(condition, fmt, ...)`: assert that logs and breaks
/// execution flow on failures. Translation of `SDLTest_Assert()`.
#[macro_export]
macro_rules! test_assert {
    ($cond:expr, $($arg:tt)*) => {
        $crate::assert::__assert($cond, ::std::format_args!($($arg)*))
    };
}

/// `assert_check!(condition, fmt, ...)`: assert for test cases that logs
/// but does not break execution flow on failures, returning the condition.
/// Translation of `SDLTest_AssertCheck()`.
#[macro_export]
macro_rules! assert_check {
    ($cond:expr, $($arg:tt)*) => {
        $crate::assert::__check($cond, ::std::format_args!($($arg)*))
    };
}

/// `assert_pass!(fmt, ...)`: explicitly pass without checking an assertion
/// condition. Translation of `SDLTest_AssertPass()`.
#[macro_export]
macro_rules! assert_pass {
    ($($arg:tt)*) => {
        $crate::assert::__pass(::std::format_args!($($arg)*))
    };
}

pub use crate::{assert_check, assert_pass, test_assert};

/// Resets the assert summary counters to zero. Translation of
/// `SDLTest_ResetAssertSummary()`.
pub fn reset_summary() {
    ASSERTS_PASSED.store(0, Ordering::Relaxed);
    ASSERTS_FAILED.store(0, Ordering::Relaxed);
}

/// The assertions passed and failed since the last [`reset_summary`]
/// (what the C code keeps in its static counters).
pub fn summary() -> (i32, i32) {
    (
        ASSERTS_PASSED.load(Ordering::Relaxed),
        ASSERTS_FAILED.load(Ordering::Relaxed),
    )
}

/// Logs summary of all assertions (total, pass, fail) since last reset
/// as INFO (failed==0) or ERROR (failed > 0). Translation of
/// `SDLTest_LogAssertSummary()`.
pub fn log_summary() {
    let (asserts_passed, asserts_failed) = summary();
    let total_asserts = asserts_passed.wrapping_add(asserts_failed);
    let success = asserts_failed == 0;

    log_message(
        if success {
            Priority::Info
        } else {
            Priority::Error
        },
        &format!(
            "Assert Summary: Total={total_asserts} {}Passed={asserts_passed}{} {}Failed={asserts_failed}{}",
            color_green(),
            color_end(),
            if success { color_green() } else { color_red() },
            color_end()
        ),
    );
}

/// Converts the current assert state into a test result:
/// [`TestResult::Passed`], [`TestResult::Failed`] or
/// [`TestResult::NoAssert`]. Translation of
/// `SDLTest_AssertSummaryToTestResult()`.
pub fn summary_to_test_result() -> TestResult {
    let (asserts_passed, asserts_failed) = summary();
    if asserts_failed > 0 {
        TestResult::Failed
    } else if asserts_passed > 0 {
        TestResult::Passed
    } else {
        TestResult::NoAssert
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::{test_lock, Capture};

    #[test]
    fn counting_and_messages() {
        let _l = test_lock();
        let capture = Capture::new();
        reset_summary();
        assert_eq!(summary_to_test_result(), TestResult::NoAssert);

        assert!(crate::assert_check!(1 + 1 == 2, "one plus {} is two", 1));
        assert_eq!(summary_to_test_result(), TestResult::Passed);
        crate::assert_pass!("explicit {}", "pass");
        assert!(!check(false, "nope"));
        assert_eq!(summary(), (2, 1));
        assert_eq!(summary_to_test_result(), TestResult::Failed);
        log_summary();
        reset_summary();
        assert_eq!(summary(), (0, 0));
        assert_eq!(summary_to_test_result(), TestResult::NoAssert);
        log_summary();
        crate::test_assert!(true, "harness {}", "ok");
        assert_eq!(summary(), (1, 0));

        assert_eq!(
            capture.lines(),
            [
                (
                    sdl3::log::Category::Test,
                    Priority::Info,
                    " : Assert 'one plus 1 is two': Passed".to_owned()
                ),
                (
                    sdl3::log::Category::Test,
                    Priority::Info,
                    " : Assert 'explicit pass': Passed".to_owned()
                ),
                (
                    sdl3::log::Category::Test,
                    Priority::Error,
                    ": Assert 'nope': Failed".to_owned()
                ),
                (
                    sdl3::log::Category::Test,
                    Priority::Error,
                    ": Assert Summary: Total=3 Passed=2 Failed=1".to_owned()
                ),
                (
                    sdl3::log::Category::Test,
                    Priority::Info,
                    " : Assert Summary: Total=0 Passed=0 Failed=0".to_owned()
                ),
                (
                    sdl3::log::Category::Test,
                    Priority::Info,
                    " : Assert 'harness ok': Passed".to_owned()
                ),
            ]
        );
        reset_summary();
    }

    #[test]
    fn colors() {
        let _l = test_lock();
        let capture = Capture::new();
        crate::internal::set_color(true);
        reset_summary();
        pass("p");
        check(false, "f");
        log_summary();
        reset_summary();
        assert_eq!(
            capture.messages(),
            [
                " : Assert 'p': \x1b[0;32mPassed\x1b[0m",
                ": Assert 'f': \x1b[0;31mFailed\x1b[0m",
                ": Assert Summary: Total=2 \x1b[0;32mPassed=1\x1b[0m \x1b[0;31mFailed=1\x1b[0m",
            ]
        );
    }
}
