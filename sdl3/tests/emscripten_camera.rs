// Tests of the Emscripten camera driver (getUserMedia()) in a real browser.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! The MediaStream camera driver (`camera/emscripten/SDL_camera_emscripten.c`)
//! in headless Chromium: a program of its own (`harness = false`) that asks
//! `tools/emscripten_run.py` for the browser, as `emscripten_audio.rs`
//! does, where the camera's iterations run from the browser's event loop
//! (the driver's `requestAnimationFrame()` loop) while the main callbacks
//! wait for them.
//!
//! The browser's own camera (the runner's fake one) draws a moving picture
//! whose pixels and timing depend on the browser, so most steps use
//! stand-ins the page installs first (`Module.cam`, `C` below), which the
//! driver can't tell from the real thing:
//!
//! * `navigator.mediaDevices.getUserMedia()` records the constraints it's
//!   asked for and returns a promise the program settles: with a stream
//!   whose one track has the settings the program chooses (a size and a
//!   frame rate, or none) and records its `stop()`, or with an error;
//! * the "video" element the driver makes is a canvas of the stream's size
//!   with known pixels (byte `i` is `(seed × 37 + i × 11) & 255`, alpha
//!   255), which the program fires `loadedmetadata` on: the driver's
//!   `drawImage()` of it and `getImageData()` give those pixels exactly;
//! * the clock (`performance.now()`, which the driver's frame loop reads,
//!   and `Date.now()`, which SDL's ticks come from on Emscripten) stands
//!   still at a whole number of milliseconds, and the program moves it: so
//!   which display frame takes a camera frame, and the frames' timestamps,
//!   are exact. (It runs again, never going back, for the browser's own
//!   camera.)
//!
//! Each step writes a trace: what SDL answered (drivers, cameras, formats,
//! permissions, errors, the camera events the main callbacks and the queue
//! got), what the page saw (the constraints asked for, the console's lines
//! from the driver, the streams stopped, the driver's state in
//! `Module['SDL3'].camera`, the frames drawn), and the frames themselves.
//! [`EXPECTED`] is the trace of upstream's C (`libSDL3.a`, SDL 3.5.0) on
//! the same page with the same steps and stand-ins, its
//! `SDL_camera_emscripten.c` patched as the translation fixes it (see
//! below), and each step's part must be the same here. The steps:
//!
//! * `init`: the drivers (the Emscripten one first, then dummy, as
//!   upstream's Emscripten build lists them), the one camera without
//!   formats and its added event, nothing asked of the browser yet;
//! * `deny`: the camera opened, pending (no format, no frames, opened only
//!   once) with `{"video":true}` asked; then a `NotAllowedError`: the
//!   denied event and the driver's console error; closing it;
//! * `approve`: a 4×2 stream at 25 fps: the approved event, RGBA32 4×2 at
//!   25/1, the hidden video and canvas; the loop starting at
//!   `loadedmetadata` and its first frame dropped; the next frame's exact
//!   pixels, timestamped with SDL's ticks when it's taken; no frame while
//!   the clock stands still;
//! * `frames`: the clock moved 40 ms (a frame, the picture changed), 100 ms
//!   (one frame: the loop skips the times it missed), 10 ms (none: not
//!   due), 10 ms more (one); nine frames not taken (eight are kept, the
//!   ninth dropped) and the next one;
//! * `close`: the stream's track stopped, the state emptied, the loop
//!   stopped;
//! * `spec`: XRGB8888 2×1 at 25 fps asked for (`{"video":{"width":2,
//!   "height":1,"frameRate":{"ideal":25}}}`), 4×2 at 30 fps given: frames
//!   scaled and converted, byte for byte but for the X bytes (upstream
//!   converts into surfaces it leaves uninitialized, and `SDL_ConvertPixels()`
//!   doesn't write the X byte of XRGB8888, so upstream's are whatever the
//!   memory held; here the surfaces start zeroed, and they are 0);
//! * `rate`: a frame rate without a size (dropped from the constraints,
//!   kept in the format); a fractional frame rate (an integer in SDL); a
//!   stream without one (0/1, and one frame only: the loop's next time is
//!   NaN), opened a display frame after the one before was closed;
//! * `pending`: a camera closed while the browser asks: the answer comes
//!   to nothing (no event, no loop; its stream isn't stopped), and the
//!   camera opens again;
//! * `real`: the runner's fake camera through the real `getUserMedia()`
//!   (allowed by the runner's fake permission prompt): approved, RGBA32 at
//!   the stream's size and frame rate, three frames of that size, opaque,
//!   not all black, their timestamps increasing; closing stops its track;
//! * `quit`: `SDL_Quit()` with a streaming camera stops it and drops the
//!   driver's state and loop; initialized again, the camera is back.
//!
//! The translation differs from upstream in three places, all marked
//! Note (upstream) in `sdl3::camera`'s Emscripten driver, and the C
//! reference is patched to match the first and the third: closing the
//! camera closes it (upstream's `OpenDevice` never sets `device->hidden`,
//! so closing does nothing at all), the surfaces a requested format had
//! prepared at open aren't prepared again over them (an assertion in a
//! debug build, a leak in upstream's release one, which this trace can't
//! see), and the browser's answer for a camera closed meanwhile is refused.
//!
//! Each step prints a libtest-like `test NAME ... ok` line (with both
//! traces when they differ); under node everything skips as `browser`.
//! `SDL3_EMSCRIPTEN_BROWSER=1 SDL3_TEST_REQUIRE=browser cargo test -p sdl3
//! --target wasm32-unknown-emscripten --test emscripten_camera` runs it as
//! CI does.

#[cfg(not(target_os = "emscripten"))]
fn main() {}

#[cfg(target_os = "emscripten")]
fn main() -> std::process::ExitCode {
    webcamera::main()
}

/// The program's request for the browser (see `emscripten_browser.rs`).
#[cfg(target_os = "emscripten")]
#[used]
#[link_section = "sdl3_runner"]
static RUNNER: [u8; 7] = *b"browser";

#[cfg(target_os = "emscripten")]
mod webcamera {
    use std::fmt::Write as _;
    use std::panic::{self, AssertUnwindSafe};
    use std::process::ExitCode;
    use std::sync::Mutex;

    use sdl3::app::{self, AppCallbacks, AppResult};
    use sdl3::camera::{self, Camera, CameraFrame, CameraID, CameraPermissionState, CameraSpec};
    use sdl3::events::{self, Event, EventType};
    use sdl3::init::{self, InitFlags};
    use sdl3::video::surface::PROP_SURFACE_ROTATION_FLOAT;
    use sdl3::video::{Colorspace, PixelFormat};
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

    // -- JavaScript helpers ----------------------------------------------------

    /// The value of the JavaScript expression `expr` (evaluated in the
    /// glue's scope), as a string.
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

    fn js_int(expr: &str) -> i32 {
        js(expr).trim().parse().unwrap_or(0)
    }

    /// Run JavaScript statements.
    fn js_run(code: &str) {
        js(&format!("(function() {{ {code} }})()"));
    }

    // The snippets the C reference runs too, verbatim.

    /// The stand-ins (see the module's documentation), as `Module.cam`
    /// (`C`): `C.log`, what the page saw (`getUserMedia` and the
    /// constraints, `play`, `stop WxH` for a stream's track, and the
    /// driver's console lines); `C.mode`, `'mock'` or `'real'` (the
    /// browser's own `getUserMedia()` and video element); `C.freeze()`, the
    /// clock stopped at a whole number of milliseconds, later than the real
    /// one (`C.now`, moved by the program; `undefined` for the real clock);
    /// `C.resolve(w, h, fps)` and `C.reject(name, message)`, the answer to
    /// the oldest `getUserMedia()` still waiting; `C.meta()`, the video's
    /// `loadedmetadata`; `C.seed` and `C.fill(canvas)`, the pixels;
    /// `C.draws`, the frames drawn from a stand-in video (`C.realDraws`,
    /// from a real one).
    const MOCK_INSTALL: &str = "var C = Module.cam = { log: [], mode: 'mock', pending: [], draws: 0, realDraws: 0, w: 4, h: 2, seed: 1, now: undefined }; \
C.realNow = performance.now.bind(performance); C.realDate = Date.now.bind(Date); C.offset = 0; C.dateOffset = 0; C.date0 = 0; \
performance.now = function() { return C.now === undefined ? C.realNow() + C.offset : C.now; }; \
Date.now = function() { return C.now === undefined ? C.realDate() + C.dateOffset : C.date0 + C.now; }; \
C.freeze = function() { var n = performance.now(), d = Date.now(); C.now = Math.ceil(n) + 1; C.date0 = Math.ceil(d) + 1 - C.now; }; \
C.unfreeze = function() { var n = C.now, d = C.date0 + C.now; C.now = undefined; C.offset = Math.max(0, n - C.realNow()); C.dateOffset = Math.max(0, d - C.realDate()); }; \
C.fill = function(cv) { var x = cv.getContext('2d'); var img = x.createImageData(C.w, C.h); for (var i = 0; i < img.data.length; i++) img.data[i] = (i % 4 == 3) ? 255 : ((C.seed * 37 + i * 11) & 255); x.putImageData(img, 0, 0); }; \
C.realCreate = document.createElement.bind(document); \
document.createElement = function(tag, options) { if (tag !== 'video' || C.mode !== 'mock') return C.realCreate(tag, options); \
var v = C.realCreate('canvas'); v.width = C.w; v.height = C.h; C.fill(v); \
Object.defineProperty(v, 'width', { value: C.w, writable: true }); Object.defineProperty(v, 'height', { value: C.h, writable: true }); \
v.sdl3_fake = true; v.play = function() { C.log.push('play'); return Promise.resolve(); }; C.video = v; return v; }; \
C.meta = function() { C.video.dispatchEvent(new Event('loadedmetadata')); }; \
C.stream = function(w, h, fps) { var t = { kind: 'video', readyState: 'live', \
getSettings: function() { var s = { width: w, height: h }; if (fps !== undefined) s.frameRate = fps; return s; }, \
stop: function() { t.readyState = 'ended'; C.log.push('stop ' + w + 'x' + h); } }; \
return { getVideoTracks: function() { return [t]; }, getTracks: function() { return [t]; }, track: t }; }; \
C.resolve = function(w, h, fps) { C.w = w; C.h = h; C.lastStream = C.stream(w, h, fps); C.pending.shift().resolve(C.lastStream); }; \
C.reject = function(name, message) { C.pending.shift().reject(new DOMException(message, name)); }; \
C.realGUM = navigator.mediaDevices.getUserMedia.bind(navigator.mediaDevices); \
navigator.mediaDevices.getUserMedia = function(c) { C.log.push('getUserMedia ' + JSON.stringify(c)); if (C.mode !== 'mock') return C.realGUM(c); \
return new Promise(function(resolve, reject) { C.pending.push({ resolve: resolve, reject: reject }); }); }; \
var drawImage = CanvasRenderingContext2D.prototype.drawImage; \
CanvasRenderingContext2D.prototype.drawImage = function(img) { if (img && img.sdl3_fake) C.draws++; else if (img instanceof HTMLVideoElement) C.realDraws++; return drawImage.apply(this, arguments); }; \
var log = console.log, error = console.error; \
console.log = function(m) { if (String(m).indexOf('Camera is opened!') === 0) C.log.push('log ' + m); return log.apply(console, arguments); }; \
console.error = function(m) { if (String(m).indexOf('Tried to open camera') === 0) C.log.push('error ' + m); return error.apply(console, arguments); };";

    /// What the page saw since the last time (and forget it).
    const CAM_LOG: &str =
        "(function() { var l = Module.cam.log.join(' | '); Module.cam.log = []; return l; })()";

    /// The driver's state, `Module['SDL3'].camera`.
    const CAM_STATE: &str = "(function() { var c = Module['SDL3'].camera; if (c === undefined) return 'undefined'; var k = Object.keys(c); if (k.length == 0) return '{}'; \
return k.join(',') + '; ' + c.width + 'x' + c.height + ' fps ' + c.fps + ' every ' + c.fpsincrms + ' ms, due in ' + (c.next_frame_time - performance.now()) + \
'; video ' + c.video.width + 'x' + c.video.height + ' ' + c.video.style.display + ' playing the stream ' + (c.video.srcObject === Module.cam.lastStream) + \
'; canvas ' + c.canvas.width + 'x' + c.canvas.height + ' ' + c.canvas.style.display + ' context ' + (c.ctx2d.canvas === c.canvas); })()";

    /// How long until the loop's next frame is due (in milliseconds).
    const CAM_DUE: &str = "String(Module['SDL3'].camera.next_frame_time - performance.now())";

    /// Whether the driver's console line for the browser's own camera is
    /// the one its state says (and forget what the page saw).
    const CAM_REAL_LOG: &str = "(function() { var C = Module.cam; var c = Module['SDL3'].camera; var want = 'log Camera is opened! Actual spec: (' + c.width + 'x' + c.height + '), fps=' + c.fps; \
var l = C.log.join(' | '); C.log = []; return String(l === want); })()";

    /// Whether the driver's state is the browser's stream's settings.
    const CAM_REAL_STATE: &str = "(function() { var c = Module['SDL3'].camera; var s = c.stream.getVideoTracks()[0].getSettings(); \
return 'size ' + (c.width === s.width && c.height === s.height) + ' fps ' + (c.fps === s.frameRate); })()";

    // -- the trace -------------------------------------------------------------

    static TRACE: Mutex<String> = Mutex::new(String::new());

    /// A line of the trace.
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

    /// Compare the trace with upstream's, step by step; the result.
    fn finish() -> bool {
        let trace = TRACE.lock().unwrap().clone();
        let expected = sections(EXPECTED);
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
                println!("test {name} ... FAILED (not in upstream's trace)\n{body}");
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

    // -- SDL -------------------------------------------------------------------

    /// The camera IDs seen, in order: `#1` is the first.
    static SEEN: Mutex<Vec<CameraID>> = Mutex::new(Vec::new());

    fn tag(id: CameraID) -> usize {
        if id == 0 {
            return 0;
        }
        let mut seen = SEEN.lock().unwrap();
        if let Some(i) = seen.iter().position(|&s| s == id) {
            return i + 1;
        }
        seen.push(id);
        seen.len()
    }

    /// The camera events, as the main callbacks got them and then the
    /// queue's.
    static EVENTS: Mutex<String> = Mutex::new(String::new());

    fn record_event(event: &Event) {
        let Event::CameraDevice(e) = event else {
            return;
        };
        let what = match e.event_type {
            EventType::CAMERA_DEVICE_ADDED => "added",
            EventType::CAMERA_DEVICE_REMOVED => "removed",
            EventType::CAMERA_DEVICE_APPROVED => "approved",
            EventType::CAMERA_DEVICE_DENIED => "denied",
            _ => return,
        };
        let one = format!(" {what} #{}", tag(e.which));
        EVENTS.lock().unwrap().push_str(&one);
    }

    fn events_line() {
        events::pump();
        for e in events::get_events(
            EventType::CAMERA_DEVICE_ADDED,
            EventType::CAMERA_DEVICE_DENIED,
            32,
        )
        .unwrap_or_default()
        {
            record_event(&e);
        }
        let mut ev = EVENTS.lock().unwrap();
        out!("events:{}", if ev.is_empty() { " none" } else { &ev });
        ev.clear();
    }

    fn jslog() {
        let l = js(CAM_LOG);
        out!("log: {}", if l.is_empty() { "(nothing)" } else { &l });
    }

    fn table(label: &str) {
        out!("{label}: {}", js(CAM_STATE));
    }

    fn fmtname(f: PixelFormat) -> &'static str {
        &f.name()["SDL_PIXELFORMAT_".len()..]
    }

    fn format_line(cam: &Camera) {
        match cam.format() {
            Ok(spec) => out!(
                "format {} {}x{} {}/{} colorspace 0x{:x}",
                fmtname(spec.format),
                spec.width,
                spec.height,
                spec.framerate_numerator,
                spec.framerate_denominator,
                spec.colorspace.0
            ),
            Err(e) => out!("format: err {}", e.message()),
        }
    }

    fn acquire_err(cam: &Camera) {
        match cam.acquire_frame() {
            Ok(Some(_)) => out!("acquire: a frame?!"),
            Ok(None) => out!("acquire: err (none)"),
            Err(e) => out!("acquire: err {}", e.message()),
        }
    }

    fn pattern_ok(f: &CameraFrame, seed: i32) -> i32 {
        let p = f.pixels().unwrap();
        let (w, h, pitch) = (f.width() as usize, f.height() as usize, f.pitch() as usize);
        for y in 0..h {
            for i in 0..w * 4 {
                let k = (y * w * 4 + i) as i32;
                let want = if k % 4 == 3 {
                    255
                } else {
                    (seed * 37 + k * 11) & 255
                };
                if p[y * pitch + i] as i32 != want {
                    return 0;
                }
            }
        }
        1
    }

    fn frame_line(f: &mut CameraFrame) {
        let rot = f
            .properties()
            .get_float(PROP_SURFACE_ROTATION_FLOAT)
            .unwrap_or(-1.0);
        out!(
            "frame {}x{} {} pitch {} colorspace 0x{:x} rotation {}",
            f.width(),
            f.height(),
            fmtname(f.format()),
            f.pitch(),
            f.colorspace().0,
            rot
        );
    }

    /// The frame's bytes, but for the X bytes of an XRGB8888 frame (see the
    /// module's documentation).
    fn hex_line(f: &CameraFrame) {
        let p = f.pixels().unwrap();
        let mut s = String::from("bytes");
        for y in 0..f.height() as usize {
            for i in 0..f.width() as usize * 4 {
                if f.format() == PixelFormat::XRGB8888 && i % 4 == 3 {
                    s.push_str(" --");
                } else {
                    write!(s, " {:02x}", p[y * f.pitch() as usize + i]).unwrap();
                }
            }
        }
        out!("{s}");
    }

    fn draws() -> i32 {
        js_int("String(Module.cam.draws)")
    }

    fn ok(b: bool) -> i32 {
        b as i32
    }

    fn spec_of(format: PixelFormat, width: i32, height: i32, num: i32, den: i32) -> CameraSpec {
        CameraSpec {
            format,
            colorspace: Colorspace::UNKNOWN,
            width,
            height,
            framerate_numerator: num,
            framerate_denominator: den,
        }
    }

    fn open_cam(spec: Option<&CameraSpec>) -> Option<Camera> {
        let id = camera::cameras()
            .unwrap_or_default()
            .first()
            .copied()
            .unwrap_or(0);
        let c = Camera::open(id, spec).ok();
        out!(
            "open {} permission {}",
            ok(c.is_some()),
            c.as_ref().map_or(-9, |c| c.permission_state() as i32)
        );
        c
    }

    fn list() {
        let ids = camera::cameras().unwrap_or_default();
        let mut s = format!("cameras {}:", ids.len());
        for &id in &ids {
            let nspecs = camera::camera_supported_formats(id).map_or(-1, |s| s.len() as i32);
            write!(
                s,
                " #{} '{}' position {} formats {}",
                tag(id),
                camera::camera_name(id).unwrap_or_default(),
                camera::camera_position(id) as i32,
                nspecs
            )
            .unwrap();
        }
        out!("{s}");
    }

    fn init_line() {
        let ok_ = init::init(InitFlags::CAMERA).is_ok();
        out!(
            "init {} driver {}",
            ok(ok_),
            camera::current_camera_driver().unwrap_or("(null)")
        );
    }

    struct State {
        phase: i32,
        waited: i32,
        k: i32,
        base: i32,
        last_ts: u64,
        cam: Option<Camera>,
        real_frames: i32,
        real_ok: bool,
        real_opaque: bool,
        real_nonzero: bool,
        real_increase: bool,
    }

    impl State {
        fn next(&mut self) {
            self.phase += 1;
            self.waited = 0;
        }

        fn timed_out(&mut self, what: &str, limit: i32) {
            self.waited += 1;
            if self.waited > limit {
                out!("TIMEOUT {what}");
                self.next();
            }
        }

        fn cam(&self) -> &Camera {
            self.cam.as_ref().expect("an open camera")
        }

        fn permission(&self) -> CameraPermissionState {
            self.cam().permission_state()
        }

        fn frame(&self) -> Option<CameraFrame> {
            self.cam().acquire_frame().ok().flatten()
        }

        fn close_cam(&mut self) {
            self.cam = None;
        }

        /// One iteration of the main callbacks; `Some` once it's over.
        fn iterate(&mut self) -> Option<AppResult> {
            match self.phase {
                0 => {
                    step("init");
                    js_run(MOCK_INSTALL);
                    let names: Vec<&str> = (0..camera::num_camera_drivers())
                        .map(|i| camera::camera_driver(i).unwrap())
                        .collect();
                    out!("drivers {}", names.join(","));
                    init_line();
                    table("table");
                    list();
                    events_line();
                    jslog();
                    self.next();
                }
                1 => {
                    step("deny");
                    self.cam = open_cam(None);
                    format_line(self.cam());
                    acquire_err(self.cam());
                    let id = camera::cameras().unwrap()[0];
                    match Camera::open(id, None) {
                        Ok(_) => out!("open again: opened?!"),
                        Err(e) => out!("open again: {}", e.message()),
                    }
                    jslog();
                    js_run("Module.cam.reject('NotAllowedError', 'Permission denied');");
                    self.next();
                }
                2 => {
                    if self.permission() != CameraPermissionState::Pending {
                        events_line();
                        out!("permission {}", self.permission() as i32);
                        format_line(self.cam());
                        acquire_err(self.cam());
                        jslog();
                        table("table");
                        self.close_cam();
                        table("after close");
                        self.next();
                    } else {
                        self.timed_out("denial", 300);
                    }
                }
                3 => {
                    step("approve");
                    js_run("Module.cam.freeze();");
                    self.cam = open_cam(None);
                    jslog();
                    js_run("Module.cam.resolve(4, 2, 25);");
                    self.next();
                }
                4 => {
                    if self.permission() == CameraPermissionState::Approved {
                        events_line();
                        out!("permission {}", self.permission() as i32);
                        format_line(self.cam());
                        jslog();
                        table("table");
                        let f = self.frame();
                        out!(
                            "before loadedmetadata: frame {} draws {}",
                            if f.is_some() { "yes?!" } else { "none" },
                            draws()
                        );
                        drop(f);
                        js_run("Module.cam.meta(); Module.cam.now += 1;");
                        let f = self.frame();
                        out!(
                            "at loadedmetadata: frame {} draws {}",
                            if f.is_some() { "yes?!" } else { "none" },
                            draws()
                        );
                        drop(f);
                        self.next();
                    } else {
                        self.timed_out("approval", 300);
                    }
                }
                5 => {
                    if let Some(mut f) = self.frame() {
                        frame_line(&mut f);
                        let ts = f.timestamp_ns();
                        out!(
                            "pattern 1: {} timestamp is now: {} draws {}",
                            pattern_ok(&f, 1),
                            ok(ts == sdl3::timer::ticks_ns()),
                            draws()
                        );
                        self.last_ts = ts;
                        drop(f);
                        self.next();
                    } else {
                        self.timed_out("first frame", 300);
                    }
                }
                6 => {
                    self.waited += 1;
                    if self.waited >= 5 {
                        let f = self.frame();
                        out!(
                            "5 display frames later: frame {} draws {}",
                            if f.is_some() { "yes?!" } else { "none" },
                            draws()
                        );
                        drop(f);
                        self.next();
                    }
                }
                7 => {
                    step("frames");
                    js_run("Module.cam.seed = 2; Module.cam.fill(Module.cam.video); Module.cam.now += 40;");
                    self.next();
                }
                8 => {
                    if let Some(f) = self.frame() {
                        let ts = f.timestamp_ns();
                        out!(
                            "40 ms later: pattern 2: {} timestamp +{} draws {}",
                            pattern_ok(&f, 2),
                            ts.wrapping_sub(self.last_ts),
                            draws()
                        );
                        self.last_ts = ts;
                        drop(f);
                        js_run("Module.cam.now += 100;");
                        self.next();
                    } else {
                        self.timed_out("frame after 40 ms", 300);
                    }
                }
                9 => {
                    if let Some(f) = self.frame() {
                        let ts = f.timestamp_ns();
                        out!(
                            "100 ms later: timestamp +{} draws {} next due in {}",
                            ts.wrapping_sub(self.last_ts),
                            draws(),
                            js(CAM_DUE)
                        );
                        self.last_ts = ts;
                        drop(f);
                        js_run("Module.cam.now += 10;");
                        self.next();
                    } else {
                        self.timed_out("frame after 100 ms", 300);
                    }
                }
                10 => {
                    self.waited += 1;
                    if self.waited >= 5 {
                        let f = self.frame();
                        out!(
                            "10 ms later: frame {} draws {} next due in {}",
                            if f.is_some() { "yes?!" } else { "none" },
                            draws(),
                            js(CAM_DUE)
                        );
                        drop(f);
                        js_run("Module.cam.now += 10;");
                        self.next();
                    }
                }
                11 => {
                    if let Some(f) = self.frame() {
                        let ts = f.timestamp_ns();
                        out!(
                            "10 ms later: timestamp +{} draws {} next due in {}",
                            ts.wrapping_sub(self.last_ts),
                            draws(),
                            js(CAM_DUE)
                        );
                        self.last_ts = ts;
                        drop(f);
                        self.k = 0;
                        self.base = draws();
                        self.next();
                    } else {
                        self.timed_out("frame after 20 ms", 300);
                    }
                }
                12 => {
                    // nine frames, 40 ms apart, none of them taken
                    if draws() == self.base + self.k {
                        if self.k == 9 {
                            self.next();
                        } else {
                            js_run("Module.cam.now += 40;");
                            self.k += 1;
                            self.waited = 0;
                        }
                    } else {
                        self.timed_out("nine frames", 300);
                    }
                }
                13 => {
                    let mut got = 0;
                    let mut steady = true;
                    while let Some(f) = self.frame() {
                        let ts = f.timestamp_ns();
                        if ts.wrapping_sub(self.last_ts) != 40_000_000 {
                            steady = false;
                        }
                        self.last_ts = ts;
                        got += 1;
                        drop(f);
                    }
                    out!(
                        "nine frames not taken: draws {}, then {} frames, each +40 ms: {}",
                        draws() - self.base,
                        got,
                        ok(steady)
                    );
                    js_run("Module.cam.now += 40;");
                    self.next();
                }
                14 => {
                    if let Some(f) = self.frame() {
                        out!(
                            "40 ms later: timestamp +{} (the ninth dropped)",
                            f.timestamp_ns().wrapping_sub(self.last_ts)
                        );
                        drop(f);
                        self.next();
                    } else {
                        self.timed_out("frame after the queue", 300);
                    }
                }
                15 => {
                    step("close");
                    self.base = draws();
                    self.close_cam();
                    jslog();
                    table("table");
                    js_run("Module.cam.now += 100;");
                    self.next();
                }
                16 => {
                    self.waited += 1;
                    if self.waited >= 5 {
                        out!("100 ms later: draws +{}", draws() - self.base);
                        self.next();
                    }
                }
                17 => {
                    step("spec");
                    let spec = spec_of(PixelFormat::XRGB8888, 2, 1, 25, 1);
                    self.cam = open_cam(Some(&spec));
                    jslog();
                    js_run("Module.cam.seed = 3; Module.cam.resolve(4, 2, 30);");
                    self.next();
                }
                18 => {
                    if self.permission() == CameraPermissionState::Approved {
                        events_line();
                        format_line(self.cam());
                        jslog();
                        table("table");
                        js_run("Module.cam.meta(); Module.cam.now += 1;");
                        self.next();
                    } else {
                        self.timed_out("approval", 300);
                    }
                }
                19 => {
                    if let Some(mut f) = self.frame() {
                        frame_line(&mut f);
                        hex_line(&f);
                        drop(f);
                        self.close_cam();
                        jslog();
                        self.next();
                    } else {
                        self.timed_out("scaled frame", 300);
                    }
                }
                20 => {
                    step("rate");
                    let spec = spec_of(PixelFormat::UNKNOWN, 0, 0, 30, 1);
                    self.cam = open_cam(Some(&spec));
                    jslog();
                    js_run("Module.cam.resolve(2, 2, 29.97);");
                    self.next();
                }
                23 if self.cam.is_none() => {
                    // (a display frame after closing: see the driver's
                    // Note (upstream) on the loop of a camera closed and
                    // opened again)
                    self.cam = open_cam(None);
                    js_run("Module.cam.resolve(2, 2);");
                }
                21 | 23 => {
                    if self.permission() == CameraPermissionState::Approved {
                        events_line();
                        format_line(self.cam());
                        jslog();
                        table("table");
                        js_run("Module.cam.meta(); Module.cam.now += 1;");
                        self.next();
                    } else {
                        self.timed_out("approval", 300);
                    }
                }
                22 => {
                    if let Some(mut f) = self.frame() {
                        frame_line(&mut f);
                        out!("pattern 3: {}", pattern_ok(&f, 3));
                        drop(f);
                        self.close_cam();
                        jslog();
                        self.next();
                    } else {
                        self.timed_out("frame", 300);
                    }
                }
                24 => {
                    if let Some(mut f) = self.frame() {
                        frame_line(&mut f);
                        drop(f);
                        self.base = draws();
                        js_run("Module.cam.now += 200;");
                        self.next();
                    } else {
                        self.timed_out("frame", 300);
                    }
                }
                25 => {
                    self.waited += 1;
                    if self.waited >= 5 {
                        let f = self.frame();
                        out!(
                            "200 ms later: frame {} draws +{}",
                            if f.is_some() { "yes?!" } else { "none" },
                            draws() - self.base
                        );
                        drop(f);
                        self.close_cam();
                        jslog();
                        self.next();
                    }
                }
                26 => {
                    step("pending");
                    self.cam = open_cam(None);
                    self.close_cam();
                    jslog();
                    table("table");
                    js_run("Module.cam.resolve(4, 2, 25);");
                    self.next();
                }
                27 => {
                    self.waited += 1;
                    if self.waited >= 5 {
                        events_line();
                        jslog();
                        table("table");
                        out!(
                            "the stream's track: {}",
                            js("Module.cam.lastStream.track.readyState")
                        );
                        self.cam = open_cam(None);
                        js_run("Module.cam.resolve(4, 2, 25);");
                        self.next();
                    }
                }
                28 => {
                    if self.permission() == CameraPermissionState::Approved {
                        events_line();
                        format_line(self.cam());
                        jslog();
                        self.close_cam();
                        jslog();
                        self.next();
                    } else {
                        self.timed_out("approval", 300);
                    }
                }
                29 => {
                    step("real");
                    js_run("Module.cam.unfreeze(); Module.cam.mode = 'real';");
                    self.cam = open_cam(None);
                    jslog();
                    self.next();
                }
                30 => {
                    if self.permission() != CameraPermissionState::Pending {
                        events_line();
                        out!("permission {}", self.permission() as i32);
                        let spec = self.cam().format().unwrap_or_default();
                        out!(
                            "format {}, size positive {} the stream's {}, fps positive {} the stream's {}",
                            fmtname(spec.format),
                            ok(spec.width > 0 && spec.height > 0),
                            ok(spec.width == js_int("String(Module['SDL3'].camera.width)")
                                && spec.height == js_int("String(Module['SDL3'].camera.height)")),
                            ok(spec.framerate_numerator > 0),
                            ok(spec.framerate_numerator
                                == js_int("String(Math.trunc(Module['SDL3'].camera.fps))")
                                && spec.framerate_denominator == 1)
                        );
                        out!(
                            "the console's line: {}, the settings': {}",
                            js(CAM_REAL_LOG),
                            js(CAM_REAL_STATE)
                        );
                        self.next();
                    } else {
                        self.timed_out("real approval", 1200);
                    }
                }
                31 => {
                    if let Some(f) = self.frame() {
                        let spec = self.cam().format().unwrap_or_default();
                        if self.real_frames == 0 {
                            self.real_ok = true;
                            self.real_opaque = true;
                            self.real_increase = true;
                        }
                        if f.width() != spec.width
                            || f.height() != spec.height
                            || f.format() != spec.format
                            || f.pitch() != f.width() * 4
                        {
                            self.real_ok = false;
                        }
                        let p = f.pixels().unwrap();
                        for y in 0..f.height() as usize {
                            for x in 0..f.width() as usize {
                                let q = &p[y * f.pitch() as usize + x * 4..][..4];
                                if q[3] != 255 {
                                    self.real_opaque = false;
                                }
                                if q[0] != 0 || q[1] != 0 || q[2] != 0 {
                                    self.real_nonzero = true;
                                }
                            }
                        }
                        let ts = f.timestamp_ns();
                        if self.real_frames > 0 && ts <= self.last_ts {
                            self.real_increase = false;
                        }
                        self.last_ts = ts;
                        drop(f);
                        self.real_frames += 1;
                        if self.real_frames == 3 {
                            out!(
                                "3 frames: size and format {} opaque {} some color {} timestamps increase {}",
                                ok(self.real_ok),
                                ok(self.real_opaque),
                                ok(self.real_nonzero),
                                ok(self.real_increase)
                            );
                            js_run("Module.cam.realStream = Module['SDL3'].camera.stream;");
                            self.close_cam();
                            table("table");
                            out!(
                                "the stream's track: {}",
                                js("Module.cam.realStream.getVideoTracks()[0].readyState")
                            );
                            self.next();
                        }
                    } else {
                        self.timed_out("real frames", 1200);
                    }
                }
                32 => {
                    step("quit");
                    js_run("Module.cam.mode = 'mock'; Module.cam.freeze();");
                    self.cam = open_cam(None);
                    js_run("Module.cam.resolve(4, 2, 25);");
                    self.next();
                }
                33 => {
                    if self.permission() == CameraPermissionState::Approved {
                        events_line();
                        jslog();
                        js_run("Module.cam.meta(); Module.cam.now += 1;");
                        self.next();
                    } else {
                        self.timed_out("approval", 300);
                    }
                }
                34 => {
                    if let Some(f) = self.frame() {
                        drop(f);
                        out!("a frame");
                        self.base = draws();
                        init::quit();
                        // (closed by quit(); the handle is dropped after)
                        self.cam = None;
                        jslog();
                        table("after quit");
                        js_run("Module.cam.now += 100;");
                        self.next();
                    } else {
                        self.timed_out("frame", 300);
                    }
                }
                35 => {
                    self.waited += 1;
                    if self.waited >= 5 {
                        out!("100 ms later: draws +{}", draws() - self.base);
                        init_line();
                        table("table");
                        list();
                        self.next();
                    }
                }
                36 => {
                    events_line();
                    init::quit();
                    table("after quit");
                    self.next();
                }
                _ => {
                    return Some(if finish() {
                        AppResult::Success
                    } else {
                        AppResult::Failure
                    });
                }
            }
            None
        }
    }

    pub(super) fn main() -> ExitCode {
        if !in_browser() {
            let capability = "browser";
            if required(capability) {
                panic!("required capability {capability} unavailable: not in a browser");
            }
            println!(
                "note: skipping, {capability} unavailable: not in a browser (under node: run it with SDL3_EMSCRIPTEN_BROWSER=1, or with CHROME naming Chrome or Chromium)"
            );
            let steps = sections(EXPECTED);
            for (name, _) in &steps {
                println!("test {name} ... ok");
            }
            println!("\ntest result: ok. {} passed; 0 failed", steps.len());
            return ExitCode::SUCCESS;
        }
        ExitCode::from(app::run_app(sdl_main) as u8)
    }

    fn sdl_main(args: &[String]) -> i32 {
        app::enter_app_main_callbacks(args, move |_| {
            (
                App {
                    st: State {
                        phase: 0,
                        waited: 0,
                        k: 0,
                        base: 0,
                        last_ts: 0,
                        cam: None,
                        real_frames: 0,
                        real_ok: false,
                        real_opaque: false,
                        real_nonzero: false,
                        real_increase: false,
                    },
                },
                AppResult::Continue,
            )
        })
    }

    // -- the main callbacks ----------------------------------------------------

    struct App {
        st: State,
    }

    /// The phase past the last one, where the trace is compared.
    const END: i32 = 37;

    impl AppCallbacks for App {
        fn iterate(&mut self) -> AppResult {
            match panic::catch_unwind(AssertUnwindSafe(|| self.st.iterate())) {
                Ok(Some(result)) => result,
                Ok(None) => AppResult::Continue,
                Err(_) => {
                    out!("FAILED: phase {} panicked", self.st.phase);
                    if self.st.phase >= END {
                        // (in the end itself)
                        return AppResult::Failure;
                    }
                    self.st.phase = END;
                    AppResult::Continue
                }
            }
        }

        fn event(&mut self, event: &Event) -> AppResult {
            record_event(event);
            AppResult::Continue
        }

        fn quit(&mut self, _result: AppResult) {
            self.st.cam = None;
            init::quit();
        }
    }

    /// The trace of upstream's C on the same page with the same steps.
    const EXPECTED: &str = r#"== init
drivers emscripten,dummy
init 1 driver emscripten
table: {}
cameras 1: #1 'Web browser's camera' position 0 formats 0
events: added #1
log: (nothing)
== deny
open 1 permission 0
format: err Camera permission has not been granted
acquire: err Camera permission has not been granted
open again: Camera already opened
log: getUserMedia {"video":true}
events: denied #1
permission -1
format: err Camera permission has not been granted
acquire: err Camera permission has not been granted
log: error Tried to open camera but it threw an error! NotAllowedError: Permission denied
table: {}
after close: {}
== approve
open 1 permission 0
log: getUserMedia {"video":true}
events: approved #1
permission 1
format ABGR8888 4x2 25/1 colorspace 0x0
log: log Camera is opened! Actual spec: (4x2), fps=25 | play
table: width,height,fps,fpsincrms,stream,video,canvas,ctx2d,next_frame_time; 4x2 fps 25 every 40 ms, due in 0; video 4x2 none playing the stream true; canvas 4x2 none context true
before loadedmetadata: frame none draws 0
at loadedmetadata: frame none draws 1
frame 4x2 ABGR8888 pitch 16 colorspace 0x0 rotation 0
pattern 1: 1 timestamp is now: 1 draws 2
5 display frames later: frame none draws 2
== frames
40 ms later: pattern 2: 1 timestamp +40000000 draws 3
100 ms later: timestamp +100000000 draws 4 next due in 19
10 ms later: frame none draws 4 next due in 9
10 ms later: timestamp +20000000 draws 5 next due in 39
nine frames not taken: draws 9, then 8 frames, each +40 ms: 1
40 ms later: timestamp +80000000 (the ninth dropped)
== close
log: stop 4x2
table: {}
100 ms later: draws +0
== spec
open 1 permission 0
log: getUserMedia {"video":{"width":2,"height":1,"frameRate":{"ideal":25}}}
events: approved #1
format XRGB8888 2x1 25/1 colorspace 0x0
log: log Camera is opened! Actual spec: (4x2), fps=30 | play
table: width,height,fps,fpsincrms,stream,video,canvas,ctx2d,next_frame_time; 4x2 fps 30 every 33.333333333333336 ms, due in 0; video 4x2 none playing the stream true; canvas 4x2 none context true
frame 2x1 XRGB8888 pitch 8 colorspace 0x0 rotation 0
bytes 61 56 4b -- b9 ae a3 --
log: stop 4x2
== rate
open 1 permission 0
log: getUserMedia {"video":true}
events: approved #1
format ABGR8888 2x2 30/1 colorspace 0x0
log: log Camera is opened! Actual spec: (2x2), fps=29.97 | play
table: width,height,fps,fpsincrms,stream,video,canvas,ctx2d,next_frame_time; 2x2 fps 29.97 every 33.366700033366705 ms, due in 0; video 2x2 none playing the stream true; canvas 2x2 none context true
frame 2x2 ABGR8888 pitch 8 colorspace 0x0 rotation 0
pattern 3: 1
log: stop 2x2
open 1 permission 0
events: approved #1
format ABGR8888 2x2 0/1 colorspace 0x0
log: getUserMedia {"video":true} | log Camera is opened! Actual spec: (2x2), fps=undefined | play
table: width,height,fps,fpsincrms,stream,video,canvas,ctx2d,next_frame_time; 2x2 fps undefined every NaN ms, due in 0; video 2x2 none playing the stream true; canvas 2x2 none context true
frame 2x2 ABGR8888 pitch 8 colorspace 0x0 rotation 0
200 ms later: frame none draws +0
log: stop 2x2
== pending
open 1 permission 0
log: getUserMedia {"video":true}
table: {}
events: none
log: log Camera is opened! Actual spec: (4x2), fps=25
table: {}
the stream's track: live
open 1 permission 0
events: approved #1
format ABGR8888 4x2 25/1 colorspace 0x0
log: getUserMedia {"video":true} | log Camera is opened! Actual spec: (4x2), fps=25 | play
log: stop 4x2
== real
open 1 permission 0
log: getUserMedia {"video":true}
events: approved #1
permission 1
format ABGR8888, size positive 1 the stream's 1, fps positive 1 the stream's 1
the console's line: true, the settings': size true fps true
3 frames: size and format 1 opaque 1 some color 1 timestamps increase 1
table: {}
the stream's track: ended
== quit
open 1 permission 0
events: approved #1
log: getUserMedia {"video":true} | log Camera is opened! Actual spec: (4x2), fps=25 | play
a frame
log: stop 4x2
after quit: undefined
100 ms later: draws +0
init 1 driver emscripten
table: {}
cameras 1: #2 'Web browser's camera' position 0 formats 0
events: added #2
after quit: undefined
"#;
}
