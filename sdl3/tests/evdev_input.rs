// Tests of the console input reader (the evdev reader of the KMS/DRM driver
// and of the dummy driver's `evdev` variant) on uinput devices.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! A program (`harness = false`), run by `tools/linux_console_run.py`
//! (which makes its uinput devices, as it asks), on the dummy video
//! driver's `evdev` variant (`SDL_VIDEO_DRIVER=evdev`: a 1024x768 display,
//! and its window, which has the focus), tracing the input devices and the
//! events from:
//!
//! * a keyboard: keys, Shift, a repeat, the pad with Num Lock, the
//!   built-in keymap's Caps Lock (a Control lock, whose keymap isn't
//!   built in), Control (likewise), an unknown key; their text through
//!   the console keyboard's built-in keymaps (the runner leaves the
//!   program no console);
//! * a mouse with high-resolution wheels: motion, its eight buttons, the
//!   wheels (their low-resolution events ignored), and the relative mode
//!   (the reader's own, a mouse-state hook here);
//! * an absolute pointer (a virtual machine's tablet): positions on the
//!   display;
//! * a multitouch screen: two fingers down, moved and up (and the mouse
//!   events SDL makes of the first), then the reader's buffer overflowed
//!   (`SYN_DROPPED`) while a finger came down, which the reader finds by
//!   asking for the slots (`EVIOCGMTSLOTS`);
//! * hotplug: a keyboard added, typed on and removed, and the tablet
//!   removed (the udev monitor's, waited for by name, within 10 s).
//!
//! The kernel has a request's events in the reader's buffers when the
//! runner answers (uinput writes them through at once), so the events of
//! each step are read by one pump; only the hotplug events, which come
//! through udev, are waited for.
//!
//! The expected trace is upstream's C SDL's, from the same program in C
//! (upstream built with CMake at ea7a2da, run by the same runner on the
//! same CI runner). Without the runner, it runs itself under it when
//! /dev/uinput is writable, and skips (capability `uinput`) otherwise,
//! unless `SDL3_TEST_REQUIRE` asks for it. Elsewhere than Linux it does
//! nothing.

#[cfg(not(target_os = "linux"))]
fn main() {}

#[cfg(target_os = "linux")]
fn main() -> std::process::ExitCode {
    app::main()
}

#[cfg(target_os = "linux")]
mod app {
    use std::collections::HashMap;
    use std::io::{BufRead, Write};
    use std::process::ExitCode;
    use std::time::{Duration, Instant};

    use sdl3::events::keyboard;
    use sdl3::events::window::WindowFlags;
    use sdl3::events::{self, mouse, touch, Event, EventType};
    use sdl3::hints;
    use sdl3::init::{self, InitFlags};
    use sdl3::video::{self, Window};

    /// Upstream's trace (the C program, by the runner, on the CI runner).
    const EXPECTED: &str = "\
";

    const PREFIX: &str = "sdl3-console-runner: ";
    const KEYBOARD: &str = "SDL3 evdev keyboard";
    const MOUSE: &str = "SDL3 evdev mouse";
    const TABLET: &str = "SDL3 evdev tablet";
    const TOUCHSCREEN: &str = "SDL3 evdev touchscreen";
    const KEYBOARD2: &str = "SDL3 evdev keyboard 2";

    /// Whether `SDL3_TEST_REQUIRE` makes a capability mandatory.
    fn required(capability: &str) -> bool {
        std::env::var("SDL3_TEST_REQUIRE")
            .unwrap_or_default()
            .split(',')
            .map(str::trim)
            .any(|c| c == "all" || c == capability)
    }

    /// Skip (all of) the program, as the library's tests do.
    fn skip(capability: &str, reason: &str) -> ExitCode {
        assert!(
            !required(capability),
            "required capability {capability} unavailable: {reason}"
        );
        eprintln!("note: skipping, {capability} unavailable: {reason}");
        if let Ok(path) = std::env::var("SDL3_TEST_SKIP_LOG") {
            if let Ok(mut file) = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(path)
            {
                let _ = writeln!(file, "{capability}\tevdev_input\t{reason}");
            }
        }
        ExitCode::SUCCESS
    }

    /// Ask the runner, and return its answer's lines.
    fn request(line: &str) -> Vec<String> {
        println!("{PREFIX}request {line}");
        std::io::stdout().flush().unwrap();
        let mut replies = Vec::new();
        for answer in std::io::stdin().lock().lines() {
            let answer = answer.expect("the runner's answer");
            if answer == format!("{PREFIX}done") {
                return replies;
            }
            if let Some(reply) = answer.strip_prefix(&format!("{PREFIX}reply ")) {
                replies.push(reply.to_owned());
            }
        }
        panic!("the runner ended without answering {line:?}");
    }

    /// The trace, printed as it's made.
    struct Trace {
        lines: Vec<String>,
        /// The names of the devices by ID (kept after they're removed)
        keyboards: HashMap<u32, String>,
        mice: HashMap<u32, String>,
        window: events::WindowID,
    }

    impl Trace {
        fn line(&mut self, line: String) {
            println!("trace: {line}");
            self.lines.push(line);
        }

        fn ours(name: &str) -> bool {
            name.starts_with("SDL3 evdev")
        }

        fn window(&self, id: events::WindowID) -> &'static str {
            if id == self.window {
                "window"
            } else if id == 0 {
                "none"
            } else {
                "another"
            }
        }

        fn mouse(&self, which: mouse::MouseID) -> String {
            match which {
                mouse::GLOBAL_MOUSE_ID => "global".into(),
                mouse::TOUCH_MOUSE_ID => "touch".into(),
                mouse::PEN_MOUSE_ID => "pen".into(),
                id => self
                    .mice
                    .get(&id)
                    .cloned()
                    .unwrap_or_else(|| "other".into()),
            }
        }

        /// The event's line, if it's one of the traced ones (of these
        /// devices).
        fn describe(&mut self, event: &Event) -> Option<String> {
            match event {
                Event::KeyboardDevice(d) => {
                    let added = d.event_type == EventType::KEYBOARD_ADDED;
                    if added {
                        let name = keyboard::keyboard_name(d.which).unwrap_or_default();
                        self.keyboards.insert(d.which, name);
                    }
                    let name = self.keyboards.get(&d.which).cloned().unwrap_or_default();
                    Self::ours(&name).then(|| {
                        format!(
                            "keyboard {}: {name}",
                            if added { "added" } else { "removed" }
                        )
                    })
                }
                Event::MouseDevice(d) => {
                    let added = d.event_type == EventType::MOUSE_ADDED;
                    if added {
                        let name = mouse::mouse_name(d.which).unwrap_or_default();
                        self.mice.insert(d.which, name);
                    }
                    let name = self.mice.get(&d.which).cloned().unwrap_or_default();
                    Self::ours(&name).then(|| {
                        format!("mouse {}: {name}", if added { "added" } else { "removed" })
                    })
                }
                Event::Key(k) => Some(format!(
                    "key {} {} key {} raw {} mod 0x{:04x} repeat {} {} from {}",
                    if k.down { "down" } else { "up" },
                    k.scancode.name(),
                    k.key.name(),
                    k.raw,
                    k.modifiers.0,
                    k.repeat as i32,
                    self.window(k.window_id),
                    self.keyboards.get(&k.which).map_or("other", String::as_str)
                )),
                Event::TextInput(t) => {
                    Some(format!("text \"{}\" {}", t.text, self.window(t.window_id)))
                }
                Event::MouseMotion(m) => Some(format!(
                    "motion {} at ({:.4}, {:.4}) rel ({:.4}, {:.4}) state 0x{:x} {}",
                    self.mouse(m.which),
                    m.x,
                    m.y,
                    m.xrel,
                    m.yrel,
                    m.state.0,
                    self.window(m.window_id)
                )),
                Event::MouseButton(b) => Some(format!(
                    "button {} {} {} at ({:.4}, {:.4}) {}",
                    if b.down { "down" } else { "up" },
                    b.button,
                    self.mouse(b.which),
                    b.x,
                    b.y,
                    self.window(b.window_id)
                )),
                Event::MouseWheel(w) => Some(format!(
                    "wheel {} ({:.4}, {:.4}) integer ({}, {}) direction {} at ({:.4}, {:.4}) {}",
                    self.mouse(w.which),
                    w.x,
                    w.y,
                    w.integer_x,
                    w.integer_y,
                    w.direction as i32,
                    w.mouse_x,
                    w.mouse_y,
                    self.window(w.window_id)
                )),
                Event::TouchFinger(f) => Some(format!(
                    "finger {} {} on {} at ({:.4}, {:.4}) delta ({:.4}, {:.4}) pressure {:.4} {}",
                    match f.event_type {
                        EventType::FINGER_DOWN => "down",
                        EventType::FINGER_UP => "up",
                        EventType::FINGER_MOTION => "motion",
                        _ => "canceled",
                    },
                    f.finger_id,
                    touch::touch_device_name(f.touch_id).unwrap_or_default(),
                    f.x,
                    f.y,
                    f.dx,
                    f.dy,
                    f.pressure,
                    self.window(f.window_id)
                )),
                _ => None,
            }
        }

        /// Pump once, and trace the events (all of a request's events are
        /// read by then).
        fn pump(&mut self) {
            events::pump();
            while let Some(event) = events::poll() {
                if let Some(line) = self.describe(&event) {
                    self.line(line);
                }
            }
        }

        /// Pump until the line comes (the hotplug events, through udev's
        /// monitor), tracing the events, for up to 10 s.
        fn wait_for(&mut self, line: &str) {
            let end = Instant::now() + Duration::from_secs(10);
            loop {
                events::pump();
                while let Some(event) = events::poll() {
                    if let Some(l) = self.describe(&event) {
                        let done = l == line;
                        self.line(l);
                        if done {
                            return;
                        }
                    }
                }
                if Instant::now() >= end {
                    self.line(format!("timeout: no {line:?} in 10 s"));
                    return;
                }
                std::thread::sleep(Duration::from_millis(10));
            }
        }

        /// A request of events (shell words: TYPE CODE VALUE ...) for a
        /// device, then the events they make.
        fn events(&mut self, device: &str, events: &str) {
            for reply in request(&format!("uinput events '{device}' {events}")) {
                if !reply.starts_with("uinput: sent ") {
                    self.line(reply);
                }
            }
            self.pump();
        }
    }

    /// A key's press and release (and SYN_REPORT after each).
    fn tap(key: &str) -> String {
        format!("EV_KEY {key} 1 EV_SYN SYN_REPORT 0 EV_KEY {key} 0 EV_SYN SYN_REPORT 0")
    }

    pub(super) fn main() -> ExitCode {
        if std::env::var_os("SDL3_CONSOLE_RUNNER").is_none() {
            // (as root, or with a udev rule: see tools/linux_console_run.py)
            // SAFETY: access() reads the NUL-terminated path.
            if unsafe { libc::access(c"/dev/uinput".as_ptr(), libc::W_OK) } != 0 {
                return skip("uinput", "/dev/uinput isn't writable");
            }
            let manifest = std::env::var("CARGO_MANIFEST_DIR")
                .unwrap_or_else(|_| env!("CARGO_MANIFEST_DIR").to_owned());
            let runner = std::path::Path::new(&manifest).join("../tools/linux_console_run.py");
            let status = std::process::Command::new("python3")
                .arg(runner)
                .arg(std::env::current_exe().unwrap())
                .args(std::env::args_os().skip(1))
                .status()
                .expect("python3 runs tools/linux_console_run.py");
            return ExitCode::from(status.code().unwrap_or(1) as u8);
        }

        // The devices, before SDL starts
        let mut setup = Vec::new();
        for (name, kind) in [
            (KEYBOARD, "keyboard"),
            (MOUSE, "mouse"),
            (TABLET, "tablet"),
            (TOUCHSCREEN, "touchscreen"),
        ] {
            for reply in request(&format!("uinput add '{name}' {kind}")) {
                if reply.starts_with("uinput: unreadable ") {
                    return skip("uinput", &format!("{reply}: the reader can't open it"));
                }
                setup.push(reply);
            }
        }

        let mut t = Trace {
            lines: Vec::new(),
            keyboards: HashMap::new(),
            mice: HashMap::new(),
            window: 0,
        };
        for line in setup {
            t.line(line);
        }

        hints::set(hints::VIDEO_DRIVER, "evdev").unwrap();
        match init::init(InitFlags::VIDEO) {
            Ok(()) => {
                let driver = video::current_video_driver().unwrap_or("(none)");
                t.line(format!("init: ok ({driver})"));
            }
            Err(e) => {
                t.line(format!("init: {e}"));
                return finish(t);
            }
        }
        let window = Window::create("evdev_input", 1024, 768, WindowFlags::default()).unwrap();
        t.window = window.id();
        window.start_text_input().unwrap();

        // The devices found (sorted: udev lists them by their paths)
        let mut found = Vec::new();
        while let Some(event) = events::poll() {
            if let Some(line) = t.describe(&event) {
                found.push(line);
            }
        }
        found.sort();
        for line in found {
            t.line(line);
        }
        let mut touches: Vec<String> = touch::touch_devices()
            .into_iter()
            .map(|id| {
                format!(
                    "touch device: {} ({})",
                    touch::touch_device_name(id).unwrap_or_default(),
                    match touch::touch_device_type(id) {
                        touch::TouchDeviceType::Direct => "direct",
                        touch::TouchDeviceType::IndirectAbsolute => "indirect absolute",
                        touch::TouchDeviceType::IndirectRelative => "indirect relative",
                        touch::TouchDeviceType::Invalid => "invalid",
                    }
                )
            })
            .filter(|line| line.contains("SDL3 evdev"))
            .collect();
        touches.sort();
        for line in touches {
            t.line(line);
        }

        t.line("-- keyboard".into());
        t.events(KEYBOARD, &tap("KEY_A"));
        t.events(
            KEYBOARD,
            &format!(
                "EV_KEY KEY_LEFTSHIFT 1 EV_SYN SYN_REPORT 0 {} {} EV_KEY KEY_LEFTSHIFT 0 EV_SYN SYN_REPORT 0",
                tap("KEY_A"),
                tap("KEY_1")
            ),
        );
        t.events(
            KEYBOARD,
            "EV_KEY KEY_B 1 EV_SYN SYN_REPORT 0 EV_KEY KEY_B 2 EV_SYN SYN_REPORT 0 \
             EV_KEY KEY_B 2 EV_SYN SYN_REPORT 0 EV_KEY KEY_B 0 EV_SYN SYN_REPORT 0",
        );
        t.events(
            KEYBOARD,
            &format!("{} {}", tap("KEY_KP7"), tap("KEY_NUMLOCK")),
        );
        t.events(
            KEYBOARD,
            &format!(
                "{} {} {}",
                tap("KEY_KP7"),
                tap("KEY_KPENTER"),
                tap("KEY_NUMLOCK")
            ),
        );
        t.events(
            KEYBOARD,
            &format!(
                "{} {} {}",
                tap("KEY_CAPSLOCK"),
                tap("KEY_A"),
                tap("KEY_CAPSLOCK")
            ),
        );
        t.events(
            KEYBOARD,
            &format!("EV_KEY KEY_LEFTCTRL 1 EV_SYN SYN_REPORT 0 {} EV_KEY KEY_LEFTCTRL 0 EV_SYN SYN_REPORT 0", tap("KEY_C")),
        );
        t.events(KEYBOARD, &format!("{} {}", tap("KEY_SPACE"), tap("0x2fe")));

        t.line("-- mouse".into());
        t.events(MOUSE, "EV_REL REL_X 10 EV_REL REL_Y 5 EV_SYN SYN_REPORT 0");
        t.events(
            MOUSE,
            "EV_REL REL_X -4 EV_SYN SYN_REPORT 0 EV_REL REL_Y 2 EV_SYN SYN_REPORT 0",
        );
        for button in [
            "BTN_LEFT",
            "BTN_RIGHT",
            "BTN_MIDDLE",
            "BTN_SIDE",
            "BTN_EXTRA",
            "BTN_FORWARD",
            "BTN_BACK",
            "BTN_TASK",
        ] {
            t.events(MOUSE, &tap(button));
        }
        t.events(
            MOUSE,
            "EV_REL REL_WHEEL 1 EV_REL REL_WHEEL_HI_RES 120 EV_SYN SYN_REPORT 0 \
             EV_REL REL_HWHEEL_HI_RES -60 EV_SYN SYN_REPORT 0 EV_REL REL_WHEEL_HI_RES 30 EV_SYN SYN_REPORT 0",
        );
        let relative = window.set_relative_mouse_mode(true);
        t.line(format!(
            "relative mode: {}",
            if relative.is_ok() { "ok" } else { "failed" }
        ));
        t.events(MOUSE, "EV_REL REL_X 3 EV_REL REL_Y -2 EV_SYN SYN_REPORT 0");
        t.events(MOUSE, &tap("BTN_LEFT"));
        let relative = window.set_relative_mouse_mode(false);
        t.line(format!(
            "relative mode off: {}",
            if relative.is_ok() { "ok" } else { "failed" }
        ));
        t.pump();

        t.line("-- tablet".into());
        t.events(
            TABLET,
            "EV_ABS ABS_X 16384 EV_ABS ABS_Y 8192 EV_SYN SYN_REPORT 0",
        );
        t.events(TABLET, "EV_ABS ABS_X 32000 EV_SYN SYN_REPORT 0");
        t.events(TABLET, &tap("BTN_LEFT"));

        t.line("-- touchscreen".into());
        t.events(
            TOUCHSCREEN,
            "EV_ABS ABS_MT_SLOT 0 EV_ABS ABS_MT_TRACKING_ID 100 EV_ABS ABS_MT_POSITION_X 100 \
             EV_ABS ABS_MT_POSITION_Y 200 EV_ABS ABS_MT_PRESSURE 128 EV_KEY BTN_TOUCH 1 \
             EV_ABS ABS_X 100 EV_ABS ABS_Y 200 EV_SYN SYN_REPORT 0",
        );
        t.events(
            TOUCHSCREEN,
            "EV_ABS ABS_MT_SLOT 1 EV_ABS ABS_MT_TRACKING_ID 101 EV_ABS ABS_MT_POSITION_X 900 \
             EV_ABS ABS_MT_POSITION_Y 800 EV_ABS ABS_MT_PRESSURE 255 EV_SYN SYN_REPORT 0",
        );
        t.events(
            TOUCHSCREEN,
            "EV_ABS ABS_MT_SLOT 0 EV_ABS ABS_MT_POSITION_X 150 EV_ABS ABS_X 150 EV_SYN SYN_REPORT 0",
        );
        t.events(
            TOUCHSCREEN,
            "EV_ABS ABS_MT_TRACKING_ID -1 EV_ABS ABS_MT_SLOT 1 EV_ABS ABS_MT_TRACKING_ID -1 \
             EV_KEY BTN_TOUCH 0 EV_SYN SYN_REPORT 0",
        );
        // The reader's buffer overflowed: a finger down, moved 300 times,
        // without SDL reading (the reader's buffer of this device holds
        // fewer events); then SYN_DROPPED, and the slots read back
        let mut overflow = String::from(
            "EV_ABS ABS_MT_SLOT 0 EV_ABS ABS_MT_TRACKING_ID 200 EV_ABS ABS_MT_POSITION_X 500 \
             EV_ABS ABS_MT_POSITION_Y 500 EV_ABS ABS_MT_PRESSURE 200 EV_SYN SYN_REPORT 0",
        );
        for i in 1..=300 {
            overflow += &format!(
                " EV_ABS ABS_MT_POSITION_X {} EV_SYN SYN_REPORT 0",
                500 + i % 200
            );
        }
        overflow +=
            " EV_ABS ABS_MT_POSITION_X 700 EV_ABS ABS_MT_POSITION_Y 600 EV_SYN SYN_REPORT 0";
        t.events(TOUCHSCREEN, &overflow);
        t.events(
            TOUCHSCREEN,
            "EV_ABS ABS_MT_TRACKING_ID -1 EV_SYN SYN_REPORT 0",
        );

        t.line("-- hotplug".into());
        for reply in request(&format!("uinput add '{KEYBOARD2}' keyboard")) {
            t.line(reply);
        }
        t.wait_for(&format!("keyboard added: {KEYBOARD2}"));
        t.events(KEYBOARD2, &tap("KEY_Z"));
        for reply in request(&format!("uinput remove '{KEYBOARD2}'")) {
            t.line(reply);
        }
        t.wait_for(&format!("keyboard removed: {KEYBOARD2}"));
        for reply in request(&format!("uinput remove '{TABLET}'")) {
            t.line(reply);
        }
        t.wait_for(&format!("mouse removed: {TABLET}"));

        window.destroy();
        init::quit();
        t.line("quit".into());
        finish(t)
    }

    /// Compare the trace with upstream's.
    fn finish(t: Trace) -> ExitCode {
        let actual = t.lines.join("\n") + "\n";
        if actual == EXPECTED {
            println!(
                "evdev_input: the trace is upstream's ({} lines)",
                t.lines.len()
            );
            return ExitCode::SUCCESS;
        }
        let expected: Vec<&str> = EXPECTED.lines().collect();
        for (i, line) in t.lines.iter().enumerate() {
            if expected.get(i) != Some(&line.as_str()) {
                println!(
                    "evdev_input: line {}: {line:?}, upstream's {:?}",
                    i + 1,
                    expected.get(i)
                );
                break;
            }
        }
        if expected.len() != t.lines.len() {
            println!(
                "evdev_input: {} lines, upstream's {}",
                t.lines.len(),
                expected.len()
            );
        }
        ExitCode::FAILURE
    }
}
