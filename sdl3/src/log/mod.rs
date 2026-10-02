// Rust translation of src/SDL_log.c from Simple DirectMedia Layer.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! Simple log messages with priorities and categories.
//!
//! A message's [`Priority`] signifies how important it is; its [`Category`]
//! says what domain it belongs to. Every category has a minimum priority:
//! a message is only output if its priority is at least that of its category.
//! SDL's own logs are below the default thresholds, so they are quiet unless
//! the `SDL_LOGGING` hint ([`hints::LOGGING`](crate::hints::LOGGING)) turns
//! them on, e.g. `SDL_LOGGING=video=debug,*=warn`.
//!
//! Direct translation of `SDL_log.c`. Formatting uses the `log!`,
//! `info!`, `warn!`, ... macros in this module.

use std::fmt;
use std::sync::{Arc, Mutex, MutexGuard};

use crate::error::{Error, Result};
use crate::thread::{InitState, RawMutex};

/// Log categories. Translation of `SDL_LogCategory`.
///
/// Values ≥ [`Category::CUSTOM_BASE`] are for application use.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Category {
    Application,
    Error,
    Assert,
    System,
    Audio,
    Video,
    Render,
    Input,
    Test,
    Gpu,
    /// Reserved for future SDL library use (`SDL_LOG_CATEGORY_RESERVED2..10`, raw 10–18).
    Reserved(u8),
    /// Application-defined category; holds the raw value (≥ 19).
    Custom(u32),
}

impl Category {
    /// Translation of `SDL_LOG_CATEGORY_CUSTOM`.
    pub const CUSTOM_BASE: u32 = 19;

    /// The C integer value.
    pub const fn to_raw(self) -> i32 {
        match self {
            Category::Application => 0,
            Category::Error => 1,
            Category::Assert => 2,
            Category::System => 3,
            Category::Audio => 4,
            Category::Video => 5,
            Category::Render => 6,
            Category::Input => 7,
            Category::Test => 8,
            Category::Gpu => 9,
            Category::Reserved(n) => n as i32,
            Category::Custom(n) => n as i32,
        }
    }

    /// From the C integer value.
    pub const fn from_raw(raw: i32) -> Category {
        match raw {
            0 => Category::Application,
            1 => Category::Error,
            2 => Category::Assert,
            3 => Category::System,
            4 => Category::Audio,
            5 => Category::Video,
            6 => Category::Render,
            7 => Category::Input,
            8 => Category::Test,
            9 => Category::Gpu,
            10..=18 => Category::Reserved(raw as u8),
            _ => Category::Custom(raw as u32),
        }
    }
}

/// Log priorities, least to most important. Translation of `SDL_LogPriority`.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub enum Priority {
    Trace = 1,
    Verbose,
    Debug,
    Info,
    Warn,
    Error,
    Critical,
}

impl Priority {
    const COUNT: usize = 8; // SDL_LOG_PRIORITY_COUNT

    const fn from_raw(raw: i32) -> Option<Priority> {
        Some(match raw {
            1 => Priority::Trace,
            2 => Priority::Verbose,
            3 => Priority::Debug,
            4 => Priority::Info,
            5 => Priority::Warn,
            6 => Priority::Error,
            7 => Priority::Critical,
            _ => return None,
        })
    }
}

/// A minimum-priority threshold for a category: `Quiet` disables it.
/// (Translation of the `SDL_LogPriority` filter values, where
/// `SDL_LOG_PRIORITY_COUNT` means "quiet".)
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Threshold {
    /// Output messages of this priority and above.
    AtLeast(Priority),
    /// Output nothing.
    Quiet,
}

impl Threshold {
    fn passes(self, p: Priority) -> bool {
        match self {
            Threshold::AtLeast(min) => p >= min,
            Threshold::Quiet => false,
        }
    }
}

impl From<Priority> for Threshold {
    fn from(p: Priority) -> Threshold {
        Threshold::AtLeast(p)
    }
}

/// A log message as delivered to the output function. Translation of the
/// `SDL_LogOutputFunction` arguments.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Record<'a> {
    pub category: Category,
    pub priority: Priority,
    pub message: &'a str,
}

type OutputFn = dyn Fn(&Record<'_>) + Send + Sync + 'static;

const DEFAULT_CATEGORY: i32 = -1;
const NUM_BUILTIN: usize = Category::CUSTOM_BASE as usize;

struct LogLevel {
    category: i32,
    threshold: Threshold,
}

struct LogState {
    loglevels: Vec<LogLevel>, // categories >= CUSTOM_BASE (a linked list upstream)
    thresholds: [Option<Threshold>; NUM_BUILTIN],
    default_threshold: Option<Threshold>,
}

struct FunctionState {
    output: Option<Arc<OutputFn>>,
    prefixes: [Option<String>; Priority::COUNT],
}

static LOG_INIT: InitState = InitState::new();
/// Translation of `SDL_log_lock` (recursive; held around priority updates).
static LOG_LOCK: RawMutex = RawMutex::new();
/// Translation of `SDL_log_function_lock` (recursive; held while calling the output function).
static LOG_FUNCTION_LOCK: RawMutex = RawMutex::new();
static LOG_STATE: Mutex<LogState> = Mutex::new(LogState {
    loglevels: Vec::new(),
    thresholds: [None; NUM_BUILTIN],
    default_threshold: None,
});
static FUNCTION_STATE: Mutex<FunctionState> = Mutex::new(FunctionState {
    output: None,
    prefixes: [None, None, None, None, None, None, None, None],
});
static HINT_WATCH: Mutex<Option<crate::hints::Callback>> = Mutex::new(None);

// If this list changes, update the documentation for SDL_HINT_LOGGING
static PRIORITY_NAMES: [&str; 7] = [
    "TRACE", "VERBOSE", "DEBUG", "INFO", "WARN", "ERROR", "CRITICAL",
];
// If this list changes, update the documentation for SDL_HINT_LOGGING
static CATEGORY_NAMES: [&str; 10] = [
    "APP", "ERROR", "ASSERT", "SYSTEM", "AUDIO", "VIDEO", "RENDER", "INPUT", "TEST", "GPU",
];

fn state() -> MutexGuard<'static, LogState> {
    LOG_STATE.lock().unwrap_or_else(|e| e.into_inner())
}

fn fstate() -> MutexGuard<'static, FunctionState> {
    FUNCTION_STATE.lock().unwrap_or_else(|e| e.into_inner())
}

/// Translation of `SDL_InitLog()`.
fn init_log() {
    if !LOG_INIT.should_init() {
        return;
    }
    *HINT_WATCH.lock().unwrap_or_else(|e| e.into_inner()) =
        crate::hints::watch(crate::hints::LOGGING, |_| reset_priorities()).ok();
    LOG_INIT.set_initialized(true);
}

/// Translation of `SDL_QuitLog()`.
pub(crate) fn quit_log() {
    if !LOG_INIT.should_quit() {
        return;
    }
    *HINT_WATCH.lock().unwrap_or_else(|e| e.into_inner()) = None;
    cleanup_priorities();
    cleanup_prefixes();
    LOG_INIT.set_initialized(false);
}

/// Translation of `SDL_CheckInitLog()`.
fn check_init() {
    if !LOG_INIT.is_initialized_or_initializing_here() {
        init_log();
    }
}

fn cleanup_priorities() {
    LOG_LOCK.lock();
    state().loglevels.clear();
    LOG_LOCK.unlock();
}

/// Set the threshold of every category. Translation of `SDL_SetLogPriorities()`.
pub fn set_all_priorities(threshold: impl Into<Threshold>) {
    let threshold = threshold.into();
    check_init();
    LOG_LOCK.lock();
    {
        cleanup_priorities();
        let mut st = state();
        st.default_threshold = Some(threshold);
        st.thresholds = [Some(threshold); NUM_BUILTIN];
    }
    LOG_LOCK.unlock();
}

/// Set the threshold of one category. Translation of `SDL_SetLogPriority()`.
pub fn set_priority(category: Category, threshold: impl Into<Threshold>) {
    let threshold = threshold.into();
    check_init();
    LOG_LOCK.lock();
    set_priority_locked(category.to_raw(), threshold);
    LOG_LOCK.unlock();
}

fn set_priority_locked(category: i32, threshold: Threshold) {
    let mut st = state();
    if (0..NUM_BUILTIN as i32).contains(&category) {
        st.thresholds[category as usize] = Some(threshold);
    } else if let Some(entry) = st.loglevels.iter_mut().find(|e| e.category == category) {
        entry.threshold = threshold;
    } else {
        st.loglevels.insert(
            0,
            LogLevel {
                category,
                threshold,
            },
        );
    }
}

/// The threshold of a category. Translation of `SDL_GetLogPriority()`.
pub fn priority(category: Category) -> Threshold {
    check_init();
    let raw = category.to_raw();

    // Bypass the lock for known categories
    // Technically if the priority was set on a different CPU the value might not
    // be visible on this CPU for a while, but in practice it's fast enough that
    // this performance improvement is worthwhile.
    if (0..NUM_BUILTIN as i32).contains(&raw) {
        return state().thresholds[raw as usize].unwrap_or(Threshold::Quiet);
    }

    LOG_LOCK.lock();
    let threshold = {
        let st = state();
        st.loglevels
            .iter()
            .find(|e| e.category == raw)
            .map(|e| e.threshold)
            .or(st.default_threshold)
            .unwrap_or(Threshold::Quiet)
    };
    LOG_LOCK.unlock();
    threshold
}

/// `SDL_strncasecmp(a, b, n) == 0`: compare at most `n` bytes case-insensitively,
/// stopping at the end of either string (C NUL semantics).
fn strncasecmp_eq(a: &str, b: &str, n: usize) -> bool {
    let (a, b) = (a.as_bytes(), b.as_bytes());
    for i in 0..n {
        let ca = a.get(i).copied().unwrap_or(0).to_ascii_lowercase();
        let cb = b.get(i).copied().unwrap_or(0).to_ascii_lowercase();
        if ca != cb {
            return false;
        }
        if ca == 0 {
            return true;
        }
    }
    true
}

fn parse_category(string: &str, length: usize) -> Option<i32> {
    let first = string.as_bytes().first().copied().unwrap_or(0);
    if first.is_ascii_digit() {
        return Some(crate::stdlib::atoi(string));
    }
    if first == b'*' {
        return Some(DEFAULT_CATEGORY);
    }
    CATEGORY_NAMES
        .iter()
        .position(|name| strncasecmp_eq(string, name, length))
        .map(|i| i as i32)
}

fn parse_threshold(string: &str, length: usize) -> Option<Threshold> {
    let first = string.as_bytes().first().copied().unwrap_or(0);
    if first.is_ascii_digit() {
        let i = crate::stdlib::atoi(string);
        if i == 0 {
            // 0 has a special meaning of "disable this category"
            return Some(Threshold::Quiet);
        }
        return Priority::from_raw(i).map(Threshold::AtLeast);
    }
    if strncasecmp_eq(string, "quiet", length) {
        return Some(Threshold::Quiet);
    }
    PRIORITY_NAMES
        .iter()
        .position(|name| strncasecmp_eq(string, name, length))
        .and_then(|i| Priority::from_raw(i as i32 + 1))
        .map(Threshold::AtLeast)
}

/// Translation of `ParseLogPriorities()`.
fn parse_priorities(hint: &str) {
    if !hint.contains('=') {
        if let Some(threshold) = parse_threshold(hint, hint.len()) {
            set_all_priorities(threshold);
        }
        return;
    }

    LOG_LOCK.lock();
    let mut rest: Option<&str> = Some(hint);
    while let Some(cur) = rest {
        let Some(sep) = cur.find('=') else { break };
        let after_sep = &cur[sep..];
        let next = after_sep.find(',').map(|i| &after_sep[i + 1..]);

        if let Some(category) = parse_category(cur, sep) {
            let value = &cur[sep + 1..];
            let len = match next {
                Some(n) => value.len() - n.len() - 1,
                None => value.len(),
            };
            if let Some(threshold) = parse_threshold(value, len) {
                if category == DEFAULT_CATEGORY {
                    let mut st = state();
                    for t in st.thresholds.iter_mut() {
                        if t.is_none() {
                            *t = Some(threshold);
                        }
                    }
                    st.default_threshold = Some(threshold);
                } else {
                    set_priority_locked(category, threshold);
                }
            }
        }
        rest = next;
    }
    LOG_LOCK.unlock();
}

/// Reset all thresholds to their defaults, honouring the `SDL_LOGGING` hint
/// and the `DEBUG_INVOCATION` environment variable. Translation of `SDL_ResetLogPriorities()`.
pub fn reset_priorities() {
    check_init();
    LOG_LOCK.lock();
    {
        let env = std::env::var("DEBUG_INVOCATION").ok();
        let debug = matches!(env.as_deref(), Some(e) if !e.is_empty() && !e.starts_with('0'));

        cleanup_priorities();
        {
            let mut st = state();
            st.default_threshold = None;
            st.thresholds = [None; NUM_BUILTIN];
        }

        if let Some(hint) = crate::hints::get(crate::hints::LOGGING) {
            parse_priorities(&hint);
        }

        let mut st = state();
        if st.default_threshold.is_none() {
            st.default_threshold = Some(Threshold::AtLeast(Priority::Error));
        }
        for (i, slot) in st.thresholds.iter_mut().enumerate() {
            if slot.is_some() {
                continue;
            }
            *slot = Some(Threshold::AtLeast(match Category::from_raw(i as i32) {
                Category::Application => {
                    if debug {
                        Priority::Debug
                    } else {
                        Priority::Info
                    }
                }
                Category::Assert => Priority::Warn,
                Category::Test => Priority::Verbose,
                _ => {
                    if debug {
                        Priority::Debug
                    } else {
                        Priority::Error
                    }
                }
            }));
        }
    }
    LOG_LOCK.unlock();
}

fn cleanup_prefixes() {
    LOG_FUNCTION_LOCK.lock();
    fstate().prefixes = Default::default();
    LOG_FUNCTION_LOCK.unlock();
}

/// Translation of `GetLogPriorityPrefix()`.
fn prefix_for(fs: &FunctionState, priority: Priority) -> String {
    if let Some(prefix) = &fs.prefixes[priority as usize] {
        return prefix.clone();
    }
    match priority {
        Priority::Warn => "WARNING: ".to_owned(),
        Priority::Error | Priority::Critical => "ERROR: ".to_owned(),
        _ => String::new(),
    }
}

/// Set the text prepended to messages of a priority by the default output.
///
/// By default `Info` and below have no prefix, and `Warn` and higher have a
/// prefix showing their priority, e.g. `"WARNING: "`. `None` restores the default.
/// Translation of `SDL_SetLogPriorityPrefix()`.
pub fn set_prefix(priority: Priority, prefix: Option<&str>) -> Result<()> {
    let _ = Error::invalid_param; // (the C range check cannot fail with an enum)
    LOG_FUNCTION_LOCK.lock();
    fstate().prefixes[priority as usize] = prefix.map(str::to_owned);
    LOG_FUNCTION_LOCK.unlock();
    Ok(())
}

/// Log a message. Translation of `SDL_LogMessage()`.
///
/// A trailing newline (or `\r\n`) is removed, as upstream does.
pub fn log(category: Category, priority: Priority, message: &str) {
    // Nothing to do if we don't have an output function
    let output = fstate().output.clone();
    let Some(output) = output else { return };

    // See if we want to do anything with this message
    if !self::priority(category).passes(priority) {
        return;
    }

    // Chop off final endline.
    let mut message = message;
    if let Some(stripped) = message.strip_suffix('\n') {
        message = stripped;
        if let Some(stripped) = message.strip_suffix('\r') {
            // catch "\r\n", too.
            message = stripped;
        }
    }

    LOG_FUNCTION_LOCK.lock();
    output(&Record {
        category,
        priority,
        message,
    });
    LOG_FUNCTION_LOCK.unlock();
}

/// The default output: prefix + message to stderr. Translation of the stdio
/// path of `SDL_LogOutput()` (the Windows debugger / Android logcat / Apple
/// NSLog paths are platform backends).
pub fn default_output(record: &Record<'_>) {
    let prefix = prefix_for(&fstate(), record.priority);
    eprintln!("{prefix}{}", record.message);
}

/// Replace the output function. It is called with a lock held, so it is
/// never run by more than one thread at once; it may log itself.
/// Translation of `SDL_SetLogOutputFunction()`.
pub fn set_output(output: impl Fn(&Record<'_>) + Send + Sync + 'static) {
    LOG_FUNCTION_LOCK.lock();
    fstate().output = Some(Arc::new(output));
    LOG_FUNCTION_LOCK.unlock();
}

/// Restore the default output function (`SDL_GetDefaultLogOutputFunction()`).
pub fn reset_output() {
    set_output(default_output);
}

/// Discard all log output (translation of setting a `NULL` output function).
pub fn disable_output() {
    LOG_FUNCTION_LOCK.lock();
    fstate().output = None;
    LOG_FUNCTION_LOCK.unlock();
}

/// Install the default output on first use.
fn ensure_default_output() {
    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once(|| {
        let mut fs = fstate();
        if fs.output.is_none() {
            fs.output = Some(Arc::new(default_output));
        }
    });
}

impl fmt::Display for Category {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Category::Reserved(_) => f.write_str("RESERVED"),
            Category::Custom(_) => f.write_str("CUSTOM"),
            other => f.write_str(CATEGORY_NAMES[other.to_raw() as usize]),
        }
    }
}

#[doc(hidden)]
pub fn __log(category: Category, priority: Priority, args: fmt::Arguments<'_>) {
    ensure_default_output();
    match args.as_str() {
        Some(s) => log(category, priority, s),
        None => log(category, priority, &args.to_string()),
    }
}

/// Log at `Info` priority in the `Application` category. Translation of `SDL_Log()`.
#[macro_export]
macro_rules! log {
    ($($arg:tt)*) => {
        $crate::log::__log($crate::log::Category::Application, $crate::log::Priority::Info, ::std::format_args!($($arg)*))
    };
}

/// Log at `Trace` priority: `trace!(category, fmt, ...)`. Translation of `SDL_LogTrace()`.
#[macro_export]
macro_rules! trace {
    ($cat:expr, $($arg:tt)*) => { $crate::log::__log($cat, $crate::log::Priority::Trace, ::std::format_args!($($arg)*)) };
}
/// Log at `Verbose` priority: `verbose!(category, fmt, ...)`. Translation of `SDL_LogVerbose()`.
#[macro_export]
macro_rules! verbose {
    ($cat:expr, $($arg:tt)*) => { $crate::log::__log($cat, $crate::log::Priority::Verbose, ::std::format_args!($($arg)*)) };
}
/// Log at `Debug` priority: `debug!(category, fmt, ...)`. Translation of `SDL_LogDebug()`.
#[macro_export]
macro_rules! debug {
    ($cat:expr, $($arg:tt)*) => { $crate::log::__log($cat, $crate::log::Priority::Debug, ::std::format_args!($($arg)*)) };
}
/// Log at `Info` priority: `info!(category, fmt, ...)`, or `info!(fmt, ...)` for
/// the `Application` category. Translation of `SDL_LogInfo()`.
#[macro_export]
macro_rules! info {
    ($fmt:literal $(, $arg:expr)* $(,)?) => { $crate::log::__log($crate::log::Category::Application, $crate::log::Priority::Info, ::std::format_args!($fmt $(, $arg)*)) };
    ($cat:expr, $($arg:tt)*) => { $crate::log::__log($cat, $crate::log::Priority::Info, ::std::format_args!($($arg)*)) };
}
/// Log at `Warn` priority: `warn!(category, fmt, ...)`. Translation of `SDL_LogWarn()`.
#[macro_export]
macro_rules! warn {
    ($cat:expr, $($arg:tt)*) => { $crate::log::__log($cat, $crate::log::Priority::Warn, ::std::format_args!($($arg)*)) };
}
/// Log at `Error` priority: `error!(category, fmt, ...)`. Translation of `SDL_LogError()`.
#[macro_export]
macro_rules! error {
    ($cat:expr, $($arg:tt)*) => { $crate::log::__log($cat, $crate::log::Priority::Error, ::std::format_args!($($arg)*)) };
}
/// Log at `Critical` priority: `critical!(category, fmt, ...)`. Translation of `SDL_LogCritical()`.
#[macro_export]
macro_rules! critical {
    ($cat:expr, $($arg:tt)*) => { $crate::log::__log($cat, $crate::log::Priority::Critical, ::std::format_args!($($arg)*)) };
}

pub use crate::{critical, debug, error, info, log as log_app, trace, verbose, warn};

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    pub(crate) use crate::test_support::TEST_LOCK;
    type Captured = Arc<Mutex<Vec<(Category, Priority, String)>>>;

    fn install() -> Captured {
        let captured: Captured = Arc::new(Mutex::new(Vec::new()));
        let c = captured.clone();
        set_output(move |r| {
            // Re-entrancy: the output function may itself query the log system.
            let _ = priority(r.category);
            c.lock()
                .unwrap()
                .push((r.category, r.priority, r.message.to_owned()));
        });
        reset_priorities();
        captured
    }

    #[test]
    fn defaults_and_filtering() {
        let _l = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let captured = install();
        assert_eq!(
            priority(Category::Application),
            Threshold::AtLeast(Priority::Info)
        );
        assert_eq!(
            priority(Category::Assert),
            Threshold::AtLeast(Priority::Warn)
        );
        assert_eq!(
            priority(Category::Test),
            Threshold::AtLeast(Priority::Verbose)
        );
        assert_eq!(
            priority(Category::Video),
            Threshold::AtLeast(Priority::Error)
        );
        assert_eq!(
            priority(Category::Custom(22)),
            Threshold::AtLeast(Priority::Error)
        );

        crate::log!("hello {}", 1);
        debug!(Category::Application, "filtered");
        error!(Category::Video, "video {}\r\n", "bad");
        info!("plain {}", "app");
        let got = captured.lock().unwrap().clone();
        assert_eq!(got.len(), 3);
        assert_eq!(
            got[0],
            (Category::Application, Priority::Info, "hello 1".to_owned())
        );
        assert_eq!(
            got[1],
            (Category::Video, Priority::Error, "video bad".to_owned())
        );
        assert_eq!(
            got[2],
            (
                Category::Application,
                Priority::Info,
                "plain app".to_owned()
            )
        );

        set_priority(Category::Custom(22), Priority::Trace);
        assert_eq!(
            priority(Category::Custom(22)),
            Threshold::AtLeast(Priority::Trace)
        );
        set_all_priorities(Priority::Critical);
        assert_eq!(
            priority(Category::Application),
            Threshold::AtLeast(Priority::Critical)
        );
        assert_eq!(
            priority(Category::Custom(22)),
            Threshold::AtLeast(Priority::Critical)
        );
        set_priority(Category::Video, Threshold::Quiet);
        critical!(Category::Video, "dropped");
        assert_eq!(captured.lock().unwrap().len(), 3);
        reset_output();
        reset_priorities();
    }

    #[test]
    fn hint_parsing() {
        let _l = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let _captured = install();
        crate::hints::set(
            crate::hints::LOGGING,
            "video=debug,app=quiet,*=warn,42=trace",
        )
        .unwrap();
        // The hint watcher resets priorities for us.
        assert_eq!(
            priority(Category::Video),
            Threshold::AtLeast(Priority::Debug)
        );
        assert_eq!(priority(Category::Application), Threshold::Quiet);
        assert_eq!(
            priority(Category::Audio),
            Threshold::AtLeast(Priority::Warn)
        );
        assert_eq!(
            priority(Category::Custom(42)),
            Threshold::AtLeast(Priority::Trace)
        );
        assert_eq!(
            priority(Category::Custom(20)),
            Threshold::AtLeast(Priority::Warn)
        );
        crate::hints::set(crate::hints::LOGGING, "verbose").unwrap();
        assert_eq!(
            priority(Category::Video),
            Threshold::AtLeast(Priority::Verbose)
        );
        assert_eq!(
            priority(Category::Custom(1234)),
            Threshold::AtLeast(Priority::Verbose)
        );
        crate::hints::set(crate::hints::LOGGING, "0").unwrap();
        assert_eq!(priority(Category::Video), Threshold::Quiet);
        crate::hints::reset(crate::hints::LOGGING);
        assert_eq!(
            priority(Category::Video),
            Threshold::AtLeast(Priority::Error)
        );
        reset_output();
    }

    #[test]
    fn prefixes_and_output() {
        let _l = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        set_prefix(Priority::Warn, Some("W> ")).unwrap();
        assert_eq!(prefix_for(&fstate(), Priority::Warn), "W> ");
        set_prefix(Priority::Warn, None).unwrap();
        assert_eq!(prefix_for(&fstate(), Priority::Warn), "WARNING: ");
        assert_eq!(prefix_for(&fstate(), Priority::Critical), "ERROR: ");
        assert_eq!(prefix_for(&fstate(), Priority::Info), "");
        disable_output();
        crate::log!("dropped silently");
        reset_output();
        assert_eq!(
            Category::from_raw(Category::Video.to_raw()),
            Category::Video
        );
        assert_eq!(Category::from_raw(12), Category::Reserved(12));
        assert_eq!(Category::from_raw(99), Category::Custom(99));
        assert_eq!(Category::Gpu.to_string(), "GPU");
        assert_eq!(Category::Custom(99).to_string(), "CUSTOM");
    }

    #[test]
    fn strncasecmp_semantics() {
        assert!(strncasecmp_eq("app", "APP", 3));
        assert!(strncasecmp_eq("ap", "APP", 2)); // prefix match, like C
        assert!(!strncasecmp_eq("apx", "APP", 3));
        assert!(!strncasecmp_eq("application", "APP", 11));
        assert!(strncasecmp_eq("app=debug", "APP", 3));
    }
}
