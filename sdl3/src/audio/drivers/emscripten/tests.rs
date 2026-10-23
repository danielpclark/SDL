// Tests of the Emscripten audio driver (and the dummy driver's Emscripten
// parts) under node (in a browser: sdl3/tests/emscripten_audio.rs).
//
// Node has no Web Audio, so the driver is unavailable there; the other
// tests stand in for it a Web Audio of their own (`AudioContext`,
// `ScriptProcessorNode`, `AudioBuffer`, `getUserMedia()` and the timers),
// which records what the driver's JavaScript asks of it and lets the test
// fire its events synchronously: the nodes' `onaudioprocess`, the silence
// timers, the microphone's answer.

use std::ffi::CString;
use std::sync::atomic::{AtomicI32, Ordering};
use std::sync::Arc;

use sdl3_emscripten::{em_asm, em_asm_int};

use crate::audio::drivers::tests::QuitAudioOnDrop;
use crate::audio::{
    audio_device_format, audio_device_name, audio_driver, current_audio_driver, num_audio_drivers,
    open_audio_device_stream, playback_devices, recording_devices, AudioDevice, AudioFormat,
    AudioSpec, AudioStream, AUDIO_DEVICE_DEFAULT_PLAYBACK, AUDIO_DEVICE_DEFAULT_RECORDING,
};
use crate::hints;
use crate::init::{self, InitFlags};

// -- JavaScript ---------------------------------------------------------------

/// The value of a JavaScript expression (in the glue's scope), as an `i32`.
fn js_int(expr: &str) -> i32 {
    let code = CString::new(expr).unwrap();
    // SAFETY: plain JavaScript; the code is NUL-terminated.
    unsafe { em_asm_int!("return Number(eval(UTF8ToString($0))) | 0;", code.as_ptr()) }
}

/// The value of a JavaScript expression, as a string.
fn js_str(expr: &str) -> String {
    let code = CString::new(expr).unwrap();
    // SAFETY: plain JavaScript; the code is NUL-terminated.
    let len = unsafe {
        em_asm_int!(
            "Module['sdl3_test_s'] = new TextEncoder().encode(String(eval(UTF8ToString($0)))); return Module['sdl3_test_s'].length;",
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
    let code = CString::new(format!("(function() {{ {code} }})()")).unwrap();
    // SAFETY: plain JavaScript; the code is NUL-terminated.
    unsafe { em_asm!("eval(UTF8ToString($0));", code.as_ptr()) };
}

/// The first `n` values of a JavaScript `Float32Array` expression.
fn js_f32s(expr: &str, n: usize) -> Vec<f32> {
    let code = CString::new(expr).unwrap();
    let mut out = vec![f32::NAN; n];
    // SAFETY: writes `n` floats into `out`.
    unsafe {
        em_asm!(
            "HEAPF32.set(eval(UTF8ToString($0)).subarray(0, $2), $1 >>> 2);",
            code.as_ptr(),
            out.as_mut_ptr(),
            n as i32
        )
    };
    out
}

/// Whether this JavaScript has Web Audio of its own (a browser).
fn has_web_audio() -> bool {
    // SAFETY: plain JavaScript.
    unsafe { sdl3_emscripten::audio::sdl3_audio_available() != 0 }
}

/// A stand-in Web Audio (and timers, and `navigator.userActivation` and
/// `navigator.mediaDevices`), as `Module.sdl3_test_audio` (`M`): the
/// contexts made (`M.contexts`), the nodes (`M.nodes`), the timers
/// (`M.timers`: `{fn, ms, cleared}`, by id − 1) and the pending
/// `getUserMedia()` answers (`M.gum`). `AudioBuffer`s are `M.Buf(channels,
/// frames, rate)`. Taken away (and SDL's audio state with it) when dropped.
struct WebAudio;

impl WebAudio {
    fn new(rate: i32, state: &str) -> WebAudio {
        js_run(&format!(
            r#"
            var M = Module['sdl3_test_audio'] = {{ rate: {rate}, state: '{state}', contexts: [], nodes: [], timers: [], gum: [] }};
            M.saved = {{ AudioContext: globalThis.AudioContext, setInterval: globalThis.setInterval, clearInterval: globalThis.clearInterval }};
            M.Buf = function(channels, frames, rate) {{
                this.numberOfChannels = channels; this.length = frames; this.sampleRate = rate; this.data = [];
                for (var c = 0; c < channels; c++) this.data.push(new Float32Array(frames));
            }};
            M.Buf.prototype.getChannelData = function(c) {{ return this.data[c]; }};
            var Node = function(kind, args) {{ this.kind = kind; this.args = args; this.connected = []; this.disconnected = 0; M.nodes.push(this); }};
            Node.prototype.connect = function(n) {{ this.connected.push(n.kind); }};
            Node.prototype.disconnect = function() {{ this.disconnected++; }};
            var Context = function() {{ this.sampleRate = M.rate; this.state = M.state; this.destination = new Node('destination', []); this.closed = 0; this.resumed = 0; M.contexts.push(this); }};
            Context.prototype.createScriptProcessor = function(size, ins, outs) {{ var n = new Node('script', [size, ins, outs]); n.bufferSize = size; return n; }};
            Context.prototype.createBuffer = function(channels, frames, rate) {{ return new M.Buf(channels, frames, rate); }};
            Context.prototype.createMediaStreamSource = function(stream) {{ return new Node('source', [stream]); }};
            Context.prototype.close = function() {{ this.closed++; this.state = 'closed'; }};
            Context.prototype.resume = function() {{ this.resumed++; }};
            globalThis.AudioContext = Context;
            globalThis.setInterval = function(fn, ms) {{ M.timers.push({{ fn: fn, ms: ms, cleared: false }}); return M.timers.length; }};
            globalThis.clearInterval = function(id) {{ M.timers[id - 1].cleared = true; }};
            navigator.userActivation = {{ hasBeenActive: false }};
            navigator.mediaDevices = {{ getUserMedia: function(constraints) {{
                M.constraints = JSON.stringify(constraints);
                return {{ then: function(yes) {{ M.gum.push(yes); return {{ catch: function(no) {{}} }}; }} }};
            }} }};
            M.stream = function() {{
                var s = {{ tracks: ['t1'], removed: [] }};
                s.getAudioTracks = function() {{ return s.tracks.slice(); }};
                s.removeTrack = function(t) {{ s.removed.push(t); }};
                return s;
            }};
            var SDL3 = Module['SDL3'];
            if (SDL3) {{ SDL3.audioContext = undefined; SDL3.audio_playback = undefined; SDL3.audio_recording = undefined; }}
        "#
        ));
        WebAudio
    }
}

impl Drop for WebAudio {
    fn drop(&mut self) {
        for _ in 0..8 {
            if current_audio_driver().is_none() {
                break;
            }
            init::quit_subsystem(InitFlags::AUDIO);
        }
        js_run(
            r#"
            var M = Module['sdl3_test_audio'];
            globalThis.AudioContext = M.saved.AudioContext;
            if (M.saved.AudioContext === undefined) delete globalThis.AudioContext;
            globalThis.setInterval = M.saved.setInterval;
            globalThis.clearInterval = M.saved.clearInterval;
            delete navigator.userActivation;
            delete navigator.mediaDevices;
            delete Module['sdl3_test_audio'];
            var SDL3 = Module['SDL3'];
            if (SDL3) { SDL3.audioContext = undefined; SDL3.audio_playback = undefined; SDL3.audio_recording = undefined; }
        "#,
        );
        hints::reset(hints::AUDIO_DRIVER);
    }
}

/// A stream callback that feeds silence, and the bytes it was asked for.
fn counting_callback() -> (
    Arc<AtomicI32>,
    impl Fn(&AudioStream, i32, i32) + Send + Sync + 'static,
) {
    let pulled = Arc::new(AtomicI32::new(0));
    let p2 = pulled.clone();
    (
        pulled,
        move |s: &AudioStream, additional: i32, _total: i32| {
            if additional > 0 {
                s.put_data(&vec![0u8; additional as usize]).unwrap();
                p2.fetch_add(additional, Ordering::SeqCst);
            }
        },
    )
}

/// Interleaved `f32` frames as bytes.
fn f32_bytes(samples: &[f32]) -> Vec<u8> {
    samples.iter().flat_map(|x| x.to_ne_bytes()).collect()
}

/// `n` frames of a stereo ramp: frame `i` is `((i + 1) / 8192, -(i + 1) /
/// 8192)`, exact in `f32`.
fn ramp(n: usize) -> Vec<f32> {
    (0..n)
        .flat_map(|i| {
            let v = (i + 1) as f32 / 8192.0;
            [v, -v]
        })
        .collect()
}

// -- the tests ----------------------------------------------------------------

/// Under node there's no `AudioContext`: the driver is unavailable, as
/// upstream's is, and initialization goes on to the next driver. Note
/// (upstream): the message is the core's (upstream's is the driver's "No
/// audio context available").
#[test]
fn unavailable_without_web_audio() {
    let _l = crate::test_support::test_lock();
    let _cleanup = QuitAudioOnDrop;
    if has_web_audio() {
        println!("note: skipping, this JavaScript has Web Audio (a browser)");
        return;
    }

    // First in the list, before disk and dummy, as upstream's bootstrap.
    let names: Vec<_> = (0..num_audio_drivers())
        .map(|i| audio_driver(i).unwrap())
        .collect();
    assert_eq!(names, ["emscripten", "disk", "dummy"]);
    assert!((super::EMSCRIPTENAUDIO_BOOTSTRAP.init)().is_none());

    // Without the hint: tried, and none is left (disk and dummy are
    // demand-only).
    hints::reset(hints::AUDIO_DRIVER);
    let e = init::init_subsystem(InitFlags::AUDIO).unwrap_err();
    assert_eq!(e.message(), "Audio target '' failed to initialize");
    assert!(current_audio_driver().is_none());

    // Asked for by name: not available.
    hints::set(hints::AUDIO_DRIVER, "emscripten").unwrap();
    let e = init::init_subsystem(InitFlags::AUDIO).unwrap_err();
    assert_eq!(
        e.message(),
        "Audio target 'emscripten' failed to initialize"
    );

    // The first that works, after it.
    hints::set(hints::AUDIO_DRIVER, "emscripten,dummy").unwrap();
    init::init_subsystem(InitFlags::AUDIO).unwrap();
    assert_eq!(current_audio_driver(), Some("dummy"));
    init::quit_subsystem(InitFlags::AUDIO);
}

/// Playback: the device is the context's, at its rate, in floats, and each
/// `onaudioprocess` gets one device buffer, the bound streams' mix.
#[test]
fn playback_through_web_audio() {
    let _l = crate::test_support::test_lock();
    let _web = WebAudio::new(48000, "running");

    init::init_subsystem(InitFlags::AUDIO).unwrap();
    assert_eq!(current_audio_driver(), Some("emscripten"));
    let playback = playback_devices().unwrap();
    let recording = recording_devices().unwrap();
    assert_eq!((playback.len(), recording.len()), (1, 1));
    assert_eq!(
        audio_device_name(playback[0]).unwrap(),
        "System audio playback device"
    );
    // Before it's opened, the core's default.
    assert_eq!(
        audio_device_format(playback[0]).unwrap().0,
        AudioSpec::new(AudioFormat::S16, 2, 44100)
    );
    // No context until a device opens.
    assert_eq!(js_int("Module['sdl3_test_audio'].contexts.length"), 0);

    let dev = AudioDevice::open(AUDIO_DEVICE_DEFAULT_PLAYBACK, None).unwrap();
    let (spec, frames) = audio_device_format(playback[0]).unwrap();
    assert_eq!(spec, AudioSpec::new(AudioFormat::F32, 2, 48000));
    assert_eq!(frames, 2048, "twice the default for 48 kHz");
    assert_eq!(js_int("Module['sdl3_test_audio'].contexts.length"), 1);
    assert_eq!(
        js_str("var n = Module['SDL3'].audio_playback.scriptProcessorNode; n.kind + ' ' + n.args + ' ' + n.connected"),
        "script 2048,0,2 destination"
    );
    // Running: no silence timer. Both device tables are made.
    assert_eq!(js_int("Module['sdl3_test_audio'].timers.length"), 0);
    assert_eq!(
        js_str("typeof Module['SDL3'].audio_playback.silenceTimer + ' ' + JSON.stringify(Module['SDL3'].audio_recording)"),
        "undefined {}"
    );

    let process = || {
        js_run(
            "var M = Module['sdl3_test_audio']; M.out = new M.Buf(2, 2048, 48000); M.out.data[0].fill(9); M.out.data[1].fill(9); \
             Module['SDL3'].audio_playback.scriptProcessorNode.onaudioprocess({ outputBuffer: M.out });",
        );
        (
            js_f32s("Module['sdl3_test_audio'].out.data[0]", 2048),
            js_f32s("Module['sdl3_test_audio'].out.data[1]", 2048),
        )
    };

    // Nothing bound: silence.
    let (l, r) = process();
    assert!(l.iter().chain(&r).all(|&x| x == 0.0));

    // A stream at the device's format: its samples, deinterleaved.
    let stream = AudioStream::new(Some(&spec), None).unwrap();
    dev.bind(&stream).unwrap();
    let data = ramp(3000);
    stream.put_data(&f32_bytes(&data)).unwrap();
    let (l, r) = process();
    for i in 0..2048 {
        assert_eq!((l[i], r[i]), (data[2 * i], data[2 * i + 1]), "frame {i}");
    }
    // The rest, then silence.
    let (l, r) = process();
    for i in 0..2048 {
        let want = if i < 3000 - 2048 {
            (data[2 * (2048 + i)], data[2 * (2048 + i) + 1])
        } else {
            (0.0, 0.0)
        };
        assert_eq!((l[i], r[i]), want, "frame {}", 2048 + i);
    }

    // Paused: silence, and the stream keeps its data.
    stream.put_data(&f32_bytes(&ramp(16))).unwrap();
    dev.pause().unwrap();
    let (l, _) = process();
    assert!(l.iter().all(|&x| x == 0.0));
    assert_eq!(stream.queued(), 16 * 8);
    dev.resume().unwrap();
    let (l, _) = process();
    assert_eq!(l[..2], [1.0 / 8192.0, 2.0 / 8192.0]);
    assert_eq!(stream.queued(), 0);
    drop(stream);

    // Format conversion: S16 mono at the device's rate, to float stereo.
    let s16 = AudioStream::new(Some(&AudioSpec::new(AudioFormat::S16, 1, 48000)), None).unwrap();
    dev.bind(&s16).unwrap();
    let samples: [i16; 4] = [16384, -8192, 32767, -32768];
    s16.put_data(
        &samples
            .iter()
            .flat_map(|x| x.to_ne_bytes())
            .collect::<Vec<u8>>(),
    )
    .unwrap();
    let (l, r) = process();
    assert_eq!(l[..5], [0.5, -0.25, 32767.0 / 32768.0, -1.0, 0.0]);
    assert_eq!(r[..5], l[..5]);
    drop(s16);

    // Closed: the node goes.
    drop(dev);
    assert_eq!(
        js_str("var n = Module['sdl3_test_audio'].nodes.filter((n) => n.kind === 'script'); n.length + ' ' + n[0].disconnected"),
        "1 1"
    );
    assert_eq!(js_str("typeof Module['SDL3'].audio_playback"), "undefined");
    // Note (upstream): the recording table (made when the playback device
    // opened) is still there, so the context isn't closed.
    assert_eq!(
        js_str("var c = Module['sdl3_test_audio'].contexts[0]; c.closed + ' ' + (Module['SDL3'].audioContext === c)"),
        "0 true"
    );

    // Opened again: the same context, a new node.
    let dev = AudioDevice::open(AUDIO_DEVICE_DEFAULT_PLAYBACK, None).unwrap();
    assert_eq!(js_int("Module['sdl3_test_audio'].contexts.length"), 1);
    assert_eq!(
        js_int("Module['sdl3_test_audio'].nodes.filter((n) => n.kind === 'script').length"),
        2
    );
    drop(dev);
}

/// Autoplay blocked (a suspended context): a silence timer runs the
/// iterations until the node's first `onaudioprocess`, and resumes the
/// context once the page has had a user gesture.
#[test]
fn playback_while_autoplay_is_blocked() {
    let _l = crate::test_support::test_lock();
    let _web = WebAudio::new(44100, "suspended");
    init::init_subsystem(InitFlags::AUDIO).unwrap();

    let (pulled, callback) = counting_callback();
    let stream = open_audio_device_stream(
        AUDIO_DEVICE_DEFAULT_PLAYBACK,
        Some(&AudioSpec::new(AudioFormat::F32, 2, 44100)),
        Some(callback),
    )
    .unwrap();
    stream.resume_device().unwrap();

    // The timer: every 2048 frames at 44.1 kHz, with a silent stereo buffer.
    assert_eq!(
        js_str("var t = Module['sdl3_test_audio'].timers; t.length + ' ' + t[0].ms.toFixed(3) + ' ' + t[0].cleared"),
        "1 46.440 false"
    );
    assert_eq!(
        js_str("var b = Module['SDL3'].audio_playback.silenceBuffer; b.numberOfChannels + ' ' + b.length + ' ' + b.sampleRate"),
        "2 2048 44100"
    );
    let tick = || js_run("Module['sdl3_test_audio'].timers[0].fn();");

    // Each tick runs an iteration, which pulls the stream (into the
    // silence buffer, which is thrown away).
    tick();
    assert_eq!(pulled.load(Ordering::SeqCst), 2048 * 8);
    assert_eq!(
        js_str("typeof Module['SDL3'].audio_playback.currentPlaybackBuffer"),
        "undefined"
    );
    // Without a user gesture the context isn't resumed; after one, it is.
    assert_eq!(js_int("Module['sdl3_test_audio'].contexts[0].resumed"), 0);
    js_run("navigator.userActivation.hasBeenActive = true;");
    tick();
    assert_eq!(pulled.load(Ordering::SeqCst), 2 * 2048 * 8);
    assert_eq!(js_int("Module['sdl3_test_audio'].contexts[0].resumed"), 1);

    // The node's first event stops the timer.
    js_run(
        "var M = Module['sdl3_test_audio']; Module['SDL3'].audio_playback.scriptProcessorNode.onaudioprocess({ outputBuffer: new M.Buf(2, 2048, 44100) });",
    );
    assert_eq!(pulled.load(Ordering::SeqCst), 3 * 2048 * 8);
    assert_eq!(
        js_str("Module['sdl3_test_audio'].timers[0].cleared + ' ' + typeof Module['SDL3'].audio_playback.silenceTimer + ' ' + typeof Module['SDL3'].audio_playback.silenceBuffer"),
        "true undefined undefined"
    );
    drop(stream);
}

/// Recording: silence from a timer until the microphone answers, then the
/// microphone's samples through a `ScriptProcessorNode`, interleaved.
#[test]
fn recording_through_web_audio() {
    let _l = crate::test_support::test_lock();
    let _web = WebAudio::new(48000, "running");
    init::init_subsystem(InitFlags::AUDIO).unwrap();
    assert_eq!(
        audio_device_name(AUDIO_DEVICE_DEFAULT_RECORDING).unwrap(),
        "System audio recording device"
    );

    let dev = AudioDevice::open(AUDIO_DEVICE_DEFAULT_RECORDING, None).unwrap();
    let (spec, frames) = dev.format().unwrap();
    assert_eq!(spec, AudioSpec::new(AudioFormat::F32, 1, 48000));
    assert_eq!(frames, 2048);
    assert_eq!(
        js_str("var M = Module['sdl3_test_audio']; M.timers.length + ' ' + M.timers[0].ms.toFixed(3) + ' ' + M.gum.length + ' ' + M.constraints"),
        "1 42.667 1 {\"audio\":true,\"video\":false}"
    );
    let stream = AudioStream::new(None, Some(&spec)).unwrap();
    dev.bind(&stream).unwrap();

    // The silence timer.
    js_run("Module['sdl3_test_audio'].timers[0].fn();");
    let mut buf = vec![0xFFu8; 2048 * 4];
    assert_eq!(stream.get_data(&mut buf).unwrap(), 2048 * 4);
    assert!(buf.iter().all(|&b| b == 0));

    // The microphone: the timer stops, the stream's source goes into a
    // node of the device's channels, connected to the destination.
    js_run("var M = Module['sdl3_test_audio']; M.mic = M.stream(); M.gum[0](M.mic);");
    assert_eq!(
        js_str("var M = Module['sdl3_test_audio']; var r = Module['SDL3'].audio_recording; M.timers[0].cleared + ' ' + typeof r.silenceTimer + ' ' + r.mediaStreamNode.connected + ' ' + r.scriptProcessorNode.args + ' ' + r.scriptProcessorNode.connected + ' ' + (r.stream === M.mic)"),
        "true undefined script 2048,1,1 destination true"
    );
    let record = |channels: usize| {
        js_run(&format!(
            "var M = Module['sdl3_test_audio']; var i = new M.Buf({channels}, 2048, 48000); \
             for (var c = 0; c < {channels}; c++) for (var j = 0; j < 2048; j++) i.data[c][j] = (c ? -1 : 1) * (j + 1) / 8192; \
             M.out = new M.Buf(1, 2048, 48000); M.out.data[0].fill(9); \
             Module['SDL3'].audio_recording.scriptProcessorNode.onaudioprocess({{ inputBuffer: i, outputBuffer: M.out }});"
        ));
        // (the node's output is silenced)
        assert!(js_f32s("Module['sdl3_test_audio'].out.data[0]", 2048)
            .iter()
            .all(|&x| x == 0.0));
    };
    record(1);
    let mut buf = vec![0u8; 2048 * 4];
    assert_eq!(stream.get_data(&mut buf).unwrap(), 2048 * 4);
    let got: Vec<f32> = buf
        .chunks_exact(4)
        .map(|b| f32::from_ne_bytes([b[0], b[1], b[2], b[3]]))
        .collect();
    let want: Vec<f32> = (0..2048).map(|j| (j + 1) as f32 / 8192.0).collect();
    assert_eq!(got, want);
    drop(stream);

    // Closed: the tracks, the nodes and the table go.
    drop(dev);
    assert_eq!(
        js_str("var M = Module['sdl3_test_audio']; var n = M.nodes.filter((n) => n.kind !== 'destination'); M.mic.removed + ' ' + n.map((n) => n.kind + n.disconnected) + ' ' + typeof Module['SDL3'].audio_recording"),
        "t1 source1,script1 undefined"
    );

    // Stereo (the device opens at the spec asked for): interleaved.
    let spec2 = AudioSpec::new(AudioFormat::F32, 2, 48000);
    let dev = AudioDevice::open(AUDIO_DEVICE_DEFAULT_RECORDING, Some(&spec2)).unwrap();
    assert_eq!(dev.format().unwrap().0, spec2);
    let stream = AudioStream::new(None, Some(&spec2)).unwrap();
    dev.bind(&stream).unwrap();
    js_run("var M = Module['sdl3_test_audio']; M.gum[1](M.stream());");
    assert_eq!(
        js_str("String(Module['SDL3'].audio_recording.scriptProcessorNode.args)"),
        "2048,2,1"
    );
    record(2);
    let mut buf = vec![0u8; 2048 * 8];
    assert_eq!(stream.get_data(&mut buf).unwrap(), 2048 * 8);
    let got: Vec<f32> = buf
        .chunks_exact(4)
        .map(|b| f32::from_ne_bytes([b[0], b[1], b[2], b[3]]))
        .collect();
    assert_eq!(got, ramp(2048));
    drop(stream);
    drop(dev);

    // Note (upstream): the playback table (made when the recording device
    // opened) is still there, so the context isn't closed.
    assert_eq!(
        js_str("Module['sdl3_test_audio'].contexts[0].closed + ' ' + JSON.stringify(Module['SDL3'].audio_playback)"),
        "0 {}"
    );
}

/// Both devices closed: the context is closed and forgotten; the next
/// device makes a new one.
#[test]
fn the_context_closes_with_both_devices() {
    let _l = crate::test_support::test_lock();
    let _web = WebAudio::new(48000, "running");
    init::init_subsystem(InitFlags::AUDIO).unwrap();

    let play = AudioDevice::open(AUDIO_DEVICE_DEFAULT_PLAYBACK, None).unwrap();
    let rec = AudioDevice::open(AUDIO_DEVICE_DEFAULT_RECORDING, None).unwrap();
    assert_eq!(js_int("Module['sdl3_test_audio'].contexts.length"), 1);
    drop(rec);
    assert_eq!(js_int("Module['sdl3_test_audio'].contexts[0].closed"), 0);
    drop(play);
    assert_eq!(
        js_str("Module['sdl3_test_audio'].contexts[0].closed + ' ' + typeof Module['SDL3'].audioContext"),
        "1 undefined"
    );
    let play = AudioDevice::open(AUDIO_DEVICE_DEFAULT_PLAYBACK, None).unwrap();
    assert_eq!(js_int("Module['sdl3_test_audio'].contexts.length"), 2);

    // Quitting closes the opened device (Note (upstream): not the context,
    // the recording table being there).
    init::quit_subsystem(InitFlags::AUDIO);
    assert_eq!(
        js_str("typeof Module['SDL3'].audio_playback + ' ' + Module['sdl3_test_audio'].nodes.filter((n) => n.kind === 'script').map((n) => n.disconnected) + ' ' + Module['sdl3_test_audio'].contexts[1].closed"),
        "undefined 1,1 0"
    );
    drop(play); // (closing what's gone does nothing)
}

/// Without `getUserMedia()` there's no recording device.
#[test]
fn no_recording_without_get_user_media() {
    let _l = crate::test_support::test_lock();
    let _web = WebAudio::new(48000, "running");
    js_run("delete navigator.mediaDevices;");
    init::init_subsystem(InitFlags::AUDIO).unwrap();
    assert_eq!(playback_devices().unwrap().len(), 1);
    assert!(recording_devices().unwrap().is_empty());
    assert!(AudioDevice::open(AUDIO_DEVICE_DEFAULT_RECORDING, None).is_err());
}

/// The dummy driver on Emscripten: a repeating timer per device runs its
/// iterations (`ProvidesOwnCallbackThread`).
#[test]
fn dummy_driver_timers() {
    let _l = crate::test_support::test_lock();
    let _web = WebAudio::new(48000, "running");
    hints::set(hints::AUDIO_DRIVER, "dummy").unwrap();
    init::init_subsystem(InitFlags::AUDIO).unwrap();
    assert_eq!(current_audio_driver(), Some("dummy"));
    assert_eq!(
        js_str("JSON.stringify(Module['SDL3'].dummy_audio)"),
        "{\"timers\":[null,null]}"
    );

    let (pulled, callback) = counting_callback();
    let stream =
        open_audio_device_stream(AUDIO_DEVICE_DEFAULT_PLAYBACK, None, Some(callback)).unwrap();
    stream.resume_device().unwrap();
    // Every 1024 frames at 44.1 kHz (the device's default spec).
    assert_eq!(
        js_str("var M = Module['sdl3_test_audio']; M.timers.length + ' ' + M.timers[0].ms.toFixed(3) + ' ' + (Module['SDL3'].dummy_audio.timers[0] === 1)"),
        "1 23.220 true"
    );
    js_run("Module['sdl3_test_audio'].timers[0].fn();");
    assert_eq!(pulled.load(Ordering::SeqCst), 1024 * 4);

    let rec = AudioDevice::open(AUDIO_DEVICE_DEFAULT_RECORDING, None).unwrap();
    let rs = AudioStream::new(None, Some(&AudioSpec::new(AudioFormat::S16, 1, 44100))).unwrap();
    rec.bind(&rs).unwrap();
    assert_eq!(js_int("Module['SDL3'].dummy_audio.timers[1]"), 2);
    js_run("Module['sdl3_test_audio'].timers[1].fn();");
    let mut buf = vec![0xFFu8; 64];
    assert_eq!(rs.get_data(&mut buf).unwrap(), 64);
    assert!(buf.iter().all(|&b| b == 0), "silence");

    drop(stream);
    drop(rec);
    assert_eq!(
        js_str("var M = Module['sdl3_test_audio']; M.timers.map((t) => t.cleared) + ' ' + JSON.stringify(Module['SDL3'].dummy_audio)"),
        "true,true {\"timers\":[null,null]}"
    );
}
