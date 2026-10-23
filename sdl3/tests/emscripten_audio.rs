// Tests of the Emscripten audio driver (Web Audio) in a real browser.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! The Web Audio driver (`SDL_emscriptenaudio.c`) and the dummy driver's
//! Emscripten timers in headless Chromium: a program of its own (`harness
//! = false`) that asks `tools/emscripten_run.py` for the browser, as
//! `emscripten_gl.rs` does, where the devices' iterations run from the
//! browser's event loop (the nodes' `onaudioprocess`, the timers) while the
//! main callbacks wait for them.
//!
//! Each step writes a trace: what SDL answered (drivers, devices, formats,
//! events, errors), what the driver's JavaScript state is
//! (`Module['SDL3']`'s `audioContext`, `audio_playback` and
//! `audio_recording`), and the samples that reached Web Audio: the
//! playback node's `onaudioprocess` is wrapped to keep copies of the
//! `outputBuffer`s SDL filled, which the steps check. [`EXPECTED`] is the
//! trace of upstream's C (`libSDL3.a`, SDL 3.5.0) on the same page with the
//! same steps, and each step's part must be the same here. The values are
//! the same at any `AudioContext` rate from 22050 Hz up to 48000 Hz (the
//! rate is shown as `ctx`; the device buffers are 2048 frames there), so
//! the trace doesn't depend on the machine's audio output. The steps:
//!
//! * `init`: the drivers (Web Audio first, as upstream's), the devices
//!   (one default of each kind, with their device-added events) and their
//!   formats before they're opened; no `AudioContext` yet;
//! * `playback`: the default playback device opened, at the context's rate
//!   in floats, with its `ScriptProcessorNode`; a stream at the device's
//!   format reaches the node's output buffers sample for sample (a ramp of
//!   3000 frames, then silence);
//! * `pause`: a paused device outputs silence and leaves the stream's data,
//!   which plays once it's resumed;
//! * `convert`: an S16 mono stream at the device's rate (converted to float
//!   stereo, exactly), and an F32 mono stream at half the rate (resampled);
//! * `record`: the default recording device, its silence timer until the
//!   microphone (the runner's fake one) answers, then the microphone's
//!   samples in the bound stream;
//! * `close`: the recording device closed (the microphone's track removed
//!   from its stream, not stopped), then the playback device, which closes
//!   the context;
//! * `reopen`: a new context for a new device, which closing the device
//!   leaves open (upstream's devices both have tables once either opens);
//! * `autoplay`: the context suspended (as the browser's autoplay policy
//!   leaves it without a user gesture), so the device's silence timer runs
//!   the iterations (the application's callback is pulled); then a trusted
//!   click from the runner (a user activation), after which the timer
//!   resumes the context and the node takes over;
//! * `dummy`: the dummy driver's timers (`Module['SDL3'].dummy_audio`)
//!   feeding a playback and a recording device;
//! * `quit`: `SDL_Quit()` with both Web Audio devices open closes them, and
//!   the context.
//!
//! One answer is the repository's, marked Note (upstream) in
//! `sdl3::audio`'s resampler: the resampled stream's samples, which this
//! resampler computes as upstream's builds with SIMD intrinsics do (6 zero
//! crossings) where upstream's Emscripten build, which has none, uses 5
//! (the same number of samples, a little less in total).
//!
//! Each step prints a libtest-like `test NAME ... ok` line (with both
//! traces when they differ); under node everything skips as `browser`, and
//! without Web Audio as `webaudio`. `SDL3_EMSCRIPTEN_BROWSER=1
//! SDL3_TEST_REQUIRE=browser,webaudio cargo test -p sdl3 --target
//! wasm32-unknown-emscripten --test emscripten_audio` runs it as CI does.

#[cfg(not(target_os = "emscripten"))]
fn main() {}

#[cfg(target_os = "emscripten")]
fn main() -> std::process::ExitCode {
    webaudio::main()
}

/// The program's request for the browser (see `emscripten_browser.rs`).
#[cfg(target_os = "emscripten")]
#[used]
#[link_section = "sdl3_runner"]
static RUNNER: [u8; 7] = *b"browser";

#[cfg(target_os = "emscripten")]
mod webaudio {
    use std::fmt::Write as _;
    use std::panic::{self, AssertUnwindSafe};
    use std::process::ExitCode;
    use std::sync::atomic::{AtomicI32, Ordering};
    use std::sync::Mutex;
    use std::time::{Duration, Instant};

    use sdl3::app::{self, AppCallbacks, AppResult};
    use sdl3::audio::{
        self, AudioDeviceID, AudioFormat, AudioSpec, AudioStream, AUDIO_DEVICE_DEFAULT_PLAYBACK,
        AUDIO_DEVICE_DEFAULT_RECORDING,
    };
    use sdl3::events::{self, Event, EventType};
    use sdl3::hints;
    use sdl3::init::{self, InitFlags};
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

    fn has_web_audio() -> bool {
        // SAFETY: plain JavaScript.
        unsafe {
            em_asm_int!(
                "return (typeof AudioContext !== 'undefined' || typeof webkitAudioContext !== 'undefined') ? 1 : 0;"
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

    /// Run JavaScript statements.
    fn js_run(code: &str) {
        js(&format!("(function() {{ {code} }})()"));
    }

    fn js_int(expr: &str) -> i32 {
        js(expr).parse().unwrap_or(-1)
    }

    // The snippets the C reference runs too, verbatim.

    /// Wraps the playback node's `onaudioprocess`: after SDL's, copies of
    /// the output buffer's two channels while `Module.cap.on` (32 at most).
    const CAPTURE_INSTALL: &str = "var SDL3 = Module['SDL3']; var n = SDL3.audio_playback.scriptProcessorNode; var f = n.onaudioprocess; \
Module.cap = { on: false, calls: 0, l: [], r: [] }; \
n.onaudioprocess = function(e) { f(e); var c = Module.cap; c.calls++; if (c.on && c.l.length < 32) { c.l.push(Float32Array.from(e.outputBuffer.getChannelData(0))); c.r.push(Float32Array.from(e.outputBuffer.getChannelData(1))); } };";
    const CAPTURE_START: &str = "Module.cap.on = true; Module.cap.l = []; Module.cap.r = [];";
    const CAPTURED: &str = "String(Module.cap.l.length)";
    const RAMP_CHECK: &str = "(function() { var c = Module.cap; var L = [], R = []; c.l.forEach((b) => L.push(...b)); c.r.forEach((b) => R.push(...b)); \
var k = L.findIndex((v) => v != 0); var ok = 1; for (var i = 0; i < 3000; i++) { var v = Math.fround((i + 1) / 8192); if (L[k + i] !== v || R[k + i] !== -v) { ok = 0; break; } } \
var rest = 1; for (var i = k + 3000; i < L.length; i++) if (L[i] !== 0 || R[i] !== 0) { rest = 0; break; } \
return 'first nonzero ' + k + ' ramp ' + ok + ' then silence ' + rest + ' ' + (L.length - k - 3000 >= 2048 ? '(a buffer or more)' : '(short)'); })()";
    const SILENT_CHECK: &str = "(function() { var c = Module.cap; var z = 1; c.l.concat(c.r).forEach((b) => { if (b.some((v) => v != 0)) z = 0; }); return String(z); })()";
    const FIRST_CHECK: &str = "(function() { var c = Module.cap; var L = []; c.l.forEach((b) => L.push(...b)); var k = L.findIndex((v) => v != 0); return 'first nonzero ' + k + ' values ' + L.slice(k, k + 2).join(','); })()";
    const CONVERT_CHECK: &str = "(function() { var c = Module.cap; var L = [], R = []; c.l.forEach((b) => L.push(...b)); c.r.forEach((b) => R.push(...b)); var k = L.findIndex((v) => v != 0); \
var same = 1; for (var i = 0; i < L.length; i++) if (L[i] !== R[i]) same = 0; return 'first nonzero ' + k + ' left ' + L.slice(k, k + 5).join(',') + ' right same ' + same; })()";
    const RESAMPLE_CHECK: &str = "(function() { var c = Module.cap; var L = [], R = []; c.l.forEach((b) => L.push(...b)); c.r.forEach((b) => R.push(...b)); \
var nz = 0, sum = 0, max = 0, same = 1; for (var i = 0; i < L.length; i++) { if (L[i] != 0) nz++; sum += L[i]; max = Math.max(max, L[i]); if (L[i] !== R[i]) same = 0; } \
return 'nonzero ' + nz + ' sum ' + sum.toFixed(4) + ' max ' + max.toFixed(4) + ' right same ' + same; })()";

    /// A trusted click at (60, 40) in the canvas: the runner's
    /// `Input.dispatchMouseEvent`, once the page is back in its event loop.
    const TRUSTED_CLICK: &str = "var r = document.querySelector('#canvas').getBoundingClientRect(); var x = r.left + 60, y = r.top + 40; \
for (var p of [{ type: 'mouseMoved', x: x, y: y }, { type: 'mousePressed', x: x, y: y, button: 'left', buttons: 1, clickCount: 1 }, { type: 'mouseReleased', x: x, y: y, button: 'left', buttons: 0, clickCount: 1 }]) \
sdl3RunnerReport(JSON.stringify(['input', ['dispatchMouseEvent', p]]));";

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

    /// [`EXPECTED`] with this repository's answer for the resampled stream
    /// (see the module's documentation).
    fn expected() -> String {
        EXPECTED.replace(
            "F32 mono at half the rate: nonzero 200 sum 99.8272 max 0.5568",
            "F32 mono at half the rate: nonzero 200 sum 99.8396 max 0.5603",
        )
    }

    /// Compare the trace with upstream's, step by step; the result.
    fn finish() -> bool {
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

    /// A device's format, with its rate as `ctx` where it's the
    /// `AudioContext`'s (`web_audio`).
    fn format_line_of(label: &str, id: AudioDeviceID, web_audio: bool) {
        let (spec, frames) = match audio::audio_device_format(id) {
            Ok(f) => f,
            Err(e) => {
                out!("{label} format: err {}", e.message());
                return;
            }
        };
        let ctx = js("String(Module['SDL3'] && Module['SDL3'].audioContext ? Module['SDL3'].audioContext.sampleRate : 0)");
        let rate = if web_audio && ctx.parse::<i32>().ok() == Some(spec.freq) {
            "ctx".to_owned()
        } else {
            spec.freq.to_string()
        };
        out!(
            "{label} format: {} {} {rate} frames {frames}",
            &spec.format.name()[10..],
            spec.channels
        );
    }

    fn format_line(label: &str, id: AudioDeviceID) {
        format_line_of(label, id, true);
    }

    /// The bytes the counting callback was asked for.
    static PULLED: AtomicI32 = AtomicI32::new(0);

    /// A stream callback that feeds silence and counts what it was asked
    /// for (the C's `counting`).
    fn counting(s: &AudioStream, additional: i32, _total: i32) {
        if additional > 0 {
            s.put_data(&vec![0u8; additional as usize]).unwrap();
            PULLED.fetch_add(additional, Ordering::SeqCst);
        }
    }

    fn put_ramp(s: &AudioStream, n: usize) {
        let d: Vec<u8> = (0..n)
            .flat_map(|i| {
                let v = (i + 1) as f32 / 8192.0;
                [v, -v]
            })
            .flat_map(f32::to_ne_bytes)
            .collect();
        s.put_data(&d).unwrap();
    }

    fn ok(r: bool) -> i32 {
        r as i32
    }

    struct State {
        phase: u32,
        t0: Instant,
        rate: i32,
        play: AudioDeviceID,
        rec: AudioDeviceID,
        stream: Option<AudioStream>,
        rstream: Option<AudioStream>,
        mic_seen: bool,
    }

    impl State {
        fn next(&mut self) {
            self.phase += 1;
            self.t0 = Instant::now();
        }

        fn timed_out(&mut self, what: &str) {
            if self.t0.elapsed() > Duration::from_secs(10) {
                out!("TIMEOUT {what}");
                self.next();
            }
        }

        fn init_step(&mut self) {
            step("init");
            let names: Vec<_> = (0..audio::num_audio_drivers())
                .map(|i| audio::audio_driver(i).unwrap())
                .collect();
            out!("drivers {}", names.join(","));
            let r = init::init(InitFlags::AUDIO);
            out!(
                "init {} driver {}",
                ok(r.is_ok()),
                audio::current_audio_driver().unwrap_or("(null)")
            );
            events::pump();
            let (mut ap, mut ar) = (0, 0);
            while let Some(e) = events::poll() {
                if let Event::AudioDevice(e) = e {
                    if e.event_type == EventType::AUDIO_DEVICE_ADDED {
                        if e.recording {
                            ar += 1;
                        } else {
                            ap += 1;
                        }
                    }
                }
            }
            out!("added playback {ap} recording {ar}");
            let p = audio::playback_devices().unwrap_or_default();
            out!(
                "playback devices {} [{}]",
                p.len(),
                p.first()
                    .map(|&d| audio::audio_device_name(d).unwrap())
                    .unwrap_or_default()
            );
            let r = audio::recording_devices().unwrap_or_default();
            out!(
                "recording devices {} [{}]",
                r.len(),
                r.first()
                    .map(|&d| audio::audio_device_name(d).unwrap())
                    .unwrap_or_default()
            );
            format_line("default playback", AUDIO_DEVICE_DEFAULT_PLAYBACK);
            format_line("default recording", AUDIO_DEVICE_DEFAULT_RECORDING);
            out!("context {}", js("typeof Module['SDL3'].audioContext"));
        }

        fn playback_step(&mut self) {
            step("playback");
            self.play = audio::open_audio_device(AUDIO_DEVICE_DEFAULT_PLAYBACK, None).unwrap_or(0);
            out!(
                "open {} paused {}",
                ok(self.play != 0),
                ok(audio::audio_device_paused(self.play))
            );
            format_line("physical", AUDIO_DEVICE_DEFAULT_PLAYBACK);
            format_line("logical", self.play);
            self.rate = js_int("String(Module['SDL3'].audioContext.sampleRate)");
            out!(
                "context {} script {} recording table {}",
                js("typeof Module['SDL3'].audioContext"),
                js("String(Module['SDL3'].audio_playback.scriptProcessorNode.bufferSize)"),
                js("JSON.stringify(Module['SDL3'].audio_recording)")
            );
            js_run(CAPTURE_INSTALL);
            let stream =
                AudioStream::new(Some(&AudioSpec::new(AudioFormat::F32, 2, self.rate)), None)
                    .unwrap();
            out!(
                "bind {}",
                ok(audio::bind_audio_stream(self.play, &stream).is_ok())
            );
            put_ramp(&stream, 3000);
            js_run(CAPTURE_START);
            self.stream = Some(stream);
            self.next();
        }

        fn record_step(&mut self) {
            step("record");
            self.rec = audio::open_audio_device(AUDIO_DEVICE_DEFAULT_RECORDING, None).unwrap_or(0);
            out!("open {}", ok(self.rec != 0));
            format_line("physical", AUDIO_DEVICE_DEFAULT_RECORDING);
            out!(
                "silence timer {} buffer {} getUserMedia asked {}",
                js("typeof Module['SDL3'].audio_recording.silenceTimer"),
                js("(function() { var b = Module['SDL3'].audio_recording.silenceBuffer; return b.numberOfChannels + 'x' + b.length + (b.sampleRate === Module['SDL3'].audioContext.sampleRate ? ' at ctx' : ''); })()"),
                js("String(Module['SDL3'].audio_recording.mediaStreamNode === undefined)")
            );
            let rstream =
                AudioStream::new(None, Some(&AudioSpec::new(AudioFormat::F32, 1, self.rate)))
                    .unwrap();
            out!(
                "bind {}",
                ok(audio::bind_audio_stream(self.rec, &rstream).is_ok())
            );
            self.rstream = Some(rstream);
            self.next();
        }

        /// One iteration of the main callbacks; `Some` once it's over.
        fn iterate(&mut self) -> Option<AppResult> {
            match self.phase {
                0 => {
                    self.init_step();
                    self.playback_step();
                }
                1 => {
                    // the ramp, through the node
                    if js_int(CAPTURED) >= 3 {
                        out!("{}", js(RAMP_CHECK));
                        self.next();
                    } else {
                        self.timed_out("ramp");
                    }
                }
                2 => {
                    step("pause");
                    let _ = audio::pause_audio_device(self.play);
                    out!("paused {}", ok(audio::audio_device_paused(self.play)));
                    put_ramp(self.stream.as_ref().unwrap(), 16);
                    js_run(CAPTURE_START);
                    self.next();
                }
                3 => {
                    if js_int(CAPTURED) >= 3 {
                        out!(
                            "while paused: silence {} queued {}",
                            js(SILENT_CHECK),
                            self.stream.as_ref().unwrap().queued()
                        );
                        let _ = audio::resume_audio_device(self.play);
                        out!("resumed paused {}", ok(audio::audio_device_paused(self.play)));
                        js_run(CAPTURE_START);
                        self.next();
                    } else {
                        self.timed_out("pause");
                    }
                }
                4 => {
                    if js_int(CAPTURED) >= 2 {
                        out!(
                            "after resume: {} queued {}",
                            js(FIRST_CHECK),
                            self.stream.as_ref().unwrap().queued()
                        );
                        step("convert");
                        drop(self.stream.take());
                        let stream = AudioStream::new(
                            Some(&AudioSpec::new(AudioFormat::S16, 1, self.rate)),
                            None,
                        )
                        .unwrap();
                        out!(
                            "bind {}",
                            ok(audio::bind_audio_stream(self.play, &stream).is_ok())
                        );
                        let s16: [i16; 4] = [16384, -8192, 32767, -32768];
                        let bytes: Vec<u8> = s16.iter().flat_map(|x| x.to_ne_bytes()).collect();
                        stream.put_data(&bytes).unwrap();
                        js_run(CAPTURE_START);
                        self.stream = Some(stream);
                        self.next();
                    } else {
                        self.timed_out("resume");
                    }
                }
                5 => {
                    if js_int(CAPTURED) >= 2 {
                        out!("S16 mono: {}", js(CONVERT_CHECK));
                        drop(self.stream.take());
                        let stream = AudioStream::new(
                            Some(&AudioSpec::new(AudioFormat::F32, 1, self.rate / 2)),
                            None,
                        )
                        .unwrap();
                        out!(
                            "bind {}",
                            ok(audio::bind_audio_stream(self.play, &stream).is_ok())
                        );
                        let half: Vec<u8> = [0.5f32; 100]
                            .iter()
                            .flat_map(|x| x.to_ne_bytes())
                            .collect();
                        stream.put_data(&half).unwrap();
                        stream.flush();
                        js_run(CAPTURE_START);
                        self.stream = Some(stream);
                        self.next();
                    } else {
                        self.timed_out("convert");
                    }
                }
                6 => {
                    if js_int(CAPTURED) >= 2 {
                        out!("F32 mono at half the rate: {}", js(RESAMPLE_CHECK));
                        drop(self.stream.take());
                        self.record_step();
                    } else {
                        self.timed_out("resample");
                    }
                }
                7 => {
                    // the microphone
                    let rstream = self.rstream.as_ref().unwrap();
                    let mut buf = vec![0u8; 65536];
                    loop {
                        let n = rstream.get_data(&mut buf).unwrap_or(0);
                        if n == 0 {
                            break;
                        }
                        if buf[..n]
                            .chunks_exact(4)
                            .any(|b| f32::from_ne_bytes([b[0], b[1], b[2], b[3]]) != 0.0)
                        {
                            self.mic_seen = true;
                        }
                    }
                    if self.mic_seen
                        && js_int("String(Module['SDL3'].audio_recording.mediaStreamNode !== undefined ? 1 : 0)") != 0
                    {
                        out!(
                            "mic data {} timer {} script {} tracks {}",
                            ok(self.mic_seen),
                            js("typeof Module['SDL3'].audio_recording.silenceTimer"),
                            js("String(Module['SDL3'].audio_recording.scriptProcessorNode.bufferSize)"),
                            js("String(Module['SDL3'].audio_recording.stream.getAudioTracks().length)")
                        );
                        step("close");
                        js_run("Module.mic = Module['SDL3'].audio_recording.stream; Module.micTrack = Module.mic.getAudioTracks()[0];");
                        drop(self.rstream.take());
                        audio::close_audio_device(self.rec);
                        out!(
                            "recording closed: table {} context {} tracks {} track {}",
                            js("typeof Module['SDL3'].audio_recording"),
                            js("typeof Module['SDL3'].audioContext"),
                            js("String(Module.mic.getAudioTracks().length)"),
                            js("Module.micTrack.readyState")
                        );
                        js_run("Module.oldCtx = Module['SDL3'].audioContext;");
                        audio::close_audio_device(self.play);
                        out!(
                            "playback closed: table {} context {}",
                            js("typeof Module['SDL3'].audio_playback"),
                            js("typeof Module['SDL3'].audioContext")
                        );
                        self.next();
                    } else {
                        self.timed_out("microphone");
                    }
                }
                8 => {
                    if js("Module.oldCtx.state") == "closed" {
                        out!("the context: closed");
                        step("reopen");
                        self.play =
                            audio::open_audio_device(AUDIO_DEVICE_DEFAULT_PLAYBACK, None).unwrap_or(0);
                        out!(
                            "open {} new context {}",
                            ok(self.play != 0),
                            js("String(Module['SDL3'].audioContext !== Module.oldCtx)")
                        );
                        audio::close_audio_device(self.play);
                        out!(
                            "closed: context {} recording table {}",
                            js("typeof Module['SDL3'].audioContext"),
                            js("JSON.stringify(Module['SDL3'].audio_recording)")
                        );
                        step("autoplay");
                        js_run("Module.sus = Module['SDL3'].audioContext; Module.sus.suspend();");
                        self.next();
                    } else {
                        self.timed_out("context close");
                    }
                }
                9 => {
                    if js("Module.sus.state") == "suspended" {
                        out!("suspended");
                        PULLED.store(0, Ordering::SeqCst);
                        let stream = audio::open_audio_device_stream(
                            AUDIO_DEVICE_DEFAULT_PLAYBACK,
                            Some(&AudioSpec::new(AudioFormat::F32, 2, self.rate)),
                            Some(counting),
                        )
                        .ok();
                        if let Some(s) = &stream {
                            let _ = s.resume_device();
                        }
                        out!(
                            "open {} timer {} state {} active {} same context {}",
                            ok(stream.is_some()),
                            js("typeof Module['SDL3'].audio_playback.silenceTimer"),
                            js("Module['SDL3'].audioContext.state"),
                            js("String(navigator.userActivation.hasBeenActive)"),
                            js("String(Module['SDL3'].audioContext === Module.sus)")
                        );
                        self.stream = stream;
                        self.next();
                    } else {
                        self.timed_out("suspend");
                    }
                }
                10 => {
                    if PULLED.load(Ordering::SeqCst) >= 3 * 2048 * 8 {
                        out!(
                            "pumped while suspended: state {} timer {}",
                            js("Module['SDL3'].audioContext.state"),
                            js("typeof Module['SDL3'].audio_playback.silenceTimer")
                        );
                        js_run(TRUSTED_CLICK);
                        self.next();
                    } else {
                        self.timed_out("silence timer");
                    }
                }
                11 => {
                    if js("Module['SDL3'].audioContext.state + ' ' + typeof Module['SDL3'].audio_playback.silenceTimer") == "running undefined" {
                        out!(
                            "after a click: active {} state {} timer {}",
                            js("String(navigator.userActivation.hasBeenActive)"),
                            js("Module['SDL3'].audioContext.state"),
                            js("typeof Module['SDL3'].audio_playback.silenceTimer")
                        );
                        drop(self.stream.take());
                        out!("closed: context {}", js("typeof Module['SDL3'].audioContext"));
                        init::quit_subsystem(InitFlags::AUDIO);
                        out!(
                            "quit: driver {} context {}",
                            audio::current_audio_driver().unwrap_or("(none)"),
                            js("typeof Module['SDL3'].audioContext")
                        );
                        step("dummy");
                        hints::set(hints::AUDIO_DRIVER, "dummy").unwrap();
                        let r = init::init_subsystem(InitFlags::AUDIO);
                        out!(
                            "init {} driver {} timers {}",
                            ok(r.is_ok()),
                            audio::current_audio_driver().unwrap_or("(null)"),
                            js("JSON.stringify(Module['SDL3'].dummy_audio)")
                        );
                        PULLED.store(0, Ordering::SeqCst);
                        let stream = audio::open_audio_device_stream(
                            AUDIO_DEVICE_DEFAULT_PLAYBACK,
                            None,
                            Some(counting),
                        )
                        .unwrap();
                        stream.resume_device().unwrap();
                        format_line_of("playback", AUDIO_DEVICE_DEFAULT_PLAYBACK, false);
                        self.rec =
                            audio::open_audio_device(AUDIO_DEVICE_DEFAULT_RECORDING, None).unwrap_or(0);
                        let rstream = AudioStream::new(
                            None,
                            Some(&AudioSpec::new(AudioFormat::S16, 1, 44100)),
                        )
                        .unwrap();
                        let _ = audio::bind_audio_stream(self.rec, &rstream);
                        out!(
                            "timers {}",
                            js("Module['SDL3'].dummy_audio.timers.map((t) => typeof t).join(',')")
                        );
                        self.stream = Some(stream);
                        self.rstream = Some(rstream);
                        self.next();
                    } else {
                        self.timed_out("resume after click");
                    }
                }
                12 => {
                    let available = self.rstream.as_ref().map_or(0, |s| s.available());
                    if PULLED.load(Ordering::SeqCst) > 0 && available > 0 {
                        let mut buf = [0xFFu8; 64];
                        let n = self.rstream.as_ref().unwrap().get_data(&mut buf).unwrap_or(0);
                        let z = buf[..n].iter().all(|&b| b == 0);
                        out!(
                            "pulled {} recorded {n} silence {}",
                            ok(PULLED.load(Ordering::SeqCst) > 0),
                            ok(z)
                        );
                        drop(self.stream.take());
                        drop(self.rstream.take());
                        audio::close_audio_device(self.rec);
                        out!("closed: timers {}", js("JSON.stringify(Module['SDL3'].dummy_audio)"));
                        init::quit_subsystem(InitFlags::AUDIO);
                        hints::reset(hints::AUDIO_DRIVER);
                        step("quit");
                        let r = init::init_subsystem(InitFlags::AUDIO);
                        out!(
                            "init {} driver {}",
                            ok(r.is_ok()),
                            audio::current_audio_driver().unwrap_or("(null)")
                        );
                        self.play =
                            audio::open_audio_device(AUDIO_DEVICE_DEFAULT_PLAYBACK, None).unwrap_or(0);
                        self.rec =
                            audio::open_audio_device(AUDIO_DEVICE_DEFAULT_RECORDING, None).unwrap_or(0);
                        out!(
                            "open {} {} context {}",
                            ok(self.play != 0),
                            ok(self.rec != 0),
                            js("typeof Module['SDL3'].audioContext")
                        );
                        js_run("Module.lastCtx = Module['SDL3'].audioContext;");
                        init::quit();
                        out!(
                            "after quit: playback {} recording {} context {}",
                            js("typeof Module['SDL3'].audio_playback"),
                            js("typeof Module['SDL3'].audio_recording"),
                            js("typeof Module['SDL3'].audioContext")
                        );
                        self.next();
                    } else {
                        self.timed_out("dummy");
                    }
                }
                13 => {
                    if js("Module.lastCtx.state") == "closed" {
                        out!("the context: closed");
                        self.next();
                    } else {
                        self.timed_out("last context close");
                    }
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
        let skip = |capability: &str, reason: &str| {
            if required(capability) {
                panic!("required capability {capability} unavailable: {reason}");
            }
            println!("note: skipping, {capability} unavailable: {reason}");
            let steps = sections(EXPECTED);
            for (name, _) in &steps {
                println!("test {name} ... ok");
            }
            println!("\ntest result: ok. {} passed; 0 failed", steps.len());
            ExitCode::SUCCESS
        };
        if !in_browser() {
            return skip(
                "browser",
                "not in a browser (under node: run it with SDL3_EMSCRIPTEN_BROWSER=1, or with CHROME naming Chrome or Chromium)",
            );
        }
        if !has_web_audio() {
            return skip("webaudio", "no AudioContext");
        }
        ExitCode::from(app::run_app(sdl_main) as u8)
    }

    fn sdl_main(args: &[String]) -> i32 {
        app::enter_app_main_callbacks(args, move |_| {
            (
                App {
                    st: State {
                        phase: 0,
                        t0: Instant::now(),
                        rate: 0,
                        play: 0,
                        rec: 0,
                        stream: None,
                        rstream: None,
                        mic_seen: false,
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

    impl AppCallbacks for App {
        fn iterate(&mut self) -> AppResult {
            match panic::catch_unwind(AssertUnwindSafe(|| self.st.iterate())) {
                Ok(Some(result)) => result,
                Ok(None) => AppResult::Continue,
                Err(_) => {
                    out!("FAILED: phase {} panicked", self.st.phase);
                    if self.st.phase >= 14 {
                        // (in the end itself)
                        return AppResult::Failure;
                    }
                    self.st.phase = 14;
                    AppResult::Continue
                }
            }
        }

        fn event(&mut self, _event: &Event) -> AppResult {
            AppResult::Continue
        }

        fn quit(&mut self, _result: AppResult) {
            self.st.stream = None;
            self.st.rstream = None;
            init::quit();
        }
    }

    /// The trace of upstream's C on the same page with the same steps.
    const EXPECTED: &str = r#"== init
drivers emscripten,disk,dummy
init 1 driver emscripten
added playback 1 recording 1
playback devices 1 [System audio playback device]
recording devices 1 [System audio recording device]
default playback format: S16LE 2 44100 frames 1024
default recording format: S16LE 1 44100 frames 1024
context undefined
== playback
open 1 paused 0
physical format: F32LE 2 ctx frames 2048
logical format: F32LE 2 ctx frames 2048
context object script 2048 recording table {}
bind 1
first nonzero 0 ramp 1 then silence 1 (a buffer or more)
== pause
paused 1
while paused: silence 1 queued 128
resumed paused 0
after resume: first nonzero 0 values 0.0001220703125,0.000244140625 queued 0
== convert
bind 1
S16 mono: first nonzero 0 left 0.5,-0.25,0.999969482421875,-1,0 right same 1
bind 1
F32 mono at half the rate: nonzero 200 sum 99.8272 max 0.5568 right same 1
== record
open 1
physical format: F32LE 1 ctx frames 2048
silence timer number buffer 1x2048 at ctx getUserMedia asked true
bind 1
mic data 1 timer undefined script 2048 tracks 1
== close
recording closed: table undefined context object tracks 0 track live
playback closed: table undefined context undefined
the context: closed
== reopen
open 1 new context true
closed: context object recording table {}
== autoplay
suspended
open 1 timer number state suspended active false same context true
pumped while suspended: state suspended timer number
after a click: active true state running timer undefined
closed: context object
quit: driver (none) context object
== dummy
init 1 driver dummy timers {"timers":[null,null]}
playback format: S16LE 2 44100 frames 1024
timers number,number
pulled 1 recorded 64 silence 1
closed: timers {"timers":[null,null]}
== quit
init 1 driver emscripten
open 1 1 context object
after quit: playback undefined recording undefined context undefined
the context: closed
"#;
}
