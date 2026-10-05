// Tests of the GPU renderer.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! The tables are checked against upstream's values. The scenes are drawn
//! by the GPU renderer on a real device (the GPU API's Vulkan backend,
//! through the offscreen driver's headless surface: Mesa's lavapipe on
//! Linux CI) and by the software renderer, and the pixels compared:
//! exactly where the math is exact (clears, fills, points, 1:1 copies,
//! palettes, render targets read back), within a small tolerance where the
//! GPU's float blending, modulation or YUV conversion rounds differently.
//! The renderer's device is made in debug mode ([`hints::RENDER_GPU_DEBUG`]),
//! so `VK_LAYER_KHRONOS_validation` checks it when it's installed. Without
//! a Vulkan loader and driver with `VK_EXT_headless_surface` the device
//! tests report a skip (capability `vulkan`) and pass.
//!
//! The scaled copies avoid the exact 2:1 ties of nearest sampling (a
//! downscale whose pixel centers fall on texel edges), where rasterizers
//! differ.

use std::sync::Arc;

use super::shaders::{FragmentShaderId, VertexShaderId};
use super::*;
use crate::events::queue::{get_events, pump};
use crate::events::{Event, EventType};
use crate::init::{self, InitFlags};
use crate::render::{
    Renderer, Texture, TextureAccess, TextureCreateInfo, Vertex,
    PROP_RENDERER_MAX_TEXTURE_SIZE_NUMBER, PROP_RENDERER_TEXTURE_WRAPPING_BOOLEAN,
    SOFTWARE_RENDERER,
};
use crate::video::blendmode::{BlendFactor as BF, BlendOperation as BO};
use crate::video::pixels::Palette;
use crate::video::surface::share_palette;

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
    // The `*_len` of upstream's shaders/*.spv.h, and the sources table.
    let vertex = [
        (VertexShaderId::Linepoint, 992),
        (VertexShaderId::TriColor, 1100),
        (VertexShaderId::TriTexture, 1100),
    ];
    for (id, len) in vertex {
        let s = id.sources();
        assert_eq!(s.spirv.len(), len, "{id:?}");
        assert_eq!((s.num_samplers, s.num_uniform_buffers), (0, 1), "{id:?}");
    }
    let fragment = [
        (FragmentShaderId::Color, 796, 0),
        (FragmentShaderId::TextureRgb, 1380, 1),
        (FragmentShaderId::TextureRgba, 1296, 1),
        (FragmentShaderId::TextureAdvanced, 17260, 3),
    ];
    for (id, len, samplers) in fragment {
        let s = id.sources();
        assert_eq!(s.spirv.len(), len, "{id:?}");
        assert_eq!(
            (s.num_samplers, s.num_uniform_buffers),
            (samplers, 1),
            "{id:?}"
        );
    }
    let all = VertexShaderId::ALL
        .iter()
        .map(|id| id.sources())
        .chain(FragmentShaderId::ALL.iter().map(|id| id.sources()));
    for s in all {
        let word = |i: usize| u32::from_le_bytes(s.spirv[4 * i..4 * i + 4].try_into().unwrap());
        // SPIR-V magic, version 1.0, DXC's generator id (Google spiregg)
        assert_eq!(word(0), 0x07230203);
        assert_eq!(word(1), 0x00010000);
        assert_eq!(word(2) >> 16, 14);
        // ... ending with OpReturn, OpFunctionEnd
        let n = s.spirv.len() / 4;
        assert_eq!(s.spirv.len() % 4, 0);
        assert_eq!((word(n - 2), word(n - 1)), (0x000100fd, 0x00010038));
    }
    for (i, id) in VertexShaderId::ALL.iter().enumerate() {
        assert_eq!(*id as usize, i);
    }
    for (i, id) in FragmentShaderId::ALL.iter().enumerate() {
        assert_eq!(*id as usize, i);
    }
    // The first bytes of color_frag_spv after the header (OpCapability Shader)
    let color = FragmentShaderId::Color.sources().spirv;
    assert_eq!(
        &color[20..28],
        &[0x11, 0x00, 0x02, 0x00, 0x01, 0x00, 0x00, 0x00]
    );
}

#[test]
fn blend_conversions() {
    use gpu::{BlendFactor as G, BlendOp as GO};
    let factors = [
        (BF::Zero, G::Zero),
        (BF::One, G::One),
        (BF::SrcColor, G::SrcColor),
        (BF::OneMinusSrcColor, G::OneMinusSrcColor),
        (BF::SrcAlpha, G::SrcAlpha),
        (BF::OneMinusSrcAlpha, G::OneMinusSrcAlpha),
        (BF::DstColor, G::DstColor),
        (BF::OneMinusDstColor, G::OneMinusDstColor),
        (BF::DstAlpha, G::DstAlpha),
        (BF::OneMinusDstAlpha, G::OneMinusDstAlpha),
    ];
    for (sdl, g) in factors {
        assert_eq!(convert_blend_factor(Some(sdl)), Some(g));
    }
    assert_eq!(convert_blend_factor(None), None);
    let ops = [
        (BO::Add, GO::Add),
        (BO::Subtract, GO::Subtract),
        (BO::RevSubtract, GO::ReverseSubtract),
        (BO::Minimum, GO::Min),
        (BO::Maximum, GO::Max),
    ];
    for (sdl, g) in ops {
        assert_eq!(convert_blend_operation(Some(sdl)), Some(g));
    }
    assert_eq!(convert_blend_operation(None), None);

    // (BlendMode::NONE is ONE, ZERO, ADD, with blending off)
    let none = pipeline::blend_state(BlendMode::NONE).unwrap();
    assert!(!none.enable_blend);
    assert_eq!(
        (
            none.src_color_blendfactor,
            none.dst_color_blendfactor,
            none.color_blend_op
        ),
        (G::One, G::Zero, GO::Add)
    );
    let blend = pipeline::blend_state(BlendMode::BLEND).unwrap();
    assert!(blend.enable_blend);
    assert_eq!(blend.color_write_mask, gpu::ColorComponentFlags(0xF));
    assert_eq!(
        (blend.src_color_blendfactor, blend.dst_color_blendfactor),
        (G::SrcAlpha, G::OneMinusSrcAlpha)
    );
    assert_eq!(
        (blend.src_alpha_blendfactor, blend.dst_alpha_blendfactor),
        (G::One, G::OneMinusSrcAlpha)
    );
    let custom = BlendMode::compose_custom(
        BF::DstColor,
        BF::Zero,
        BO::Minimum,
        BF::One,
        BF::OneMinusDstAlpha,
        BO::Maximum,
    );
    let custom = pipeline::blend_state(custom).unwrap();
    assert_eq!(
        (custom.color_blend_op, custom.alpha_blend_op),
        (GO::Min, GO::Max)
    );
    // (an invalid mode has none)
    assert!(pipeline::blend_state(BlendMode(0x0F0F_0F0F)).is_none());
}

#[test]
fn vertex_layouts_and_uniforms() {
    use gpu::VertexElementFormat as F;
    let (desc, attribs) = pipeline::vertex_layout(VertexShaderId::Linepoint);
    assert_eq!(desc.pitch, 24);
    let formats: Vec<_> = attribs
        .iter()
        .map(|a| (a.location, a.format, a.offset))
        .collect();
    assert_eq!(formats, [(0, F::Float2, 0), (1, F::Float4, 8)]);
    for id in [VertexShaderId::TriColor, VertexShaderId::TriTexture] {
        let (desc, attribs) = pipeline::vertex_layout(id);
        assert_eq!(desc.pitch, 32);
        let formats: Vec<_> = attribs
            .iter()
            .map(|a| (a.location, a.format, a.offset))
            .collect();
        assert_eq!(
            formats,
            [(0, F::Float2, 0), (1, F::Float4, 8), (2, F::Float2, 24)]
        );
    }

    // (the C structures' sizes)
    assert_eq!(VertexShaderUniformData::default().bytes().len(), 64);
    assert_eq!(SimpleFragmentShaderUniformData::default().bytes().len(), 4);
    let mut advanced = AdvancedFragmentShaderUniformData {
        texture_type: TEXTURETYPE_NV21,
        sdr_white_point: 80.0,
        ..Default::default()
    };
    advanced.ycbcr_matrix[15] = 2.0;
    let bytes = advanced.bytes();
    assert_eq!(bytes.len(), 112);
    assert_eq!(&bytes[4..8], &9.0f32.to_ne_bytes());
    assert_eq!(&bytes[44..48], &80.0f32.to_ne_bytes());
    assert_eq!(&bytes[108..112], &2.0f32.to_ne_bytes());

    // (the values of texture_advanced.frag.hlsl)
    assert_eq!(
        [
            TEXTURETYPE_RGB,
            TEXTURETYPE_RGB_PIXELART,
            TEXTURETYPE_RGBA,
            TEXTURETYPE_RGBA_PIXELART,
            TEXTURETYPE_PALETTE_NEAREST,
            TEXTURETYPE_PALETTE_LINEAR,
            TEXTURETYPE_PALETTE_PIXELART,
            TEXTURETYPE_NV12,
            TEXTURETYPE_NV21,
            TEXTURETYPE_YUV
        ],
        [1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0, 9.0, 10.0]
    );
    assert_eq!(
        [
            INPUTTYPE_UNSPECIFIED,
            INPUTTYPE_SRGB,
            INPUTTYPE_SCRGB,
            INPUTTYPE_HDR10
        ],
        [0.0, 1.0, 2.0, 3.0]
    );
    assert_eq!((TONEMAP_NONE, TONEMAP_CHROME), (0.0, 2.0));

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

    // The supported formats map to GPU formats.
    for format in SUPPORTED_FORMATS {
        assert!(
            TextureFormat::from_pixel_format(format).is_some(),
            "{format:?}"
        );
    }

    // The device is asked for the shader formats the renderer has: SPIR-V.
    let props = Properties::new();
    fill_supported_shader_formats(&props).unwrap();
    let get = |name| props.get_bool(name);
    assert_eq!(
        get(gpu::PROP_GPU_DEVICE_CREATE_SHADERS_PRIVATE_BOOLEAN),
        Some(false)
    );
    assert_eq!(
        get(gpu::PROP_GPU_DEVICE_CREATE_SHADERS_SPIRV_BOOLEAN),
        Some(true)
    );
    assert_eq!(
        get(gpu::PROP_GPU_DEVICE_CREATE_SHADERS_DXIL_BOOLEAN),
        Some(false)
    );
    assert_eq!(
        get(gpu::PROP_GPU_DEVICE_CREATE_SHADERS_MSL_BOOLEAN),
        Some(false)
    );
}

// ---------------------------------------------------------------------------
// With a device
// ---------------------------------------------------------------------------

/// Video up on the offscreen driver, with the renderer's devices in debug
/// mode (validated); quit when dropped.
struct Video;

impl Video {
    fn init() -> Video {
        init::quit();
        hints::set(hints::VIDEO_DRIVER, "offscreen").unwrap();
        hints::set(hints::RENDER_GPU_DEBUG, "1").unwrap();
        init::init(InitFlags::VIDEO).unwrap();
        Video
    }
}

impl Drop for Video {
    fn drop(&mut self) {
        init::quit();
        hints::reset(hints::RENDER_GPU_DEBUG);
        hints::reset(hints::VIDEO_DRIVER);
    }
}

/// An offscreen window with a GPU renderer, or `None` (with the skip
/// reported) when there is no Vulkan.
fn gpu_renderer(w: i32, h: i32) -> Option<(Window, Renderer)> {
    let window = Window::create("gpu", w, h, WindowFlags::default()).unwrap();
    match Renderer::for_window(&window, Some(GPU_RENDERER)) {
        Ok(r) => Some((window, r)),
        Err(e) => {
            crate::test_support::skip("vulkan", format_args!("no GPU renderer ({})", e.message()));
            window.destroy();
            None
        }
    }
}

/// A software renderer drawing into an ARGB8888 surface (the format the
/// GPU renderer reads its B8G8R8A8 backbuffer back in).
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
fn differences(a: &Surface<'_>, b: &Surface<'_>, tol: u8) -> Vec<Difference> {
    let (pa, pb) = (rows(a), rows(b));
    let bpp = a.format().bytes_per_pixel() as usize;
    let w = a.width() as usize;
    // (the padding byte of XRGB8888 and XBGR8888 isn't compared)
    let padding = matches!(a.format(), PixelFormat::XRGB8888 | PixelFormat::XBGR8888).then_some(3);
    pa.chunks(bpp)
        .zip(pb.chunks(bpp))
        .enumerate()
        .filter(|(_, (x, y))| {
            x.iter()
                .zip(*y)
                .enumerate()
                .any(|(c, (p, q))| Some(c) != padding && p.abs_diff(*q) > tol)
        })
        .map(|(i, (x, y))| (((i % w) as i32, (i / w) as i32), x.to_vec(), y.to_vec()))
        .collect()
}

/// Compare two images, each channel within `tol` (the software image is
/// converted to the GPU image's format first).
fn assert_close(what: &str, gpu: &Surface<'_>, sw: &Surface<'_>, tol: u8) {
    let converted;
    let sw = if sw.format() != gpu.format() {
        converted = sw.convert(gpu.format()).unwrap();
        &converted
    } else {
        sw
    };
    assert_eq!(
        (gpu.width(), gpu.height()),
        (sw.width(), sw.height()),
        "{what}: sizes"
    );
    let bad = differences(gpu, sw, tol);
    if let Some(((x, y), pa, pb)) = bad.first() {
        panic!(
            "{what}: {} pixels differ by more than {tol}; first at ({x}, {y}): gpu {pa:?} sw {pb:?}",
            bad.len()
        );
    }
}

/// Draw `scene` with the GPU renderer and the software renderer, and
/// compare the results (`tol` per channel).
fn compare(what: &str, r: &mut Renderer, tol: u8, scene: impl Fn(&mut Renderer)) {
    scene(r);
    let gpu = r.read_pixels(None).unwrap();
    let mut sw = software_renderer(W, H);
    scene(&mut sw);
    let sw = sw.read_pixels(None).unwrap();
    assert_close(what, &gpu, &sw, tol);
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

/// A static texture of `format` with `pixels` (RGBA32, converted).
fn texture_from(r: &mut Renderer, format: PixelFormat, w: i32, h: i32, pixels: &[u8]) -> Texture {
    let mut src = Surface::new(w, h, PixelFormat::RGBA32).unwrap();
    let pitch = src.pitch() as usize;
    let dst = src.pixels_mut().unwrap();
    for y in 0..h as usize {
        dst[y * pitch..][..w as usize * 4]
            .copy_from_slice(&pixels[y * w as usize * 4..][..w as usize * 4]);
    }
    let converted = src.convert(format).unwrap();
    let t = r
        .create_texture(format, TextureAccess::Static, w, h)
        .unwrap();
    r.update_texture(t, None, converted.raw_pixels().unwrap(), converted.pitch())
        .unwrap();
    t
}

/// A static texture of `format` with [`pattern`]'s pixels.
fn pattern_texture(r: &mut Renderer, format: PixelFormat, w: i32, h: i32) -> Texture {
    texture_from(r, format, w, h, &pattern(w, h))
}

#[test]
fn gpu_renderer_basics() {
    let _l = crate::test_support::test_lock();
    let _video = Video::init();
    let Some((window, mut r)) = gpu_renderer(W, H) else {
        return;
    };
    assert_eq!(r.name(), "gpu");
    // (the GPU API's Vulkan backend made the window a Vulkan one)
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
    let device = props
        .get_any::<gpu::Device>(PROP_RENDERER_GPU_DEVICE_POINTER)
        .unwrap();
    assert_eq!(device.driver(), "vulkan");
    assert!(device.shader_formats().contains(ShaderFormat::SPIRV));

    let formats = r.texture_formats();
    assert_eq!(
        &formats[..4],
        &[
            PixelFormat::BGRA32,
            PixelFormat::RGBA32,
            PixelFormat::BGRX32,
            PixelFormat::RGBX32
        ]
    );
    for f in [
        PixelFormat::RGB565,
        PixelFormat::INDEX8,
        PixelFormat::IYUV,
        PixelFormat::YV12,
        PixelFormat::I444,
        PixelFormat::NV12,
        PixelFormat::NV21,
        PixelFormat::P010,
    ] {
        assert!(formats.contains(&f), "{f:?} supported");
    }

    // The GPU textures are published as properties.
    let t = r
        .create_texture(PixelFormat::ARGB8888, TextureAccess::Static, 8, 8)
        .unwrap();
    let tp = r.texture_properties(t).unwrap();
    let main = tp
        .get_any::<gpu::Texture>(PROP_TEXTURE_GPU_TEXTURE_POINTER)
        .unwrap();
    assert_eq!(main.info.format, TextureFormat::B8G8R8A8_UNORM);
    assert_eq!((main.info.width, main.info.height), (8, 8));
    assert!(tp
        .get_any::<gpu::Texture>(PROP_TEXTURE_GPU_TEXTURE_U_POINTER)
        .is_none());
    let yuv = r
        .create_texture(PixelFormat::IYUV, TextureAccess::Static, 9, 7)
        .unwrap();
    let yp = r.texture_properties(yuv).unwrap();
    let y = yp
        .get_any::<gpu::Texture>(PROP_TEXTURE_GPU_TEXTURE_POINTER)
        .unwrap();
    let u = yp
        .get_any::<gpu::Texture>(PROP_TEXTURE_GPU_TEXTURE_U_POINTER)
        .unwrap();
    let v = yp
        .get_any::<gpu::Texture>(PROP_TEXTURE_GPU_TEXTURE_V_POINTER)
        .unwrap();
    assert_eq!(y.info.format, TextureFormat::R8_UNORM);
    assert_eq!((u.info.width, u.info.height), (5, 4));
    assert!(!Arc::ptr_eq(&u, &v));
    // (FIXME (upstream): P010's default colorspace, HDR10, has no YCbCr
    // matrix, so a P010 texture needs a YCbCr colorspace)
    assert!(r
        .create_texture(PixelFormat::P010, TextureAccess::Static, 8, 8)
        .is_err());
    let nv = r
        .create_texture_with(&TextureCreateInfo {
            format: PixelFormat::P010,
            width: 8,
            height: 8,
            colorspace: Some(Colorspace::BT2020_LIMITED),
            ..Default::default()
        })
        .unwrap();
    let np = r.texture_properties(nv).unwrap();
    let uv = np
        .get_any::<gpu::Texture>(PROP_TEXTURE_GPU_TEXTURE_UV_POINTER)
        .unwrap();
    assert_eq!(uv.info.format, TextureFormat::R16G16_UNORM);
    assert_eq!((uv.info.width, uv.info.height), (4, 4));
    // (FIXME (upstream): an I444 texture's V property is its U texture)
    let i444 = r
        .create_texture(PixelFormat::I444, TextureAccess::Static, 8, 8)
        .unwrap();
    let ip = r.texture_properties(i444).unwrap();
    let u = ip
        .get_any::<gpu::Texture>(PROP_TEXTURE_GPU_TEXTURE_U_POINTER)
        .unwrap();
    let v = ip
        .get_any::<gpu::Texture>(PROP_TEXTURE_GPU_TEXTURE_V_POINTER)
        .unwrap();
    assert!(Arc::ptr_eq(&u, &v));
    assert_eq!((u.info.width, u.info.height), (8, 8));
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
    for t in [t, yuv, nv, i444] {
        r.destroy_texture(t);
    }

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

    // A frame with only a clear, and one with nothing at all
    r.set_draw_color(4, 5, 6, 255);
    r.clear().unwrap();
    assert_eq!(
        rows(&r.read_pixels(Some(&Rect::new(3, 3, 1, 1))).unwrap()),
        [6, 5, 4, 255]
    );
    r.present().unwrap();
    r.present().unwrap();

    // A resized window: the backbuffer follows the swapchain at the next
    // present, which asks for a redraw.
    window.set_size(40, 30).unwrap();
    for _ in 0..10 {
        pump();
    }
    let _ = get_events(EventType::FIRST, EventType::LAST, 100000);
    r.set_draw_color(9, 8, 7, 255);
    r.clear().unwrap();
    assert_eq!(r.output_size().unwrap(), (40, 30));
    r.present().unwrap();
    pump();
    let exposed = get_events(EventType::WINDOW_EXPOSED, EventType::WINDOW_EXPOSED, 100)
        .unwrap()
        .into_iter()
        .filter(|e| matches!(e, Event::Window(w) if w.window_id == window.id()))
        .count();
    assert!(exposed >= 1, "an exposed event after the resize");
    r.set_draw_color(9, 8, 7, 255);
    r.clear().unwrap();
    let s = r.read_pixels(None).unwrap();
    assert_eq!((s.width(), s.height()), (40, 30));
    assert!(rows(&s).chunks(4).all(|p| p == [7, 8, 9, 255]));
    r.present().unwrap();

    // The window can go first.
    window.destroy();
    assert!(r.clear().is_err());
    drop(r);
}

#[test]
fn gpu_matches_software_for_shapes() {
    let _l = crate::test_support::test_lock();
    let _video = Video::init();
    let Some((_window, mut r)) = gpu_renderer(W, H) else {
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

    // A clear between draws restarts the render pass.
    compare("clear between draws", &mut r, 0, |r| {
        r.set_draw_color(0, 0, 0, 255);
        r.clear().unwrap();
        r.set_draw_color(200, 0, 0, 255);
        r.render_fill_rect(Some(&FRect::new(1.0, 1.0, 10.0, 10.0)))
            .unwrap();
        r.set_draw_color(0, 0, 200, 255);
        r.clear().unwrap();
        r.set_draw_color(0, 200, 0, 255);
        r.render_fill_rect(Some(&FRect::new(5.0, 5.0, 10.0, 10.0)))
            .unwrap();
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

    // Many flushes in one frame: each uploads its vertices into the cycled
    // vertex buffer of the same command buffer, which grows on the way.
    compare("many flushes", &mut r, 0, |r| {
        r.set_draw_color(0, 0, 0, 255);
        r.clear().unwrap();
        for i in 0..300 {
            r.set_draw_color((i % 256) as u8, 100, 200, 255);
            r.render_point((i % W) as f32, (i / W) as f32).unwrap();
            r.flush().unwrap();
        }
        let points: Vec<FPoint> = (0..3000)
            .map(|i| FPoint {
                x: (i % W) as f32,
                y: (10 + i / W % 30) as f32,
            })
            .collect();
        r.set_draw_color(1, 2, 3, 255);
        r.render_points(&points).unwrap();
    });
}

#[test]
fn gpu_matches_software_for_textures() {
    let _l = crate::test_support::test_lock();
    let _video = Video::init();
    let Some((_window, mut r)) = gpu_renderer(W, H) else {
        return;
    };
    let mut sw = software_renderer(W, H);

    for format in [
        PixelFormat::ARGB8888,
        PixelFormat::ABGR8888,
        PixelFormat::XRGB8888,
        PixelFormat::RGB565,
    ] {
        let tg = pattern_texture(&mut r, format, 16, 12);
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
            scene(&mut r, tg);
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
        r.destroy_texture(tg);
        sw.destroy_texture(ts);
    }

    // FIXME (upstream): an ARGB4444 texture (one with alpha other than
    // RGBA32 and BGRA32) is drawn with the RGB shader, as if it were
    // opaque: compare it with the software renderer's copy of the same
    // texture made opaque.
    {
        let mut opaque = pattern(16, 12);
        for p in opaque.chunks_mut(4) {
            p[3] = 255;
        }
        let tg = pattern_texture(&mut r, PixelFormat::ARGB4444, 16, 12);
        let ts = texture_from(&mut sw, PixelFormat::ARGB4444, 16, 12, &opaque);
        for (rr, t) in [(&mut r, tg), (&mut sw, ts)] {
            rr.set_draw_color(30, 60, 90, 255);
            rr.clear().unwrap();
            rr.set_texture_blend_mode(t, BlendMode::BLEND).unwrap();
            rr.set_texture_scale_mode(t, ScaleMode::Nearest).unwrap();
            rr.render_texture(t, None, Some(&FRect::new(2.0, 3.0, 16.0, 12.0)))
                .unwrap();
        }
        assert_close(
            "ARGB4444 texture (drawn opaque)",
            &r.read_pixels(None).unwrap(),
            &sw.read_pixels(None).unwrap(),
            1,
        );
        r.destroy_texture(tg);
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
    let tg = make(&mut r);
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
        r.set_texture_palette(tg, Some(shared.clone())).unwrap();
        sw.set_texture_palette(ts, Some(shared)).unwrap();
    }
    for (rr, t) in [(&mut r, tg), (&mut sw, ts)] {
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
    // (at 1:1 the shader's linear filtering of the palette is exact)
    for (rr, t) in [(&mut r, tg), (&mut sw, ts)] {
        rr.set_texture_scale_mode(t, ScaleMode::Linear).unwrap();
        rr.set_texture_palette(t, Some(palette.clone())).unwrap();
        rr.set_draw_color(0, 0, 0, 255);
        rr.clear().unwrap();
        rr.render_texture(t, None, Some(&FRect::new(7.0, 3.0, 16.0, 12.0)))
            .unwrap();
    }
    assert_close(
        "linear palette 1:1",
        &r.read_pixels(None).unwrap(),
        &sw.read_pixels(None).unwrap(),
        1,
    );
    r.destroy_texture(tg);
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
    let tg = make(&mut r);
    let ts = make(&mut sw);
    let gpu_streaming = r.read_pixels(Some(&Rect::new(10, 10, 20, 20))).unwrap();
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
        &crop(&gpu_streaming),
        &crop(&sw_streaming),
        0,
    );
    r.destroy_texture(tg);
    sw.destroy_texture(ts);

    // Linear scaling and pixel art go through other shaders.
    let tg = pattern_texture(&mut r, PixelFormat::ARGB8888, 16, 12);
    let ts = pattern_texture(&mut sw, PixelFormat::ARGB8888, 16, 12);
    for mode in [ScaleMode::Linear, ScaleMode::PixelArt] {
        for (rr, t) in [(&mut r, tg), (&mut sw, ts)] {
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
    // Wrapped texture coordinates (a sampler of its own)
    for (rr, t) in [(&mut r, tg), (&mut sw, ts)] {
        rr.set_texture_scale_mode(t, ScaleMode::Nearest).unwrap();
        rr.set_texture_address_mode(TextureAddressMode::Wrap, TextureAddressMode::Wrap);
        rr.set_draw_color(0, 0, 0, 255);
        rr.clear().unwrap();
        let v = |x: f32, y: f32, u: f32, v: f32| Vertex {
            position: FPoint { x, y },
            color: FColor::new(1.0, 1.0, 1.0, 1.0),
            tex_coord: FPoint { x: u, y: v },
        };
        rr.render_geometry(
            Some(t),
            &[
                v(4.0, 4.0, 0.0, 0.0),
                v(36.0, 4.0, 2.0, 0.0),
                v(36.0, 28.0, 2.0, 2.0),
                v(4.0, 28.0, 0.0, 2.0),
            ],
            Some(&[0, 1, 2, 0, 2, 3]),
        )
        .unwrap();
        rr.set_texture_address_mode(TextureAddressMode::Auto, TextureAddressMode::Auto);
    }
    assert_close(
        "wrapped copy",
        &r.read_pixels(None).unwrap(),
        &sw.read_pixels(None).unwrap(),
        0,
    );
}

#[test]
fn gpu_matches_software_for_targets() {
    let _l = crate::test_support::test_lock();
    let _video = Video::init();
    let Some((_window, mut r)) = gpu_renderer(W, H) else {
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
            r.set_texture_blend_mode(target, BlendMode::NONE).unwrap();
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
            // 1:1, scaled up 2x, and scaled down 3:2 (whose pixel centers
            // stay off the texel edges)
            r.render_texture(target, None, Some(&FRect::new(2.0, 2.0, 24.0, 18.0)))
                .unwrap();
            r.render_texture(target, None, Some(&FRect::new(16.0, 22.0, 48.0, 36.0)))
                .unwrap();
            r.render_texture(target, None, Some(&FRect::new(40.0, 2.0, 16.0, 12.0)))
                .unwrap();
            let out = r.read_pixels(None).unwrap();
            r.destroy_texture(target);
            (read, out)
        };
        let tg = pattern_texture(&mut r, PixelFormat::ABGR8888, 16, 12);
        let ts = pattern_texture(&mut sw, PixelFormat::ABGR8888, 16, 12);
        let (gpu_read, gpu_out) = scene(&mut r, tg);
        let (sw_read, sw_out) = scene(&mut sw, ts);
        let what = format!("{} target", format.name());
        assert_eq!(gpu_read.format(), format, "{what} read format");
        assert_close(&format!("{what} read"), &gpu_read, &sw_read, 0);
        assert_close(&what, &gpu_out, &sw_out, 0);
        r.destroy_texture(tg);
        sw.destroy_texture(ts);
    }

    // A target drawn blended (the target's colors round as the blending
    // does)
    let scene = |r: &mut Renderer| {
        let target = r
            .create_texture(PixelFormat::ARGB8888, TextureAccess::Target, 16, 16)
            .unwrap();
        r.set_render_target(Some(target)).unwrap();
        r.set_draw_blend_mode(BlendMode::NONE).unwrap();
        r.set_draw_color(200, 100, 50, 128);
        r.clear().unwrap();
        r.set_render_target(None).unwrap();
        r.set_draw_color(10, 20, 30, 255);
        r.clear().unwrap();
        r.set_texture_scale_mode(target, ScaleMode::Nearest)
            .unwrap();
        r.render_texture(target, None, Some(&FRect::new(5.0, 5.0, 16.0, 16.0)))
            .unwrap();
        let out = r.read_pixels(None).unwrap();
        r.destroy_texture(target);
        out
    };
    assert_close("blended target", &scene(&mut r), &scene(&mut sw), 2);

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
fn gpu_matches_software_for_yuv() {
    let _l = crate::test_support::test_lock();
    let _video = Video::init();
    let Some((_window, mut r)) = gpu_renderer(W, H) else {
        return;
    };
    let mut sw = software_renderer(W, H);
    let (w, h) = (16usize, 12usize);
    let (y, u, v) = yuv_planes(w, h);
    let uv: Vec<u8> = u.iter().zip(&v).flat_map(|(a, b)| [*a, *b]).collect();
    let vu: Vec<u8> = u.iter().zip(&v).flat_map(|(a, b)| [*b, *a]).collect();

    // (the shaders convert in floats, the software path in fixed point)
    const TOL: u8 = 3;

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
            assert_close(&what, &scene(&mut r), &scene(&mut sw), TOL);
        }
    }

    // Whole-texture updates with all the planes in one buffer, and a
    // streaming texture written through a lock.
    let mut all = y.clone();
    all.extend_from_slice(&u);
    all.extend_from_slice(&v);
    let mut yv = y.clone();
    yv.extend_from_slice(&v);
    yv.extend_from_slice(&u);
    let mut nv = y.clone();
    nv.extend_from_slice(&uv);
    let mut nv21 = y.clone();
    nv21.extend_from_slice(&vu);
    for (format, data) in [
        (PixelFormat::IYUV, &all),
        (PixelFormat::YV12, &yv),
        (PixelFormat::NV12, &nv),
        (PixelFormat::NV21, &nv21),
    ] {
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
                TOL,
            );
        }
    }

    // An I444 texture: full-size chroma planes
    let (cw, ch) = (w, h);
    let u444: Vec<u8> = (0..cw * ch).map(|i| (110 + (i % cw) * 2) as u8).collect();
    let v444: Vec<u8> = (0..cw * ch).map(|i| (146 - (i / cw) * 3) as u8).collect();
    let scene = |r: &mut Renderer| {
        let t = r
            .create_texture(PixelFormat::I444, TextureAccess::Static, w as i32, h as i32)
            .unwrap();
        r.set_texture_scale_mode(t, ScaleMode::Nearest).unwrap();
        r.update_yuv_texture(t, None, &y, w as i32, &u444, w as i32, &v444, w as i32)
            .unwrap();
        r.set_draw_color(0, 0, 0, 255);
        r.clear().unwrap();
        r.render_texture(t, None, Some(&FRect::new(3.0, 3.0, 32.0, 24.0)))
            .unwrap();
        r.destroy_texture(t);
        r.read_pixels(None).unwrap()
    };
    assert_close("I444 texture", &scene(&mut r), &scene(&mut sw), TOL);

    // A part of a texture updated: the chroma rectangle covers it.
    let scene = |r: &mut Renderer| {
        let t = r
            .create_texture(PixelFormat::NV12, TextureAccess::Static, w as i32, h as i32)
            .unwrap();
        r.set_texture_scale_mode(t, ScaleMode::Nearest).unwrap();
        r.update_nv_texture(t, None, &y, w as i32, &uv, w as i32)
            .unwrap();
        let gray_y = vec![128u8; 6 * 4];
        let gray_uv = vec![128u8; 6 * 2];
        r.update_nv_texture(t, Some(&Rect::new(4, 2, 6, 4)), &gray_y, 6, &gray_uv, 6)
            .unwrap();
        r.set_draw_color(0, 0, 0, 255);
        r.clear().unwrap();
        r.render_texture(t, None, Some(&FRect::new(3.0, 3.0, 32.0, 24.0)))
            .unwrap();
        r.destroy_texture(t);
        r.read_pixels(None).unwrap()
    };
    assert_close("NV12 partial update", &scene(&mut r), &scene(&mut sw), TOL);

    // (a buffer without the chroma planes is refused, not read past its end)
    let t = r
        .create_texture(PixelFormat::IYUV, TextureAccess::Static, w as i32, h as i32)
        .unwrap();
    assert!(r.update_texture(t, None, &y, w as i32).is_err());
}

#[test]
fn gpu_line_methods() {
    let _l = crate::test_support::test_lock();
    let _video = Video::init();
    // GPU lines (SDL_RENDER_LINE_METHOD 2) are drawn alone, without the
    // point at their end the other renderers add: each line leaves out its
    // last pixel. Straight ones hit the same pixels as the software
    // renderer's lines one pixel shorter.
    hints::set(hints::RENDER_LINE_METHOD, "2").unwrap();
    let created = gpu_renderer(W, H);
    hints::reset(hints::RENDER_LINE_METHOD);
    let Some((_window, mut r)) = created else {
        return;
    };
    r.set_draw_color(0, 0, 0, 255);
    r.clear().unwrap();
    r.set_draw_color(255, 128, 0, 255);
    // (two single lines are one line list draw, three points a strip)
    r.render_line(2.0, 4.0, 60.0, 4.0).unwrap();
    r.render_line(7.0, 10.0, 7.0, 40.0).unwrap();
    r.render_lines(&[
        FPoint { x: 20.0, y: 20.0 },
        FPoint { x: 40.0, y: 20.0 },
        FPoint { x: 40.0, y: 30.0 },
    ])
    .unwrap();
    let gpu = r.read_pixels(None).unwrap();

    let mut sw = software_renderer(W, H);
    sw.set_draw_color(0, 0, 0, 255);
    sw.clear().unwrap();
    sw.set_draw_color(255, 128, 0, 255);
    sw.render_line(2.0, 4.0, 59.0, 4.0).unwrap();
    sw.render_line(7.0, 10.0, 7.0, 39.0).unwrap();
    sw.render_line(20.0, 20.0, 39.0, 20.0).unwrap();
    sw.render_line(40.0, 20.0, 40.0, 29.0).unwrap();
    assert_close("lines", &gpu, &sw.read_pixels(None).unwrap(), 0);
}

#[test]
fn gpu_pipelines_are_cached() {
    let _l = crate::test_support::test_lock();
    let _video = Video::init();
    let Some((window, _r)) = gpu_renderer(16, 16) else {
        return;
    };
    // (a renderer of its own, to look at its state: the window's is taken)
    let other = Window::create("gpu 2", 16, 16, WindowFlags::default()).unwrap();
    let mut g = GpuRenderer::for_window(other, Colorspace::SRGB, 0).unwrap();
    let format = g.backbuffer.as_ref().unwrap().format;
    let shaders = g.shaders.take().unwrap();
    let mut params = PipelineParameters {
        blend_mode: BlendMode::BLEND,
        frag_shader: FragmentShaderId::Color,
        vert_shader: VertexShaderId::Linepoint,
        attachment_format: format,
        primitive_type: PrimitiveType::PointList,
    };
    g.pipeline_cache
        .get_pipeline(&shaders, &g.device, &params)
        .unwrap();
    g.pipeline_cache
        .get_pipeline(&shaders, &g.device, &params)
        .unwrap();
    assert_eq!(g.pipeline_cache.len(), 1);
    params.blend_mode = BlendMode::ADD;
    g.pipeline_cache
        .get_pipeline(&shaders, &g.device, &params)
        .unwrap();
    params.vert_shader = VertexShaderId::TriTexture;
    params.frag_shader = FragmentShaderId::TextureAdvanced;
    params.primitive_type = PrimitiveType::TriangleList;
    g.pipeline_cache
        .get_pipeline(&shaders, &g.device, &params)
        .unwrap();
    assert_eq!(g.pipeline_cache.len(), 3);
    // (no pipeline for a mode the renderer doesn't support)
    params.blend_mode = BlendMode(0x0F0F_0F0F);
    assert!(g
        .pipeline_cache
        .get_pipeline(&shaders, &g.device, &params)
        .is_err());
    assert_eq!(g.pipeline_cache.len(), 3);

    // The samplers are made once per key; INDEX8 textures sample nearest.
    let key = g
        .get_sampler(
            PixelFormat::INDEX8,
            ScaleMode::Linear,
            TextureAddressMode::Clamp,
            TextureAddressMode::Wrap,
        )
        .unwrap();
    assert_eq!(key, 0b101);
    assert!(g
        .get_sampler(
            PixelFormat::RGBA32,
            ScaleMode::Linear,
            TextureAddressMode::Auto,
            TextureAddressMode::Clamp,
        )
        .is_err());
    assert_eq!(g.samplers.iter().filter(|s| s.is_some()).count(), 1);

    g.shaders = Some(shaders);
    g.destroy();
    g.destroy();
    drop(g);
    other.destroy();
    window.destroy();
}

#[test]
fn gpu_fails_without_vulkan_surfaces() {
    let _l = crate::test_support::test_lock();
    // The dummy driver has no Vulkan: the renderer can't be created, and
    // the default renderer is the software one.
    init::quit();
    hints::set(hints::VIDEO_DRIVER, "dummy").unwrap();
    init::init(InitFlags::VIDEO).unwrap();
    let window = Window::create("no gpu", 16, 16, WindowFlags::default()).unwrap();
    assert!(Renderer::for_window(&window, Some(GPU_RENDERER)).is_err());
    let r = Renderer::for_window(&window, None).unwrap();
    assert_eq!(r.name(), SOFTWARE_RENDERER);
    drop(r);
    window.destroy();
    init::quit();
    hints::reset(hints::VIDEO_DRIVER);
}
