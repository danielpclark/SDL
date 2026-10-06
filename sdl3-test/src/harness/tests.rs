// Tests of the harness: the results, logs and orders upstream's
// SDL_test_harness.c makes, with values from its C.

use std::sync::atomic::{AtomicU32, Ordering as AtomicOrdering};
use std::sync::Mutex;

use super::*;
use crate::test_support::{test_lock, Capture};

fn passes(_: &mut TestData) -> TestStatus {
    crate::assert_check!(true, "Always passes");
    TestStatus::Completed
}

fn fails(_: &mut TestData) -> TestStatus {
    crate::assert_check!(true, "Passes first");
    crate::assert_check!(false, "Then fails");
    TestStatus::Completed
}

fn skips(_: &mut TestData) -> TestStatus {
    TestStatus::Skipped
}

fn no_asserts(_: &mut TestData) -> TestStatus {
    TestStatus::Completed
}

fn aborts(_: &mut TestData) -> TestStatus {
    crate::assert_pass!("Before aborting");
    TestStatus::Aborted
}

fn only_starts(_: &mut TestData) -> TestStatus {
    crate::assert_pass!("Started");
    TestStatus::Started
}

static PASSES: TestCase = TestCase {
    test_case: passes,
    name: "passes",
    description: "A passing test",
    enabled: true,
};
static FAILS: TestCase = TestCase {
    test_case: fails,
    name: "fails",
    description: "A failing test",
    enabled: true,
};
static SKIPS: TestCase = TestCase {
    test_case: skips,
    name: "skips",
    description: "",
    enabled: true,
};
static DISABLED: TestCase = TestCase {
    test_case: passes,
    name: "disabled",
    description: "A disabled test",
    enabled: false,
};
static NO_ASSERTS: TestCase = TestCase {
    test_case: no_asserts,
    name: "no_asserts",
    description: "",
    enabled: true,
};
static ABORTS: TestCase = TestCase {
    test_case: aborts,
    name: "aborts",
    description: "",
    enabled: true,
};
static ONLY_STARTS: TestCase = TestCase {
    test_case: only_starts,
    name: "only_starts",
    description: "",
    enabled: true,
};

static MIXED: TestSuite = TestSuite {
    name: "Mixed",
    test_set_up: None,
    test_cases: &[&PASSES, &FAILS, &SKIPS, &DISABLED],
    test_tear_down: None,
};

static GOOD: TestSuite = TestSuite {
    name: "Good",
    test_set_up: None,
    test_cases: &[&PASSES, &SKIPS, &DISABLED],
    test_tear_down: None,
};

/// The log without the lines with run times (which vary).
fn transcript(capture: &Capture) -> Vec<String> {
    capture
        .messages()
        .into_iter()
        .filter(|m| !m.contains("untime"))
        .map(|m| {
            m.strip_prefix(" : ")
                .or_else(|| m.strip_prefix(": "))
                .unwrap_or(&m)
                .to_owned()
        })
        .collect()
}

#[test]
fn exec_keys_from_upstream() {
    let _l = test_lock();
    let _capture = Capture::new();
    // From upstream's SDLTest_GenerateExecKey() (a little-endian machine's).
    if cfg!(target_endian = "little") {
        assert_eq!(
            generate_exec_key("ABCDEFGHIJKLMNOP", "SDLtest", "sdltest_randomNumber", 1),
            6996997713402912175
        );
        assert_eq!(
            generate_exec_key("seed", "Suite", "test", 12),
            8105885556343298683
        );
        assert_eq!(
            generate_exec_key("ABCDEFGHIJKLMNOP", "random testSuites", "initialisation", 1),
            4130934721879088958
        );
    }
    assert_eq!(generate_exec_key("", "Suite", "test", 1), 0);
    assert_eq!(generate_exec_key("seed", "", "test", 1), 0);
    assert_eq!(generate_exec_key("seed", "Suite", "", 1), 0);
    assert_eq!(generate_exec_key("seed", "Suite", "test", 0), 0);
}

#[test]
fn run_seeds() {
    let _l = test_lock();
    let capture = Capture::new();
    // As testautomation_sdltest.c's sdltest_generateRunSeed.
    for i in (1..=10).step_by(3) {
        let seed = generate_run_seed(i).unwrap();
        assert_eq!(seed.len(), i as usize);
        assert!(seed
            .bytes()
            .all(|b| b.is_ascii_digit() || b.is_ascii_uppercase()));
    }
    for j in -2..=0 {
        assert_eq!(generate_run_seed(j), None);
    }
    assert_eq!(
        capture.lines().last().unwrap().2,
        ": The length of the harness seed must be >0."
    );
}

#[test]
fn arguments() {
    let mut runner = TestSuiteRunner::without_state(&[]);
    let argv = [
        "prog",
        "--iterations",
        "3",
        "--EXECKEY",
        "1234",
        "--seed",
        "S",
        "--filter",
        "f",
        "--random-order",
        "--other",
        "--seed",
    ];
    assert_eq!(runner.parse_argument(&argv, 0), 0);
    assert_eq!(runner.parse_argument(&argv, 1), 2);
    assert_eq!(runner.user().test_iterations, 3);
    assert_eq!(runner.parse_argument(&argv, 3), 2);
    assert_eq!(runner.user().exec_key, 1234);
    assert_eq!(runner.parse_argument(&argv, 5), 2);
    assert_eq!(runner.user().run_seed.as_deref(), Some("S"));
    assert_eq!(runner.parse_argument(&argv, 7), 2);
    assert_eq!(runner.user().filter.as_deref(), Some("f"));
    assert_eq!(runner.parse_argument(&argv, 9), 1);
    assert!(runner.user().random_order);
    assert_eq!(runner.parse_argument(&argv, 10), 0);
    // An option without its value isn't taken.
    assert_eq!(runner.parse_argument(&argv, 11), 0);
    assert_eq!(runner.parse_argument(&argv, 12), 0);
    // Fewer than one iteration is one; a key that isn't a number is ignored.
    assert_eq!(runner.parse_argument(&["--iterations", "-4"], 0), 2);
    assert_eq!(runner.user().test_iterations, 1);
    assert_eq!(runner.parse_argument(&["--execKey", "x"], 0), 2);
    assert_eq!(runner.user().exec_key, 1234);
    assert_eq!(TestSuiteRunner::usage().len(), 5);
}

#[test]
fn pass_fail_skip() {
    let _l = test_lock();
    let capture = Capture::new();
    let suites = [&MIXED, &GOOD];
    let mut runner = TestSuiteRunner::without_state(&suites);
    runner.set_seed(Some("ABCDEFGHIJKLMNOP"));
    assert_eq!(runner.execute(), 1);
    let key = |suite: &str, test: &str| generate_exec_key("ABCDEFGHIJKLMNOP", suite, test, 1);
    let expected = [
        "::::: Test Run /w seed 'ABCDEFGHIJKLMNOP' started".to_owned(),
        "===== Test Suite 1: 'Mixed' started".to_owned(),
        "----- Test Case 1.1: 'passes' started".to_owned(),
        "Test Description: 'A passing test'".to_owned(),
        format!("Test Iteration 1: execKey {}", key("Mixed", "passes")),
        "Assert 'Always passes': Passed".to_owned(),
        "Assert Summary: Total=1 Passed=1 Failed=0".to_owned(),
        ">>> Test 'passes': Passed".to_owned(),
        "----- Test Case 1.2: 'fails' started".to_owned(),
        "Test Description: 'A failing test'".to_owned(),
        format!("Test Iteration 1: execKey {}", key("Mixed", "fails")),
        "Assert 'Passes first': Passed".to_owned(),
        "Assert 'Then fails': Failed".to_owned(),
        "Assert Summary: Total=2 Passed=1 Failed=1".to_owned(),
        ">>> Test 'fails': Failed".to_owned(),
        "----- Test Case 1.3: 'skips' started".to_owned(),
        format!("Test Iteration 1: execKey {}", key("Mixed", "skips")),
        ">>> Test 'skips': Skipped (Programmatically)".to_owned(),
        "----- Test Case 1.4: 'disabled' started".to_owned(),
        "Test Description: 'A disabled test'".to_owned(),
        format!("Test Iteration 1: execKey {}", key("Mixed", "disabled")),
        ">>> Test 'disabled': Skipped (Disabled)".to_owned(),
        "Suite Summary: Total=4 Passed=1 Failed=1 Skipped=2".to_owned(),
        ">>> Suite 'Mixed': Failed".to_owned(),
        "===== Test Suite 2: 'Good' started".to_owned(),
        "----- Test Case 2.1: 'passes' started".to_owned(),
        "Test Description: 'A passing test'".to_owned(),
        format!("Test Iteration 1: execKey {}", key("Good", "passes")),
        "Assert 'Always passes': Passed".to_owned(),
        "Assert Summary: Total=1 Passed=1 Failed=0".to_owned(),
        ">>> Test 'passes': Passed".to_owned(),
        "----- Test Case 2.2: 'skips' started".to_owned(),
        format!("Test Iteration 1: execKey {}", key("Good", "skips")),
        ">>> Test 'skips': Skipped (Programmatically)".to_owned(),
        "----- Test Case 2.3: 'disabled' started".to_owned(),
        "Test Description: 'A disabled test'".to_owned(),
        format!("Test Iteration 1: execKey {}", key("Good", "disabled")),
        ">>> Test 'disabled': Skipped (Disabled)".to_owned(),
        "Suite Summary: Total=3 Passed=1 Failed=0 Skipped=2".to_owned(),
        ">>> Suite 'Good': Passed".to_owned(),
        "Run Summary: Total=7 Passed=2 Failed=1 Skipped=4".to_owned(),
        ">>> Run /w seed 'ABCDEFGHIJKLMNOP': Failed".to_owned(),
        "Harness input to repro failures:".to_owned(),
        " --seed ABCDEFGHIJKLMNOP --filter fails".to_owned(),
        "Exit code: 1".to_owned(),
    ];
    assert_eq!(transcript(&capture), expected);

    // The priorities: failures are errors.
    let lines = capture.lines();
    let priority_of = |text: &str| lines.iter().find(|l| l.2.contains(text)).unwrap().1;
    assert_eq!(priority_of(">>> Test 'fails'"), Priority::Error);
    assert_eq!(priority_of(">>> Suite 'Mixed'"), Priority::Error);
    assert_eq!(priority_of(">>> Suite 'Good'"), Priority::Info);
    assert_eq!(priority_of("Run Summary"), Priority::Error);

    // Without the failing test, the run passes.
    capture.clear();
    let suites = [&GOOD];
    let mut runner = TestSuiteRunner::without_state(&suites);
    assert_eq!(runner.execute(), 0);
    let messages = transcript(&capture);
    assert!(messages.contains(&"Run Summary: Total=3 Passed=1 Failed=0 Skipped=2".to_owned()));
    // (a random 16-character seed)
    assert_eq!(
        messages[0].len(),
        "::::: Test Run /w seed '' started".len() + 16
    );
    assert_eq!(messages.last().unwrap(), "Exit code: 0");
}

#[test]
fn other_results() {
    static ODD: TestSuite = TestSuite {
        name: "Odd",
        test_set_up: None,
        test_cases: &[&NO_ASSERTS, &ABORTS, &ONLY_STARTS],
        test_tear_down: None,
    };
    let _l = test_lock();
    let capture = Capture::new();
    let suites = [&ODD];
    let mut runner = TestSuiteRunner::without_state(&suites);
    runner.set_seed(Some("X"));
    assert_eq!(runner.execute(), 1);
    let messages = transcript(&capture);
    for line in [
        ">>> Test 'no_asserts': No Asserts",
        ">>> Test 'aborts': Failed (Aborted)",
        ">>> Test 'aborts': Failed",
        ">>> Test 'only_starts': Skipped (test started, but did not return TEST_COMPLETED)",
        ">>> Test 'only_starts': Failed",
        "Suite Summary: Total=3 Passed=0 Failed=3 Skipped=0",
        " --seed X --filter aborts",
        " --seed X --filter only_starts",
    ] {
        assert!(
            messages.contains(&line.to_owned()),
            "{line} in {messages:#?}"
        );
    }
    // A test without asserts counts as failed, but isn't listed as one to repro.
    assert_eq!(
        messages.iter().filter(|m| m.starts_with(" --seed")).count(),
        2,
        "{messages:#?}"
    );
}

static SET_UP_RUNS: AtomicU32 = AtomicU32::new(0);
static TEAR_DOWN_DATA: Mutex<Vec<u32>> = Mutex::new(Vec::new());

fn set_up(data: &mut TestData) {
    let n = SET_UP_RUNS.fetch_add(1, AtomicOrdering::SeqCst) + 1;
    *data = Some(Box::new(n));
}

fn uses_data(data: &mut TestData) -> TestStatus {
    let n = data.as_ref().and_then(|d| d.downcast_ref::<u32>()).copied();
    crate::assert_check!(n.is_some(), "Setup made data");
    *data = Some(Box::new(n.unwrap_or(0) * 10));
    TestStatus::Completed
}

fn tear_down(data: &mut TestData) {
    let n = data.as_ref().and_then(|d| d.downcast_ref::<u32>()).copied();
    TEAR_DOWN_DATA.lock().unwrap().push(n.unwrap_or(0));
    // (asserts failing here don't count)
    crate::assert_check!(false, "Ignored");
}

fn failing_set_up(_: &mut TestData) {
    crate::assert_check!(false, "Setup fails");
}

#[test]
fn set_up_and_tear_down() {
    static USES_DATA: TestCase = TestCase {
        test_case: uses_data,
        name: "uses_data",
        description: "",
        enabled: true,
    };
    static WITH_DATA: TestSuite = TestSuite {
        name: "WithData",
        test_set_up: Some(set_up),
        test_cases: &[&USES_DATA, &USES_DATA],
        test_tear_down: Some(tear_down),
    };
    static FAILING_SET_UP: TestSuite = TestSuite {
        name: "FailingSetUp",
        test_set_up: Some(failing_set_up),
        test_cases: &[&PASSES],
        test_tear_down: Some(tear_down),
    };
    let _l = test_lock();
    let capture = Capture::new();
    SET_UP_RUNS.store(0, AtomicOrdering::SeqCst);
    TEAR_DOWN_DATA.lock().unwrap().clear();
    let suites = [&WITH_DATA, &FAILING_SET_UP];
    let mut runner = TestSuiteRunner::without_state(&suites);
    runner.set_iterations(2);
    assert_eq!(runner.execute(), 1);
    assert_eq!(SET_UP_RUNS.load(AtomicOrdering::SeqCst), 4);
    assert_eq!(*TEAR_DOWN_DATA.lock().unwrap(), [10, 20, 30, 40]);
    let messages = transcript(&capture);
    for line in [
        "Suite Summary: Total=4 Passed=4 Failed=0 Skipped=0",
        ">>> Suite Setup 'FailingSetUp': Failed",
        "Suite Summary: Total=2 Passed=0 Failed=2 Skipped=0",
        "Run Summary: Total=6 Passed=4 Failed=2 Skipped=0",
    ] {
        assert!(
            messages.contains(&line.to_owned()),
            "{line} in {messages:#?}"
        );
    }
    // A setup failure isn't a test failure for the final result or repro list.
    assert!(!messages.iter().any(|m| m.starts_with(" --seed")));
}

static RECORDED: Mutex<Vec<u32>> = Mutex::new(Vec::new());

fn record(_: &mut TestData) -> TestStatus {
    RECORDED
        .lock()
        .unwrap()
        .push(crate::fuzzer::random_uint32());
    crate::assert_pass!("Recorded");
    TestStatus::Completed
}

#[test]
fn iterations_and_exec_keys() {
    static RECORD: TestCase = TestCase {
        test_case: record,
        name: "record",
        description: "",
        enabled: true,
    };
    static RECORDER: TestSuite = TestSuite {
        name: "Recorder",
        test_set_up: None,
        test_cases: &[&RECORD],
        test_tear_down: None,
    };
    let _l = test_lock();
    let capture = Capture::new();
    let suites = [&RECORDER];
    let mut runner = TestSuiteRunner::without_state(&suites);
    runner.set_seed(Some("ABCDEFGHIJKLMNOP"));
    runner.set_iterations(3);
    RECORDED.lock().unwrap().clear();
    assert_eq!(runner.execute(), 0);
    // Each iteration's fuzzer starts from its own key; values from upstream's C.
    if cfg!(target_endian = "little") {
        assert_eq!(
            *RECORDED.lock().unwrap(),
            [127747354, 3110857900, 1082613165]
        );
    }
    let messages = transcript(&capture);
    assert!(messages.contains(&"Fuzzer invocations: 1".to_owned()));
    assert!(messages.contains(&"Suite Summary: Total=3 Passed=3 Failed=0 Skipped=0".to_owned()));
    assert!(capture
        .messages()
        .iter()
        .any(|m| m.contains("Runtime of 3 iterations: ")));
    assert!(capture
        .messages()
        .iter()
        .any(|m| m.contains("Average Test runtime: ")));

    // A fixed execution key: every iteration draws the same.
    RECORDED.lock().unwrap().clear();
    runner.set_exec_key(0x0123456789ABCDEF);
    assert_eq!(runner.execute(), 0);
    let first = Rng::from_state(0x0123456789ABCDEF).next_u32();
    assert_eq!(*RECORDED.lock().unwrap(), [first, first, first]);
}

#[test]
fn filters() {
    let _l = test_lock();
    let capture = Capture::new();
    let suites = [&MIXED, &GOOD];

    // A suite, by name ignoring case.
    let mut runner = TestSuiteRunner::without_state(&suites);
    runner.set_filter(Some("good"));
    assert_eq!(runner.execute(), 0);
    let messages = transcript(&capture);
    assert!(messages.contains(&"Filtering: running only suite 'Good'".to_owned()));
    assert!(messages.contains(&"===== Test Suite 1: 'Mixed' skipped".to_owned()));
    assert!(messages.contains(&"Run Summary: Total=3 Passed=1 Failed=0 Skipped=2".to_owned()));

    // A test: the first suite that has it, and a disabled test is run.
    capture.clear();
    let mut runner = TestSuiteRunner::without_state(&suites);
    runner.set_filter(Some("DISABLED"));
    assert_eq!(runner.execute(), 0);
    let messages = transcript(&capture);
    for line in [
        "Filtering: running only test 'disabled' in suite 'Mixed'",
        "===== Test Case 1.1: 'passes' skipped",
        "===== Test Case 1.2: 'fails' skipped",
        "===== Test Case 1.3: 'skips' skipped",
        "Force run of disabled test since test filter was set",
        ">>> Test 'disabled': Passed",
        "===== Test Suite 2: 'Good' skipped",
        "Run Summary: Total=1 Passed=1 Failed=0 Skipped=0",
    ] {
        assert!(
            messages.contains(&line.to_owned()),
            "{line} in {messages:#?}"
        );
    }

    // Nothing: the tests are listed.
    capture.clear();
    let mut runner = TestSuiteRunner::without_state(&suites);
    runner.set_filter(Some("nothing"));
    assert_eq!(runner.execute(), 2);
    let messages = transcript(&capture);
    assert_eq!(
        messages[1..],
        [
            "Filter 'nothing' did not match any test suite/case.",
            "Test suite: Mixed",
            "      test: passes",
            "      test: fails",
            "      test: skips",
            "      test: disabled (disabled)",
            "Test suite: Good",
            "      test: passes",
            "      test: skips",
            "      test: disabled (disabled)",
            "Exit code: 2",
        ]
    );

    // No tests at all.
    capture.clear();
    static EMPTY: TestSuite = TestSuite {
        name: "Empty",
        test_set_up: None,
        test_cases: &[],
        test_tear_down: None,
    };
    let suites = [&EMPTY];
    assert_eq!(TestSuiteRunner::without_state(&suites).execute(), -1);
    assert_eq!(transcript(&capture).last().unwrap(), "No tests to run?");
}

#[test]
fn random_order() {
    static A: TestCase = TestCase {
        test_case: passes,
        name: "a",
        description: "",
        enabled: true,
    };
    static B: TestCase = TestCase {
        test_case: passes,
        name: "b",
        description: "",
        enabled: true,
    };
    static C: TestCase = TestCase {
        test_case: passes,
        name: "c",
        description: "",
        enabled: true,
    };
    static D: TestCase = TestCase {
        test_case: passes,
        name: "d",
        description: "",
        enabled: true,
    };
    static S0: TestSuite = TestSuite {
        name: "S0",
        test_set_up: None,
        test_cases: &[&A, &B, &C, &D],
        test_tear_down: None,
    };
    static S1: TestSuite = TestSuite {
        name: "S1",
        test_set_up: None,
        test_cases: &[&A, &B],
        test_tear_down: None,
    };
    static S2: TestSuite = TestSuite {
        name: "S2",
        test_set_up: None,
        test_cases: &[&A, &B, &C],
        test_tear_down: None,
    };
    let _l = test_lock();
    let capture = Capture::new();
    let suites = [&S0, &S1, &S2];
    let mut runner = TestSuiteRunner::without_state(&suites);
    runner.set_seed(Some("ORDERSEED"));
    runner.set_random_order(true);
    assert_eq!(runner.execute(), 0);
    let started: Vec<String> = transcript(&capture)
        .into_iter()
        .filter(|m| m.starts_with("----- Test Case"))
        .collect();
    // The order upstream's C shuffles to for this seed: suites 0, 1, 2 (the
    // last never moves), tests 1 3 0 2, 0 1 and 0 2 1 (each suite's
    // shuffle draws from the fuzzer its last test left).
    if cfg!(target_endian = "little") {
        assert_eq!(
            started,
            [
                "----- Test Case 1.2: 'b' started",
                "----- Test Case 1.4: 'd' started",
                "----- Test Case 1.1: 'a' started",
                "----- Test Case 1.3: 'c' started",
                "----- Test Case 2.1: 'a' started",
                "----- Test Case 2.2: 'b' started",
                "----- Test Case 3.1: 'a' started",
                "----- Test Case 3.3: 'c' started",
                "----- Test Case 3.2: 'b' started",
            ]
        );
    }

    // A single suite (where upstream's shuffle indexes before the list).
    capture.clear();
    let suites = [&S0];
    let mut runner = TestSuiteRunner::without_state(&suites);
    runner.set_random_order(true);
    assert_eq!(runner.execute(), 0);
    assert_eq!(
        transcript(&capture)
            .iter()
            .filter(|m| m.ends_with(": Passed") && m.starts_with(">>> Test"))
            .count(),
        4
    );
}

#[test]
fn colored_summary() {
    let _l = test_lock();
    let capture = Capture::new();
    crate::internal::set_color(true);
    log_summary(false, "Run", 3, 1, 1, 1);
    log_final_result(true, "Test", "t", None, "Skipped (Disabled)");
    assert_eq!(
        capture.messages(),
        [
            ": Run Summary: Total=3 \x1b[0;32mPassed=1\x1b[0m \x1b[0;31mFailed=1\x1b[0m \x1b[0;94mSkipped=1\x1b[0m",
            " : \x1b[0;93m>>> Test 't':\x1b[0m Skipped (Disabled)",
        ]
    );
}
