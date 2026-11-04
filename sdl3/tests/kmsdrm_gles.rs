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
//!   rectangles, a translucent one blended, a texture), read back (their
//!   hash) and presented;
//! * the window surface (`SDL_GetWindowSurface()`, through a renderer's
//!   texture: the driver has no framebuffer of its own), filled and
//!   updated.
//!
//! Apart from the trace (lines `rust: `), the renderers' frames read back
//! are compared with the software renderer's (each channel within 1).
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

    use super::kmsdrm_support::scene::{differing_from_software, draw_frame, fnv, pattern, rgba};
    use super::kmsdrm_support::{gl_clear, result, run_on_tty3, Trace};
    use sdl3::events::window::WindowFlags;
    use sdl3::hints;
    use sdl3::init::{self, InitFlags};
    use sdl3::render::Renderer;
    use sdl3::video::gl::{self, GlContext};
    use sdl3::video::{self, Rect, Window};

    /// Upstream's trace (the C program, by the runner, on the CI runner).
    const EXPECTED: &str = "\
versions: kernel 7.0.0-1017-azure, Mesa 25.2.8-0ubuntu0.24.04.4, libdrm 2.4.125-1ubuntu0.1~24.04.2
drm: crtc 1024x768 (1024x768) on
drm: plane primary fb 1024x768 XR24 src 0,0 1024x768 crtc 0,0 1024x768
-- fenced (atomic)
init: ok (kmsdrm)
window: ok
context: ok
swap interval 0: ok
swap 0: ok
drm: crtc 1024x768 (1024x768) on
drm: plane primary fb 1024x768 XR24 src 0,0 1024x768 crtc 0,0 1024x768
drm: plane cursor fb 512x512 AR24 src 0,0 512x512 crtc 0,0 512x512
drm: crc 0xbc7130f0
drm: pixel 0,0 0x00000000
drm: pixel 100,100 0x00000000
drm: pixel 300,200 0x00000000
drm: pixel 512,384 0x00000000
drm: pixel 700,500 0x00000000
drm: pixel 1023,767 0x00000000
swap 1: ok
drm: crc 0x87ba7f7a
drm: pixel 0,0 0xff00ff00
drm: pixel 100,100 0xff00ff00
drm: pixel 300,200 0xff00ff00
drm: pixel 512,384 0xff00ff00
drm: pixel 700,500 0xff00ff00
drm: pixel 1023,767 0xff00ff00
swap 2: ok
drm: crc 0xd9867bd7
drm: pixel 0,0 0xff0000ff
drm: pixel 100,100 0xff0000ff
drm: pixel 300,200 0xff0000ff
drm: pixel 512,384 0xff0000ff
drm: pixel 700,500 0xff0000ff
drm: pixel 1023,767 0xff0000ff
swap interval 1: ok
swap 0: ok
drm: crc 0x9b7e27fb
drm: pixel 0,0 0xffff0000
drm: pixel 100,100 0xffff0000
drm: pixel 300,200 0xffff0000
drm: pixel 512,384 0xffff0000
drm: pixel 700,500 0xffff0000
drm: pixel 1023,767 0xffff0000
swap 1: ok
drm: crc 0x87ba7f7a
drm: pixel 0,0 0xff00ff00
drm: pixel 100,100 0xff00ff00
drm: pixel 300,200 0xff00ff00
drm: pixel 512,384 0xff00ff00
drm: pixel 700,500 0xff00ff00
drm: pixel 1023,767 0xff00ff00
swap 2: ok
drm: crc 0xd9867bd7
drm: pixel 0,0 0xff0000ff
drm: pixel 100,100 0xff0000ff
drm: pixel 300,200 0xff0000ff
drm: pixel 512,384 0xff0000ff
drm: pixel 700,500 0xff0000ff
drm: pixel 1023,767 0xff0000ff
swap twice: ok
drm: crc 0xf99e45f0
drm: pixel 0,0 0xff808080
drm: pixel 100,100 0xff808080
drm: pixel 300,200 0xff808080
drm: pixel 512,384 0xff808080
drm: pixel 700,500 0xff808080
drm: pixel 1023,767 0xff808080
quit
drm: crtc 1024x768 (1024x768) on
drm: plane primary fb 1024x768 XR24 src 0,0 1024x768 crtc 0,0 1024x768
-- double buffered (atomic, SDL_VIDEO_DOUBLE_BUFFER=1)
init: ok (kmsdrm)
window: ok
context: ok
swap interval 0: ok
swap 0: ok
drm: crtc 1024x768 (1024x768) on
drm: plane primary fb 1024x768 XR24 src 0,0 1024x768 crtc 0,0 1024x768
drm: plane cursor fb 512x512 AR24 src 0,0 512x512 crtc 0,0 512x512
drm: crc 0xbc7130f0
drm: pixel 0,0 0x00000000
drm: pixel 100,100 0x00000000
drm: pixel 300,200 0x00000000
drm: pixel 512,384 0x00000000
drm: pixel 700,500 0x00000000
drm: pixel 1023,767 0x00000000
swap 1: ok
drm: crc 0x87ba7f7a
drm: pixel 0,0 0xff00ff00
drm: pixel 100,100 0xff00ff00
drm: pixel 300,200 0xff00ff00
drm: pixel 512,384 0xff00ff00
drm: pixel 700,500 0xff00ff00
drm: pixel 1023,767 0xff00ff00
swap 2: ok
drm: crc 0xd9867bd7
drm: pixel 0,0 0xff0000ff
drm: pixel 100,100 0xff0000ff
drm: pixel 300,200 0xff0000ff
drm: pixel 512,384 0xff0000ff
drm: pixel 700,500 0xff0000ff
drm: pixel 1023,767 0xff0000ff
swap interval 1: ok
swap 0: ok
drm: crc 0x9b7e27fb
drm: pixel 0,0 0xffff0000
drm: pixel 100,100 0xffff0000
drm: pixel 300,200 0xffff0000
drm: pixel 512,384 0xffff0000
drm: pixel 700,500 0xffff0000
drm: pixel 1023,767 0xffff0000
swap 1: ok
drm: crc 0x87ba7f7a
drm: pixel 0,0 0xff00ff00
drm: pixel 100,100 0xff00ff00
drm: pixel 300,200 0xff00ff00
drm: pixel 512,384 0xff00ff00
drm: pixel 700,500 0xff00ff00
drm: pixel 1023,767 0xff00ff00
swap 2: ok
drm: crc 0xd9867bd7
drm: pixel 0,0 0xff0000ff
drm: pixel 100,100 0xff0000ff
drm: pixel 300,200 0xff0000ff
drm: pixel 512,384 0xff0000ff
drm: pixel 700,500 0xff0000ff
drm: pixel 1023,767 0xff0000ff
swap twice: ok
drm: crc 0xf99e45f0
drm: pixel 0,0 0xff808080
drm: pixel 100,100 0xff808080
drm: pixel 300,200 0xff808080
drm: pixel 512,384 0xff808080
drm: pixel 700,500 0xff808080
drm: pixel 1023,767 0xff808080
quit
drm: crtc 1024x768 (1024x768) on
drm: plane primary fb 1024x768 XR24 src 0,0 1024x768 crtc 0,0 1024x768
-- legacy (SDL_KMSDRM_ATOMIC=0)
init: ok (kmsdrm)
window: ok
context: ok
swap interval 0: ok
swap 0: ok
drm: crtc 1024x768 (1024x768) on
drm: plane primary fb 1024x768 XR24 src 0,0 1024x768 crtc 0,0 1024x768
drm: plane cursor fb 512x512 AR24 src 0,0 512x512 crtc 0,0 512x512
drm: crc 0xbc7130f0
drm: pixel 0,0 0x00000000
drm: pixel 100,100 0x00000000
drm: pixel 300,200 0x00000000
drm: pixel 512,384 0x00000000
drm: pixel 700,500 0x00000000
drm: pixel 1023,767 0x00000000
swap 1: ok
drm: crc 0x87ba7f7a
drm: pixel 0,0 0xff00ff00
drm: pixel 100,100 0xff00ff00
drm: pixel 300,200 0xff00ff00
drm: pixel 512,384 0xff00ff00
drm: pixel 700,500 0xff00ff00
drm: pixel 1023,767 0xff00ff00
swap 2: ok
drm: crc 0xd9867bd7
drm: pixel 0,0 0xff0000ff
drm: pixel 100,100 0xff0000ff
drm: pixel 300,200 0xff0000ff
drm: pixel 512,384 0xff0000ff
drm: pixel 700,500 0xff0000ff
drm: pixel 1023,767 0xff0000ff
swap interval 1: ok
swap 0: ok
drm: crc 0x9b7e27fb
drm: pixel 0,0 0xffff0000
drm: pixel 100,100 0xffff0000
drm: pixel 300,200 0xffff0000
drm: pixel 512,384 0xffff0000
drm: pixel 700,500 0xffff0000
drm: pixel 1023,767 0xffff0000
swap 1: ok
drm: crc 0x87ba7f7a
drm: pixel 0,0 0xff00ff00
drm: pixel 100,100 0xff00ff00
drm: pixel 300,200 0xff00ff00
drm: pixel 512,384 0xff00ff00
drm: pixel 700,500 0xff00ff00
drm: pixel 1023,767 0xff00ff00
swap 2: ok
drm: crc 0xd9867bd7
drm: pixel 0,0 0xff0000ff
drm: pixel 100,100 0xff0000ff
drm: pixel 300,200 0xff0000ff
drm: pixel 512,384 0xff0000ff
drm: pixel 700,500 0xff0000ff
drm: pixel 1023,767 0xff0000ff
swap twice: ok
drm: crc 0xf99e45f0
drm: pixel 0,0 0xff808080
drm: pixel 100,100 0xff808080
drm: pixel 300,200 0xff808080
drm: pixel 512,384 0xff808080
drm: pixel 700,500 0xff808080
drm: pixel 1023,767 0xff808080
quit
drm: crtc 1024x768 (1024x768) on
drm: plane primary fb 1024x768 XR24 src 0,0 1024x768 crtc 0,0 1024x768
-- legacy, double buffered
init: ok (kmsdrm)
window: ok
context: ok
swap interval 0: ok
swap 0: ok
drm: crtc 1024x768 (1024x768) on
drm: plane primary fb 1024x768 XR24 src 0,0 1024x768 crtc 0,0 1024x768
drm: plane cursor fb 512x512 AR24 src 0,0 512x512 crtc 0,0 512x512
drm: crc 0xbc7130f0
drm: pixel 0,0 0x00000000
drm: pixel 100,100 0x00000000
drm: pixel 300,200 0x00000000
drm: pixel 512,384 0x00000000
drm: pixel 700,500 0x00000000
drm: pixel 1023,767 0x00000000
swap 1: ok
drm: crc 0x87ba7f7a
drm: pixel 0,0 0xff00ff00
drm: pixel 100,100 0xff00ff00
drm: pixel 300,200 0xff00ff00
drm: pixel 512,384 0xff00ff00
drm: pixel 700,500 0xff00ff00
drm: pixel 1023,767 0xff00ff00
swap 2: ok
drm: crc 0xd9867bd7
drm: pixel 0,0 0xff0000ff
drm: pixel 100,100 0xff0000ff
drm: pixel 300,200 0xff0000ff
drm: pixel 512,384 0xff0000ff
drm: pixel 700,500 0xff0000ff
drm: pixel 1023,767 0xff0000ff
swap interval 1: ok
swap 0: ok
drm: crc 0x9b7e27fb
drm: pixel 0,0 0xffff0000
drm: pixel 100,100 0xffff0000
drm: pixel 300,200 0xffff0000
drm: pixel 512,384 0xffff0000
drm: pixel 700,500 0xffff0000
drm: pixel 1023,767 0xffff0000
swap 1: ok
drm: crc 0x87ba7f7a
drm: pixel 0,0 0xff00ff00
drm: pixel 100,100 0xff00ff00
drm: pixel 300,200 0xff00ff00
drm: pixel 512,384 0xff00ff00
drm: pixel 700,500 0xff00ff00
drm: pixel 1023,767 0xff00ff00
swap 2: ok
drm: crc 0xd9867bd7
drm: pixel 0,0 0xff0000ff
drm: pixel 100,100 0xff0000ff
drm: pixel 300,200 0xff0000ff
drm: pixel 512,384 0xff0000ff
drm: pixel 700,500 0xff0000ff
drm: pixel 1023,767 0xff0000ff
swap twice: ok
drm: crc 0xf99e45f0
drm: pixel 0,0 0xff808080
drm: pixel 100,100 0xff808080
drm: pixel 300,200 0xff808080
drm: pixel 512,384 0xff808080
drm: pixel 700,500 0xff808080
drm: pixel 1023,767 0xff808080
quit
drm: crtc 1024x768 (1024x768) on
drm: plane primary fb 1024x768 XR24 src 0,0 1024x768 crtc 0,0 1024x768
-- renderer opengles2
init: ok (kmsdrm)
window: ok
renderer: ok
name: opengles2
output size: (1024, 768)
texture: ok
read back 0: 1024x768 9b087a25
present 0: ok
drm: crc 0xbc7130f0
drm: pixel 0,0 0x00000000
drm: pixel 100,100 0x00000000
drm: pixel 300,200 0x00000000
drm: pixel 512,384 0x00000000
drm: pixel 700,500 0x00000000
drm: pixel 1023,767 0x00000000
read back 1: 1024x768 b9939625
present 1: ok
drm: crc 0xeb45f386
drm: pixel 0,0 0xff208060
drm: pixel 100,100 0xffff8080
drm: pixel 300,200 0xff00ff00
drm: pixel 512,384 0xff0000ff
drm: pixel 700,500 0xff8080ff
drm: pixel 1023,767 0xff208060
drm: crtc 1024x768 (1024x768) on
drm: plane primary fb 1024x768 XR24 src 0,0 1024x768 crtc 0,0 1024x768
drm: plane cursor fb 512x512 AR24 src 0,0 512x512 crtc 0,0 512x512
quit
-- renderer opengl
init: ok (kmsdrm)
window: ok
renderer: ok
name: opengl
output size: (1024, 768)
texture: ok
read back 0: 1024x768 9b087a25
present 0: ok
drm: crc 0xbc7130f0
drm: pixel 0,0 0x00000000
drm: pixel 100,100 0x00000000
drm: pixel 300,200 0x00000000
drm: pixel 512,384 0x00000000
drm: pixel 700,500 0x00000000
drm: pixel 1023,767 0x00000000
read back 1: 1024x768 b9939625
present 1: ok
drm: crc 0xeb45f386
drm: pixel 0,0 0xff208060
drm: pixel 100,100 0xffff8080
drm: pixel 300,200 0xff00ff00
drm: pixel 512,384 0xff0000ff
drm: pixel 700,500 0xff8080ff
drm: pixel 1023,767 0xff208060
drm: crtc 1024x768 (1024x768) on
drm: plane primary fb 1024x768 XR24 src 0,0 1024x768 crtc 0,0 1024x768
drm: plane cursor fb 512x512 AR24 src 0,0 512x512 crtc 0,0 512x512
quit
-- window surface
init: ok (kmsdrm)
window: ok
surface: ok
surface: 1024x768 SDL_PIXELFORMAT_ARGB8888
update 0: ok
drm: crc 0xbc7130f0
drm: pixel 0,0 0x00000000
drm: pixel 100,100 0x00000000
drm: pixel 300,200 0x00000000
drm: pixel 512,384 0x00000000
drm: pixel 700,500 0x00000000
drm: pixel 1023,767 0x00000000
update 1: ok
drm: crc 0x7e943c28
drm: pixel 0,0 0xff108430
drm: pixel 100,100 0xffff0000
drm: pixel 300,200 0xff00ff00
drm: pixel 512,384 0xff108430
drm: pixel 700,500 0xff108430
drm: pixel 1023,767 0xff108430
drm: crtc 1024x768 (1024x768) on
drm: plane primary fb 1024x768 XR24 src 0,0 1024x768 crtc 0,0 1024x768
drm: plane cursor fb 512x512 AR24 src 0,0 512x512 crtc 0,0 512x512
quit
drm: crtc 1024x768 (1024x768) on
drm: plane primary fb 1024x768 XR24 src 0,0 1024x768 crtc 0,0 1024x768
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

    /// The renderers' frames: read back (their hash traced) before they're
    /// presented, and (apart from the trace) compared with the software
    /// renderer's.
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
                let texture = r.create_texture_from_surface(&mut pattern());
                t.line(format!("texture: {}", result(&texture)));
                for frame in 0..2 {
                    draw_frame(&mut r, frame, texture.as_ref().ok().copied());
                    match r.read_pixels(None) {
                        Ok(read) => {
                            let image = rgba(&read);
                            t.line(format!(
                                "read back {frame}: {}x{} {:08x}",
                                image.0,
                                image.1,
                                fnv(&image.2)
                            ));
                            // The software renderer's frame (Rust only)
                            let bad = differing_from_software(&image, frame);
                            t.rust(
                                bad == Some(0),
                                format!(
                                    "{name} frame {frame}: {bad:?} pixels differ from the software renderer's"
                                ),
                            );
                        }
                        Err(e) => t.line(format!("read back {frame}: error: {e}")),
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

    pub(super) fn main() -> ExitCode {
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

        t.ask("drm state");
        t.finish(PROGRAM, EXPECTED)
    }
}
