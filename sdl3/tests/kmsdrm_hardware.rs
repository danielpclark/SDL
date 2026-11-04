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
//!
//! Elsewhere than Linux there is nothing.

#![cfg(target_os = "linux")]

#[path = "kmsdrm_support/mod.rs"]
mod kmsdrm_support;

use kmsdrm_support::run_hardware_check;
use kmsdrm_support::scene::{differing_from_software, draw_frame, fnv, pattern, rgba};
use kmsdrm_support::vk::Vk;
use sdl3::events::window::WindowFlags;
use sdl3::hints;
use sdl3::init::{self, InitFlags};
use sdl3::render::Renderer;
use sdl3::video::{self, vulkan, FColor, Window};

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
