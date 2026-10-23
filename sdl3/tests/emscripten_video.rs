// Tests of the Emscripten video driver in a real browser.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! The canvas video driver (`video/emscripten/`: `SDL_emscriptenvideo.c`,
//! `SDL_emscriptenframebuffer.c`, `SDL_emscriptenmouse.c`) in headless
//! Chromium: a program of its own (`harness = false`) that asks
//! `tools/emscripten_run.py` for the browser, as `emscripten_browser.rs`
//! does, on the runner's page (a `<canvas id="canvas">`, 640x480 before
//! SDL sizes it, with no CSS size).
//!
//! The synchronous checks first, each on what the page shows (the canvas's
//! size and CSS size, `document.title`, the icon's `<link>`, the canvas's
//! pixels and CSS `cursor`, the DOM of a fill-document window), with the
//! browser's events dispatched by the check where one is needed:
//!
//! * `video_init`: the driver, its display (the screen, its mode RGBA32 at
//!   the page's `devicePixelRatio`; the usable bounds are the page's inner
//!   size) and the hint it sets;
//! * `canvas`: a window's canvas, its size and properties, the focus,
//!   `Module.requestFullscreen()` through SDL, and the canvas sized 0x0
//!   once the window is destroyed;
//! * `pixel_density`: a high pixel density window at a `devicePixelRatio` of
//!   2 (a stand-in for the page's): the canvas twice the window's size, its
//!   CSS size the window's;
//! * `external_size`: a canvas the page's CSS sizes, which the window
//!   follows (and its `resize` events);
//! * `resize`: a `resize` event after the pixel ratio changed;
//! * `title`, `icon`: `document.title`; the icon's PNG in a `blob:` URL;
//! * `framebuffer`: the window surface's pixels, read back from the canvas
//!   (`getImageData()`);
//! * `cursors`: each system cursor's CSS name (upstream's table), hidden
//!   (`none`) and shown, and color cursors as PNG data URLs with their hot
//!   spot;
//! * `global_mouse_state`: the document's `mousemove`, `mousedown` and
//!   `mouseup`;
//! * `system_theme`: `prefers-color-scheme`, and a change (the listener,
//!   with a stand-in `matchMedia`);
//! * `fill_document`: a canvas that becomes the page, the second one that
//!   can't, and the page put back;
//! * `fullscreen_request`, `pointer_lock_request`: without a user gesture
//!   emscripten defers the requests (`EMSCRIPTEN_RESULT_DEFERRED`) until
//!   one of its event handlers runs: SDL reports fullscreen pending (and
//!   refuses another request meanwhile), relative mode on;
//! * `message_box`: without Asyncify, `alert()` (a stand-in records it).
//!
//! Then the main callbacks, which wait for the browser: the runner sends a
//! trusted key press (a user gesture, which SDL's `keydown` handler, one of
//! emscripten's, runs the deferred requests in): `fullscreen`, the
//! window fullscreen and back (`fullscreenchange`); `pointer_lock`, the
//! pointer locked (`pointerlockchange`), or headless Chromium's refusal
//! (`pointerlockerror`), which is printed. Finally `quit`: what video's
//! end undoes.
//!
//! Each check prints a libtest-like `test NAME ... ok` line; under node
//! everything skips as `browser`. `SDL3_EMSCRIPTEN_BROWSER=1
//! SDL3_TEST_REQUIRE=browser cargo test -p sdl3 --target
//! wasm32-unknown-emscripten --test emscripten_video` runs it as CI does.

#[cfg(not(target_os = "emscripten"))]
fn main() {}

#[cfg(target_os = "emscripten")]
fn main() -> std::process::ExitCode {
    video::main()
}

/// The program's request for the browser (see `emscripten_browser.rs`).
#[cfg(target_os = "emscripten")]
#[used]
#[link_section = "sdl3_runner"]
static RUNNER: [u8; 7] = *b"browser";

#[cfg(target_os = "emscripten")]
mod video {
    use std::panic::{self, AssertUnwindSafe};
    use std::process::ExitCode;
    use std::time::Duration;

    use sdl3::app::{self, AppCallbacks, AppResult};
    use sdl3::events::mouse::{self, MouseButtonFlags, SystemCursor};
    use sdl3::events::window::WindowFlags;
    use sdl3::events::{self, keyboard, Event, EventType};
    use sdl3::hints;
    use sdl3::init::{self, InitFlags};
    use sdl3::io::IoStream;
    use sdl3::properties::Properties;
    use sdl3::timer;
    use sdl3::video::messagebox::{
        show_message_box, show_simple_message_box, MessageBoxButtonData, MessageBoxButtonFlags,
        MessageBoxData, MessageBoxFlags,
    };
    use sdl3::video::window::{
        PROP_WINDOW_CREATE_EMSCRIPTEN_CANVAS_ID_STRING, PROP_WINDOW_CREATE_FLAGS_NUMBER,
        PROP_WINDOW_CREATE_HEIGHT_NUMBER, PROP_WINDOW_CREATE_TITLE_STRING,
        PROP_WINDOW_CREATE_WIDTH_NUMBER, PROP_WINDOW_EMSCRIPTEN_CANVAS_ID_STRING,
        PROP_WINDOW_EMSCRIPTEN_KEYBOARD_ELEMENT_STRING,
    };
    use sdl3::video::{self as sdl_video, PixelFormat, Rect, Surface, SystemTheme, Window};
    use sdl3_emscripten::{em_asm, em_asm_double, em_asm_int};

    /// How long the checks may wait for the browser.
    const PATIENCE: Duration = Duration::from_secs(5);

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

    #[derive(Default)]
    struct Report {
        passed: u32,
        failed: Vec<&'static str>,
    }

    impl Report {
        fn run(&mut self, name: &'static str, check: impl FnOnce()) {
            let ok = panic::catch_unwind(AssertUnwindSafe(check)).is_ok();
            self.done(name, ok);
        }

        fn done(&mut self, name: &'static str, ok: bool) {
            if ok {
                self.passed += 1;
                println!("test {name} ... ok");
            } else {
                self.failed.push(name);
                println!("test {name} ... FAILED");
            }
        }

        fn finish(&self) -> bool {
            let status = if self.failed.is_empty() {
                "ok"
            } else {
                "FAILED"
            };
            if !self.failed.is_empty() {
                println!("\nfailures:");
                for name in &self.failed {
                    println!("    {name}");
                }
            }
            println!(
                "\ntest result: {status}. {} passed; {} failed",
                self.passed,
                self.failed.len()
            );
            self.failed.is_empty()
        }
    }

    const CHECKS: [&str; 19] = [
        "video_init",
        "canvas",
        "pixel_density",
        "external_size",
        "resize",
        "title",
        "icon",
        "framebuffer",
        "cursors",
        "global_mouse_state",
        "system_theme",
        "fill_document",
        "fullscreen_request",
        "pointer_lock_request",
        "message_box",
        "fullscreen",
        "pointer_lock",
        "quit",
        "iterations",
    ];

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

    // -- JavaScript helpers ----------------------------------------------------

    /// The value of the JavaScript expression `expr` (evaluated in the
    /// glue's scope: `Module`, `JSEvents`, …), as a string.
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

    /// The JavaScript expression `expr` as a number.
    fn js_num(expr: &str) -> f64 {
        let code = format!("{expr}\0");
        // SAFETY: plain JavaScript; the code is NUL-terminated.
        unsafe { em_asm_double!("return Number(eval(UTF8ToString($0)));", code.as_ptr()) }
    }

    /// Run JavaScript statements.
    fn js_run(code: &str) {
        js(&format!("(function() {{ {code} }})()"));
    }

    /// `#canvas`'s `width`x`height`.
    fn canvas_size(selector: &str) -> (i32, i32) {
        (
            js_num(&format!("document.querySelector('{selector}').width")) as i32,
            js_num(&format!("document.querySelector('{selector}').height")) as i32,
        )
    }

    /// The CSS `width` and `height` set on the canvas's style.
    fn canvas_css(selector: &str) -> (String, String) {
        (
            js(&format!("document.querySelector('{selector}').style.width")),
            js(&format!(
                "document.querySelector('{selector}').style.height"
            )),
        )
    }

    /// A CSS `cursor` value as the browser writes it back.
    fn css_cursor(value: &str) -> String {
        let value = value.replace('\\', "\\\\").replace('\'', "\\'");
        js(&format!(
            "(function() {{ var d = document.createElement('div'); d.style.cursor = '{value}'; return d.style.cursor; }})()"
        ))
    }

    /// `Module['canvas']`'s CSS `cursor`.
    fn canvas_cursor() -> String {
        js("Module['canvas'].style.cursor")
    }

    fn drain() -> Vec<Event> {
        let mut out = Vec::new();
        while let Some(e) = events::poll() {
            out.push(e);
        }
        out
    }

    /// The window events of `window` in `events`, as `(type, data1, data2)`.
    fn window_events(events: &[Event], window: &Window) -> Vec<(EventType, i32, i32)> {
        events
            .iter()
            .filter_map(|e| match e {
                Event::Window(w) if w.window_id == window.id() => {
                    Some((w.event_type, w.data1, w.data2))
                }
                _ => None,
            })
            .collect()
    }

    /// A window event, as [`window_events`] lists them.
    const fn ev(event_type: EventType, data1: i32, data2: i32) -> (EventType, i32, i32) {
        (event_type, data1, data2)
    }

    /// What upstream's C sends a window it creates (no fill-document,
    /// `w`x`h` pixels).
    fn created(w: i32, h: i32) -> [(EventType, i32, i32); 5] {
        [
            ev(EventType::WINDOW_MOUSE_ENTER, 0, 0),
            ev(EventType::WINDOW_FOCUS_GAINED, 0, 0),
            ev(EventType::WINDOW_SHOWN, 0, 0),
            ev(EventType::WINDOW_PIXEL_SIZE_CHANGED, w, h),
            ev(EventType::WINDOW_SAFE_AREA_CHANGED, 0, 0),
        ]
    }

    /// What upstream's C sends a window resized to `w`x`h` (at a pixel
    /// density of 1).
    fn resized(w: i32, h: i32) -> [(EventType, i32, i32); 3] {
        [
            ev(EventType::WINDOW_RESIZED, w, h),
            ev(EventType::WINDOW_PIXEL_SIZE_CHANGED, w, h),
            ev(EventType::WINDOW_SAFE_AREA_CHANGED, 0, 0),
        ]
    }

    fn hex(bytes: &[u8]) -> String {
        bytes.iter().map(|b| format!("{b:02x}")).collect()
    }

    /// Makes the page's `devicePixelRatio` `ratio` (a stand-in; `None`
    /// puts the browser's back).
    fn set_device_pixel_ratio(ratio: Option<f64>) {
        match ratio {
            Some(ratio) => js_run(&format!(
                "if (!('sdl3TestDpr' in Module)) Module.sdl3TestDpr = Object.getOwnPropertyDescriptor(window, 'devicePixelRatio') || null; \
                 Object.defineProperty(window, 'devicePixelRatio', {{ configurable: true, get: () => {ratio} }});"
            )),
            None => js_run(
                "if ('sdl3TestDpr' in Module) { if (Module.sdl3TestDpr) Object.defineProperty(window, 'devicePixelRatio', Module.sdl3TestDpr); else delete window.devicePixelRatio; delete Module.sdl3TestDpr; }",
            ),
        }
    }

    /// The pixels of a canvas (its 2D context's), as hex.
    fn canvas_pixels(selector: &str) -> String {
        js(&format!(
            "(function() {{ var c = document.querySelector('{selector}'); var d = c.getContext('2d').getImageData(0, 0, c.width, c.height).data; \
             var h = ''; for (var i = 0; i < d.length; i++) h += ('0' + d[i].toString(16)).slice(-2); return h; }})()"
        ))
    }

    /// An opaque test pattern.
    fn pattern(surface: &mut Surface<'_>, seed: u8) {
        let (w, h, pitch) = (surface.width(), surface.height(), surface.pitch() as usize);
        let bpp = surface.format().bytes_per_pixel() as usize;
        let px = surface.pixels_mut().unwrap();
        for y in 0..h as usize {
            for x in 0..w as usize {
                let p = &mut px[y * pitch + x * bpp..][..bpp];
                p.copy_from_slice(&[
                    (x as u8).wrapping_mul(4).wrapping_add(seed),
                    (y as u8).wrapping_mul(8),
                    (x + y) as u8 ^ seed,
                    255,
                ]);
            }
        }
    }

    pub(super) fn main() -> ExitCode {
        if !in_browser() {
            let mut report = Report::default();
            for name in CHECKS {
                report.run(name, || {
                    skip(
                        "browser",
                        "not in a browser (under node: run it with SDL3_EMSCRIPTEN_BROWSER=1, or with CHROME naming Chrome or Chromium)",
                    )
                });
            }
            return if report.finish() {
                ExitCode::SUCCESS
            } else {
                ExitCode::FAILURE
            };
        }
        ExitCode::from(app::run_app(sdl_main) as u8)
    }

    fn sdl_main(args: &[String]) -> i32 {
        let mut report = Report::default();
        report.run("video_init", video_init);
        report.run("canvas", canvas);
        report.run("pixel_density", pixel_density);
        report.run("external_size", external_size);
        report.run("resize", resize);
        report.run("title", title);
        report.run("icon", icon);
        report.run("framebuffer", framebuffer);
        report.run("cursors", cursors);
        report.run("global_mouse_state", global_mouse_state);
        report.run("system_theme", system_theme);
        report.run("fill_document", fill_document);
        let mut window = None;
        report.run("fullscreen_request", || window = Some(fullscreen_request()));
        if let Some(w) = window {
            report.run("pointer_lock_request", || pointer_lock_request(w));
        } else {
            report.done("pointer_lock_request", false);
        }
        report.run("message_box", message_box);
        app::enter_app_main_callbacks(args, move |_| {
            let app = App {
                report,
                window,
                start: timer::ticks(),
                iterations: 0,
                step: Step::Start,
                since: timer::ticks(),
                timed_out: false,
                events: Vec::new(),
            };
            (app, AppResult::Continue)
        })
    }

    // -- video_init ------------------------------------------------------------

    fn video_init() {
        init::init(InitFlags::VIDEO).unwrap();
        assert_eq!(sdl_video::current_video_driver().unwrap(), "emscripten");
        // (Firefox's blur on the fullscreen prompt)
        assert_eq!(
            hints::get(hints::VIDEO_MINIMIZE_ON_FOCUS_LOSS).as_deref(),
            Some("0")
        );
        let displays = sdl_video::displays().unwrap();
        assert_eq!(displays.len(), 1);
        let display = displays[0];
        let (sw, sh) = (
            js_num("screen.width") as i32,
            js_num("screen.height") as i32,
        );
        let (iw, ih) = (
            js_num("window.innerWidth") as i32,
            js_num("window.innerHeight") as i32,
        );
        let ratio = js_num("window.devicePixelRatio");
        let mode = sdl_video::desktop_display_mode(display).unwrap();
        println!(
            "video_init: screen {sw}x{sh}, page {iw}x{ih}, devicePixelRatio {ratio}: {} {}x{} @{}",
            mode.format.name(),
            mode.w,
            mode.h,
            mode.pixel_density
        );
        assert_eq!(mode.format, PixelFormat::RGBA32);
        assert_eq!((mode.w, mode.h), (sw, sh));
        assert_eq!(mode.pixel_density, ratio as f32);
        assert_eq!(
            sdl_video::display_bounds(display).unwrap(),
            Rect::new(0, 0, sw, sh)
        );
        assert_eq!(
            sdl_video::display_usable_bounds(display).unwrap(),
            Rect::new(0, 0, iw, ih)
        );
        // The default cursor is set, not shown (nothing has the mouse focus).
        assert!(mouse::default_cursor().is_some());
        assert_eq!(canvas_cursor(), "");
        drain();
    }

    // -- canvas ----------------------------------------------------------------

    fn canvas() {
        let w = Window::create("canvas", 320, 200, WindowFlags::NONE).unwrap();
        assert_eq!(canvas_size("#canvas"), (320, 200));
        assert_eq!(canvas_css("#canvas"), (String::new(), String::new()));
        assert_eq!(w.size().unwrap(), (320, 200));
        assert_eq!(w.size_in_pixels().unwrap(), (320, 200));
        assert_eq!(w.pixel_density().unwrap(), 1.0);
        let props = w.properties().unwrap();
        assert_eq!(
            props
                .get_string(PROP_WINDOW_EMSCRIPTEN_CANVAS_ID_STRING)
                .as_deref(),
            Some("#canvas")
        );
        assert_eq!(
            props
                .get_string(PROP_WINDOW_EMSCRIPTEN_KEYBOARD_ELEMENT_STRING)
                .as_deref(),
            Some("#window")
        );
        // One window, it always has focus; its cursor is the default one.
        assert_eq!(mouse::mouse_focus(), Some(w.id()));
        assert_eq!(keyboard::keyboard_focus(), Some(w.id()));
        mouse::set_cursor(None).unwrap();
        assert_eq!(canvas_cursor(), "default");
        // emscripten's fullscreen button goes through SDL
        assert!(js("String(Module.requestFullscreen)").contains("_requestFullscreenThroughSDL("));
        assert_eq!(window_events(&drain(), &w), created(320, 200));

        w.set_size(400, 300).unwrap();
        assert_eq!(canvas_size("#canvas"), (400, 300));
        assert_eq!(w.size().unwrap(), (400, 300));
        assert_eq!(window_events(&drain(), &w), resized(400, 300));
        assert_eq!(w.size_in_pixels().unwrap(), (400, 300));

        let id = w.id();
        w.destroy();
        // We can't destroy the canvas, so resize it to zero instead
        assert_eq!(canvas_size("#canvas"), (0, 0));
        assert!(!js("String(Module.requestFullscreen)").contains("_requestFullscreenThroughSDL"));
        assert_ne!(mouse::mouse_focus(), Some(id));
        drain();
    }

    // -- pixel_density ---------------------------------------------------------

    fn pixel_density() {
        set_device_pixel_ratio(Some(2.0));
        let w = Window::create("hidpi", 100, 80, WindowFlags::HIGH_PIXEL_DENSITY).unwrap();
        assert_eq!(canvas_size("#canvas"), (200, 160));
        assert_eq!(canvas_css("#canvas"), ("100px".into(), "80px".into()));
        assert_eq!(w.size().unwrap(), (100, 80));
        assert_eq!(w.size_in_pixels().unwrap(), (200, 160));
        assert_eq!(w.pixel_density().unwrap(), 2.0);
        w.set_size(50, 40).unwrap();
        assert_eq!(canvas_size("#canvas"), (100, 80));
        assert_eq!(canvas_css("#canvas"), ("50px".into(), "40px".into()));
        assert_eq!(w.size_in_pixels().unwrap(), (100, 80));
        // The framebuffer is the window's size in pixels.
        let surface = w.surface().unwrap();
        {
            let s = surface.lock();
            assert_eq!((s.width(), s.height()), (100, 80));
            assert_eq!(s.format(), PixelFormat::RGBA32);
        }
        drop(surface);

        // Without the flag the ratio is 1.
        let flat = {
            let props = Properties::new();
            props.set(PROP_WINDOW_CREATE_TITLE_STRING, "flat").unwrap();
            props.set(PROP_WINDOW_CREATE_WIDTH_NUMBER, 30).unwrap();
            props.set(PROP_WINDOW_CREATE_HEIGHT_NUMBER, 20).unwrap();
            js_run("var c = document.createElement('canvas'); c.id = 'sdl3-test-flat'; document.body.appendChild(c);");
            props
                .set(
                    PROP_WINDOW_CREATE_EMSCRIPTEN_CANVAS_ID_STRING,
                    "#sdl3-test-flat",
                )
                .unwrap();
            Window::create_with_properties(&props).unwrap()
        };
        assert_eq!(canvas_size("#sdl3-test-flat"), (30, 20));
        assert_eq!(
            canvas_css("#sdl3-test-flat"),
            (String::new(), String::new())
        );
        assert_eq!(flat.pixel_density().unwrap(), 1.0);
        flat.destroy();
        js_run("document.getElementById('sdl3-test-flat').remove();");

        w.destroy();
        assert_eq!(canvas_size("#canvas"), (0, 0));
        // (the CSS size a canvas of a high pixel density window is left with)
        assert_eq!(canvas_css("#canvas"), ("1px".into(), "1px".into()));
        set_device_pixel_ratio(None);
        js_run(
            "var c = document.querySelector('#canvas'); c.style.width = ''; c.style.height = '';",
        );
        drain();
    }

    // -- external_size ---------------------------------------------------------

    fn external_size() {
        js_run("var c = document.querySelector('#canvas'); c.style.width = '256px'; c.style.height = '128px';");
        let w = Window::create(
            "css",
            320,
            200,
            WindowFlags::RESIZABLE | WindowFlags::FILL_DOCUMENT,
        )
        .unwrap();
        // The page's CSS sizes it: no fill-document (upstream keeps the flag).
        assert_eq!(w.size().unwrap(), (256, 128));
        assert_eq!(canvas_size("#canvas"), (256, 128));
        assert_eq!(
            js("!!document.getElementById('SDL3_fill_document_background_elements')"),
            "false"
        );
        assert!(w.flags().unwrap().contains(WindowFlags::FILL_DOCUMENT));
        assert_eq!(window_events(&drain(), &w), created(256, 128));

        // The page changes the CSS size: a resize event follows it.
        js_run("var c = document.querySelector('#canvas'); c.style.width = '300px'; c.style.height = '150px'; window.dispatchEvent(new Event('resize'));");
        assert_eq!(canvas_size("#canvas"), (300, 150));
        assert_eq!(w.size().unwrap(), (300, 150));
        assert_eq!(window_events(&drain(), &w), resized(300, 150));
        // (the CSS stays the page's)
        assert_eq!(canvas_css("#canvas"), ("300px".into(), "150px".into()));
        w.destroy();
        js_run(
            "var c = document.querySelector('#canvas'); c.style.width = ''; c.style.height = '';",
        );
        drain();
    }

    // -- resize ----------------------------------------------------------------

    fn resize() {
        let w = Window::create(
            "resize",
            200,
            100,
            WindowFlags::RESIZABLE | WindowFlags::HIGH_PIXEL_DENSITY,
        )
        .unwrap();
        assert_eq!(canvas_size("#canvas"), (200, 100));
        drain();
        // Nothing changed: nothing sent.
        js_run("window.dispatchEvent(new Event('resize'));");
        assert_eq!(window_events(&drain(), &w), []);
        // The pixel ratio changed: the canvas follows, and the event is
        // forced although the size is the same.
        set_device_pixel_ratio(Some(2.0));
        js_run("window.dispatchEvent(new Event('resize'));");
        assert_eq!(canvas_size("#canvas"), (400, 200));
        assert_eq!(canvas_css("#canvas"), ("200px".into(), "100px".into()));
        assert_eq!(w.size_in_pixels().unwrap(), (400, 200));
        assert_eq!(
            window_events(&drain(), &w),
            [
                ev(EventType::WINDOW_RESIZED, 200, 100),
                ev(EventType::WINDOW_PIXEL_SIZE_CHANGED, 400, 200),
                ev(EventType::WINDOW_DISPLAY_SCALE_CHANGED, 0, 0),
            ]
        );
        set_device_pixel_ratio(None);
        w.destroy();
        js_run(
            "var c = document.querySelector('#canvas'); c.style.width = ''; c.style.height = '';",
        );
        drain();
    }

    // -- title and icon --------------------------------------------------------

    fn title() {
        let w = Window::create("first title", 64, 64, WindowFlags::NONE).unwrap();
        assert_eq!(js("document.title"), "first title");
        let title = "Ünïcödé — ✓ title";
        w.set_title(title).unwrap();
        assert_eq!(js("document.title"), title);
        w.destroy();
        drain();
    }

    fn icon() {
        let w = Window::create("icon", 64, 64, WindowFlags::NONE).unwrap();
        // (the runner's page has a <link rel="icon" href="data:,">)
        assert_eq!(
            js("document.querySelectorAll(\"link[rel~='icon']\").length"),
            "1"
        );
        let mut icon = Surface::new(16, 16, PixelFormat::RGBA32).unwrap();
        pattern(&mut icon, 7);
        let png = |s: &Surface<'_>| {
            let mut stream = IoStream::from_dynamic_mem();
            s.duplicate().unwrap().save_png_io(&mut stream).unwrap();
            stream.dynamic_memory().unwrap().to_vec()
        };
        // The bytes behind the icon's URL, as hex ('' if it doesn't load).
        let fetch = |url: &str| {
            js(&format!(
                "(function() {{ try {{ var x = new XMLHttpRequest(); x.open('GET', '{url}', false); x.overrideMimeType('text/plain; charset=x-user-defined'); x.send(); \
                 var s = x.responseText, h = ''; for (var i = 0; i < s.length; i++) h += ('0' + (s.charCodeAt(i) & 255).toString(16)).slice(-2); return h; }} catch (e) {{ return ''; }} }})()"
            ))
        };
        w.set_icon(&icon).unwrap();
        let href = js("document.querySelector(\"link[rel~='icon']\").href");
        assert!(href.starts_with("blob:"), "{href}");
        assert_eq!(fetch(&href), hex(&png(&icon)));
        // Another icon: a new URL, the old one revoked.
        pattern(&mut icon, 99);
        w.set_icon(&icon).unwrap();
        let href2 = js("document.querySelector(\"link[rel~='icon']\").href");
        assert!(href2.starts_with("blob:") && href2 != href, "{href2}");
        assert_eq!(fetch(&href2), hex(&png(&icon)));
        assert_eq!(fetch(&href), "");
        assert_eq!(
            js("document.querySelectorAll(\"link[rel~='icon']\").length"),
            "1"
        );
        println!("icon: {} bytes of PNG at {href2}", png(&icon).len());
        w.destroy();
        drain();
    }

    // -- framebuffer -----------------------------------------------------------

    fn framebuffer() {
        let w = Window::create("framebuffer", 64, 32, WindowFlags::NONE).unwrap();
        let surface = w.surface().unwrap();
        let pixels = |s: &Surface<'_>| {
            let (w, h, pitch) = (s.width() as usize, s.height() as usize, s.pitch() as usize);
            let px = s.pixels().unwrap();
            let mut out = Vec::new();
            for y in 0..h {
                out.extend_from_slice(&px[y * pitch..][..w * 4]);
            }
            out
        };
        let expected = {
            let mut s = surface.lock();
            assert_eq!(s.format(), PixelFormat::RGBA32);
            assert_eq!((s.width(), s.height()), (64, 32));
            assert_eq!(s.pitch(), 64 * 4);
            // (zeroed, as upstream's SDL_CreateSurfaceZeroed())
            assert!(s.pixels().unwrap().iter().all(|&b| b == 0));
            pattern(&mut s, 3);
            pixels(&s)
        };
        surface.update().unwrap();
        assert_eq!(canvas_pixels("#canvas"), hex(&expected));
        assert_eq!(
            js("Module['SDL3'].ctxCanvas === document.querySelector('#canvas')"),
            "true"
        );
        assert_eq!(js("Module['SDL3'].w + 'x' + Module['SDL3'].h"), "64x32");

        // Again, with rectangles (the whole surface goes, as upstream's).
        let expected = {
            let mut s = surface.lock();
            pattern(&mut s, 200);
            pixels(&s)
        };
        surface.update_rects(&[Rect::new(0, 0, 1, 1)]).unwrap();
        assert_eq!(canvas_pixels("#canvas"), hex(&expected));
        drop(surface);

        // A new size, a new surface, a new ImageData.
        w.set_size(16, 8).unwrap();
        let surface = w.surface().unwrap();
        let expected = {
            let mut s = surface.lock();
            assert_eq!((s.width(), s.height()), (16, 8));
            pattern(&mut s, 42);
            pixels(&s)
        };
        surface.update().unwrap();
        assert_eq!(canvas_pixels("#canvas"), hex(&expected));
        assert_eq!(js("Module['SDL3'].w + 'x' + Module['SDL3'].h"), "16x8");
        drop(surface);
        w.destroy();
        drain();
    }

    // -- cursors ---------------------------------------------------------------

    /// Upstream's `SDL_GetCSSCursorName()`, in `SDL_SystemCursor` order.
    const CSS_CURSORS: [&str; SystemCursor::COUNT] = [
        "default",
        "text",
        "wait",
        "crosshair",
        "progress",
        "nwse-resize",
        "nesw-resize",
        "ew-resize",
        "ns-resize",
        "move",
        "not-allowed",
        "pointer",
        "nw-resize",
        "n-resize",
        "ne-resize",
        "e-resize",
        "se-resize",
        "s-resize",
        "sw-resize",
        "w-resize",
        "context-menu",
        "help",
        "cell",
        "vertical-text",
        "alias",
        "copy",
        "no-drop",
        "grab",
        "grabbing",
        "col-resize",
        "row-resize",
        "all-scroll",
        "zoom-in",
        "zoom-out",
    ];

    fn cursors() {
        let w = Window::create("cursors", 64, 64, WindowFlags::NONE).unwrap();
        for (i, name) in CSS_CURSORS.iter().enumerate() {
            let id = SystemCursor::from_index(i as i32).unwrap();
            let cursor = mouse::create_system_cursor(id).unwrap();
            mouse::set_cursor(Some(&cursor)).unwrap();
            assert_eq!(canvas_cursor(), *name, "{id:?}");
        }
        mouse::hide_cursor();
        assert_eq!(canvas_cursor(), "none");
        mouse::show_cursor();
        assert_eq!(canvas_cursor(), "zoom-out");

        // A color cursor: its pixels as a PNG data URL, with the hot spot.
        let mut image = Surface::new(8, 8, PixelFormat::RGBA32).unwrap();
        pattern(&mut image, 11);
        let data_url = |s: &Surface<'_>| {
            let px = s.pixels().unwrap();
            // SAFETY: plain JavaScript, reading the 8x8 RGBA32 pixels.
            let len = unsafe {
                em_asm_int!(
                    r#"
                    var c = document.createElement('canvas'); c.width = 8; c.height = 8;
                    var ctx = c.getContext('2d'); var image = ctx.createImageData(8, 8);
                    image.data.set(HEAPU8.subarray($0, $0 + 256)); ctx.putImageData(image, 0, 0);
                    Module['sdl3_test_url'] = c.toDataURL(); return Module['sdl3_test_url'].length;
                "#,
                    px.as_ptr()
                )
            };
            let url = js("Module['sdl3_test_url']");
            assert_eq!(url.len(), len as usize);
            url
        };
        let url = data_url(&image);
        assert!(url.starts_with("data:image/png;base64,"), "{url}");
        for (hot_x, hot_y, expected) in [
            (2, 3, format!("url({url}) 2 3, auto")),
            (0, 0, format!("url({url}), auto")),
        ] {
            let cursor = mouse::create_color_cursor(&image, hot_x, hot_y).unwrap();
            mouse::set_cursor(Some(&cursor)).unwrap();
            assert_eq!(
                canvas_cursor(),
                css_cursor(&expected),
                "hot spot {hot_x},{hot_y}"
            );
        }
        println!(
            "cursors: a color cursor is {} bytes of CSS",
            canvas_cursor().len()
        );
        mouse::set_cursor(mouse::default_cursor().as_ref()).unwrap();
        assert_eq!(canvas_cursor(), "default");
        w.destroy();
        drain();
    }

    // -- global_mouse_state ----------------------------------------------------

    fn global_mouse_state() {
        let mouse_event = |kind: &str, x: i32, y: i32, button: i32| {
            js_run(&format!(
                "document.dispatchEvent(new MouseEvent('{kind}', {{ clientX: {x}, clientY: {y}, button: {button} }}));"
            ))
        };
        mouse_event("mousemove", 12, 34, 0);
        assert_eq!(
            mouse::global_mouse_state(),
            (12.0, 34.0, MouseButtonFlags::NONE)
        );
        mouse_event("mousedown", 13, 35, 0);
        mouse_event("mousemove", 20, 40, 0);
        assert_eq!(
            mouse::global_mouse_state(),
            (20.0, 40.0, MouseButtonFlags::LMASK)
        );
        // (the DOM's button 1 is the middle one, 2 the right one: SDL's bits)
        mouse_event("mousedown", 20, 40, 2);
        mouse_event("mousedown", 20, 40, 1);
        assert_eq!(
            mouse::global_mouse_state().2,
            MouseButtonFlags::LMASK | MouseButtonFlags::MMASK | MouseButtonFlags::RMASK
        );
        // (out of range: ignored)
        mouse_event("mousedown", 20, 40, 5);
        mouse_event("mouseup", 21, 41, 0);
        assert_eq!(
            mouse::global_mouse_state(),
            (
                20.0,
                40.0,
                MouseButtonFlags::MMASK | MouseButtonFlags::RMASK
            )
        );
        mouse_event("mouseup", 21, 41, 1);
        mouse_event("mouseup", 21, 41, 2);
        mouse_event("mousemove", 0, 0, 0);
        assert_eq!(
            mouse::global_mouse_state(),
            (0.0, 0.0, MouseButtonFlags::NONE)
        );
        drain();
    }

    // -- system_theme ----------------------------------------------------------

    fn system_theme() {
        let page = js(
            "window.matchMedia('(prefers-color-scheme: light)').matches ? 'light' : window.matchMedia('(prefers-color-scheme: dark)').matches ? 'dark' : 'unknown'",
        );
        let theme = sdl_video::system_theme();
        println!("system_theme: the page prefers {page}: {theme:?}");
        let expected = match page.as_str() {
            "light" => SystemTheme::Light,
            "dark" => SystemTheme::Dark,
            _ => SystemTheme::Unknown,
        };
        assert_eq!(theme, expected);
        assert_eq!(
            js("typeof Module['SDL3'].eventHandlerThemeChanged"),
            "function"
        );
        drain();
        let theme_events = || {
            drain()
                .iter()
                .filter(|e| e.event_type() == EventType::SYSTEM_THEME_CHANGED)
                .count()
        };
        // The listener (a change of `prefers-color-scheme: dark`), with
        // the page's preference dark, then light, then neither.
        js_run("Module.sdl3TestMatchMedia = window.matchMedia; Module.sdl3TestScheme = 'dark'; window.matchMedia = (q) => ({ matches: q === '(prefers-color-scheme: ' + Module.sdl3TestScheme + ')' });");
        let change = |scheme: &str| {
            js_run(&format!(
                "Module.sdl3TestScheme = '{scheme}'; Module['SDL3'].eventHandlerThemeChanged({{}});"
            ))
        };
        change("dark");
        assert_eq!(sdl_video::system_theme(), SystemTheme::Dark);
        assert_eq!(theme_events(), usize::from(expected != SystemTheme::Dark));
        change("dark");
        assert_eq!(theme_events(), 0);
        change("light");
        assert_eq!(sdl_video::system_theme(), SystemTheme::Light);
        assert_eq!(theme_events(), 1);
        change("none");
        assert_eq!(sdl_video::system_theme(), SystemTheme::Unknown);
        assert_eq!(theme_events(), 1);
        js_run("window.matchMedia = Module.sdl3TestMatchMedia; delete Module.sdl3TestMatchMedia; delete Module.sdl3TestScheme; Module['SDL3'].eventHandlerThemeChanged({});");
        assert_eq!(sdl_video::system_theme(), expected);
        assert_eq!(
            theme_events(),
            usize::from(expected != SystemTheme::Unknown)
        );
    }

    // -- fill_document ---------------------------------------------------------

    fn fill_document() {
        js_run(
            "var c = document.createElement('canvas'); c.id = 'sdl3-test-fill'; c.style.position = 'absolute'; c.style.top = '5px'; c.style.left = '6px'; document.body.appendChild(c); \
             document.body.appendChild(document.createTextNode('\\n')); \
             var p = document.createElement('p'); p.id = 'sdl3-test-text'; p.textContent = 'text'; document.body.appendChild(p);",
        );
        let body = || {
            js("Array.from(document.body.children).map((e) => e.tagName + (e.id ? '#' + e.id : '')).join(' ')")
        };
        let before = body();
        // How upstream's JavaScript puts the page back: the canvas where it
        // was (before its next sibling, a text node, which stayed), the
        // other elements in their order where the hidden <div> was, at the
        // end: so after the canvas.
        let restored = std::iter::once("CANVAS#sdl3-test-fill")
            .chain(before.split(' ').filter(|e| *e != "CANVAS#sdl3-test-fill"))
            .collect::<Vec<_>>()
            .join(" ");
        let (iw, ih) = (
            js_num("window.innerWidth") as i32,
            js_num("window.innerHeight") as i32,
        );
        let props = Properties::new();
        props.set(PROP_WINDOW_CREATE_TITLE_STRING, "fill").unwrap();
        props.set(PROP_WINDOW_CREATE_WIDTH_NUMBER, 120).unwrap();
        props.set(PROP_WINDOW_CREATE_HEIGHT_NUMBER, 90).unwrap();
        props
            .set(
                PROP_WINDOW_CREATE_FLAGS_NUMBER,
                WindowFlags::FILL_DOCUMENT.0 as i64,
            )
            .unwrap();
        props
            .set(
                PROP_WINDOW_CREATE_EMSCRIPTEN_CANVAS_ID_STRING,
                "#sdl3-test-fill",
            )
            .unwrap();
        let w = Window::create_with_properties(&props).unwrap();
        assert!(w.flags().unwrap().contains(WindowFlags::FILL_DOCUMENT));
        assert_eq!(w.size().unwrap(), (iw, ih));
        assert_eq!(canvas_size("#sdl3-test-fill"), (iw, ih));
        // The canvas is the page; the rest is hidden.
        let filled = body();
        println!("fill_document: {before} -> {filled}");
        assert_eq!(
            filled,
            "DIV#SDL3_fill_document_background_elements CANVAS#sdl3-test-fill"
        );
        assert_eq!(
            js("document.getElementById('SDL3_fill_document_background_elements').style.display"),
            "none"
        );
        assert_eq!(
            js("Array.from(document.getElementById('SDL3_fill_document_background_elements').children).map((e) => e.id).join(' ')"),
            before
                .split(' ')
                .filter(|e| *e != "CANVAS#sdl3-test-fill")
                .map(|e| e.split_once('#').map_or("", |(_, id)| id))
                .collect::<Vec<_>>()
                .join(" ")
        );
        assert_eq!(
            js("(function() { var s = document.getElementById('sdl3-test-fill').style; return s.position + ' ' + s.top + ' ' + s.left; })()"),
            "fixed 0px 0px"
        );
        // (the resize, sent while it's made, before the front end's events)
        let [_, _, _, pixels, safe] = created(iw, ih);
        assert_eq!(
            window_events(&drain(), &w),
            [
                ev(EventType::WINDOW_RESIZED, iw, ih),
                pixels,
                safe,
                ev(EventType::WINDOW_MOUSE_ENTER, 0, 0),
                ev(EventType::WINDOW_FOCUS_GAINED, 0, 0),
                ev(EventType::WINDOW_SHOWN, 0, 0),
            ]
        );

        // Its size is the page's.
        w.set_size(10, 10).unwrap();
        assert_eq!(canvas_size("#sdl3-test-fill"), (iw, ih));
        // (a resize event of the page, the same size: nothing to send)
        js_run("window.dispatchEvent(new Event('resize'));");
        assert_eq!(canvas_size("#sdl3-test-fill"), (iw, ih));
        drain();

        // Only one at a time.
        let other = Window::create("other", 50, 40, WindowFlags::FILL_DOCUMENT).unwrap();
        assert!(!other.flags().unwrap().contains(WindowFlags::FILL_DOCUMENT));
        // (its canvas, #canvas, is hidden with the rest of the page: its CSS
        // size is 0x0, an external size, which the window takes)
        assert_eq!(other.size().unwrap(), (0, 0));
        let e = other.set_fill_document(true).unwrap_err();
        assert_eq!(
            e.message(),
            "Only one fill-document window allowed at a time."
        );
        other.destroy();

        // Back: the page as it was, the window its size before.
        w.set_fill_document(false).unwrap();
        assert!(!w.flags().unwrap().contains(WindowFlags::FILL_DOCUMENT));
        assert_eq!(body(), restored);
        assert_eq!(
            js("(function() { var s = document.getElementById('sdl3-test-fill').style; return s.position + ' ' + s.top + ' ' + s.left; })()"),
            "absolute 5px 6px"
        );
        assert_eq!(w.size().unwrap(), (120, 90));
        assert_eq!(canvas_size("#sdl3-test-fill"), (120, 90));
        // (the focus went to the other window, and stayed with it)
        let [r, p, s] = resized(120, 90);
        assert_eq!(
            window_events(&drain(), &w),
            [
                ev(EventType::WINDOW_MOUSE_LEAVE, 0, 0),
                ev(EventType::WINDOW_FOCUS_LOST, 0, 0),
                r,
                p,
                s,
            ]
        );

        // And once more, then destroyed while it fills the page.
        w.set_fill_document(true).unwrap();
        assert_eq!(
            js("!!document.getElementById('SDL3_fill_document_background_elements')"),
            "true"
        );
        w.destroy();
        assert_eq!(body(), restored);
        assert_eq!(canvas_size("#sdl3-test-fill"), (0, 0));
        js_run("document.getElementById('sdl3-test-fill').remove(); document.getElementById('sdl3-test-text').remove();");
        drain();
    }

    // -- fullscreen and pointer lock requests ----------------------------------

    /// The calls emscripten has deferred until its next event handler.
    fn deferred_calls() -> i32 {
        js_num("JSEvents.deferredCalls.length") as i32
    }

    /// The window the main callbacks go on with.
    fn fullscreen_request() -> Window {
        let w = Window::create("fullscreen", 160, 120, WindowFlags::NONE).unwrap();
        drain();
        assert_eq!(deferred_calls(), 0);
        // Without a user gesture: deferred, pending.
        w.set_fullscreen(true).unwrap();
        assert_eq!(deferred_calls(), 1);
        assert!(!w.flags().unwrap().contains(WindowFlags::FULLSCREEN));
        assert_eq!(js("document.fullscreenElement"), "null");
        // In progress: another request fails (and isn't deferred again).
        let e = w.set_fullscreen(true).unwrap_err();
        assert_eq!(e.message(), "Couldn't set the window fullscreen");
        // emscripten's own button: through SDL, which refuses it too.
        js_run("Module.requestFullscreen(false, false);");
        assert_eq!(deferred_calls(), 1);
        // A fullscreenchange without a fullscreen element (and none before)
        // names no canvas: upstream's handler finds no window.
        js_run("document.dispatchEvent(new Event('fullscreenchange'));");
        assert!(w.set_fullscreen(true).is_err());
        let events = window_events(&drain(), &w);
        assert!(
            !events
                .iter()
                .any(|e| e.0 == EventType::WINDOW_ENTER_FULLSCREEN),
            "{events:?}"
        );
        w
    }

    fn pointer_lock_request(w: Window) {
        assert_eq!(mouse::mouse_focus(), Some(w.id()));
        let before = deferred_calls();
        w.set_relative_mouse_mode(true).unwrap();
        assert!(w.relative_mouse_mode().unwrap());
        assert_eq!(deferred_calls(), before + 1);
        assert_eq!(js("document.pointerLockElement"), "null");
        // Leaving drops the deferred request.
        w.set_relative_mouse_mode(false).unwrap();
        assert!(!w.relative_mouse_mode().unwrap());
        assert_eq!(deferred_calls(), before);
        drain();
    }

    // -- message_box -----------------------------------------------------------

    fn message_box() {
        js_run("Module.sdl3TestAlert = window.alert; Module.sdl3TestAlerts = []; window.alert = (m) => Module.sdl3TestAlerts.push(m);");
        show_simple_message_box(MessageBoxFlags::INFORMATION, "Title", "The message", None)
            .unwrap();
        let data = MessageBoxData {
            flags: MessageBoxFlags::WARNING,
            title: "Two".into(),
            message: "buttons".into(),
            buttons: vec![
                MessageBoxButtonData {
                    flags: MessageBoxButtonFlags::RETURNKEY_DEFAULT,
                    button_id: 1,
                    text: "Yes".into(),
                },
                MessageBoxButtonData {
                    flags: MessageBoxButtonFlags::ESCAPEKEY_DEFAULT,
                    button_id: 2,
                    text: "No".into(),
                },
            ],
            ..Default::default()
        };
        // (no button: alert() has none)
        assert_eq!(show_message_box(&data).unwrap(), -1);
        assert_eq!(
            js("JSON.stringify(Module.sdl3TestAlerts)"),
            r#"["Title\n\nThe message","Two\n\nbuttons"]"#
        );
        // (no dialog: that's the Asyncify path)
        assert_eq!(js("document.querySelectorAll('dialog').length"), "0");
        js_run("window.alert = Module.sdl3TestAlert; delete Module.sdl3TestAlert; delete Module.sdl3TestAlerts;");
        drain();
    }

    // -- the main callbacks: trusted input -------------------------------------

    /// A trusted key press, which the runner sends (`Input.dispatchKeyEvent`)
    /// once the page is back in its event loop.
    fn press_key() {
        js_run(
            "for (var t of ['keyDown', 'keyUp']) sdl3RunnerReport(JSON.stringify(['input', ['dispatchKeyEvent', \
             { type: t, key: 'a', code: 'KeyA', text: t === 'keyDown' ? 'a' : undefined, windowsVirtualKeyCode: 65, nativeVirtualKeyCode: 65 }]]));",
        );
    }

    #[derive(Debug)]
    enum Step {
        Start,
        /// The key press sent: waiting for fullscreen.
        EnterFullscreen,
        LeaveFullscreen,
        /// The key press sent: waiting for the pointer lock (or its error).
        PointerLock,
        Unlock,
        Done,
    }

    struct App {
        report: Report,
        window: Option<Window>,
        start: Duration,
        iterations: u32,
        step: Step,
        /// When the step started.
        since: Duration,
        timed_out: bool,
        /// The events the main callbacks delivered during the step.
        events: Vec<Event>,
    }

    impl App {
        fn late(&self) -> bool {
            timer::ticks() - self.since > PATIENCE
        }

        fn next(&mut self, step: Step) {
            self.step = step;
            self.since = timer::ticks();
            self.events.clear();
        }

        /// The browser's `fullscreenchange` events so far.
        fn fullscreen_changes(&self) -> i32 {
            js_num("Module.sdl3TestFullscreenChanges") as i32
        }

        fn fullscreen(&self) -> bool {
            self.window
                .is_some_and(|w| w.flags().unwrap().contains(WindowFlags::FULLSCREEN))
        }

        fn step(&mut self) {
            let Some(window) = self.window else {
                self.report.done("fullscreen", false);
                self.report.done("pointer_lock", false);
                self.next(Step::Done);
                return;
            };
            match self.step {
                Step::Start => {
                    // SDL's keydown handler (the window's keyboard element is
                    // #window), which runs emscripten's deferred requests
                    assert_eq!(
                        js("JSEvents.eventHandlers.filter((h) => h.eventTypeString == 'keydown' && h.target === window).length"),
                        "1"
                    );
                    js_run("Module.sdl3TestFullscreenChanges = 0; document.addEventListener('fullscreenchange', () => { Module.sdl3TestFullscreenChanges++; });");
                    // (the fullscreen request of fullscreen_request(),
                    // still deferred)
                    assert_eq!(deferred_calls(), 1);
                    press_key();
                    self.next(Step::EnterFullscreen);
                }
                Step::EnterFullscreen => {
                    let events = window_events(&self.events, &window);
                    if self.fullscreen_changes() >= 1 && self.fullscreen() {
                        println!(
                            "fullscreen: {} ({events:?})",
                            js("document.fullscreenElement.id")
                        );
                        assert_eq!(js("document.fullscreenElement.id"), "canvas");
                        // (upstream's C, on the same page: the canvas resize
                        // callback's size, the screen's, then the front end's
                        // events)
                        let (sw, sh) = (
                            js_num("screen.width") as i32,
                            js_num("screen.height") as i32,
                        );
                        assert_eq!(
                            events,
                            [
                                ev(EventType::WINDOW_RESIZED, sw, sh),
                                ev(EventType::WINDOW_PIXEL_SIZE_CHANGED, sw, sh),
                                ev(EventType::WINDOW_SAFE_AREA_CHANGED, 0, 0),
                                ev(EventType::WINDOW_ENTER_FULLSCREEN, 0, 0),
                                ev(EventType::WINDOW_MOVED, 0, 0),
                            ]
                        );
                        assert_eq!(window.size().unwrap(), (sw, sh));
                        // (fullscreen desktop: the canvas stretched to the screen)
                        println!(
                            "fullscreen: the window {:?}, the canvas {:?} (CSS {:?})",
                            window.size().unwrap(),
                            canvas_size("#canvas"),
                            canvas_css("#canvas")
                        );
                        window.set_fullscreen(false).unwrap();
                        self.next(Step::LeaveFullscreen);
                    } else if self.late() {
                        // (recorded: what headless Chromium did with the request)
                        println!(
                            "fullscreen: not entered after the key press; deferred calls {}, events {events:?}",
                            deferred_calls()
                        );
                        self.report.done("fullscreen", false);
                        self.start_pointer_lock();
                    }
                }
                Step::LeaveFullscreen => {
                    let events = window_events(&self.events, &window);
                    if self.fullscreen_changes() >= 2 {
                        assert_eq!(js("document.fullscreenElement"), "null");
                        assert!(!self.fullscreen());
                        // The browser's fullscreenchange came: no change in
                        // progress, another request is taken.
                        println!("fullscreen: left ({events:?})");
                        // (upstream's C: the front end's events when it
                        // leaves, the window back where it was centered,
                        // then the browser's fullscreenchange)
                        let kinds: Vec<_> = events.iter().map(|e| e.0).collect();
                        assert_eq!(
                            kinds,
                            [
                                EventType::WINDOW_LEAVE_FULLSCREEN,
                                EventType::WINDOW_MOVED,
                                EventType::WINDOW_RESIZED,
                                EventType::WINDOW_PIXEL_SIZE_CHANGED,
                                EventType::WINDOW_SAFE_AREA_CHANGED,
                            ]
                        );
                        assert_eq!(&events[2..4], &resized(160, 120)[..2]);
                        assert_eq!(window.size().unwrap(), (160, 120));
                        assert!(window.set_fullscreen(true).is_ok());
                        assert_eq!(deferred_calls(), 1);
                        self.report.done("fullscreen", true);
                        self.start_pointer_lock();
                    } else if self.late() {
                        println!("fullscreen: still fullscreen ({events:?})");
                        self.report.done("fullscreen", false);
                        self.start_pointer_lock();
                    }
                }
                Step::PointerLock => {
                    let state = js("Module.sdl3TestLock");
                    if state != "pending" {
                        println!("pointer_lock: {state}");
                        if state == "locked" {
                            assert_eq!(js("document.pointerLockElement.id"), "canvas");
                        } else {
                            // (headless Chromium refuses the lock:
                            // recorded, not a failure of SDL's)
                            assert_eq!(state, "error");
                        }
                        window.set_relative_mouse_mode(false).unwrap();
                        self.next(Step::Unlock);
                    } else if self.late() {
                        println!("pointer_lock: neither locked nor refused after the key press");
                        window.set_relative_mouse_mode(false).unwrap();
                        self.report.done("pointer_lock", false);
                        self.next(Step::Done);
                    }
                }
                Step::Unlock => {
                    if js("document.pointerLockElement") == "null" {
                        assert_eq!(deferred_calls(), 0);
                        self.report.done("pointer_lock", true);
                        self.next(Step::Done);
                    } else if self.late() {
                        self.report.done("pointer_lock", false);
                        self.next(Step::Done);
                    }
                }
                Step::Done => {}
            }
        }

        fn start_pointer_lock(&mut self) {
            js_run(
                "Module.sdl3TestLock = 'pending'; \
                 document.addEventListener('pointerlockchange', () => { if (document.pointerLockElement) Module.sdl3TestLock = 'locked'; }); \
                 document.addEventListener('pointerlockerror', () => { Module.sdl3TestLock = 'error'; });",
            );
            // (nothing else deferred, for the key press)
            js_run("JSEvents.deferredCalls.length = 0;");
            let window = self.window.unwrap();
            window.set_relative_mouse_mode(true).unwrap();
            assert_eq!(deferred_calls(), 1);
            press_key();
            self.next(Step::PointerLock);
        }
    }

    impl AppCallbacks for App {
        fn iterate(&mut self) -> AppResult {
            self.iterations += 1;
            if !matches!(self.step, Step::Done)
                && panic::catch_unwind(AssertUnwindSafe(|| self.step())).is_err()
            {
                println!("(failed at {:?})", self.step);
                match self.step {
                    Step::Start | Step::EnterFullscreen | Step::LeaveFullscreen => {
                        self.report.done("fullscreen", false);
                        self.report.done("pointer_lock", false);
                    }
                    _ => self.report.done("pointer_lock", false),
                }
                self.next(Step::Done);
            }
            if matches!(self.step, Step::Done) {
                return if self.report.failed.is_empty() {
                    AppResult::Success
                } else {
                    AppResult::Failure
                };
            }
            if timer::ticks() - self.start > PATIENCE * 6 {
                println!("the checks didn't finish in {} s", (PATIENCE * 6).as_secs());
                self.timed_out = true;
                return AppResult::Failure;
            }
            AppResult::Continue
        }

        fn event(&mut self, event: &Event) -> AppResult {
            self.events.push(event.clone());
            AppResult::Continue
        }

        fn quit(&mut self, result: AppResult) {
            self.report.done("iterations", !self.timed_out);
            let seconds = (timer::ticks() - self.start).as_secs_f64();
            println!("{} iterations in {seconds:.2} s", self.iterations);
            let window = self.window.take();
            self.report.run("quit", move || {
                if let Some(w) = window {
                    w.destroy();
                }
                init::quit();
                // The theme listener is gone, emscripten's fullscreen
                // button does nothing.
                assert_eq!(
                    js("typeof Module['SDL3'].eventHandlerThemeChanged"),
                    "undefined"
                );
                assert!(!js("String(Module.requestFullscreen)")
                    .contains("_requestFullscreenThroughSDL"));
                assert_eq!(canvas_size("#canvas"), (0, 0));
            });
            let ok = self.report.finish();
            if ok != (result == AppResult::Success) {
                println!("(ended with {result:?})");
            }
        }
    }
}
