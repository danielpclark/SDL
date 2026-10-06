// Rust translation of src/test/SDL_test_harness.c and include/SDL3/SDL_test_harness.h
// from Simple DirectMedia Layer.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! Test suite related functions of SDL test framework.
//!
//! Defines types for test case definitions and the test execution harness
//! API: suites of test cases, run by a [`TestSuiteRunner`] with a run seed
//! (from which each test's fuzzer execution key is made), an optional
//! filter, iterations and random order, logging each test's result and a
//! summary.
//!
//! Based on original GSOC code by Markus Kauppila <markus.kauppila@gmail.com>
//!
//! ```
//! use sdl3_test::harness::{TestCase, TestData, TestStatus, TestSuite, TestSuiteRunner};
//!
//! fn two_plus_two(_: &mut TestData) -> TestStatus {
//!     sdl3_test::assert_check!(2 + 2 == 4, "Verify 2 + 2 == {}", 4);
//!     TestStatus::Completed
//! }
//!
//! static TEST1: TestCase = TestCase {
//!     test_case: two_plus_two,
//!     name: "two_plus_two",
//!     description: "Adds two and two",
//!     enabled: true,
//! };
//!
//! static SUITE: TestSuite = TestSuite {
//!     name: "Arithmetic",
//!     test_set_up: None,
//!     test_cases: &[&TEST1],
//!     test_tear_down: None,
//! };
//!
//! let suites = [&SUITE];
//! let mut runner = TestSuiteRunner::new(&suites);
//! runner.set_seed(Some("ABCDEFGHIJKLMNOP"));
//! assert_eq!(runner.execute(), 0);
//! ```

use std::any::Any;
use std::cmp::Ordering;
use std::time::Duration;

use sdl3::log::Priority;
use sdl3::stdlib::string::{strcasecmp, strtol, strtoull};
use sdl3::stdlib::Rng;

use crate::internal::{color_blue, color_end, color_green, color_red, color_yellow};
use crate::log::log_message;
use crate::md5::Md5Context;
use crate::{assert, fuzzer, log, log_error};

/* ! Definition of all the possible test return values of the test case method */

/// What a test case function returns. Translation of the `TEST_ABORTED`,
/// `TEST_STARTED`, `TEST_COMPLETED` and `TEST_SKIPPED` values.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum TestStatus {
    /// The test was aborted early (`TEST_ABORTED`, -1).
    Aborted = -1,
    /// The test started but did not complete (`TEST_STARTED`, 0).
    Started = 0,
    /// The test completed; its asserts decide its result (`TEST_COMPLETED`, 1).
    Completed = 1,
    /// The test skipped itself (`TEST_SKIPPED`, 2).
    Skipped = 2,
}

/* ! Definition of all the possible test results for the harness */

/// The result of running a test. Translation of the `TEST_RESULT_*` values.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum TestResult {
    /// `TEST_RESULT_PASSED` (0)
    Passed = 0,
    /// `TEST_RESULT_FAILED` (1)
    Failed = 1,
    /// `TEST_RESULT_NO_ASSERT` (2): completed without any assert.
    NoAssert = 2,
    /// `TEST_RESULT_SKIPPED` (3)
    Skipped = 3,
    /// `TEST_RESULT_SETUP_FAILURE` (4): the suite's setup failed an assert.
    SetupFailure = 4,
}

/// The data a suite's setup function makes for each of its tests, given to
/// the test case and the teardown function, and dropped after them. It
/// starts as `None`. Translation of the `void *arg` of the test functions.
pub type TestData = Option<Box<dyn Any>>;

/// Function pointer to a test case setup function (run before every test).
/// Translation of `SDLTest_TestCaseSetUpFp`.
pub type TestCaseSetUpFn = fn(&mut TestData);

/// Function pointer to a test case function. Translation of `SDLTest_TestCaseFp`.
pub type TestCaseFn = fn(&mut TestData) -> TestStatus;

/// Function pointer to a test case teardown function (run after every
/// test). Translation of `SDLTest_TestCaseTearDownFp`.
pub type TestCaseTearDownFn = fn(&mut TestData);

/// Holds information about a single test case. Translation of
/// `SDLTest_TestCaseReference`.
#[derive(Clone, Copy, Debug)]
pub struct TestCase {
    /// Func2Stress
    pub test_case: TestCaseFn,
    /// Short name (or function name) "Func2Stress"
    pub name: &'static str,
    /// Long name or full description "This test pushes func2() to the limit."
    pub description: &'static str,
    /// Set to `true` (`TEST_ENABLED`) or `false` (`TEST_DISABLED`; the test
    /// won't be run)
    pub enabled: bool,
}

/// Holds information about a test suite (multiple test cases). Translation
/// of `SDLTest_TestSuiteReference`.
#[derive(Clone, Copy, Debug)]
pub struct TestSuite {
    /// "PlatformSuite"
    pub name: &'static str,
    /// The function that is run before each test. `None` skips.
    pub test_set_up: Option<TestCaseSetUpFn>,
    /// The test cases that are run as part of the suite.
    pub test_cases: &'static [&'static TestCase],
    /// The function that is run after each test. `None` skips.
    pub test_tear_down: Option<TestCaseTearDownFn>,
}

/// Translation of `SDLTest_LogSummary()`.
fn log_summary(success: bool, name: &str, total: i32, passed: i32, failed: i32, skipped: i32) {
    log_message(
        if success {
            Priority::Info
        } else {
            Priority::Error
        },
        &format!(
            "{name} Summary: Total={total} {}Passed={passed}{} {}Failed={failed}{} {}Skipped={skipped}{}",
            color_green(),
            color_end(),
            if success { color_green() } else { color_red() },
            color_end(),
            color_blue(),
            color_end()
        ),
    );
}

/// Translation of `SDLTest_LogFinalResult()`.
fn log_final_result(
    success: bool,
    stage: &str,
    name: &str,
    color_message: Option<&str>,
    message: &str,
) {
    let priority = if success {
        Priority::Info
    } else {
        Priority::Error
    };
    log_message(
        priority,
        &format!(
            "{}>>> {stage} '{name}':{} {}{message}{}",
            color_yellow(),
            color_end(),
            color_message.unwrap_or(""),
            if color_message.is_some() {
                color_end()
            } else {
                ""
            }
        ),
    );
}

/// Holds information about the execution of test suites: the suites, and
/// the run seed, execution key, filter, iterations and order to run them
/// with. Translation of `SDLTest_TestSuiteRunner`.
#[derive(Debug)]
pub struct TestSuiteRunner<'a> {
    test_suites: &'a [&'a TestSuite],
    run_seed: Option<String>,
    exec_key: u64,
    filter: Option<String>,
    test_iterations: i32,
    random_order: bool,
}

/* ! Timeout for single test case execution */
static TEST_CASE_TIMEOUT: u32 = 3600;

static COMMON_HARNESS_USAGE: [&str; 5] = [
    "[--iterations #]",
    "[--execKey #]",
    "[--seed string]",
    "[--filter suite_name|test_name]",
    "[--random-order]",
];

/// Generates a random run seed string for the harness. The generated seed
/// will contain `length` alphanumeric characters (0-9A-Z).
///
/// Returns `None` (with an error logged) if `length` is not above 0.
/// Translation of `SDLTest_GenerateRunSeed()`.
pub fn generate_run_seed(length: i32) -> Option<String> {
    let mut random_context = Rng::from_state(sdl3::timer::performance_counter());

    /* Sanity check input */
    if length <= 0 {
        log_error!("The length of the harness seed must be >0.");
        return None;
    }

    /* Generate a random string of alphanumeric characters */
    let mut buffer = String::with_capacity(length as usize);
    for _ in 0..length {
        let v = random_context.below(10 + 26);
        let ch = if v < 10 {
            (b'0' + v as u8) as char
        } else {
            (b'A' + (v - 10) as u8) as char
        };
        buffer.push(ch);
    }

    Some(buffer)
}

/// Generates an execution key for the fuzzer: half of the MD5 digest of the
/// run seed, suite name, test name and iteration (and the string's
/// terminating NUL), as a native-endian `u64`.
///
/// Returns 0 (with an error logged) if a string is empty or the iteration
/// is not above 0. Translation of `SDLTest_GenerateExecKey()`.
pub fn generate_exec_key(run_seed: &str, suite_name: &str, test_name: &str, iteration: i32) -> u64 {
    if run_seed.is_empty() {
        log_error!("Invalid runSeed string.");
        return 0;
    }

    if suite_name.is_empty() {
        log_error!("Invalid suiteName string.");
        return 0;
    }

    if test_name.is_empty() {
        log_error!("Invalid testName string.");
        return 0;
    }

    if iteration <= 0 {
        log_error!("Invalid iteration count.");
        return 0;
    }

    /* Convert iteration number into a string */
    /* Combine the parameters into single string */
    // (with the NUL that ends it in C, which is hashed too)
    let buffer = format!("{run_seed}{suite_name}{test_name}{iteration}\0");

    /* Hash string and use half of the digest as 64bit exec key */
    let mut md5_context = Md5Context::new();
    md5_context.update(buffer.as_bytes());
    let digest = md5_context.finalize();
    let mut key = [0u8; 8];
    key.copy_from_slice(&digest[..8]);

    u64::from_ne_bytes(key)
}

/// Set timeout handler for test: a timer that calls [`bail_out`] after
/// `timeout` seconds. Returns `None` on failure. Translation of
/// `SDLTest_SetTestTimeout()`.
fn set_test_timeout(timeout: i32) -> Option<sdl3::timer::Timer> {
    if timeout < 0 {
        log_error!("Timeout value must be bigger than zero.");
        return None;
    }

    /* Set timer */
    let timeout_in_milliseconds = (timeout as u32).wrapping_mul(1000);
    match sdl3::timer::Timer::new(
        Duration::from_millis(timeout_in_milliseconds as u64),
        |_| bail_out(),
    ) {
        Ok(timer) => Some(timer),
        Err(error) => {
            log_error!("Creation of SDL timer failed: {}", error.message());
            None
        }
    }
}

/// Timeout handler. Aborts test run and exits harness process. Translation
/// of `SDLTest_BailOut()`.
fn bail_out() -> ! {
    log_error!("TestCaseTimeout timer expired. Aborting test run.");
    std::process::exit(TestStatus::Aborted as i32); /* bail out from the test */
}

/// Execute a test using the given execution key, forcing a disabled test to
/// run if `force_test_run`. Returns the test case result. Translation of
/// `SDLTest_RunTest()`.
fn run_test(
    test_suite: &TestSuite,
    test_case: &TestCase,
    exec_key: u64,
    force_test_run: bool,
) -> TestResult {
    let mut data: TestData = None;

    if !test_case.enabled && !force_test_run {
        log_final_result(true, "Test", test_case.name, None, "Skipped (Disabled)");
        return TestResult::Skipped;
    }

    /* Initialize fuzzer */
    fuzzer::init(exec_key);

    /* Reset assert tracker */
    assert::reset_summary();

    /* Set timeout timer */
    // (cancelled when it drops, which also covers the early return below,
    // where upstream leaves the timer running: Note (upstream))
    let timer = set_test_timeout(TEST_CASE_TIMEOUT as i32);

    /* Maybe run suite initializer function */
    if let Some(test_set_up) = test_suite.test_set_up {
        test_set_up(&mut data);
        if assert::summary_to_test_result() == TestResult::Failed {
            log_final_result(
                false,
                "Suite Setup",
                test_suite.name,
                Some(color_red()),
                "Failed",
            );
            return TestResult::SetupFailure;
        }
    }

    /* Run test case function */
    let test_case_result = (test_case.test_case)(&mut data);

    /* Convert test execution result into harness result */
    let test_result = match test_case_result {
        /* Test was programmatically skipped */
        TestStatus::Skipped => TestResult::Skipped,
        /* Test did not return a TEST_COMPLETED value; assume it failed */
        TestStatus::Started => TestResult::Failed,
        /* Test was aborted early; assume it failed */
        TestStatus::Aborted => TestResult::Failed,
        /* Perform failure analysis based on asserts */
        TestStatus::Completed => assert::summary_to_test_result(),
    };

    /* Maybe run suite cleanup function (ignore failed asserts) */
    if let Some(test_tear_down) = test_suite.test_tear_down {
        test_tear_down(&mut data);
    }
    drop(data);

    /* Cancel timeout timer */
    if let Some(timer) = timer {
        timer.cancel();
    }

    /* Report on asserts and fuzzer usage */
    let fuzzer_count = fuzzer::invocation_count();
    if fuzzer_count > 0 {
        log!("Fuzzer invocations: {}", fuzzer_count);
    }

    /* Final log based on test execution result */
    match test_case_result {
        TestStatus::Skipped => {
            /* Test was programmatically skipped */
            log_final_result(
                true,
                "Test",
                test_case.name,
                Some(color_blue()),
                "Skipped (Programmatically)",
            );
        }
        TestStatus::Started => {
            /* Test did not return a TEST_COMPLETED value; assume it failed */
            log_final_result(
                false,
                "Test",
                test_case.name,
                Some(color_red()),
                "Skipped (test started, but did not return TEST_COMPLETED)",
            );
        }
        TestStatus::Aborted => {
            /* Test was aborted early; assume it failed */
            log_final_result(
                false,
                "Test",
                test_case.name,
                Some(color_red()),
                "Failed (Aborted)",
            );
        }
        TestStatus::Completed => {
            assert::log_summary();
        }
    }

    test_result
}

/* Gets a timer value in seconds */
fn get_clock() -> f32 {
    sdl3::timer::performance_counter() as f32 / sdl3::timer::performance_frequency() as f32
}

/// `SDL_strcasecmp(a, b) == 0`.
fn equal_ignoring_case(a: &str, b: &str) -> bool {
    strcasecmp(a, b) == Ordering::Equal
}

impl<'a> TestSuiteRunner<'a> {
    /// Create a new test suite runner, that will execute the given test
    /// suites. Translation of `SDLTest_CreateTestSuiteRunner()` (the command
    /// line options go through [`parse_argument`](Self::parse_argument));
    /// dropping it is `SDLTest_DestroyTestSuiteRunner()`.
    pub fn new(test_suites: &'a [&'a TestSuite]) -> TestSuiteRunner<'a> {
        TestSuiteRunner {
            test_suites,
            run_seed: None,
            exec_key: 0,
            filter: None,
            test_iterations: 0,
            random_order: false,
        }
    }

    /// Set the run seed (`--seed`); without one (or with an empty one), a
    /// random one is made for each run.
    pub fn set_seed(&mut self, run_seed: Option<&str>) {
        self.run_seed = run_seed.map(str::to_owned);
    }

    /// Set the execution key (`--execKey`) every test's fuzzer starts from;
    /// 0 (the default) makes one from the run seed and the test.
    pub fn set_exec_key(&mut self, exec_key: u64) {
        self.exec_key = exec_key;
    }

    /// Run only the suite, or else the test, of this name, compared
    /// ignoring case (`--filter`).
    pub fn set_filter(&mut self, filter: Option<&str>) {
        self.filter = filter.map(str::to_owned);
    }

    /// Run each test this many times, at least once (`--iterations`).
    pub fn set_iterations(&mut self, test_iterations: i32) {
        self.test_iterations = test_iterations;
    }

    /// Run the suites, and the tests in each suite, in a random order
    /// (`--random-order`).
    pub fn set_random_order(&mut self, random_order: bool) {
        self.random_order = random_order;
    }

    /// The usage of the harness's command line options, printed with `--help`.
    /// Translation of `common_harness_usage`.
    pub fn usage() -> &'static [&'static str] {
        &COMMON_HARNESS_USAGE
    }

    /// Parse the harness's command line option at `argv[index]`
    /// (`--iterations #`, `--execKey #`, `--seed string`, `--filter name`,
    /// `--random-order`), returning the number of arguments taken, or 0 when
    /// it isn't one of these. Translation of `SDLTest_TestSuiteCommonArg()`.
    pub fn parse_argument(&mut self, argv: &[impl AsRef<str>], index: usize) -> i32 {
        let Some(arg) = argv.get(index).map(AsRef::as_ref) else {
            return 0;
        };
        let next = argv.get(index + 1).map(AsRef::as_ref);

        if equal_ignoring_case(arg, "--iterations") {
            if let Some(next) = next {
                // (SDL_atoi())
                self.test_iterations = strtol(next, 10).0 as i32;
                if self.test_iterations < 1 {
                    self.test_iterations = 1;
                }
                return 2;
            }
        } else if equal_ignoring_case(arg, "--execKey") {
            if let Some(next) = next {
                // (SDL_sscanf(..., "%" SDL_PRIu64): the value is kept when
                // there is no number)
                let (value, consumed) = strtoull(next, 10);
                if consumed > 0 {
                    self.exec_key = value;
                }
                return 2;
            }
        } else if equal_ignoring_case(arg, "--seed") {
            if let Some(next) = next {
                self.run_seed = Some(next.to_owned());
                return 2;
            }
        } else if equal_ignoring_case(arg, "--filter") {
            if let Some(next) = next {
                self.filter = Some(next.to_owned());
                return 2;
            }
        } else if equal_ignoring_case(arg, "--random-order") {
            self.random_order = true;
            return 1;
        }
        0
    }

    /// Execute a test suite using the configured run seed, execution key,
    /// filter, etc.
    ///
    /// The filter string is matched to the suite name (full comparison) to
    /// select a single suite, or if no suite matches, it is matched to the
    /// test names (full comparison) to select a single test.
    ///
    /// Returns the test run result: 0 when all tests passed, 1 if any tests
    /// failed, 2 if the filter matched nothing (or no seed could be made)
    /// and -1 when there are no tests. Translation of
    /// `SDLTest_ExecuteTestSuiteRunner()`.
    pub fn execute(&mut self) -> i32 {
        let mut total_number_of_tests = 0;
        let mut failed_tests: Vec<&TestCase> = Vec::new();
        let mut suite_filter = false;
        let mut suite_filter_name: Option<&str> = None;
        let mut test_filter = false;
        let mut test_filter_name: Option<&str> = None;
        let mut force_test_run = false;
        let mut test_result = TestResult::Passed;

        /* Sanitize test iterations */
        if self.test_iterations < 1 {
            self.test_iterations = 1;
        }

        /* Generate run see if we don't have one already */
        let run_seed = match self.run_seed.as_deref() {
            Some(run_seed) if !run_seed.is_empty() => run_seed.to_owned(),
            _ => match generate_run_seed(16) {
                Some(run_seed) => run_seed,
                None => {
                    log_error!("Generating a random seed failed");
                    return 2;
                }
            },
        };

        /* Reset per-run counters */
        let mut total_test_failed_count = 0;
        let mut total_test_passed_count = 0;
        let mut total_test_skipped_count = 0;

        /* Take time - run start */
        let run_start_seconds = get_clock();

        /* Log run with fuzzer parameters */
        log!("::::: Test Run /w seed '{}' started\n", run_seed);

        /* Count the total number of tests */
        for test_suite in self.test_suites {
            total_number_of_tests += test_suite.test_cases.len();
        }

        if total_number_of_tests == 0 {
            log_error!("No tests to run?");
            return -1;
        }

        /* Pre-allocate an array for tracking failed tests (potentially all test cases) */
        failed_tests.reserve(total_number_of_tests);

        /* Initialize filtering */
        if let Some(filter) = self.filter.as_deref().filter(|f| !f.is_empty()) {
            /* Loop over all suites to check if we have a filter match */
            'suites: for test_suite in self.test_suites {
                if equal_ignoring_case(filter, test_suite.name) {
                    /* Matched a suite name */
                    suite_filter = true;
                    suite_filter_name = Some(test_suite.name);
                    log!("Filtering: running only suite '{}'", test_suite.name);
                    break;
                }

                /* Within each suite, loop over all test cases to check if we have a filter match */
                for test_case in test_suite.test_cases {
                    if equal_ignoring_case(filter, test_case.name) {
                        /* Matched a test name */
                        suite_filter = true;
                        suite_filter_name = Some(test_suite.name);
                        test_filter = true;
                        test_filter_name = Some(test_case.name);
                        log!(
                            "Filtering: running only test '{}' in suite '{}'",
                            test_case.name,
                            test_suite.name
                        );
                        // (upstream's loop over the suites stops once the
                        // test filter is set)
                        break 'suites;
                    }
                }
            }

            if !suite_filter && !test_filter {
                log_error!("Filter '{}' did not match any test suite/case.", filter);
                for test_suite in self.test_suites {
                    log!("Test suite: {}", test_suite.name);

                    /* Within each suite, loop over all test cases to check if we have a filter match */
                    for test_case in test_suite.test_cases {
                        log!(
                            "      test: {}{}",
                            test_case.name,
                            if test_case.enabled { "" } else { " (disabled)" }
                        );
                    }
                }
                log!("Exit code: 2");
                return 2;
            }

            self.random_order = false;
        }

        /* Number of test suites */
        let mut nb_suites = self.test_suites.len() as i32;

        let mut array_suites: Vec<usize> = (0..self.test_suites.len()).collect();

        /* Mix the list of suites to run them in random order */
        {
            /* Exclude last test "subsystemsTestSuite" which is said to interfere with other tests */
            nb_suites -= 1;

            let exec_key = if self.exec_key != 0 {
                self.exec_key
            } else {
                /* dummy values to have random numbers working */
                generate_exec_key(&run_seed, "random testSuites", "initialisation", 1)
            };

            /* Initialize fuzzer */
            fuzzer::init(exec_key);

            for _ in 0..100 {
                let a = fuzzer::random_integer_in_range(0, nb_suites - 1);
                let b = fuzzer::random_integer_in_range(0, nb_suites - 1);
                /*
                 * NB: prevent swapping here to make sure the tests start with the same
                 * random seed (whether they are run in order or not).
                 * So we consume same number of SDLTest_RandomIntegerInRange() in all cases.
                 *
                 * If some random value were used at initialization before the tests start, the --seed wouldn't do the same with or without randomOrder.
                 */
                /* Swap */
                // Note (upstream): with a single suite the range is
                // [0, -1], and C swaps with arraySuites[-1]; here an index
                // out of the list isn't swapped.
                if self.random_order && a >= 0 && b >= 0 {
                    array_suites.swap(a as usize, b as usize);
                }
            }

            /* re-add last lest */
            nb_suites += 1;
        }

        /* Loop over all suites */
        for &suite_index in array_suites.iter().take(nb_suites as usize) {
            let test_suite = self.test_suites[suite_index];
            let current_suite_name = test_suite.name;
            let suite_counter = suite_index + 1;

            /* Filter suite if flag set and we have a name */
            if suite_filter
                && suite_filter_name.is_some_and(|name| !equal_ignoring_case(name, test_suite.name))
            {
                /* Skip suite */
                log!(
                    "===== Test Suite {}: '{}' {}skipped{}\n",
                    suite_counter,
                    current_suite_name,
                    color_blue(),
                    color_end()
                );
            } else {
                let nb_test_cases = test_suite.test_cases.len() as i32;
                let mut array_test_cases: Vec<usize> = (0..test_suite.test_cases.len()).collect();

                /* Mix the list of testCases to run them in random order */
                for _ in 0..100 {
                    let a = fuzzer::random_integer_in_range(0, nb_test_cases - 1);
                    let b = fuzzer::random_integer_in_range(0, nb_test_cases - 1);
                    /* Swap */
                    /* See previous note */
                    // Note (upstream): as above, for a suite without tests.
                    if self.random_order && a >= 0 && b >= 0 {
                        array_test_cases.swap(a as usize, b as usize);
                    }
                }

                /* Reset per-suite counters */
                let mut test_failed_count = 0;
                let mut test_passed_count = 0;
                let mut test_skipped_count = 0;

                /* Take time - suite start */
                let suite_start_seconds = get_clock();

                /* Log suite started */
                log!(
                    "===== Test Suite {}: '{}' started\n",
                    suite_counter,
                    current_suite_name
                );

                /* Loop over all test cases */
                for &test_index in &array_test_cases {
                    let test_case = test_suite.test_cases[test_index];
                    let current_test_name = test_case.name;
                    let test_counter = test_index + 1;

                    /* Filter tests if flag set and we have a name */
                    if test_filter
                        && test_filter_name
                            .is_some_and(|name| !equal_ignoring_case(name, test_case.name))
                    {
                        /* Skip test */
                        log!(
                            "===== Test Case {}.{}: '{}' {}skipped{}\n",
                            suite_counter,
                            test_counter,
                            current_test_name,
                            color_blue(),
                            color_end()
                        );
                    } else {
                        /* Override 'disabled' flag if we specified a test filter (i.e. force run for debugging) */
                        if test_filter && !test_case.enabled {
                            log!("Force run of disabled test since test filter was set");
                            force_test_run = true;
                        }

                        /* Take time - test start */
                        let test_start_seconds = get_clock();

                        /* Log test started */
                        log!(
                            "{}----- Test Case {}.{}: '{}' started{}",
                            color_yellow(),
                            suite_counter,
                            test_counter,
                            current_test_name,
                            color_end()
                        );
                        if !test_case.description.is_empty() {
                            log!("Test Description: '{}'", test_case.description);
                        }

                        /* Loop over all iterations */
                        let mut iteration_counter = 0;
                        while iteration_counter < self.test_iterations {
                            iteration_counter += 1;

                            let exec_key = if self.exec_key != 0 {
                                self.exec_key
                            } else {
                                generate_exec_key(
                                    &run_seed,
                                    test_suite.name,
                                    test_case.name,
                                    iteration_counter,
                                )
                            };

                            log!("Test Iteration {}: execKey {}", iteration_counter, exec_key);
                            test_result = run_test(test_suite, test_case, exec_key, force_test_run);

                            if test_result == TestResult::Passed {
                                test_passed_count += 1;
                                total_test_passed_count += 1;
                            } else if test_result == TestResult::Skipped {
                                test_skipped_count += 1;
                                total_test_skipped_count += 1;
                            } else {
                                test_failed_count += 1;
                                total_test_failed_count += 1;
                            }
                        }

                        /* Take time - test end */
                        let test_end_seconds = get_clock();
                        let mut runtime = test_end_seconds - test_start_seconds;
                        if runtime < 0.0 {
                            runtime = 0.0;
                        }

                        if self.test_iterations > 1 {
                            /* Log test runtime */
                            log!(
                                "Runtime of {} iterations: {:.1} sec",
                                self.test_iterations,
                                runtime
                            );
                            log!(
                                "Average Test runtime: {:.5} sec",
                                runtime / self.test_iterations as f32
                            );
                        } else {
                            /* Log test runtime */
                            log!("Total Test runtime: {:.1} sec", runtime);
                        }

                        /* Log final test result */
                        match test_result {
                            TestResult::Passed => log_final_result(
                                true,
                                "Test",
                                current_test_name,
                                Some(color_green()),
                                "Passed",
                            ),
                            TestResult::Failed => log_final_result(
                                false,
                                "Test",
                                current_test_name,
                                Some(color_red()),
                                "Failed",
                            ),
                            TestResult::NoAssert => log_final_result(
                                false,
                                "Test",
                                current_test_name,
                                Some(color_blue()),
                                "No Asserts",
                            ),
                            TestResult::Skipped | TestResult::SetupFailure => {}
                        }

                        /* Collect failed test case references for repro-step display */
                        if test_result == TestResult::Failed {
                            failed_tests.push(test_case);
                        }
                    }
                }

                /* Take time - suite end */
                let suite_end_seconds = get_clock();
                let mut runtime = suite_end_seconds - suite_start_seconds;
                if runtime < 0.0 {
                    runtime = 0.0;
                }

                /* Log suite runtime */
                log!("Total Suite runtime: {:.1} sec", runtime);

                /* Log summary and final Suite result */
                let count_sum = test_passed_count + test_failed_count + test_skipped_count;
                if test_failed_count == 0 {
                    log_summary(
                        true,
                        "Suite",
                        count_sum,
                        test_passed_count,
                        test_failed_count,
                        test_skipped_count,
                    );
                    log_final_result(
                        true,
                        "Suite",
                        current_suite_name,
                        Some(color_green()),
                        "Passed",
                    );
                } else {
                    log_summary(
                        false,
                        "Suite",
                        count_sum,
                        test_passed_count,
                        test_failed_count,
                        test_skipped_count,
                    );
                    log_final_result(
                        false,
                        "Suite",
                        current_suite_name,
                        Some(color_red()),
                        "Failed",
                    );
                }
            }
        }

        /* Take time - run end */
        let run_end_seconds = get_clock();
        let mut runtime = run_end_seconds - run_start_seconds;
        if runtime < 0.0 {
            runtime = 0.0;
        }

        /* Log total runtime */
        log!("Total Run runtime: {:.1} sec", runtime);

        /* Log summary and final run result */
        let count_sum =
            total_test_passed_count + total_test_failed_count + total_test_skipped_count;
        let run_result = if total_test_failed_count == 0 {
            log_summary(
                true,
                "Run",
                count_sum,
                total_test_passed_count,
                total_test_failed_count,
                total_test_skipped_count,
            );
            log_final_result(
                true,
                "Run /w seed",
                &run_seed,
                Some(color_green()),
                "Passed",
            );
            0
        } else {
            log_summary(
                false,
                "Run",
                count_sum,
                total_test_passed_count,
                total_test_failed_count,
                total_test_skipped_count,
            );
            log_final_result(false, "Run /w seed", &run_seed, Some(color_red()), "Failed");
            1
        };

        /* Print repro steps for failed tests */
        if !failed_tests.is_empty() {
            log!("Harness input to repro failures:");
            for failed_test in &failed_tests {
                log!(
                    "{} --seed {} --filter {}{}",
                    color_red(),
                    run_seed,
                    failed_test.name,
                    color_end()
                );
            }
        }

        log!("Exit code: {}", run_result);
        run_result
    }
}

#[cfg(test)]
mod tests;
