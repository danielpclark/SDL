// Tests of the KMS/DRM video driver's cursor and warps on a vkms card.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! A program (`harness = false`), run as root on tty3 by
//! `tools/linux_console_run.py --vt 3` (see `kmsdrm_support`), on the
//! KMS/DRM driver and a vkms card loaded with its cursor plane
//! (`enable_cursor=1`, where `drmModeSetCursor()` puts the driver's cursor
//! buffer), tracing the cursor as the card scans it out (the runner's
//! `drm state`: the cursor plane's framebuffer and position; `drm crc`:
//! the frame composed with it):
//!
//! * before the first window, the mouse's entry points the driver hasn't
//!   set yet (the warps, cursors);
//! * the default cursor, which the window's creation shows (and its
//!   focus), on the frame the first swap sets;
//! * warps in the window and global: the motion events, the mouse's
//!   position, the cursor plane moved;
//! * a color cursor with a hot spot (`drmModeSetCursor2()`), its
//!   position, hidden and shown, the relative mode (the evdev reader's:
//!   the cursor hidden while it's on), a system cursor (which the driver
//!   can't make), the cursor destroyed (the default one again);
//! * the window destroyed, and SDL quit.
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

    use super::kmsdrm_support::{gl_clear, result, run_on_tty3, Trace};
    use sdl3::events::mouse::{self, SystemCursor};
    use sdl3::events::window::WindowFlags;
    use sdl3::events::{self, Event};
    use sdl3::init::{self, InitFlags};
    use sdl3::video::gl::{self, GlContext};
    use sdl3::video::{self, PixelFormat, Rect, Surface, Window};

    /// Upstream's trace (the C program, by the runner, on the CI runner).
    const EXPECTED: &str = "\
versions: (the reference's)
";

    const PROGRAM: &str = "kmsdrm_cursor";

    /// The mouse events since the last call, and the mouse's state.
    fn mouse_events(t: &mut Trace, window: &Window) {
        events::pump();
        while let Some(event) = events::poll() {
            let w = |id| {
                if id == window.id() {
                    "window"
                } else {
                    "another"
                }
            };
            match event {
                Event::MouseMotion(m) => t.line(format!(
                    "motion {} at ({:.1}, {:.1}) rel ({:.1}, {:.1}) {}",
                    m.which,
                    m.x,
                    m.y,
                    m.xrel,
                    m.yrel,
                    w(m.window_id)
                )),
                Event::Window(e) => t.line(format!(
                    "window event 0x{:x} {} {} {}",
                    e.event_type.0,
                    e.data1,
                    e.data2,
                    w(e.window_id)
                )),
                _ => {}
            }
        }
        let (x, y, buttons) = mouse::mouse_state();
        t.line(format!(
            "mouse: at ({x:.1}, {y:.1}) buttons 0x{:x} focus {} cursor visible {}",
            buttons.0,
            match mouse::mouse_focus() {
                Some(id) if id == window.id() => "window",
                Some(_) => "another",
                None => "none",
            },
            mouse::cursor_visible()
        ));
    }

    /// The cursor as scanned out.
    fn cursor(t: &mut Trace) {
        t.ask("drm state");
        t.ask("drm crc");
    }

    /// A 32x32 cursor: a red square with a white border, its hot spot at
    /// (5, 7).
    fn color_cursor() -> sdl3::Result<mouse::Cursor> {
        let mut s = Surface::new(32, 32, PixelFormat::ARGB8888)?;
        let white = s.map_rgba(255, 255, 255, 255);
        s.fill_rect(None, white)?;
        let red = s.map_rgba(255, 0, 0, 255);
        s.fill_rect(Some(&Rect::new(2, 2, 28, 28)), red)?;
        let half = s.map_rgba(0, 0, 255, 128);
        s.fill_rect(Some(&Rect::new(8, 8, 16, 16)), half)?;
        mouse::create_color_cursor(&s, 5, 7)
    }

    pub(super) fn main() -> ExitCode {
        if let Err(code) = run_on_tty3(PROGRAM, false) {
            return code;
        }
        let mut t = Trace::new();

        t.line("-- init");
        if let Err(e) = init::init(InitFlags::VIDEO) {
            t.line(format!("init: {e}"));
            return t.finish(PROGRAM, EXPECTED);
        }
        t.line(format!(
            "init: ok ({})",
            video::current_video_driver().unwrap_or("(none)")
        ));
        // (no window yet: the mouse's entry points aren't the driver's)
        t.line(format!(
            "warp global before a window: {}",
            result(&mouse::warp_mouse_global(10.0, 10.0))
        ));
        t.line(format!(
            "color cursor before a window: {}",
            result(&color_cursor())
        ));
        t.line(format!(
            "default cursor before a window: {}",
            mouse::default_cursor().is_some()
        ));

        t.line("-- window");
        let window = Window::create("kmsdrm_cursor", 1024, 768, WindowFlags::default());
        t.line(format!("window: {}", result(&window)));
        let Ok(window) = window else {
            init::quit();
            return t.finish(PROGRAM, EXPECTED);
        };
        t.line(format!(
            "default cursor: {}, current is default: {}",
            mouse::default_cursor().is_some(),
            mouse::cursor() == mouse::default_cursor()
        ));
        mouse_events(&mut t, &window);
        t.ask("drm state");
        let context = GlContext::new(&window);
        t.line(format!("context: {}", result(&context)));
        gl_clear(0.0, 0.0, 0.5);
        t.line(format!("swap: {}", result(&gl::gl_swap_window(&window))));
        mouse_events(&mut t, &window);
        cursor(&mut t);

        t.line("-- warps");
        mouse::warp_mouse_in_window(Some(window.id()), 100.0, 200.0);
        mouse_events(&mut t, &window);
        cursor(&mut t);
        t.line(format!(
            "warp global: {}",
            result(&mouse::warp_mouse_global(300.5, 400.5))
        ));
        mouse_events(&mut t, &window);
        cursor(&mut t);
        mouse::warp_mouse_in_window(Some(window.id()), 1023.0, 767.0);
        mouse_events(&mut t, &window);
        t.ask("drm state");

        t.line("-- color cursor");
        mouse::warp_mouse_in_window(Some(window.id()), 400.0, 300.0);
        mouse_events(&mut t, &window);
        let custom = color_cursor();
        t.line(format!("color cursor: {}", result(&custom)));
        if let Ok(custom) = &custom {
            t.line(format!(
                "set cursor: {}",
                result(&mouse::set_cursor(Some(custom)))
            ));
            cursor(&mut t);
            mouse::warp_mouse_in_window(Some(window.id()), 50.0, 60.0);
            mouse_events(&mut t, &window);
            cursor(&mut t);
        }

        t.line("-- hidden and shown");
        mouse::hide_cursor();
        mouse_events(&mut t, &window);
        cursor(&mut t);
        mouse::show_cursor();
        mouse_events(&mut t, &window);
        cursor(&mut t);

        t.line("-- relative mode");
        t.line(format!(
            "relative mode on: {}",
            result(&window.set_relative_mouse_mode(true))
        ));
        mouse_events(&mut t, &window);
        t.ask("drm state");
        t.line(format!(
            "relative mode off: {}",
            result(&window.set_relative_mouse_mode(false))
        ));
        mouse_events(&mut t, &window);
        t.ask("drm state");

        t.line("-- system cursor");
        t.line(format!(
            "system cursor: {}",
            result(&mouse::create_system_cursor(SystemCursor::Pointer))
        ));

        t.line("-- destroyed");
        if let Ok(custom) = custom {
            mouse::destroy_cursor(custom);
            t.line(format!(
                "cursor destroyed, current is default: {}",
                mouse::cursor() == mouse::default_cursor()
            ));
        }
        mouse_events(&mut t, &window);
        cursor(&mut t);
        drop(context);
        window.destroy();
        t.line("window destroyed");
        t.ask("drm state");
        init::quit();
        t.line("quit");
        t.ask("drm state");

        t.finish(PROGRAM, EXPECTED)
    }
}
