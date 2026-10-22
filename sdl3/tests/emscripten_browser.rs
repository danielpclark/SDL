// Tests of the Emscripten backends in a real browser.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! The Emscripten paths that need a browser, in headless Chromium: what the
//! tests under node can only stub. A program of its own (`harness = false`)
//! that asks `tools/emscripten_run.py` for the browser (the custom section
//! below), where it is an application: the synchronous checks first, then
//! `run_app()` and the main callbacks, whose iterations (from
//! `requestAnimationFrame()`) wait for what the browser answers from its
//! event loop.
//!
//! * `battery`: the Battery Status API itself. The first `power_info()` has
//!   no data yet (emscripten's `navigator.getBattery()` answers through a
//!   promise, upstream's uninitialized answer); once the promise has
//!   answered, the state is what upstream's rules make of the battery the
//!   page sees (headless Chromium without a battery reports a full one on
//!   mains, `NoBattery`).
//! * `open_url`: `open_url()` through the browser's own `window.open()`,
//!   which succeeds (whether the browser shows the popup or blocks it).
//! * `devicemotion`: a `DeviceMotionEvent` dispatched on `window` reaches
//!   the sensor driver's handler, with upstream's values (those of the node
//!   test's first event); then whatever the browser sends by itself (one
//!   event without data where there are no motion sensors) is printed.
//! * `webgl`: a WebGL context through emscripten's WebGL API
//!   (`emscripten_webgl_create_context()`, on a canvas of its own), with
//!   its `GL_VERSION`, `GL_RENDERER` and `GL_SHADING_LANGUAGE_VERSION`.
//! * `webaudio`: an `AudioContext`, which must start running (the runner's
//!   autoplay policy needs no user gesture), with its sample rate.
//!
//! Each check prints a libtest-like `test NAME ... ok` line, after the
//! notes of what it skipped: without a browser (under node) everything
//! skips as `browser`, and without WebGL or Web Audio those checks skip as
//! `webgl` or `webaudio`, unless `SDL3_TEST_REQUIRE` names the capability.
//! `SDL3_EMSCRIPTEN_BROWSER=1 SDL3_TEST_REQUIRE=browser,webgl,webaudio
//! cargo test -p sdl3 --target wasm32-unknown-emscripten --test
//! emscripten_browser` (with `CHROME` naming the browser) runs it as CI
//! does: see "Emscripten (browser)" in docs/HARDWARE_TESTING.md.

#[cfg(not(target_os = "emscripten"))]
fn main() {}

#[cfg(target_os = "emscripten")]
fn main() -> std::process::ExitCode {
    browser::main()
}

/// The program's request for the browser: `tools/emscripten_run.py` runs a
/// program whose .wasm has this section in headless Chromium.
#[cfg(target_os = "emscripten")]
#[used]
#[link_section = "sdl3_runner"]
static RUNNER: [u8; 7] = *b"browser";

#[cfg(target_os = "emscripten")]
mod browser {
    use std::ffi::{c_char, CStr};
    use std::panic::{self, AssertUnwindSafe};
    use std::process::ExitCode;
    use std::time::Duration;

    use sdl3::app::{self, AppCallbacks, AppResult};
    use sdl3::events::{self, Event, SensorID};
    use sdl3::init::{self, InitFlags};
    use sdl3::power::{self, PowerInfo, PowerState};
    use sdl3::sensor::{self, Sensor};
    use sdl3::timer;
    use sdl3_emscripten::sys;
    use sdl3_emscripten::{em_asm, em_asm_double, em_asm_int};

    /// How long the checks may wait for the browser's answers.
    const PATIENCE: Duration = Duration::from_secs(10);

    /// Whether `SDL3_TEST_REQUIRE` makes `capability` mandatory.
    fn required(capability: &str) -> bool {
        std::env::var("SDL3_TEST_REQUIRE")
            .unwrap_or_default()
            .split(',')
            .map(str::trim)
            .any(|c| c == "all" || c == capability)
    }

    /// A missing capability: a note (the CI skip list reads it), or a
    /// failure when it's required.
    fn skip(capability: &str, reason: &str) {
        if required(capability) {
            panic!("required capability {capability} unavailable: {reason}");
        }
        println!("note: skipping, {capability} unavailable: {reason}");
    }

    /// The checks' results, in libtest's words.
    #[derive(Default)]
    struct Report {
        passed: u32,
        failed: Vec<&'static str>,
    }

    impl Report {
        /// Run one check (a panic fails it) and print its `test` line.
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

    const CHECKS: [&str; 5] = ["webgl", "webaudio", "open_url", "devicemotion", "battery"];

    /// Whether this is a page in a browser (not node, with or without a
    /// stand-in `window`).
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
        // As C's `return SDL_RunApp(...)` from `main`: 0 goes back to the
        // event loop, which runs the main callbacks; they end the program.
        ExitCode::from(app::run_app(sdl_main) as u8)
    }

    fn sdl_main(args: &[String]) -> i32 {
        let mut report = Report::default();
        report.run("webgl", webgl);
        // (its `test` line once it runs, in the iterations)
        let audio = match panic::catch_unwind(webaudio_start) {
            Ok(Some(())) => true,
            Ok(None) => {
                report.done("webaudio", true);
                false
            }
            Err(_) => {
                report.done("webaudio", false);
                false
            }
        };
        report.run("open_url", open_url);
        let mut motion = None;
        report.run("devicemotion", || motion = Some(devicemotion()));
        app::enter_app_main_callbacks(args, move |_| {
            let app = App {
                report,
                start: timer::ticks(),
                iterations: 0,
                audio,
                battery: Battery::Start,
                motion,
                motion_events: Vec::new(),
            };
            (app, AppResult::Continue)
        })
    }

    // -- webgl ---------------------------------------------------------------

    extern "C" {
        // (emscripten's WebGL implementation of GLES2: JavaScript)
        fn glGetString(name: u32) -> *const c_char;
    }
    const GL_VENDOR: u32 = 0x1F00;
    const GL_RENDERER: u32 = 0x1F01;
    const GL_VERSION: u32 = 0x1F02;
    const GL_SHADING_LANGUAGE_VERSION: u32 = 0x8B8C;

    fn gl_string(name: u32) -> String {
        // SAFETY: a context is current; GL strings are NUL-terminated and
        // live as long as the context.
        let s = unsafe { glGetString(name) };
        assert!(!s.is_null(), "glGetString({name:#x}) is NULL");
        // SAFETY: as above.
        unsafe { CStr::from_ptr(s) }.to_string_lossy().into_owned()
    }

    /// A WebGL context on a canvas of the check's own: WebGL 2 if the
    /// program has it (`-sMAX_WEBGL_VERSION=2`; the test programs have
    /// WebGL 1 only, which this then gets), else WebGL 1.
    fn webgl() {
        // SAFETY: plain JavaScript.
        unsafe {
            em_asm!(
                r#"
                var c = document.createElement('canvas');
                c.id = 'sdl3-test-webgl';
                c.width = 64;
                c.height = 64;
                document.body.appendChild(c);
            "#
            )
        };
        let mut attributes = std::mem::MaybeUninit::uninit();
        // SAFETY: emscripten fills in the structure.
        let mut attributes = unsafe {
            sys::emscripten_webgl_init_context_attributes(attributes.as_mut_ptr());
            attributes.assume_init()
        };
        let mut context = 0;
        for version in [2, 1] {
            attributes.majorVersion = version;
            // SAFETY: a NUL-terminated selector and initialized attributes.
            context = unsafe {
                sys::emscripten_webgl_create_context(c"#sdl3-test-webgl".as_ptr(), &attributes)
            };
            if context > 0 {
                break;
            }
        }
        if context == 0 {
            // SAFETY: plain JavaScript.
            let webgl = unsafe {
                em_asm_int!("return typeof WebGLRenderingContext === 'function' ? 1 : 0;")
            };
            let reason = if webgl != 0 {
                "no WebGL context (Chromium needs --enable-unsafe-swiftshader --use-angle=swiftshader without a GPU)"
            } else {
                "no WebGLRenderingContext"
            };
            skip("webgl", reason);
            return;
        }
        // SAFETY: the context just made.
        let made_current = unsafe { sys::emscripten_webgl_make_context_current(context) };
        assert_eq!(made_current, sys::EMSCRIPTEN_RESULT_SUCCESS);
        let version = gl_string(GL_VERSION);
        println!(
            "webgl: {version}; {} ({}); {}",
            gl_string(GL_RENDERER),
            gl_string(GL_VENDOR),
            gl_string(GL_SHADING_LANGUAGE_VERSION)
        );
        assert!(version.starts_with("OpenGL ES "), "{version}");
        assert!(version.contains("WebGL"), "{version}");
        // What's behind the browser's "WebKit WebGL" (SwiftShader, without
        // a GPU), on a context of its own.
        let mut buf = [0u8; 256];
        // SAFETY: plain JavaScript, writing at most the buffer's length.
        let len = unsafe {
            em_asm_int!(
                r#"
                var gl = document.createElement('canvas').getContext('webgl');
                var info = gl && gl.getExtension('WEBGL_debug_renderer_info');
                var s = info ? gl.getParameter(info.UNMASKED_RENDERER_WEBGL) : '';
                var bytes = new TextEncoder().encode(s).subarray(0, $1);
                HEAPU8.set(bytes, $0);
                return bytes.length;
            "#,
                buf.as_mut_ptr(),
                buf.len() as i32
            )
        };
        let unmasked = String::from_utf8_lossy(&buf[..len as usize]);
        if !unmasked.is_empty() {
            println!("webgl: the renderer is {unmasked}");
        }
        // SAFETY: the context, which nothing uses any more.
        unsafe {
            sys::emscripten_webgl_make_context_current(0);
            assert_eq!(
                sys::emscripten_webgl_destroy_context(context),
                sys::EMSCRIPTEN_RESULT_SUCCESS
            );
        }
    }

    // -- webaudio ------------------------------------------------------------

    /// An `AudioContext` (`None` if there's none: skipped). The iterations
    /// then wait for it to run.
    fn webaudio_start() -> Option<()> {
        // SAFETY: plain JavaScript.
        let rate = unsafe {
            em_asm_int!(
                r#"
                var C = globalThis.AudioContext || globalThis.webkitAudioContext;
                if (!C) return -1;
                try {
                    Module['sdl3_test_audio'] = new C();
                } catch (e) {
                    console.log('webaudio: ' + e);
                    return -2;
                }
                return Module['sdl3_test_audio'].sampleRate | 0;
            "#
            )
        };
        match rate {
            -1 => skip("webaudio", "no AudioContext"),
            -2 => skip("webaudio", "AudioContext() failed"),
            _ => {
                println!("webaudio: an AudioContext at {rate} Hz");
                assert!(rate > 0);
                return Some(());
            }
        }
        None
    }

    /// `(running, its clock in seconds)`.
    fn webaudio_state() -> (bool, f64) {
        // SAFETY: plain JavaScript.
        unsafe {
            (
                em_asm_int!("return Module['sdl3_test_audio'].state === 'running' ? 1 : 0;") != 0,
                em_asm_double!("return Module['sdl3_test_audio'].currentTime;"),
            )
        }
    }

    // -- open_url ------------------------------------------------------------

    fn open_url() {
        // The browser's own window.open(), no stand-in.
        // SAFETY: plain JavaScript.
        let native = unsafe {
            em_asm_int!(
                r#"return typeof window.open === 'function' && /\[native code\]/.test(Function.prototype.toString.call(window.open)) ? 1 : 0;"#
            )
        };
        assert_eq!(native, 1, "window.open() isn't the browser's");
        sdl3::misc::open_url("about:blank").unwrap();
        println!("open_url: about:blank opened");
    }

    // -- devicemotion --------------------------------------------------------

    /// The sensors' updates, as `(which, data)`, from the event queue.
    fn sensor_events() -> Vec<(SensorID, [f32; 3])> {
        sensor::update_sensors();
        let mut updates = Vec::new();
        while let Some(event) = events::poll() {
            if let Event::Sensor(e) = event {
                updates.push((e.which, [e.data[0], e.data[1], e.data[2]]));
            }
        }
        updates
    }

    /// The open sensors (accelerometer, gyroscope).
    struct Motion {
        accel: Sensor,
        gyro: Sensor,
    }

    /// A real `devicemotion` event on `window` reaches the driver's handler,
    /// with the values of upstream's C for the same event (as in the node
    /// test `emscripten_devicemotion_matches_upstream`).
    #[allow(clippy::excessive_precision)] // (as upstream's C printed them, with %.9g)
    fn devicemotion() -> Motion {
        init::init_subsystem(InitFlags::SENSOR).unwrap();
        let ids = sensor::sensors();
        assert_eq!(ids.len(), 2);
        let accel = Sensor::open(ids[0]).unwrap();
        let gyro = Sensor::open(ids[1]).unwrap();
        assert_eq!(accel.name().unwrap().as_deref(), Some("Accelerometer"));
        assert_eq!(gyro.name().unwrap().as_deref(), Some("Gyroscope"));
        assert_eq!(sensor_events(), []);

        // SAFETY: plain JavaScript.
        let dispatched = unsafe {
            em_asm_int!(
                r#"
                if (typeof DeviceMotionEvent !== 'function') return -1;
                window.dispatchEvent(new DeviceMotionEvent('devicemotion', {
                    acceleration: { x: 0.5, y: -1.25, z: 2 },
                    accelerationIncludingGravity: { x: 0.5, y: -1.25, z: 11.80665 },
                    rotationRate: { alpha: 90, beta: -45, gamma: 1.5 },
                    interval: 16,
                }));
                return 1;
            "#
            )
        };
        if dispatched < 0 {
            skip("browser", "no DeviceMotionEvent");
            return Motion { accel, gyro };
        }
        let bits = |d: &[f32; 3]| d.map(f32::to_bits);
        let gyro_data = [1.57079637f32, -0.785398185, 0.0261799395];
        let accel_data = [0.5f32, -1.25, 11.8066502];
        let got: Vec<_> = sensor_events()
            .iter()
            .map(|(id, d)| (*id, bits(d)))
            .collect();
        // (the most recently opened sensor first, as upstream's list)
        assert_eq!(
            got,
            [
                (gyro.id(), bits(&gyro_data)),
                (accel.id(), bits(&accel_data))
            ]
        );
        let mut data = [0f32; 3];
        accel.data(&mut data).unwrap();
        assert_eq!(bits(&data), bits(&accel_data));
        println!("devicemotion: the event reached both sensors: {gyro_data:?}, {accel_data:?}");
        Motion { accel, gyro }
    }

    // -- battery -------------------------------------------------------------

    enum Battery {
        Start,
        /// Waiting for the page's own `getBattery()` promise.
        Waiting,
        /// It answered, an iteration ago.
        Answered,
        Done,
    }

    fn info(i: PowerInfo) -> (PowerState, Option<u64>, Option<u8>) {
        (i.state, i.time_left.map(|t| t.as_secs()), i.percent)
    }

    /// What upstream's `SDL_GetPowerInfo_Emscripten()` makes of a battery.
    fn expected_power(
        level: f64,
        charging: bool,
        charging_time: f64,
        discharging_time: f64,
    ) -> (PowerState, Option<u64>, Option<u8>) {
        if level == 1.0 && charging && charging_time == 0.0 {
            return (PowerState::NoBattery, None, None);
        }
        let state = match (charging, charging_time == 0.0) {
            (true, true) => PowerState::Charged,
            (true, false) => PowerState::Charging,
            (false, _) => PowerState::OnBattery,
        };
        let seconds = discharging_time as i32;
        let percent = (level as i32) * 100;
        (
            state,
            u64::try_from(seconds).ok(),
            u8::try_from(percent).ok(),
        )
    }

    // -- the iterations ------------------------------------------------------

    struct App {
        report: Report,
        start: Duration,
        iterations: u32,
        /// The AudioContext, until it runs.
        audio: bool,
        battery: Battery,
        motion: Option<Motion>,
        /// The browser's own devicemotion events.
        motion_events: Vec<(SensorID, [f32; 6])>,
    }

    impl App {
        fn late(&self) -> bool {
            timer::ticks() - self.start > PATIENCE
        }

        fn battery(&mut self) {
            match self.battery {
                Battery::Start => {
                    // SAFETY: plain JavaScript.
                    let api = unsafe {
                        em_asm_int!("return typeof navigator.getBattery === 'function' ? 1 : 0;")
                    };
                    if api == 0 {
                        println!("battery: no Battery Status API in this browser");
                        assert_eq!(power::power_info().state, PowerState::Unknown);
                        self.battery = Battery::Done;
                        self.report.done("battery", true);
                        return;
                    }
                    // The first call asks for the battery, which isn't
                    // there yet (upstream's uninitialized answer, zeroes).
                    assert_eq!(
                        info(power::power_info()),
                        (PowerState::OnBattery, Some(0), Some(0))
                    );
                    // SAFETY: plain JavaScript.
                    unsafe {
                        em_asm!(
                            r#"
                            navigator.getBattery().then((b) => {
                                Module['sdl3_test_battery'] = [b.level, b.charging ? 1 : 0, b.chargingTime, b.dischargingTime];
                            });
                        "#
                        )
                    };
                    self.battery = Battery::Waiting;
                }
                Battery::Waiting => {
                    // SAFETY: plain JavaScript.
                    let answered =
                        unsafe { em_asm_int!("return Module['sdl3_test_battery'] ? 1 : 0;") };
                    if answered != 0 {
                        self.battery = Battery::Answered;
                    } else if self.late() {
                        println!("battery: navigator.getBattery() never answered");
                        self.battery = Battery::Done;
                        self.report.done("battery", false);
                    }
                }
                Battery::Answered => {
                    // SAFETY: plain JavaScript.
                    let field = |i: i32| unsafe {
                        em_asm_double!("return Module['sdl3_test_battery'][$0];", i)
                    };
                    let (level, charging, charging_time, discharging_time) =
                        (field(0), field(1) != 0.0, field(2), field(3));
                    let got = info(power::power_info());
                    println!(
                        "battery: level {level}, charging {charging}, chargingTime {charging_time}, dischargingTime {discharging_time}: {got:?}"
                    );
                    let expected = expected_power(level, charging, charging_time, discharging_time);
                    let ok = got == expected;
                    if !ok {
                        println!("battery: expected {expected:?}");
                    }
                    self.battery = Battery::Done;
                    self.report.done("battery", ok);
                }
                Battery::Done => {}
            }
        }

        /// The AudioContext must start running by itself (the autoplay
        /// policy): its clock moves.
        fn audio(&mut self) {
            if !self.audio {
                return;
            }
            let (running, time) = webaudio_state();
            if running && time > 0.0 {
                println!("webaudio: running, its clock at {time:.3} s");
                self.audio = false;
                self.close_audio();
                self.report.done("webaudio", true);
            } else if self.late() {
                self.audio = false;
                self.close_audio();
                let reason = format!(
                    "the AudioContext doesn't run (state running: {running}, clock at {time} s): the autoplay policy?"
                );
                let ok = panic::catch_unwind(|| skip("webaudio", &reason)).is_ok();
                self.report.done("webaudio", ok);
            }
        }

        fn close_audio(&self) {
            // SAFETY: plain JavaScript.
            unsafe {
                em_asm!("Module['sdl3_test_audio'].close(); delete Module['sdl3_test_audio'];")
            };
        }

        fn finished(&self) -> bool {
            matches!(self.battery, Battery::Done) && !self.audio
        }
    }

    impl AppCallbacks for App {
        fn iterate(&mut self) -> AppResult {
            self.iterations += 1;
            if !matches!(self.battery, Battery::Done)
                && panic::catch_unwind(AssertUnwindSafe(|| self.battery())).is_err()
            {
                self.battery = Battery::Done;
                self.report.done("battery", false);
            }
            if panic::catch_unwind(AssertUnwindSafe(|| self.audio())).is_err() {
                self.audio = false;
                self.report.done("webaudio", false);
            }
            // (a little longer for the browser's own devicemotion event)
            let elapsed = timer::ticks() - self.start;
            if self.finished() && elapsed > Duration::from_millis(500) {
                return if self.report.failed.is_empty() {
                    AppResult::Success
                } else {
                    AppResult::Failure
                };
            }
            if elapsed > PATIENCE * 2 {
                println!("the checks didn't finish in {} s", (PATIENCE * 2).as_secs());
                self.report.done("iterations", false);
                return AppResult::Failure;
            }
            AppResult::Continue
        }

        fn event(&mut self, event: &Event) -> AppResult {
            if let Event::Sensor(e) = event {
                self.motion_events.push((e.which, e.data));
            }
            AppResult::Continue
        }

        fn quit(&mut self, result: AppResult) {
            let seconds = (timer::ticks() - self.start).as_secs_f64();
            println!(
                "{} iterations in {seconds:.2} s (requestAnimationFrame())",
                self.iterations
            );
            if let Some(motion) = self.motion.take() {
                if self.motion_events.is_empty() {
                    println!("devicemotion: no events from the browser itself");
                }
                for (which, data) in &self.motion_events {
                    let sensor = if *which == motion.accel.id() {
                        "accelerometer"
                    } else if *which == motion.gyro.id() {
                        "gyroscope"
                    } else {
                        "?"
                    };
                    println!(
                        "devicemotion: from the browser itself, {sensor}: {:?}",
                        &data[..3]
                    );
                }
                drop(motion);
                init::quit_subsystem(InitFlags::SENSOR);
            }
            let ok = self.report.finish();
            if ok != (result == AppResult::Success) {
                println!("(ended with {result:?})");
            }
        }
    }
}
