// Tests of the Emscripten event handling in a real browser.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! The DOM events of the canvas video driver (`SDL_emscriptenevents.c`)
//! in headless Chromium: a program of its own (`harness = false`) that
//! asks `tools/emscripten_run.py` for the browser, as `emscripten_video.rs`
//! does.
//!
//! Each step dispatches the page's events (synchronous `dispatchEvent()`,
//! through the helpers of [`PRELUDE`]) and writes a trace: what the DOM
//! said (which events SDL's handlers `preventDefault()`ed), the modifier
//! state and the focus, then every SDL event the step made, in a format
//! with every field that matters. [`EXPECTED`] is the trace of upstream's
//! C (`libSDL3.a`, SDL 3.5.0) on the same page with the same steps, and
//! each step's part must be the same here. The steps:
//!
//! * `created`: the window's handlers (html5.h's, by event type), its
//!   canvas's `touch-action`, `/tmp/filedrop`;
//! * `keys`, `modifiers`, `text_input`, `text_input_off`, `lock_keys`: key
//!   presses (DOM codes to scancodes, keys to keycodes, the keys without a
//!   code, the navigation keys that are always prevented, repeats), text
//!   input (`keypress`), the lock-key check (`getModifierState()`);
//! * `mouse`, `capture`, `scaled`, `wheel`: pointer events of a mouse
//!   (motion, the five buttons, enter and leave, the imaginary capture and
//!   the document's `pointerup`, a canvas scaled by CSS) and wheel steps;
//! * `touch`, `pen`, `pen_again`: touch's fingers (with SDL's mouse
//!   emulation) and the one pen (proximity, touch, eraser, buttons, axes);
//! * `focus`, `visibility`, `beforeunload` (an event watch's
//!   `SDL_EVENT_TERMINATING`), `orientation`, `drop_text`;
//! * `thrown`: a JavaScript exception from an event watch, which leaves the
//!   key handler (`"C-unwind"`) for the browser, which reports it;
//!
//! then, in the main loop (`emscripten_set_main_loop()`, as the C):
//! `drop_files`, `drop_files_read` (two files, read by `FileReader`s into
//! `/tmp/filedrop`, and their contents, read back with `std::fs`);
//! `trusted_click` (a trusted click: its pointer events, and a deferred
//! fullscreen request it doesn't run), `trusted_key` (a trusted key press:
//! SDL's own `keydown` handler runs the request), `leave_fullscreen`;
//! `two_windows`, `two_windows_input`, `two_windows_focus`,
//! `destroy_second` (upstream's handlers of one window, the canvas's
//! listeners per window: a second window takes the keyboard handlers, and
//! takes them away when it's destroyed); `quit`.
//!
//! Before each destroy the C reference removes `/tmp/filedrop` and what's
//! in it, and makes it again, empty: upstream's drag handlers' removal
//! throws (and ends the program) once the directory is gone or has files
//! in it, where it means to remove it all, as the translation does (Note
//! (upstream) in `sdl3_emscripten::video::events`). The trace is of that
//! patched behaviour.
//!
//! The steps' pointer input is theirs: the browser's own pointer and mouse
//! events (trusted ones, which the steps don't send but for the click)
//! are kept from the page, in the C's run as here. Browsers make them as
//! they please: headless Chromium 141 moves its pointer to the page's
//! corner when the canvas goes fullscreen and back, and the old headless
//! mode when the page starts; CI's Chrome 155 doesn't.
//!
//! Each step prints a libtest-like `test NAME ... ok` line (with both
//! traces when they differ); under node everything skips as `browser`.
//! `SDL3_EMSCRIPTEN_BROWSER=1 SDL3_TEST_REQUIRE=browser cargo test -p sdl3
//! --target wasm32-unknown-emscripten --test emscripten_events` runs it as
//! CI does.

#[cfg(not(target_os = "emscripten"))]
fn main() {}

#[cfg(target_os = "emscripten")]
fn main() -> std::process::ExitCode {
    events::main()
}

/// The program's request for the browser (see `emscripten_browser.rs`).
#[cfg(target_os = "emscripten")]
#[used]
#[link_section = "sdl3_runner"]
static RUNNER: [u8; 7] = *b"browser";

#[cfg(target_os = "emscripten")]
mod events {
    use std::panic::{self, AssertUnwindSafe};
    use std::process::ExitCode;
    use std::sync::Mutex;
    use std::time::Duration;

    use sdl3::events::keyboard::{self, Scancode};
    use sdl3::events::mouse;
    use sdl3::events::window::WindowFlags;
    use sdl3::events::{self, Event, EventType, WindowID};
    use sdl3::init::{self, InitFlags};
    use sdl3::properties::Properties;
    use sdl3::timer;
    use sdl3::video::window::{
        PROP_WINDOW_CREATE_EMSCRIPTEN_CANVAS_ID_STRING, PROP_WINDOW_CREATE_HEIGHT_NUMBER,
        PROP_WINDOW_CREATE_TITLE_STRING, PROP_WINDOW_CREATE_WIDTH_NUMBER,
    };
    use sdl3::video::Window;
    use sdl3_emscripten::sys;
    use sdl3_emscripten::{em_asm, em_asm_int};

    fn required(capability: &str) -> bool {
        std::env::var("SDL3_TEST_REQUIRE")
            .unwrap_or_default()
            .split(',')
            .map(str::trim)
            .any(|c| c == "all" || c == capability)
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

    /// The value of the JavaScript expression `expr` (evaluated in the
    /// glue's scope: `Module`, `JSEvents`, `FS`, …), as a string.
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

    // -- the trace -------------------------------------------------------------

    /// What the steps wrote so far (the event watch writes too).
    static TRACE: Mutex<String> = Mutex::new(String::new());

    /// The test's windows, by index (`w1`, `w2`; `w0` for none, `w9` for
    /// one that isn't the test's any more).
    static WINDOWS: Mutex<[Option<Window>; 2]> = Mutex::new([None; 2]);

    fn out(line: &str) {
        let mut t = TRACE.lock().unwrap();
        t.push_str(line);
        t.push('\n');
    }

    fn window(index: usize) -> Window {
        WINDOWS.lock().unwrap()[index].unwrap()
    }

    fn win_index(id: WindowID) -> usize {
        let windows = WINDOWS.lock().unwrap();
        match windows.iter().position(|w| w.is_some_and(|w| w.id() == id)) {
            Some(i) => i + 1,
            None if id != 0 => 9,
            None => 0,
        }
    }

    /// An event as the C reference prints it.
    fn format_event(e: &Event) -> String {
        let t = e.event_type().0;
        let b = |v: bool| v as i32;
        match e {
            Event::Window(w) => format!(
                "window 0x{t:x} {} {} w{}",
                w.data1,
                w.data2,
                win_index(w.window_id)
            ),
            Event::Display(d) => format!("display 0x{t:x} {} {}", d.data1, d.data2),
            Event::Key(k) => format!(
                "key 0x{t:x} sc={} key=0x{:x} mod=0x{:x} down={} repeat={} w{}",
                k.scancode.0,
                k.key.0,
                k.modifiers.0,
                b(k.down),
                b(k.repeat),
                win_index(k.window_id)
            ),
            Event::TextInput(x) => format!("text \"{}\" w{}", x.text, win_index(x.window_id)),
            Event::MouseMotion(m) => format!(
                "motion which={} state=0x{:x} x={:.4} y={:.4} xrel={:.4} yrel={:.4} w{}",
                m.which,
                m.state.0,
                m.x,
                m.y,
                m.xrel,
                m.yrel,
                win_index(m.window_id)
            ),
            Event::MouseButton(m) => format!(
                "button 0x{t:x} which={} button={} down={} clicks={} x={:.4} y={:.4} w{}",
                m.which,
                m.button,
                b(m.down),
                m.clicks,
                m.x,
                m.y,
                win_index(m.window_id)
            ),
            Event::MouseWheel(m) => format!(
                "wheel which={} x={:.4} y={:.4} ix={} iy={} dir={} mx={:.4} my={:.4} w{}",
                m.which,
                m.x,
                m.y,
                m.integer_x,
                m.integer_y,
                m.direction as i32,
                m.mouse_x,
                m.mouse_y,
                win_index(m.window_id)
            ),
            Event::TouchFinger(f) => format!(
                "finger 0x{t:x} touch={} finger={} x={:.4} y={:.4} dx={:.4} dy={:.4} p={:.4} w{}",
                f.touch_id,
                f.finger_id,
                f.x,
                f.y,
                f.dx,
                f.dy,
                f.pressure,
                win_index(f.window_id)
            ),
            Event::PenProximity(p) => format!("pen 0x{t:x} w{}", win_index(p.window_id)),
            Event::PenTouch(p) => format!(
                "pen 0x{t:x} state=0x{:x} x={:.4} y={:.4} eraser={} down={} w{}",
                p.pen_state.0,
                p.x,
                p.y,
                b(p.eraser),
                b(p.down),
                win_index(p.window_id)
            ),
            Event::PenMotion(p) => format!(
                "pen 0x{t:x} state=0x{:x} x={:.4} y={:.4} w{}",
                p.pen_state.0,
                p.x,
                p.y,
                win_index(p.window_id)
            ),
            Event::PenButton(p) => format!(
                "pen 0x{t:x} state=0x{:x} x={:.4} y={:.4} button={} down={} w{}",
                p.pen_state.0,
                p.x,
                p.y,
                p.button,
                b(p.down),
                win_index(p.window_id)
            ),
            Event::PenAxis(p) => format!(
                "pen 0x{t:x} state=0x{:x} x={:.4} y={:.4} axis={} value={:.4} w{}",
                p.pen_state.0,
                p.x,
                p.y,
                p.axis as i32,
                p.value,
                win_index(p.window_id)
            ),
            Event::Drop(d) => format!(
                "drop 0x{t:x} x={:.4} y={:.4} data={} w{}",
                d.x,
                d.y,
                d.data.as_deref().unwrap_or("(null)"),
                win_index(d.window_id)
            ),
            _ => format!("event 0x{t:x}"),
        }
    }

    fn drain() {
        while let Some(e) = events::poll() {
            out(&format_event(&e));
        }
    }

    /// What the step's JavaScript can do first.
    #[derive(Clone, Copy, Debug, PartialEq)]
    enum Action {
        None,
        Create1,
        StartText,
        StopText,
        Create2,
        Destroy2,
    }

    struct Step {
        name: &'static str,
        action: Action,
        code: &'static str,
    }

    fn action(a: Action) {
        match a {
            Action::None => {}
            Action::Create1 => {
                let w = Window::create("events", 320, 200, WindowFlags::NONE).unwrap();
                WINDOWS.lock().unwrap()[0] = Some(w);
            }
            Action::StartText => window(0).start_text_input().unwrap(),
            Action::StopText => window(0).stop_text_input().unwrap(),
            Action::Create2 => {
                js_run("var c = document.createElement('canvas'); c.id = 'canvas2'; c.tabIndex = -1; document.body.appendChild(c);");
                let props = Properties::new();
                props
                    .set(PROP_WINDOW_CREATE_TITLE_STRING, "second")
                    .unwrap();
                props.set(PROP_WINDOW_CREATE_WIDTH_NUMBER, 100).unwrap();
                props.set(PROP_WINDOW_CREATE_HEIGHT_NUMBER, 80).unwrap();
                props
                    .set(PROP_WINDOW_CREATE_EMSCRIPTEN_CANVAS_ID_STRING, "#canvas2")
                    .unwrap();
                let w = Window::create_with_properties(&props).unwrap();
                WINDOWS.lock().unwrap()[1] = Some(w);
            }
            Action::Destroy2 => {
                let w = window(1);
                WINDOWS.lock().unwrap()[1] = None;
                w.destroy();
            }
        }
    }

    /// A step: its action, its JavaScript (whose result is the DOM's side),
    /// then SDL's events.
    fn run(name: &str) {
        let step = STEPS.iter().find(|s| s.name == name).unwrap();
        action(step.action);
        out(&format!("== {}", step.name));
        let dom = js(&format!(
            "(function() {{ var T = Module.sdl3t; {} }})()",
            step.code
        ));
        out(&format!("dom: {dom}"));
        let focus = |w: Option<WindowID>| w.map_or(0, win_index);
        out(&format!(
            "mod=0x{:x} focus=w{} kbd=w{}",
            keyboard::mod_state().0,
            focus(mouse::mouse_focus()),
            focus(keyboard::keyboard_focus())
        ));
        drain();
    }

    /// The event watch: `SDL_EVENT_TERMINATING` (which isn't queued), and
    /// a JavaScript exception for F9.
    fn watch(e: &Event) {
        let t = e.event_type();
        if t == EventType::TERMINATING {
            out(&format!("watch 0x{:x}", t.0));
        }
        if let Event::Key(k) = e {
            if k.down && k.scancode == Scancode::F9 {
                out("watch: throwing");
                // SAFETY: plain JavaScript (which throws).
                unsafe { em_asm!("throw new Error('sdl3 test: thrown in an event watch');") };
            }
        }
    }

    // -- the main loop ---------------------------------------------------------

    /// How long the main loop waits for the browser (the files' reads, the
    /// trusted input's effects).
    const PATIENCE: Duration = Duration::from_secs(5);

    #[derive(Clone, Copy, Debug, PartialEq)]
    enum Phase {
        DropFiles,
        /// Waiting for the `FileReader`s.
        FilesRead,
        /// The trusted click sent.
        Click,
        /// The trusted key press sent: waiting for fullscreen.
        Key,
        /// Waiting for the end of fullscreen.
        Leave,
    }

    struct Loop {
        phase: Phase,
        since: Duration,
    }

    static LOOP: Mutex<Loop> = Mutex::new(Loop {
        phase: Phase::DropFiles,
        since: Duration::ZERO,
    });

    fn fullscreen() -> bool {
        window(0).flags().unwrap().contains(WindowFlags::FULLSCREEN)
    }

    fn state(label: &str) {
        out(&format!(
            "{label}: deferred={} fullscreen={} element={}",
            js("JSEvents.deferredCalls.length"),
            fullscreen() as i32,
            js("String(document.fullscreenElement && document.fullscreenElement.id)")
        ));
    }

    /// A trusted click at (60, 40) in the canvas: the runner's
    /// `Input.dispatchMouseEvent`, once the page is back in its event loop.
    fn trusted_click() {
        js_run(
            "var r = document.querySelector('#canvas').getBoundingClientRect(); var x = r.left + 60, y = r.top + 40; \
             for (var p of [{ type: 'mouseMoved', x: x, y: y }, { type: 'mousePressed', x: x, y: y, button: 'left', buttons: 1, clickCount: 1 }, { type: 'mouseReleased', x: x, y: y, button: 'left', buttons: 0, clickCount: 1 }]) \
             sdl3RunnerReport(JSON.stringify(['input', ['dispatchMouseEvent', p]]));",
        );
    }

    /// A trusted key press (`Input.dispatchKeyEvent`).
    fn trusted_key() {
        js_run(
            "for (var t of ['keyDown', 'keyUp']) sdl3RunnerReport(JSON.stringify(['input', ['dispatchKeyEvent', \
             { type: t, key: 'a', code: 'KeyA', text: t === 'keyDown' ? 'a' : undefined, windowsVirtualKeyCode: 65, nativeVirtualKeyCode: 65 }]]));",
        );
    }

    /// A dropped file's contents, read back with `std::fs` (MEMFS).
    fn dropped(dir: u32) -> String {
        let dir = format!("/tmp/filedrop/{dir}");
        let mut names: Vec<_> = std::fs::read_dir(&dir)
            .map(|d| d.map(|e| e.unwrap().file_name()).collect())
            .unwrap_or_default();
        names.sort();
        names
            .iter()
            .map(|n| std::fs::read_to_string(format!("{dir}/{}", n.to_string_lossy())).unwrap())
            .collect::<Vec<_>>()
            .join(",")
    }

    /// The main loop's step; true once it's done.
    fn step(l: &mut Loop) -> bool {
        let now = timer::ticks();
        let late = now - l.since > PATIENCE;
        let next = |l: &mut Loop, phase| {
            l.phase = phase;
            l.since = now;
        };
        match l.phase {
            Phase::DropFiles => {
                run("drop_files");
                next(l, Phase::FilesRead);
            }
            Phase::FilesRead => {
                // (the C waits 1.5 s; this waits until the second read
                // made its directory, or as long)
                if js("FS.analyzePath('/tmp/filedrop/1').exists") == "true"
                    || now - l.since > Duration::from_millis(1500)
                {
                    run("drop_files_read");
                    out(&format!("contents: [{}] [{}]", dropped(0), dropped(1)));
                    out("== trusted_click");
                    out(&format!(
                        "fullscreen request: {}",
                        window(0).set_fullscreen(true).is_ok() as i32
                    ));
                    state("before the click");
                    // (the browser's own pointer events are kept from the
                    // page, but for the click's)
                    js_run("Module.sdl3t.blocking = false;");
                    trusted_click();
                    next(l, Phase::Click);
                }
            }
            Phase::Click => {
                if now - l.since > Duration::from_millis(1000) {
                    state("after the click");
                    drain();
                    js_run("Module.sdl3t.blocking = true;");
                    out("== trusted_key");
                    trusted_key();
                    next(l, Phase::Key);
                }
            }
            Phase::Key => {
                // (fullscreen, and the key let go: the runner's keyUp comes
                // when it comes)
                if (fullscreen() && !keyboard::is_pressed(Scancode::A)) || late {
                    state("after the key");
                    drain();
                    out("== leave_fullscreen");
                    out(&format!(
                        "leave: {}",
                        window(0).set_fullscreen(false).is_ok() as i32
                    ));
                    next(l, Phase::Leave);
                }
            }
            Phase::Leave => {
                if (js("String(document.fullscreenElement)") == "null"
                    && now - l.since > Duration::from_millis(500))
                    || late
                {
                    state("left");
                    drain();
                    run("two_windows");
                    run("two_windows_input");
                    run("two_windows_focus");
                    run("destroy_second");
                    out("== quit");
                    let w = window(0);
                    WINDOWS.lock().unwrap()[0] = None;
                    w.destroy();
                    out(&format!(
                        "destroyed: {} | filedrop={} | touchAction={}",
                        js("Module.sdl3t.handlers()"),
                        js("Module.sdl3t.filedrop()"),
                        js("document.querySelector('#canvas').style.touchAction")
                    ));
                    drain();
                    init::quit();
                    println!(
                        "(the browser's own pointer events kept from the page: {})",
                        js("Module.sdl3t.blocked")
                    );
                    out(&format!(
                        "quit: [{}] mouseup={} lockkeys={}",
                        js("Module.sdl3t.handlers()"),
                        js("typeof document.sdlEventHandlerMouseButtonUpGlobal"),
                        js("typeof document.sdlEventHandlerLockKeysCheck")
                    ));
                    return true;
                }
            }
        }
        false
    }

    /// The main loop's iteration (as the C's `iterate()`).
    unsafe extern "C-unwind" fn iterate() {
        // (finish() exits, which is a JavaScript exception: not inside
        // catch_unwind(), which can't catch a foreign exception)
        let result = {
            let mut l = LOOP.lock().unwrap();
            panic::catch_unwind(AssertUnwindSafe(|| step(&mut l)))
        };
        match result {
            Ok(true) => finish(),
            Ok(false) => {}
            Err(_) => {
                println!("(the main loop failed)");
                out("== (the main loop failed)");
                finish();
            }
        }
        if timer::ticks() > PATIENCE * 12 {
            println!("the steps didn't finish in {} s", (PATIENCE * 12).as_secs());
            out("== (timed out)");
            finish();
        }
    }

    // -- the comparison --------------------------------------------------------

    /// The trace's steps: each `== NAME` line and what follows.
    fn sections(trace: &str) -> Vec<(String, String)> {
        let mut out: Vec<(String, String)> = Vec::new();
        for line in trace.lines() {
            if let Some(name) = line.strip_prefix("== ") {
                out.push((name.to_owned(), String::new()));
            } else if line == "-- main loop" {
                continue;
            } else if let Some((_, body)) = out.last_mut() {
                body.push_str(line);
                body.push('\n');
            }
        }
        out
    }

    /// [`EXPECTED`] with the page's sizes: the screen's (`{SW}`x`{SH}`),
    /// and where the window goes back when it leaves fullscreen, centered in
    /// the page (`{CX}`, `{CY}`).
    fn expected() -> String {
        let n = |e: &str| js(e).parse::<i32>().unwrap();
        let (sw, sh) = (n("screen.width"), n("screen.height"));
        let (iw, ih) = (n("window.innerWidth"), n("window.innerHeight"));
        EXPECTED
            .replace("{SW}", &sw.to_string())
            .replace("{SH}", &sh.to_string())
            .replace("{CX}", &((iw - 320) / 2).to_string())
            .replace("{CY}", &((ih - 200) / 2).to_string())
    }

    fn finish() -> ! {
        let trace = TRACE.lock().unwrap().clone();
        let expected = sections(&expected());
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
                println!("test {name} ... FAILED (not a step of the C's)\n{body}");
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
        // (the main loop keeps the runtime alive, and exit() would wait for
        // it: as the C, it ends first)
        // SAFETY: a plain call.
        unsafe { sys::emscripten_cancel_main_loop() };
        std::process::exit(if failed.is_empty() { 0 } else { 1 })
    }

    pub(super) fn main() -> ExitCode {
        if !in_browser() {
            let reason = "not in a browser (under node: run it with SDL3_EMSCRIPTEN_BROWSER=1, or with CHROME naming Chrome or Chromium)";
            if required("browser") {
                panic!("required capability browser unavailable: {reason}");
            }
            println!("note: skipping, browser unavailable: {reason}");
            for (name, _) in sections(EXPECTED) {
                println!("test {name} ... ok");
            }
            println!(
                "\ntest result: ok. {} passed; 0 failed",
                sections(EXPECTED).len()
            );
            return ExitCode::SUCCESS;
        }

        init::init(InitFlags::VIDEO).unwrap();
        js_run(PRELUDE);
        events::add_watch(watch).detach();
        for step in STEPS.iter().take_while(|s| s.name != "drop_files") {
            run(step.name);
        }
        // SAFETY: an extern "C-unwind" fn with the main loop's signature.
        unsafe { sys::emscripten_set_main_loop(Some(iterate), 0, false) };
        // (the main loop goes on)
        ExitCode::SUCCESS
    }

    // -- the scenario ----------------------------------------------------------

    /// The page's helpers (`Module.sdl3t`, `T` in the steps): key presses
    /// as a browser makes them (`keydown`, `keypress` unless prevented,
    /// `keyup`), pointer, wheel and drag events at a position in an
    /// element, html5.h's handlers by event type, `/tmp/filedrop`; and
    /// `T.blocking` (on but for the trusted click), which keeps the
    /// browser's own (trusted) pointer and mouse events from the page,
    /// counting them in `T.blocked`.
    const PRELUDE: &str = r#"var T = Module.sdl3t = {};
T.target = function(sel) { return sel === 'document' ? document : sel === 'window' ? window : document.querySelector(sel); };
T.at = function(sel, o) { var t = T.target(sel); var r = t.getBoundingClientRect ? t.getBoundingClientRect() : { left: 0, top: 0 }; return [r.left + ((o && o.x) || 0), r.top + ((o && o.y) || 0)]; };
T.done = function(ev, ok) { return ev.type + (ok ? '' : ' (prevented)'); };
T.key = function(type, key, code, o) {
  var e = new KeyboardEvent(type, Object.assign({ key: key, code: code, bubbles: true, cancelable: true }, o || {}));
  return T.done(e, document.body.dispatchEvent(e)) + ' ' + code + ':' + key;
};
T.press = function(key, code, o) {
  var r = [T.key('keydown', key, code, o)];
  if (r[0].indexOf('(prevented)') < 0 && [...key].length == 1 && !(o && o.ctrlKey)) { r.push(T.key('keypress', key, code, o)); }
  r.push(T.key('keyup', key, code, o));
  return r.join(', ');
};
T.ptr = function(sel, type, o) {
  var at = T.at(sel, o);
  var init = Object.assign({ bubbles: type != 'pointerenter' && type != 'pointerleave', cancelable: true, pointerId: 1, pointerType: 'mouse', isPrimary: true, button: -1, buttons: 0 }, o || {});
  init.clientX = at[0]; init.clientY = at[1];
  var e = new PointerEvent(type, init);
  return T.done(e, T.target(sel).dispatchEvent(e));
};
T.wheel = function(sel, o) {
  var at = T.at(sel, o);
  var init = Object.assign({ bubbles: true, cancelable: true }, o || {});
  init.clientX = at[0]; init.clientY = at[1];
  var e = new WheelEvent('wheel', init);
  return T.done(e, T.target(sel).dispatchEvent(e));
};
T.drag = function(sel, type, dt, o) {
  var at = T.at(sel, o);
  var e = new DragEvent(type, { bubbles: true, cancelable: true, dataTransfer: dt, clientX: at[0], clientY: at[1] });
  return T.done(e, T.target(sel).dispatchEvent(e));
};
T.fire = function(sel, e) { return T.done(e, T.target(sel).dispatchEvent(e)); };
T.all = function(list) { return list.join(', '); };
T.handlers = function() { return JSEvents.eventHandlers.map(function(h) { return h.eventTypeString; }).join(' '); };
T.filedrop = function() { return FS.analyzePath('/tmp/filedrop').exists; };
T.blocking = true;
T.blocked = 0;
for (var t of ['pointerover', 'pointerenter', 'pointermove', 'pointerdown', 'pointerup', 'pointercancel', 'pointerout', 'pointerleave', 'mouseover', 'mouseenter', 'mousemove', 'mousedown', 'mouseup', 'mouseout', 'mouseleave']) {
  window.addEventListener(t, function(e) { if (T.blocking && e.isTrusted) { T.blocked++; e.stopImmediatePropagation(); } }, true);
}
"#;

    /// The steps (the C reference's, the same text).
    const STEPS: &[Step] = &[
        Step {
            name: "created",
            action: Action::Create1,
            code: r#"return T.handlers() + ' | touchAction=' + document.querySelector('#canvas').style.touchAction + ' filedrop=' + T.filedrop();"#,
        },
        Step {
            name: "keys",
            action: Action::None,
            code: r#"return T.all([T.press('a', 'KeyA'), T.press('A', 'KeyA', { shiftKey: true }), T.press('q', 'KeyA'), T.press('1', 'Digit1'), T.press('1', 'Numpad1', { location: 3 }), T.press('é', 'Quote'), T.press('😀', ''), T.press('Enter', 'Enter'), T.press('Tab', 'Tab'), T.press('ArrowLeft', 'ArrowLeft'), T.press('F5', 'F5'), T.press('F13', 'F13'), T.press('Sleep', ''), T.press('MediaPlay', 'Unidentified'), T.press('SoftRight', ''), T.press('Unidentified', 'Fn'), T.press('a', 'KeyA', { ctrlKey: true }), T.press('/', 'NumpadDivide', { location: 3 }), T.press('Meta', 'OSLeft', { location: 1 })]);"#,
        },
        Step {
            name: "modifiers",
            action: Action::None,
            code: r#"return T.all([T.key('keydown', 'Shift', 'ShiftLeft', { location: 1, shiftKey: true }), T.press('B', 'KeyB', { shiftKey: true }), T.key('keyup', 'Shift', 'ShiftLeft', { location: 1 }), T.key('keydown', 'Control', 'ControlRight', { location: 2, ctrlKey: true }), T.key('keyup', 'Control', 'ControlRight', { location: 2 }), T.key('keydown', 'r', 'KeyR'), T.key('keydown', 'r', 'KeyR', { repeat: true }), T.key('keyup', 'r', 'KeyR')]);"#,
        },
        Step {
            name: "text_input",
            action: Action::StartText,
            code: r#"return T.all([T.press('b', 'KeyB'), T.press('€', 'KeyE', { altKey: true }), T.press('Backspace', 'Backspace'), T.press('ArrowUp', 'ArrowUp'), T.press('F2', 'F2'), T.press('v', 'KeyV', { ctrlKey: true }), T.press('Shift', 'ShiftLeft', { location: 1 })]);"#,
        },
        Step {
            name: "text_input_off",
            action: Action::StopText,
            code: r#"return T.all([T.press('c', 'KeyC'), T.key('keypress', 'c', 'KeyC')]);"#,
        },
        Step {
            name: "lock_keys",
            action: Action::None,
            code: r#"return T.all([T.press('x', 'KeyX', { modifierCapsLock: true }), T.press('y', 'KeyY', { modifierNumLock: true, modifierScrollLock: true }), T.press('CapsLock', 'CapsLock'), T.press('z', 'KeyZ', { modifierCapsLock: true, modifierNumLock: true, modifierScrollLock: true }), T.press('NumLock', 'NumLock'), T.press('w', 'KeyW')]);"#,
        },
        Step {
            name: "mouse",
            action: Action::None,
            code: r#"var c = document.querySelector('#canvas'); c.style.marginLeft = '10px'; c.style.marginTop = '5px';
return T.all([T.ptr('#canvas', 'pointerleave', { x: -1, y: 3 }), T.ptr('#canvas', 'pointerenter', { x: 4, y: 6 }), T.ptr('#canvas', 'pointermove', { x: 50, y: 60, movementX: 3, movementY: -2 }), T.ptr('#canvas', 'pointerdown', { x: 50, y: 60, button: 0, buttons: 1 }), T.ptr('#canvas', 'pointermove', { x: 51.5, y: 61.25, buttons: 1 }), T.ptr('#canvas', 'pointerdown', { x: 52, y: 62, button: 2, buttons: 3 }), T.ptr('#canvas', 'pointerup', { x: 52, y: 62, button: 0, buttons: 2 }), T.ptr('#canvas', 'pointerup', { x: 52, y: 62, button: 2, buttons: 0 }), T.ptr('#canvas', 'pointerdown', { x: 1, y: 2, button: 1, buttons: 4 }), T.ptr('#canvas', 'pointerup', { x: 1, y: 2, button: 1 }), T.ptr('#canvas', 'pointerdown', { x: 1, y: 2, button: 3, buttons: 8 }), T.ptr('#canvas', 'pointerup', { x: 1, y: 2, button: 3 }), T.ptr('#canvas', 'pointerdown', { x: 1, y: 2, button: 4, buttons: 16 }), T.ptr('#canvas', 'pointerup', { x: 1, y: 2, button: 4 }), T.ptr('#canvas', 'pointerdown', { x: 1, y: 2, button: 7, buttons: 0 }), T.ptr('#canvas', 'pointermove', { x: 2, y: 2, pointerType: 'other' })]);"#,
        },
        Step {
            name: "capture",
            action: Action::None,
            code: r#"return T.all([T.ptr('#canvas', 'pointerdown', { x: 20, y: 20, button: 0, buttons: 1 }), T.ptr('#canvas', 'pointerleave', { x: -5, y: 20, buttons: 1 }), T.ptr('document', 'pointerup', { x: 3, y: 20, button: 0 }), T.ptr('#canvas', 'pointerenter', { x: 5, y: 20 }), T.ptr('#canvas', 'pointerdown', { x: 20, y: 20, button: 0, buttons: 1 }), T.ptr('#canvas', 'pointerleave', { x: -5, y: 20, buttons: 1 }), T.ptr('#canvas', 'pointerenter', { x: 5, y: 21, buttons: 1 }), T.ptr('#canvas', 'pointerup', { x: 6, y: 21, button: 0 })]);"#,
        },
        Step {
            name: "scaled",
            action: Action::None,
            code: r#"var c = document.querySelector('#canvas'); c.style.width = '640px'; c.style.height = '400px';
var r = T.all([T.ptr('#canvas', 'pointermove', { x: 100, y: 50 }), T.ptr('#canvas', 'pointerdown', { x: 101, y: 51, button: 0, buttons: 1 }), T.ptr('#canvas', 'pointerup', { x: 101, y: 51, button: 0 }), T.ptr('#canvas', 'pointerleave', { x: 641, y: 51 }), T.ptr('#canvas', 'pointerenter', { x: 639, y: 399 })]);
c.style.width = ''; c.style.height = ''; return r;"#,
        },
        Step {
            name: "wheel",
            action: Action::None,
            code: r#"return T.all([T.wheel('#canvas', { x: 7, y: 8, deltaY: 100 }), T.wheel('#canvas', { x: 7, y: 8, deltaY: -50, deltaX: 25 }), T.wheel('#canvas', { x: 7, y: 8, deltaY: 3, deltaMode: 1 }), T.wheel('#canvas', { x: 9, y: 8, deltaX: -1, deltaMode: 2 }), T.wheel('#canvas', { x: 9, y: 8, deltaY: 0 })]);"#,
        },
        Step {
            name: "touch",
            action: Action::None,
            code: r#"return T.all([T.ptr('#canvas', 'pointerenter', { x: 32, y: 20, pointerType: 'touch', pointerId: 7 }), T.ptr('#canvas', 'pointerdown', { x: 32, y: 20, pointerType: 'touch', pointerId: 7, button: 0, buttons: 1 }), T.ptr('#canvas', 'pointermove', { x: 64, y: 40, pointerType: 'touch', pointerId: 7, buttons: 1 }), T.ptr('#canvas', 'pointerup', { x: 64, y: 40, pointerType: 'touch', pointerId: 7, button: 0 }), T.ptr('#canvas', 'pointerleave', { x: 64, y: 40, pointerType: 'touch', pointerId: 7 }), T.ptr('#canvas', 'pointerdown', { x: 319, y: 199, pointerType: 'touch', pointerId: 8, button: 0, buttons: 1 }), T.ptr('#canvas', 'pointercancel', { x: 300, y: 100, pointerType: 'touch', pointerId: 8 })]);"#,
        },
        Step {
            name: "pen",
            action: Action::None,
            code: r#"var p = { pointerType: 'pen', pointerId: 9 };
return T.all([T.ptr('#canvas', 'pointerenter', Object.assign({ x: 10, y: 10 }, p)), T.ptr('#canvas', 'pointerdown', Object.assign({ x: 11, y: 12, button: 0, buttons: 1, pressure: 0.5, tangentialPressure: 0.25, tiltX: 30, tiltY: -15, twist: 90 }, p)), T.ptr('#canvas', 'pointermove', Object.assign({ x: 12, y: 13, buttons: 1, pressure: 0.75, tiltX: 31, twist: 91 }, p)), T.ptr('#canvas', 'pointerdown', Object.assign({ x: 12, y: 13, button: 2, buttons: 3, pressure: 0.75 }, p)), T.ptr('#canvas', 'pointerup', Object.assign({ x: 12, y: 13, button: 2, buttons: 1, pressure: 0.75 }, p)), T.ptr('#canvas', 'pointerdown', Object.assign({ x: 12, y: 13, button: 1, buttons: 5, pressure: 0.75 }, p)), T.ptr('#canvas', 'pointerup', Object.assign({ x: 12, y: 13, button: 1, buttons: 1, pressure: 0.75 }, p)), T.ptr('#canvas', 'pointerdown', Object.assign({ x: 12, y: 13, button: 5, buttons: 33, pressure: 0.75 }, p)), T.ptr('#canvas', 'pointerup', Object.assign({ x: 12, y: 13, button: 5, buttons: 1, pressure: 0.75 }, p)), T.ptr('#canvas', 'pointerup', Object.assign({ x: 14, y: 15, button: 0, buttons: 0 }, p)), T.ptr('#canvas', 'pointerleave', Object.assign({ x: 14, y: 15 }, p))]);"#,
        },
        Step {
            name: "pen_again",
            action: Action::None,
            code: r#"var p = { pointerType: 'pen', pointerId: 10 };
return T.all([T.ptr('#canvas', 'pointerenter', Object.assign({ x: 20, y: 20, pressure: 0.1 }, p)), T.ptr('#canvas', 'pointerleave', Object.assign({ x: 21, y: 20 }, p))]);"#,
        },
        Step {
            name: "focus",
            action: Action::None,
            code: r#"document.hasFocus = () => false;
var r = [T.key('keydown', 'k', 'KeyK'), T.fire('window', new FocusEvent('blur')), T.fire('window', new FocusEvent('focus'))];
document.hasFocus = () => true; r.push(T.fire('window', new FocusEvent('focus')));
delete document.hasFocus; return T.all(r);"#,
        },
        Step {
            name: "visibility",
            action: Action::None,
            code: r#"Object.defineProperty(document, 'hidden', { configurable: true, get: () => true });
var r = [T.fire('document', new Event('visibilitychange'))];
delete document.hidden; r.push(T.fire('document', new Event('visibilitychange'))); return T.all(r) + ' hidden=' + document.hidden;"#,
        },
        Step {
            name: "beforeunload",
            action: Action::None,
            code: r#"return T.fire('window', new Event('beforeunload', { cancelable: true }));"#,
        },
        Step {
            name: "orientation",
            action: Action::None,
            code: r#"var ok = screen.orientation.dispatchEvent(new Event('change')); return 'change ' + screen.orientation.type + ' ' + screen.orientation.angle + (ok ? '' : ' (prevented)');"#,
        },
        Step {
            name: "drop_text",
            action: Action::None,
            code: r#"var dt = new DataTransfer(); dt.setData('text/plain', 'dropped text');
return T.all([T.drag('#canvas', 'dragover', dt, { x: 30, y: 40 }), T.drag('#canvas', 'dragover', dt, { x: 31.7, y: 40 }), T.drag('#canvas', 'drop', dt, { x: 31, y: 41 }), T.drag('#canvas', 'dragover', dt, { x: 2, y: 3 }), T.drag('#canvas', 'dragleave', dt, { x: -1, y: 3 }), T.drag('#canvas', 'dragover', dt, { x: 5, y: 6 }), T.drag('#canvas', 'dragend', dt, { x: 5, y: 6 })]);"#,
        },
        Step {
            name: "thrown",
            action: Action::None,
            code: r#"Module.sdl3tErrors = []; Module.sdl3tOnError = (e) => { Module.sdl3tErrors.push(String(e.message)); e.preventDefault(); };
window.addEventListener('error', Module.sdl3tOnError);
var r = T.all([T.press('F9', 'F9'), T.press('F8', 'F8')]);
window.removeEventListener('error', Module.sdl3tOnError); return r + ' errors=' + JSON.stringify(Module.sdl3tErrors);"#,
        },
        Step {
            name: "drop_files",
            action: Action::None,
            code: r#"var dt = new DataTransfer(); dt.items.add(new File(['the first file'], 'a.txt')); dt.items.add(new File(['the second'], 'b b.txt'));
return T.all([T.drag('#canvas', 'dragover', dt, { x: 8, y: 9 }), T.drag('#canvas', 'drop', dt, { x: 8, y: 9 })]);"#,
        },
        Step {
            name: "drop_files_read",
            action: Action::None,
            code: r#"return 'files: ' + FS.readdir('/tmp/filedrop').join(' ');"#,
        },
        Step {
            name: "two_windows",
            action: Action::Create2,
            code: r#"return T.handlers();"#,
        },
        Step {
            name: "two_windows_input",
            action: Action::None,
            code: r#"return T.all([T.press('a', 'KeyA'), T.ptr('#canvas2', 'pointerenter', { x: 1, y: 1 }), T.ptr('#canvas2', 'pointermove', { x: 5, y: 6 }), T.wheel('#canvas2', { x: 5, y: 6, deltaY: 100 }), T.ptr('#canvas', 'pointerenter', { x: 5, y: 6 }), T.ptr('#canvas2', 'pointerleave', { x: -1, y: 6 }), T.ptr('#canvas', 'pointerdown', { x: 5, y: 6, button: 0, buttons: 1 }), T.ptr('#canvas', 'pointerup', { x: 5, y: 6, button: 0 })]);"#,
        },
        Step {
            name: "two_windows_focus",
            action: Action::None,
            code: r#"var r = [];
document.querySelector('#canvas2').focus(); r.push(T.fire('window', new FocusEvent('focus')));
document.querySelector('#canvas').focus(); r.push(T.fire('window', new FocusEvent('focus')));
document.activeElement.blur(); return T.all(r);"#,
        },
        Step {
            name: "destroy_second",
            action: Action::Destroy2,
            code: r#"return T.all([T.press('a', 'KeyA'), T.ptr('#canvas', 'pointermove', { x: 7, y: 8 })]) + ' | ' + T.handlers() + ' | filedrop=' + T.filedrop();"#,
        },
    ];

    /// Upstream's trace (see the module documentation).
    const EXPECTED: &str = r#"== created
dom: focus blur pointerlockchange webkitfullscreenchange fullscreenchange resize wheel change keydown keyup keypress visibilitychange beforeunload | touchAction=none filedrop=true
mod=0x0 focus=w1 kbd=w1
event 0x305
event 0x404
window 0x20c 0 0 w1
window 0x20e 0 0 w1
window 0x202 0 0 w1
window 0x207 320 200 w1
window 0x215 0 0 w1
== keys
dom: keydown (prevented) KeyA:a, keyup (prevented) KeyA:a, keydown (prevented) KeyA:A, keyup (prevented) KeyA:A, keydown (prevented) KeyA:q, keyup (prevented) KeyA:q, keydown (prevented) Digit1:1, keyup (prevented) Digit1:1, keydown (prevented) Numpad1:1, keyup (prevented) Numpad1:1, keydown (prevented) Quote:é, keyup (prevented) Quote:é, keydown :😀, keypress :😀, keyup :😀, keydown (prevented) Enter:Enter, keyup (prevented) Enter:Enter, keydown (prevented) Tab:Tab, keyup (prevented) Tab:Tab, keydown (prevented) ArrowLeft:ArrowLeft, keyup (prevented) ArrowLeft:ArrowLeft, keydown (prevented) F5:F5, keyup (prevented) F5:F5, keydown (prevented) F13:F13, keyup (prevented) F13:F13, keydown (prevented) :Sleep, keyup (prevented) :Sleep, keydown (prevented) Unidentified:MediaPlay, keyup (prevented) Unidentified:MediaPlay, keydown (prevented) :SoftRight, keyup (prevented) :SoftRight, keydown Fn:Unidentified, keyup Fn:Unidentified, keydown (prevented) KeyA:a, keyup (prevented) KeyA:a, keydown (prevented) NumpadDivide:/, keyup (prevented) NumpadDivide:/, keydown (prevented) OSLeft:Meta, keyup (prevented) OSLeft:Meta
mod=0x0 focus=w1 kbd=w1
key 0x300 sc=4 key=0x61 mod=0x0 down=1 repeat=0 w1
key 0x301 sc=4 key=0x61 mod=0x0 down=0 repeat=0 w1
key 0x300 sc=4 key=0x61 mod=0x0 down=1 repeat=0 w1
key 0x301 sc=4 key=0x61 mod=0x0 down=0 repeat=0 w1
key 0x300 sc=4 key=0x61 mod=0x0 down=1 repeat=0 w1
key 0x301 sc=4 key=0x61 mod=0x0 down=0 repeat=0 w1
key 0x300 sc=30 key=0x31 mod=0x0 down=1 repeat=0 w1
key 0x301 sc=30 key=0x31 mod=0x0 down=0 repeat=0 w1
key 0x300 sc=89 key=0x40000059 mod=0x0 down=1 repeat=0 w1
key 0x301 sc=89 key=0x40000059 mod=0x0 down=0 repeat=0 w1
key 0x300 sc=52 key=0x27 mod=0x0 down=1 repeat=0 w1
key 0x301 sc=52 key=0x27 mod=0x0 down=0 repeat=0 w1
key 0x300 sc=40 key=0xd mod=0x0 down=1 repeat=0 w1
key 0x301 sc=40 key=0xd mod=0x0 down=0 repeat=0 w1
key 0x300 sc=43 key=0x9 mod=0x0 down=1 repeat=0 w1
key 0x301 sc=43 key=0x9 mod=0x0 down=0 repeat=0 w1
key 0x300 sc=80 key=0x40000050 mod=0x0 down=1 repeat=0 w1
key 0x301 sc=80 key=0x40000050 mod=0x0 down=0 repeat=0 w1
key 0x300 sc=62 key=0x4000003e mod=0x0 down=1 repeat=0 w1
key 0x301 sc=62 key=0x4000003e mod=0x0 down=0 repeat=0 w1
key 0x300 sc=104 key=0x40000068 mod=0x0 down=1 repeat=0 w1
key 0x301 sc=104 key=0x40000068 mod=0x0 down=0 repeat=0 w1
key 0x300 sc=258 key=0x40000102 mod=0x0 down=1 repeat=0 w1
key 0x301 sc=258 key=0x40000102 mod=0x0 down=0 repeat=0 w1
key 0x300 sc=262 key=0x40000106 mod=0x0 down=1 repeat=0 w1
key 0x301 sc=262 key=0x40000106 mod=0x0 down=0 repeat=0 w1
key 0x300 sc=282 key=0x4000011a mod=0x0 down=1 repeat=0 w1
key 0x301 sc=282 key=0x4000011a mod=0x0 down=0 repeat=0 w1
key 0x300 sc=4 key=0x61 mod=0x0 down=1 repeat=0 w1
key 0x301 sc=4 key=0x61 mod=0x0 down=0 repeat=0 w1
key 0x300 sc=84 key=0x40000054 mod=0x0 down=1 repeat=0 w1
key 0x301 sc=84 key=0x40000054 mod=0x0 down=0 repeat=0 w1
key 0x300 sc=227 key=0x400000e3 mod=0x400 down=1 repeat=0 w1
key 0x301 sc=227 key=0x400000e3 mod=0x0 down=0 repeat=0 w1
== modifiers
dom: keydown (prevented) ShiftLeft:Shift, keydown (prevented) KeyB:B, keyup (prevented) KeyB:B, keyup (prevented) ShiftLeft:Shift, keydown (prevented) ControlRight:Control, keyup (prevented) ControlRight:Control, keydown (prevented) KeyR:r, keydown (prevented) KeyR:r, keyup (prevented) KeyR:r
mod=0x0 focus=w1 kbd=w1
key 0x300 sc=225 key=0x400000e1 mod=0x1 down=1 repeat=0 w1
key 0x300 sc=5 key=0x62 mod=0x1 down=1 repeat=0 w1
key 0x301 sc=5 key=0x62 mod=0x1 down=0 repeat=0 w1
key 0x301 sc=225 key=0x400000e1 mod=0x0 down=0 repeat=0 w1
key 0x300 sc=228 key=0x400000e4 mod=0x80 down=1 repeat=0 w1
key 0x301 sc=228 key=0x400000e4 mod=0x0 down=0 repeat=0 w1
key 0x300 sc=21 key=0x72 mod=0x0 down=1 repeat=0 w1
key 0x300 sc=21 key=0x72 mod=0x0 down=1 repeat=1 w1
key 0x301 sc=21 key=0x72 mod=0x0 down=0 repeat=0 w1
== text_input
dom: keydown KeyB:b, keypress (prevented) KeyB:b, keyup (prevented) KeyB:b, keydown KeyE:€, keypress (prevented) KeyE:€, keyup (prevented) KeyE:€, keydown (prevented) Backspace:Backspace, keyup (prevented) Backspace:Backspace, keydown (prevented) ArrowUp:ArrowUp, keyup (prevented) ArrowUp:ArrowUp, keydown (prevented) F2:F2, keyup (prevented) F2:F2, keydown (prevented) KeyV:v, keyup (prevented) KeyV:v, keydown ShiftLeft:Shift, keyup (prevented) ShiftLeft:Shift
mod=0x0 focus=w1 kbd=w1
key 0x300 sc=5 key=0x62 mod=0x0 down=1 repeat=0 w1
text "b" w1
key 0x301 sc=5 key=0x62 mod=0x0 down=0 repeat=0 w1
key 0x300 sc=8 key=0x65 mod=0x0 down=1 repeat=0 w1
text "€" w1
key 0x301 sc=8 key=0x65 mod=0x0 down=0 repeat=0 w1
key 0x300 sc=42 key=0x8 mod=0x0 down=1 repeat=0 w1
key 0x301 sc=42 key=0x8 mod=0x0 down=0 repeat=0 w1
key 0x300 sc=82 key=0x40000052 mod=0x0 down=1 repeat=0 w1
key 0x301 sc=82 key=0x40000052 mod=0x0 down=0 repeat=0 w1
key 0x300 sc=59 key=0x4000003b mod=0x0 down=1 repeat=0 w1
key 0x301 sc=59 key=0x4000003b mod=0x0 down=0 repeat=0 w1
key 0x300 sc=25 key=0x76 mod=0x0 down=1 repeat=0 w1
key 0x301 sc=25 key=0x76 mod=0x0 down=0 repeat=0 w1
key 0x300 sc=225 key=0x400000e1 mod=0x1 down=1 repeat=0 w1
key 0x301 sc=225 key=0x400000e1 mod=0x0 down=0 repeat=0 w1
== text_input_off
dom: keydown (prevented) KeyC:c, keyup (prevented) KeyC:c, keypress KeyC:c
mod=0x0 focus=w1 kbd=w1
key 0x300 sc=6 key=0x63 mod=0x0 down=1 repeat=0 w1
key 0x301 sc=6 key=0x63 mod=0x0 down=0 repeat=0 w1
== lock_keys
dom: keydown (prevented) KeyX:x, keyup (prevented) KeyX:x, keydown (prevented) KeyY:y, keyup (prevented) KeyY:y, keydown (prevented) CapsLock:CapsLock, keyup (prevented) CapsLock:CapsLock, keydown (prevented) KeyZ:z, keyup (prevented) KeyZ:z, keydown (prevented) NumLock:NumLock, keyup (prevented) NumLock:NumLock, keydown (prevented) KeyW:w, keyup (prevented) KeyW:w
mod=0x0 focus=w1 kbd=w1
key 0x300 sc=27 key=0x78 mod=0x2000 down=1 repeat=0 w1
key 0x301 sc=27 key=0x78 mod=0x2000 down=0 repeat=0 w1
key 0x300 sc=28 key=0x79 mod=0x9000 down=1 repeat=0 w1
key 0x301 sc=28 key=0x79 mod=0x9000 down=0 repeat=0 w1
key 0x300 sc=57 key=0x40000039 mod=0xb000 down=1 repeat=0 w1
key 0x301 sc=57 key=0x40000039 mod=0xb000 down=0 repeat=0 w1
key 0x300 sc=29 key=0x7a mod=0xb000 down=1 repeat=0 w1
key 0x301 sc=29 key=0x7a mod=0xb000 down=0 repeat=0 w1
key 0x300 sc=83 key=0x40000053 mod=0xa000 down=1 repeat=0 w1
key 0x301 sc=83 key=0x40000053 mod=0xa000 down=0 repeat=0 w1
key 0x300 sc=26 key=0x77 mod=0x0 down=1 repeat=0 w1
key 0x301 sc=26 key=0x77 mod=0x0 down=0 repeat=0 w1
== mouse
dom: pointerleave, pointerenter, pointermove, pointerdown, pointermove, pointerdown, pointerup, pointerup, pointerdown, pointerup, pointerdown, pointerup, pointerdown, pointerup, pointerdown, pointermove
mod=0x0 focus=w1 kbd=w1
motion which=0 state=0x0 x=0.0000 y=3.0000 xrel=0.0000 yrel=0.0000 w1
window 0x20d 0 0 w1
window 0x20c 0 0 w1
motion which=0 state=0x0 x=4.0000 y=6.0000 xrel=0.0000 yrel=0.0000 w1
motion which=0 state=0x0 x=50.0000 y=60.0000 xrel=46.0000 yrel=54.0000 w1
button 0x401 which=0 button=1 down=1 clicks=1 x=50.0000 y=60.0000 w1
motion which=0 state=0x1 x=51.5000 y=61.2500 xrel=1.5000 yrel=1.2500 w1
motion which=0 state=0x1 x=52.0000 y=62.0000 xrel=0.5000 yrel=0.7500 w1
button 0x401 which=0 button=3 down=1 clicks=1 x=52.0000 y=62.0000 w1
button 0x402 which=0 button=1 down=0 clicks=1 x=52.0000 y=62.0000 w1
button 0x402 which=0 button=3 down=0 clicks=1 x=52.0000 y=62.0000 w1
motion which=0 state=0x0 x=1.0000 y=2.0000 xrel=-51.0000 yrel=-60.0000 w1
button 0x401 which=0 button=2 down=1 clicks=1 x=1.0000 y=2.0000 w1
button 0x402 which=0 button=2 down=0 clicks=1 x=1.0000 y=2.0000 w1
button 0x401 which=0 button=4 down=1 clicks=1 x=1.0000 y=2.0000 w1
button 0x402 which=0 button=4 down=0 clicks=1 x=1.0000 y=2.0000 w1
button 0x401 which=0 button=5 down=1 clicks=1 x=1.0000 y=2.0000 w1
button 0x402 which=0 button=5 down=0 clicks=1 x=1.0000 y=2.0000 w1
== capture
dom: pointerdown, pointerleave, pointerup, pointerenter, pointerdown, pointerleave, pointerenter, pointerup
mod=0x0 focus=w1 kbd=w1
motion which=0 state=0x0 x=20.0000 y=20.0000 xrel=19.0000 yrel=18.0000 w1
button 0x401 which=0 button=1 down=1 clicks=1 x=20.0000 y=20.0000 w1
motion which=0 state=0x1 x=-5.0000 y=20.0000 xrel=-25.0000 yrel=0.0000 w1
button 0x402 which=0 button=1 down=0 clicks=1 x=-5.0000 y=20.0000 w1
window 0x20d 0 0 w1
window 0x20c 0 0 w1
motion which=0 state=0x0 x=5.0000 y=20.0000 xrel=0.0000 yrel=0.0000 w1
motion which=0 state=0x0 x=20.0000 y=20.0000 xrel=15.0000 yrel=0.0000 w1
button 0x401 which=0 button=1 down=1 clicks=2 x=20.0000 y=20.0000 w1
motion which=0 state=0x1 x=-5.0000 y=20.0000 xrel=-25.0000 yrel=0.0000 w1
motion which=0 state=0x1 x=5.0000 y=21.0000 xrel=10.0000 yrel=1.0000 w1
motion which=0 state=0x1 x=6.0000 y=21.0000 xrel=1.0000 yrel=0.0000 w1
button 0x402 which=0 button=1 down=0 clicks=2 x=6.0000 y=21.0000 w1
== scaled
dom: pointermove, pointerdown, pointerup, pointerleave, pointerenter
mod=0x0 focus=w1 kbd=w1
motion which=0 state=0x0 x=50.0000 y=25.0000 xrel=44.0000 yrel=4.0000 w1
motion which=0 state=0x0 x=50.5000 y=25.5000 xrel=0.5000 yrel=0.5000 w1
button 0x401 which=0 button=1 down=1 clicks=3 x=50.5000 y=25.5000 w1
button 0x402 which=0 button=1 down=0 clicks=3 x=50.5000 y=25.5000 w1
motion which=0 state=0x0 x=319.0000 y=25.5000 xrel=268.5000 yrel=0.0000 w1
window 0x20d 0 0 w1
window 0x20c 0 0 w1
motion which=0 state=0x0 x=319.5000 y=199.5000 xrel=0.0000 yrel=0.0000 w1
== wheel
dom: wheel (prevented), wheel (prevented), wheel (prevented), wheel (prevented), wheel (prevented)
mod=0x0 focus=w1 kbd=w1
wheel which=0 x=0.0000 y=-1.0000 ix=0 iy=-1 dir=0 mx=319.5000 my=199.5000 w1
wheel which=0 x=0.2500 y=0.5000 ix=0 iy=0 dir=0 mx=319.5000 my=199.5000 w1
wheel which=0 x=0.0000 y=-1.0000 ix=0 iy=0 dir=0 mx=319.5000 my=199.5000 w1
wheel which=0 x=-80.0000 y=-0.0000 ix=-79 iy=0 dir=0 mx=319.5000 my=199.5000 w1
== touch
dom: pointerenter, pointerdown, pointermove, pointerup, pointerleave, pointerdown, pointercancel
mod=0x0 focus=w1 kbd=w1
motion which=4294967295 state=0x0 x=32.1003 y=20.1005 xrel=0.0000 yrel=0.0000 w1
button 0x401 which=4294967295 button=1 down=1 clicks=1 x=32.1003 y=20.1005 w1
finger 0x700 touch=1 finger=8 x=0.1003 y=0.1005 dx=0.0000 dy=0.0000 p=1.0000 w1
motion which=4294967295 state=0x1 x=64.2006 y=40.2010 xrel=32.1003 yrel=20.1005 w1
finger 0x702 touch=1 finger=8 x=0.2006 y=0.2010 dx=0.1003 dy=0.1005 p=1.0000 w1
button 0x402 which=4294967295 button=1 down=0 clicks=1 x=64.2006 y=40.2010 w1
finger 0x701 touch=1 finger=8 x=0.2006 y=0.2010 dx=0.0000 dy=0.0000 p=1.0000 w1
motion which=4294967295 state=0x0 x=319.0000 y=199.0000 xrel=0.0000 yrel=0.0000 w1
button 0x401 which=4294967295 button=1 down=1 clicks=1 x=319.0000 y=199.0000 w1
finger 0x700 touch=1 finger=9 x=1.0000 y=1.0000 dx=0.0000 dy=0.0000 p=1.0000 w1
button 0x402 which=4294967295 button=1 down=0 clicks=1 x=319.0000 y=199.0000 w1
finger 0x703 touch=1 finger=9 x=1.0000 y=1.0000 dx=0.0000 dy=0.0000 p=1.0000 w1
== pen
dom: pointerenter, pointerdown, pointermove, pointerdown, pointerup, pointerdown, pointerup, pointerdown, pointerup, pointerup, pointerleave
mod=0x0 focus=w1 kbd=w1
pen 0x1300 w1
pen 0x1306 state=0x80000000 x=10.0000 y=10.0000 w1
motion which=4294967294 state=0x0 x=10.0000 y=10.0000 xrel=-309.0000 yrel=-189.0000 w1
pen 0x1306 state=0x80000000 x=11.0000 y=12.0000 w1
motion which=4294967294 state=0x0 x=11.0000 y=12.0000 xrel=1.0000 yrel=2.0000 w1
pen 0x1302 state=0x80000001 x=11.0000 y=12.0000 eraser=0 down=1 w1
button 0x401 which=4294967294 button=1 down=1 clicks=1 x=11.0000 y=12.0000 w1
finger 0x700 touch=18446744073709551614 finger=1 x=0.0344 y=0.0600 dx=0.0000 dy=0.0000 p=0.0000 w1
pen 0x1307 state=0x80000001 x=11.0000 y=12.0000 axis=0 value=0.5000 w1
finger 0x702 touch=18446744073709551614 finger=1 x=0.0344 y=0.0600 dx=0.0000 dy=0.0000 p=0.5000 w1
pen 0x1307 state=0x80000001 x=11.0000 y=12.0000 axis=6 value=0.2500 w1
pen 0x1307 state=0x80000001 x=11.0000 y=12.0000 axis=1 value=30.0000 w1
pen 0x1307 state=0x80000001 x=11.0000 y=12.0000 axis=2 value=-15.0000 w1
pen 0x1307 state=0x80000001 x=11.0000 y=12.0000 axis=4 value=90.0000 w1
pen 0x1306 state=0x80000001 x=12.0000 y=13.0000 w1
motion which=4294967294 state=0x1 x=12.0000 y=13.0000 xrel=1.0000 yrel=1.0000 w1
finger 0x702 touch=18446744073709551614 finger=1 x=0.0375 y=0.0650 dx=0.0031 dy=0.0050 p=0.5000 w1
pen 0x1307 state=0x80000001 x=12.0000 y=13.0000 axis=0 value=0.7500 w1
finger 0x702 touch=18446744073709551614 finger=1 x=0.0375 y=0.0650 dx=0.0000 dy=0.0000 p=0.7500 w1
pen 0x1307 state=0x80000001 x=12.0000 y=13.0000 axis=6 value=0.0000 w1
pen 0x1307 state=0x80000001 x=12.0000 y=13.0000 axis=1 value=31.0000 w1
pen 0x1307 state=0x80000001 x=12.0000 y=13.0000 axis=2 value=0.0000 w1
pen 0x1307 state=0x80000001 x=12.0000 y=13.0000 axis=4 value=91.0000 w1
pen 0x1304 state=0x80000003 x=12.0000 y=13.0000 button=1 down=1 w1
button 0x401 which=4294967294 button=3 down=1 clicks=1 x=12.0000 y=13.0000 w1
pen 0x1307 state=0x80000003 x=12.0000 y=13.0000 axis=1 value=0.0000 w1
pen 0x1307 state=0x80000003 x=12.0000 y=13.0000 axis=4 value=0.0000 w1
pen 0x1305 state=0x80000001 x=12.0000 y=13.0000 button=1 down=0 w1
button 0x402 which=4294967294 button=3 down=0 clicks=1 x=12.0000 y=13.0000 w1
pen 0x1304 state=0x80000005 x=12.0000 y=13.0000 button=2 down=1 w1
button 0x401 which=4294967294 button=2 down=1 clicks=1 x=12.0000 y=13.0000 w1
pen 0x1305 state=0x80000001 x=12.0000 y=13.0000 button=2 down=0 w1
button 0x402 which=4294967294 button=2 down=0 clicks=1 x=12.0000 y=13.0000 w1
pen 0x1302 state=0xc0000001 x=12.0000 y=13.0000 eraser=1 down=1 w1
finger 0x703 touch=18446744073709551614 finger=1 x=0.0375 y=0.0650 dx=0.0000 dy=0.0000 p=0.7500 w1
finger 0x700 touch=18446744073709551614 finger=1 x=0.0375 y=0.0650 dx=0.0000 dy=0.0000 p=0.7500 w1
pen 0x1303 state=0xc0000000 x=12.0000 y=13.0000 eraser=1 down=0 w1
button 0x402 which=4294967294 button=1 down=0 clicks=1 x=12.0000 y=13.0000 w1
finger 0x701 touch=18446744073709551614 finger=1 x=0.0375 y=0.0650 dx=0.0000 dy=0.0000 p=0.7500 w1
pen 0x1306 state=0xc0000000 x=14.0000 y=15.0000 w1
motion which=4294967294 state=0x0 x=14.0000 y=15.0000 xrel=2.0000 yrel=2.0000 w1
pen 0x1303 state=0x80000000 x=14.0000 y=15.0000 eraser=0 down=0 w1
pen 0x1307 state=0x80000000 x=14.0000 y=15.0000 axis=0 value=0.0000 w1
pen 0x1301 w1
== pen_again
dom: pointerenter, pointerleave
mod=0x0 focus=w1 kbd=w1
pen 0x1300 w1
pen 0x1306 state=0x80000000 x=20.0000 y=20.0000 w1
motion which=4294967294 state=0x0 x=20.0000 y=20.0000 xrel=6.0000 yrel=5.0000 w1
pen 0x1307 state=0x80000000 x=20.0000 y=20.0000 axis=0 value=0.1000 w1
pen 0x1306 state=0x80000000 x=21.0000 y=20.0000 w1
motion which=4294967294 state=0x0 x=21.0000 y=20.0000 xrel=1.0000 yrel=0.0000 w1
pen 0x1307 state=0x80000000 x=21.0000 y=20.0000 axis=0 value=0.0000 w1
pen 0x1301 w1
== focus
dom: keydown (prevented) KeyK:k, blur, focus, focus
mod=0x0 focus=w1 kbd=w1
key 0x300 sc=14 key=0x6b mod=0x0 down=1 repeat=0 w1
key 0x301 sc=14 key=0x6b mod=0x0 down=0 repeat=0 w1
window 0x20f 0 0 w1
window 0x20e 0 0 w1
== visibility
dom: visibilitychange, visibilitychange hidden=false
mod=0x0 focus=w1 kbd=w1
window 0x203 0 0 w1
window 0x202 0 0 w1
== beforeunload
watch 0x101
dom: beforeunload
mod=0x0 focus=w1 kbd=w1
== orientation
dom: change landscape-primary 0
mod=0x0 focus=w1 kbd=w1
display 0x151 1 0
== drop_text
dom: dragover (prevented), dragover (prevented), drop (prevented), dragover (prevented), dragleave (prevented), dragover (prevented), dragend (prevented)
mod=0x0 focus=w1 kbd=w1
drop 0x1002 x=0.0000 y=0.0000 data=(null) w1
drop 0x1004 x=30.0000 y=40.0000 data=(null) w1
drop 0x1004 x=31.0000 y=40.0000 data=(null) w1
drop 0x1001 x=31.0000 y=40.0000 data=dropped text w1
drop 0x1003 x=31.0000 y=40.0000 data=(null) w1
drop 0x1002 x=0.0000 y=0.0000 data=(null) w1
drop 0x1004 x=2.0000 y=3.0000 data=(null) w1
drop 0x1003 x=2.0000 y=3.0000 data=(null) w1
drop 0x1002 x=0.0000 y=0.0000 data=(null) w1
drop 0x1004 x=5.0000 y=6.0000 data=(null) w1
drop 0x1003 x=5.0000 y=6.0000 data=(null) w1
== thrown
watch: throwing
dom: keydown F9:F9, keyup (prevented) F9:F9, keydown (prevented) F8:F8, keyup (prevented) F8:F8 errors=["Uncaught Error: sdl3 test: thrown in an event watch"]
mod=0x0 focus=w1 kbd=w1
key 0x301 sc=66 key=0x40000042 mod=0x0 down=0 repeat=0 w1
key 0x300 sc=65 key=0x40000041 mod=0x0 down=1 repeat=0 w1
key 0x301 sc=65 key=0x40000041 mod=0x0 down=0 repeat=0 w1
-- main loop
== drop_files
dom: dragover (prevented), drop (prevented)
mod=0x0 focus=w1 kbd=w1
drop 0x1002 x=0.0000 y=0.0000 data=(null) w1
drop 0x1004 x=8.0000 y=9.0000 data=(null) w1
drop 0x1003 x=8.0000 y=9.0000 data=(null) w1
== drop_files_read
dom: files: . .. 0 1
mod=0x0 focus=w1 kbd=w1
drop 0x1002 x=0.0000 y=0.0000 data=(null) w1
drop 0x1000 x=0.0000 y=0.0000 data=/tmp/filedrop/0/a.txt w1
drop 0x1000 x=0.0000 y=0.0000 data=/tmp/filedrop/1/b b.txt w1
drop 0x1003 x=0.0000 y=0.0000 data=(null) w1
contents: [the first file] [the second]
== trusted_click
fullscreen request: 1
before the click: deferred=1 fullscreen=0 element=null
after the click: deferred=1 fullscreen=0 element=null
motion which=0 state=0x0 x=60.0000 y=40.0000 xrel=39.0000 yrel=20.0000 w1
button 0x401 which=0 button=1 down=1 clicks=1 x=60.0000 y=40.0000 w1
button 0x402 which=0 button=1 down=0 clicks=1 x=60.0000 y=40.0000 w1
== trusted_key
after the key: deferred=0 fullscreen=1 element=canvas
window 0x206 {SW} {SH} w1
window 0x207 {SW} {SH} w1
window 0x215 0 0 w1
key 0x300 sc=4 key=0x61 mod=0x0 down=1 repeat=0 w1
key 0x301 sc=4 key=0x61 mod=0x0 down=0 repeat=0 w1
window 0x217 0 0 w1
window 0x205 0 0 w1
== leave_fullscreen
leave: 1
left: deferred=0 fullscreen=0 element=null
window 0x218 0 0 w1
window 0x205 {CX} {CY} w1
window 0x206 320 200 w1
window 0x207 320 200 w1
window 0x215 0 0 w1
== two_windows
dom: focus blur pointerlockchange webkitfullscreenchange fullscreenchange resize wheel change visibilitychange beforeunload wheel change keydown keyup keypress visibilitychange beforeunload
mod=0x0 focus=w2 kbd=w2
window 0x20d 0 0 w1
window 0x20c 0 0 w2
window 0x20f 0 0 w1
window 0x20e 0 0 w2
window 0x202 0 0 w2
window 0x207 100 80 w2
window 0x215 0 0 w2
== two_windows_input
dom: keydown (prevented) KeyA:a, keyup (prevented) KeyA:a, pointerenter, pointermove, wheel (prevented), pointerenter, pointerleave, pointerdown, pointerup
mod=0x0 focus=w1 kbd=w2
key 0x300 sc=4 key=0x61 mod=0x0 down=1 repeat=0 w2
key 0x301 sc=4 key=0x61 mod=0x0 down=0 repeat=0 w2
motion which=0 state=0x0 x=1.0000 y=1.0000 xrel=0.0000 yrel=0.0000 w2
motion which=0 state=0x0 x=5.0000 y=6.0000 xrel=4.0000 yrel=5.0000 w2
wheel which=0 x=0.0000 y=-1.0000 ix=0 iy=-1 dir=0 mx=5.0000 my=6.0000 w2
window 0x20d 0 0 w2
window 0x20c 0 0 w1
motion which=0 state=0x0 x=5.0000 y=6.0000 xrel=0.0000 yrel=0.0000 w1
window 0x20d 0 0 w1
window 0x20c 0 0 w1
motion which=0 state=0x0 x=5.0000 y=6.0000 xrel=0.0000 yrel=0.0000 w1
button 0x401 which=0 button=1 down=1 clicks=1 x=5.0000 y=6.0000 w1
button 0x402 which=0 button=1 down=0 clicks=1 x=5.0000 y=6.0000 w1
== two_windows_focus
dom: focus, focus
mod=0x0 focus=w1 kbd=w1
window 0x20f 0 0 w2
window 0x20e 0 0 w1
== destroy_second
dom: keydown KeyA:a, keypress KeyA:a, keyup KeyA:a, pointermove | focus blur pointerlockchange webkitfullscreenchange fullscreenchange resize wheel | filedrop=false
mod=0x0 focus=w1 kbd=w0
window 0x219 0 0 w9
window 0x20d 0 0 w1
window 0x20f 0 0 w1
window 0x20c 0 0 w1
motion which=0 state=0x0 x=7.0000 y=8.0000 xrel=0.0000 yrel=0.0000 w1
== quit
destroyed: focus blur pointerlockchange webkitfullscreenchange fullscreenchange resize | filedrop=false | touchAction=
window 0x219 0 0 w9
quit: [] mouseup=undefined lockkeys=function
"#;
}
