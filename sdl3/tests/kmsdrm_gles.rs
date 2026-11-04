// Tests of OpenGL (ES) on the KMS/DRM video driver: its three swap paths,
// the OpenGL and OpenGL ES 2 renderers, and window surfaces, on a vkms
// card.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! A program (`harness = false`), run as root on tty3 by
//! `tools/linux_console_run.py --vt 3` (see `kmsdrm_support`), on the
//! KMS/DRM driver and a vkms card (Mesa's kms_swrast and llvmpipe behind
//! EGL on GBM), tracing each frame as the card scans it out (the runner's
//! `drm crc`, the CRC vkms computes of the composed frame, and `drm
//! pixels`, read from the primary plane's framebuffer):
//!
//! * frames of one color (`glClear()`) swapped by each of the swap paths:
//!   the atomic commit fenced both ways (the default: vkms's EGL has
//!   EGL_ANDROID_native_fence_sync), the blocking atomic commit
//!   (`SDL_VIDEO_DOUBLE_BUFFER=1`), the legacy `drmModeSetCrtc()` and
//!   `drmModePageFlip()` (`SDL_KMSDRM_ATOMIC=0`, with and without
//!   `SDL_VIDEO_DOUBLE_BUFFER`), at swap intervals 0 and 1;
//! * the OpenGL ES 2 and OpenGL renderers: their frames (a clear, filled
//!   rectangles, a texture), presented;
//! * the window surface (`SDL_GetWindowSurface()`, through a renderer's
//!   texture: the driver has no framebuffer of its own), filled and
//!   updated.
//!
//! Apart from the trace (lines `rust: `), children of the program
//! (`--child scenes NAME`) draw the renderer scenes of the other
//! renderers' tests (`uikit_support/scenes.rs`) with the OpenGL ES 2 and
//! OpenGL renderers on the card, and compare them with the software
//! renderer's.
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
#[path = "uikit_support/scenes.rs"]
mod scenes;

#[cfg(target_os = "linux")]
fn main() -> std::process::ExitCode {
    app::main()
}

#[cfg(target_os = "linux")]
mod app {
    use std::io::{BufRead, BufReader};
    use std::process::{Command, ExitCode, Stdio};

    use super::kmsdrm_support::{gl_clear, result, run_on_tty3, Trace};
    use super::scenes;
    use sdl3::events::window::WindowFlags;
    use sdl3::hints;
    use sdl3::init::{self, InitFlags};
    use sdl3::render::Renderer;
    use sdl3::video::gl::{self, GlContext};
    use sdl3::video::{self, FRect, PixelFormat, Rect, Surface, Window};

    /// Upstream's trace (the C program, by the runner, on the CI runner).
    const EXPECTED: &str = "\
versions: (the reference's)
";

    const PROGRAM: &str = "kmsdrm_gles";

    /// The points read back of each frame.
    const PIXELS: &str = "drm pixels 0 0 100 100 300 200 512 384 700 500 1023 767";

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

    /// What the card scans out after a frame.
    fn scanout(t: &mut Trace) {
        sdl3::events::pump();
        while sdl3::events::poll().is_some() {}
        t.ask("drm crc");
        t.ask(PIXELS);
    }

    /// The swap paths: frames of one color.
    fn swap_path(t: &mut Trace, label: &str, settings: &[(&str, &str)]) {
        t.line(format!("-- {label}"));
        for (name, value) in settings {
            hints::set(name, value).unwrap();
        }
        if init(t) {
            let window = Window::create("kmsdrm_gles", 1024, 768, WindowFlags::default());
            t.line(format!("window: {}", result(&window)));
            if let Ok(window) = window {
                let context = GlContext::new(&window);
                t.line(format!("context: {}", result(&context)));
                if let Ok(context) = context {
                    let colors = [(1.0, 0.0, 0.0), (0.0, 1.0, 0.0), (0.0, 0.0, 1.0)];
                    for interval in [0, 1] {
                        t.line(format!(
                            "swap interval {interval}: {}",
                            result(&gl::gl_set_swap_interval(interval))
                        ));
                        for (i, &(r, g, b)) in colors.iter().enumerate() {
                            gl_clear(r, g, b);
                            t.line(format!(
                                "swap {i}: {}",
                                result(&gl::gl_swap_window(&window))
                            ));
                            if i == 0 && interval == 0 {
                                t.ask("drm state");
                            }
                            scanout(t);
                        }
                    }
                    // (two swaps, read once: the second of a pair)
                    gl_clear(1.0, 1.0, 1.0);
                    let _ = gl::gl_swap_window(&window);
                    gl_clear(0.5, 0.5, 0.5);
                    t.line(format!(
                        "swap twice: {}",
                        result(&gl::gl_swap_window(&window))
                    ));
                    scanout(t);
                    drop(context);
                }
                window.destroy();
            }
            quit(t);
        }
        for (name, _) in settings {
            hints::reset(name);
        }
        t.ask("drm state");
    }

    /// A 16x16 texture pattern (the one the renderer scenes use).
    fn pattern() -> Surface<'static> {
        let mut s = Surface::new(16, 16, PixelFormat::ARGB8888).unwrap();
        for y in 0..16 {
            for x in 0..16 {
                let color = s.map_rgb(
                    (x * 16) as u8,
                    (y * 16) as u8,
                    if (x + y) % 2 == 0 { 255 } else { 0 },
                );
                s.fill_rect(Some(&Rect::new(x, y, 1, 1)), color).unwrap();
            }
        }
        s
    }

    /// The renderers' frames.
    fn renderer(t: &mut Trace, name: &str) {
        t.line(format!("-- renderer {name}"));
        if !init(t) {
            return;
        }
        let window = Window::create("kmsdrm_gles", 1024, 768, WindowFlags::default());
        t.line(format!("window: {}", result(&window)));
        if let Ok(window) = window {
            let r = Renderer::for_window(&window, Some(name));
            t.line(format!("renderer: {}", result(&r)));
            if let Ok(mut r) = r {
                t.line(format!("name: {}", r.name()));
                t.line(format!(
                    "output size: {:?}",
                    r.output_size().unwrap_or_default()
                ));
                let mut surface = pattern();
                let texture = r.create_texture_from_surface(&mut surface);
                t.line(format!("texture: {}", result(&texture)));
                for frame in 0..2 {
                    r.set_draw_color(32, 64 + frame * 64, 96, 255);
                    let _ = r.clear();
                    r.set_draw_color(255, 0, 0, 255);
                    let _ = r.render_fill_rect(Some(&FRect::new(50.0, 50.0, 100.0, 100.0)));
                    r.set_draw_color(0, 255, 0, 255);
                    let _ = r.render_fill_rect(Some(&FRect::new(250.0, 150.0, 100.0, 100.0)));
                    r.set_draw_color(0, 0, 255, 255);
                    let _ = r.render_fill_rect(Some(&FRect::new(
                        450.0 + frame as f32 * 50.0,
                        334.0,
                        124.0,
                        100.0,
                    )));
                    if let Ok(texture) = texture {
                        let _ = r.render_texture(
                            texture,
                            None,
                            Some(&FRect::new(692.0, 492.0, 16.0, 16.0)),
                        );
                    }
                    t.line(format!("present {frame}: {}", result(&r.present())));
                    scanout(t);
                }
                t.ask("drm state");
            }
            window.destroy();
        }
        quit(t);
    }

    /// The window surface.
    fn window_surface(t: &mut Trace) {
        t.line("-- window surface");
        if !init(t) {
            return;
        }
        let window = Window::create("kmsdrm_gles", 1024, 768, WindowFlags::default());
        t.line(format!("window: {}", result(&window)));
        if let Ok(window) = window {
            let surface = window.surface();
            t.line(format!("surface: {}", result(&surface)));
            if let Ok(surface) = surface {
                {
                    let s = surface.lock();
                    t.line(format!(
                        "surface: {}x{} {}",
                        s.width(),
                        s.height(),
                        s.format().name()
                    ));
                }
                for frame in 0..2 {
                    {
                        let mut s = surface.lock();
                        let bg = s.map_rgb(16, 32 + frame * 100, 48);
                        let _ = s.fill_rect(None, bg);
                        let red = s.map_rgb(255, 0, 0);
                        let _ = s.fill_rect(Some(&Rect::new(50, 50, 100, 100)), red);
                        let green = s.map_rgb(0, 255, 0);
                        let _ = s.fill_rect(
                            Some(&Rect::new(250 + frame as i32 * 25, 150, 100, 100)),
                            green,
                        );
                    }
                    t.line(format!(
                        "update {frame}: {}",
                        result(&window.update_surface())
                    ));
                    scanout(t);
                }
                t.ask("drm state");
            }
            window.destroy();
        }
        quit(t);
    }

    /// `--child scenes NAME`: the renderer scenes with the renderer NAME,
    /// compared with the software renderer (Rust only).
    fn child_scenes(name: &str) -> ExitCode {
        if let Err(e) = init::init(InitFlags::VIDEO) {
            println!("child: init: {e}");
            return ExitCode::FAILURE;
        }
        let window = Window::create("kmsdrm_gles", 1024, 768, WindowFlags::default()).unwrap();
        let mut r = Renderer::for_window(&window, Some(name)).unwrap();
        scenes::scenes(&mut r, name);
        scenes::check_custom_blend(&mut r);
        let mismatches = scenes::MISMATCHES.lock().unwrap().clone();
        let count = scenes::TRACE.lock().unwrap().len();
        drop(r);
        window.destroy();
        init::quit();
        if mismatches.is_empty() {
            println!("child: {count} scenes, the software renderer's");
            ExitCode::SUCCESS
        } else {
            for m in &mismatches {
                println!("child: {m}");
            }
            ExitCode::FAILURE
        }
    }

    pub(super) fn main() -> ExitCode {
        let args: Vec<String> = std::env::args().collect();
        if args.len() == 4 && args[1] == "--child" && args[2] == "scenes" {
            return child_scenes(&args[3]);
        }
        if let Err(code) = run_on_tty3(PROGRAM, false) {
            return code;
        }
        let mut t = Trace::new();
        t.ask("drm state");

        swap_path(&mut t, "fenced (atomic)", &[]);
        swap_path(
            &mut t,
            "double buffered (atomic, SDL_VIDEO_DOUBLE_BUFFER=1)",
            &[(hints::VIDEO_DOUBLE_BUFFER, "1")],
        );
        swap_path(
            &mut t,
            "legacy (SDL_KMSDRM_ATOMIC=0)",
            &[(hints::KMSDRM_ATOMIC, "0")],
        );
        swap_path(
            &mut t,
            "legacy, double buffered",
            &[
                (hints::KMSDRM_ATOMIC, "0"),
                (hints::VIDEO_DOUBLE_BUFFER, "1"),
            ],
        );
        renderer(&mut t, "opengles2");
        renderer(&mut t, "opengl");
        window_surface(&mut t);

        // The renderer scenes against the software renderer (Rust only)
        for name in ["opengles2", "opengl"] {
            let mut child = Command::new(std::env::current_exe().unwrap())
                .args(["--child", "scenes", name])
                .stdout(Stdio::piped())
                .spawn()
                .expect("the child runs");
            let mut said = Vec::new();
            for line in BufReader::new(child.stdout.take().unwrap()).lines() {
                let line = line.expect("the child's output");
                if let Some(l) = line.strip_prefix("child: ") {
                    said.push(l.to_owned());
                }
            }
            let status = child.wait().expect("the child ends");
            t.rust(
                status.success(),
                format!("scenes {name}: {} ({status})", said.join("; ")),
            );
        }

        t.ask("drm state");
        t.finish(PROGRAM, EXPECTED)
    }
}
