// The program of the Emscripten Asyncify and JSPI tests.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! The program `emscripten_asyncify.rs` (linked with `-sASYNCIFY=1`) and
//! `emscripten_jspi.rs` (linked with `-sJSPI`) both are; see the former.

use std::fmt::Write as _;
use std::panic::{self, AssertUnwindSafe};
use std::process::ExitCode;
use std::sync::Mutex;
use std::time::Duration;

use sdl3::app::{self, AppCallbacks, AppResult};
use sdl3::events::window::WindowFlags;
use sdl3::events::{self, Event, EventType, UserEvent};
use sdl3::hints;
use sdl3::init::{self, InitFlags};
use sdl3::properties::Properties;
use sdl3::timer::{self, Timer};
use sdl3::video::gl::{self, GlContext};
use sdl3::video::messagebox::{
    show_message_box, show_simple_message_box, MessageBoxButtonData, MessageBoxButtonFlags,
    MessageBoxColor, MessageBoxColorScheme, MessageBoxData, MessageBoxFlags,
};
use sdl3::video::window::{
    PROP_WINDOW_CREATE_EMSCRIPTEN_CANVAS_ID_STRING, PROP_WINDOW_CREATE_FLAGS_NUMBER,
    PROP_WINDOW_CREATE_HEIGHT_NUMBER, PROP_WINDOW_CREATE_TITLE_STRING,
    PROP_WINDOW_CREATE_WIDTH_NUMBER,
};
use sdl3::video::{self as sdl_video, Window};
use sdl3_emscripten::{em_asm, em_asm_int, sys};

/// How the program is linked: Asyncify (`-sASYNCIFY=1`) or JSPI
/// (`-sJSPI`), which `emscripten_has_asyncify()` answers 1 and 2 for.
#[derive(Clone, Copy)]
pub(crate) struct Mode {
    pub(crate) jspi: bool,
}

fn required(capability: &str) -> bool {
    std::env::var("SDL3_TEST_REQUIRE")
        .unwrap_or_default()
        .split(',')
        .map(str::trim)
        .any(|c| c == "all" || c == capability)
}

fn skip(capability: &str, reason: &str) {
    if required(capability) {
        panic!("required capability {capability} unavailable: {reason}");
    }
    println!("note: skipping, {capability} unavailable: {reason}");
}

fn in_browser() -> bool {
    // SAFETY: plain JavaScript.
    unsafe {
        em_asm_int!(
            r#"
            var node = typeof process === 'object' && !!(process.versions && process.versions.node);
            return !node && typeof window === 'object' && typeof document === 'object' && typeof navigator === 'object' ? 1 : 0;
        "#
        ) != 0
    }
}

fn has_webgl() -> bool {
    // SAFETY: plain JavaScript.
    unsafe {
        em_asm_int!(
            "try { return document.createElement('canvas').getContext('webgl') ? 1 : 0; } catch (e) { return 0; }"
        ) != 0
    }
}

// -- JavaScript helpers --------------------------------------------------------

/// The value of the JavaScript expression `expr` (evaluated in the glue's
/// scope: `Module`, `GL`, …), as a string.
fn js(expr: &str) -> String {
    let code = format!("{expr}\0");
    // SAFETY: plain JavaScript; the code is NUL-terminated.
    let len = unsafe {
        em_asm_int!(
            r#"
            var v = eval(UTF8ToString($0));
            Module['sdl3_test_s'] = new TextEncoder().encode(String(v));
            return Module['sdl3_test_s'].length;
        "#,
            code.as_ptr()
        )
    };
    let mut buf = vec![0u8; len as usize];
    // SAFETY: writes the `len` bytes made above.
    unsafe {
        em_asm!(
            "HEAPU8.set(Module['sdl3_test_s'], $0); delete Module['sdl3_test_s'];",
            buf.as_mut_ptr()
        )
    };
    String::from_utf8(buf).unwrap()
}

/// Run JavaScript statements.
fn js_run(code: &str) {
    js(&format!("(function() {{ {code} }})()"));
}

/// The event loop's log: `what` once a `setTimeout(0)` set now has run.
fn later(what: &str) {
    js_run(&format!(
        "setTimeout(() => Module.sdl3TestLog.push('{what}'), 0);"
    ));
}

/// What the event loop has run, as JSON.
fn event_log() -> String {
    js("JSON.stringify(Module.sdl3TestLog)")
}

fn clear_event_log() {
    js_run("Module.sdl3TestLog = [];");
}

// -- the trace -----------------------------------------------------------------

static TRACE: Mutex<String> = Mutex::new(String::new());

/// A line of the trace. (The arguments are evaluated with the trace
/// locked: nothing that sleeps goes in them, since a timer that fires
/// meanwhile writes to the trace too.)
macro_rules! out {
    ($($arg:tt)*) => {
        writeln!(TRACE.lock().unwrap(), $($arg)*).unwrap()
    };
}

/// Start a step's part of the trace.
fn step(name: &str) {
    out!("== {name}");
}

fn sections(trace: &str) -> Vec<(String, String)> {
    let mut out: Vec<(String, String)> = Vec::new();
    for line in trace.lines() {
        if let Some(name) = line.strip_prefix("== ") {
            out.push((name.to_owned(), String::new()));
        } else if let Some((_, body)) = out.last_mut() {
            body.push_str(line);
            body.push('\n');
        }
    }
    out
}

/// The steps that need a page (and `gl_swap`, WebGL), which under node
/// aren't there.
const BROWSER_STEPS: [&str; 4] = ["video", "message_box", "framebuffer", "gl_swap"];

/// Upstream's trace for `mode` (see [`EXPECTED`]), with the steps there are
/// here, and this repository's answer for `alert()`'s button.
fn expected(mode: Mode, browser: bool, webgl: bool) -> Vec<(String, String)> {
    let mut trace = EXPECTED.to_owned();
    if mode.jspi {
        trace = trace
            .replace("has_asyncify 1", "has_asyncify 2")
            .replace(MAIN_CALLBACKS_ASYNCIFY, MAIN_CALLBACKS_JSPI);
    }
    // (upstream leaves the caller's button ID as it was, -2 here; this
    // returns -1, as with alert() on every page)
    let trace = trace.replace(
        "without asyncify: 1 button -2 alerts",
        "without asyncify: 1 button -1 alerts",
    );
    sections(&trace)
        .into_iter()
        .filter(|(name, _)| {
            (browser || !BROWSER_STEPS.contains(&name.as_str())) && (webgl || name != "gl_swap")
        })
        .collect()
}

/// Compare the trace with upstream's, step by step; the result.
fn finish(mode: Mode, browser: bool, webgl: bool) -> bool {
    let trace = TRACE.lock().unwrap().clone();
    let expected = expected(mode, browser, webgl);
    let actual = sections(&trace);
    if std::env::var_os("SDL3_TEST_TRACE").is_some() {
        println!("{trace}");
    }
    let mut passed = 0;
    let mut failed = Vec::new();
    for (name, want) in &expected {
        let got = actual.iter().find(|(n, _)| n == name).map(|(_, b)| b);
        if got == Some(want) {
            passed += 1;
            println!("test {name} ... ok");
        } else {
            println!("test {name} ... FAILED");
            println!(
                "--- upstream's C:\n{want}--- here:\n{}",
                got.map_or("(no such step)\n", |s| s)
            );
            failed.push(name.clone());
        }
    }
    for (name, body) in &actual {
        if !expected.iter().any(|(n, _)| n == name) {
            println!("test {name} ... FAILED");
            println!("(a step upstream's trace hasn't)\n{body}");
            failed.push(name.clone());
        }
    }
    if !failed.is_empty() {
        println!("\nfailures:");
        for name in &failed {
            println!("    {name}");
        }
    }
    let status = if failed.is_empty() { "ok" } else { "FAILED" };
    println!(
        "\ntest result: {status}. {passed} passed; {} failed",
        failed.len()
    );
    failed.is_empty()
}

/// Run a step; a panic is recorded in its trace.
fn guarded(name: &str, f: impl FnOnce()) {
    if panic::catch_unwind(AssertUnwindSafe(f)).is_err() {
        out!("FAILED: {name} panicked");
    }
}

// -- the steps -----------------------------------------------------------------

fn asyncify_step() {
    step("asyncify");
    // SAFETY: a plain call.
    let mode = unsafe { sys::emscripten_has_asyncify() };
    out!("has_asyncify {mode}");
}

fn delay_step() {
    step("delay");
    clear_event_log();
    later("timeout 1");
    // (not how long it took: under node a setTimeout() of 20 ms can end a
    // little early on SDL's clock)
    timer::delay(Duration::from_millis(20));
    out!("delay 20: {}", event_log());
    hints::set(hints::EMSCRIPTEN_ASYNCIFY, "0").unwrap();
    later("timeout 2");
    timer::delay(Duration::from_millis(20));
    out!("delay 20 without asyncify: {}", event_log());
    hints::reset(hints::EMSCRIPTEN_ASYNCIFY);
    timer::delay(Duration::ZERO);
    out!("delay 0: {}", event_log());
    later("timeout 3");
    timer::delay(Duration::from_micros(500));
    out!("delay 0.5 ms: {}", event_log());
    later("timeout 4");
    timer::delay_precise(Duration::from_millis(5));
    out!("delay precise 5 ms: {}", event_log());
}

fn one_shot(ms: u64) {
    Timer::new(Duration::from_millis(ms), |interval| {
        out!("timer {} fired", interval.as_millis());
        None
    })
    .unwrap()
    .detach();
}

fn timers_step() {
    step("timers");
    one_shot(10);
    timer::delay(Duration::from_millis(50));
    out!("delay 50 returned");
    hints::set(hints::EMSCRIPTEN_ASYNCIFY, "0").unwrap();
    one_shot(5);
    timer::delay(Duration::from_millis(30));
    out!("delay 30 without asyncify returned");
    hints::reset(hints::EMSCRIPTEN_ASYNCIFY);
    timer::delay(Duration::from_millis(10));
    out!("delay 10 returned");
}

fn push_later(ms: u64, code: i32) {
    Timer::new(Duration::from_millis(ms), move |_| {
        events::push(Event::User(UserEvent {
            event_type: EventType::USER,
            timestamp: Duration::ZERO,
            window_id: 0,
            code,
            data1: None,
            data2: None,
        }))
        .unwrap();
        None
    })
    .unwrap()
    .detach();
}

fn waited(event: Option<Event>) -> String {
    match event {
        Some(Event::User(e)) => format!("1 type 0x{:x} code {}", e.event_type.0, e.code),
        Some(e) => format!("1 type 0x{:x} code 0", e.event_type().0),
        None => "0".into(),
    }
}

fn wait_event_step() {
    step("wait_event");
    events::flush_events(EventType::FIRST, EventType::LAST);
    push_later(10, 7);
    let t0 = timer::ticks();
    let event = events::wait_timeout(Some(Duration::from_millis(2000))).unwrap();
    let early = (timer::ticks() - t0 < Duration::from_millis(1000)) as i32;
    out!("wait 2000: {}, before the timeout {early}", waited(event));
    hints::set(hints::EMSCRIPTEN_ASYNCIFY, "0").unwrap();
    push_later(10, 8);
    let event = events::wait_timeout(Some(Duration::from_millis(50))).unwrap();
    out!("wait 50 without asyncify: {}", waited(event));
    hints::reset(hints::EMSCRIPTEN_ASYNCIFY);
    let event = events::wait_timeout(Some(Duration::from_millis(2000))).unwrap();
    out!("wait 2000: {}", waited(event));
}

fn video_step() {
    step("video");
    let ok = init::init_subsystem(InitFlags::VIDEO).is_ok() as i32;
    out!(
        "init {ok} driver {}",
        sdl_video::current_video_driver().unwrap_or("(none)")
    );
}

/// The C's `answer()`: once the dialog is open, what it shows goes in the
/// event loop's log, then `action` (a key, the hover, or a button's
/// index).
fn answer(action: &str) {
    js_run(&format!(
        r#"Module.sdl3TestLog = []; var log = Module.sdl3TestLog; var poll = setInterval(() => {{
            var d = document.querySelector('dialog.SDL3_messagebox[open]'); if (!d) return; clearInterval(poll);
            log.push('id ' + /^SDL3_messagebox_[0-9]+$/.test(d.id) + ' modal ' + d.matches(':modal') + ' in body ' + (d.parentNode === document.body));
            log.push('h1 [' + d.querySelector('h1').innerText + '] p [' + d.querySelector('p').innerText + ']');
            log.push('color [' + d.style.color + '] background [' + d.style.backgroundColor + ']');
            var buttons = d.querySelectorAll('button');
            for (var b of buttons) log.push('button [' + b.innerText + '] border [' + b.style.borderColor + '] background [' + b.style.backgroundColor + ']');
            var action = '{action}';
            if (action == 'enter') d.dispatchEvent(new KeyboardEvent('keydown', {{ key: 'Enter', bubbles: true, cancelable: true }}));
            else if (action == 'escape') d.dispatchEvent(new KeyboardEvent('keydown', {{ key: 'Escape', bubbles: true, cancelable: true }}));
            else if (action == 'hover') {{ buttons[0].dispatchEvent(new MouseEvent('mouseenter')); log.push('hovered [' + buttons[0].style.backgroundColor + ']'); buttons[0].dispatchEvent(new MouseEvent('mouseleave')); log.push('left [' + buttons[0].style.backgroundColor + ']'); buttons[0].click(); }}
            else if (action == 'nothing') {{ d.dispatchEvent(new KeyboardEvent('keydown', {{ key: 'Escape', bubbles: true, cancelable: true }})); setTimeout(() => {{ log.push('still open ' + d.open); buttons[0].click(); }}, 30); }}
            else buttons[Number(action)].click();
        }}, 5);"#
    ));
}

fn show(label: &str, data: &MessageBoxData, action: &str) {
    answer(action);
    let result = show_message_box(data);
    let (ok, button) = match result {
        Ok(button) => (1, button),
        Err(_) => (0, -2),
    };
    out!("{label}: {ok} button {button}");
    out!("{label}: {}", event_log());
}

fn button(flags: MessageBoxButtonFlags, button_id: i32, text: &str) -> MessageBoxButtonData {
    MessageBoxButtonData {
        flags,
        button_id,
        text: text.into(),
    }
}

fn message_box_step() {
    step("message_box");
    let rgb = |r, g, b| MessageBoxColor { r, g, b };
    let scheme = MessageBoxColorScheme {
        colors: [
            rgb(10, 20, 30),    // background
            rgb(200, 210, 220), // text
            rgb(1, 2, 3),       // button border
            rgb(40, 50, 60),    // button background
            rgb(70, 80, 90),    // button selected
        ],
    };
    let mut data = MessageBoxData {
        flags: MessageBoxFlags::WARNING,
        window: None,
        title: "The title".into(),
        message: "The message\non two lines".into(),
        buttons: vec![
            button(MessageBoxButtonFlags::RETURNKEY_DEFAULT, 1, "OK"),
            button(MessageBoxButtonFlags::ESCAPEKEY_DEFAULT, 2, "Cancel"),
            button(MessageBoxButtonFlags::NONE, 3, "Other"),
        ],
        color_scheme: Some(scheme),
    };
    show("enter", &data, "enter");
    show("escape", &data, "escape");
    show("click", &data, "2");
    show("hover", &data, "hover");
    data.color_scheme = None;
    show("no colors", &data, "1");
    let no_keys = MessageBoxData {
        flags: MessageBoxFlags::INFORMATION,
        window: None,
        title: "No keys".into(),
        message: String::new(),
        buttons: vec![button(MessageBoxButtonFlags::NONE, 5, "Plain")],
        color_scheme: None,
    };
    show("no default", &no_keys, "nothing");
    answer("enter");
    let simple =
        show_simple_message_box(MessageBoxFlags::ERROR, "Simple", "A simple one", None).is_ok();
    out!("simple: {}", simple as i32);
    out!("simple: {}", event_log());
    out!(
        "dialogs {}",
        js("String(document.querySelectorAll('dialog.SDL3_messagebox').length)")
    );
    // without Asyncify: alert()
    js_run("Module.sdl3TestAlert = window.alert; Module.sdl3TestAlerts = []; window.alert = (m) => Module.sdl3TestAlerts.push(m);");
    hints::set(hints::EMSCRIPTEN_ASYNCIFY, "0").unwrap();
    let result = show_message_box(&data);
    hints::reset(hints::EMSCRIPTEN_ASYNCIFY);
    let (ok, button) = match result {
        Ok(button) => (1, button),
        Err(_) => (0, -2),
    };
    out!(
        "without asyncify: {ok} button {button} alerts {}",
        js("JSON.stringify(Module.sdl3TestAlerts)")
    );
    js_run("window.alert = Module.sdl3TestAlert; delete Module.sdl3TestAlert; delete Module.sdl3TestAlerts;");
    // the dialogs stay in the page (upstream's); take them away
    js_run("document.querySelectorAll('dialog.SDL3_messagebox').forEach((d) => d.remove());");
}

fn framebuffer_step() {
    step("framebuffer");
    let w = Window::create("framebuffer", 32, 16, WindowFlags::NONE).unwrap();
    let surface = w.surface().unwrap();
    {
        let mut s = surface.lock();
        let red = s.map_rgba(255, 0, 0, 255);
        s.fill_rect(None, red).unwrap();
    }
    clear_event_log();
    later("timeout 1");
    let r = surface.update().is_ok() as i32;
    out!("update {r}: {}", event_log());
    hints::set(hints::EMSCRIPTEN_ASYNCIFY, "0").unwrap();
    later("timeout 2");
    let r = surface.update().is_ok() as i32;
    out!("update without asyncify {r}: {}", event_log());
    hints::reset(hints::EMSCRIPTEN_ASYNCIFY);
    timer::delay(Duration::ZERO);
    out!("delay 0: {}", event_log());
    out!(
        "canvas {}",
        js("(function() { var c = document.getElementById('canvas'); var o = document.createElement('canvas'); o.width = c.width; o.height = c.height; var g = o.getContext('2d'); g.drawImage(c, 0, 0); return c.width + 'x' + c.height + ' ' + Array.from(g.getImageData(1, 1, 1, 1).data).join(','); })()")
    );
    drop(surface);
    w.destroy();
}

fn gl_swap_step() {
    step("gl_swap");
    // (a canvas of its own: the framebuffer's has a 2D context)
    js_run("var c = document.createElement('canvas'); c.id = 'gl'; document.body.appendChild(c);");
    let p = Properties::new();
    p.set(PROP_WINDOW_CREATE_TITLE_STRING, "gl").unwrap();
    p.set(PROP_WINDOW_CREATE_WIDTH_NUMBER, 32i64).unwrap();
    p.set(PROP_WINDOW_CREATE_HEIGHT_NUMBER, 16i64).unwrap();
    p.set(
        PROP_WINDOW_CREATE_FLAGS_NUMBER,
        WindowFlags::OPENGL.0 as i64,
    )
    .unwrap();
    p.set(PROP_WINDOW_CREATE_EMSCRIPTEN_CANVAS_ID_STRING, "#gl")
        .unwrap();
    let w = Window::create_with_properties(&p).unwrap();
    let c = GlContext::new(&w);
    out!("context {}", c.is_ok() as i32);
    clear_event_log();
    later("timeout 1");
    let r = gl::gl_swap_window(&w).is_ok() as i32;
    out!("swap {r}: {}", event_log());
    hints::set(hints::EMSCRIPTEN_ASYNCIFY, "0").unwrap();
    later("timeout 2");
    let r = gl::gl_swap_window(&w).is_ok() as i32;
    out!("swap without asyncify {r}: {}", event_log());
    hints::reset(hints::EMSCRIPTEN_ASYNCIFY);
    timer::delay(Duration::ZERO);
    out!("delay 0: {}", event_log());
    drop(c);
    w.destroy();
    js_run("document.getElementById('gl').remove();");
}

// -- the main callbacks --------------------------------------------------------

struct App {
    mode: Mode,
    browser: bool,
    webgl: bool,
    iterations: u32,
}

impl App {
    fn iterate_step(&mut self) {
        self.iterations += 1;
        later(&format!("timeout {}", self.iterations));
        // (with JSPI the main loop's function can't suspend: emscripten
        // calls it without WebAssembly.promising, and a sleep throws a
        // SuspendError, as upstream's does; the hint keeps it from sleeping)
        let jspi = self.mode.jspi;
        if jspi {
            hints::set(hints::EMSCRIPTEN_ASYNCIFY, "0").unwrap();
        }
        timer::delay(Duration::from_millis(5));
        if jspi {
            hints::reset(hints::EMSCRIPTEN_ASYNCIFY);
        }
        out!("iteration {}: {}", self.iterations, event_log());
    }
}

impl AppCallbacks for App {
    fn iterate(&mut self) -> AppResult {
        let ok = panic::catch_unwind(AssertUnwindSafe(|| self.iterate_step())).is_ok();
        if !ok {
            out!("FAILED: iteration {} panicked", self.iterations);
        }
        if ok && self.iterations < 3 {
            return AppResult::Continue;
        }
        init::quit();
        if finish(self.mode, self.browser, self.webgl) {
            AppResult::Success
        } else {
            AppResult::Failure
        }
    }

    fn event(&mut self, _event: &Event) -> AppResult {
        AppResult::Continue
    }
}

fn sdl_main(mode: Mode, args: &[String]) -> i32 {
    guarded("asyncify", asyncify_step);
    init::init(InitFlags::EVENTS).unwrap();
    guarded("delay", delay_step);
    guarded("timers", timers_step);
    guarded("wait_event", wait_event_step);
    let browser = in_browser();
    let mut webgl = false;
    if browser {
        guarded("video", video_step);
        guarded("message_box", message_box_step);
        guarded("framebuffer", framebuffer_step);
        webgl = has_webgl();
        if webgl {
            guarded("gl_swap", gl_swap_step);
        } else {
            skip(
                "webgl",
                "no WebGL context (Chromium needs --enable-unsafe-swiftshader --use-angle=swiftshader without a GPU)",
            );
        }
    } else {
        skip(
            "browser",
            "not in a browser (under node: run it with SDL3_EMSCRIPTEN_BROWSER=1, or with CHROME naming Chrome or Chromium)",
        );
    }
    app::enter_app_main_callbacks(args, move |_| {
        step("main_callbacks");
        clear_event_log();
        let app = App {
            mode,
            browser,
            webgl,
            iterations: 0,
        };
        (app, AppResult::Continue)
    })
}

pub(crate) fn main(mode: Mode) -> ExitCode {
    // (as the C, which calls SDL_EnterAppMainCallbacks() from main())
    init::set_main_ready();
    let args: Vec<String> = std::env::args().collect();
    ExitCode::from(sdl_main(mode, &args) as u8)
}

/// The main callbacks' iterations with Asyncify: each one's `delay()`
/// sleeps, and the `setTimeout()` it set runs meanwhile.
const MAIN_CALLBACKS_ASYNCIFY: &str = r#"== main_callbacks
iteration 1: ["timeout 1"]
iteration 2: ["timeout 1","timeout 2"]
iteration 3: ["timeout 1","timeout 2","timeout 3"]
"#;

/// With JSPI, where the iterations' `delay()` waits busily (the hint): each
/// one's `setTimeout()` runs after it.
const MAIN_CALLBACKS_JSPI: &str = r#"== main_callbacks
iteration 1: []
iteration 2: ["timeout 1"]
iteration 3: ["timeout 1","timeout 2"]
"#;

/// Upstream's C SDL (3.5.0, emcc 6.0.13, -sASYNCIFY=1) on the runner's page
/// in headless Chromium: the same steps (scratchpad `pr14/cref/asyncify.c`
/// when this was written). Under node the C has the same trace without the
/// browser's steps; with -sJSPI the same with `has_asyncify 2` and
/// [`MAIN_CALLBACKS_JSPI`].
const EXPECTED: &str = r#"== asyncify
has_asyncify 1
== delay
delay 20: ["timeout 1"]
delay 20 without asyncify: ["timeout 1"]
delay 0: ["timeout 1","timeout 2"]
delay 0.5 ms: ["timeout 1","timeout 2","timeout 3"]
delay precise 5 ms: ["timeout 1","timeout 2","timeout 3","timeout 4"]
== timers
timer 10 fired
delay 50 returned
delay 30 without asyncify returned
timer 5 fired
delay 10 returned
== wait_event
wait 2000: 1 type 0x8000 code 7, before the timeout 1
wait 50 without asyncify: 0
wait 2000: 1 type 0x8000 code 8
== video
init 1 driver emscripten
== message_box
enter: 1 button 1
enter: ["id true modal true in body true","h1 [The title] p [The message\non two lines]","color [rgb(200, 210, 220)] background [rgb(10, 20, 30)]","button [OK] border [rgb(1, 2, 3)] background [rgb(40, 50, 60)]","button [Cancel] border [rgb(1, 2, 3)] background [rgb(40, 50, 60)]","button [Other] border [rgb(1, 2, 3)] background [rgb(40, 50, 60)]"]
escape: 1 button 2
escape: ["id true modal true in body true","h1 [The title] p [The message\non two lines]","color [rgb(200, 210, 220)] background [rgb(10, 20, 30)]","button [OK] border [rgb(1, 2, 3)] background [rgb(40, 50, 60)]","button [Cancel] border [rgb(1, 2, 3)] background [rgb(40, 50, 60)]","button [Other] border [rgb(1, 2, 3)] background [rgb(40, 50, 60)]"]
click: 1 button 3
click: ["id true modal true in body true","h1 [The title] p [The message\non two lines]","color [rgb(200, 210, 220)] background [rgb(10, 20, 30)]","button [OK] border [rgb(1, 2, 3)] background [rgb(40, 50, 60)]","button [Cancel] border [rgb(1, 2, 3)] background [rgb(40, 50, 60)]","button [Other] border [rgb(1, 2, 3)] background [rgb(40, 50, 60)]"]
hover: 1 button 1
hover: ["id true modal true in body true","h1 [The title] p [The message\non two lines]","color [rgb(200, 210, 220)] background [rgb(10, 20, 30)]","button [OK] border [rgb(1, 2, 3)] background [rgb(40, 50, 60)]","button [Cancel] border [rgb(1, 2, 3)] background [rgb(40, 50, 60)]","button [Other] border [rgb(1, 2, 3)] background [rgb(40, 50, 60)]","hovered [rgb(70, 80, 90)]","left [rgb(40, 50, 60)]"]
no colors: 1 button 2
no colors: ["id true modal true in body true","h1 [The title] p [The message\non two lines]","color [] background []","button [OK] border [] background []","button [Cancel] border [] background []","button [Other] border [] background []"]
no default: 1 button 5
no default: ["id true modal true in body true","h1 [No keys] p []","color [] background []","button [Plain] border [] background []","still open true"]
simple: 1
simple: ["id true modal true in body true","h1 [Simple] p [A simple one]","color [] background []","button [OK] border [] background []"]
dialogs 7
without asyncify: 1 button -2 alerts ["The title\n\nThe message\non two lines"]
== framebuffer
update 1: ["timeout 1"]
update without asyncify 1: ["timeout 1"]
delay 0: ["timeout 1","timeout 2"]
canvas 32x16 255,0,0,255
== gl_swap
context 1
swap 1: ["timeout 1"]
swap without asyncify 1: ["timeout 1"]
delay 0: ["timeout 1","timeout 2"]
== main_callbacks
iteration 1: ["timeout 1"]
iteration 2: ["timeout 1","timeout 2"]
iteration 3: ["timeout 1","timeout 2","timeout 3"]
"#;
