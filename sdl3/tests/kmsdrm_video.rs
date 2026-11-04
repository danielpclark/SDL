// Tests of the KMS/DRM video driver's displays, modes and windows on a
// vkms card.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! A program (`harness = false`), run as root on tty3 by
//! `tools/linux_console_run.py --vt 3` (see `kmsdrm_support`), on the
//! KMS/DRM driver (`SDL_VIDEO_DRIVER=kmsdrm`) and a vkms card, tracing:
//!
//! * the display of the connected connector: its name (the connector's
//!   type and number), bounds, desktop and current modes (their refresh
//!   rates from the mode's clock and totals), content scale,
//!   orientations and panel orientation (vkms has none: 0), and its
//!   fullscreen modes (the connector's), with the closest ones to a few
//!   sizes;
//! * a window: its flags (OpenGL, which the driver adds), size, the
//!   driver's properties, its events (the resize to the mode chosen, the
//!   move to the display's origin, the focus), then frames swapped
//!   (`glClear()`), each traced as what the card scans out (the runner's
//!   `drm state`, `drm crc` and `drm pixels`): the mode set on the first
//!   swap, the closest to the window's size;
//! * fullscreen with a mode of 800x600 (the mode set on the next swap),
//!   fullscreen leaving it, a new window size: their events and the
//!   modes then;
//! * the window destroyed and SDL quit, and the card's state then (what
//!   the CRTC was restored to);
//! * without DRM master (the runner holds every card's): without
//!   `SDL_KMSDRM_DEVICE_INDEX` no card is available (DRM master is
//!   required: `SDL_KMSDRM_REQUIRE_DRM_MASTER`); with the index (which
//!   upstream takes as it is) and `SDL_KMSDRM_REQUIRE_DRM_MASTER=0`, the
//!   driver starts, and the first swap fails to set the mode.
//!
//! The expected trace is upstream's C SDL's, from the same program in C
//! (upstream built with CMake at ea7a2da, `SDL_KMSDRM=ON`,
//! `SDL_KMSDRM_SHARED=ON`, run by the same runner on the same CI runner,
//! kernel and Mesa). Without the runner, it runs itself under it as root
//! when the capabilities are there (`kmsdrm`, `egl`, `vt`), and skips
//! otherwise, unless `SDL3_TEST_REQUIRE` asks for them. Elsewhere than
//! Linux it does nothing.

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

    use super::kmsdrm_support::{gl_clear, mode_line, result, run_on_tty3, Trace};
    use sdl3::events::window::WindowFlags;
    use sdl3::events::{self, Event};
    use sdl3::hints;
    use sdl3::init::{self, InitFlags};
    use sdl3::video::gl::{self, GlContext};
    use sdl3::video::{self, Window};

    /// Upstream's trace (the C program, by the runner, on the CI runner).
    const EXPECTED: &str = "\
versions: (the reference's)
";

    const PROGRAM: &str = "kmsdrm_video";

    /// The window events since the last call (of the window).
    fn window_events(t: &mut Trace, window: &Window) {
        events::pump();
        while let Some(event) = events::poll() {
            match event {
                Event::Window(w) => t.line(format!(
                    "window event 0x{:x} {} {}{}",
                    w.event_type.0,
                    w.data1,
                    w.data2,
                    if w.window_id == window.id() {
                        ""
                    } else {
                        " (another window)"
                    }
                )),
                Event::Display(d) => t.line(format!(
                    "display event 0x{:x} {} {}",
                    d.event_type.0, d.data1, d.data2
                )),
                _ => {}
            }
        }
    }

    /// What the card scans out.
    fn scanout(t: &mut Trace) {
        t.ask("drm state");
        t.ask("drm crc");
        t.ask("drm pixels 0 0 320 240 799 599 1023 767");
    }

    /// A frame of one color, swapped, and what the card scans out then.
    fn frame(t: &mut Trace, window: &Window, rgb: (f32, f32, f32)) {
        gl_clear(rgb.0, rgb.1, rgb.2);
        t.line(format!(
            "swap ({:.1}, {:.1}, {:.1}): {}",
            rgb.0,
            rgb.1,
            rgb.2,
            result(&gl::gl_swap_window(window))
        ));
        window_events(t, window);
        scanout(t);
    }

    fn window_state(t: &mut Trace, window: &Window) {
        let flags = window.flags().unwrap_or_default();
        let (w, h) = window.size().unwrap_or_default();
        let (pw, ph) = window.size_in_pixels().unwrap_or_default();
        t.line(format!(
            "window: flags 0x{:x} size {w}x{h} pixels {pw}x{ph} fullscreen mode {}",
            flags.0,
            match window.fullscreen_mode() {
                Ok(Some(m)) => mode_line(&m),
                Ok(None) => "none".to_owned(),
                Err(e) => format!("error: {e}"),
            }
        ));
    }

    fn displays(t: &mut Trace) {
        let ids = video::displays().unwrap_or_default();
        t.line(format!("displays: {}", ids.len()));
        for (i, &id) in ids.iter().enumerate() {
            let bounds = video::display_bounds(id).unwrap_or_default();
            let usable = video::display_usable_bounds(id).unwrap_or_default();
            t.line(format!(
                "display {i}: {:?} bounds {},{} {}x{} usable {},{} {}x{}{}",
                video::display_name(id).unwrap_or_default(),
                bounds.x,
                bounds.y,
                bounds.w,
                bounds.h,
                usable.x,
                usable.y,
                usable.w,
                usable.h,
                if Some(id) == video::primary_display().ok() {
                    " (primary)"
                } else {
                    ""
                }
            ));
            if let Ok(m) = video::desktop_display_mode(id) {
                t.line(format!("desktop mode: {}", mode_line(&m)));
            }
            if let Ok(m) = video::current_display_mode(id) {
                t.line(format!("current mode: {}", mode_line(&m)));
            }
            let props = video::display_properties(id).ok();
            t.line(format!(
                "content scale {:.2}, orientation natural {} current {}, panel orientation {:?}",
                video::display_content_scale(id).unwrap_or(0.0),
                video::natural_display_orientation(id) as i32,
                video::current_display_orientation(id) as i32,
                props.and_then(|p| p.get_number("SDL.display.KMSDRM.panel_orientation"))
            ));
            let modes = video::fullscreen_display_modes(id).unwrap_or_default();
            t.line(format!("fullscreen modes: {}", modes.len()));
            for m in &modes {
                t.line(format!("mode: {}", mode_line(m)));
            }
            for (w, h) in [
                (640, 480),
                (800, 600),
                (1000, 700),
                (1024, 768),
                (5000, 5000),
            ] {
                t.line(format!(
                    "closest to {w}x{h}: {}",
                    match video::closest_fullscreen_display_mode(id, w, h, 0.0, false) {
                        Ok(m) => mode_line(&m),
                        Err(e) => format!("error: {e}"),
                    }
                ));
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

    pub(super) fn main() -> ExitCode {
        if let Err(code) = run_on_tty3(PROGRAM, false) {
            return code;
        }
        let mut t = Trace::new();
        t.ask("drm state");

        t.line("-- init");
        if !init(&mut t) {
            return t.finish(PROGRAM, EXPECTED);
        }
        displays(&mut t);

        t.line("-- window");
        let window = Window::create("kmsdrm_video", 640, 480, WindowFlags::default());
        t.line(format!("window: {}", result(&window)));
        let Ok(window) = window else {
            quit(&mut t);
            return t.finish(PROGRAM, EXPECTED);
        };
        window_state(&mut t, &window);
        let props = window.properties().unwrap();
        t.line(format!(
            "properties: device index {:?}, drm fd set {}, gbm device set {}",
            props.get_number("SDL.window.kmsdrm.dev_index"),
            props
                .get_number("SDL.window.kmsdrm.drm_fd")
                .is_some_and(|fd| fd >= 0),
            props
                .get_number("SDL.window.kmsdrm.gbm_dev")
                .is_some_and(|p| p != 0)
        ));
        window_events(&mut t, &window);
        t.ask("drm state");

        let context = GlContext::new(&window);
        t.line(format!("context: {}", result(&context)));
        let Ok(context) = context else {
            window.destroy();
            quit(&mut t);
            return t.finish(PROGRAM, EXPECTED);
        };
        t.line(format!(
            "swap interval: {}",
            match gl::gl_get_swap_interval() {
                Ok(i) => i.to_string(),
                Err(e) => format!("error: {e}"),
            }
        ));
        frame(&mut t, &window, (1.0, 0.0, 0.0));
        frame(&mut t, &window, (0.0, 1.0, 0.0));
        window_state(&mut t, &window);

        t.line("-- fullscreen 800x600");
        let display = window.display().unwrap_or(0);
        let mode = video::closest_fullscreen_display_mode(display, 800, 600, 0.0, false);
        t.line(format!(
            "set fullscreen mode: {}",
            match &mode {
                Ok(m) => result(&window.set_fullscreen_mode(Some(m))),
                Err(e) => format!("error: {e}"),
            }
        ));
        t.line(format!(
            "set fullscreen: {}",
            result(&window.set_fullscreen(true))
        ));
        window_events(&mut t, &window);
        window_state(&mut t, &window);
        if let Ok(m) = video::current_display_mode(display) {
            t.line(format!("current mode: {}", mode_line(&m)));
        }
        frame(&mut t, &window, (0.0, 0.0, 1.0));

        t.line("-- leave fullscreen");
        t.line(format!(
            "set fullscreen false: {}",
            result(&window.set_fullscreen(false))
        ));
        window_events(&mut t, &window);
        window_state(&mut t, &window);
        if let Ok(m) = video::current_display_mode(display) {
            t.line(format!("current mode: {}", mode_line(&m)));
        }
        frame(&mut t, &window, (1.0, 1.0, 0.0));

        t.line("-- size 1024x768");
        t.line(format!("set size: {}", result(&window.set_size(1024, 768))));
        window_events(&mut t, &window);
        window_state(&mut t, &window);
        frame(&mut t, &window, (0.0, 1.0, 1.0));

        t.line("-- destroyed");
        drop(context);
        window.destroy();
        t.line("window destroyed");
        t.ask("drm state");
        quit(&mut t);
        t.ask("drm state");
        t.ask("drm master");

        t.line("-- without DRM master");
        t.ask("drm master hold");
        // (the index set empty, over the variable: the cards are looked
        // through)
        hints::set_with_priority(
            hints::KMSDRM_DEVICE_INDEX,
            Some(""),
            hints::Priority::Override,
        )
        .unwrap();
        if init(&mut t) {
            quit(&mut t);
        }
        hints::reset(hints::KMSDRM_DEVICE_INDEX);
        hints::set(hints::KMSDRM_REQUIRE_DRM_MASTER, "0").unwrap();
        if init(&mut t) {
            displays(&mut t);
            let window = Window::create("kmsdrm_video", 1024, 768, WindowFlags::default());
            t.line(format!("window: {}", result(&window)));
            if let Ok(window) = window {
                let context = GlContext::new(&window);
                t.line(format!("context: {}", result(&context)));
                if context.is_ok() {
                    gl_clear(1.0, 0.0, 1.0);
                    t.line(format!("swap: {}", result(&gl::gl_swap_window(&window))));
                }
                drop(context);
                window.destroy();
                t.line("window destroyed");
            }
            quit(&mut t);
        }
        hints::reset(hints::KMSDRM_REQUIRE_DRM_MASTER);
        t.ask("drm state");
        t.ask("drm master release");
        t.ask("drm state");

        t.finish(PROGRAM, EXPECTED)
    }
}
