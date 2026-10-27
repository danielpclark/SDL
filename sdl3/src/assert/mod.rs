// Rust translation of src/SDL_assert.c and include/SDL3/SDL_assert.h from Simple DirectMedia Layer.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! SDL assertions: a configurable, reportable alternative to `assert!`.
//!
//! [`sdl_assert!`](crate::sdl_assert), [`sdl_assert_release!`](crate::sdl_assert_release),
//! [`sdl_assert_paranoid!`](crate::sdl_assert_paranoid) and
//! [`sdl_assert_always!`](crate::sdl_assert_always) translate the `SDL_assert*`
//! macros. A failed assertion calls the assertion handler, which decides
//! whether to retry, break into the debugger, abort, or ignore it (once or
//! forever). Every triggered assertion is recorded for
//! [`assertion_report`].
//!
//! Which macros are active follows `SDL_ASSERT_LEVEL`:
//!
//! | level | `sdl_assert!` | `sdl_assert_release!` | `sdl_assert_paranoid!` |
//! |---|---|---|---|
//! | 0 | off | off | off |
//! | 1 (release builds) | off | on | off |
//! | 2 (debug builds) | on | on | off |
//! | 3 | on | on | on |
//!
//! The default level is 2 when the *calling* crate is built with
//! `debug_assertions` and 1 otherwise, like the C header's `_DEBUG` check.
//! The `assert-level-0` … `assert-level-3` crate features override it.
//! Disabled assertions still type-check their condition but never evaluate it.

use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};

use crate::atomic::SpinLock;
use crate::hints;
use crate::log::{self, Category, Priority};
use crate::thread::RawMutex;

/// Possible outcomes from a triggered assertion. Translation of `SDL_AssertState`.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum AssertState {
    /// Retry the assert immediately.
    Retry,
    /// Make the debugger trigger a breakpoint.
    Break,
    /// Terminate the program.
    Abort,
    /// Ignore the assert.
    Ignore,
    /// Ignore the assert from now on.
    AlwaysIgnore,
}

/// Information about an assertion failure. Translation of `SDL_AssertData`.
///
/// Each assertion macro call site owns one `static` instance, exactly as in
/// C. The fields that change at run time are atomics.
#[derive(Debug)]
pub struct AssertData {
    always_ignore: AtomicBool,
    trigger_count: AtomicU32,
    condition: &'static str,
    filename: &'static str,
    linenum: u32,
    function: &'static str,
    in_report: AtomicBool,
}

impl AssertData {
    /// The per-call-site record (used by the assertion macros).
    pub const fn new(
        condition: &'static str,
        function: &'static str,
        filename: &'static str,
        linenum: u32,
    ) -> Self {
        AssertData {
            always_ignore: AtomicBool::new(false),
            trigger_count: AtomicU32::new(0),
            condition,
            filename,
            linenum,
            function,
            in_report: AtomicBool::new(false),
        }
    }

    /// true if app should always continue when assertion is triggered.
    pub fn always_ignore(&self) -> bool {
        self.always_ignore.load(Ordering::Relaxed)
    }
    /// Number of times this assertion has been triggered.
    pub fn trigger_count(&self) -> u32 {
        self.trigger_count.load(Ordering::Relaxed)
    }
    /// A string of this assert's test code.
    pub fn condition(&self) -> &'static str {
        self.condition
    }
    /// The source file where this assert lives.
    pub fn filename(&self) -> &'static str {
        self.filename
    }
    /// The line in `filename` where this assert lives.
    pub fn linenum(&self) -> u32 {
        self.linenum
    }
    /// The name of the function (module path, in Rust) where this assert lives.
    pub fn function(&self) -> &'static str {
        self.function
    }
}

/// A callback that fires when an SDL assertion fails.
/// Translation of `SDL_AssertionHandler` (the `void *userdata` is captured).
pub type AssertionHandler = Arc<dyn Fn(&AssertData) -> AssertState + Send + Sync>;

struct AssertGlobals {
    /// We keep all triggered assertions in a list so we can generate a report later.
    /// Translation of `triggered_assertions` (newest first, like the C linked list).
    triggered_assertions: Vec<&'static AssertData>,
    /// `None` means the default handler ([`default_assertion_handler`]).
    assertion_handler: Option<AssertionHandler>,
}

static GLOBALS: Mutex<AssertGlobals> = Mutex::new(AssertGlobals {
    triggered_assertions: Vec::new(),
    assertion_handler: None,
});

/// Translation of `assertion_mutex` (recursive, held while the handler runs).
static ASSERTION_MUTEX: RawMutex = RawMutex::new();

/// Translation of the `static int assertion_running` in `SDL_ReportAssertion()`.
static ASSERTION_RUNNING: AtomicU32 = AtomicU32::new(0);

/// Translation of the `static SDL_SpinLock spinlock` in `SDL_ReportAssertion()`.
/// Upstream uses it to create the mutex lazily; the Rust mutex is a static, so
/// the spinlock only keeps the structure of the original.
static REPORT_SPINLOCK: SpinLock<()> = SpinLock::new(());

fn globals() -> MutexGuard<'static, AssertGlobals> {
    GLOBALS.lock().unwrap_or_else(|e| e.into_inner())
}

/// The active assertion level (0–3). See the module docs.
///
/// `debug` is the *caller's* `cfg!(debug_assertions)`; the macros pass it.
#[doc(hidden)]
pub const fn assert_level(debug: bool) -> u8 {
    if cfg!(feature = "assert-level-0") {
        0
    } else if cfg!(feature = "assert-level-3") {
        3
    } else if cfg!(feature = "assert-level-2") {
        2
    } else if cfg!(feature = "assert-level-1") {
        1
    } else if debug {
        2
    } else {
        1
    }
}

/// Translation of `debug_print()`. (Through the logging macros' entry, which
/// installs the default output first, as upstream's `SDL_LogMessageV()` has
/// it: an assertion that fails before anything else logs is shown too.)
fn debug_print(message: &str) {
    log::__log(Category::Assert, Priority::Warn, format_args!("{message}"));
}

/// Translation of `SDL_AddAssertionToReport()`.
fn add_assertion_to_report(data: &'static AssertData) {
    /* (data) is always a static struct defined with the assert macros, so
    we don't have to worry about copying or allocating them. */
    let count = data.trigger_count.fetch_add(1, Ordering::Relaxed) + 1;
    if count == 1 && !data.in_report.swap(true, Ordering::Relaxed) {
        // not yet added?
        globals().triggered_assertions.insert(0, data);
    }
}

/// Translation of `ENDLINE`.
const ENDLINE: &str = if cfg!(windows) { "\r\n" } else { "\n" };

/// The message shown for a failed assertion. Translation of `SDL_RenderAssertMessage()`.
pub fn render_assert_message(data: &AssertData) -> String {
    let count = data.trigger_count();
    format!(
        "Assertion failure at {} ({}:{}), triggered {} {}:{ENDLINE}  '{}'",
        data.function,
        data.filename,
        data.linenum,
        count,
        if count == 1 { "time" } else { "times" },
        data.condition
    )
}

/// Translation of `SDL_GenerateAssertionReport()`.
fn generate_assertion_report() {
    let (items, custom_handler) = {
        let g = globals();
        (
            g.triggered_assertions.clone(),
            g.assertion_handler.is_some(),
        )
    };

    // only do this if the app hasn't assigned an assertion handler.
    if !items.is_empty() && custom_handler {
        debug_print("\n\nSDL assertion report.\n");
        debug_print("All SDL assertions between last init/quit:\n\n");

        for item in &items {
            let count = item.trigger_count();
            debug_print(&format!(
                "'{}'\n    * {} ({}:{})\n    * triggered {} time{}.\n    * always ignore: {}.\n",
                item.condition,
                item.function,
                item.filename,
                item.linenum,
                count,
                if count == 1 { "" } else { "s" },
                if item.always_ignore() { "yes" } else { "no" }
            ));
        }
        debug_print("\n");

        reset_assertion_report();
    }
}

/// Translation of `SDL_AbortAssertion()`.
fn abort_assertion() -> ! {
    crate::init::quit();
    std::process::abort();
}

/// The default assertion handler: logs the failure, then honours
/// `SDL_HINT_ASSERT` or asks on the terminal. Translation of `SDL_PromptAssertion()`.
///
/// Upstream shows a message box first; message boxes arrive with the video
/// subsystem, so this goes straight to upstream's stdio fallback (what C SDL
/// does when `SDL_ShowMessageBox()` fails).
///
/// On Emscripten that fallback is the page's `window.prompt()`, asked
/// until the reply is one of `a`, `r`, `i` or `A` (a cancelled prompt is
/// `i`). Under node there is no `window`: the prompt's JavaScript throws a
/// `ReferenceError`, which ends the program with exit status 1. That's how
/// upstream's ends there too, one step earlier: its message box on
/// Emscripten (the video driver's, not translated yet) is `alert()`, which
/// node doesn't have either ("alert is not defined"). Set
/// `SDL_HINT_ASSERT` (as tests do) or an assertion handler to answer
/// without asking.
pub fn default_assertion_handler(data: &AssertData) -> AssertState {
    let message = render_assert_message(data);

    debug_print(&format!("\n\n{message}\n\n"));

    // let env. variable override, so unit tests won't block in a GUI.
    if let Some(hint) = hints::get(hints::ASSERT) {
        return match hint.as_str() {
            "abort" => AssertState::Abort,
            "break" => AssertState::Break,
            "retry" => AssertState::Retry,
            "ignore" => AssertState::Ignore,
            "always_ignore" => AssertState::AlwaysIgnore,
            _ => AssertState::Abort, // oh well.
        };
    }

    // Leave fullscreen mode, if possible (scary!) -- needs the video subsystem.

    #[cfg(target_os = "emscripten")]
    {
        prompt_assertion(&message)
    }
    #[cfg(not(target_os = "emscripten"))]
    {
        stdio_prompt_assertion()
    }
}

/// Upstream's Emscripten branch of `SDL_PromptAssertion()`: ask with
/// `window.prompt()`.
#[cfg(target_os = "emscripten")]
fn prompt_assertion(message: &str) -> AssertState {
    // (the JavaScript reads the message up to a NUL)
    let message = std::ffi::CString::new(message.replace('\0', "")).unwrap_or_default();
    // This is nasty, but we can't block on a custom UI.
    loop {
        // SAFETY: the message is NUL-terminated.
        let reply = unsafe { sdl3_emscripten::assert::sdl3_prompt_assertion(message.as_ptr()) };
        match u8::try_from(reply).map(char::from) {
            Ok('a') => return AssertState::Abort,
            // (currently) no break functionality on Emscripten
            // Ok('b') => return AssertState::Break,
            Ok('r') => return AssertState::Retry,
            Ok('i') => return AssertState::Ignore,
            Ok('A') => return AssertState::AlwaysIgnore,
            _ => {} // (asks again)
        }
    }
}

/// Upstream's stdio branch of `SDL_PromptAssertion()`.
#[cfg(not(target_os = "emscripten"))]
fn stdio_prompt_assertion() -> AssertState {
    // this is a little hacky.
    let mut state = AssertState::Abort;
    loop {
        use std::io::{BufRead, Write};
        let mut stderr = std::io::stderr();
        let _ = write!(stderr, "Abort/Break/Retry/Ignore/AlwaysIgnore? [abriA] : ");
        let _ = stderr.flush();
        let mut buf = String::new();
        match std::io::stdin().lock().read_line(&mut buf) {
            Ok(0) | Err(_) => break,
            Ok(_) => {}
        }
        if buf.starts_with('a') {
            state = AssertState::Abort;
            break;
        } else if buf.starts_with('b') {
            state = AssertState::Break;
            break;
        } else if buf.starts_with('r') {
            state = AssertState::Retry;
            break;
        } else if buf.starts_with('i') {
            state = AssertState::Ignore;
            break;
        } else if buf.starts_with('A') {
            state = AssertState::AlwaysIgnore;
            break;
        }
    }
    state
}

/// Never call this directly; use the assertion macros.
/// Translation of `SDL_ReportAssertion()`.
pub fn report_assertion(data: &'static AssertData) -> AssertState {
    let mut state = AssertState::Ignore;

    drop(REPORT_SPINLOCK.lock());
    ASSERTION_MUTEX.lock();

    add_assertion_to_report(data);

    let running = ASSERTION_RUNNING.fetch_add(1, Ordering::SeqCst) + 1;
    if running > 1 {
        // assert during assert! Abort.
        if running == 2 {
            abort_assertion();
        } else if running == 3 {
            // Abort asserted!
            std::process::abort();
        } else {
            loop {
                // do nothing but spin; what else can you do?!
                std::hint::spin_loop();
            }
        }
    }

    if !data.always_ignore() {
        let handler = globals().assertion_handler.clone();
        state = match handler {
            Some(handler) => handler(data),
            None => default_assertion_handler(data),
        };
    }

    match state {
        AssertState::AlwaysIgnore => {
            state = AssertState::Ignore;
            data.always_ignore.store(true, Ordering::Relaxed);
        }
        AssertState::Ignore | AssertState::Retry | AssertState::Break => {} // macro handles these.
        AssertState::Abort => abort_assertion(),
    }

    ASSERTION_RUNNING.fetch_sub(1, Ordering::SeqCst);
    ASSERTION_MUTEX.unlock();

    state
}

/// Translation of `SDL_AssertionsQuit()`; called from [`init::quit`](crate::init::quit).
pub(crate) fn assertions_quit() {
    if assert_level(cfg!(debug_assertions)) > 0 {
        generate_assertion_report();
    }
}

/// Set an application-defined assertion handler.
/// Translation of `SDL_SetAssertionHandler(handler, userdata)`.
pub fn set_assertion_handler(handler: impl Fn(&AssertData) -> AssertState + Send + Sync + 'static) {
    globals().assertion_handler = Some(Arc::new(handler));
}

/// Go back to the default handler. Translation of `SDL_SetAssertionHandler(NULL, NULL)`.
pub fn reset_assertion_handler() {
    globals().assertion_handler = None;
}

/// The current assertion handler, or `None` if it is the default one
/// ([`default_assertion_handler`]). Translation of `SDL_GetAssertionHandler()`.
pub fn assertion_handler() -> Option<AssertionHandler> {
    globals().assertion_handler.clone()
}

/// A list of all assertion failures since the last reset, newest first.
/// Translation of `SDL_GetAssertionReport()`.
pub fn assertion_report() -> Vec<&'static AssertData> {
    globals().triggered_assertions.clone()
}

/// Clear the list of all assertion failures. Translation of `SDL_ResetAssertionReport()`.
pub fn reset_assertion_report() {
    let items = std::mem::take(&mut globals().triggered_assertions);
    for item in items {
        item.always_ignore.store(false, Ordering::Relaxed);
        item.trigger_count.store(0, Ordering::Relaxed);
        item.in_report.store(false, Ordering::Relaxed);
    }
}

/// Stop the program in the debugger, or abort if no breakpoint instruction is
/// known for this target. Translation of `SDL_TriggerBreakpoint()`.
#[inline(always)]
pub fn trigger_breakpoint() {
    #[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
    // SAFETY: `int3` raises a debug trap and touches no memory or registers;
    // without a debugger attached the OS delivers SIGTRAP / an exception,
    // exactly as with C SDL's `__debugbreak()` / `int $3`.
    unsafe {
        std::arch::asm!("int3", options(nomem, nostack));
    }
    #[cfg(any(target_arch = "riscv32", target_arch = "riscv64"))]
    // SAFETY: as above, `ebreak` only raises a breakpoint exception.
    unsafe {
        std::arch::asm!("ebreak", options(nomem, nostack));
    }
    #[cfg(all(target_arch = "aarch64", target_vendor = "apple"))]
    // SAFETY: as above (upstream uses `brk #22` on Apple ARM64).
    unsafe {
        std::arch::asm!("brk #22", options(nomem, nostack));
    }
    #[cfg(all(target_arch = "aarch64", not(target_vendor = "apple")))]
    // SAFETY: as above (upstream uses `brk #0xF000` on Windows ARM64; it is
    // the conventional debugger trap on other AArch64 targets too).
    unsafe {
        std::arch::asm!("brk #0xF000", options(nomem, nostack));
    }
    #[cfg(not(any(
        target_arch = "x86",
        target_arch = "x86_64",
        target_arch = "riscv32",
        target_arch = "riscv64",
        target_arch = "aarch64"
    )))]
    {
        // __builtin_trap()
        std::process::abort();
    }
}

/// The body of `SDL_enabled_assert(condition)`.
#[doc(hidden)]
#[macro_export]
macro_rules! __sdl_enabled_assert {
    ($cond:expr) => {
        $crate::__sdl_enabled_assert!(@text stringify!($cond), $cond)
    };
    (@text $text:expr, $cond:expr) => {
        while !($cond) {
            static SDL_ASSERT_DATA: $crate::assert::AssertData = $crate::assert::AssertData::new(
                $text,
                module_path!(),
                file!(),
                line!(),
            );
            let sdl_assert_state = $crate::assert::report_assertion(&SDL_ASSERT_DATA);
            if sdl_assert_state == $crate::assert::AssertState::Retry {
                continue; // go again.
            } else if sdl_assert_state == $crate::assert::AssertState::Break {
                $crate::assert::trigger_breakpoint();
            }
            break; // not retrying.
        }
    };
}

/// The body of `SDL_disabled_assert(condition)`: type-check, never evaluate.
#[doc(hidden)]
#[macro_export]
macro_rules! __sdl_disabled_assert {
    ($cond:expr) => {
        if false {
            let _ = $cond;
        }
    };
}

/// An assertion active in debug builds (level ≥ 2). Translation of `SDL_assert()`.
#[macro_export]
macro_rules! sdl_assert {
    // C's `SDL_assert(!"message")` idiom: always fails, reporting `!"message"`.
    (! $msg:literal $(,)?) => {{
        if $crate::assert::assert_level(cfg!(debug_assertions)) >= 2 {
            $crate::__sdl_enabled_assert!(@text concat!("!", stringify!($msg)), false);
        }
    }};
    ($cond:expr $(,)?) => {{
        if $crate::assert::assert_level(cfg!(debug_assertions)) >= 2 {
            $crate::__sdl_enabled_assert!($cond);
        } else {
            $crate::__sdl_disabled_assert!($cond);
        }
    }};
}

/// An assertion active in release builds too (level ≥ 1). Translation of `SDL_assert_release()`.
#[macro_export]
macro_rules! sdl_assert_release {
    // C's `SDL_assert(!"message")` idiom: always fails, reporting `!"message"`.
    (! $msg:literal $(,)?) => {{
        if $crate::assert::assert_level(cfg!(debug_assertions)) >= 1 {
            $crate::__sdl_enabled_assert!(@text concat!("!", stringify!($msg)), false);
        }
    }};
    ($cond:expr $(,)?) => {{
        if $crate::assert::assert_level(cfg!(debug_assertions)) >= 1 {
            $crate::__sdl_enabled_assert!($cond);
        } else {
            $crate::__sdl_disabled_assert!($cond);
        }
    }};
}

/// An expensive assertion, only active at level 3. Translation of `SDL_assert_paranoid()`.
#[macro_export]
macro_rules! sdl_assert_paranoid {
    // C's `SDL_assert(!"message")` idiom: always fails, reporting `!"message"`.
    (! $msg:literal $(,)?) => {{
        if $crate::assert::assert_level(cfg!(debug_assertions)) >= 3 {
            $crate::__sdl_enabled_assert!(@text concat!("!", stringify!($msg)), false);
        }
    }};
    ($cond:expr $(,)?) => {{
        if $crate::assert::assert_level(cfg!(debug_assertions)) >= 3 {
            $crate::__sdl_enabled_assert!($cond);
        } else {
            $crate::__sdl_disabled_assert!($cond);
        }
    }};
}

/// An assertion that is always active. Translation of `SDL_assert_always()`.
#[macro_export]
macro_rules! sdl_assert_always {
    (! $msg:literal $(,)?) => {{
        $crate::__sdl_enabled_assert!(@text concat!("!", stringify!($msg)), false);
    }};
    ($cond:expr $(,)?) => {{
        $crate::__sdl_enabled_assert!($cond);
    }};
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicUsize;

    #[test]
    fn handler_report_and_states() {
        let _l = crate::test_support::test_lock();
        reset_assertion_report();

        let calls = Arc::new(AtomicUsize::new(0));
        let c2 = calls.clone();
        set_assertion_handler(move |data| {
            let n = c2.fetch_add(1, Ordering::SeqCst);
            assert!(data.condition().contains("x == 2"));
            // Retry once, then always ignore.
            if n == 0 {
                AssertState::Retry
            } else {
                AssertState::AlwaysIgnore
            }
        });
        assert!(assertion_handler().is_some());

        let x = 1;
        let fire = || crate::sdl_assert_always!(x == 2);
        fire();
        assert_eq!(
            calls.load(Ordering::SeqCst),
            2,
            "retry re-evaluates and reports again"
        );
        fire();
        assert_eq!(
            calls.load(Ordering::SeqCst),
            2,
            "always-ignore skips the handler"
        );

        let report = assertion_report();
        assert_eq!(report.len(), 1);
        let item = report[0];
        assert_eq!(item.trigger_count(), 3);
        assert!(item.always_ignore());
        assert!(item.filename().ends_with("mod.rs"));
        assert_eq!(item.function(), module_path!());
        let msg = render_assert_message(item);
        assert!(
            msg.starts_with("Assertion failure at sdl3::assert::tests ("),
            "{msg}"
        );
        assert!(
            msg.ends_with("), triggered 3 times:\n  'x == 2'")
                || msg.ends_with("), triggered 3 times:\r\n  'x == 2'"),
            "{msg}"
        );

        // Passing assertions never report.
        crate::sdl_assert_always!(x == 1);
        assert_eq!(assertion_report().len(), 1);

        reset_assertion_report();
        assert!(assertion_report().is_empty());
        assert_eq!(item.trigger_count(), 0);
        assert!(!item.always_ignore());

        reset_assertion_handler();
        assert!(assertion_handler().is_none());
    }

    #[test]
    fn default_handler_obeys_hint() {
        let _l = crate::test_support::test_lock();
        reset_assertion_report();
        log::disable_output();
        // (over SDL_ASSERT in the environment, which tools/ios_run.py sets)
        let set = |v| hints::set_with_priority(hints::ASSERT, Some(v), hints::Priority::Override);
        set("always_ignore").unwrap();
        let fire = || crate::sdl_assert_release!(1 + 1 == 3);
        fire();
        fire();
        let report = assertion_report();
        assert_eq!(report.len(), 1);
        assert!(report[0].always_ignore());
        assert_eq!(report[0].trigger_count(), 2);

        static D: AssertData = AssertData::new("c", "f", "file.c", 7);
        for (v, s) in [
            ("abort", AssertState::Abort),
            ("break", AssertState::Break),
            ("retry", AssertState::Retry),
            ("ignore", AssertState::Ignore),
            ("always_ignore", AssertState::AlwaysIgnore),
            ("nonsense", AssertState::Abort),
        ] {
            set(v).unwrap();
            assert_eq!(default_assertion_handler(&D), s);
        }
        hints::reset(hints::ASSERT);
        reset_assertion_report();
        log::reset_output();
    }

    #[test]
    fn disabled_assert_does_not_evaluate() {
        let mut evaluated = false;
        crate::__sdl_disabled_assert!({
            evaluated = true;
            false
        });
        assert!(!evaluated);
        assert_eq!(assert_level(true), 2);
        assert_eq!(assert_level(false), 1);
        // Paranoid assertions are off at the default levels.
        crate::sdl_assert_paranoid!({
            evaluated = true;
            true
        });
        assert!(!evaluated);
    }

    /// The `window.prompt()` loop, with a page's prompt stubbed: the replies,
    /// and what it's asked.
    #[test]
    #[cfg(target_os = "emscripten")]
    fn emscripten_prompt() {
        use sdl3_emscripten::{em_asm, em_asm_int};
        let _l = crate::test_support::test_lock();
        hints::reset(hints::ASSERT);
        log::disable_output();
        static D: AssertData = AssertData::new("x == 2", "f", "file.c", 7);
        // SAFETY: plain JavaScript.
        unsafe {
            em_asm!(
                r#"
                Module['sdl3_prompts'] = [];
                globalThis.window = { prompt: function(str, def) {
                    Module['sdl3_prompts'].push([str, def]);
                    return Module['sdl3_replies'].shift();
                } };
            "#
            )
        };
        for (replies, state, asked) in [
            ("[null]", AssertState::Ignore, 1),
            ("['r']", AssertState::Retry, 1),
            ("['i']", AssertState::Ignore, 1),
            ("['A']", AssertState::AlwaysIgnore, 1),
            ("['a']", AssertState::Abort, 1),
            // asked again: too long, empty, unknown, no break
            ("['ra', '', 'x', 'b', 'R', 'r']", AssertState::Retry, 6),
        ] {
            let js = format!("Module['sdl3_replies'] = {replies}; Module['sdl3_prompts'] = [];\0");
            // SAFETY: plain JavaScript, NUL-terminated.
            unsafe { em_asm!("eval(UTF8ToString($0));", js.as_ptr()) };
            assert_eq!(default_assertion_handler(&D), state, "{replies}");
            // SAFETY: plain JavaScript.
            let n = unsafe { em_asm_int!("return Module['sdl3_prompts'].length;") };
            assert_eq!(n, asked, "{replies}");
        }
        // What it asks, with "i" as the default reply.
        let expected = format!(
            "{}\n\nAbort/Retry/Ignore/AlwaysIgnore? [ariA] :\0",
            render_assert_message(&D)
        );
        // SAFETY: plain JavaScript, NUL-terminated.
        let same = unsafe {
            em_asm_int!(
                r#"var p = Module['sdl3_prompts'][0]; return (p[0] === UTF8ToString($0) && p[1] === 'i') ? 1 : 0;"#,
                expected.as_ptr()
            )
        };
        assert_eq!(same, 1);
        // SAFETY: plain JavaScript.
        unsafe { em_asm!("delete globalThis.window;") };
        log::reset_output();
    }
}
