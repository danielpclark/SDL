// Tests of the Emscripten joystick driver (the Gamepad API) in a real browser.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! The Gamepad API joystick driver (`joystick/emscripten/SDL_sysjoystick.c`)
//! in headless Chromium: a program of its own (`harness = false`) that asks
//! `tools/emscripten_run.py` for the browser, as `emscripten_audio.rs`
//! does.
//!
//! A headless browser has no gamepads, so the page gets a stand-in Gamepad
//! API: `navigator.getGamepads()` answers the program's own gamepads
//! (`Module.pads`, plain objects with the fields emscripten and the
//! driver read: `id`, `index`, `mapping`, `connected`, `timestamp`, `axes`,
//! `buttons` and `vibrationActuator`, whose `playEffect()` records its
//! arguments), `navigator.userAgent` is the program's, and connecting and
//! disconnecting one dispatches a `gamepadconnected` or
//! `gamepaddisconnected` event with it on the window, where emscripten's
//! handlers call the driver. Nothing of it is the browser's own, so it
//! doesn't depend on the browser's version.
//!
//! Each step writes a trace: the events SDL queued (joystick IDs as `#1`,
//! `#2`, … in the order they're first seen), the joysticks and what SDL
//! says of them (names, GUIDs, USB IDs, types, paths, player indices,
//! whether each is a gamepad, with which mapping), the open joysticks'
//! axes, buttons and hats and the gamepads' buttons and axes, and the
//! rumble effects played. [`EXPECTED`] is the trace of upstream's C
//! (`libSDL3.a`, SDL 3.5.0) on the same page with the same steps and the
//! same stand-in, and each step's part must be the same here. The steps:
//!
//! * `init`: a standard gamepad (Chrome's PS4 controller on Linux) already
//!   connected when SDL initializes, with the `"default"` gamepad mapping;
//! * `open`: opened as a gamepad: six axes (the triggers' two from their
//!   analog buttons), eleven buttons (no d-pad or trigger buttons) and a
//!   hat, rumble and trigger rumble; the button and the stick held when it
//!   connected aren't its state;
//! * `input`: buttons, sticks, triggers and the d-pad, as joystick and
//!   gamepad events and states; nothing for the same timestamp; up and
//!   down, left and right at once;
//! * `rumble`: the motors and the triggers through
//!   `vibrationActuator.playEffect('dual-rumble', …)`, the same values
//!   again (nothing), and the rumble stopped when it expires;
//! * `unmapped`: a gamepad without the standard mapping, as Firefox names
//!   it, on Windows, whose axis 9 is a hat; a gamepad too (the default
//!   mapping), without rumble or LED;
//! * `more`: Chrome's XInput controller on Android (an Xbox One S over
//!   Bluetooth, as it can rumble its triggers), Firefox's standard gamepad
//!   on a Mac (the triggers as axes), and the same gamepads connected
//!   again (ignored);
//! * `disconnect`: an open gamepad and another disconnected;
//! * `reconnect`: a new instance at the same index, and a Chrome gamepad
//!   with USB IDs but no mapping;
//! * `quit`: the subsystem quit (the event handlers removed), then
//!   initialized again with the gamepads connected then, and `SDL_Quit()`.
//!
//! Each step prints a libtest-like `test NAME ... ok` line (with both
//! traces when they differ); under node everything skips as `browser`.
//! `SDL3_EMSCRIPTEN_BROWSER=1 SDL3_TEST_REQUIRE=browser cargo test -p sdl3
//! --target wasm32-unknown-emscripten --test emscripten_joystick` runs it
//! as CI does.

#[cfg(not(target_os = "emscripten"))]
fn main() {}

#[cfg(target_os = "emscripten")]
fn main() -> std::process::ExitCode {
    gamepads::main()
}

/// The program's request for the browser (see `emscripten_browser.rs`).
#[cfg(target_os = "emscripten")]
#[used]
#[link_section = "sdl3_runner"]
static RUNNER: [u8; 7] = *b"browser";

#[cfg(target_os = "emscripten")]
mod gamepads {
    use std::fmt::Write as _;
    use std::panic::{self, AssertUnwindSafe};
    use std::process::ExitCode;
    use std::sync::Mutex;
    use std::time::{Duration, Instant};

    use sdl3::events::{self, Event, EventType, JoystickID};
    use sdl3::init::{self, InitFlags};
    use sdl3::joystick::gamepad::{self, Gamepad, GamepadAxis, GamepadButton};
    use sdl3::joystick::{
        self, Joystick, PROP_JOYSTICK_CAP_RGB_LED_BOOLEAN, PROP_JOYSTICK_CAP_RUMBLE_BOOLEAN,
        PROP_JOYSTICK_CAP_TRIGGER_RUMBLE_BOOLEAN,
    };
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

    /// Run JavaScript statements.
    fn js_run(code: &str) {
        js(&format!("(function() {{ {code} }})()"));
    }

    // The snippets the C reference runs too, verbatim.

    /// The stand-in Gamepad API (see the module's documentation):
    /// `Module.pads` (`P`), four slots of gamepads; `P.ua` the user agent;
    /// `P.pad(index, id, mapping, buttons, axes)` makes a gamepad (released,
    /// centered, timestamp 1); `P.actuator(effects)` a `vibrationActuator`
    /// whose effects go into `P.rumbles`; `P.press(g, i, value)` sets a
    /// button; `P.connect(index, g)` and `P.disconnect(index)` change the
    /// slots and dispatch the events.
    const MOCK_INSTALL: &str = "var P = Module.pads = [null, null, null, null]; \
P.ua = 'Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/141.0.0.0 Safari/537.36'; \
P.rumbles = []; \
P.pad = function(index, id, mapping, buttons, axes) { var b = []; for (var i = 0; i < buttons; i++) b.push({ pressed: false, touched: false, value: 0 }); \
return { id: id, index: index, mapping: mapping, connected: true, timestamp: 1, axes: new Array(axes).fill(0), buttons: b }; }; \
P.actuator = function(effects) { return { effects: effects, playEffect: function(type, params) { P.rumbles.push(type + ' ' + JSON.stringify(params)); return Promise.resolve('complete'); } }; }; \
P.press = function(g, i, v) { g.buttons[i].pressed = v > 0; g.buttons[i].touched = v > 0; g.buttons[i].value = v; }; \
P.connect = function(index, g) { P[index] = g; var e = new Event('gamepadconnected'); e.gamepad = g; window.dispatchEvent(e); }; \
P.disconnect = function(index) { var g = P[index]; g.connected = false; P[index] = null; var e = new Event('gamepaddisconnected'); e.gamepad = g; window.dispatchEvent(e); }; \
Object.defineProperty(navigator, 'getGamepads', { value: function() { return P.slice(0, 4); }, configurable: true, writable: true }); \
Object.defineProperty(navigator, 'userAgent', { get: function() { return P.ua; }, configurable: true });";

    const PS4_ID: &str = "'Wireless Controller (STANDARD GAMEPAD Vendor: 054c Product: 09cc)'";

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

    /// The joystick IDs seen, in order: `#1` is the first.
    static SEEN: Mutex<Vec<JoystickID>> = Mutex::new(Vec::new());

    fn tag(id: JoystickID) -> usize {
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

    /// The `n`-th joystick ID seen (from 0).
    fn seen(n: usize) -> JoystickID {
        SEEN.lock().unwrap()[n]
    }

    fn ok(r: bool) -> i32 {
        r as i32
    }

    /// The events queued (taken from the queue without pumping it).
    fn events() {
        for e in events::get_events(EventType::FIRST, EventType::LAST, 64).unwrap() {
            match e {
                Event::JoyDevice(e) => {
                    let kind = match e.event_type {
                        EventType::JOYSTICK_ADDED => "added",
                        EventType::JOYSTICK_REMOVED => "removed",
                        EventType::JOYSTICK_UPDATE_COMPLETE => "update complete",
                        _ => "?",
                    };
                    out!("event joy {kind} #{}", tag(e.which));
                }
                Event::JoyAxis(e) => {
                    out!("event joy axis #{} {} {}", tag(e.which), e.axis, e.value)
                }
                Event::JoyHat(e) => out!("event joy hat #{} {} {}", tag(e.which), e.hat, e.value),
                Event::JoyButton(e) => out!(
                    "event joy button #{} {} {}",
                    tag(e.which),
                    e.button,
                    ok(e.down)
                ),
                Event::GamepadDevice(e) => {
                    let kind = match e.event_type {
                        EventType::GAMEPAD_ADDED => "added",
                        EventType::GAMEPAD_REMOVED => "removed",
                        EventType::GAMEPAD_REMAPPED => "remapped",
                        EventType::GAMEPAD_UPDATE_COMPLETE => "update complete",
                        _ => "?",
                    };
                    out!("event gamepad {kind} #{}", tag(e.which));
                }
                Event::GamepadAxis(e) => out!(
                    "event gamepad axis #{} {} {}",
                    tag(e.which),
                    e.axis,
                    e.value
                ),
                Event::GamepadButton(e) => out!(
                    "event gamepad button #{} {} {}",
                    tag(e.which),
                    e.button,
                    ok(e.down)
                ),
                e => out!("event {:#x}", e.event_type().0),
            }
        }
    }

    fn list() {
        let ids = joystick::joysticks();
        let tags: String = ids.iter().map(|&id| format!(" #{}", tag(id))).collect();
        out!("joysticks {}:{tags}", ids.len());
    }

    fn info(id: JoystickID) {
        let name = joystick::joystick_name_for_id(id).ok().flatten();
        out!(
            "#{} name {} guid {}",
            tag(id),
            name.as_deref().unwrap_or("(null)"),
            joystick::joystick_guid_for_id(id)
        );
        let path = match joystick::joystick_path_for_id(id) {
            Ok(p) => p,
            Err(e) => e.message().to_owned(),
        };
        out!(
            "#{} ids {:04x} {:04x} {:04x} type {} path {path} player {}",
            tag(id),
            joystick::joystick_vendor_for_id(id),
            joystick::joystick_product_for_id(id),
            joystick::joystick_product_version_for_id(id),
            joystick::joystick_type_for_id(id) as i32,
            joystick::joystick_player_index_for_id(id)
        );
        let gname = gamepad::gamepad_name_for_id(id).ok().flatten();
        out!(
            "#{} gamepad {} {} type {}",
            tag(id),
            ok(gamepad::is_gamepad(id)),
            gname.as_deref().unwrap_or("(null)"),
            gamepad::gamepad_type_for_id(id)
                .as_str()
                .unwrap_or("(null)")
        );
        out!(
            "#{} mapping {}",
            tag(id),
            gamepad::gamepad_mapping_for_id(id)
                .as_deref()
                .unwrap_or("(null)")
        );
    }

    fn opened(j: &Joystick) {
        let props = j.properties().unwrap();
        let cap = |name: &str| ok(props.get_bool(name).unwrap_or(false));
        let serial = j.serial().ok().flatten();
        out!(
            "open #{} axes {} buttons {} hats {} balls {} rumble {} trigger rumble {} led {} serial {}",
            tag(j.id()),
            j.num_axes().unwrap(),
            j.num_buttons().unwrap(),
            j.num_hats().unwrap(),
            j.num_balls().unwrap(),
            cap(PROP_JOYSTICK_CAP_RUMBLE_BOOLEAN),
            cap(PROP_JOYSTICK_CAP_TRIGGER_RUMBLE_BOOLEAN),
            cap(PROP_JOYSTICK_CAP_RGB_LED_BOOLEAN),
            serial.as_deref().unwrap_or("(null)")
        );
    }

    fn jstate(j: &Joystick) {
        let axes: String = (0..j.num_axes().unwrap_or(0))
            .map(|i| format!(" {}", j.axis(i).unwrap_or(0)))
            .collect();
        let buttons: String = (0..j.num_buttons().unwrap_or(0))
            .map(|i| {
                if j.button(i).unwrap_or(false) {
                    '1'
                } else {
                    '0'
                }
            })
            .collect();
        let hats: String = (0..j.num_hats().unwrap_or(0))
            .map(|i| format!(" {}", j.hat(i).unwrap_or(0)))
            .collect();
        out!(
            "#{} connected {} axes{axes} buttons {buttons} hats{hats}",
            tag(j.id()),
            ok(j.connected())
        );
    }

    fn gstate(g: &Gamepad) {
        let axes: String = GamepadAxis::all()
            .map(|a| format!(" {}", g.axis(a)))
            .collect();
        let buttons: String = GamepadButton::all()
            .map(|b| if g.button(b) { '1' } else { '0' })
            .collect();
        out!("#{} gamepad axes{axes} buttons {buttons}", tag(g.id()));
    }

    fn rumbles() {
        out!(
            "rumbles {}",
            js("Module.pads.rumbles.splice(0).join(' | ') || '(none)'")
        );
    }

    /// `r` as upstream prints a result and the error after it (only the
    /// failures are printed so: the error after a success is stale).
    fn result(r: sdl3::error::Result<()>) -> String {
        match r {
            Ok(()) => "1".to_owned(),
            Err(e) => format!("0 {}", e.message()),
        }
    }

    fn spin(ms: u64) {
        let t = Instant::now();
        while t.elapsed() < Duration::from_millis(ms) {}
    }

    fn update() {
        joystick::update_joysticks();
    }

    fn run() {
        step("init");
        js_run(MOCK_INSTALL);
        // A standard gamepad (Chrome's PS4 controller) already connected, with
        // a button held and a stick pushed, and rumble for its motors and triggers
        js_run(&format!("var g = Module.pads.pad(0, {PS4_ID}, 'standard', 17, 4); g.vibrationActuator = Module.pads.actuator(['dual-rumble', 'trigger-rumble']); \
Module.pads.press(g, 1, 1); g.axes[1] = 0.25; Module.pads[0] = g;"));
        out!("init {}", ok(init::init(InitFlags::GAMEPAD).is_ok()));
        events();
        list();
        info(seen(0));

        step("open");
        let pad = Gamepad::open(seen(0)).unwrap();
        let joy = pad.joystick().unwrap();
        opened(&joy);
        events();
        jstate(&joy);
        gstate(&pad);
        let again = Joystick::open(seen(0)).map(|j| j.id());
        out!("open again: {}", ok(again.ok() == Some(joy.id())));

        step("input");
        js_run("var P = Module.pads, g = P[0]; P.press(g, 0, 1); P.press(g, 1, 0); P.press(g, 9, 1); P.press(g, 16, 1); P.press(g, 7, 0.5); P.press(g, 12, 1); P.press(g, 15, 1); \
g.axes[0] = 0.5; g.axes[1] = 0.25; g.axes[3] = -1; g.timestamp = 2;");
        update();
        events();
        jstate(&joy);
        gstate(&pad);
        // The same timestamp: nothing
        js_run("var P = Module.pads, g = P[0]; P.press(g, 2, 1);");
        update();
        events();
        // Up and down, left and right at once; the triggers, released and full
        js_run("var P = Module.pads, g = P[0]; P.press(g, 13, 1); P.press(g, 14, 1); P.press(g, 7, 1); P.press(g, 6, 1); P.press(g, 0, 0); g.axes[0] = -0.75; g.timestamp = 3;");
        update();
        events();
        jstate(&joy);
        gstate(&pad);
        js_run("var P = Module.pads, g = P[0]; for (var i of [2, 6, 7, 9, 12, 13, 14, 15, 16]) P.press(g, i, 0); g.axes = [0, 0, 0, 0]; g.timestamp = 4;");
        update();
        events();
        jstate(&joy);

        step("rumble");
        out!("rumble {}", result(pad.rumble(0x4000, 0x8000, 100)));
        rumbles();
        out!("rumble again {}", result(pad.rumble(0x4000, 0x8000, 100)));
        rumbles();
        out!(
            "triggers {}",
            result(pad.rumble_triggers(0xFFFF, 0x1234, 100))
        );
        rumbles();
        spin(150);
        update();
        events();
        out!("expired:");
        rumbles();

        step("unmapped");
        // A gamepad the browser doesn't map, as Firefox names it, on Windows:
        // its axis 9 is a hat
        js_run("var P = Module.pads; P.ua = 'Mozilla/5.0 (Windows NT 10.0; Win64; x64; rv:140.0) Gecko/20100101 Firefox/140.0'; \
var g = P.pad(1, '046d-c216-Logitech Dual Action', '', 12, 10); g.axes[9] = 3.2857; P.connect(1, g);");
        events();
        list();
        info(seen(1));
        let joy2 = Joystick::open(seen(1)).unwrap();
        opened(&joy2);
        events();
        jstate(&joy2);
        out!("rumble {}", result(joy2.rumble(1, 1, 100)));
        out!("triggers {}", result(joy2.rumble_triggers(1, 1, 100)));
        out!("led {}", result(joy2.set_led(1, 2, 3)));
        js_run("var P = Module.pads, g = P[1]; g.axes[9] = 5 / 7; g.axes[2] = -0.25; P.press(g, 11, 1); g.timestamp = 2;");
        update();
        events();
        jstate(&joy2);
        js_run("var P = Module.pads, g = P[1]; g.axes[9] = -1; g.timestamp = 3;");
        update();
        events();
        js_run("var P = Module.pads, g = P[1]; g.axes[9] = 3.2857; g.timestamp = 4;");
        update();
        events();
        jstate(&joy2);

        step("more");
        // Chrome's XInput controller on Android (trigger rumble: an Xbox One S
        // over Bluetooth), Firefox's standard gamepad (the triggers as axes) on
        // a Mac, a Chrome gamepad with IDs but no mapping, and a duplicate
        js_run("var P = Module.pads; P.ua = 'Mozilla/5.0 (Linux; Android 14; Pixel 8) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/141.0.0.0 Mobile Safari/537.36'; \
var g = P.pad(2, 'Xbox 360 Controller (XInput STANDARD GAMEPAD)', 'standard', 17, 4); g.vibrationActuator = P.actuator(['dual-rumble', 'trigger-rumble']); P.connect(2, g); \
P.ua = 'Mozilla/5.0 (Macintosh; Intel Mac OS X 10.15; rv:140.0) Gecko/20100101 Firefox/140.0'; \
g = P.pad(3, '45e-2ea-Microsoft X-Box One S pad', 'standard', 17, 6); g.vibrationActuator = P.actuator(['dual-rumble']); P.connect(3, g); P.connect(3, g);");
        events();
        list();
        info(seen(2));
        info(seen(3));
        let pad3 = Gamepad::open(seen(3)).unwrap();
        let joy3 = pad3.joystick().unwrap();
        opened(&joy3);
        events();
        out!("triggers {}", result(pad3.rumble_triggers(1, 1, 100)));
        js_run("var P = Module.pads, g = P[3]; g.axes[2] = 1; g.axes[5] = 0.5; P.press(g, 6, 1); P.press(g, 8, 1); g.timestamp = 2;");
        update();
        events();
        jstate(&joy3);
        gstate(&pad3);
        drop(joy3);
        drop(pad3);
        js_run("var P = Module.pads; P.ua = 'Mozilla/5.0 (X11; CrOS x86_64 14541.0.0)'; P.connect(0, P[0]);");
        events();
        list();

        step("disconnect");
        js_run("Module.pads.disconnect(0);");
        events();
        list();
        out!("connected {} {}", ok(pad.connected()), ok(joy.connected()));
        out!("rumble {}", result(pad.rumble(1, 1, 100)));
        js_run("Module.pads.disconnect(2);");
        events();
        list();
        update();
        events();
        jstate(&joy);
        drop(joy);
        drop(pad);
        events();

        step("reconnect");
        js_run(&format!("var P = Module.pads; P.ua = 'Mozilla/5.0 (X11; Linux x86_64)'; var g = P.pad(0, {PS4_ID}, 'standard', 17, 4); P.connect(0, g); \
g = P.pad(2, 'usb gamepad            (Vendor: 0810 Product: e501)', '', 10, 2); P.connect(2, g);"));
        events();
        list();
        info(seen(4));
        info(seen(5));
        let pad = Gamepad::open(seen(4)).unwrap();
        let joy = pad.joystick().unwrap();
        opened(&joy);
        events();
        out!("rumble {}", result(pad.rumble(1, 1, 100)));
        drop(joy);
        drop(pad);
        drop(joy2);

        step("quit");
        init::quit_subsystem(InitFlags::GAMEPAD);
        out!("after quit: has joystick {}", ok(joystick::has_joystick()));
        js_run("var P = Module.pads; P.disconnect(1); P.connect(1, P.pad(1, '046d-c216-Logitech Dual Action', '', 12, 10));");
        out!("init {}", ok(init::init(InitFlags::JOYSTICK).is_ok()));
        events();
        list();
        for id in joystick::joysticks() {
            let name = joystick::joystick_name_for_id(id).ok().flatten();
            out!("#{} {}", tag(id), name.as_deref().unwrap_or("(null)"));
        }
        init::quit();
        out!(
            "after SDL_Quit: has joystick {}",
            ok(joystick::has_joystick())
        );
        out!("done");
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
        if panic::catch_unwind(AssertUnwindSafe(run)).is_err() {
            out!("FAILED: panicked");
            init::quit();
        }
        if finish() {
            ExitCode::SUCCESS
        } else {
            ExitCode::FAILURE
        }
    }

    /// The trace of upstream's C on the same page with the same steps.
    const EXPECTED: &str = r#"== init
init 1
event joy added #1
event gamepad added #1
joysticks 1: #1
#1 name Wireless Controller guid 03009b514c050000cc09000000000082
#1 ids 054c 09cc 0000 type 1 path That operation is not supported player 0
#1 gamepad 1 Wireless Controller type ps4
#1 mapping 03009b514c050000cc09000000000082,*,a:b0,b:b1,back:b6,dpdown:h0.4,dpleft:h0.8,dpright:h0.2,dpup:h0.1,guide:b10,leftshoulder:b4,leftstick:b8,lefttrigger:a4,leftx:a0,lefty:a1,rightshoulder:b5,rightstick:b9,righttrigger:a5,rightx:a2,righty:a3,start:b7,x:b2,y:b3,
== open
open #1 axes 6 buttons 11 hats 1 balls 0 rumble 1 trigger rumble 1 led 0 serial (null)
#1 connected 1 axes 0 0 0 0 -32768 -32768 buttons 00000000000 hats 0
#1 gamepad axes 0 0 0 0 0 0 buttons 00000000000000000000000000
open again: 1
== input
event gamepad button #1 0 1
event joy button #1 0 1
event gamepad button #1 6 1
event joy button #1 7 1
event gamepad button #1 5 1
event joy button #1 10 1
event gamepad button #1 14 1
event gamepad button #1 11 1
event joy hat #1 0 3
event gamepad update complete #1
event joy update complete #1
#1 connected 1 axes 16383 0 0 -32767 -32767 0 buttons 10000001001 hats 3
#1 gamepad axes 16383 0 0 -32767 0 16383 buttons 10000110000100100000000000
event gamepad button #1 0 0
event joy button #1 0 0
event gamepad button #1 2 1
event joy button #1 2 1
event gamepad axis #1 0 16383
event joy axis #1 0 16383
event gamepad axis #1 0 -24575
event joy axis #1 0 -24575
event gamepad axis #1 4 0
event joy axis #1 4 -32767
event gamepad axis #1 4 32767
event joy axis #1 4 32767
event gamepad axis #1 5 16383
event joy axis #1 5 0
event gamepad axis #1 5 32767
event joy axis #1 5 32767
event gamepad button #1 13 1
event gamepad button #1 14 0
event joy hat #1 0 9
event gamepad update complete #1
event joy update complete #1
#1 connected 1 axes -24575 0 0 -32767 32767 32767 buttons 00100001001 hats 9
#1 gamepad axes -24575 0 0 -32767 32767 32767 buttons 00100110000101000000000000
event gamepad button #1 2 0
event joy button #1 2 0
event gamepad button #1 6 0
event joy button #1 7 0
event joy button #1 10 0
event gamepad axis #1 0 0
event joy axis #1 0 0
event gamepad axis #1 4 0
event joy axis #1 4 -32767
event gamepad axis #1 5 0
event joy axis #1 5 -32767
event gamepad button #1 13 0
event gamepad button #1 11 0
event joy hat #1 0 0
event gamepad update complete #1
event joy update complete #1
#1 connected 1 axes 0 0 0 0 -32767 -32767 buttons 00000000000 hats 0
== rumble
rumble 1
rumbles dual-rumble {"startDelay":0,"duration":3000,"weakMagnitude":0.5000076295109483,"strongMagnitude":0.2500038147554742,"leftTrigger":0,"rightTrigger":0}
rumble again 1
rumbles (none)
triggers 1
rumbles dual-rumble {"startDelay":0,"duration":3000,"weakMagnitude":0.5000076295109483,"strongMagnitude":0.2500038147554742,"leftTrigger":1,"rightTrigger":0.07110704203860532}
event gamepad update complete #1
event joy update complete #1
expired:
rumbles dual-rumble {"startDelay":0,"duration":3000,"weakMagnitude":0,"strongMagnitude":0,"leftTrigger":1,"rightTrigger":0.07110704203860532} | dual-rumble {"startDelay":0,"duration":3000,"weakMagnitude":0,"strongMagnitude":0,"leftTrigger":0,"rightTrigger":0}
== unmapped
event joy added #2
event gamepad added #2
joysticks 2: #1 #2
#2 name Logitech Dual Action guid 0300040e6d04000016c2000000000005
#2 ids 046d c216 0000 type 1 path That operation is not supported player 1
#2 gamepad 1 Logitech Dual Action type standard
#2 mapping 0300040e6d04000016c2000000000005,*,a:b0,b:b1,back:b6,dpdown:h0.4,dpleft:h0.8,dpright:h0.2,dpup:h0.1,guide:b10,leftshoulder:b4,leftstick:b8,lefttrigger:a4,leftx:a0,lefty:a1,rightshoulder:b5,rightstick:b9,righttrigger:a5,rightx:a2,righty:a3,start:b7,x:b2,y:b3,
open #2 axes 10 buttons 12 hats 1 balls 0 rumble 0 trigger rumble 0 led 0 serial (null)
#2 connected 1 axes 0 0 0 0 0 0 0 0 0 0 buttons 000000000000 hats 0
rumble 0 That operation is not supported
triggers 0 That operation is not supported
led 0 That operation is not supported
event joy button #2 11 1
event joy hat #2 0 8
event joy update complete #2
event gamepad update complete #1
event joy update complete #1
#2 connected 1 axes 0 0 -8191 0 0 0 0 0 0 0 buttons 000000000001 hats 8
event joy hat #2 0 1
event joy update complete #2
event gamepad update complete #1
event joy update complete #1
event joy hat #2 0 0
event joy update complete #2
event gamepad update complete #1
event joy update complete #1
#2 connected 1 axes 0 0 -8191 0 0 0 0 0 0 0 buttons 000000000001 hats 0
== more
event joy added #3
event gamepad added #3
event joy added #4
event gamepad added #4
joysticks 4: #1 #2 #3 #4
#3 name Xbox One S Controller guid 050003f05e040000e002000000000081
#3 ids 045e 02e0 0000 type 1 path That operation is not supported player 2
#3 gamepad 1 Xbox One S Controller type xboxone
#3 mapping 050003f05e040000e002000000000081,*,a:b0,b:b1,back:b6,dpdown:h0.4,dpleft:h0.8,dpright:h0.2,dpup:h0.1,guide:b10,leftshoulder:b4,leftstick:b8,lefttrigger:a4,leftx:a0,lefty:a1,rightshoulder:b5,rightstick:b9,righttrigger:a5,rightx:a2,righty:a3,start:b7,x:b2,y:b3,
#4 name Xbox One S Controller guid 0300e8fb5e040000ea02000000000084
#4 ids 045e 02ea 0000 type 1 path That operation is not supported player 3
#4 gamepad 1 Xbox One S Controller type xboxone
#4 mapping 0300e8fb5e040000ea02000000000084,*,a:b0,b:b1,back:b6,dpdown:h0.4,dpleft:h0.8,dpright:h0.2,dpup:h0.1,guide:b10,leftshoulder:b4,leftstick:b8,lefttrigger:a4,leftx:a0,lefty:a1,rightshoulder:b5,rightstick:b9,righttrigger:a5,rightx:a2,righty:a3,start:b7,x:b2,y:b3,
open #4 axes 6 buttons 11 hats 1 balls 0 rumble 1 trigger rumble 0 led 0 serial (null)
triggers 0 That operation is not supported
event gamepad button #4 4 1
event joy button #4 6 1
event gamepad update complete #4
event joy update complete #4
event gamepad update complete #1
event joy update complete #1
#4 connected 1 axes 0 0 32767 0 -32768 16383 buttons 00000010000 hats 0
#4 gamepad axes 0 0 32767 0 0 24575 buttons 00001000000000000000000000
joysticks 4: #1 #2 #3 #4
== disconnect
event gamepad axis #1 0 16383
event joy axis #1 0 16383
event gamepad axis #1 5 16383
event joy axis #1 5 0
event gamepad axis #1 0 0
event gamepad axis #1 5 0
event gamepad removed #1
event joy removed #1
joysticks 3: #2 #3 #4
connected 0 0
rumble 0 That operation is not supported
event gamepad removed #3
event joy removed #3
joysticks 2: #2 #4
event gamepad update complete #1
event joy update complete #1
#1 connected 0 axes 16383 0 0 0 -32767 0 buttons 00000000000 hats 0
== reconnect
event joy added #5
event gamepad added #5
event joy added #6
event gamepad added #6
joysticks 4: #2 #4 #5 #6
#5 name Wireless Controller guid 03009b514c050000cc09000000000082
#5 ids 054c 09cc 0000 type 1 path That operation is not supported player 0
#5 gamepad 1 Wireless Controller type ps4
#5 mapping 03009b514c050000cc09000000000082,*,a:b0,b:b1,back:b6,dpdown:h0.4,dpleft:h0.8,dpright:h0.2,dpup:h0.1,guide:b10,leftshoulder:b4,leftstick:b8,lefttrigger:a4,leftx:a0,lefty:a1,rightshoulder:b5,rightstick:b9,righttrigger:a5,rightx:a2,righty:a3,start:b7,x:b2,y:b3,
#6 name usb gamepad guid 03007e411008000001e5000000000002
#6 ids 0810 e501 0000 type 1 path That operation is not supported player 2
#6 gamepad 1 usb gamepad type standard
#6 mapping 03007e411008000001e5000000000002,*,a:b0,b:b1,back:b6,dpdown:h0.4,dpleft:h0.8,dpright:h0.2,dpup:h0.1,guide:b10,leftshoulder:b4,leftstick:b8,lefttrigger:a4,leftx:a0,lefty:a1,rightshoulder:b5,rightstick:b9,righttrigger:a5,rightx:a2,righty:a3,start:b7,x:b2,y:b3,
open #5 axes 6 buttons 11 hats 1 balls 0 rumble 0 trigger rumble 0 led 0 serial (null)
rumble 0 That operation is not supported
== quit
after quit: has joystick 0
init 1
event joy added #7
event joy added #8
event joy added #9
event joy added #10
joysticks 4: #7 #8 #9 #10
#7 Wireless Controller
#8 Logitech Dual Action
#9 usb gamepad
#10 Xbox One S Controller
after SDL_Quit: has joystick 0
done
"#;
}
