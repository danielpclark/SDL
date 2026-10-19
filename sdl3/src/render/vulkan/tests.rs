// Tests of the Vulkan renderer.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! The tables are checked against upstream's values. The scenes are drawn
//! by the Vulkan renderer on a real device, through the offscreen driver's
//! headless surface (Mesa's lavapipe on Linux CI) or, on Windows, the
//! Windows video driver's Win32 surfaces (the GPU drivers there have no
//! `VK_EXT_headless_surface`), and by the software renderer, and the
//! pixels compared: exactly where the math is exact (clears, fills,
//! points, 1:1 copies, palettes), within a small tolerance where the GPU's
//! float blending or YUV conversion rounds differently. The renderer is
//! also made on Vulkan objects the test makes itself, with the validation
//! layer (when it's installed) and a messenger that fails the test on its
//! errors, drawing with images the test made and filled. Without a Vulkan
//! loader and driver (with `VK_EXT_headless_surface` on the offscreen
//! driver) the device tests report a skip (capability `vulkan`) and pass.

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
#[cfg(not(any(windows, target_os = "macos")))] // (direct3d11 and metal come first there: see their tests)
fn driver_list_follows_upstreams_order() {
    use crate::render::{num_render_drivers, render_driver};
    // (render_drivers[] in SDL_render.c: ..., GLES2, ..., VULKAN, GPU, SW)
    assert_eq!(num_render_drivers(), 5);
    assert_eq!(render_driver(0).unwrap(), "opengl");
    assert_eq!(render_driver(1).unwrap(), "opengles2");
    assert_eq!(render_driver(2).unwrap(), "vulkan");
    assert_eq!(render_driver(3).unwrap(), "gpu");
    assert_eq!(render_driver(4).unwrap(), SOFTWARE_RENDERER);
    assert!(render_driver(5).is_err());
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

/// Video up on the offscreen driver (the Windows one on Windows, whose
/// Vulkan drivers have no `VK_EXT_headless_surface`); quit when dropped.
struct Video;

impl Video {
    fn init() -> Video {
        init::quit();
        let driver = if cfg!(windows) {
            "windows"
        } else {
            "offscreen"
        };
        hints::set(hints::VIDEO_DRIVER, driver).unwrap();
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

/// The Vulkan renderer's backend itself for an offscreen window, or
/// `None` (with the skip reported) when there is no Vulkan.
fn vulkan_backend(w: i32, h: i32) -> Option<(Window, VulkanRenderer)> {
    let window = Window::create("vulkan", w, h, WindowFlags::VULKAN).unwrap();
    match VulkanRenderer::for_window(window, Colorspace::SRGB, &Default::default()) {
        Ok(data) => Some((window, data)),
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

/// The device functions of `data`, with `name` replaced by `f` (the
/// tests' fault injection).
fn device_functions_with(
    data: &VulkanRenderer,
    name: &CStr,
    f: PfnVkVoidFunction,
) -> DeviceFunctions {
    let inst = data.inst();
    let device = data.device;
    // SAFETY: the instance's vkGetDeviceProcAddr returns the functions of
    // the names; the caller's replacement has the type of its name's.
    unsafe {
        DeviceFunctions::load(|n| {
            if n == name {
                Some(f)
            } else {
                (inst.get_device_proc_addr)(device, n.as_ptr())
            }
        })
    }
    .unwrap()
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
    // corner, so that the interpolation is exact; 0.5 is 127.5 in UNORM, a
    // tie lavapipe rounds up like the software renderer and GPUs such as
    // NVIDIA's round down, so the channels are allowed to differ by 1).
    compare("geometry", &mut r, 1, |r| {
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
        // (VK_EXT_4444_formats' where the device has it, converted where
        // not)
        PixelFormat::ARGB4444,
        PixelFormat::ABGR4444,
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
                .create_texture(format, TextureAccess::Target, 24, 18)
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
            r.render_point(22.0, 1.0).unwrap();
            r.set_texture_blend_mode(src, BlendMode::NONE).unwrap();
            r.set_texture_scale_mode(src, ScaleMode::Nearest).unwrap();
            r.render_texture(src, None, Some(&FRect::new(6.0, 5.0, 16.0, 12.0)))
                .unwrap();
            let read = r.read_pixels(Some(&Rect::new(1, 2, 22, 15))).unwrap();
            r.set_render_target(None).unwrap();
            r.set_draw_color(0, 0, 0, 255);
            r.clear().unwrap();
            // 1:1, and scaled down 3:2 (whose pixel centers stay off the
            // texel edges; at an exact 2:1 nearest sampling is a tie
            // rasterizers break differently)
            r.render_texture(target, None, Some(&FRect::new(4.0, 4.0, 24.0, 18.0)))
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

    r.destroy_texture(t);

    // More draws with new constants than a constant buffer holds: those
    // past the end of the first buffer go to the next (upstream checks only
    // the offset, so with a small minUniformBufferOffsetAlignment the last
    // constants of a buffer, with their YCbCr matrix, go past its end; see
    // the Note in set_draw_state())
    let scene = |r: &mut Renderer| {
        let iyuv = r
            .create_texture(PixelFormat::IYUV, TextureAccess::Static, 2, 2)
            .unwrap();
        r.update_yuv_texture(iyuv, None, &[200; 4], 2, &[90], 1, &[170], 1)
            .unwrap();
        let nv12 = r
            .create_texture(PixelFormat::NV12, TextureAccess::Static, 2, 2)
            .unwrap();
        r.update_nv_texture(nv12, None, &[120; 4], 2, &[160, 80], 2)
            .unwrap();
        r.set_draw_color(0, 0, 0, 255);
        r.clear().unwrap();
        // (the texture types alternate, so each draw has new constants)
        for i in 0..(W * H) {
            let t = if i % 2 == 0 { iyuv } else { nv12 };
            r.set_texture_scale_mode(t, ScaleMode::Nearest).unwrap();
            let (x, y) = ((i % W) as f32, (i / W) as f32);
            r.render_texture(t, None, Some(&FRect::new(x, y, 1.0, 1.0)))
                .unwrap();
        }
        let pixels = r.read_pixels(None).unwrap();
        r.destroy_texture(iyuv);
        r.destroy_texture(nv12);
        pixels
    };
    assert_close("constant buffers", &scene(&mut r), &scene(&mut sw), 3);
}

/// A descriptor pool that can't be made fails the swapchain's resources
/// (upstream's goes on with a null pool).
#[test]
fn vulkan_fails_without_its_descriptor_pools() {
    unsafe extern "system" fn create_descriptor_pool(
        _: VkDevice,
        _: *const VkDescriptorPoolCreateInfo,
        _: Alloc,
        _: *mut VkDescriptorPool,
    ) -> VkResult {
        VK_ERROR_OUT_OF_DEVICE_MEMORY
    }

    let _l = crate::test_support::test_lock();
    let _video = Video::init();
    let Some((window, mut data)) = vulkan_backend(W, H) else {
        return;
    };
    type PfnVkCreateDescriptorPool = unsafe extern "system" fn(
        VkDevice,
        *const VkDescriptorPoolCreateInfo,
        Alloc,
        *mut VkDescriptorPool,
    ) -> VkResult;
    // SAFETY: a function pointer, called as the type it has.
    let failing = unsafe {
        std::mem::transmute::<PfnVkCreateDescriptorPool, PfnVkVoidFunction>(create_descriptor_pool)
    };
    let dev = device_functions_with(&data, c"vkCreateDescriptorPool", failing);
    data.wait_for_gpu();
    data.dev = Some(Rc::new(dev));
    let (rc, e) = data.create_window_size_dependent_resources().unwrap_err();
    assert_eq!(rc, VK_ERROR_OUT_OF_DEVICE_MEMORY);
    assert_eq!(
        e.message(),
        "vkCreateDescrptorPool(): VK_ERROR_OUT_OF_DEVICE_MEMORY"
    );
    // (the renderer is torn down)
    assert!(data.dev.is_none() && data.descriptor_pools.is_empty());
    data.destroy();
    drop(data);
    window.destroy();
}

/// The renderer's reference to the Vulkan loader is released when it's
/// destroyed, and a device reset doesn't take another (upstream's is never
/// released).
#[test]
fn vulkan_releases_the_loader() {
    let _l = crate::test_support::test_lock();
    let _video = Video::init();
    let loaded = || crate::video::core::with_device(|v| v.vulkan_loader_loaded).unwrap();
    let window = Window::create("vulkan", W, H, WindowFlags::VULKAN).unwrap();
    let before = loaded();
    let mut data = match VulkanRenderer::for_window(window, Colorspace::SRGB, &Default::default()) {
        Ok(data) => data,
        Err(e) => {
            crate::test_support::skip(
                "vulkan",
                format_args!("no Vulkan renderer ({})", e.message()),
            );
            // (a renderer that couldn't be made holds no reference)
            assert_eq!(loaded(), before);
            window.destroy();
            return;
        }
    };
    assert_eq!(loaded(), before + 1);
    assert!(data.handle_device_lost());
    let _ = crate::events::queue::get_events(EventType::FIRST, EventType::LAST, 100);
    assert_eq!(loaded(), before + 1);
    data.destroy();
    assert_eq!(loaded(), before);
    drop(data);
    window.destroy();
}

/// After a device loss, a palette the front end keeps draws with its
/// colors on the new device (upstream's images are the lost device's).
#[test]
fn vulkan_palettes_survive_a_device_loss() {
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

    type PfnVkQueueSubmit =
        unsafe extern "system" fn(VkQueue, u32, *const VkSubmitInfo, VkFence) -> VkResult;
    type PfnVkQueueWaitIdle = unsafe extern "system" fn(VkQueue) -> VkResult;
    static REAL_SUBMIT: AtomicUsize = AtomicUsize::new(0);
    static WAIT_IDLE: AtomicUsize = AtomicUsize::new(0);
    static LOSE: AtomicBool = AtomicBool::new(false);
    /// vkQueueSubmit, which reports a lost device once when LOSE is set
    /// (after the queue's work is done, so the renderer's objects are idle).
    unsafe extern "system" fn queue_submit(
        queue: VkQueue,
        count: u32,
        submits: *const VkSubmitInfo,
        fence: VkFence,
    ) -> VkResult {
        // SAFETY: the real functions, stored below.
        unsafe {
            if LOSE.swap(false, Ordering::SeqCst) {
                let wait: PfnVkQueueWaitIdle =
                    std::mem::transmute(WAIT_IDLE.load(Ordering::SeqCst));
                wait(queue);
                return VK_ERROR_DEVICE_LOST;
            }
            let submit: PfnVkQueueSubmit = std::mem::transmute(REAL_SUBMIT.load(Ordering::SeqCst));
            submit(queue, count, submits, fence)
        }
    }

    let _l = crate::test_support::test_lock();
    let _video = Video::init();
    let Some((window, mut data)) = vulkan_backend(W, H) else {
        return;
    };
    REAL_SUBMIT.store(data.dev().queue_submit as usize, Ordering::SeqCst);
    WAIT_IDLE.store(data.inst().queue_wait_idle as usize, Ordering::SeqCst);
    // SAFETY: a function pointer, called as the type it has.
    let failing =
        unsafe { std::mem::transmute::<PfnVkQueueSubmit, PfnVkVoidFunction>(queue_submit) };
    let dev = device_functions_with(&data, c"vkQueueSubmit", failing);
    data.dev = Some(Rc::new(dev));
    let mut r = Renderer::finish_create(
        Box::new(data),
        PixelFormat::UNKNOWN,
        (0, 0),
        &Default::default(),
        0,
        Some(window),
    )
    .unwrap();
    let mut sw = software_renderer(W, H);

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
    let lost = make(&mut r);
    let ts = make(&mut sw);
    let expected = sw.read_pixels(None).unwrap();
    assert_close(
        "palette texture",
        &r.read_pixels(None).unwrap(),
        &expected,
        0,
    );

    // The device is lost at the present, and the renderer recovers.
    r.clear().unwrap();
    r.flush().unwrap();
    LOSE.store(true, Ordering::SeqCst);
    // (SDL_RenderPresent() goes on without the error)
    r.present().unwrap();
    assert!(!LOSE.load(Ordering::SeqCst));
    let events = crate::events::queue::get_events(EventType::FIRST, EventType::LAST, 100).unwrap();
    assert!(events
        .iter()
        .any(|e| matches!(e, Event::Render(e) if e.event_type == EventType::RENDER_DEVICE_RESET)));

    // A texture made again (while the lost one keeps the palette the
    // front end shares) draws with the palette's colors.
    let t = make(&mut r);
    assert_close(
        "palette texture after a device loss",
        &r.read_pixels(None).unwrap(),
        &expected,
        0,
    );
    r.destroy_texture(t);
    r.destroy_texture(lost);
    sw.destroy_texture(ts);
    drop(r);
    window.destroy();
}

/// The renderer's own device is made with VK_EXT_4444_formats when the
/// physical device has it, and its formats are offered only then.
#[test]
fn vulkan_4444_formats_with_their_extension() {
    let _l = crate::test_support::test_lock();
    let _video = Video::init();
    let Some((window, mut data)) = vulkan_backend(W, H) else {
        return;
    };
    let formats = data.texture_formats.clone();
    println!("VK_EXT_4444_formats: {}", data.supports_ext_4444_formats);
    for format in [PixelFormat::ARGB4444, PixelFormat::ABGR4444] {
        if !data.supports_ext_4444_formats {
            assert!(!formats.contains(&format), "{format:?}");
        }
    }
    data.destroy();
    drop(data);
    window.destroy();
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

// ---------------------------------------------------------------------------
// On the application's Vulkan objects
// ---------------------------------------------------------------------------

/// `VkDebugUtilsMessengerCreateInfoEXT`
#[repr(C)]
struct DebugUtilsMessengerCreateInfo {
    s_type: VkStructureType,
    p_next: *const c_void,
    flags: VkFlags,
    message_severity: VkFlags,
    message_type: VkFlags,
    pfn_user_callback: DebugUtilsMessengerCallback,
    p_user_data: *mut c_void,
}

/// The start of `VkDebugUtilsMessengerCallbackDataEXT`, up to `pMessage`.
#[repr(C)]
struct DebugUtilsMessengerCallbackData {
    s_type: VkStructureType,
    p_next: *const c_void,
    flags: VkFlags,
    p_message_id_name: *const c_char,
    message_id_number: i32,
    p_message: *const c_char,
}

type DebugUtilsMessengerCallback = unsafe extern "system" fn(
    VkFlags,
    VkFlags,
    *const DebugUtilsMessengerCallbackData,
    *mut c_void,
) -> VkBool32;

const VK_STRUCTURE_TYPE_DEBUG_UTILS_MESSENGER_CREATE_INFO_EXT: VkStructureType = 1000128004;
const VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_SAMPLER_YCBCR_CONVERSION_FEATURES_: VkStructureType =
    1000156004;
const VK_DEBUG_UTILS_MESSAGE_SEVERITY_WARNING_BIT_EXT: VkFlags = 0x100;
const VK_DEBUG_UTILS_MESSAGE_SEVERITY_ERROR_BIT_EXT: VkFlags = 0x1000;
const VK_DEBUG_UTILS_MESSAGE_TYPE_VALIDATION_BIT_EXT: VkFlags = 0x2;

/// The validation layer's errors, while a test's instance has its
/// messenger.
static VALIDATION_ERRORS: std::sync::Mutex<Vec<String>> = std::sync::Mutex::new(Vec::new());

/// The messenger's callback: the validation errors are kept.
unsafe extern "system" fn debug_callback(
    severity: VkFlags,
    _types: VkFlags,
    data: *const DebugUtilsMessengerCallbackData,
    _user: *mut c_void,
) -> VkBool32 {
    // SAFETY: the layer's callback data, with a NUL-terminated message.
    let message = unsafe { CStr::from_ptr((*data).p_message) }
        .to_string_lossy()
        .into_owned();
    eprintln!("validation: {message}");
    if severity & VK_DEBUG_UTILS_MESSAGE_SEVERITY_ERROR_BIT_EXT != 0 {
        VALIDATION_ERRORS
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .push(message);
    }
    VK_FALSE
}

/// A Vulkan instance, surface and device of the test's own, as an
/// application makes them for the renderer, with the validation layer
/// (when it's installed) reporting to [`VALIDATION_ERRORS`].
struct AppVulkan {
    inst: InstanceFunctions,
    dev: DeviceFunctions,
    instance: VkInstance,
    messenger: u64,
    destroy_messenger: Option<unsafe extern "system" fn(VkInstance, u64, Alloc)>,
    surface: VkSurfaceKHR,
    physical_device: VkPhysicalDevice,
    memory_properties: VkPhysicalDeviceMemoryProperties,
    graphics_family: u32,
    present_family: u32,
    device: VkDevice,
    queue: VkQueue,
    command_pool: VkCommandPool,
    /// The device has the sampler YCbCr conversion (as the renderer's
    /// would): it takes NV12 textures.
    ycbcr: bool,
}

impl AppVulkan {
    /// The objects for `window`, or `None` (with the skip reported) without
    /// Vulkan.
    fn new(window: &Window) -> Option<AppVulkan> {
        let skip = |what: &str| {
            crate::test_support::skip("vulkan", format_args!("no Vulkan ({what})"));
            None
        };
        if crate::video::vulkan::vulkan_load_library(None).is_err() {
            return skip("no loader");
        }
        let gipa = match crate::video::vulkan::vulkan_get_vk_get_instance_proc_addr() {
            Ok(f) if f != 0 => f,
            _ => return skip("no vkGetInstanceProcAddr"),
        };
        // SAFETY: the loader's vkGetInstanceProcAddr.
        let gipa: PfnVkGetInstanceProcAddr = unsafe { std::mem::transmute(gipa) };
        let lookup = |instance: VkInstance| {
            move |name: &CStr| {
                // SAFETY: a null or live instance and a NUL-terminated name.
                unsafe { gipa(instance, name.as_ptr()) }
            }
        };
        // SAFETY: vkGetInstanceProcAddr returns the functions of the names.
        let global = unsafe { GlobalFunctions::load(lookup(null_mut())) }.ok()?;

        // The validation layer, if it's installed, and its messages.
        let mut count = 0;
        // SAFETY: a null array queries the count.
        unsafe { (global.enumerate_instance_layer_properties)(&mut count, null_mut()) };
        let mut layers = vec![VkLayerProperties::default(); count as usize];
        // SAFETY: the array has room for the count.
        unsafe { (global.enumerate_instance_layer_properties)(&mut count, layers.as_mut_ptr()) };
        let validation = layers.iter().any(|l| {
            // SAFETY: a NUL-terminated name in the array.
            (unsafe { CStr::from_ptr(l.layer_name.as_ptr()) }) == SDL_VULKAN_VALIDATION_LAYER_NAME
        });
        let mut count = 0;
        // SAFETY: as above.
        unsafe { (global.enumerate_instance_extension_properties)(null(), &mut count, null_mut()) };
        let mut extensions = vec![VkExtensionProperties::default(); count as usize];
        // SAFETY: as above.
        unsafe {
            (global.enumerate_instance_extension_properties)(
                null(),
                &mut count,
                extensions.as_mut_ptr(),
            )
        };
        let has = |name: &CStr| {
            extensions.iter().any(|e| {
                // SAFETY: as above.
                (unsafe { CStr::from_ptr(e.extension_name.as_ptr()) }) == name
            })
        };
        let debug_utils = validation && has(c"VK_EXT_debug_utils");

        // (the video driver's surface extensions, and the one the renderer
        // looks for)
        let mut names: Vec<CString> = crate::video::vulkan::vulkan_instance_extensions()
            .unwrap_or_default()
            .iter()
            .map(|e| CString::new(*e).unwrap())
            .collect();
        if has(VK_KHR_GET_PHYSICAL_DEVICE_PROPERTIES_2_EXTENSION_NAME) {
            names.push(VK_KHR_GET_PHYSICAL_DEVICE_PROPERTIES_2_EXTENSION_NAME.to_owned());
        }
        if debug_utils {
            names.push(c"VK_EXT_debug_utils".to_owned());
        }
        let name_ptrs: Vec<*const c_char> = names.iter().map(|n| n.as_ptr()).collect();
        let layer = [SDL_VULKAN_VALIDATION_LAYER_NAME.as_ptr()];
        let app_info = VkApplicationInfo {
            s_type: VK_STRUCTURE_TYPE_APPLICATION_INFO,
            api_version: VK_API_VERSION_1_0,
            ..Default::default()
        };
        let create_info = VkInstanceCreateInfo {
            s_type: VK_STRUCTURE_TYPE_INSTANCE_CREATE_INFO,
            p_application_info: &app_info,
            enabled_layer_count: validation as u32,
            pp_enabled_layer_names: layer.as_ptr(),
            enabled_extension_count: name_ptrs.len() as u32,
            pp_enabled_extension_names: name_ptrs.as_ptr(),
            ..Default::default()
        };
        let mut instance = null_mut();
        // SAFETY: the create info and its names outlive the call.
        let result = unsafe { (global.create_instance)(&create_info, null(), &mut instance) };
        if result != VK_SUCCESS {
            return skip("vkCreateInstance() failed");
        }
        // SAFETY: as above, with the instance.
        let inst = unsafe { InstanceFunctions::load(lookup(instance)) }.unwrap();

        let (mut messenger, mut destroy_messenger) = (0, None);
        if debug_utils {
            type Create = unsafe extern "system" fn(
                VkInstance,
                *const DebugUtilsMessengerCreateInfo,
                Alloc,
                *mut u64,
            ) -> VkResult;
            type Destroy = unsafe extern "system" fn(VkInstance, u64, Alloc);
            // SAFETY: the functions of these names have these types.
            let (create, destroy) = unsafe {
                (
                    std::mem::transmute::<PfnVkVoidFunction, Create>(
                        lookup(instance)(c"vkCreateDebugUtilsMessengerEXT").unwrap(),
                    ),
                    std::mem::transmute::<PfnVkVoidFunction, Destroy>(
                        lookup(instance)(c"vkDestroyDebugUtilsMessengerEXT").unwrap(),
                    ),
                )
            };
            let info = DebugUtilsMessengerCreateInfo {
                s_type: VK_STRUCTURE_TYPE_DEBUG_UTILS_MESSENGER_CREATE_INFO_EXT,
                p_next: null(),
                flags: 0,
                message_severity: VK_DEBUG_UTILS_MESSAGE_SEVERITY_WARNING_BIT_EXT
                    | VK_DEBUG_UTILS_MESSAGE_SEVERITY_ERROR_BIT_EXT,
                message_type: VK_DEBUG_UTILS_MESSAGE_TYPE_VALIDATION_BIT_EXT,
                pfn_user_callback: debug_callback,
                p_user_data: null_mut(),
            };
            // SAFETY: a valid create info.
            assert_eq!(
                unsafe { create(instance, &info, null(), &mut messenger) },
                VK_SUCCESS
            );
            destroy_messenger = Some(destroy);
        }
        VALIDATION_ERRORS
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clear();

        let surface =
            match crate::video::vulkan::vulkan_create_surface(window, instance as usize, 0) {
                Ok(surface) => surface,
                Err(e) => {
                    // SAFETY: the instance and messenger just made, unused.
                    unsafe {
                        if let Some(destroy) = destroy_messenger {
                            destroy(instance, messenger, null());
                        }
                        (inst.destroy_instance)(instance, null());
                    }
                    return skip(e.message());
                }
            };

        // The first physical device with a queue family that renders and
        // presents to the surface.
        let mut count = 0;
        // SAFETY: a null array queries the count.
        unsafe { (inst.enumerate_physical_devices)(instance, &mut count, null_mut()) };
        let mut physical_devices = vec![null_mut(); count as usize];
        // SAFETY: the array has room for the count.
        unsafe {
            (inst.enumerate_physical_devices)(instance, &mut count, physical_devices.as_mut_ptr())
        };
        let mut found = None;
        for &pd in &physical_devices {
            let mut n = 0;
            // SAFETY: as above.
            unsafe { (inst.get_physical_device_queue_family_properties)(pd, &mut n, null_mut()) };
            let mut families = vec![VkQueueFamilyProperties::default(); n as usize];
            // SAFETY: as above.
            unsafe {
                (inst.get_physical_device_queue_family_properties)(
                    pd,
                    &mut n,
                    families.as_mut_ptr(),
                )
            };
            for (i, family) in families.iter().enumerate() {
                let mut present = VK_FALSE;
                // SAFETY: a family of the device, the instance's surface.
                unsafe {
                    (inst.get_physical_device_surface_support_khr)(
                        pd,
                        i as u32,
                        surface,
                        &mut present,
                    )
                };
                if family.queue_flags & VK_QUEUE_GRAPHICS_BIT != 0 && present != VK_FALSE {
                    found = Some((pd, i as u32));
                    break;
                }
            }
            if found.is_some() {
                break;
            }
        }
        let Some((physical_device, family)) = found else {
            panic!("no physical device that renders to the surface");
        };
        let mut memory_properties = VkPhysicalDeviceMemoryProperties::default();
        // SAFETY: as above.
        unsafe {
            (inst.get_physical_device_memory_properties)(physical_device, &mut memory_properties)
        };

        // The device, with the extensions the renderer's would have.
        let mut count = 0;
        // SAFETY: as above.
        unsafe {
            (inst.enumerate_device_extension_properties)(
                physical_device,
                null(),
                &mut count,
                null_mut(),
            )
        };
        let mut device_extensions = vec![VkExtensionProperties::default(); count as usize];
        // SAFETY: as above.
        unsafe {
            (inst.enumerate_device_extension_properties)(
                physical_device,
                null(),
                &mut count,
                device_extensions.as_mut_ptr(),
            )
        };
        let device_has = |name: &CStr| {
            device_extensions.iter().any(|e| {
                // SAFETY: as above.
                (unsafe { CStr::from_ptr(e.extension_name.as_ptr()) }) == name
            })
        };
        let ycbcr_extensions = [
            c"VK_KHR_sampler_ycbcr_conversion",
            c"VK_KHR_maintenance1",
            c"VK_KHR_maintenance2",
            c"VK_KHR_bind_memory2",
            c"VK_KHR_get_memory_requirements2",
        ];
        let ycbcr = has(VK_KHR_GET_PHYSICAL_DEVICE_PROPERTIES_2_EXTENSION_NAME)
            && ycbcr_extensions.iter().all(|e| device_has(e));
        let mut device_extension_ptrs = vec![VK_KHR_SWAPCHAIN_EXTENSION_NAME.as_ptr()];
        if ycbcr {
            device_extension_ptrs.extend(ycbcr_extensions.iter().map(|e| e.as_ptr()));
        }
        let priority = [1.0f32];
        let queue_info = VkDeviceQueueCreateInfo {
            s_type: VK_STRUCTURE_TYPE_DEVICE_QUEUE_CREATE_INFO,
            queue_family_index: family,
            queue_count: 1,
            p_queue_priorities: priority.as_ptr(),
            ..Default::default()
        };
        let ycbcr_features = VkPhysicalDeviceSamplerYcbcrConversionFeatures {
            s_type: VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_SAMPLER_YCBCR_CONVERSION_FEATURES_,
            sampler_ycbcr_conversion: VK_TRUE,
            ..Default::default()
        };
        let device_info = VkDeviceCreateInfo {
            s_type: VK_STRUCTURE_TYPE_DEVICE_CREATE_INFO,
            p_next: if ycbcr {
                &ycbcr_features as *const _ as *const c_void
            } else {
                null()
            },
            queue_create_info_count: 1,
            p_queue_create_infos: &queue_info,
            enabled_extension_count: device_extension_ptrs.len() as u32,
            pp_enabled_extension_names: device_extension_ptrs.as_ptr(),
            ..Default::default()
        };
        let mut device = null_mut();
        // SAFETY: the create info and what it points to outlive the call.
        let result =
            unsafe { (inst.create_device)(physical_device, &device_info, null(), &mut device) };
        assert_eq!(result, VK_SUCCESS, "vkCreateDevice()");
        let get_device_proc_addr = inst.get_device_proc_addr;
        // SAFETY: vkGetDeviceProcAddr returns the functions of the names.
        let dev = unsafe {
            DeviceFunctions::load(|name: &CStr| get_device_proc_addr(device, name.as_ptr()))
        }
        .unwrap();
        let mut queue = null_mut();
        // SAFETY: the queue the device was made with.
        unsafe { (dev.get_device_queue)(device, family, 0, &mut queue) };
        let pool_info = VkCommandPoolCreateInfo {
            s_type: VK_STRUCTURE_TYPE_COMMAND_POOL_CREATE_INFO,
            queue_family_index: family,
            ..Default::default()
        };
        let mut command_pool = VK_NULL_HANDLE;
        // SAFETY: a valid create info.
        unsafe { (dev.create_command_pool)(device, &pool_info, null(), &mut command_pool) };

        Some(AppVulkan {
            inst,
            dev,
            instance,
            messenger,
            destroy_messenger,
            surface,
            physical_device,
            memory_properties,
            graphics_family: family,
            present_family: family,
            device,
            queue,
            command_pool,
            ycbcr,
        })
    }

    /// The renderer's creation options for these objects.
    fn create_info(&self) -> RendererCreateInfo {
        // SAFETY: the test's live objects, which outlive the renderers made
        // with them.
        unsafe {
            RendererCreateInfo {
                name: Some(VULKAN_RENDERER.to_owned()),
                vulkan_instance: NativeObject::from_ptr(self.instance),
                vulkan_surface: NativeObject::from_raw(self.surface),
                vulkan_physical_device: NativeObject::from_ptr(self.physical_device),
                vulkan_device: NativeObject::from_ptr(self.device),
                vulkan_graphics_queue_family_index: Some(self.graphics_family),
                vulkan_present_queue_family_index: Some(self.present_family),
                ..RendererCreateInfo::default()
            }
        }
    }

    /// A memory type of `bits` with `flags`.
    fn memory_type(&self, bits: u32, flags: VkMemoryPropertyFlags) -> u32 {
        let props = &self.memory_properties;
        (0..props.memory_type_count)
            .find(|&i| {
                bits & (1 << i) != 0
                    && props.memory_types[i as usize].property_flags & flags == flags
            })
            .expect("a memory type")
    }

    /// An image as the renderer makes its own, with its memory.
    fn image(
        &self,
        format: VkFormat,
        (w, h): (u32, u32),
        usage: VkImageUsageFlags,
    ) -> (VkImage, VkDeviceMemory) {
        let info = VkImageCreateInfo {
            s_type: VK_STRUCTURE_TYPE_IMAGE_CREATE_INFO,
            flags: VK_IMAGE_CREATE_MUTABLE_FORMAT_BIT,
            image_type: VK_IMAGE_TYPE_2D,
            format,
            extent: VkExtent3D {
                width: w,
                height: h,
                depth: 1,
            },
            mip_levels: 1,
            array_layers: 1,
            samples: VK_SAMPLE_COUNT_1_BIT,
            tiling: VK_IMAGE_TILING_OPTIMAL,
            usage,
            sharing_mode: VK_SHARING_MODE_EXCLUSIVE,
            initial_layout: VK_IMAGE_LAYOUT_UNDEFINED,
            ..Default::default()
        };
        let mut image = VK_NULL_HANDLE;
        let mut requirements = VkMemoryRequirements::default();
        let mut memory = VK_NULL_HANDLE;
        // SAFETY: valid create and allocate infos; the image is bound to
        // its memory.
        unsafe {
            assert_eq!(
                (self.dev.create_image)(self.device, &info, null(), &mut image),
                VK_SUCCESS
            );
            (self.dev.get_image_memory_requirements)(self.device, image, &mut requirements);
            let allocate = VkMemoryAllocateInfo {
                s_type: VK_STRUCTURE_TYPE_MEMORY_ALLOCATE_INFO,
                allocation_size: requirements.size,
                memory_type_index: self.memory_type(
                    requirements.memory_type_bits,
                    VK_MEMORY_PROPERTY_DEVICE_LOCAL_BIT,
                ),
                ..Default::default()
            };
            assert_eq!(
                (self.dev.allocate_memory)(self.device, &allocate, null(), &mut memory),
                VK_SUCCESS
            );
            assert_eq!(
                (self.dev.bind_image_memory)(self.device, image, memory, 0),
                VK_SUCCESS
            );
        }
        (image, memory)
    }

    /// Upload the planes of `image` (each `(aspect, width, height, bytes per
    /// pixel, data)`), leaving it in `VK_IMAGE_LAYOUT_SHADER_READ_ONLY_OPTIMAL`,
    /// and wait for the queue.
    fn upload(&self, image: VkImage, planes: &[(VkImageAspectFlags, u32, u32, u32, &[u8])]) {
        let size: usize = planes.iter().map(|p| p.4.len()).sum();
        let buffer_info = VkBufferCreateInfo {
            s_type: VK_STRUCTURE_TYPE_BUFFER_CREATE_INFO,
            size: size as VkDeviceSize,
            usage: VK_BUFFER_USAGE_TRANSFER_SRC_BIT,
            sharing_mode: VK_SHARING_MODE_EXCLUSIVE,
            ..Default::default()
        };
        let aspects = planes.iter().fold(0, |a, p| a | p.0);
        let range = VkImageSubresourceRange {
            aspect_mask: aspects,
            base_mip_level: 0,
            level_count: 1,
            base_array_layer: 0,
            layer_count: 1,
        };
        // SAFETY: the buffer, its memory and the command buffer are made,
        // used and released here; the copies stay in the buffer and image.
        unsafe {
            let mut buffer = VK_NULL_HANDLE;
            assert_eq!(
                (self.dev.create_buffer)(self.device, &buffer_info, null(), &mut buffer),
                VK_SUCCESS
            );
            let mut requirements = VkMemoryRequirements::default();
            (self.dev.get_buffer_memory_requirements)(self.device, buffer, &mut requirements);
            let allocate = VkMemoryAllocateInfo {
                s_type: VK_STRUCTURE_TYPE_MEMORY_ALLOCATE_INFO,
                allocation_size: requirements.size,
                memory_type_index: self.memory_type(
                    requirements.memory_type_bits,
                    VK_MEMORY_PROPERTY_HOST_VISIBLE_BIT | VK_MEMORY_PROPERTY_HOST_COHERENT_BIT,
                ),
                ..Default::default()
            };
            let mut memory = VK_NULL_HANDLE;
            assert_eq!(
                (self.dev.allocate_memory)(self.device, &allocate, null(), &mut memory),
                VK_SUCCESS
            );
            (self.dev.bind_buffer_memory)(self.device, buffer, memory, 0);
            let mut mapped = null_mut();
            (self.dev.map_memory)(self.device, memory, 0, size as VkDeviceSize, 0, &mut mapped);
            let mut offset = 0;
            let mut regions = Vec::new();
            for &(aspect, w, h, bpp, data) in planes {
                std::ptr::copy_nonoverlapping(
                    data.as_ptr(),
                    (mapped as *mut u8).add(offset),
                    data.len(),
                );
                assert_eq!(data.len(), (w * h * bpp) as usize);
                regions.push(VkBufferImageCopy {
                    buffer_offset: offset as VkDeviceSize,
                    buffer_row_length: 0,
                    buffer_image_height: 0,
                    image_subresource: VkImageSubresourceLayers {
                        aspect_mask: aspect,
                        mip_level: 0,
                        base_array_layer: 0,
                        layer_count: 1,
                    },
                    image_offset: VkOffset3D::default(),
                    image_extent: VkExtent3D {
                        width: w,
                        height: h,
                        depth: 1,
                    },
                });
                offset += data.len();
            }
            (self.dev.unmap_memory)(self.device, memory);

            let allocate = VkCommandBufferAllocateInfo {
                s_type: VK_STRUCTURE_TYPE_COMMAND_BUFFER_ALLOCATE_INFO,
                command_pool: self.command_pool,
                level: VK_COMMAND_BUFFER_LEVEL_PRIMARY,
                command_buffer_count: 1,
                ..Default::default()
            };
            let mut cb = null_mut();
            (self.dev.allocate_command_buffers)(self.device, &allocate, &mut cb);
            let begin = VkCommandBufferBeginInfo {
                s_type: VK_STRUCTURE_TYPE_COMMAND_BUFFER_BEGIN_INFO,
                flags: VK_COMMAND_BUFFER_USAGE_ONE_TIME_SUBMIT_BIT,
                ..Default::default()
            };
            (self.dev.begin_command_buffer)(cb, &begin);
            let barrier = |old, new, src, dst| VkImageMemoryBarrier {
                s_type: VK_STRUCTURE_TYPE_IMAGE_MEMORY_BARRIER,
                src_access_mask: src,
                dst_access_mask: dst,
                old_layout: old,
                new_layout: new,
                src_queue_family_index: VK_QUEUE_FAMILY_IGNORED,
                dst_queue_family_index: VK_QUEUE_FAMILY_IGNORED,
                image,
                subresource_range: range,
                ..Default::default()
            };
            let to_transfer = barrier(
                VK_IMAGE_LAYOUT_UNDEFINED,
                VK_IMAGE_LAYOUT_TRANSFER_DST_OPTIMAL,
                0,
                VK_ACCESS_TRANSFER_WRITE_BIT,
            );
            (self.dev.cmd_pipeline_barrier)(
                cb,
                VK_PIPELINE_STAGE_TOP_OF_PIPE_BIT,
                VK_PIPELINE_STAGE_TRANSFER_BIT,
                0,
                0,
                null(),
                0,
                null(),
                1,
                &to_transfer,
            );
            (self.dev.cmd_copy_buffer_to_image)(
                cb,
                buffer,
                image,
                VK_IMAGE_LAYOUT_TRANSFER_DST_OPTIMAL,
                regions.len() as u32,
                regions.as_ptr(),
            );
            let to_shader = barrier(
                VK_IMAGE_LAYOUT_TRANSFER_DST_OPTIMAL,
                VK_IMAGE_LAYOUT_SHADER_READ_ONLY_OPTIMAL,
                VK_ACCESS_TRANSFER_WRITE_BIT,
                VK_ACCESS_SHADER_READ_BIT,
            );
            (self.dev.cmd_pipeline_barrier)(
                cb,
                VK_PIPELINE_STAGE_TRANSFER_BIT,
                VK_PIPELINE_STAGE_FRAGMENT_SHADER_BIT,
                0,
                0,
                null(),
                0,
                null(),
                1,
                &to_shader,
            );
            (self.dev.end_command_buffer)(cb);
            let submit = VkSubmitInfo {
                s_type: VK_STRUCTURE_TYPE_SUBMIT_INFO,
                command_buffer_count: 1,
                p_command_buffers: &cb,
                ..Default::default()
            };
            assert_eq!(
                (self.dev.queue_submit)(self.queue, 1, &submit, VK_NULL_HANDLE),
                VK_SUCCESS
            );
            (self.inst.queue_wait_idle)(self.queue);
            (self.dev.free_command_buffers)(self.device, self.command_pool, 1, &cb);
            (self.dev.destroy_buffer)(self.device, buffer, null());
            (self.dev.free_memory)(self.device, memory, null());
        }
    }

    /// Release an image of [`AppVulkan::image`], once the device is idle.
    fn destroy_image(&self, (image, memory): (VkImage, VkDeviceMemory)) {
        // SAFETY: the test's image and memory, no longer in use.
        unsafe {
            (self.dev.device_wait_idle)(self.device);
            (self.dev.destroy_image)(self.device, image, null());
            (self.dev.free_memory)(self.device, memory, null());
        }
    }

    /// The validation errors so far.
    fn errors(&self) -> Vec<String> {
        VALIDATION_ERRORS
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }
}

impl Drop for AppVulkan {
    fn drop(&mut self) {
        // SAFETY: the test's objects, which nothing uses any more (the
        // renderers are gone).
        unsafe {
            (self.dev.device_wait_idle)(self.device);
            (self.dev.destroy_command_pool)(self.device, self.command_pool, null());
            (self.dev.destroy_device)(self.device, null());
            (self.inst.destroy_surface_khr)(self.instance, self.surface, null());
            if let Some(destroy) = self.destroy_messenger {
                destroy(self.instance, self.messenger, null());
            }
            (self.inst.destroy_instance)(self.instance, null());
        }
    }
}

/// The Vulkan renderer on the application's instance, surface, physical
/// device, device and queue family draws as the software renderer does,
/// publishes them, wraps the application's images (filled by the
/// application, or drawn into as targets) and leaves them all to the
/// application; with no validation error.
#[test]
fn vulkan_renderer_on_existing_vulkan_objects() {
    let _l = crate::test_support::test_lock();
    let _video = Video::init();
    // (a Vulkan window loads the Vulkan loader, which may not be there)
    let window = match Window::create("vulkan", W, H, WindowFlags::VULKAN) {
        Ok(window) => window,
        Err(e) => {
            crate::test_support::skip("vulkan", format_args!("no Vulkan window ({})", e.message()));
            return;
        }
    };
    let Some(app) = AppVulkan::new(&window) else {
        window.destroy();
        return;
    };
    let mut r = Renderer::for_window_with(&window, &app.create_info()).unwrap();
    assert_eq!(r.name(), VULKAN_RENDERER);
    let props = r.properties();
    assert_eq!(
        props.get_number(PROP_RENDERER_VULKAN_INSTANCE_POINTER),
        Some(app.instance as i64)
    );
    assert_eq!(
        props.get_number(PROP_RENDERER_VULKAN_SURFACE_NUMBER),
        Some(app.surface as i64)
    );
    assert_eq!(
        props.get_number(PROP_RENDERER_VULKAN_PHYSICAL_DEVICE_POINTER),
        Some(app.physical_device as i64)
    );
    assert_eq!(
        props.get_number(PROP_RENDERER_VULKAN_DEVICE_POINTER),
        Some(app.device as i64)
    );
    assert_eq!(
        props.get_number(PROP_RENDERER_VULKAN_GRAPHICS_QUEUE_FAMILY_INDEX_NUMBER),
        Some(app.graphics_family as i64)
    );

    let scene = |r: &mut Renderer| {
        r.set_draw_color(12, 34, 56, 255);
        r.clear().unwrap();
        r.set_draw_color(200, 100, 50, 255);
        r.render_fill_rect(Some(&FRect::new(4.0, 5.0, 20.0, 10.0)))
            .unwrap();
        r.render_line(1.0, 40.0, 60.0, 30.0).unwrap();
        let t = pattern_texture(r, PixelFormat::ARGB8888, 16, 12);
        r.set_texture_blend_mode(t, BlendMode::NONE).unwrap();
        r.render_texture(t, None, Some(&FRect::new(30.0, 20.0, 16.0, 12.0)))
            .unwrap();
        r.destroy_texture(t);
    };
    compare("existing Vulkan objects", &mut r, 1, scene);
    r.present().unwrap();

    // A static texture of the application's image, filled by it.
    let (w, h) = (16u32, 12u32);
    let usage = VK_IMAGE_USAGE_SAMPLED_BIT
        | VK_IMAGE_USAGE_TRANSFER_DST_BIT
        | VK_IMAGE_USAGE_TRANSFER_SRC_BIT;
    let image = app.image(VK_FORMAT_B8G8R8A8_UNORM, (w, h), usage);
    let pattern_rgba = pattern(w as i32, h as i32);
    let bgra: Vec<u8> = pattern_rgba
        .chunks(4)
        .flat_map(|p| [p[2], p[1], p[0], p[3]])
        .collect();
    app.upload(image.0, &[(VK_IMAGE_ASPECT_COLOR_BIT, w, h, 4, &bgra)]);
    // SAFETY: the test's image, of the renderer's device, alive until it's
    // destroyed below.
    let vk_image = unsafe { NativeObject::from_raw(image.0) };
    let t = r
        .create_texture_with(&TextureCreateInfo {
            format: PixelFormat::ARGB8888,
            width: w as i32,
            height: h as i32,
            vulkan_texture: vk_image,
            // (the views take the image's extra usage too)
            vulkan_usage: VK_IMAGE_USAGE_TRANSFER_SRC_BIT,
            ..TextureCreateInfo::default()
        })
        .unwrap();
    assert_eq!(
        r.texture_properties(t)
            .unwrap()
            .get_number(PROP_TEXTURE_VULKAN_TEXTURE_NUMBER),
        Some(image.0 as i64)
    );
    let draw = |r: &mut Renderer, t: Texture| {
        r.set_draw_color(0, 0, 0, 255);
        r.clear().unwrap();
        r.set_texture_blend_mode(t, BlendMode::NONE).unwrap();
        r.set_texture_scale_mode(t, ScaleMode::Nearest).unwrap();
        let (tw, th) = r.texture_size(t).unwrap();
        r.render_texture(t, None, Some(&FRect::new(2.0, 2.0, tw * 2.0, th * 2.0)))
            .unwrap();
        r.read_pixels(None).unwrap()
    };
    let mut sw = software_renderer(W, H);
    let st = pattern_texture(&mut sw, PixelFormat::ARGB8888, w as i32, h as i32);
    assert_close(
        "existing Vulkan image",
        &draw(&mut r, t),
        &draw(&mut sw, st),
        0,
    );
    r.destroy_texture(t);
    sw.destroy_texture(st);
    r.flush().unwrap();

    // A target of the application's image, in VK_IMAGE_LAYOUT_UNDEFINED.
    let (tw, th) = (24u32, 18u32);
    let target_image = app.image(
        VK_FORMAT_B8G8R8A8_UNORM,
        (tw, th),
        VK_IMAGE_USAGE_SAMPLED_BIT
            | VK_IMAGE_USAGE_TRANSFER_DST_BIT
            | VK_IMAGE_USAGE_COLOR_ATTACHMENT_BIT
            | VK_IMAGE_USAGE_TRANSFER_SRC_BIT,
    );
    // SAFETY: as above.
    let vk_target = unsafe { NativeObject::from_raw(target_image.0) };
    let target = r
        .create_texture_with(&TextureCreateInfo {
            format: PixelFormat::ARGB8888,
            access: TextureAccess::Target,
            width: tw as i32,
            height: th as i32,
            vulkan_texture: vk_target,
            vulkan_layout: Some(VK_IMAGE_LAYOUT_UNDEFINED),
            ..TextureCreateInfo::default()
        })
        .unwrap();
    let sw_target = sw
        .create_texture(
            PixelFormat::ARGB8888,
            TextureAccess::Target,
            tw as i32,
            th as i32,
        )
        .unwrap();
    let into_target = |r: &mut Renderer, target: Texture| {
        let src = pattern_texture(r, PixelFormat::ARGB8888, 16, 12);
        r.set_render_target(Some(target)).unwrap();
        r.set_draw_color(250, 40, 10, 255);
        r.clear().unwrap();
        r.set_draw_color(10, 40, 250, 255);
        r.render_fill_rect(Some(&FRect::new(2.0, 3.0, 10.0, 6.0)))
            .unwrap();
        r.render_point(22.0, 1.0).unwrap();
        r.set_texture_blend_mode(src, BlendMode::NONE).unwrap();
        r.set_texture_scale_mode(src, ScaleMode::Nearest).unwrap();
        r.render_texture(src, None, Some(&FRect::new(6.0, 5.0, 16.0, 12.0)))
            .unwrap();
        let read = r.read_pixels(None).unwrap();
        r.set_render_target(None).unwrap();
        r.destroy_texture(src);
        read
    };
    assert_close(
        "existing Vulkan target",
        &into_target(&mut r, target),
        &into_target(&mut sw, sw_target),
        0,
    );
    assert_close(
        "existing Vulkan target drawn",
        &draw(&mut r, target),
        &draw(&mut sw, sw_target),
        0,
    );
    r.destroy_texture(target);
    r.present().unwrap();

    // The application's device has no VK_EXT_4444_formats, so its formats
    // aren't offered (upstream's are, and its query of them is reported).
    for format in [PixelFormat::ARGB4444, PixelFormat::ABGR4444] {
        assert!(!r.texture_formats().contains(&format), "{format:?}");
    }

    // An NV12 texture of the application's two-plane image.
    if app.ycbcr && r.texture_formats().contains(&PixelFormat::NV12) {
        let (y, u, v) = yuv_planes(w as usize, h as usize);
        let uv: Vec<u8> = u.iter().zip(&v).flat_map(|(a, b)| [*a, *b]).collect();
        let nv12 = app.image(
            VK_FORMAT_G8_B8R8_2PLANE_420_UNORM,
            (w, h),
            VK_IMAGE_USAGE_SAMPLED_BIT | VK_IMAGE_USAGE_TRANSFER_DST_BIT,
        );
        app.upload(
            nv12.0,
            &[
                (VK_IMAGE_ASPECT_PLANE_0_BIT, w, h, 1, &y),
                (VK_IMAGE_ASPECT_PLANE_0_BIT << 1, w / 2, h / 2, 2, &uv),
            ],
        );
        // SAFETY: as above.
        let vk_nv12 = unsafe { NativeObject::from_raw(nv12.0) };
        let t = r
            .create_texture_with(&TextureCreateInfo {
                format: PixelFormat::NV12,
                width: w as i32,
                height: h as i32,
                vulkan_texture: vk_nv12,
                ..TextureCreateInfo::default()
            })
            .unwrap();
        let st = sw
            .create_texture(PixelFormat::NV12, TextureAccess::Static, w as i32, h as i32)
            .unwrap();
        sw.update_nv_texture(st, None, &y, w as i32, &uv, w as i32)
            .unwrap();
        // (the shaders convert YUV in floats, the software path in fixed
        // point)
        assert_close(
            "existing Vulkan NV12 image",
            &draw(&mut r, t),
            &draw(&mut sw, st),
            3,
        );
        r.destroy_texture(t);
        r.flush().unwrap();
        app.destroy_image(nv12);
    }

    // The renderer leaves everything to the application.
    drop(r);
    app.destroy_image(image);
    app.destroy_image(target_image);
    let errors = app.errors();
    assert!(errors.is_empty(), "validation errors: {errors:#?}");
    drop(app);
    window.destroy();
}
