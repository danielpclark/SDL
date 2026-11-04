// Tests of Vulkan on the KMS/DRM video driver, on a vkms card.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! A program (`harness = false`), run as root on tty3 by
//! `tools/linux_console_run.py --vt 3` (see `kmsdrm_support`), on the
//! KMS/DRM driver and a vkms card, with the system's Vulkan loader and
//! Mesa's lavapipe, tracing what is reachable there:
//!
//! * the instance extensions (`VK_KHR_surface`, `VK_KHR_display`), the
//!   loader loaded (counted, a second path refused), `vkGetInstanceProcAddr`;
//! * an instance with those extensions, and one without `VK_KHR_display`;
//!   the physical devices (their types; their names apart from the
//!   trace), and the displays and planes they enumerate (lavapipe: none on
//!   vkms);
//! * a Vulkan window: its flags (not OpenGL), size, events, and the card
//!   then (no GBM or EGL surface: the console's scanout stays);
//! * its surface, failing as upstream's does (lavapipe is a CPU device,
//!   which upstream's choice of a GPU, integrated or discrete only,
//!   refuses before it looks for displays), and on the instance without
//!   `VK_KHR_display`; the presentation support (the driver has no
//!   function: always);
//! * the driver's Vulkan mode: fullscreen with a mode of 800x600 and a new
//!   size (no mode set, no surfaces dirtied), an OpenGL window made in it
//!   (no GBM: its context fails), destroyed (which leaves Vulkan mode,
//!   though the Vulkan window stays: upstream's), the Vulkan window
//!   destroyed;
//! * an OpenGL window after it (GBM set up then), a frame swapped, the card
//!   then;
//! * the Vulkan and GPU renderers on a Vulkan window (failing with the
//!   surface);
//! * the loader unloaded, SDL quit.
//!
//! A surface, a swapchain, and the renderers drawing on it need a display
//! that the Vulkan driver enumerates: lavapipe offers `VK_KHR_display` but
//! enumerates no displays and no planes on vkms, so after the trace the
//! program skips the rest (`vulkan`, by design); `kmsdrm_hardware`'s
//! Vulkan check does it on a GPU's driver (anv, radv).
//!
//! The expected trace is upstream's C SDL's, from the same program in C
//! (upstream built with CMake at ea7a2da, `SDL_KMSDRM=ON`,
//! `SDL_KMSDRM_SHARED=ON`, `SDL_VULKAN=ON`, run by the same runner on the
//! same CI runner, kernel and Mesa), patched as the translation fixes
//! upstream's references to the loader (the loader would stay loaded at
//! the end): the Vulkan renderer's is released when it is destroyed,
//! which upstream never does (`Note (upstream)` in `render::vulkan`), and
//! the GPU API's Vulkan backend releases the one it takes for its entry
//! points, which upstream never does either, one more with each
//! PrepareDriver and CreateDevice (`Note (upstream)` in `gpu::vulkan`'s
//! `load_entry_points()`). Without the runner, it runs itself
//! under it as root when the capabilities are there (`kmsdrm`, `egl`,
//! `vt`), and skips otherwise, unless `SDL3_TEST_REQUIRE` asks for them;
//! without a Vulkan loader with `VK_KHR_display`, it skips (`vulkan`).
//! Elsewhere than Linux it does nothing.

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

    use super::kmsdrm_support::vk::Vk;
    use super::kmsdrm_support::{gl_clear, mode_line, result, run_on_tty3, skip, Trace};
    use sdl3::events::window::WindowFlags;
    use sdl3::events::{self, Event};
    use sdl3::init::{self, InitFlags};
    use sdl3::render::Renderer;
    use sdl3::video::gl::{self, GlContext};
    use sdl3::video::{self, vulkan, Window};

    /// Upstream's trace (the C program, by the runner, on the CI runner).
    const EXPECTED: &str = "\
versions: kernel 7.0.0-1017-azure, Mesa 25.2.8-0ubuntu0.24.04.4, libdrm 2.4.125-1ubuntu0.1~24.04.2
drm: crtc 1024x768 (1024x768) on
drm: plane primary fb 1024x768 XR24 src 0,0 1024x768 crtc 0,0 1024x768
-- init
init: ok (kmsdrm)
-- library
instance extensions: VK_KHR_surface, VK_KHR_display
load the default library: ok
load it again: ok
load another library: error: Vulkan loader library already loaded
vkGetInstanceProcAddr: ok
-- instance
instance: ok
instance without VK_KHR_display: ok
physical devices: 1
physical device 0: CPU, 0 displays, 0 planes
-- vulkan window
vulkan window: ok
vulkan window: flags 0x10000600 size 640x480 fullscreen mode none
window event 0x20c 0 0
window event 0x20e 0 0
window event 0x205 0 0
window event 0x202 0 0
window event 0x207 640 480
window event 0x215 0 0
drm: crtc 1024x768 (1024x768) on
drm: plane primary fb 1024x768 XR24 src 0,0 1024x768 crtc 0,0 1024x768
-- surface
surface: error: Vulkan can't find a valid physical device (gpu).
presentation support: true
surface without VK_KHR_display: error: VK_KHR_display extension is not enabled in the Vulkan instance.
drm: crtc 1024x768 (1024x768) on
drm: plane primary fb 1024x768 XR24 src 0,0 1024x768 crtc 0,0 1024x768
-- vulkan mode
set fullscreen mode: ok
set fullscreen: ok
window event 0x217 0 0
window event 0x206 800 600
window event 0x207 800 600
window event 0x215 0 0
vulkan window: flags 0x10000601 size 800x600 fullscreen mode 800x600@60.32 (40000000/663168) density 1.00 SDL_PIXELFORMAT_ARGB8888
current mode: 800x600@60.32 (40000000/663168) density 1.00 SDL_PIXELFORMAT_ARGB8888
set fullscreen false: ok
window event 0x218 0 0
current mode: 1024x768@60.00 (65000000/1083264) density 1.00 SDL_PIXELFORMAT_ARGB8888
set size: ok
vulkan window: flags 0x10000600 size 800x600 fullscreen mode 800x600@60.32 (40000000/663168) density 1.00 SDL_PIXELFORMAT_ARGB8888
drm: crtc 1024x768 (1024x768) on
drm: plane primary fb 1024x768 XR24 src 0,0 1024x768 crtc 0,0 1024x768
opengl window in vulkan mode: ok
opengl window: flags 0x602 size 640x480 fullscreen mode none
window event 0x20d 0 0
window event 0x20c 0 0
window event 0x20f 0 0
window event 0x20e 0 0
window event 0x205 0 0
window event 0x202 0 0
window event 0x207 640 480
window event 0x215 0 0
context: error: EGL not initialized
opengl window destroyed
window event 0x219 0 0
drm: crtc 1024x768 (1024x768) on
drm: plane primary fb 1024x768 XR24 src 0,0 1024x768 crtc 0,0 1024x768
vulkan window destroyed
window event 0x219 0 0
-- an opengl window after
window: ok
window: flags 0x602 size 1024x768 fullscreen mode none
window event 0x207 1024 768
window event 0x20c 0 0
window event 0x20e 0 0
window event 0x202 0 0
window event 0x215 0 0
context: ok
swap (1.0, 0.0, 0.0): ok
swap (0.0, 1.0, 0.0): ok
drm: crtc 1024x768 (1024x768) on
drm: plane primary fb 1024x768 XR24 src 0,0 1024x768 crtc 0,0 1024x768
drm: plane cursor fb 512x512 AR24 src 0,0 512x512 crtc 0,0 512x512
drm: crc 0x87ba7f7a
window destroyed
window event 0x219 0 0
-- renderers on a vulkan window
vulkan window: ok
window event 0x20c 0 0
window event 0x20e 0 0
window event 0x202 0 0
window event 0x207 1024 768
window event 0x215 0 0
renderer vulkan: error: Vulkan can't find a valid physical device (gpu).
vulkan window: flags 0x10000600 size 1024x768 fullscreen mode none
renderer gpu: error: Vulkan can't find a valid physical device (gpu).
vulkan window: flags 0x10000600 size 1024x768 fullscreen mode none
vulkan window destroyed
window event 0x219 0 0
unloaded: vkGetInstanceProcAddr: error: No Vulkan loader has been loaded
quit
drm: crtc 1024x768 (1024x768) on
drm: plane primary fb 1024x768 XR24 src 0,0 1024x768 crtc 0,0 1024x768
";

    const PROGRAM: &str = "kmsdrm_vulkan";

    /// The window events since the last call.
    fn window_events(t: &mut Trace) {
        events::pump();
        while let Some(event) = events::poll() {
            if let Event::Window(w) = event {
                t.line(format!(
                    "window event 0x{:x} {} {}",
                    w.event_type.0, w.data1, w.data2
                ));
            }
        }
    }

    fn window_state(t: &mut Trace, label: &str, window: &Window) {
        let flags = window.flags().unwrap_or_default();
        let (w, h) = window.size().unwrap_or_default();
        t.line(format!(
            "{label}: flags 0x{:x} size {w}x{h} fullscreen mode {}",
            flags.0,
            match window.fullscreen_mode() {
                Ok(Some(m)) => mode_line(&m),
                Ok(None) => "none".to_owned(),
                Err(e) => format!("error: {e}"),
            }
        ));
    }

    fn current_mode(t: &mut Trace, window: &Window) {
        let display = window.display().unwrap_or(0);
        if let Ok(m) = video::current_display_mode(display) {
            t.line(format!("current mode: {}", mode_line(&m)));
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

    /// The device types, as traced.
    fn device_type(device_type: u32) -> &'static str {
        match device_type {
            0 => "other",
            1 => "integrated GPU",
            2 => "discrete GPU",
            3 => "virtual GPU",
            4 => "CPU",
            _ => "unknown",
        }
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

        t.line("-- library");
        let extensions = vulkan::vulkan_instance_extensions();
        t.line(format!(
            "instance extensions: {}",
            extensions
                .as_ref()
                .map_or_else(|e| format!("error: {e}"), |x| x.join(", "))
        ));
        let loaded = vulkan::vulkan_load_library(None);
        t.line(format!("load the default library: {}", result(&loaded)));
        if let Err(e) = loaded {
            // (no loader, or one without VK_KHR_display: nothing to trace)
            quit(&mut t);
            return skip(
                PROGRAM,
                "vulkan",
                &format!("no Vulkan loader with VK_KHR_display ({e})"),
            );
        }
        t.line(format!(
            "load it again: {}",
            result(&vulkan::vulkan_load_library(None))
        ));
        t.line(format!(
            "load another library: {}",
            result(&vulkan::vulkan_load_library(Some("libvulkan.so")))
        ));
        t.line(format!(
            "vkGetInstanceProcAddr: {}",
            result(&vulkan::vulkan_get_vk_get_instance_proc_addr())
        ));

        t.line("-- instance");
        let vk = Vk::new();
        let extensions = extensions.unwrap_or_default();
        let instance = vk.create_instance(&extensions);
        t.line(format!(
            "instance: {}",
            match &instance {
                Ok(_) => "ok".to_owned(),
                Err(r) => format!("VkResult {r}"),
            }
        ));
        let plain = vk.create_instance(&["VK_KHR_surface"]);
        t.line(format!(
            "instance without VK_KHR_display: {}",
            match &plain {
                Ok(_) => "ok".to_owned(),
                Err(r) => format!("VkResult {r}"),
            }
        ));
        let mut devices_found = Vec::new();
        if let Ok(instance) = instance {
            let devices = vk.physical_devices(instance);
            t.line(format!("physical devices: {}", devices.len()));
            for (i, &d) in devices.iter().enumerate() {
                let props = vk.device_properties(instance, d);
                // (the name has the LLVM version and the CPU's vector width)
                println!("note: physical device {i}: {}", props.name);
                let counts = vk.display_and_plane_counts(instance, d);
                t.line(format!(
                    "physical device {i}: {}, {}",
                    device_type(props.device_type),
                    match counts {
                        Some((displays, planes)) => format!("{displays} displays, {planes} planes"),
                        None => "no VK_KHR_display functions".to_owned(),
                    }
                ));
                devices_found.push(format!(
                    "{} ({}, {})",
                    props.name,
                    device_type(props.device_type),
                    match counts {
                        Some((displays, planes)) => format!("{displays} displays, {planes} planes"),
                        None => "no VK_KHR_display".to_owned(),
                    }
                ));
            }
        }

        t.line("-- vulkan window");
        let window = Window::create("kmsdrm_vulkan", 640, 480, WindowFlags::VULKAN);
        t.line(format!("vulkan window: {}", result(&window)));
        let mut surface_error = None;
        if let Ok(window) = window {
            window_state(&mut t, "vulkan window", &window);
            window_events(&mut t);
            t.ask("drm state");

            t.line("-- surface");
            if let Ok(instance) = instance {
                let surface = vulkan::vulkan_create_surface(&window, instance as usize, 0);
                t.line(format!("surface: {}", result(&surface)));
                match surface {
                    Ok(surface) => vulkan::vulkan_destroy_surface(instance as usize, surface, 0),
                    Err(e) => surface_error = Some(e.to_string()),
                }
                t.line(format!(
                    "presentation support: {}",
                    match vulkan::vulkan_presentation_support(instance as usize, 1, 0) {
                        Ok(s) => s.to_string(),
                        Err(e) => format!("error: {e}"),
                    }
                ));
            }
            if let Ok(plain) = plain {
                let surface = vulkan::vulkan_create_surface(&window, plain as usize, 0);
                t.line(format!(
                    "surface without VK_KHR_display: {}",
                    result(&surface)
                ));
            }
            t.ask("drm state");

            t.line("-- vulkan mode");
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
            window_events(&mut t);
            window_state(&mut t, "vulkan window", &window);
            current_mode(&mut t, &window);
            t.line(format!(
                "set fullscreen false: {}",
                result(&window.set_fullscreen(false))
            ));
            window_events(&mut t);
            current_mode(&mut t, &window);
            t.line(format!("set size: {}", result(&window.set_size(1024, 768))));
            window_events(&mut t);
            window_state(&mut t, "vulkan window", &window);
            t.ask("drm state");

            let gl_window = Window::create("kmsdrm_vulkan gl", 640, 480, WindowFlags::OPENGL);
            t.line(format!(
                "opengl window in vulkan mode: {}",
                result(&gl_window)
            ));
            if let Ok(gl_window) = gl_window {
                window_state(&mut t, "opengl window", &gl_window);
                window_events(&mut t);
                let context = GlContext::new(&gl_window);
                t.line(format!("context: {}", result(&context)));
                drop(context);
                gl_window.destroy();
                t.line("opengl window destroyed");
                window_events(&mut t);
            }
            t.ask("drm state");
            window.destroy();
            t.line("vulkan window destroyed");
            window_events(&mut t);
        }

        t.line("-- an opengl window after");
        let window = Window::create("kmsdrm_vulkan", 1024, 768, WindowFlags::default());
        t.line(format!("window: {}", result(&window)));
        if let Ok(window) = window {
            window_state(&mut t, "window", &window);
            window_events(&mut t);
            let context = GlContext::new(&window);
            t.line(format!("context: {}", result(&context)));
            if context.is_ok() {
                for (r, g, b) in [(1.0, 0.0, 0.0), (0.0, 1.0, 0.0)] {
                    gl_clear(r, g, b);
                    t.line(format!(
                        "swap ({r:.1}, {g:.1}, {b:.1}): {}",
                        result(&gl::gl_swap_window(&window))
                    ));
                }
                window_events(&mut t);
                t.ask("drm state");
                t.ask("drm crc");
            }
            drop(context);
            window.destroy();
            t.line("window destroyed");
            window_events(&mut t);
        }

        t.line("-- renderers on a vulkan window");
        let window = Window::create("kmsdrm_vulkan", 1024, 768, WindowFlags::VULKAN);
        t.line(format!("vulkan window: {}", result(&window)));
        if let Ok(window) = window {
            window_events(&mut t);
            for name in ["vulkan", "gpu"] {
                let r = Renderer::for_window(&window, Some(name));
                t.line(format!("renderer {name}: {}", result(&r)));
                drop(r);
                window_state(&mut t, "vulkan window", &window);
            }
            window.destroy();
            t.line("vulkan window destroyed");
            window_events(&mut t);
        }

        if let Ok(plain) = plain {
            vk.destroy_instance(plain);
        }
        if let Ok(instance) = instance {
            vk.destroy_instance(instance);
        }
        vulkan::vulkan_unload_library();
        vulkan::vulkan_unload_library();
        t.line(format!(
            "unloaded: vkGetInstanceProcAddr: {}",
            result(&vulkan::vulkan_get_vk_get_instance_proc_addr())
        ));
        quit(&mut t);
        t.ask("drm state");

        let code = t.finish(PROGRAM, EXPECTED);
        if code != ExitCode::SUCCESS {
            return code;
        }
        match surface_error {
            // The rest needs a display the Vulkan driver enumerates
            Some(e) => skip(
                PROGRAM,
                "vulkan",
                &format!(
                    "by design: no display plane surface on this card ({e}); the Vulkan devices: {}; a surface, a swapchain and the Vulkan and GPU renderers on it are kmsdrm_hardware's check",
                    devices_found.join(", ")
                ),
            ),
            None => code,
        }
    }
}
