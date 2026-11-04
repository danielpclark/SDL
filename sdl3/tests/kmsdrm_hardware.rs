// The KMS/DRM video driver's hardware checks: what a vkms card can't show.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! `#[ignore]`d tests (`cargo test -p sdl3 --test kmsdrm_hardware --
//! --ignored`; see "Hardware checks" in docs/HARDWARE_TESTING.md), for a
//! machine with a real GPU and display, run as root: each runs itself
//! alone as root on tty3 through `tools/linux_console_run.py --vt 3` (see
//! `kmsdrm_support`), on the card the driver picks (or
//! `SDL_KMSDRM_DEVICE_INDEX`'s), with `SDL_VIDEO_DRIVER=kmsdrm`. They
//! print what they find, and skip as `kmsdrm`, `vt` or `vulkan` without
//! the pieces they need (fail with `SDL3_TEST_REQUIRE`).
//!
//! * Vulkan on a GPU's driver (anv, radv: `VK_KHR_display` with the
//!   card's displays): a Vulkan window's surface on a display plane, the
//!   GPU API's swapchain on it (frames cleared and presented), and the
//!   Vulkan and GPU renderers on it, their frames read back against the
//!   software renderer's (each channel within 1).
//! * The swap paths on a GPU (the fenced atomic commits with its
//!   `EGL_ANDROID_native_fence_sync` fences, the blocking ones, the legacy
//!   page flips), their page flips a refresh apart at swap interval 1;
//!   async page flips (`DRM_CAP_ASYNC_PAGE_FLIP`, the legacy flips at
//!   interval 0); a framebuffer modifier other than linear; the cursor
//!   plane's size (`DRM_CAP_CURSOR_WIDTH`/`HEIGHT`); each display against
//!   its connector's modes; VRR (`vrr_capable`, the CRTC's `VRR_ENABLED`
//!   set and restored); panel orientation (SDL's is 0: `FIXME (upstream)`,
//!   read from the CRTC's ID); a non-US console keymap's text. These skip
//!   (`kmsdrm`) on a vkms card, and without the hardware they look at.
//!
//! Elsewhere than Linux there is nothing.

#![cfg(target_os = "linux")]

#[path = "kmsdrm_support/mod.rs"]
mod kmsdrm_support;

use std::time::{Duration, Instant};

use kmsdrm_support::scene::{differing_from_software, draw_frame, fnv, pattern, rgba};
use kmsdrm_support::vk::Vk;
use kmsdrm_support::{gl_clear, request, run_hardware_check};
use sdl3::events::window::WindowFlags;
use sdl3::events::{self, Event};
use sdl3::hints;
use sdl3::init::{self, InitFlags};
use sdl3::render::Renderer;
use sdl3::video::gl::{self, GlContext};
use sdl3::video::{self, vulkan, FColor, PixelFormat, Surface, Window};

/// The test's skip, under the runner (it can't skip for the parent: it
/// fails if the capability is required, and passes otherwise).
fn skip(test: &str, capability: &str, reason: &str) {
    let _ = kmsdrm_support::skip(test, capability, reason);
}

fn init_kmsdrm() {
    init::init(InitFlags::VIDEO).expect("SDL_Init(SDL_INIT_VIDEO)");
    assert_eq!(video::current_video_driver().ok(), Some("kmsdrm"));
}

#[test]
#[ignore = "hardware: a GPU's Vulkan driver with VK_KHR_display on a KMS/DRM card, as root on tty3"]
fn hardware_vulkan_surface_swapchain_and_renderers() {
    const TEST: &str = "hardware_vulkan_surface_swapchain_and_renderers";
    if let Some(status) = run_hardware_check(TEST) {
        assert_eq!(status, 0, "{TEST} as root on tty3 failed (above)");
        return;
    }
    init_kmsdrm();
    let display = video::primary_display().unwrap();
    let desktop = video::desktop_display_mode(display).unwrap();
    println!(
        "display {:?}: {}x{}@{:.2}",
        video::display_name(display).unwrap_or_default(),
        desktop.w,
        desktop.h,
        desktop.refresh_rate
    );
    if let Err(e) = vulkan::vulkan_load_library(None) {
        skip(
            TEST,
            "vulkan",
            &format!("no Vulkan loader with VK_KHR_display ({e})"),
        );
        init::quit();
        return;
    }
    let extensions = vulkan::vulkan_instance_extensions().unwrap();
    assert_eq!(extensions, ["VK_KHR_surface", "VK_KHR_display"]);
    let vk = Vk::new();
    let instance = vk.create_instance(&extensions).expect("vkCreateInstance");
    let devices = vk.physical_devices(instance);
    let mut gpu = None;
    for (i, &d) in devices.iter().enumerate() {
        let props = vk.device_properties(instance, d);
        let counts = vk.display_and_plane_counts(instance, d);
        println!(
            "physical device {i}: {} (type {}, Vulkan {}.{}), displays and planes {counts:?}",
            props.name, props.device_type, props.api_major, props.api_minor
        );
        // (upstream's choice: the first integrated or discrete GPU)
        if gpu.is_none() && (props.device_type == 1 || props.device_type == 2) {
            gpu = Some(d);
        }
    }
    let Some(gpu) = gpu else {
        skip(
            TEST,
            "vulkan",
            "no integrated or discrete GPU among the Vulkan devices",
        );
        vk.destroy_instance(instance);
        vulkan::vulkan_unload_library();
        init::quit();
        return;
    };

    // The surface on a display plane, the window's size a mode of the
    // display's
    let window = Window::create("kmsdrm_hardware", desktop.w, desktop.h, WindowFlags::VULKAN)
        .expect("a Vulkan window");
    assert!(window.flags().unwrap().contains(WindowFlags::VULKAN));
    assert!(!window.flags().unwrap().contains(WindowFlags::OPENGL));
    let surface = vulkan::vulkan_create_surface(&window, instance as usize, 0)
        .expect("a display plane surface");
    let (result, caps) = vk.surface_capabilities(instance, gpu, surface);
    println!("surface: capabilities ({result}): {caps:?}");
    assert_eq!(result, 0);
    assert_eq!(
        caps.current_extent,
        [desktop.w as u32, desktop.h as u32],
        "the surface's extent is the window's"
    );
    vulkan::vulkan_destroy_surface(instance as usize, surface, 0);
    vk.destroy_instance(instance);

    // The GPU API's swapchain on it
    {
        use sdl3::gpu::{ColorTargetInfo, Device, LoadOp, ShaderFormat};
        let device = Device::new(ShaderFormat::SPIRV, false, Some("vulkan")).expect("a GPU device");
        device
            .claim_window(&window)
            .expect("the window claimed (a swapchain)");
        println!(
            "gpu: swapchain format {:?}",
            device.swapchain_texture_format(&window)
        );
        for (i, color) in [
            FColor::new(1.0, 0.0, 0.0, 1.0),
            FColor::new(0.0, 1.0, 0.0, 1.0),
            FColor::new(0.0, 0.0, 1.0, 1.0),
        ]
        .into_iter()
        .enumerate()
        {
            let mut cb = device.acquire_command_buffer().unwrap();
            let texture = cb
                .wait_and_acquire_swapchain_texture(&window)
                .expect("a swapchain texture")
                .expect("a swapchain texture (not minimized)");
            assert_eq!(
                (texture.width, texture.height),
                (desktop.w as u32, desktop.h as u32)
            );
            let target = ColorTargetInfo {
                load_op: LoadOp::Clear,
                clear_color: color,
                ..ColorTargetInfo::new(&texture.texture)
            };
            cb.begin_render_pass(&[target], None).unwrap().end();
            cb.submit().unwrap();
            println!("gpu: frame {i} presented");
        }
        device.wait_for_idle().unwrap();
        device.release_window(&window);
    }

    // The Vulkan and GPU renderers, against the software renderer
    hints::set(hints::GPU_DRIVER, "vulkan").unwrap();
    for name in ["vulkan", "gpu"] {
        let mut r = Renderer::for_window(&window, Some(name))
            .unwrap_or_else(|e| panic!("the {name} renderer: {e}"));
        assert_eq!(r.name(), name);
        let texture = r.create_texture_from_surface(&mut pattern()).unwrap();
        for frame in 0..3 {
            draw_frame(&mut r, frame, Some(texture));
            let image = rgba(&r.read_pixels(None).unwrap());
            let bad = differing_from_software(&image, frame);
            println!(
                "{name} frame {frame}: {}x{} {:08x}, {bad:?} pixels differ from the software renderer's",
                image.0,
                image.1,
                fnv(&image.2)
            );
            assert_eq!(bad, Some(0), "{name} frame {frame}");
            r.present().unwrap();
        }
    }
    hints::reset(hints::GPU_DRIVER);
    window.destroy();
    vulkan::vulkan_unload_library();
    init::quit();
}

/// Whether the card the checks run on (`SDL_KMSDRM_DEVICE_INDEX`'s, which
/// `run_hardware_check()` sets) is a vkms one, a faux device (the checks
/// want a GPU's driver and real connectors).
fn on_vkms() -> bool {
    let card = std::env::var("SDL_KMSDRM_DEVICE_INDEX").unwrap_or_default();
    std::fs::canonicalize(format!("/sys/class/drm/card{card}/device"))
        .is_ok_and(|p| p.starts_with("/sys/devices/faux"))
}

/// Under the runner: skip on a vkms card (`true`: skipped).
fn skip_on_vkms(test: &str) -> bool {
    if on_vkms() {
        skip(
            test,
            "kmsdrm",
            "the card is vkms's (a GPU's driver and a real display are wanted)",
        );
        return true;
    }
    false
}

/// The runner's `drm info` lines.
fn drm_info() -> Vec<String> {
    let lines = request("drm info");
    for line in &lines {
        println!("{line}");
    }
    lines
}

/// The value of a `drm: cap NAME V` line.
fn cap(info: &[String], name: &str) -> Option<u64> {
    info.iter()
        .find_map(|l| l.strip_prefix(&format!("drm: cap {name} ")))
        .and_then(|v| v.parse().ok())
}

/// The `drm: connector NAME ...` line of a connector.
fn connector<'a>(info: &'a [String], name: &str) -> Option<&'a str> {
    info.iter()
        .find_map(|l| l.strip_prefix(&format!("drm: connector {name} ")))
}

/// A field of a connector line (`vrr_capable V`, `panel orientation V`).
fn field<'a>(line: &'a str, name: &str) -> Option<&'a str> {
    let at = line.find(&format!("{name} "))? + name.len() + 1;
    line[at..].split(',').next()
}

/// A window of the primary display's desktop mode, with a context.
fn fullscreen_window(title: &str) -> (Window, GlContext) {
    let display = video::primary_display().unwrap();
    let desktop = video::desktop_display_mode(display).unwrap();
    let window =
        Window::create(title, desktop.w, desktop.h, WindowFlags::default()).expect("a window");
    let context = GlContext::new(&window).expect("a context");
    (window, context)
}

/// The times between `count` swaps of frames of one color (the first
/// ten, of the mode set, left out).
fn swap_intervals(window: &Window, count: usize) -> Vec<f64> {
    let mut times = Vec::new();
    for frame in 0..count + 10 {
        let shade = (frame % 16) as f32 / 16.0;
        gl_clear(shade, 0.0, 1.0 - shade);
        gl::gl_swap_window(window).expect("a swap");
        events::pump();
        times.push(Instant::now());
    }
    times
        .windows(2)
        .skip(10)
        .map(|w| (w[1] - w[0]).as_secs_f64() * 1000.0)
        .collect()
}

fn median(values: &[f64]) -> f64 {
    let mut v = values.to_vec();
    v.sort_by(f64::total_cmp);
    v[v.len() / 2]
}

#[test]
#[ignore = "hardware: a GPU's KMS driver and a display, as root on tty3"]
fn hardware_swap_paths_and_page_flip_timing() {
    const TEST: &str = "hardware_swap_paths_and_page_flip_timing";
    if let Some(status) = run_hardware_check(TEST) {
        assert_eq!(status, 0, "{TEST} as root on tty3 failed (above)");
        return;
    }
    if skip_on_vkms(TEST) {
        return;
    }
    for (label, settings) in [
        ("fenced (atomic)", &[][..]),
        (
            "double buffered (atomic)",
            &[(hints::VIDEO_DOUBLE_BUFFER, "1")][..],
        ),
        ("legacy", &[(hints::KMSDRM_ATOMIC, "0")][..]),
        (
            "legacy, double buffered",
            &[
                (hints::KMSDRM_ATOMIC, "0"),
                (hints::VIDEO_DOUBLE_BUFFER, "1"),
            ][..],
        ),
    ] {
        for (name, value) in settings {
            hints::set(name, value).unwrap();
        }
        init_kmsdrm();
        let refresh = video::desktop_display_mode(video::primary_display().unwrap())
            .unwrap()
            .refresh_rate as f64;
        let period = 1000.0 / refresh;
        let (window, context) = fullscreen_window("kmsdrm_hardware");
        for interval in [1, 0] {
            gl::gl_set_swap_interval(interval).unwrap();
            let intervals = swap_intervals(&window, 120);
            let m = median(&intervals);
            println!(
                "{label}, swap interval {interval}: median {m:.2} ms between swaps ({refresh:.2} Hz: {period:.2} ms), min {:.2} max {:.2}",
                intervals.iter().copied().fold(f64::INFINITY, f64::min),
                intervals.iter().copied().fold(0.0, f64::max)
            );
            if interval == 1 {
                // (vsync: a flip a refresh)
                assert!(
                    (period * 0.8..period * 1.25).contains(&m),
                    "{label}: swaps at interval 1 {m:.2} ms apart, not a refresh's {period:.2} ms"
                );
            }
        }
        drop(context);
        window.destroy();
        init::quit();
        for (name, _) in settings {
            hints::reset(name);
        }
    }
}

#[test]
#[ignore = "hardware: a GPU's KMS driver with DRM_CAP_ASYNC_PAGE_FLIP, as root on tty3"]
fn hardware_async_page_flips() {
    const TEST: &str = "hardware_async_page_flips";
    if let Some(status) = run_hardware_check(TEST) {
        assert_eq!(status, 0, "{TEST} as root on tty3 failed (above)");
        return;
    }
    if skip_on_vkms(TEST) {
        return;
    }
    let info = drm_info();
    if cap(&info, "ASYNC_PAGE_FLIP") != Some(1) {
        skip(TEST, "kmsdrm", "no DRM_CAP_ASYNC_PAGE_FLIP");
        return;
    }
    // (the legacy page flips at swap interval 0 are async ones,
    // DRM_MODE_PAGE_FLIP_ASYNC: upstream's)
    hints::set(hints::KMSDRM_ATOMIC, "0").unwrap();
    init_kmsdrm();
    let refresh = video::desktop_display_mode(video::primary_display().unwrap())
        .unwrap()
        .refresh_rate as f64;
    let (window, context) = fullscreen_window("kmsdrm_hardware");
    gl::gl_set_swap_interval(0).unwrap();
    let intervals = swap_intervals(&window, 240);
    let m = median(&intervals);
    println!(
        "async page flips: median {m:.2} ms between swaps ({refresh:.2} Hz: {:.2} ms)",
        1000.0 / refresh
    );
    drop(context);
    window.destroy();
    init::quit();
    hints::reset(hints::KMSDRM_ATOMIC);
}

#[test]
#[ignore = "hardware: a GPU's KMS driver with framebuffer modifiers, as root on tty3"]
fn hardware_framebuffer_modifiers() {
    const TEST: &str = "hardware_framebuffer_modifiers";
    if let Some(status) = run_hardware_check(TEST) {
        assert_eq!(status, 0, "{TEST} as root on tty3 failed (above)");
        return;
    }
    if skip_on_vkms(TEST) {
        return;
    }
    init_kmsdrm();
    let (window, context) = fullscreen_window("kmsdrm_hardware");
    for frame in 0..3 {
        gl_clear(frame as f32 / 2.0, 0.5, 0.0);
        gl::gl_swap_window(&window).unwrap();
    }
    let info = drm_info();
    let primary = info
        .iter()
        .find(|l| l.starts_with("drm: plane primary "))
        .expect("the primary plane scans out the window");
    let (w, h) = window.size_in_pixels().unwrap();
    assert!(
        primary.starts_with(&format!("drm: plane primary fb {w}x{h} ")),
        "{primary}: not the window's framebuffer"
    );
    let modifier = primary.rsplit(' ').next().unwrap();
    println!(
        "the window's framebuffer: modifier {modifier} (ADDFB2_MODIFIERS {:?})",
        cap(&info, "ADDFB2_MODIFIERS")
    );
    drop(context);
    window.destroy();
    init::quit();
    if modifier == "none" || modifier == "0x0" {
        skip(
            TEST,
            "kmsdrm",
            &format!("the GPU's scanout buffers are linear (modifier {modifier})"),
        );
    }
}

#[test]
#[ignore = "hardware: a GPU's KMS driver and its cursor plane, as root on tty3"]
fn hardware_cursor_size() {
    const TEST: &str = "hardware_cursor_size";
    if let Some(status) = run_hardware_check(TEST) {
        assert_eq!(status, 0, "{TEST} as root on tty3 failed (above)");
        return;
    }
    if skip_on_vkms(TEST) {
        return;
    }
    let info = drm_info();
    let (cw, ch) = (
        cap(&info, "CURSOR_WIDTH").unwrap_or(64),
        cap(&info, "CURSOR_HEIGHT").unwrap_or(64),
    );
    init_kmsdrm();
    let (window, context) = fullscreen_window("kmsdrm_hardware");
    gl_clear(0.0, 0.0, 0.5);
    gl::gl_swap_window(&window).unwrap();
    let mut surface = Surface::new(32, 32, PixelFormat::ARGB8888).unwrap();
    let red = surface.map_rgba(255, 0, 0, 255);
    surface.fill_rect(None, red).unwrap();
    let cursor = sdl3::events::mouse::create_color_cursor(&surface, 4, 4).expect("a cursor");
    sdl3::events::mouse::set_cursor(Some(&cursor)).unwrap();
    sdl3::events::mouse::show_cursor();
    gl::gl_swap_window(&window).unwrap();
    let state = request("drm state");
    for line in &state {
        println!("{line}");
    }
    let cursor_plane = state.iter().find(|l| l.starts_with("drm: plane cursor "));
    println!("DRM_CAP_CURSOR_WIDTH/HEIGHT {cw}x{ch}, the cursor plane: {cursor_plane:?}");
    if let Some(line) = cursor_plane {
        // (upstream's cursor buffer object is the capability's size)
        assert!(
            line.starts_with(&format!("drm: plane cursor fb {cw}x{ch} ")),
            "{line}: not a {cw}x{ch} buffer"
        );
    }
    sdl3::events::mouse::destroy_cursor(cursor);
    drop(context);
    window.destroy();
    init::quit();
}

#[test]
#[ignore = "hardware: a GPU's KMS driver and real connectors, as root on tty3"]
fn hardware_connector_modes() {
    const TEST: &str = "hardware_connector_modes";
    if let Some(status) = run_hardware_check(TEST) {
        assert_eq!(status, 0, "{TEST} as root on tty3 failed (above)");
        return;
    }
    if skip_on_vkms(TEST) {
        return;
    }
    let info = drm_info();
    init_kmsdrm();
    for id in video::displays().unwrap() {
        let name = video::display_name(id).unwrap_or_default();
        let desktop = video::desktop_display_mode(id).unwrap();
        let modes = video::fullscreen_display_modes(id).unwrap();
        println!(
            "display {name:?}: desktop {}x{}@{:.2}, {} fullscreen modes",
            desktop.w,
            desktop.h,
            desktop.refresh_rate,
            modes.len()
        );
        for m in &modes {
            println!("  {}x{}@{:.2}", m.w, m.h, m.refresh_rate);
        }
        // (a display per connected connector, named as it)
        let line = connector(&info, &name)
            .unwrap_or_else(|| panic!("display {name:?}: no such connector"));
        assert!(line.starts_with("connected,"), "{name}: {line}");
        let count: usize = line
            .split(", ")
            .nth(1)
            .and_then(|f| f.strip_suffix(" modes"))
            .and_then(|n| n.parse().ok())
            .unwrap();
        assert!(
            !modes.is_empty() && modes.len() <= count,
            "{name}: {} modes, the connector's {count}",
            modes.len()
        );
        assert!(modes.iter().any(|m| (m.w, m.h) == (desktop.w, desktop.h)));
    }
    init::quit();
}

#[test]
#[ignore = "hardware: a VRR-capable display (vrr_capable), as root on tty3"]
fn hardware_vrr() {
    const TEST: &str = "hardware_vrr";
    if let Some(status) = run_hardware_check(TEST) {
        assert_eq!(status, 0, "{TEST} as root on tty3 failed (above)");
        return;
    }
    if skip_on_vkms(TEST) {
        return;
    }
    let before = drm_info();
    let capable = before
        .iter()
        .filter_map(|l| l.strip_prefix("drm: connector "))
        .any(|l| l.contains(" connected,") && field(l, "vrr_capable") == Some("1"));
    if !capable {
        skip(TEST, "kmsdrm", "no connected connector is vrr_capable");
        return;
    }
    let enabled = |info: &[String]| {
        info.iter()
            .find_map(|l| l.strip_prefix("drm: crtc VRR_ENABLED "))
            .map(str::to_owned)
    };
    let saved = enabled(&before);
    // (upstream enables VRR on the display's CRTC as it adds the display,
    // and restores what it was as a window is destroyed)
    init_kmsdrm();
    let (window, context) = fullscreen_window("kmsdrm_hardware");
    gl_clear(0.0, 0.5, 0.0);
    gl::gl_swap_window(&window).unwrap();
    let during = drm_info();
    assert_eq!(
        enabled(&during).as_deref(),
        Some("1"),
        "VRR_ENABLED with the window"
    );
    drop(context);
    window.destroy();
    let after = drm_info();
    assert_eq!(
        enabled(&after),
        saved,
        "VRR_ENABLED restored as the window went"
    );
    init::quit();
}

#[test]
#[ignore = "hardware: a panel with a panel orientation property, as root on tty3"]
fn hardware_panel_orientation() {
    const TEST: &str = "hardware_panel_orientation";
    if let Some(status) = run_hardware_check(TEST) {
        assert_eq!(status, 0, "{TEST} as root on tty3 failed (above)");
        return;
    }
    if skip_on_vkms(TEST) {
        return;
    }
    let info = drm_info();
    init_kmsdrm();
    let mut found = false;
    for id in video::displays().unwrap() {
        let name = video::display_name(id).unwrap_or_default();
        let Some(orientation) = connector(&info, &name).and_then(|l| field(l, "panel orientation"))
        else {
            continue;
        };
        if orientation == "none" {
            continue;
        }
        found = true;
        let sdl = video::display_properties(id)
            .ok()
            .and_then(|p| p.get_number("SDL.display.KMSDRM.panel_orientation"));
        // FIXME (upstream): the property is looked up on the CRTC's ID, so
        // SDL's is always 0; the connector's value (0 normal, 1 upside
        // down, 2 left side up, 3 right side up) would be 0, 180, 90, 270
        println!(
            "display {name:?}: the connector's panel orientation {orientation}, SDL's {sdl:?}"
        );
        assert_eq!(sdl, Some(0), "upstream's lookup on the CRTC finds none");
    }
    init::quit();
    if !found {
        skip(
            TEST,
            "kmsdrm",
            "no connector has a panel orientation property",
        );
    }
}

#[test]
#[ignore = "hardware: a non-US console keymap on tty3 (loadkeys), uinput, as root"]
fn hardware_console_keymap() {
    const TEST: &str = "hardware_console_keymap";
    if let Some(status) = run_hardware_check(TEST) {
        assert_eq!(status, 0, "{TEST} as root on tty3 failed (above)");
        return;
    }
    // The console's plain keymap entries of a few keys (KDGKBENT on tty3)
    let keys = [
        ("KEY_Q", 16u8),
        ("KEY_W", 17),
        ("KEY_Y", 21),
        ("KEY_Z", 44),
        ("KEY_A", 30),
        ("KEY_M", 50),
        ("KEY_SEMICOLON", 39),
    ];
    let us = ['q', 'w', 'y', 'z', 'a', 'm', ';'];
    let tty = std::fs::File::open("/dev/tty").expect("the console");
    let mut expected = Vec::new();
    for &(_, code) in &keys {
        #[repr(C)]
        struct KbEntry {
            table: u8,
            index: u8,
            value: u16,
        }
        let mut e = KbEntry {
            table: 0,
            index: code,
            value: 0,
        };
        // SAFETY: KDGKBENT (0x4B46) fills in the entry.
        let r = unsafe { libc::ioctl(std::os::fd::AsRawFd::as_raw_fd(&tty), 0x4B46, &mut e) };
        assert_eq!(r, 0, "KDGKBENT");
        // (KT_LATIN and KT_LETTER: the character; Unicode keysyms ^ 0xf000)
        let value = e.value;
        let c = match value >> 8 {
            0x00 | 0x0b => char::from_u32((value & 0xff) as u32),
            0xf0..=0xff => char::from_u32((value ^ 0xf000) as u32),
            _ => None,
        };
        expected.push(c);
    }
    println!("the console's keymap: {expected:?}");
    if expected.iter().zip(us).all(|(c, u)| *c == Some(u)) {
        skip(
            TEST,
            "vt",
            "the console keymap is US's for these keys (loadkeys a non-US one, de or fr, first)",
        );
        return;
    }
    init_kmsdrm();
    let (window, context) = fullscreen_window("kmsdrm_hardware");
    let _ = window.start_text_input();
    const KEYBOARD: &str = "SDL3 kmsdrm hardware keyboard";
    for reply in request(&format!("uinput add '{KEYBOARD}' keyboard")) {
        assert!(reply.starts_with("uinput: added"), "{reply}");
    }
    // (the reader's keyboard added)
    let end = Instant::now() + Duration::from_secs(10);
    let mut added = false;
    while !added && Instant::now() < end {
        events::pump();
        while let Some(e) = events::poll() {
            if let Event::KeyboardDevice(d) = e {
                added |=
                    sdl3::events::keyboard::keyboard_name(d.which).is_ok_and(|n| n == KEYBOARD);
            }
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(added, "the reader didn't add the keyboard");
    for (i, &(key, _)) in keys.iter().enumerate() {
        request(&format!(
            "uinput events '{KEYBOARD}' EV_KEY {key} 1 EV_SYN SYN_REPORT 0 EV_KEY {key} 0 EV_SYN SYN_REPORT 0"
        ));
        let mut text = None;
        let end = Instant::now() + Duration::from_secs(2);
        while text.is_none() && Instant::now() < end {
            events::pump();
            while let Some(e) = events::poll() {
                if let Event::TextInput(t) = e {
                    text = Some(t.text.clone());
                }
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        println!("{key}: text {text:?}, the keymap's {:?}", expected[i]);
        if let Some(c) = expected[i].filter(|c| !c.is_control()) {
            assert_eq!(text.as_deref(), Some(c.to_string().as_str()), "{key}");
        }
    }
    request(&format!("uinput remove '{KEYBOARD}'"));
    request("vt flush");
    drop(context);
    window.destroy();
    init::quit();
}
