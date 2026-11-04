// Tests of the KMS/DRM video driver with several displays: a vkms device
// of two pipelines, made through configfs.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! A program (`harness = false`), run as root on tty3 by
//! `tools/linux_console_run.py --vt 3` (see `kmsdrm_support`), on the
//! KMS/DRM driver and a vkms device the runner makes through vkms's
//! configfs (`vkms add`: two pipelines, each a primary and a cursor plane,
//! a CRTC, an encoder and a connected connector), tracing:
//!
//! * the device's connectors and CRTCs (as the kernel's fbdev emulation
//!   sets them);
//! * its displays (a display per connected connector): their names,
//!   bounds, desktop modes, which is primary; then sorted by
//!   `SDL_VIDEO_DISPLAY_PRIORITY` (the second connector's name first;
//!   names in another order, one of them no display's);
//! * with the legacy page flips (`SDL_KMSDRM_ATOMIC=0`), a window on each
//!   display (`SDL_WINDOWPOS_UNDEFINED_DISPLAY()`): its display, flags,
//!   size and events, a context, frames of one color swapped on each, and
//!   each CRTC's scanout then (the runner's `drm state`, `drm crc` and
//!   `drm pixels` of that CRTC);
//! * with the atomic commits (the default), a window on the first display,
//!   then one on the second display alone, their frames, and both CRTCs
//!   then. Windows on both displays aren't made: upstream sets up the
//!   display plane of the first window's display only (`KMSDRM_GBMInit()`),
//!   so an atomic commit for another display's window dereferences its
//!   NULL plane (`drm_atomic_set_plane_props()`: the C program crashes
//!   there; here, `Note (upstream)`, nothing is added for it);
//! * the device removed.
//!
//! The device's card index and its connectors' numbers depend on the
//! machine's other cards: the trace names neither (`drm card` answers as
//! "the device's card", connectors as `kmsdrm_support::connector_name`
//! masks them). Every hint is set again before each init: `SDL_Quit()`
//! resets them all.
//!
//! The expected trace is upstream's C SDL's, from the same program in C
//! (upstream built with CMake at ea7a2da, run by the same runner on the
//! same CI runner, kernel and Mesa). It runs itself under the runner as
//! root when the capabilities are there (`kmsdrm`, `egl`, `vt`; vkms's
//! configfs, `kmsdrm` too), and skips otherwise, unless
//! `SDL3_TEST_REQUIRE` asks for them. Elsewhere than Linux it does
//! nothing.

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
    use std::process::ExitCode;

    use super::kmsdrm_support::{
        connector_name, gl_clear, mode_line, request, result, run_on_tty3, skip, Trace,
    };
    use sdl3::events::{self, Event};
    use sdl3::hints;
    use sdl3::init::{self, InitFlags};
    use sdl3::video::gl::{self, GlContext};
    use sdl3::video::{self, windowpos_undefined_display, Window};

    /// Upstream's trace (the C program, by the runner, on the CI runner).
    const EXPECTED: &str = "\
versions: (to be made)
";

    const PROGRAM: &str = "kmsdrm_displays";

    /// The points read back of each frame.
    const PIXELS: &str = "drm pixels 0 0 320 240 639 479 1023 767";

    fn window_events(t: &mut Trace, windows: &[&Window]) {
        events::pump();
        while let Some(event) = events::poll() {
            match event {
                Event::Window(w) => t.line(format!(
                    "window {} event 0x{:x} {} {}",
                    windows
                        .iter()
                        .position(|x| x.id() == w.window_id)
                        .map_or("?".to_owned(), |i| i.to_string()),
                    w.event_type.0,
                    w.data1,
                    w.data2
                )),
                Event::Display(d) => t.line(format!(
                    "display event 0x{:x} {} {}",
                    d.event_type.0, d.data1, d.data2
                )),
                _ => {}
            }
        }
    }

    fn displays(t: &mut Trace, card: u32) {
        let ids = video::displays().unwrap_or_default();
        t.line(format!("displays: {}", ids.len()));
        for (i, &id) in ids.iter().enumerate() {
            let bounds = video::display_bounds(id).unwrap_or_default();
            t.line(format!(
                "display {i}: {:?} bounds {},{} {}x{}{}, {} fullscreen modes",
                connector_name(card, &video::display_name(id).unwrap_or_default()),
                bounds.x,
                bounds.y,
                bounds.w,
                bounds.h,
                if Some(id) == video::primary_display().ok() {
                    " (primary)"
                } else {
                    ""
                },
                video::fullscreen_display_modes(id)
                    .unwrap_or_default()
                    .len()
            ));
            if let Ok(m) = video::desktop_display_mode(id) {
                t.line(format!("display {i}: desktop mode {}", mode_line(&m)));
            }
        }
    }

    fn init(t: &mut Trace) -> bool {
        match init::init(InitFlags::VIDEO) {
            Ok(()) => {
                t.line(format!(
                    "init: ok ({})",
                    video::current_video_driver().unwrap_or("(none)")
                ));
                true
            }
            Err(e) => {
                t.line(format!("init: {e}"));
                false
            }
        }
    }

    fn quit(t: &mut Trace) {
        init::quit();
        t.line("quit");
    }

    /// The runner's `drm card CARD CRTC`, traced without the card's index.
    fn drm_card(t: &mut Trace, card: u32, crtc: usize) {
        for line in request(&format!("drm card {card} {crtc}")) {
            if line == format!("drm: card {card} crtc {crtc}") {
                t.line(format!("drm: the device's card, crtc {crtc}"));
            } else {
                t.line(line);
            }
        }
    }

    /// Each CRTC's scanout.
    fn scanouts(t: &mut Trace, card: u32, crc: bool) {
        for crtc in 0..2 {
            drm_card(t, card, crtc);
            t.ask("drm state");
            if crc {
                t.ask("drm crc");
                t.ask(PIXELS);
            }
        }
    }

    /// Windows on the displays (by their indices), a frame of one color
    /// each, swapped.
    fn windows(t: &mut Trace, card: u32, on: &[usize]) {
        let ids = video::displays().unwrap_or_default();
        let mut windows = Vec::new();
        for (i, &id) in on.iter().filter_map(|&d| ids.get(d)).enumerate() {
            let window = Window::builder()
                .title(format!("kmsdrm_displays {i}"))
                .position(
                    windowpos_undefined_display(id),
                    windowpos_undefined_display(id),
                )
                .size(640, 480)
                .build();
            t.line(format!("window {i}: {}", result(&window)));
            let Ok(window) = window else {
                continue;
            };
            let flags = window.flags().unwrap_or_default();
            let (w, h) = window.size().unwrap_or_default();
            t.line(format!(
                "window {i}: display {:?}, flags 0x{:x} size {w}x{h}",
                window
                    .display()
                    .ok()
                    .and_then(|d| ids.iter().position(|&x| x == d)),
                flags.0
            ));
            windows.push(window);
        }
        let refs: Vec<&Window> = windows.iter().collect();
        window_events(t, &refs);
        let mut contexts = Vec::new();
        for (i, window) in windows.iter().enumerate() {
            let context = GlContext::new(window);
            t.line(format!("context {i}: {}", result(&context)));
            contexts.push(context.ok());
        }
        let colors = [(1.0, 0.0, 0.0), (0.0, 0.0, 1.0)];
        for frame in 0..2 {
            for (i, window) in windows.iter().enumerate() {
                let Some(context) = &contexts[i] else {
                    continue;
                };
                t.line(format!(
                    "window {i}: make current: {}",
                    result(&gl::gl_make_current(Some(window), Some(context)))
                ));
                let (r, g, b) = colors[(i + frame) % 2];
                gl_clear(r, g, b);
                t.line(format!(
                    "window {i}: swap {frame}: {}",
                    result(&gl::gl_swap_window(window))
                ));
            }
            window_events(t, &refs);
            scanouts(t, card, true);
        }
        drop(contexts);
        for window in windows {
            window.destroy();
        }
        t.line("windows destroyed");
        window_events(t, &[]);
    }

    pub(super) fn main() -> ExitCode {
        if let Err(code) = run_on_tty3(PROGRAM, false) {
            return code;
        }
        // (configfs mounted, as the runner's `vkms add` would)
        let vkms = std::path::Path::new("/sys/kernel/config/vkms");
        if !vkms.is_dir() {
            let _ = std::process::Command::new("mount")
                .args(["-t", "configfs", "none", "/sys/kernel/config"])
                .status();
        }
        if !vkms.is_dir() {
            return skip(
                PROGRAM,
                "kmsdrm",
                "no /sys/kernel/config/vkms (a vkms without configfs devices)",
            );
        }
        let mut t = Trace::new();

        t.line("-- a vkms device of two pipelines");
        let added = request("vkms add multi 2");
        // (`vkms: added NAME cardK CONNECTOR...`)
        let words: Vec<&str> = added.first().map_or(Vec::new(), |l| l.split(' ').collect());
        let card: Option<u32> = words
            .get(3)
            .and_then(|c| c.strip_prefix("card"))
            .and_then(|c| c.parse().ok());
        let (Some(card), [_, _, _, _, connectors @ ..]) = (card, words.as_slice()) else {
            for line in &added {
                t.line(line.clone());
            }
            return t.finish(PROGRAM, EXPECTED);
        };
        let connectors: Vec<String> = connectors.iter().map(|&c| c.to_owned()).collect();
        let masked = |names: &str| {
            names
                .split(',')
                .map(|n| {
                    if connectors.iter().any(|c| c == n) {
                        connector_name(card, n)
                    } else {
                        n.to_owned()
                    }
                })
                .collect::<Vec<_>>()
                .join(",")
        };
        t.line(format!(
            "vkms: added {}, connectors {}",
            words[2],
            masked(&connectors.join(","))
        ));
        scanouts(&mut t, card, false);
        // The device's card, over the variable the runner's environment
        // sets (again before each init: SDL_Quit() resets every hint)
        let card_hint = || {
            hints::set_with_priority(
                hints::KMSDRM_DEVICE_INDEX,
                Some(&card.to_string()),
                hints::Priority::Override,
            )
            .unwrap();
        };

        t.line("-- displays");
        card_hint();
        if init(&mut t) {
            displays(&mut t, card);
            quit(&mut t);
        }
        if let [first, second] = connectors.as_slice() {
            for priority in [
                second.clone(),
                format!("nosuch,{second},{first}"),
                format!("{first},{second}"),
            ] {
                t.line(format!(
                    "-- SDL_VIDEO_DISPLAY_PRIORITY={}",
                    masked(&priority)
                ));
                card_hint();
                hints::set(hints::VIDEO_DISPLAY_PRIORITY, &priority).unwrap();
                if init(&mut t) {
                    displays(&mut t, card);
                    quit(&mut t);
                }
                hints::reset(hints::VIDEO_DISPLAY_PRIORITY);
            }
        }

        t.line("-- a window on each display (legacy, SDL_KMSDRM_ATOMIC=0)");
        card_hint();
        hints::set(hints::KMSDRM_ATOMIC, "0").unwrap();
        if init(&mut t) {
            windows(&mut t, card, &[0, 1]);
            quit(&mut t);
        }
        hints::reset(hints::KMSDRM_ATOMIC);
        scanouts(&mut t, card, false);

        t.line("-- a window on the first display (atomic)");
        card_hint();
        if init(&mut t) {
            windows(&mut t, card, &[0]);
            quit(&mut t);
        }
        scanouts(&mut t, card, false);

        t.line("-- a window on the second display alone (atomic)");
        card_hint();
        if init(&mut t) {
            windows(&mut t, card, &[1]);
            quit(&mut t);
        }
        scanouts(&mut t, card, false);

        hints::reset(hints::KMSDRM_DEVICE_INDEX);
        t.ask("drm card");
        t.ask("vkms remove multi");
        t.finish(PROGRAM, EXPECTED)
    }
}
