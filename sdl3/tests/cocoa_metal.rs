// Tests of the Metal renderer and of Metal on Cocoa windows.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! The Metal renderer draws into a Cocoa window's Metal view, and the
//! Cocoa video driver only starts on the main thread (as upstream's
//! `Cocoa_CreateDevice()`), so this is a program of its own
//! (`harness = false`) whose `main` drives them through the public API:
//!
//! * the "metal" 2D renderer against the software renderer (as the other
//!   renderers' tests do): shapes, lines, viewports and clipping, blend
//!   modes (custom ones too), textures of each format it takes (RGB, packed
//!   16-bit, 10-bit and float, palettes, YUV and NV12), scale and address
//!   modes, streaming textures through locks, render targets, debug text,
//!   partial reads, textures wrapping the
//!   application's `MTLTexture`s, vsync, its Metal layer, command encoder
//!   and properties, and a hidden and a resized window;
//! * the GPU API's Metal swapchains on a window: claiming and releasing,
//!   compositions and present modes, frames acquired, cleared and
//!   presented, frames in flight, parameters, and a resize;
//! * the "gpu" 2D renderer on such a window against the software renderer,
//!   on its own device and on the application's, with textures wrapping
//!   GPU textures.
//!
//! Without a window server session (capability `cocoa`) or a Metal device
//! (capability `metal`) it skips, unless `SDL3_TEST_REQUIRE` asks for it.

#[cfg(not(target_os = "macos"))]
fn main() {}

#[cfg(target_os = "macos")]
fn main() {
    macos::main();
}

#[cfg(target_os = "macos")]
mod macos {
    use std::ffi::{c_char, c_void, CStr};
    use std::time::{Duration, Instant};

    use sdl3::events::window::WindowFlags;
    use sdl3::events::Event;
    use sdl3::hints;
    use sdl3::init::{self, InitFlags};
    use sdl3::render::{
        NativeObject, Renderer, RendererCreateInfo, TextureAccess, TextureAddressMode,
        TextureCreateInfo, Vertex,
    };
    use sdl3::video::{
        self, BlendFactor, BlendMode, BlendOperation, Color, FColor, FPoint, FRect, PixelFormat,
        Rect, ScaleMode, Surface, Window,
    };

    /// Whether `SDL3_TEST_REQUIRE` makes a capability mandatory.
    fn required(capability: &str) -> bool {
        std::env::var("SDL3_TEST_REQUIRE")
            .unwrap_or_default()
            .split(',')
            .map(str::trim)
            .any(|c| c == "all" || c == capability)
    }

    fn skip(capability: &str, reason: &str) {
        if required(capability) {
            panic!("required capability {capability} unavailable: {reason}");
        }
        println!("note: skipping, {capability} unavailable: {reason}");
    }

    /// Pump events for a while.
    fn pump_for(ms: u64) -> Vec<Event> {
        let end = Instant::now() + Duration::from_millis(ms);
        let mut events = Vec::new();
        while Instant::now() < end {
            while let Some(e) = sdl3::events::poll() {
                events.push(e);
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        events
    }

    pub(super) fn main() {
        // A hung frame shouldn't hang the test run.
        std::thread::spawn(|| {
            std::thread::sleep(Duration::from_secs(240));
            eprintln!("cocoa_metal test timed out");
            std::process::exit(1);
        });

        hints::set(hints::VIDEO_DRIVER, "cocoa").unwrap();
        if let Err(e) = init::init(InitFlags::VIDEO) {
            skip("cocoa", e.message());
            return;
        }
        assert_eq!(video::current_video_driver().unwrap(), "cocoa");

        if metal_renderer() {
            metal_renderer_window_states();
            gpu_swapchains();
            gpu_renderer_on_a_window();
        }

        init::quit();
        println!("cocoa_metal: all checks passed");
    }

    // ------------------------------------------------------------------
    // Comparing with the software renderer

    const RW: i32 = 64;
    const RH: i32 = 48;

    /// The pixels of a surface as tightly packed RGBA32.
    fn rgba(surface: &Surface<'_>) -> (i32, i32, Vec<u8>) {
        let s = surface.convert(PixelFormat::RGBA32).unwrap();
        let (w, h, pitch) = (s.width(), s.height(), s.pitch() as usize);
        let pixels = s.pixels().unwrap();
        let mut out = Vec::new();
        for y in 0..h as usize {
            out.extend_from_slice(&pixels[y * pitch..y * pitch + w as usize * 4]);
        }
        (w, h, out)
    }

    /// Compare two images: at most `max_bad` pixels may have a color
    /// channel off by more than `tolerance`.
    fn compare(
        what: &str,
        a: &(i32, i32, Vec<u8>),
        b: &(i32, i32, Vec<u8>),
        tolerance: u8,
        max_bad: usize,
    ) {
        assert_eq!((a.0, a.1), (b.0, b.1), "{what}: sizes");
        let mut bad = 0;
        let mut first = None;
        for (i, (pa, pb)) in a.2.chunks(4).zip(b.2.chunks(4)).enumerate() {
            if (0..3).any(|c| pa[c].abs_diff(pb[c]) > tolerance) {
                bad += 1;
                if first.is_none() {
                    first = Some((i as i32 % a.0, i as i32 / a.0, pa.to_vec(), pb.to_vec()));
                }
            }
        }
        assert!(
            bad <= max_bad,
            "{what}: {bad} pixels differ (first at {first:?}: renderer vs software)"
        );
    }

    /// Draw a scene with the window's renderer and a software renderer and
    /// compare the results.
    fn compare_scene(
        r: &mut Renderer,
        what: &str,
        tolerance: u8,
        max_bad: usize,
        scene: impl Fn(&mut Renderer) -> sdl3::error::Result<()>,
    ) {
        let name = r.name();
        scene(r).unwrap_or_else(|e| panic!("{what}: {name}: {}", e.message()));
        let a = rgba(&r.read_pixels(None).unwrap());
        r.present().unwrap();
        let mut sw =
            Renderer::software(Surface::new(RW, RH, PixelFormat::ARGB8888).unwrap()).unwrap();
        scene(&mut sw).unwrap_or_else(|e| panic!("{what}: software: {}", e.message()));
        let b = rgba(&sw.read_pixels(None).unwrap());
        compare(&format!("{name}: {what}"), &a, &b, tolerance, max_bad);
    }

    /// A 16x16 pattern with varying alpha.
    fn pattern_color(x: i32, y: i32) -> Color {
        Color::new(
            (x * 16) as u8,
            (y * 16) as u8,
            ((x + y) * 8) as u8,
            if (x + y) % 3 == 0 {
                255
            } else {
                (x * 8 + y * 7) as u8
            },
        )
    }

    /// A 16x16 surface of the pattern.
    fn pattern_surface(format: PixelFormat) -> Surface<'static> {
        let mut s = Surface::new(16, 16, format).unwrap();
        for y in 0..16 {
            for x in 0..16 {
                s.write_pixel(x, y, pattern_color(x, y)).unwrap();
            }
        }
        s
    }

    /// The scenes every hardware renderer is checked with.
    fn common_scenes(r: &mut Renderer) {
        compare_scene(r, "clear and fill", 0, 0, |r| {
            r.set_draw_color(10, 20, 30, 255);
            r.clear()?;
            r.set_draw_color(200, 100, 50, 255);
            r.render_fill_rect(Some(&FRect::new(8.0, 8.0, 32.0, 24.0)))?;
            r.set_draw_color(0, 255, 0, 255);
            r.render_fill_rects(&[
                FRect::new(-5.0, -5.0, 10.0, 10.0),
                FRect::new(50.0, 40.0, 30.0, 30.0),
            ])?;
            r.set_draw_color(255, 255, 255, 255);
            r.render_rect(Some(&FRect::new(2.0, 30.0, 12.0, 10.0)))
        });
        compare_scene(r, "points and lines", 0, 4, |r| {
            r.set_draw_color(0, 0, 0, 255);
            r.clear()?;
            r.set_draw_color(255, 128, 0, 255);
            let points: Vec<FPoint> = (0..40)
                .map(|i| FPoint {
                    x: (i * 7 % 64) as f32,
                    y: (i * 5 % 48) as f32,
                })
                .collect();
            r.render_points(&points)?;
            r.set_draw_color(255, 255, 0, 255);
            r.render_line(0.0, 0.0, 63.0, 0.0)?;
            r.render_line(5.0, 3.0, 5.0, 40.0)?;
            r.render_line(10.0, 10.0, 40.0, 30.0)?;
            r.render_lines(&[
                FPoint { x: 50.0, y: 5.0 },
                FPoint { x: 60.0, y: 5.0 },
                FPoint { x: 60.0, y: 15.0 },
            ])
        });
        compare_scene(r, "viewport and clip", 0, 0, |r| {
            r.set_draw_color(0, 0, 0, 255);
            r.clear()?;
            r.set_viewport(Some(&Rect::new(10, 6, 40, 30)))?;
            r.set_draw_color(255, 0, 0, 255);
            r.render_fill_rect(None)?;
            r.set_clip_rect(Some(&Rect::new(5, 5, 12, 8)))?;
            r.set_draw_color(0, 0, 255, 255);
            r.render_fill_rect(Some(&FRect::new(0.0, 0.0, 30.0, 30.0)))?;
            r.set_clip_rect(None)?;
            r.set_viewport(None)
        });
        compare_scene(r, "blend modes", 2, 0, |r| {
            r.set_draw_blend_mode(BlendMode::NONE)?;
            r.set_draw_color(10, 20, 30, 255);
            r.clear()?;
            for (i, mode) in [
                BlendMode::BLEND,
                BlendMode::ADD,
                BlendMode::MOD,
                BlendMode::MUL,
            ]
            .into_iter()
            .enumerate()
            {
                r.set_draw_blend_mode(mode)?;
                r.set_draw_color(220, 90, 40, 128);
                r.render_fill_rect(Some(&FRect::new(i as f32 * 12.0, 12.0, 10.0, 30.0)))?;
            }
            Ok(())
        });
        for format in [
            PixelFormat::ARGB8888,
            PixelFormat::ABGR8888,
            PixelFormat::RGB24,
            PixelFormat::XRGB8888,
        ] {
            compare_scene(r, &format!("texture {format:?}"), 2, 0, |r| {
                r.set_draw_blend_mode(BlendMode::NONE)?;
                r.set_draw_color(40, 50, 60, 255);
                r.clear()?;
                let tex = r.create_texture_from_surface(&mut pattern_surface(format))?;
                r.set_texture_scale_mode(tex, ScaleMode::Nearest)?;
                r.render_texture(tex, None, Some(&FRect::new(4.0, 4.0, 16.0, 16.0)))?;
                r.set_texture_color_mod(tex, 255, 128, 64)?;
                r.set_texture_alpha_mod(tex, 160)?;
                r.render_texture(tex, None, Some(&FRect::new(24.0, 8.0, 32.0, 32.0)))?;
                r.destroy_texture(tex);
                Ok(())
            });
        }
        // A streaming texture, updated twice
        compare_scene(r, "streaming texture", 0, 0, |r| {
            r.set_draw_color(0, 0, 0, 255);
            r.clear()?;
            let tex = r.create_texture(PixelFormat::ARGB8888, TextureAccess::Streaming, 16, 8)?;
            r.set_texture_scale_mode(tex, ScaleMode::Nearest)?;
            let mut pixels = vec![0u8; 16 * 8 * 4];
            for (i, p) in pixels.chunks_mut(4).enumerate() {
                p.copy_from_slice(&[(i * 3) as u8, (i * 5) as u8, 200, 255]);
            }
            r.update_texture(tex, None, &pixels, 64)?;
            r.render_texture(tex, None, Some(&FRect::new(0.0, 0.0, 16.0, 8.0)))?;
            for p in pixels.chunks_mut(4) {
                p[..3].iter_mut().for_each(|b| *b = b.wrapping_add(40));
            }
            r.update_texture(tex, None, &pixels, 64)?;
            r.render_texture(tex, None, Some(&FRect::new(20.0, 20.0, 16.0, 8.0)))?;
            r.destroy_texture(tex);
            Ok(())
        });
        // Render targets
        compare_scene(r, "render target", 0, 0, |r| {
            let target = r.create_texture(PixelFormat::ARGB8888, TextureAccess::Target, 32, 24)?;
            r.set_render_target(Some(target))?;
            r.set_draw_color(255, 0, 255, 255);
            r.clear()?;
            r.set_draw_color(0, 255, 0, 255);
            r.render_fill_rect(Some(&FRect::new(4.0, 4.0, 8.0, 8.0)))?;
            r.set_render_target(None)?;
            r.set_draw_color(0, 0, 0, 255);
            r.clear()?;
            r.set_texture_scale_mode(target, ScaleMode::Nearest)?;
            r.render_texture(target, None, Some(&FRect::new(8.0, 8.0, 32.0, 24.0)))?;
            r.destroy_texture(target);
            Ok(())
        });
    }

    /// YUV planes of the pattern (`IYUV`, `YV12`, `NV12`, `NV21`), from an
    /// RGB image of `w`x`h` through SDL's conversion.
    fn yuv_pixels(format: PixelFormat, w: i32, h: i32) -> Vec<u8> {
        let mut rgb = Surface::new(w, h, PixelFormat::XRGB8888).unwrap();
        for y in 0..h {
            for x in 0..w {
                rgb.write_pixel(x, y, pattern_color(x % 16, y % 16))
                    .unwrap();
            }
        }
        let size = (w * h + 2 * ((w + 1) / 2) * ((h + 1) / 2)) as usize;
        let mut out = vec![0u8; size];
        video::convert_pixels(
            w,
            h,
            PixelFormat::XRGB8888,
            rgb.pixels().unwrap(),
            rgb.pitch(),
            format,
            &mut out,
            w,
        )
        .unwrap();
        out
    }

    // ------------------------------------------------------------------
    // The bits of the Objective-C runtime the MTLTexture check needs.

    unsafe extern "C" {
        fn objc_getClass(name: *const c_char) -> *mut c_void;
        fn sel_registerName(name: *const c_char) -> *mut c_void;
        fn objc_msgSend();
        fn objc_release(obj: *mut c_void);
    }

    fn sel(name: &CStr) -> *mut c_void {
        // SAFETY: a NUL-terminated selector name.
        unsafe { sel_registerName(name.as_ptr()) }
    }

    /// A shared-storage `MTLTexture` (RGBA8Unorm, 16x16) of `device`, owned.
    fn new_rgba8_texture(device: *mut c_void) -> *mut c_void {
        const MTL_PIXEL_FORMAT_RGBA8_UNORM: usize = 70;
        const MTL_TEXTURE_USAGE_SHADER_READ: usize = 1;
        // SAFETY: MTLTextureDescriptor's class method and MTLDevice's
        // method, with their signatures.
        unsafe {
            let desc_fn: unsafe extern "C" fn(
                *mut c_void,
                *mut c_void,
                usize,
                usize,
                usize,
                bool,
            ) -> *mut c_void = std::mem::transmute(objc_msgSend as unsafe extern "C" fn());
            let set_usage: unsafe extern "C" fn(*mut c_void, *mut c_void, usize) =
                std::mem::transmute(objc_msgSend as unsafe extern "C" fn());
            let new_texture: unsafe extern "C" fn(
                *mut c_void,
                *mut c_void,
                *mut c_void,
            ) -> *mut c_void = std::mem::transmute(objc_msgSend as unsafe extern "C" fn());
            let desc = desc_fn(
                objc_getClass(c"MTLTextureDescriptor".as_ptr()),
                sel(c"texture2DDescriptorWithPixelFormat:width:height:mipmapped:"),
                MTL_PIXEL_FORMAT_RGBA8_UNORM,
                16,
                16,
                false,
            );
            set_usage(desc, sel(c"setUsage:"), MTL_TEXTURE_USAGE_SHADER_READ);
            new_texture(device, sel(c"newTextureWithDescriptor:"), desc)
        }
    }

    /// `[obj retainCount]`.
    fn retain_count(obj: *mut c_void) -> usize {
        // SAFETY: NSObject's method.
        unsafe {
            let f: unsafe extern "C" fn(*mut c_void, *mut c_void) -> usize =
                std::mem::transmute(objc_msgSend as unsafe extern "C" fn());
            f(obj, sel(c"retainCount"))
        }
    }

    /// `[obj respondsToSelector:@selector(name)]`.
    fn responds_to(obj: *mut c_void, name: &CStr) -> bool {
        // SAFETY: NSObject's method.
        unsafe {
            let f: unsafe extern "C" fn(*mut c_void, *mut c_void, *mut c_void) -> bool =
                std::mem::transmute(objc_msgSend as unsafe extern "C" fn());
            f(obj, sel(c"respondsToSelector:"), sel(name))
        }
    }

    /// The class name of an object.
    fn class_name(obj: *mut c_void) -> String {
        unsafe extern "C" {
            fn object_getClassName(obj: *mut c_void) -> *const c_char;
        }
        // SAFETY: a valid object.
        unsafe { CStr::from_ptr(object_getClassName(obj)) }
            .to_string_lossy()
            .into_owned()
    }

    // ------------------------------------------------------------------
    // The Metal renderer

    /// The Metal renderer against the software renderer; `false` (with the
    /// skip reported) without a Metal device.
    fn metal_renderer() -> bool {
        let window = Window::create("SDL Metal renderer", RW, RH, WindowFlags::default()).unwrap();
        pump_for(100);
        let mut r = match Renderer::for_window(&window, Some("metal")) {
            Ok(r) => r,
            Err(e) => {
                skip("metal", e.message());
                window.destroy();
                return false;
            }
        };
        assert_eq!(r.name(), "metal");
        // The default renderer of a window is the Metal one.
        assert!(window.flags().unwrap().contains(WindowFlags::METAL));
        assert_eq!(r.output_size().unwrap(), window.size_in_pixels().unwrap());
        let formats = r.texture_formats().to_vec();
        println!("cocoa_metal: texture formats {formats:?}");
        for f in [
            PixelFormat::ARGB8888,
            PixelFormat::ABGR8888,
            PixelFormat::ABGR2101010,
            PixelFormat::RGBA64_FLOAT,
            PixelFormat::RGBA128_FLOAT,
            PixelFormat::INDEX8,
            PixelFormat::YV12,
            PixelFormat::IYUV,
            PixelFormat::I444,
            PixelFormat::NV12,
            PixelFormat::NV21,
            PixelFormat::P010,
        ] {
            assert!(formats.contains(&f), "{f:?} in {formats:?}");
        }

        // Its properties: the device and command queue, and the maximum
        // texture size (Metal-Feature-Set-Tables.pdf).
        let props = r.properties();
        let device = props
            .get_number(sdl3::render::PROP_RENDERER_METAL_DEVICE_POINTER)
            .unwrap_or(0) as usize as *mut c_void;
        let queue = props
            .get_number(sdl3::render::PROP_RENDERER_METAL_COMMAND_QUEUE_POINTER)
            .unwrap_or(0) as usize as *mut c_void;
        assert!(!device.is_null() && !queue.is_null());
        // (the driver's classes: an MTLDevice and an MTLCommandQueue by
        // their methods)
        assert!(
            responds_to(device, c"newCommandQueue"),
            "{}",
            class_name(device)
        );
        assert!(
            responds_to(queue, c"commandBuffer"),
            "{}",
            class_name(queue)
        );
        assert_eq!(
            props.get_number(sdl3::render::PROP_RENDERER_MAX_TEXTURE_SIZE_NUMBER),
            Some(16384)
        );
        let layer = r.metal_layer().expect("a Metal layer");
        assert_eq!(class_name(layer.as_ptr()), "CAMetalLayer");
        let encoder = r.metal_command_encoder().expect("a command encoder");
        assert!(
            responds_to(encoder.as_ptr(), c"setRenderPipelineState:")
                && responds_to(encoder.as_ptr(), c"drawPrimitives:vertexStart:vertexCount:"),
            "{}",
            class_name(encoder.as_ptr())
        );
        r.present().unwrap();

        common_scenes(&mut r);

        // The packed formats Apple GPUs (family Apple1) have, and the
        // 10-bit and float ones.
        let mut packed = vec![
            PixelFormat::ABGR2101010,
            PixelFormat::RGBA64_FLOAT,
            PixelFormat::RGBA128_FLOAT,
        ];
        if formats.contains(&PixelFormat::RGB565) {
            packed.extend([
                PixelFormat::RGB565,
                PixelFormat::RGBA5551,
                PixelFormat::ARGB1555,
                PixelFormat::RGBA4444,
            ]);
        }
        for format in packed {
            // (the surface's quantization is the texture's on both sides)
            compare_scene(&mut r, &format!("texture {format:?}"), 2, 0, |r| {
                r.set_draw_blend_mode(BlendMode::NONE)?;
                r.set_draw_color(40, 50, 60, 255);
                r.clear()?;
                let mut s = pattern_surface(PixelFormat::ARGB8888).convert(format)?;
                let tex = r.create_texture_from_surface(&mut s)?;
                r.set_texture_scale_mode(tex, ScaleMode::Nearest)?;
                r.render_texture(tex, None, Some(&FRect::new(4.0, 4.0, 32.0, 32.0)))?;
                r.destroy_texture(tex);
                Ok(())
            });
        }

        // A palette texture, with its palette changed between draws
        compare_scene(&mut r, "palette texture", 0, 0, |r| {
            r.set_draw_color(0, 0, 0, 255);
            r.clear()?;
            let mut s = Surface::new(16, 16, PixelFormat::INDEX8)?;
            let palette = s.create_palette()?;
            {
                let mut p = palette.write().unwrap();
                let colors: Vec<Color> = (0..256)
                    .map(|i| Color::new(i as u8, (255 - i) as u8, (i * 7) as u8, 255))
                    .collect();
                p.set_colors(0, &colors)?;
            }
            {
                let pitch = s.pitch() as usize;
                let px = s.pixels_mut().unwrap();
                for y in 0..16 {
                    for x in 0..16 {
                        px[y * pitch + x] = (x * 16 + y) as u8;
                    }
                }
            }
            let tex = r.create_texture_from_surface(&mut s)?;
            r.set_texture_scale_mode(tex, ScaleMode::Nearest)?;
            r.render_texture(tex, None, Some(&FRect::new(0.0, 0.0, 16.0, 16.0)))?;
            r.render_texture(tex, None, Some(&FRect::new(20.0, 4.0, 32.0, 32.0)))?;
            r.destroy_texture(tex);
            Ok(())
        });

        // YUV textures: the shaders' conversion against SDL's (a few levels
        // apart at the color edges, where the chroma is subsampled)
        for format in [
            PixelFormat::IYUV,
            PixelFormat::YV12,
            PixelFormat::NV12,
            PixelFormat::NV21,
        ] {
            compare_scene(&mut r, &format!("texture {format:?}"), 6, 24, |r| {
                r.set_draw_color(0, 0, 0, 255);
                r.clear()?;
                let tex = r.create_texture(format, TextureAccess::Static, 32, 32)?;
                r.set_texture_scale_mode(tex, ScaleMode::Nearest)?;
                let pixels = yuv_pixels(format, 32, 32);
                r.update_texture(tex, None, &pixels, 32)?;
                r.render_texture(tex, None, Some(&FRect::new(8.0, 8.0, 32.0, 32.0)))?;
                r.destroy_texture(tex);
                Ok(())
            });
        }
        // The planes given separately
        compare_scene(&mut r, "texture IYUV planes", 6, 24, |r| {
            r.set_draw_color(0, 0, 0, 255);
            r.clear()?;
            let tex = r.create_texture(PixelFormat::IYUV, TextureAccess::Static, 32, 32)?;
            r.set_texture_scale_mode(tex, ScaleMode::Nearest)?;
            let pixels = yuv_pixels(PixelFormat::IYUV, 32, 32);
            let (y, rest) = pixels.split_at(32 * 32);
            let (u, v) = rest.split_at(16 * 16);
            r.update_yuv_texture(tex, None, y, 32, u, 16, v, 16)?;
            r.render_texture(tex, None, Some(&FRect::new(8.0, 8.0, 32.0, 32.0)))?;
            r.destroy_texture(tex);
            Ok(())
        });
        compare_scene(&mut r, "texture NV12 planes", 6, 24, |r| {
            r.set_draw_color(0, 0, 0, 255);
            r.clear()?;
            let tex = r.create_texture(PixelFormat::NV12, TextureAccess::Static, 32, 32)?;
            r.set_texture_scale_mode(tex, ScaleMode::Nearest)?;
            let pixels = yuv_pixels(PixelFormat::NV12, 32, 32);
            let (y, uv) = pixels.split_at(32 * 32);
            r.update_nv_texture(tex, None, y, 32, uv, 32)?;
            r.render_texture(tex, None, Some(&FRect::new(8.0, 8.0, 32.0, 32.0)))?;
            r.destroy_texture(tex);
            Ok(())
        });

        // Locked streaming textures, RGB and YUV
        compare_scene(&mut r, "locked texture", 0, 0, |r| {
            r.set_draw_color(0, 0, 0, 255);
            r.clear()?;
            let tex = r.create_texture(PixelFormat::ABGR8888, TextureAccess::Streaming, 16, 16)?;
            r.set_texture_scale_mode(tex, ScaleMode::Nearest)?;
            {
                let mut lock = r.lock_texture(tex, Some(&Rect::new(4, 2, 8, 10)))?;
                let pitch = lock.pitch() as usize;
                let px = lock.pixels();
                for y in 0..10 {
                    for x in 0..8 {
                        px[y * pitch + x * 4..][..4].copy_from_slice(&[
                            (x * 30) as u8,
                            (y * 25) as u8,
                            99,
                            255,
                        ]);
                    }
                }
            }
            r.render_texture(tex, None, Some(&FRect::new(0.0, 0.0, 32.0, 32.0)))?;
            r.destroy_texture(tex);
            Ok(())
        });
        compare_scene(&mut r, "locked NV12 texture", 6, 24, |r| {
            r.set_draw_color(0, 0, 0, 255);
            r.clear()?;
            let tex = r.create_texture(PixelFormat::NV12, TextureAccess::Streaming, 32, 32)?;
            r.set_texture_scale_mode(tex, ScaleMode::Nearest)?;
            let pixels = yuv_pixels(PixelFormat::NV12, 32, 32);
            {
                let mut lock = r.lock_texture(tex, None)?;
                lock.pixels()[..pixels.len()].copy_from_slice(&pixels);
            }
            r.render_texture(tex, None, Some(&FRect::new(8.0, 8.0, 32.0, 32.0)))?;
            r.destroy_texture(tex);
            Ok(())
        });

        // Custom blend modes, address modes and linear scaling
        // (the software renderer has no custom blend modes: the expected
        // pixels are computed, dst - src for the color, the dst alpha)
        let mode = BlendMode::compose_custom(
            BlendFactor::One,
            BlendFactor::One,
            BlendOperation::RevSubtract,
            BlendFactor::Zero,
            BlendFactor::One,
            BlendOperation::Add,
        );
        r.set_draw_blend_mode(BlendMode::NONE).unwrap();
        r.set_draw_color(100, 150, 200, 255);
        r.clear().unwrap();
        r.set_draw_blend_mode(mode).unwrap();
        r.set_draw_color(60, 60, 60, 128);
        r.render_fill_rect(Some(&FRect::new(10.0, 10.0, 30.0, 20.0)))
            .unwrap();
        r.set_draw_blend_mode(BlendMode::NONE).unwrap();
        let s = r.read_pixels(None).unwrap();
        assert_eq!(s.read_pixel(20, 20).unwrap(), Color::new(40, 90, 140, 255));
        assert_eq!(s.read_pixel(5, 5).unwrap(), Color::new(100, 150, 200, 255));
        r.present().unwrap();
        compare_scene(&mut r, "wrapping geometry", 2, 0, |r| {
            r.set_draw_color(0, 0, 0, 255);
            r.clear()?;
            let tex = r.create_texture_from_surface(&mut pattern_surface(PixelFormat::ARGB8888))?;
            r.set_texture_scale_mode(tex, ScaleMode::Nearest)?;
            r.set_texture_blend_mode(tex, BlendMode::NONE)?;
            r.set_texture_address_mode(TextureAddressMode::Wrap, TextureAddressMode::Wrap);
            let white = FColor::new(1.0, 1.0, 1.0, 1.0);
            let v = |x: f32, y: f32, u: f32, w: f32| Vertex {
                position: FPoint { x, y },
                color: white,
                tex_coord: FPoint { x: u, y: w },
            };
            let verts = [
                v(0.0, 0.0, 0.0, 0.0),
                v(32.0, 0.0, 2.0, 0.0),
                v(32.0, 32.0, 2.0, 2.0),
                v(0.0, 32.0, 0.0, 2.0),
            ];
            r.render_geometry(Some(tex), &verts, Some(&[0, 1, 2, 0, 2, 3]))?;
            r.set_texture_address_mode(TextureAddressMode::Auto, TextureAddressMode::Auto);
            r.destroy_texture(tex);
            Ok(())
        });
        compare_scene(&mut r, "linear scaling", 6, 40, |r| {
            r.set_draw_color(0, 0, 0, 255);
            r.clear()?;
            let tex = r.create_texture_from_surface(&mut pattern_surface(PixelFormat::XRGB8888))?;
            r.set_texture_scale_mode(tex, ScaleMode::Linear)?;
            r.render_texture(tex, None, Some(&FRect::new(0.0, 0.0, 48.0, 40.0)))?;
            r.destroy_texture(tex);
            Ok(())
        });
        compare_scene(&mut r, "debug text", 0, 0, |r| {
            r.set_draw_color(0, 0, 0, 255);
            r.clear()?;
            r.set_draw_color(255, 255, 255, 255);
            r.render_debug_text(2.0, 2.0, "Metal")?;
            r.render_debug_text(2.0, 20.0, "SDL 3")
        });

        // A partial read, and a read from a render target
        r.set_draw_color(0, 0, 0, 255);
        r.clear().unwrap();
        r.set_draw_color(255, 0, 0, 255);
        r.render_fill_rect(Some(&FRect::new(10.0, 10.0, 4.0, 4.0)))
            .unwrap();
        let part = r.read_pixels(Some(&Rect::new(8, 8, 8, 8))).unwrap();
        assert_eq!((part.width(), part.height()), (8, 8));
        assert_eq!(part.read_pixel(0, 0).unwrap(), Color::new(0, 0, 0, 255));
        assert_eq!(part.read_pixel(3, 3).unwrap(), Color::new(255, 0, 0, 255));
        r.present().unwrap();
        let target = r
            .create_texture(PixelFormat::ABGR8888, TextureAccess::Target, 20, 10)
            .unwrap();
        r.set_render_target(Some(target)).unwrap();
        r.set_draw_color(0, 0, 255, 255);
        r.clear().unwrap();
        let s = r.read_pixels(None).unwrap();
        assert_eq!((s.width(), s.height()), (20, 10));
        assert_eq!(s.read_pixel(19, 9).unwrap(), Color::new(0, 0, 255, 255));
        r.set_render_target(None).unwrap();
        r.destroy_texture(target);

        // A texture wrapping the application's MTLTexture, which the
        // renderer holds a reference to while the texture exists
        let mtltexture = new_rgba8_texture(device);
        assert!(!mtltexture.is_null());
        let before = retain_count(mtltexture);
        compare_scene(&mut r, "wrapped MTLTexture", 2, 0, |r| {
            r.set_draw_color(0, 0, 0, 255);
            r.clear()?;
            let tex = r.create_texture_with(&TextureCreateInfo {
                format: PixelFormat::ABGR8888,
                access: TextureAccess::Static,
                width: 16,
                height: 16,
                // SAFETY: a texture of the renderer's device, alive for the
                // whole check.
                metal_texture: unsafe { NativeObject::from_ptr(mtltexture) },
                ..Default::default()
            })?;
            if r.name() == "metal" {
                let props = r.texture_properties(tex)?;
                assert_eq!(
                    props.get_number(sdl3::render::PROP_TEXTURE_METAL_TEXTURE_POINTER),
                    Some(mtltexture as usize as i64)
                );
            }
            let mut s = pattern_surface(PixelFormat::ABGR8888);
            let pitch = s.pitch();
            r.update_texture(tex, None, s.pixels_mut().unwrap(), pitch)?;
            r.set_texture_scale_mode(tex, ScaleMode::Nearest)?;
            r.set_texture_blend_mode(tex, BlendMode::NONE)?;
            r.render_texture(tex, None, Some(&FRect::new(4.0, 4.0, 16.0, 16.0)))?;
            r.destroy_texture(tex);
            Ok(())
        });
        // (the command buffer that drew with the texture retains it until
        // its completion handler has run, which can be a moment after the
        // readback's waitUntilCompleted returned on a slow software device)
        let deadline = Instant::now() + Duration::from_secs(5);
        while retain_count(mtltexture) != before && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(5));
        }
        assert_eq!(retain_count(mtltexture), before);
        // SAFETY: the reference new_rgba8_texture() returned.
        unsafe { objc_release(mtltexture) };

        // vsync: 0 and 1 (CAMetalLayer.displaySyncEnabled), nothing else
        r.set_vsync(1).unwrap();
        assert_eq!(r.vsync(), 1);
        for _ in 0..3 {
            r.clear().unwrap();
            r.present().unwrap();
        }
        r.set_vsync(0).unwrap();
        assert_eq!(r.vsync(), 0);
        assert!(r.set_vsync(2).is_err());
        assert_eq!(r.vsync(), 0);

        drop(r);
        window.destroy();
        println!("cocoa_metal: metal renderer ok");
        true
    }

    /// A Metal renderer of a window that is resized and hidden, and of a
    /// window with its own Metal view.
    fn metal_renderer_window_states() {
        let window = Window::create("SDL Metal resize", RW, RH, WindowFlags::RESIZABLE).unwrap();
        pump_for(50);
        let mut r = Renderer::for_window(&window, Some("metal")).unwrap();
        r.set_draw_color(0, 128, 0, 255);
        r.clear().unwrap();
        r.present().unwrap();

        // The drawable follows the window.
        window.set_size(100, 70).unwrap();
        pump_for(200);
        assert_eq!(r.output_size().unwrap(), window.size_in_pixels().unwrap());
        r.clear().unwrap();
        let s = r.read_pixels(None).unwrap();
        assert_eq!((s.width(), s.height()), window.size_in_pixels().unwrap());
        assert_eq!(s.read_pixel(99, 69).unwrap(), Color::new(0, 128, 0, 255));
        r.present().unwrap();

        // Hidden: presenting goes on without error.
        window.hide().unwrap();
        pump_for(100);
        r.clear().unwrap();
        r.present().unwrap();
        window.show().unwrap();
        pump_for(100);
        r.clear().unwrap();
        r.present().unwrap();
        drop(r);

        // A renderer made again for the window uses the window's Metal view
        // (GetWindowView()).
        let mut r = Renderer::for_window(&window, Some("metal")).unwrap();
        r.set_draw_color(1, 2, 3, 255);
        r.clear().unwrap();
        assert_eq!(
            r.read_pixels(Some(&Rect::new(0, 0, 1, 1)))
                .unwrap()
                .read_pixel(0, 0)
                .unwrap(),
            Color::new(1, 2, 3, 255)
        );
        r.present().unwrap();
        drop(r);
        window.destroy();
        println!("cocoa_metal: metal renderer window states ok");
    }

    // ------------------------------------------------------------------
    // The GPU API on Cocoa windows

    fn gpu_swapchains() {
        use sdl3::gpu::{
            ColorTargetInfo, Device, LoadOp, PresentMode, ShaderFormat, SwapchainComposition,
            TextureFormat,
        };

        hints::set(hints::GPU_DRIVER, "metal").unwrap();
        let device = match Device::new(ShaderFormat::MSL | ShaderFormat::METALLIB, true, None) {
            Ok(d) => d,
            Err(e) => {
                hints::reset(hints::GPU_DRIVER);
                skip("metal", &format!("no GPU device ({})", e.message()));
                return;
            }
        };
        assert_eq!(device.driver(), "metal");
        let window = Window::create("SDL GPU swapchain", 64, 48, WindowFlags::RESIZABLE).unwrap();
        pump_for(100);

        // Not claimed yet (METAL_SupportsPresentMode() doesn't look at the
        // window)
        assert!(device.window_supports_present_mode(&window, PresentMode::Vsync));
        assert!(device.swapchain_texture_format(&window).is_err());
        {
            let mut cb = device.acquire_command_buffer().unwrap();
            let e = cb.acquire_swapchain_texture(&window).unwrap_err();
            assert_eq!(e.message(), "Window is not claimed by this SDL_GPUDevice");
            cb.cancel().unwrap();
        }

        device.claim_window(&window).unwrap();
        assert!(window.flags().unwrap().contains(WindowFlags::METAL));
        assert!(device.window_supports_swapchain_composition(&window, SwapchainComposition::Sdr));
        assert!(device.window_supports_present_mode(&window, PresentMode::Vsync));
        assert!(device.window_supports_present_mode(&window, PresentMode::Immediate));
        assert!(!device.window_supports_present_mode(&window, PresentMode::Mailbox));
        let format = device.swapchain_texture_format(&window).unwrap();
        assert_eq!(format, TextureFormat::B8G8R8A8_UNORM, "{format:?}");
        println!(
            "cocoa_metal: swapchain {format:?}, compositions: linear {} HDR10 {} extended linear {}",
            device.window_supports_swapchain_composition(&window, SwapchainComposition::SdrLinear),
            device.window_supports_swapchain_composition(&window, SwapchainComposition::Hdr10St2084),
            device.window_supports_swapchain_composition(
                &window,
                SwapchainComposition::HdrExtendedLinear
            ),
        );

        // A second claim is counted, and so is its release.
        device.claim_window(&window).unwrap();
        device.release_window(&window);
        assert_eq!(device.swapchain_texture_format(&window).unwrap(), format);

        let blue = FColor::new(0.0, 0.0, 1.0, 1.0);
        let frame = |device: &Device, block: bool| -> Option<(u32, u32)> {
            let mut cb = device.acquire_command_buffer().unwrap();
            let texture = if block {
                cb.wait_and_acquire_swapchain_texture(&window).unwrap()
            } else {
                cb.acquire_swapchain_texture(&window).unwrap()
            };
            let size = texture.as_ref().map(|t| (t.width, t.height));
            if let Some(texture) = &texture {
                let target = ColorTargetInfo {
                    load_op: LoadOp::Clear,
                    clear_color: blue,
                    ..ColorTargetInfo::new(&texture.texture)
                };
                cb.begin_render_pass(&[target], None).unwrap().end();
            }
            cb.submit().unwrap();
            size
        };
        let pixels = window.size_in_pixels().unwrap();
        let pixels = (pixels.0 as u32, pixels.1 as u32);
        for _ in 0..5 {
            assert_eq!(frame(&device, true), Some(pixels));
        }
        device.wait_for_swapchain(&window).unwrap();
        // (without blocking a frame can be skipped, which isn't an error)
        for _ in 0..3 {
            if let Some(size) = frame(&device, false) {
                assert_eq!(size, pixels);
            }
        }

        // Frames in flight and the swapchain parameters.
        device.set_allowed_frames_in_flight(3).unwrap();
        for _ in 0..4 {
            assert_eq!(frame(&device, true), Some(pixels));
        }
        device.set_allowed_frames_in_flight(1).unwrap();
        assert_eq!(frame(&device, true), Some(pixels));
        device
            .set_swapchain_parameters(&window, SwapchainComposition::Sdr, PresentMode::Immediate)
            .unwrap();
        assert_eq!(frame(&device, true), Some(pixels));
        device
            .set_swapchain_parameters(&window, SwapchainComposition::Sdr, PresentMode::Vsync)
            .unwrap();
        assert!(device
            .set_swapchain_parameters(&window, SwapchainComposition::Sdr, PresentMode::Mailbox)
            .is_err());
        device.set_allowed_frames_in_flight(2).unwrap();

        // A resize: the drawable follows the window.
        window.set_size(40, 30).unwrap();
        pump_for(200);
        let pixels = window.size_in_pixels().unwrap();
        let pixels = (pixels.0 as u32, pixels.1 as u32);
        for _ in 0..3 {
            assert_eq!(frame(&device, true), Some(pixels));
        }

        // Released: the window can't present any more (and can be claimed
        // again).
        device.release_window(&window);
        assert!(device.swapchain_texture_format(&window).is_err());
        device.claim_window(&window).unwrap();
        assert_eq!(frame(&device, true), Some(pixels));
        // (the device releases the windows it still has when it goes)
        drop(device);
        hints::reset(hints::GPU_DRIVER);
        window.destroy();
        println!("cocoa_metal: gpu swapchains ok");
    }

    /// The GPU renderer drawing to a window through Metal swapchains.
    fn gpu_renderer_on_a_window() {
        hints::set(hints::GPU_DRIVER, "metal").unwrap();
        hints::set(hints::RENDER_GPU_DEBUG, "1").unwrap();
        let window = Window::create("SDL GPU renderer", RW, RH, WindowFlags::default()).unwrap();
        pump_for(100);
        let mut r = Renderer::for_window(&window, Some("gpu")).unwrap();
        assert_eq!(r.name(), "gpu");
        assert_eq!(r.output_size().unwrap(), window.size_in_pixels().unwrap());
        common_scenes(&mut r);
        compare_scene(&mut r, "texture NV12", 6, 24, |r| {
            r.set_draw_color(0, 0, 0, 255);
            r.clear()?;
            let tex = r.create_texture(PixelFormat::NV12, TextureAccess::Static, 32, 32)?;
            r.set_texture_scale_mode(tex, ScaleMode::Nearest)?;
            let pixels = yuv_pixels(PixelFormat::NV12, 32, 32);
            r.update_texture(tex, None, &pixels, 32)?;
            r.render_texture(tex, None, Some(&FRect::new(8.0, 8.0, 32.0, 32.0)))?;
            r.destroy_texture(tex);
            Ok(())
        });

        // A texture wrapping a GPU texture of the renderer's device
        let device = r.gpu_renderer_device().expect("the renderer's device");
        let gtex = std::sync::Arc::new(
            device
                .create_texture(&sdl3::gpu::TextureCreateInfo {
                    texture_type: sdl3::gpu::TextureType::Texture2D,
                    format: sdl3::gpu::TextureFormat::R8G8B8A8_UNORM,
                    usage: sdl3::gpu::TextureUsageFlags::SAMPLER,
                    width: 16,
                    height: 16,
                    layer_count_or_depth: 1,
                    num_levels: 1,
                    sample_count: sdl3::gpu::SampleCount::One,
                    props: None,
                })
                .unwrap(),
        );
        compare_scene(&mut r, "wrapped GPU texture", 2, 0, |r| {
            r.set_draw_color(0, 0, 0, 255);
            r.clear()?;
            let tex = r.create_texture_with(&TextureCreateInfo {
                format: PixelFormat::ABGR8888,
                access: TextureAccess::Static,
                width: 16,
                height: 16,
                gpu_texture: Some(gtex.clone()),
                ..Default::default()
            })?;
            let mut s = pattern_surface(PixelFormat::ABGR8888);
            let pitch = s.pitch();
            r.update_texture(tex, None, s.pixels_mut().unwrap(), pitch)?;
            r.set_texture_scale_mode(tex, ScaleMode::Nearest)?;
            r.set_texture_blend_mode(tex, BlendMode::NONE)?;
            r.render_texture(tex, None, Some(&FRect::new(4.0, 4.0, 16.0, 16.0)))?;
            r.destroy_texture(tex);
            Ok(())
        });
        drop(gtex);
        drop(r);

        // On the application's device
        let device = sdl3::gpu::Device::new(sdl3::gpu::ShaderFormat::MSL, true, None).unwrap();
        let mut r = Renderer::for_window_with(
            &window,
            &RendererCreateInfo {
                name: Some("gpu".to_owned()),
                gpu_device: Some(device.clone()),
                ..Default::default()
            },
        )
        .unwrap();
        compare_scene(&mut r, "on an existing device", 0, 0, |r| {
            r.set_draw_color(10, 20, 30, 255);
            r.clear()?;
            r.set_draw_color(200, 100, 50, 255);
            r.render_fill_rect(Some(&FRect::new(8.0, 8.0, 32.0, 24.0)))
        });
        drop(r);
        drop(device);

        hints::reset(hints::RENDER_GPU_DEBUG);
        hints::reset(hints::GPU_DRIVER);
        window.destroy();
        println!("cocoa_metal: gpu renderer on a window ok");
    }
}
