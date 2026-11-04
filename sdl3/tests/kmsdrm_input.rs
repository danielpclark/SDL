// Tests of the KMS/DRM video driver's input (the console input reader)
// and its VT switches, on a vkms card.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! A program (`harness = false`), run as root on tty3 by
//! `tools/linux_console_run.py --vt 3` (see `kmsdrm_support`), on the
//! KMS/DRM driver and a vkms card, with a uinput keyboard and mouse,
//! tracing:
//!
//! * the devices added (by name: as root the reader also opens the
//!   machine's own), keys and their text on tty3's keymap, mouse motion
//!   and buttons, and the cursor plane moving with the mouse (the runner's
//!   `drm state`);
//! * a switch away from tty3 (`vt activate 1`, then pumps until it's
//!   done: the driver's release callback, from the reader's poll, destroys
//!   the window's surfaces and drops DRM master): the card's DRM master
//!   (free), what the card scans out then;
//! * the switch back (`vt switch 3`, then a pump: the acquire callback
//!   takes master again and makes the surfaces again, with a resize
//!   event), a frame swapped (the mode set again), and keys again;
//! * the devices removed, the window destroyed and SDL quit, tty3's modes
//!   then.
//!
//! The expected trace is upstream's C SDL's, from the same program in C
//! (upstream built with CMake at ea7a2da, `SDL_KMSDRM=ON`,
//! `SDL_KMSDRM_SHARED=ON`, run by the same runner on the same CI runner,
//! kernel and Mesa). Without the runner, it runs itself under it as root
//! when the capabilities are there (`kmsdrm`, `egl`, `vt`, `uinput`), and
//! skips otherwise, unless `SDL3_TEST_REQUIRE` asks for them. Elsewhere
//! than Linux it does nothing.

#[cfg(not(target_os = "linux"))]
fn main() {}

#[cfg(target_os = "linux")]
#[path = "kmsdrm_support/mod.rs"]
mod kmsdrm_support;

#[cfg(target_os = "linux")]
fn main() -> std::process::ExitCode {
    app::main()
}

#[cfg(target_os = "linux")]
mod app {
    use std::collections::HashMap;
    use std::process::ExitCode;

    use super::kmsdrm_support::{
        gl_clear, pump_until, request, result, run_on_tty3, skip, Trace, OTHER_VT, VT,
    };
    use sdl3::events::window::WindowFlags;
    use sdl3::events::{self, keyboard, mouse, Event};
    use sdl3::init::{self, InitFlags};
    use sdl3::video::gl::{self, GlContext};
    use sdl3::video::{self, Window};

    /// Upstream's trace (the C program, by the runner, on the CI runner).
    const EXPECTED: &str = "\
versions: kernel 7.0.0-1017-azure, Mesa 25.2.8-0ubuntu0.24.04.4, libdrm 2.4.125-1ubuntu0.1~24.04.2
vt: active 3
vt: keyboard K_UNICODE mode VT_AUTO
-- init
init: ok (kmsdrm)
window: ok
window event 0x207 1024 768 window
window event 0x20c 0 0 window
window event 0x20e 0 0 window
window event 0x202 0 0 window
window event 0x215 0 0 window
context: ok
swap: ok
vt: keyboard K_OFF mode VT_PROCESS
drm: master taken
-- devices
uinput: added SDL3 kmsdrm keyboard
uinput: added SDL3 kmsdrm mouse
keyboard added: SDL3 kmsdrm keyboard
mouse added: SDL3 kmsdrm mouse
-- keys and mouse
key down A key A mod 0x0000 window
text \"a\" window
key up A key A mod 0x0000 window
key down 1 key 1 mod 0x0000 window
text \"1\" window
key up 1 key 1 mod 0x0000 window
key down Left Shift key Left Shift mod 0x0001 window
key down B key B mod 0x0001 window
text \"B\" window
key up B key B mod 0x0001 window
key up Left Shift key Left Shift mod 0x0000 window
mouse: at (40.0, 30.0) buttons 0x0
drm: crtc 1024x768 (1024x768) on
drm: plane primary fb 1024x768 XR24 src 0,0 1024x768 crtc 0,0 1024x768
drm: plane cursor fb 512x512 AR24 src 0,0 512x512 crtc 40,30 512x512
mouse: at (40.0, 30.0) buttons 0x0
-- vt switch away
vt: activating 1
vt: active 1
vt: keyboard K_OFF mode VT_PROCESS
drm: master free
drm: crtc 1024x768 (1024x768) on
drm: plane primary fb 1024x768 XR24 src 0,0 1024x768 crtc 0,0 1024x768
drm: plane cursor fb 512x512 AR24 src 0,0 512x512 crtc 40,30 512x512
-- vt switch back
vt: active 3
vt: keyboard K_OFF mode VT_PROCESS
drm: master taken
swap: ok
drm: crtc 1024x768 (1024x768) on
drm: plane primary fb 1024x768 XR24 src 0,0 1024x768 crtc 0,0 1024x768
drm: plane cursor fb 512x512 AR24 src 0,0 512x512 crtc 40,30 512x512
drm: crc 0x0b41bf6d
key down Z key Z mod 0x0000 window
text \"z\" window
key up Z key Z mod 0x0000 window
mouse: at (20.0, 30.0) buttons 0x0
drm: crtc 1024x768 (1024x768) on
drm: plane primary fb 1024x768 XR24 src 0,0 1024x768 crtc 0,0 1024x768
drm: plane cursor fb 512x512 AR24 src 0,0 512x512 crtc 20,30 512x512
drm: crtc 1024x768 (1024x768) on
drm: plane primary fb 1024x768 XR24 src 0,0 1024x768 crtc 0,0 1024x768
drm: plane cursor fb 512x512 AR24 src 0,0 512x512 crtc 0,0 512x512
-- removed
uinput: removed SDL3 kmsdrm keyboard
uinput: removed SDL3 kmsdrm mouse
keyboard removed: SDL3 kmsdrm keyboard
mouse removed: SDL3 kmsdrm mouse
window destroyed
quit
vt: keyboard K_UNICODE mode VT_AUTO
drm: crtc 1024x768 (1024x768) on
drm: plane primary fb 1024x768 XR24 src 0,0 1024x768 crtc 0,0 1024x768
";

    const PROGRAM: &str = "kmsdrm_input";
    const KEYBOARD: &str = "SDL3 kmsdrm keyboard";
    const MOUSE: &str = "SDL3 kmsdrm mouse";

    struct Input {
        t: Trace,
        window: events::WindowID,
        /// The names of the devices by ID (kept after they're removed)
        keyboards: HashMap<u32, String>,
        mice: HashMap<u32, String>,
    }

    fn ours(name: &str) -> bool {
        name.starts_with("SDL3 kmsdrm")
    }

    impl Input {
        fn window(&self, id: events::WindowID) -> &'static str {
            if id == self.window {
                "window"
            } else if id == 0 {
                "none"
            } else {
                "another"
            }
        }

        /// Who a mouse event is from: "global" (the reader's mice, and
        /// warps), or one of these devices (`None`: another).
        fn mouse(&self, which: u32) -> Option<String> {
            if which == 0 {
                return Some("global".to_owned());
            }
            self.mice.get(&which).filter(|n| ours(n)).cloned()
        }

        /// The event's line, if it's one of the traced ones (of these
        /// devices, or of the window).
        fn describe(&mut self, event: &Event) -> Option<String> {
            match event {
                Event::KeyboardDevice(d) => {
                    if d.event_type == events::EventType::KEYBOARD_ADDED {
                        let name = keyboard::keyboard_name(d.which).unwrap_or_default();
                        self.keyboards.insert(d.which, name);
                    }
                    let name = self.keyboards.get(&d.which).cloned().unwrap_or_default();
                    let added = d.event_type == events::EventType::KEYBOARD_ADDED;
                    ours(&name).then(|| {
                        format!(
                            "keyboard {}: {name}",
                            if added { "added" } else { "removed" }
                        )
                    })
                }
                Event::MouseDevice(d) => {
                    if d.event_type == events::EventType::MOUSE_ADDED {
                        let name = mouse::mouse_name(d.which).unwrap_or_default();
                        self.mice.insert(d.which, name);
                    }
                    let name = self.mice.get(&d.which).cloned().unwrap_or_default();
                    let added = d.event_type == events::EventType::MOUSE_ADDED;
                    ours(&name).then(|| {
                        format!("mouse {}: {name}", if added { "added" } else { "removed" })
                    })
                }
                Event::Key(k) => {
                    let name = self.keyboards.get(&k.which)?;
                    ours(name).then(|| {
                        format!(
                            "key {} {} key {} mod 0x{:04x} {}",
                            if k.down { "down" } else { "up" },
                            k.scancode.name(),
                            k.key.name(),
                            k.modifiers.0,
                            self.window(k.window_id),
                        )
                    })
                }
                Event::TextInput(t) => {
                    Some(format!("text \"{}\" {}", t.text, self.window(t.window_id)))
                }
                Event::MouseMotion(m) => {
                    let who = self.mouse(m.which)?;
                    Some(format!(
                        "motion {who} at ({:.1}, {:.1}) rel ({:.1}, {:.1}) {}",
                        m.x,
                        m.y,
                        m.xrel,
                        m.yrel,
                        self.window(m.window_id)
                    ))
                }
                Event::MouseButton(b) => {
                    let who = self.mouse(b.which)?;
                    Some(format!(
                        "button {} {} {who} at ({:.1}, {:.1}) {}",
                        if b.down { "down" } else { "up" },
                        b.button,
                        b.x,
                        b.y,
                        self.window(b.window_id)
                    ))
                }
                Event::Window(w) => Some(format!(
                    "window event 0x{:x} {} {} {}",
                    w.event_type.0,
                    w.data1,
                    w.data2,
                    self.window(w.window_id)
                )),
                _ => None,
            }
        }

        /// Pump once, and trace the events.
        fn pump(&mut self) {
            events::pump();
            self.drain();
        }

        fn drain(&mut self) {
            while let Some(event) = events::poll() {
                if let Some(line) = self.describe(&event) {
                    self.t.line(line);
                }
            }
        }

        /// Pump until the lines have come, tracing the events (within 10
        /// s).
        fn wait_for(&mut self, lines: &[String]) {
            let mut missing: Vec<String> = lines.to_vec();
            let found = pump_until(|| {
                while let Some(event) = events::poll() {
                    if let Some(l) = self.describe(&event) {
                        missing.retain(|m| *m != l);
                        self.t.line(l);
                    }
                }
                missing.is_empty()
            });
            if !found {
                self.t.line(format!("timeout: no {missing:?} in 10 s"));
            }
        }

        /// A request of a device's events (shell words: TYPE CODE VALUE
        /// ...), then the events they make.
        fn send(&mut self, device: &str, events: &str) {
            for reply in request(&format!("uinput events '{device}' {events}")) {
                if !reply.starts_with("uinput: sent ") {
                    self.t.line(reply);
                }
            }
            self.pump();
        }

        fn mouse_state(&mut self) {
            let (x, y, buttons) = mouse::mouse_state();
            self.t.line(format!(
                "mouse: at ({x:.1}, {y:.1}) buttons 0x{:x}",
                buttons.0
            ));
        }
    }

    /// A key's press and release (and SYN_REPORT after each).
    fn tap(key: &str) -> String {
        format!("EV_KEY {key} 1 EV_SYN SYN_REPORT 0 EV_KEY {key} 0 EV_SYN SYN_REPORT 0")
    }

    pub(super) fn main() -> ExitCode {
        if let Err(code) = run_on_tty3(PROGRAM, true) {
            return code;
        }
        let mut i = Input {
            t: Trace::new(),
            window: 0,
            keyboards: HashMap::new(),
            mice: HashMap::new(),
        };
        i.t.ask("vt active");
        i.t.ask("vt modes");

        i.t.line("-- init");
        if let Err(e) = init::init(InitFlags::VIDEO) {
            i.t.line(format!("init: {e}"));
            return i.t.finish(PROGRAM, EXPECTED);
        }
        i.t.line(format!(
            "init: ok ({})",
            video::current_video_driver().unwrap_or("(none)")
        ));
        let window = Window::create("kmsdrm_input", 1024, 768, WindowFlags::default());
        i.t.line(format!("window: {}", result(&window)));
        let Ok(window) = window else {
            init::quit();
            return i.t.finish(PROGRAM, EXPECTED);
        };
        i.window = window.id();
        let _ = window.start_text_input();
        // (the machine's own devices: their events aren't traced)
        i.drain();
        let context = GlContext::new(&window);
        i.t.line(format!("context: {}", result(&context)));
        gl_clear(0.0, 0.0, 1.0);
        i.t.line(format!("swap: {}", result(&gl::gl_swap_window(&window))));
        i.pump();
        i.t.ask("vt modes");
        i.t.ask("drm master");

        i.t.line("-- devices");
        for (name, kind) in [(KEYBOARD, "keyboard"), (MOUSE, "mouse")] {
            for reply in request(&format!("uinput add '{name}' {kind}")) {
                if reply.starts_with("uinput: unreadable ") {
                    return skip(
                        PROGRAM,
                        "uinput",
                        &format!("{reply}: the reader can't open it"),
                    );
                }
                i.t.line(reply);
            }
        }
        i.wait_for(&[
            format!("keyboard added: {KEYBOARD}"),
            format!("mouse added: {MOUSE}"),
        ]);

        i.t.line("-- keys and mouse");
        i.send(KEYBOARD, &format!("{} {}", tap("KEY_A"), tap("KEY_1")));
        i.send(
            KEYBOARD,
            &format!(
                "EV_KEY KEY_LEFTSHIFT 1 EV_SYN SYN_REPORT 0 {} EV_KEY KEY_LEFTSHIFT 0 EV_SYN SYN_REPORT 0",
                tap("KEY_B")
            ),
        );
        i.send(MOUSE, "EV_REL REL_X 40 EV_REL REL_Y 30 EV_SYN SYN_REPORT 0");
        i.mouse_state();
        i.t.ask("drm state");
        i.send(
            MOUSE,
            "EV_KEY BTN_LEFT 1 EV_SYN SYN_REPORT 0 EV_KEY BTN_LEFT 0 EV_SYN SYN_REPORT 0",
        );
        i.mouse_state();

        i.t.line("-- vt switch away");
        // Away: the kernel waits for SDL's VT_RELDISP, which the event pump
        // makes (on the release signal), after the driver's release
        i.t.ask(&format!("vt activate {OTHER_VT}"));
        let away = format!("vt: active {OTHER_VT}");
        let mut last = Vec::new();
        pump_until(|| {
            last = request("vt active");
            last.first() == Some(&away)
        });
        for line in last {
            i.t.line(line);
        }
        i.drain();
        i.t.ask("vt modes");
        i.t.ask("drm master");
        i.t.ask("drm state");

        i.t.line("-- vt switch back");
        // Back: done at once, the acquire signal acknowledged by the next
        // pump (the driver's acquire)
        i.t.ask(&format!("vt switch {VT}"));
        i.pump();
        i.t.ask("vt modes");
        i.t.ask("drm master");
        gl_clear(0.0, 1.0, 0.0);
        i.t.line(format!("swap: {}", result(&gl::gl_swap_window(&window))));
        i.pump();
        i.t.ask("drm state");
        i.t.ask("drm crc");
        i.send(KEYBOARD, &tap("KEY_Z"));
        i.send(MOUSE, "EV_REL REL_X -20 EV_SYN SYN_REPORT 0");
        i.mouse_state();
        i.t.ask("drm state");

        // (the cursor's position is the CRTC's, kept after the program: the
        // runs start with it at the origin)
        mouse::warp_mouse_in_window(Some(window.id()), 0.0, 0.0);
        i.pump();
        i.t.ask("drm state");

        i.t.line("-- removed");
        for name in [KEYBOARD, MOUSE] {
            i.t.ask(&format!("uinput remove '{name}'"));
        }
        i.wait_for(&[
            format!("keyboard removed: {KEYBOARD}"),
            format!("mouse removed: {MOUSE}"),
        ]);
        drop(context);
        window.destroy();
        i.t.line("window destroyed");
        init::quit();
        i.t.line("quit");
        i.t.ask("vt modes");
        i.t.ask("drm state");

        i.t.finish(PROGRAM, EXPECTED)
    }
}
