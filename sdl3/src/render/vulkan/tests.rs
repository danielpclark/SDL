// Tests of the Vulkan renderer.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! The tables are checked against upstream's values. The scenes are drawn
//! by the Vulkan renderer on a real device, through the offscreen driver's
//! headless surface (Mesa's lavapipe on Linux CI), and by the software
//! renderer, and the pixels compared: exactly where the math is exact
//! (clears, fills, points, 1:1 copies, palettes), within a small tolerance
//! where the GPU's float blending or YUV conversion rounds differently.
//! Without a Vulkan loader and driver with `VK_EXT_headless_surface` the
//! device tests report a skip (capability `vulkan`) and pass.

use super::shaders::Shader;
use super::*;
use crate::events::window::WindowFlags;
use crate::init::{self, InitFlags};
use crate::render::{
    Renderer, Texture, TextureAccess, TextureCreateInfo, Vertex,
    PROP_RENDERER_MAX_TEXTURE_SIZE_NUMBER, PROP_RENDERER_TEXTURE_WRAPPING_BOOLEAN,
    SOFTWARE_RENDERER,
};
use crate::video::blendmode::{BlendFactor as BF, BlendOperation as BO};
use crate::video::pixels::Palette;
use crate::video::surface::share_palette;
use crate::video::FlipMode;

const W: i32 = 64;
const H: i32 = 48;

// ---------------------------------------------------------------------------
// Without a device
// ---------------------------------------------------------------------------

#[test]
#[cfg(not(windows))] // (direct3d11 comes first there: see its tests)
fn driver_list_follows_upstreams_order() {
    use crate::render::{num_render_drivers, render_driver};
    // (render_drivers[] in SDL_render.c: ..., GLES2, ..., VULKAN, GPU, SW)
    assert_eq!(num_render_drivers(), 4);
    assert_eq!(render_driver(0).unwrap(), "opengl");
    assert_eq!(render_driver(1).unwrap(), "opengles2");
    assert_eq!(render_driver(2).unwrap(), "vulkan");
    assert_eq!(render_driver(3).unwrap(), SOFTWARE_RENDERER);
    assert!(render_driver(4).is_err());
}

#[test]
fn spirv_shaders() {
    // The word counts of the arrays in upstream's VULKAN_*.h headers.
    let sizes = [
        (Shader::Solid, 351),
        (Shader::SolidPq, 503),
        (Shader::Rgb, 422),
        (Shader::Advanced, 4122),
        (Shader::RgbPq, 3467),
        (Shader::RgbSimple, 143),
    ];
    for (shader, words) in sizes {
        let ps = shader.pixel_shader();
        let vs = shader.vertex_shader();
        assert_eq!(ps.len(), words, "{shader:?}");
        assert_eq!(vs.len(), 373, "{shader:?}");
        // SPIR-V magic, version 1.0, glslang's generator id
        assert_eq!(&ps[..3], &[0x07230203, 0x00010000, 0x0008000b]);
        assert_eq!(&vs[..3], &[0x07230203, 0x00010000, 0x0008000b]);
        // ... ending with OpReturn, OpFunctionEnd
        assert_eq!(&ps[ps.len() - 2..], &[0x000100fd, 0x00010038]);
    }
    assert_eq!(Shader::ALL.len(), Shader::COUNT);
    for (i, shader) in Shader::ALL.iter().enumerate() {
        assert_eq!(*shader as usize, i);
    }
    // The last words of VULKAN_PixelShader_Colors.h
    let colors = Shader::Solid.pixel_shader();
    assert_eq!(
        &colors[colors.len() - 8..],
        &[
            0x00000080, 0x000000a8, 0x0000004a, 0x0003003e, 0x0000004d, 0x00000080, 0x000100fd,
            0x00010038
        ]
    );
}

#[test]
fn format_tables() {
    use PixelFormat as F;
    assert_eq!(
        vk_format_to_sdl_pixel_format(VK_FORMAT_B8G8R8A8_UNORM),
        F::BGRA32
    );
    assert_eq!(
        vk_format_to_sdl_pixel_format(VK_FORMAT_B8G8R8A8_SRGB),
        F::BGRA32
    );
    assert_eq!(
        vk_format_to_sdl_pixel_format(VK_FORMAT_R8G8B8A8_SRGB),
        F::RGBA32
    );
    assert_eq!(
        vk_format_to_sdl_pixel_format(VK_FORMAT_A4B4G4R4_UNORM_PACK16_EXT),
        F::ABGR4444
    );
    assert_eq!(
        vk_format_to_sdl_pixel_format(VK_FORMAT_R8_UNORM),
        F::UNKNOWN
    );

    assert_eq!(get_format_image_count(F::IYUV), 3);
    assert_eq!(get_format_image_count(F::NV12), 1);
    assert_eq!(get_format_image_view_count(F::NV12), 2);
    assert_eq!(get_format_image_view_count(F::I4FL), 3);
    assert_eq!(get_format_image_view_count(F::ARGB8888), 1);

    assert_eq!(
        vk_format_get_num_planes(VK_FORMAT_G8_B8R8_2PLANE_420_UNORM),
        2
    );
    assert_eq!(
        vk_format_get_num_planes(VK_FORMAT_G8_B8_R8_3PLANE_444_UNORM),
        3
    );
    assert_eq!(vk_format_get_num_planes(VK_FORMAT_R8G8B8A8_UNORM), 1);

    assert_eq!(
        get_bytes_per_pixel(VK_FORMAT_G8_B8R8_2PLANE_420_UNORM, 0),
        1
    );
    assert_eq!(
        get_bytes_per_pixel(VK_FORMAT_G8_B8R8_2PLANE_420_UNORM, 1),
        2
    );
    assert_eq!(
        get_bytes_per_pixel(VK_FORMAT_G10X6_B10X6R10X6_2PLANE_420_UNORM_3PACK16, 1),
        4
    );
    assert_eq!(get_bytes_per_pixel(VK_FORMAT_B8G8R8A8_UNORM, 0), 4);
    assert_eq!(get_bytes_per_pixel(VK_FORMAT_R5G6B5_UNORM_PACK16, 0), 2);
    assert_eq!(get_bytes_per_pixel(VK_FORMAT_R16G16B16A16_SFLOAT, 0), 8);

    let srgb = Colorspace::SRGB;
    let linear = Colorspace::SRGB_LINEAR;
    assert_eq!(
        get_vk_image_format(F::BGRA32, srgb),
        VK_FORMAT_B8G8R8A8_UNORM
    );
    assert_eq!(
        get_vk_image_format(F::BGRA32, linear),
        VK_FORMAT_B8G8R8A8_SRGB
    );
    assert_eq!(get_vk_image_format(F::INDEX8, srgb), VK_FORMAT_R8_UNORM);
    assert_eq!(
        get_vk_image_format(F::NV21, srgb),
        VK_FORMAT_G8_B8R8_2PLANE_420_UNORM
    );
    assert_eq!(get_vk_image_format(F::I0FL, srgb), VK_FORMAT_R16_UNORM);
    assert_eq!(get_vk_image_format(F::XRGB8888, srgb), VK_FORMAT_UNDEFINED);
    assert_eq!(
        get_vk_image_view_format(F::NV12, 0, srgb),
        VK_FORMAT_R8_UNORM
    );
    assert_eq!(
        get_vk_image_view_format(F::NV12, 1, srgb),
        VK_FORMAT_R8G8_UNORM
    );
    assert_eq!(
        get_vk_image_view_format(F::P010, 1, srgb),
        VK_FORMAT_R16G16_UNORM
    );
}

#[test]
fn blend_modes() {
    let custom = |sc, dc, co, sa, da, ao| BlendMode::compose_custom(sc, dc, co, sa, da, ao);
    let blend = BlendMode::BLEND;
    assert_eq!(
        get_blend_factor(blend.src_color_factor()),
        VK_BLEND_FACTOR_SRC_ALPHA
    );
    assert_eq!(
        get_blend_factor(blend.dst_color_factor()),
        VK_BLEND_FACTOR_ONE_MINUS_SRC_ALPHA
    );
    // (BlendMode::NONE is ONE, ZERO, ADD)
    assert_eq!(
        get_blend_factor(BlendMode::NONE.src_color_factor()),
        VK_BLEND_FACTOR_ONE
    );
    assert_eq!(
        get_blend_factor(BlendMode::NONE.dst_color_factor()),
        VK_BLEND_FACTOR_ZERO
    );
    assert_eq!(
        get_blend_op(Some(BO::RevSubtract)),
        VK_BLEND_OP_REVERSE_SUBTRACT
    );
    assert_eq!(get_blend_op(Some(BO::Maximum)), VK_BLEND_OP_MAX);
    assert_eq!(get_blend_op(None), VK_BLEND_OP_MAX_ENUM);
    assert_eq!(get_blend_factor(None), VK_BLEND_FACTOR_MAX_ENUM);
    // Every composed mode is supported (min/max included)
    let mode = custom(
        BF::DstColor,
        BF::Zero,
        BO::Minimum,
        BF::One,
        BF::OneMinusDstAlpha,
        BO::Maximum,
    );
    assert_ne!(
        get_blend_factor(mode.src_color_factor()),
        VK_BLEND_FACTOR_MAX_ENUM
    );
    assert_ne!(get_blend_op(mode.alpha_operation()), VK_BLEND_OP_MAX_ENUM);
}

#[test]
fn samplers_and_shader_selection() {
    use TextureAddressMode as A;
    assert_eq!(RENDER_SAMPLER_COUNT, 8);
    assert_eq!(
        render_sampler_hashkey(ScaleMode::Linear, A::Clamp, A::Clamp),
        0
    );
    assert_eq!(
        render_sampler_hashkey(ScaleMode::Nearest, A::Clamp, A::Clamp),
        1
    );
    assert_eq!(
        render_sampler_hashkey(ScaleMode::PixelArt, A::Wrap, A::Clamp),
        2
    );
    assert_eq!(
        render_sampler_hashkey(ScaleMode::Nearest, A::Wrap, A::Wrap),
        7
    );

    let mut constants = PixelShaderConstants {
        color_scale: 1.0,
        sdr_white_point: 80.0,
        ..Default::default()
    };
    assert!(!pq_shader_scales_input(&constants));
    constants.sdr_white_point = 203.0;
    assert!(pq_shader_scales_input(&constants));
    constants.sdr_white_point = 80.0;
    constants.tonemap_method = TONEMAP_CHROME;
    assert!(pq_shader_scales_input(&constants));

    // (the layout the shaders read: 12 floats and the 4x4 matrix)
    assert_eq!(size_of::<PixelShaderConstants>(), 112);
    assert_eq!(size_of::<VertexShaderConstants>(), 128);
    assert_eq!(size_of::<VertexPositionColor>(), 32);
    let a = PixelShaderConstants::default();
    let mut b = a;
    assert!(a.same_bits(&b));
    b.ycbcr_matrix[15] = -0.0;
    assert!(!a.same_bits(&b));
}

#[test]
fn matrices() {
    let id = Float4X4::identity();
    let r = Float4X4::rotation_z(std::f32::consts::PI * 0.5);
    assert!(Float4X4::multiply(&id, &r).same_bits(&r));
    assert!(Float4X4::multiply(&r, &id).same_bits(&r));
    assert_eq!(r.m[0][1], 1.0);
    assert_eq!(r.m[1][0], -1.0);
    let rr = Float4X4::multiply(&r, &r);
    assert!((rr.m[0][0] + 1.0).abs() < 1e-6);
    assert!(is_display_rotated_90_degrees(
        VK_SURFACE_TRANSFORM_ROTATE_270_BIT_KHR
    ));
    assert!(!is_display_rotated_90_degrees(
        VK_SURFACE_TRANSFORM_ROTATE_180_BIT_KHR
    ));
}

// ---------------------------------------------------------------------------
// With a device
// ---------------------------------------------------------------------------

/// Video up on the offscreen driver; quit when dropped.
struct Video;

impl Video {
    fn init() -> Video {
        init::quit();
        hints::set(hints::VIDEO_DRIVER, "offscreen").unwrap();
        init::init(InitFlags::VIDEO).unwrap();
        Video
    }
}

impl Drop for Video {
    fn drop(&mut self) {
        init::quit();
        hints::reset(hints::VIDEO_DRIVER);
    }
}

/// An offscreen window with a Vulkan renderer, or `None` (with the skip
/// reported) when there is no Vulkan.
fn vulkan_renderer(w: i32, h: i32) -> Option<(Window, Renderer)> {
    let window = Window::create("vulkan", w, h, WindowFlags::default()).unwrap();
    match Renderer::for_window(&window, Some(VULKAN_RENDERER)) {
        Ok(r) => Some((window, r)),
        Err(e) => {
            crate::test_support::skip(
                "vulkan",
                format_args!("no Vulkan renderer ({})", e.message()),
            );
            window.destroy();
            None
        }
    }
}

/// A software renderer drawing into an ARGB8888 surface (the format the
/// Vulkan renderer reads its B8G8R8A8 swapchain back in).
fn software_renderer(w: i32, h: i32) -> Renderer {
    Renderer::software(Surface::new(w, h, PixelFormat::ARGB8888).unwrap()).unwrap()
}

/// The pixels of a surface, row by row without padding.
fn rows(s: &Surface<'_>) -> Vec<u8> {
    let bpp = s.format().bytes_per_pixel() as usize;
    let row = s.width() as usize * bpp;
    let pixels = s.raw_pixels().unwrap();
    (0..s.height() as usize)
        .flat_map(|y| pixels[y * s.pitch() as usize..][..row].to_vec())
        .collect()
}

/// A pixel where two images differ: its position and both values.
type Difference = ((i32, i32), Vec<u8>, Vec<u8>);

/// The pixels where two images differ by more than `tol` in a channel.
fn differences(vk: &Surface<'_>, sw: &Surface<'_>, tol: u8) -> Vec<Difference> {
    let (a, b) = (rows(vk), rows(sw));
    let bpp = vk.format().bytes_per_pixel() as usize;
    let w = vk.width() as usize;
    // (the padding byte of XRGB8888 and XBGR8888 isn't compared)
    let padding = matches!(vk.format(), PixelFormat::XRGB8888 | PixelFormat::XBGR8888).then_some(3);
    a.chunks(bpp)
        .zip(b.chunks(bpp))
        .enumerate()
        .filter(|(_, (pa, pb))| {
            pa.iter()
                .zip(*pb)
                .enumerate()
                .any(|(c, (x, y))| Some(c) != padding && x.abs_diff(*y) > tol)
        })
        .map(|(i, (pa, pb))| (((i % w) as i32, (i / w) as i32), pa.to_vec(), pb.to_vec()))
        .collect()
}

/// Compare two images, each channel within `tol` (the software image is
/// converted to the Vulkan image's format first).
fn assert_close(what: &str, vk: &Surface<'_>, sw: &Surface<'_>, tol: u8) {
    let converted;
    let sw = if sw.format() != vk.format() {
        converted = sw.convert(vk.format()).unwrap();
        &converted
    } else {
        sw
    };
    assert_eq!(
        (vk.width(), vk.height()),
        (sw.width(), sw.height()),
        "{what}: sizes"
    );
    let bad = differences(vk, sw, tol);
    if let Some(((x, y), pa, pb)) = bad.first() {
        panic!(
            "{what}: {} pixels differ by more than {tol}; first at ({x}, {y}): vulkan {pa:?} sw {pb:?}",
            bad.len()
        );
    }
}

/// Draw `scene` with the Vulkan renderer and the software renderer, and
/// compare the results (`tol` per channel).
fn compare(what: &str, r: &mut Renderer, tol: u8, scene: impl Fn(&mut Renderer)) {
    scene(r);
    let vk = r.read_pixels(None).unwrap();
    let mut sw = software_renderer(W, H);
    scene(&mut sw);
    let sw = sw.read_pixels(None).unwrap();
    assert_close(what, &vk, &sw, tol);
}

/// An RGBA pattern with varying alpha.
fn pattern(w: i32, h: i32) -> Vec<u8> {
    let mut p = Vec::new();
    for y in 0..h {
        for x in 0..w {
            p.extend_from_slice(&[
                (x * 255 / (w - 1)) as u8,
                (y * 255 / (h - 1)) as u8,
                ((x + y) * 13 % 256) as u8,
                if (x / 4 + y / 4) % 2 == 0 {
                    255
                } else {
                    96 + x as u8
                },
            ]);
        }
    }
    p
}

/// A static texture of `format` with [`pattern`]'s pixels (given as
/// RGBA32 and converted).
fn pattern_texture(r: &mut Renderer, format: PixelFormat, w: i32, h: i32) -> Texture {
    let mut src = Surface::new(w, h, PixelFormat::RGBA32).unwrap();
    let pitch = src.pitch() as usize;
    let p = pattern(w, h);
    let pixels = src.pixels_mut().unwrap();
    for y in 0..h as usize {
        pixels[y * pitch..][..w as usize * 4]
            .copy_from_slice(&p[y * w as usize * 4..][..w as usize * 4]);
    }
    let converted = src.convert(format).unwrap();
    let t = r
        .create_texture(format, TextureAccess::Static, w, h)
        .unwrap();
    r.update_texture(t, None, converted.raw_pixels().unwrap(), converted.pitch())
        .unwrap();
    t
}

#[test]
fn vulkan_renderer_basics() {
    let _l = crate::test_support::test_lock();
    let _video = Video::init();
    let Some((window, mut r)) = vulkan_renderer(W, H) else {
        return;
    };
    assert_eq!(r.name(), "vulkan");
    assert!(window.flags().unwrap().contains(WindowFlags::VULKAN));
    assert!(!window.flags().unwrap().contains(WindowFlags::OPENGL));
    assert_eq!(r.output_size().unwrap(), (W, H));
    let props = r.properties();
    assert_eq!(
        props.get_number(PROP_RENDERER_MAX_TEXTURE_SIZE_NUMBER),
        Some(16384)
    );
    assert_eq!(
        props.get_bool(PROP_RENDERER_TEXTURE_WRAPPING_BOOLEAN),
        Some(true)
    );
    for prop in [
        PROP_RENDERER_VULKAN_INSTANCE_POINTER,
        PROP_RENDERER_VULKAN_SURFACE_NUMBER,
        PROP_RENDERER_VULKAN_PHYSICAL_DEVICE_POINTER,
        PROP_RENDERER_VULKAN_DEVICE_POINTER,
    ] {
        assert!(props.get_number(prop).unwrap_or(0) != 0, "{prop}");
    }
    assert!(props
        .get_number(PROP_RENDERER_VULKAN_GRAPHICS_QUEUE_FAMILY_INDEX_NUMBER)
        .is_some());
    assert!(props
        .get_number(PROP_RENDERER_VULKAN_PRESENT_QUEUE_FAMILY_INDEX_NUMBER)
        .is_some());
    assert!(
        props
            .get_number(PROP_RENDERER_VULKAN_SWAPCHAIN_IMAGE_COUNT_NUMBER)
            .unwrap()
            >= 1
    );
    let formats = r.texture_formats();
    assert_eq!(&formats[..2], &[PixelFormat::BGRA32, PixelFormat::RGBA32]);
    for f in [
        PixelFormat::INDEX8,
        PixelFormat::IYUV,
        PixelFormat::YV12,
        PixelFormat::NV12,
        PixelFormat::NV21,
    ] {
        assert!(formats.contains(&f), "{f:?} supported");
    }

    // The images are published as properties.
    let t = r
        .create_texture(PixelFormat::ARGB8888, TextureAccess::Static, 8, 8)
        .unwrap();
    let tp = r.texture_properties(t).unwrap();
    assert!(tp.get_number(PROP_TEXTURE_VULKAN_TEXTURE_NUMBER).unwrap() != 0);
    assert!(tp
        .get_number(PROP_TEXTURE_VULKAN_TEXTURE_U_NUMBER)
        .is_none());
    let yuv = r
        .create_texture(PixelFormat::IYUV, TextureAccess::Static, 8, 8)
        .unwrap();
    let yp = r.texture_properties(yuv).unwrap();
    assert!(yp.get_number(PROP_TEXTURE_VULKAN_TEXTURE_U_NUMBER).unwrap() != 0);
    assert!(yp.get_number(PROP_TEXTURE_VULKAN_TEXTURE_V_NUMBER).unwrap() != 0);
    // (an unsupported YUV colorspace is refused)
    assert!(r
        .create_texture_with(&TextureCreateInfo {
            format: PixelFormat::NV12,
            width: 8,
            height: 8,
            colorspace: Some(Colorspace::SRGB),
            ..Default::default()
        })
        .is_err());

    // Custom blend modes are supported
    let custom = BlendMode::compose_custom(
        BF::One,
        BF::One,
        BO::RevSubtract,
        BF::Zero,
        BF::One,
        BO::Maximum,
    );
    r.set_draw_blend_mode(custom).unwrap();
    r.set_draw_blend_mode(BlendMode::NONE).unwrap();

    // Presenting several frames goes around the swapchain
    for i in 0..5 {
        r.set_draw_color(10 * i, 2, 3, 255);
        r.clear().unwrap();
        r.present().unwrap();
    }
    r.set_vsync(1).unwrap();
    assert_eq!(r.vsync(), 1);
    r.set_draw_color(1, 2, 3, 255);
    r.clear().unwrap();
    r.present().unwrap();
    r.set_vsync(0).unwrap();
    assert!(r.set_vsync(2).is_err());

    // No semaphores is fine
    r.add_vulkan_render_semaphores(0, 0, 0).unwrap();
    assert!(software_renderer(4, 4)
        .add_vulkan_render_semaphores(0, 0, 0)
        .is_err());

    // A resized window gets a new swapchain
    window.set_size(40, 30).unwrap();
    r.set_draw_color(9, 8, 7, 255);
    r.clear().unwrap();
    assert_eq!(r.output_size().unwrap(), (40, 30));
    let s = r.read_pixels(None).unwrap();
    assert_eq!((s.width(), s.height()), (40, 30));
    assert_eq!(rows(&s)[..4], [7, 8, 9, 255]);

    // The window can go first.
    window.destroy();
    assert!(r.clear().is_err());
    drop(r);
}

#[test]
fn vulkan_matches_software_for_shapes() {
    let _l = crate::test_support::test_lock();
    let _video = Video::init();
    let Some((_window, mut r)) = vulkan_renderer(W, H) else {
        return;
    };

    compare("clear", &mut r, 0, |r| {
        r.set_draw_color(12, 34, 56, 255);
        r.clear().unwrap();
    });

    compare("opaque shapes", &mut r, 0, |r| {
        r.set_draw_color(0, 0, 0, 255);
        r.clear().unwrap();
        r.set_draw_color(200, 100, 50, 255);
        r.render_fill_rect(Some(&FRect::new(4.0, 5.0, 20.0, 10.0)))
            .unwrap();
        r.set_draw_color(10, 220, 30, 255);
        r.render_fill_rects(&[
            FRect::new(30.0, 2.0, 7.0, 9.0),
            FRect::new(40.0, 30.0, 12.0, 3.0),
        ])
        .unwrap();
        r.set_draw_color(255, 255, 255, 255);
        r.render_points(&[
            FPoint { x: 1.0, y: 1.0 },
            FPoint { x: 62.0, y: 46.0 },
            FPoint { x: 33.0, y: 20.0 },
        ])
        .unwrap();
        r.set_draw_color(90, 80, 250, 255);
        r.render_line(2.0, 40.0, 60.0, 40.0).unwrap();
        r.render_line(50.0, 2.0, 50.0, 44.0).unwrap();
        r.render_line(5.0, 20.0, 25.0, 40.0).unwrap();
        r.render_rect(Some(&FRect::new(8.0, 24.0, 14.0, 10.0)))
            .unwrap();
    });

    compare("viewport and clip", &mut r, 0, |r| {
        r.set_draw_color(0, 0, 0, 255);
        r.clear().unwrap();
        r.set_viewport(Some(&Rect::new(10, 6, 40, 30))).unwrap();
        r.set_clip_rect(Some(&Rect::new(5, 5, 20, 12))).unwrap();
        r.set_draw_color(250, 250, 0, 255);
        r.render_fill_rect(None).unwrap();
        r.set_clip_rect(None).unwrap();
        r.set_draw_color(0, 250, 250, 255);
        r.render_fill_rect(Some(&FRect::new(30.0, 20.0, 20.0, 20.0)))
            .unwrap();
        r.set_viewport(None).unwrap();
    });

    for (mode, tol) in [
        (BlendMode::BLEND, 2),
        (BlendMode::BLEND_PREMULTIPLIED, 2),
        (BlendMode::ADD, 2),
        (BlendMode::ADD_PREMULTIPLIED, 2),
        (BlendMode::MOD, 2),
        (BlendMode::MUL, 4),
    ] {
        // (the GPU blends in floats, the software renderer in integers;
        // its MUL fills drift furthest)
        compare(&format!("blended fill {mode:?}"), &mut r, tol, |r| {
            r.set_draw_blend_mode(BlendMode::NONE).unwrap();
            r.set_draw_color(40, 80, 120, 200);
            r.clear().unwrap();
            r.set_draw_blend_mode(mode).unwrap();
            r.set_draw_color(220, 60, 30, 128);
            r.render_fill_rect(Some(&FRect::new(6.0, 6.0, 40.0, 30.0)))
                .unwrap();
            r.set_draw_color(20, 200, 90, 77);
            r.render_fill_rect(Some(&FRect::new(20.0, 16.0, 40.0, 30.0)))
                .unwrap();
            r.set_draw_blend_mode(BlendMode::NONE).unwrap();
        });
    }

    // Untextured geometry with per-vertex colors (the same color at each
    // corner, so that the interpolation is exact).
    compare("geometry", &mut r, 0, |r| {
        r.set_draw_color(0, 0, 0, 255);
        r.clear().unwrap();
        let c = FColor::new(1.0, 0.5, 0.25, 1.0);
        let v = |x: f32, y: f32| Vertex {
            position: FPoint { x, y },
            color: c,
            tex_coord: FPoint::default(),
        };
        r.render_geometry(
            None,
            &[v(8.0, 8.0), v(40.0, 8.0), v(40.0, 40.0), v(8.0, 40.0)],
            Some(&[0, 1, 2, 0, 2, 3]),
        )
        .unwrap();
    });

    // Many draws in one frame: past the 256 vertex buffers, which issues
    // a batch on the way.
    compare("many flushes", &mut r, 0, |r| {
        r.set_draw_color(0, 0, 0, 255);
        r.clear().unwrap();
        for i in 0..300 {
            r.set_draw_color((i % 256) as u8, 100, 200, 255);
            r.render_point((i % W) as f32, (i / W) as f32).unwrap();
            r.flush().unwrap();
        }
    });
}

#[test]
fn vulkan_matches_software_for_textures() {
    let _l = crate::test_support::test_lock();
    let _video = Video::init();
    let Some((_window, mut r)) = vulkan_renderer(W, H) else {
        return;
    };
    let mut sw = software_renderer(W, H);

    for format in [
        PixelFormat::ARGB8888,
        PixelFormat::ABGR8888,
        PixelFormat::XRGB8888, // (through a native texture)
        PixelFormat::RGB565,
        PixelFormat::RGBA4444,
    ] {
        let tv = pattern_texture(&mut r, format, 16, 12);
        let ts = pattern_texture(&mut sw, format, 16, 12);
        for mode in [
            BlendMode::NONE,
            BlendMode::BLEND,
            BlendMode::ADD,
            BlendMode::MOD,
            BlendMode::MUL,
        ] {
            let scene = |r: &mut Renderer, t: Texture| {
                r.set_draw_color(30, 60, 90, 255);
                r.clear().unwrap();
                r.set_texture_blend_mode(t, mode).unwrap();
                r.set_texture_scale_mode(t, ScaleMode::Nearest).unwrap();
                r.set_texture_color_mod(t, 255, 255, 255).unwrap();
                r.set_texture_alpha_mod(t, 255).unwrap();
                r.render_texture(t, None, Some(&FRect::new(2.0, 3.0, 16.0, 12.0)))
                    .unwrap();
                // a part, scaled up 2x
                r.render_texture(
                    t,
                    Some(&FRect::new(4.0, 2.0, 8.0, 6.0)),
                    Some(&FRect::new(24.0, 4.0, 16.0, 12.0)),
                )
                .unwrap();
                // with color and alpha modulation
                r.set_texture_color_mod(t, 255, 128, 64).unwrap();
                r.set_texture_alpha_mod(t, 160).unwrap();
                r.render_texture(t, None, Some(&FRect::new(44.0, 30.0, 16.0, 12.0)))
                    .unwrap();
                // flipped (the software renderer's rotated copies with
                // BlendMode::NONE multiply the colors by alpha, as upstream's
                // do: SW_RenderCopyEx() adds the colors from a surface that
                // still has the alpha channel)
                if mode == BlendMode::NONE {
                    return;
                }
                r.set_texture_color_mod(t, 255, 255, 255).unwrap();
                r.set_texture_alpha_mod(t, 255).unwrap();
                r.render_texture_rotated(
                    t,
                    None,
                    Some(&FRect::new(4.0, 30.0, 16.0, 12.0)),
                    0.0,
                    None,
                    FlipMode::Horizontal,
                )
                .unwrap();
            };
            scene(&mut r, tv);
            scene(&mut sw, ts);
            let what = format!("{} texture, {mode:?}", format.name());
            // (modulated colors round differently: the GPU multiplies floats)
            let tol = if mode == BlendMode::NONE { 1 } else { 3 };
            assert_close(
                &what,
                &r.read_pixels(None).unwrap(),
                &sw.read_pixels(None).unwrap(),
                tol,
            );
        }
        r.destroy_texture(tv);
        sw.destroy_texture(ts);
    }

    // A palettized texture, nearest and (in the shader) linear
    let mut palette = Palette::new(256).unwrap();
    let colors: Vec<Color> = (0..256)
        .map(|i| Color::new(i as u8, 255 - i as u8, (i * 7) as u8, 255))
        .collect();
    palette.set_colors(0, &colors).unwrap();
    let palette = share_palette(palette);
    let indices: Vec<u8> = (0..16 * 12).map(|i| (i * 5) as u8).collect();
    let make = |r: &mut Renderer| {
        let t = r
            .create_texture(PixelFormat::INDEX8, TextureAccess::Static, 16, 12)
            .unwrap();
        r.set_texture_palette(t, Some(palette.clone())).unwrap();
        r.update_texture(t, None, &indices, 16).unwrap();
        r.set_texture_scale_mode(t, ScaleMode::Nearest).unwrap();
        r.set_draw_color(0, 0, 0, 255);
        r.clear().unwrap();
        r.render_texture(t, None, Some(&FRect::new(5.0, 5.0, 32.0, 24.0)))
            .unwrap();
        t
    };
    let tv = make(&mut r);
    let ts = make(&mut sw);
    assert_close(
        "palette texture",
        &r.read_pixels(None).unwrap(),
        &sw.read_pixels(None).unwrap(),
        0,
    );
    // An updated palette is used
    let mut palette2 = Palette::new(256).unwrap();
    palette2
        .set_colors(0, &[Color::new(200, 10, 20, 255); 256])
        .unwrap();
    {
        let shared = share_palette(palette2);
        r.set_texture_palette(tv, Some(shared.clone())).unwrap();
        sw.set_texture_palette(ts, Some(shared)).unwrap();
    }
    for rr in [&mut r, &mut sw] {
        let t = if rr.name() == "vulkan" { tv } else { ts };
        rr.set_draw_color(0, 0, 0, 255);
        rr.clear().unwrap();
        rr.render_texture(t, None, Some(&FRect::new(5.0, 5.0, 16.0, 12.0)))
            .unwrap();
    }
    assert_close(
        "new palette",
        &r.read_pixels(None).unwrap(),
        &sw.read_pixels(None).unwrap(),
        0,
    );
    r.destroy_texture(tv);
    sw.destroy_texture(ts);

    // A streaming texture, written through a lock
    let make = |r: &mut Renderer| {
        let t = r
            .create_texture(PixelFormat::ABGR8888, TextureAccess::Streaming, 10, 10)
            .unwrap();
        {
            let mut lock = r.lock_texture(t, Some(&Rect::new(2, 2, 6, 6))).unwrap();
            let pitch = lock.pitch() as usize;
            let pixels = lock.pixels();
            for y in 0..6 {
                for x in 0..6 {
                    pixels[y * pitch + x * 4..][..4].copy_from_slice(&[
                        40 * x as u8,
                        40 * y as u8,
                        200,
                        255,
                    ]);
                }
            }
        }
        r.set_texture_blend_mode(t, BlendMode::NONE).unwrap();
        r.set_texture_scale_mode(t, ScaleMode::Nearest).unwrap();
        r.set_draw_color(0, 0, 0, 255);
        r.clear().unwrap();
        r.render_texture(t, None, Some(&FRect::new(10.0, 10.0, 20.0, 20.0)))
            .unwrap();
        t
    };
    let tv = make(&mut r);
    let ts = make(&mut sw);
    let vk_streaming = r.read_pixels(Some(&Rect::new(10, 10, 20, 20))).unwrap();
    let sw_streaming = sw.read_pixels(Some(&Rect::new(10, 10, 20, 20))).unwrap();
    // (the unlocked parts of a new texture are undefined: compare the
    // locked part)
    let crop = |s: &Surface<'_>| {
        let mut c = Surface::new(12, 12, s.format()).unwrap();
        let mut src = s.duplicate().unwrap();
        src.set_blend_mode(BlendMode::NONE).unwrap();
        src.blit(Some(&Rect::new(4, 4, 12, 12)), &mut c, None)
            .unwrap();
        c
    };
    assert_close(
        "streaming texture",
        &crop(&vk_streaming),
        &crop(&sw_streaming),
        0,
    );
    // (a second lock while locked is refused: the front end unlocks
    // first, so it's the backend's own check)
    r.destroy_texture(tv);
    sw.destroy_texture(ts);

    // Linear scaling and pixel art go through other shaders.
    let tv = pattern_texture(&mut r, PixelFormat::ARGB8888, 16, 12);
    let ts = pattern_texture(&mut sw, PixelFormat::ARGB8888, 16, 12);
    for mode in [ScaleMode::Linear, ScaleMode::PixelArt] {
        for (rr, t) in [(&mut r, tv), (&mut sw, ts)] {
            rr.set_texture_scale_mode(t, mode).unwrap();
            rr.set_texture_blend_mode(t, BlendMode::NONE).unwrap();
            rr.set_draw_color(0, 0, 0, 255);
            rr.clear().unwrap();
            // (1:1, where the filters make no difference)
            rr.render_texture(t, None, Some(&FRect::new(3.0, 4.0, 16.0, 12.0)))
                .unwrap();
        }
        assert_close(
            &format!("{mode:?} 1:1 copy"),
            &r.read_pixels(None).unwrap(),
            &sw.read_pixels(None).unwrap(),
            1,
        );
    }
}

#[test]
fn vulkan_matches_software_for_targets() {
    let _l = crate::test_support::test_lock();
    let _video = Video::init();
    let Some((_window, mut r)) = vulkan_renderer(W, H) else {
        return;
    };
    let mut sw = software_renderer(W, H);

    for format in [
        PixelFormat::ARGB8888,
        PixelFormat::ABGR8888,
        PixelFormat::XRGB8888,
    ] {
        let scene = |r: &mut Renderer, src: Texture| -> (Surface<'static>, Surface<'static>) {
            let target = r
                .create_texture(format, TextureAccess::Target, 32, 24)
                .unwrap();
            r.set_texture_scale_mode(target, ScaleMode::Nearest)
                .unwrap();
            if format == PixelFormat::XRGB8888 {
                // (Vulkan has no XRGB8888 textures: the target's native
                // texture is ARGB8888, created blending, as upstream's is,
                // so it would blend with the alpha drawn into it)
                r.set_texture_blend_mode(target, BlendMode::NONE).unwrap();
            }
            r.set_render_target(Some(target)).unwrap();
            r.set_draw_color(250, 40, 10, 255);
            r.clear().unwrap();
            r.set_draw_color(10, 40, 250, 255);
            r.render_fill_rect(Some(&FRect::new(2.0, 3.0, 10.0, 6.0)))
                .unwrap();
            r.render_point(30.0, 1.0).unwrap();
            r.set_texture_blend_mode(src, BlendMode::NONE).unwrap();
            r.set_texture_scale_mode(src, ScaleMode::Nearest).unwrap();
            r.render_texture(src, None, Some(&FRect::new(14.0, 10.0, 16.0, 12.0)))
                .unwrap();
            let read = r.read_pixels(Some(&Rect::new(1, 2, 30, 20))).unwrap();
            r.set_render_target(None).unwrap();
            r.set_draw_color(0, 0, 0, 255);
            r.clear().unwrap();
            r.render_texture(target, None, Some(&FRect::new(4.0, 4.0, 32.0, 24.0)))
                .unwrap();
            r.render_texture(target, None, Some(&FRect::new(36.0, 20.0, 16.0, 12.0)))
                .unwrap();
            let out = r.read_pixels(None).unwrap();
            r.destroy_texture(target);
            (read, out)
        };
        let tv = pattern_texture(&mut r, PixelFormat::ABGR8888, 16, 12);
        let ts = pattern_texture(&mut sw, PixelFormat::ABGR8888, 16, 12);
        let (vk_read, vk_out) = scene(&mut r, tv);
        let (sw_read, sw_out) = scene(&mut sw, ts);
        let what = format!("{} target", format.name());
        assert_close(&format!("{what} read"), &vk_read, &sw_read, 0);
        // (the target is drawn blended, rounding differently; the
        // window's alpha of the XRGB8888 target is its native texture's)
        if format == PixelFormat::XRGB8888 {
            let opaque = |s: &Surface<'_>| s.convert(PixelFormat::XRGB8888).unwrap();
            assert_close(&what, &opaque(&vk_out), &opaque(&sw_out), 2);
        } else {
            assert_close(&what, &vk_out, &sw_out, 2);
        }
        r.destroy_texture(tv);
        sw.destroy_texture(ts);
    }

    // A target that's destroyed while it's the target
    let target = r
        .create_texture(PixelFormat::ARGB8888, TextureAccess::Target, 8, 8)
        .unwrap();
    r.set_render_target(Some(target)).unwrap();
    r.clear().unwrap();
    r.destroy_texture(target);
    assert_eq!(r.render_target(), None);
    r.set_draw_color(1, 2, 3, 255);
    r.clear().unwrap();
    assert_eq!(
        rows(&r.read_pixels(Some(&Rect::new(0, 0, 1, 1))).unwrap()),
        [3, 2, 1, 255]
    );
}

/// Planes of a test YUV image. The colors stay inside the RGB gamut: the
/// software conversion (`yuv_rgb_std.c`) only clamps inputs in
/// [-128, 384), as upstream's does.
fn yuv_planes(w: usize, h: usize) -> (Vec<u8>, Vec<u8>, Vec<u8>) {
    let y: Vec<u8> = (0..w * h)
        .map(|i| (60 + (i % w) * 100 / w + (i / w) * 20 / h) as u8)
        .collect();
    let (cw, ch) = (w.div_ceil(2), h.div_ceil(2));
    let u: Vec<u8> = (0..cw * ch)
        .map(|i| (110 + (i % cw) * 36 / cw) as u8)
        .collect();
    let v: Vec<u8> = (0..cw * ch)
        .map(|i| (146 - (i / cw) * 36 / ch) as u8)
        .collect();
    (y, u, v)
}

#[test]
fn vulkan_matches_software_for_yuv() {
    let _l = crate::test_support::test_lock();
    let _video = Video::init();
    let Some((_window, mut r)) = vulkan_renderer(W, H) else {
        return;
    };
    let mut sw = software_renderer(W, H);
    let (w, h) = (16usize, 12usize);
    let (y, u, v) = yuv_planes(w, h);
    let uv: Vec<u8> = u.iter().zip(&v).flat_map(|(a, b)| [*a, *b]).collect();
    let vu: Vec<u8> = u.iter().zip(&v).flat_map(|(a, b)| [*b, *a]).collect();

    for format in [
        PixelFormat::IYUV,
        PixelFormat::YV12,
        PixelFormat::NV12,
        PixelFormat::NV21,
    ] {
        for access in [TextureAccess::Static, TextureAccess::Streaming] {
            let scene = |r: &mut Renderer| {
                let t = r
                    .create_texture(format, access, w as i32, h as i32)
                    .unwrap();
                r.set_texture_scale_mode(t, ScaleMode::Nearest).unwrap();
                match format {
                    PixelFormat::NV12 => r
                        .update_nv_texture(t, None, &y, w as i32, &uv, w as i32)
                        .unwrap(),
                    PixelFormat::NV21 => r
                        .update_nv_texture(t, None, &y, w as i32, &vu, w as i32)
                        .unwrap(),
                    _ => r
                        .update_yuv_texture(
                            t,
                            None,
                            &y,
                            w as i32,
                            &u,
                            w as i32 / 2,
                            &v,
                            w as i32 / 2,
                        )
                        .unwrap(),
                }
                r.set_draw_color(0, 0, 0, 255);
                r.clear().unwrap();
                r.render_texture(t, None, Some(&FRect::new(3.0, 3.0, 32.0, 24.0)))
                    .unwrap();
                r.destroy_texture(t);
                r.read_pixels(None).unwrap()
            };
            let what = format!("{} {access:?} texture", format.name());
            // (the shaders convert in floats, the software path in fixed point)
            assert_close(&what, &scene(&mut r), &scene(&mut sw), 3);
        }
    }

    // Whole-texture updates with all the planes in one buffer, and a
    // streaming texture written through a lock.
    let mut all = y.clone();
    all.extend_from_slice(&u);
    all.extend_from_slice(&v);
    let mut nv = y.clone();
    nv.extend_from_slice(&uv);
    for (format, data) in [(PixelFormat::IYUV, &all), (PixelFormat::NV12, &nv)] {
        let scene = |r: &mut Renderer, access: TextureAccess| {
            let t = r
                .create_texture(format, access, w as i32, h as i32)
                .unwrap();
            r.set_texture_scale_mode(t, ScaleMode::Nearest).unwrap();
            if access == TextureAccess::Streaming {
                let mut lock = r.lock_texture(t, None).unwrap();
                assert_eq!(lock.pitch(), w as i32);
                lock.pixels()[..data.len()].copy_from_slice(data);
            } else {
                r.update_texture(t, None, data, w as i32).unwrap();
            }
            r.set_draw_color(0, 0, 0, 255);
            r.clear().unwrap();
            r.render_texture(t, None, Some(&FRect::new(3.0, 3.0, 32.0, 24.0)))
                .unwrap();
            r.destroy_texture(t);
            r.read_pixels(None).unwrap()
        };
        for access in [TextureAccess::Static, TextureAccess::Streaming] {
            assert_close(
                &format!("{} {access:?} update", format.name()),
                &scene(&mut r, access),
                &scene(&mut sw, access),
                3,
            );
        }
    }
    // (a buffer without the chroma planes is refused, not read past its end)
    let t = r
        .create_texture(PixelFormat::IYUV, TextureAccess::Static, w as i32, h as i32)
        .unwrap();
    assert!(r.update_texture(t, None, &y, w as i32).is_err());
}

#[test]
fn vulkan_line_methods() {
    let _l = crate::test_support::test_lock();
    let _video = Video::init();
    // Vulkan lines (SDL_RENDER_LINE_METHOD 2), with the end points drawn
    // as points, hit the same pixels as the software renderer's for
    // straight lines.
    hints::set(hints::RENDER_LINE_METHOD, "2").unwrap();
    let created = vulkan_renderer(W, H);
    hints::reset(hints::RENDER_LINE_METHOD);
    let Some((_window, mut r)) = created else {
        return;
    };
    let scene = |r: &mut Renderer| {
        r.set_draw_color(0, 0, 0, 255);
        r.clear().unwrap();
        r.set_draw_color(255, 128, 0, 255);
        r.render_line(2.0, 4.0, 60.0, 4.0).unwrap();
        r.render_line(7.0, 10.0, 7.0, 40.0).unwrap();
        r.render_lines(&[
            FPoint { x: 20.0, y: 20.0 },
            FPoint { x: 40.0, y: 20.0 },
            FPoint { x: 40.0, y: 30.0 },
        ])
        .unwrap();
        r.read_pixels(None).unwrap()
    };
    let vk = scene(&mut r);
    let sw = scene(&mut software_renderer(W, H));
    assert_close("lines", &vk, &sw, 0);
}

#[test]
fn vulkan_fails_without_vulkan_surfaces() {
    let _l = crate::test_support::test_lock();
    // The dummy driver has no Vulkan: the renderer can't be created, and
    // the default renderer is the software one.
    init::quit();
    hints::set(hints::VIDEO_DRIVER, "dummy").unwrap();
    init::init(InitFlags::VIDEO).unwrap();
    let window = Window::create("no vulkan", 16, 16, WindowFlags::default()).unwrap();
    assert!(Renderer::for_window(&window, Some(VULKAN_RENDERER)).is_err());
    assert!(!window.flags().unwrap().contains(WindowFlags::VULKAN));
    let r = Renderer::for_window(&window, None).unwrap();
    assert_eq!(r.name(), SOFTWARE_RENDERER);
    drop(r);
    window.destroy();
    init::quit();
    hints::reset(hints::VIDEO_DRIVER);
}
