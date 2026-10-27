// Tests of the Metal GPU backend.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! The tables are checked against the values of the macOS SDK's headers,
//! and the blit libraries against their format. The device tests make a
//! headless device (the system default Metal device) through the front end
//! on the offscreen video driver, with a bootstrap that doesn't ask the
//! video driver for Metal views (which only the windows need:
//! [`METAL_HEADLESS_TEST_DRIVER`]), create and
//! release every kind of resource, and read the results back exactly:
//! clears, uploads, copies, draws with vertex buffers, uniforms, samplers
//! and storage resources (indexed and indirect too), compute dispatches,
//! blits and mipmaps, fences and the deferred destruction. The mipmaps, a
//! scaled linear blit and a draw with interpolated colors are compared
//! with what upstream's C made on this machine's GPU (Apple M2 Pro), with
//! the same MSL. Without a Metal device they report a skip (capability
//! `metal`) and pass. In debug mode Metal's API validation is on.

use std::sync::atomic::Ordering;

use super::resources::{is_valid_metal_library, next_highest_alignment};
use super::shaders::*;
use super::sys::*;
use super::tables::*;
use super::*;
use crate::core::apple::foundation::available;
use crate::gpu::{
    BlendFactor, BlendOp, BlitRegion, Buffer, BufferCreateInfo, ColorTargetDescription, CompareOp,
    CullMode, Device, Filter, GraphicsPipeline, GraphicsPipelineTargetInfo, LoadOp, PrimitiveType,
    ShaderStage, StencilOp, StoreOp, TransferBuffer, TransferBufferCreateInfo, VertexAttribute,
    VertexBufferDescription, VertexElementFormat, VertexInputRate, VertexInputState,
    PROP_GPU_DEVICE_NAME_STRING, PROP_GPU_TEXTURE_CREATE_NAME_STRING,
};
use crate::hints;
use crate::init::{self, InitFlags};
use crate::video::FlipMode;

// ---------------------------------------------------------------------------
// Without a device
// ---------------------------------------------------------------------------

#[test]
fn texture_format_table() {
    let f = sdl_to_metal_texture_format;
    // (the values of MTLPixelFormat.h)
    assert_eq!(f(TextureFormat::INVALID), 0);
    assert_eq!(f(TextureFormat::A8_UNORM), 1);
    assert_eq!(f(TextureFormat::R8G8B8A8_UNORM), 70);
    assert_eq!(f(TextureFormat::R10G10B10A2_UNORM), 90);
    assert_eq!(f(TextureFormat::B5G6R5_UNORM), 40);
    assert_eq!(f(TextureFormat::B5G5R5A1_UNORM), 43);
    assert_eq!(f(TextureFormat::B4G4R4A4_UNORM), 42); // ABGR4, swizzled
    assert_eq!(f(TextureFormat::B8G8R8A8_UNORM_SRGB), 81);
    assert_eq!(f(TextureFormat::BC6H_RGB_UFLOAT), 151);
    assert_eq!(f(TextureFormat::R11G11B10_UFLOAT), 92);
    assert_eq!(f(TextureFormat::R8_INT), 14);
    // (Depth24Unorm_Stencil8, macOS's only)
    let d24 = if cfg!(target_os = "macos") { 255 } else { 0 };
    assert_eq!(f(TextureFormat::D24_UNORM), d24);
    assert_eq!(f(TextureFormat::D24_UNORM_S8_UINT), d24);
    assert_eq!(f(TextureFormat::D32_FLOAT_S8_UINT), 260);
    assert_eq!(f(TextureFormat::ASTC_4x4_UNORM), 204);
    assert_eq!(f(TextureFormat::ASTC_12x12_UNORM_SRGB), 200);
    assert_eq!(f(TextureFormat::ASTC_8x5_FLOAT), 228);
    assert_eq!(f(TextureFormat::ASTC_12x12_FLOAT), 236);
    // (out of range: no format)
    assert_eq!(f(TextureFormat(105)), 0);

    // Every format past INVALID has a Metal format (macOS 11 and later;
    // iOS and tvOS 16.4 and later, but for D24).
    for i in 1..105 {
        let format = TextureFormat(i);
        if f(format) == MTL_PIXEL_FORMAT_INVALID {
            assert!(
                cfg!(not(target_os = "macos"))
                    && [TextureFormat::D24_UNORM, TextureFormat::D24_UNORM_S8_UINT]
                        .contains(&format),
                "{i}"
            );
        }
    }
}

#[test]
fn enum_tables() {
    assert_eq!(SDL_TO_METAL_VERTEX_FORMAT.len(), 31);
    // (the values of MTLVertexDescriptor.h)
    assert_eq!(sdl_to_metal_vertex_format(VertexElementFormat::Int), 32);
    assert_eq!(sdl_to_metal_vertex_format(VertexElementFormat::Float3), 30);
    assert_eq!(
        sdl_to_metal_vertex_format(VertexElementFormat::Ubyte4Norm),
        9
    );
    assert_eq!(sdl_to_metal_vertex_format(VertexElementFormat::Byte2), 4);
    assert_eq!(sdl_to_metal_vertex_format(VertexElementFormat::Half4), 27);

    assert_eq!(SDL_TO_METAL_BLEND_FACTOR.len(), 14);
    assert_eq!(sdl_to_metal_blend_factor(BlendFactor::Zero), 0);
    assert_eq!(sdl_to_metal_blend_factor(BlendFactor::DstColor), 6);
    assert_eq!(sdl_to_metal_blend_factor(BlendFactor::ConstantColor), 11);
    assert_eq!(sdl_to_metal_blend_factor(BlendFactor::SrcAlphaSaturate), 10);
    assert_eq!(sdl_to_metal_blend_op(BlendOp::ReverseSubtract), 2);
    assert_eq!(sdl_to_metal_blend_op(BlendOp::Max), 4);
    assert_eq!(sdl_to_metal_compare_op(CompareOp::Never), 0);
    assert_eq!(sdl_to_metal_compare_op(CompareOp::LessOrEqual), 3);
    assert_eq!(sdl_to_metal_compare_op(CompareOp::Always), 7);
    assert_eq!(sdl_to_metal_stencil_op(StencilOp::IncrementAndWrap), 6);
    assert_eq!(sdl_to_metal_cull_mode(CullMode::Back), 2);
    assert_eq!(sdl_to_metal_primitive_type(PrimitiveType::TriangleList), 3);
    assert_eq!(sdl_to_metal_primitive_type(PrimitiveType::PointList), 0);
    assert_eq!(sdl_to_metal_load_op(LoadOp::Load), 1);
    assert_eq!(sdl_to_metal_load_op(LoadOp::DontCare), 0);
    assert_eq!(sdl_to_metal_store_op(StoreOp::ResolveAndStore), 3);
    assert_eq!(sdl_to_metal_sample_count(SampleCount::Eight), 8);
    assert_eq!(sdl_to_metal_texture_type(TextureType::Texture2D, true), 4);
    assert_eq!(sdl_to_metal_texture_type(TextureType::Texture3D, false), 7);
    assert_eq!(
        sdl_to_metal_color_write_mask(crate::gpu::ColorComponentFlags(0xF)),
        15
    );
    assert_eq!(
        sdl_to_metal_color_write_mask(crate::gpu::ColorComponentFlags::R),
        8
    );
    assert_eq!(sdl_to_metal_depth_clip_mode(false), 1);
    assert_eq!(
        swapchain_composition_to_format(SwapchainComposition::HdrExtendedLinear),
        TextureFormat::R16G16B16A16_FLOAT
    );
}

#[test]
fn helpers() {
    assert_eq!(next_highest_alignment(0, 4), 0);
    assert_eq!(next_highest_alignment(1, 4), 4);
    assert_eq!(next_highest_alignment(256, 256), 256);
    assert_eq!(next_highest_alignment(257, 256), 512);
    assert!(is_valid_metal_library(b"MTLB...."));
    assert!(!is_valid_metal_library(b"MTL"));
    assert!(!is_valid_metal_library(b"\x03\x02\x23\x07"));
    assert!(available!(macos(10, 15), ios(13, 0)));
    // (a platform it doesn't name passes)
    #[cfg(target_os = "macos")]
    assert!(!available!(macos(999, 0)) && available!(ios(999, 0)));
    #[cfg(target_os = "ios")]
    assert!(!available!(ios(999, 0)) && available!(macos(999, 0)));
}

#[test]
fn blit_libraries() {
    for (library, function) in [
        (&FULLSCREEN_VERT[..], "FullscreenVert"),
        (&BLIT_FROM_2D[..], "BlitFrom2D"),
        (&BLIT_FROM_2D_ARRAY[..], "BlitFrom2DArray"),
        (&BLIT_FROM_3D[..], "BlitFrom3D"),
        (&BLIT_FROM_CUBE[..], "BlitFromCube"),
        (&BLIT_FROM_CUBE_ARRAY[..], "BlitFromCubeArray"),
    ] {
        assert!(is_valid_metal_library(library), "{function}");
        // (the file size, at offset 16)
        let size = u64::from_le_bytes(library[16..24].try_into().unwrap());
        assert_eq!(size, library.len() as u64, "{function}");
        let name = format!("{function}\0");
        assert!(
            library.windows(name.len()).any(|w| w == name.as_bytes()),
            "{function}"
        );
    }
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

/// A debug-mode device through the front end, or `None` (with the skip
/// reported) without a Metal device.
fn front_end_device() -> Option<Device> {
    *crate::gpu::TEST_BACKENDS.lock().unwrap() = Some(vec![&METAL_HEADLESS_TEST_DRIVER]);
    let device = Device::new(ShaderFormat::MSL, true, Some("metal"));
    *crate::gpu::TEST_BACKENDS.lock().unwrap() = None;
    match device {
        Ok(device) => Some(device),
        Err(e) => {
            crate::test_support::skip("metal", format_args!("no Metal device ({})", e.message()));
            None
        }
    }
}

/// The Metal renderer of a front-end device.
fn device_renderer(device: &Device) -> &MetalRenderer {
    let driver: &dyn GpuDriver = &*device.shared.driver;
    // SAFETY: the metal device's driver is a MetalRenderer.
    unsafe { &*(driver as *const dyn GpuDriver as *const MetalRenderer) }
}

fn texture_info(
    texture_type: TextureType,
    format: TextureFormat,
    usage: TextureUsageFlags,
    layers: u32,
    levels: u32,
) -> TextureCreateInfo {
    TextureCreateInfo {
        texture_type,
        format,
        usage,
        width: 16,
        height: 16,
        layer_count_or_depth: layers,
        num_levels: levels,
        sample_count: SampleCount::One,
        props: None,
    }
}

/// An MSL shader of a stage.
fn msl_shader(
    device: &Device,
    source: &str,
    stage: ShaderStage,
    num_samplers: u32,
    num_storage_textures: u32,
    num_storage_buffers: u32,
    num_uniform_buffers: u32,
) -> crate::gpu::Shader {
    device
        .create_shader(&ShaderCreateInfo {
            code: source.as_bytes(),
            entrypoint: "",
            format: ShaderFormat::MSL,
            stage,
            num_samplers,
            num_storage_textures,
            num_storage_buffers,
            num_uniform_buffers,
            props: None,
        })
        .unwrap()
}

/// `vec2` positions at attribute 0 plus a vertex uniform offset.
const VERTEX_INPUT_UNIFORM: &str = "
#include <metal_stdlib>
using namespace metal;
struct U { float4 offset; };
struct VIn { float2 pos [[attribute(0)]]; };
vertex float4 main0(VIn in [[stage_in]], constant U& u [[buffer(0)]]) {
    return float4(in.pos, 0.0, 1.0) + u.offset;
}
";

/// Red.
const FRAGMENT_SOLID: &str = "
#include <metal_stdlib>
using namespace metal;
fragment float4 main0() { return float4(1.0, 0.0, 0.0, 1.0); }
";

/// A sampler at 0 and a uniform tint.
const FRAGMENT_SAMPLED_UNIFORM: &str = "
#include <metal_stdlib>
using namespace metal;
struct T { float4 tint; };
fragment float4 main0(texture2d<float> tex [[texture(0)]], sampler smp [[sampler(0)]],
                      constant T& t [[buffer(0)]]) {
    return tex.sample(smp, float2(0.5)) * t.tint;
}
";

/// The second color of a storage buffer (fragment storage buffers follow
/// the uniform buffers), times the first texel of a storage texture
/// (storage textures follow the samplers).
const FRAGMENT_STORAGE: &str = "
#include <metal_stdlib>
using namespace metal;
fragment float4 main0(const device float4* colors [[buffer(0)]],
                      texture2d<float, access::read> scale [[texture(0)]]) {
    return colors[1] * scale.read(uint2(0, 0));
}
";

/// `data[gid] = gid + add`: the uniform at buffer 0, the read-write
/// storage buffer after it.
const COMPUTE_INDEX: &str = "
#include <metal_stdlib>
using namespace metal;
struct U { uint add; };
kernel void main0(constant U& u [[buffer(0)]], device uint* data [[buffer(1)]],
                  uint3 gid [[thread_position_in_grid]]) {
    data[gid.x] = gid.x + u.add;
}
";

/// A gradient into a read-write storage texture, from a read-only storage
/// buffer of scales.
const COMPUTE_TEXTURE: &str = "
#include <metal_stdlib>
using namespace metal;
kernel void main0(const device float* scales [[buffer(0)]],
                  texture2d<float, access::write> image [[texture(0)]],
                  uint3 gid [[thread_position_in_grid]]) {
    image.write(float4(gid.x * scales[0], gid.y * scales[1], scales[2], 1.0), gid.xy);
}
";

/// The full-target triangle with interpolated colors of the reference
/// harness (scaled by a vertex uniform).
const GRADIENT_VERTEX: &str = "#include <metal_stdlib>
using namespace metal;
struct VOut { float4 pos [[position]]; float4 color; };
struct U { float4 scale; };
vertex VOut main0(uint vid [[vertex_id]], constant U& u [[buffer(0)]]) {
    float2 p[3] = { float2(-1.0, -1.0), float2(3.0, -1.0), float2(-1.0, 3.0) };
    float4 c[3] = { float4(1, 0, 0, 1), float4(0, 1, 0, 1), float4(0, 0, 1, 1) };
    VOut o; o.pos = float4(p[vid], 0, 1); o.color = c[vid] * u.scale; return o;
}
";

const GRADIENT_FRAGMENT: &str = "#include <metal_stdlib>
using namespace metal;
struct VOut { float4 pos [[position]]; float4 color; };
fragment float4 main0(VOut in [[stage_in]]) { return in.color; }
";

/// The bytes of 32-bit words.
fn word_bytes(words: &[u32]) -> Vec<u8> {
    words.iter().flat_map(|w| w.to_ne_bytes()).collect()
}

/// The bytes of floats.
fn float_bytes(floats: &[f32]) -> Vec<u8> {
    floats.iter().flat_map(|f| f.to_ne_bytes()).collect()
}

/// Bytes from a hex string.
fn hex(s: &str) -> Vec<u8> {
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap())
        .collect()
}

/// An upload transfer buffer holding `data`.
fn upload_buffer(device: &Device, data: &[u8]) -> TransferBuffer {
    let mut transfer_buffer = device
        .create_transfer_buffer(&TransferBufferCreateInfo {
            usage: TransferBufferUsage::Upload,
            size: data.len() as u32,
            props: None,
        })
        .unwrap();
    transfer_buffer.map(false).unwrap().copy_from_slice(data);
    transfer_buffer
}

/// A download transfer buffer of `size` bytes.
fn download_buffer(device: &Device, size: u32) -> TransferBuffer {
    device
        .create_transfer_buffer(&TransferBufferCreateInfo {
            usage: TransferBufferUsage::Download,
            size,
            props: None,
        })
        .unwrap()
}

/// What a transfer buffer holds.
fn read_back(transfer_buffer: &mut TransferBuffer) -> Vec<u8> {
    transfer_buffer.map(false).unwrap().to_vec()
}

/// Submit a command buffer and wait for its fence.
fn submit_and_wait(device: &Device, command_buffer: CommandBuffer) {
    let fence = command_buffer.submit_and_acquire_fence().unwrap();
    device.wait_for_fences(true, &[&fence]).unwrap();
    assert!(fence.is_signaled());
}

/// A GPU buffer.
fn gpu_buffer(device: &Device, usage: BufferUsageFlags, size: u32) -> Buffer {
    device
        .create_buffer(&BufferCreateInfo {
            usage,
            size,
            props: None,
        })
        .unwrap()
}

/// Upload data into a GPU buffer (in its own command buffer).
fn fill_buffer(device: &Device, buffer: &Buffer, data: &[u8]) {
    let upload = upload_buffer(device, data);
    let mut cb = device.acquire_command_buffer().unwrap();
    {
        let mut copy = cb.begin_copy_pass().unwrap();
        copy.upload_to_buffer(
            &TransferBufferLocation {
                transfer_buffer: &upload,
                offset: 0,
            },
            &BufferRegion {
                buffer,
                offset: 0,
                size: data.len() as u32,
            },
            false,
        );
    }
    submit_and_wait(device, cb);
}

/// The bytes of a GPU buffer.
fn buffer_contents(device: &Device, buffer: &Buffer, size: u32) -> Vec<u8> {
    let mut download = download_buffer(device, size);
    let mut cb = device.acquire_command_buffer().unwrap();
    {
        let mut copy = cb.begin_copy_pass().unwrap();
        copy.download_from_buffer(
            &BufferRegion {
                buffer,
                offset: 0,
                size,
            },
            &TransferBufferLocation {
                transfer_buffer: &download,
                offset: 0,
            },
        );
    }
    submit_and_wait(device, cb);
    read_back(&mut download)
}

/// A 2D RGBA8 texture.
fn rgba_texture(device: &Device, w: u32, h: u32, usage: TextureUsageFlags, levels: u32) -> Texture {
    device
        .create_texture(&TextureCreateInfo {
            width: w,
            height: h,
            ..texture_info(
                TextureType::Texture2D,
                TextureFormat::R8G8B8A8_UNORM,
                usage,
                1,
                levels,
            )
        })
        .unwrap()
}

/// Upload RGBA8 pixels into a texture's level (in its own command buffer).
fn fill_texture(device: &Device, texture: &Texture, level: u32, w: u32, h: u32, pixels: &[u8]) {
    let upload = upload_buffer(device, pixels);
    let mut cb = device.acquire_command_buffer().unwrap();
    {
        let mut copy = cb.begin_copy_pass().unwrap();
        copy.upload_to_texture(
            &TextureTransferInfo {
                transfer_buffer: &upload,
                offset: 0,
                pixels_per_row: w,
                rows_per_layer: h,
            },
            &TextureRegion {
                texture,
                mip_level: level,
                layer: 0,
                x: 0,
                y: 0,
                z: 0,
                w,
                h,
                d: 1,
            },
            false,
        );
    }
    submit_and_wait(device, cb);
}

/// The pixels (four bytes each) of a texture's level.
fn texture_pixels(device: &Device, texture: &Texture, level: u32, w: u32, h: u32) -> Vec<u8> {
    let mut download = download_buffer(device, w * h * 4);
    let mut cb = device.acquire_command_buffer().unwrap();
    {
        let mut copy = cb.begin_copy_pass().unwrap();
        copy.download_from_texture(
            &TextureRegion {
                texture,
                mip_level: level,
                layer: 0,
                x: 0,
                y: 0,
                z: 0,
                w,
                h,
                d: 1,
            },
            &TextureTransferInfo {
                transfer_buffer: &download,
                offset: 0,
                pixels_per_row: w,
                rows_per_layer: h,
            },
        );
    }
    submit_and_wait(device, cb);
    read_back(&mut download)
}

/// `w * h` pixels of one color.
fn solid(w: u32, h: u32, color: [u8; 4]) -> Vec<u8> {
    (0..w * h).flat_map(|_| color).collect()
}

/// A color target cleared to a color.
fn clear_target(texture: &Texture, color: FColor) -> ColorTargetInfo<'_> {
    ColorTargetInfo {
        load_op: LoadOp::Clear,
        clear_color: color,
        ..ColorTargetInfo::new(texture)
    }
}

const RED: [u8; 4] = [255, 0, 0, 255];
const BLUE: [u8; 4] = [0, 0, 255, 255];
const BLUE_F: FColor = FColor {
    r: 0.0,
    g: 0.0,
    b: 1.0,
    a: 1.0,
};

/// The texture container of a front-end texture.
fn container_of(texture: &Texture) -> Arc<TextureContainer> {
    texture.raw.downcast::<TextureContainer>().unwrap()
}

/// The 8x8 pattern of the reference harness.
fn pattern() -> Vec<u8> {
    let mut pixels = Vec::new();
    for y in 0..8u32 {
        for x in 0..8u32 {
            pixels.extend([
                (x * 31 + y * 3) as u8,
                (y * 29 + x * 5) as u8,
                ((x ^ y) * 17) as u8,
                (255 - x * y * 3) as u8,
            ]);
        }
    }
    pixels
}

#[test]
fn device_formats_and_properties() {
    let _l = crate::test_support::test_lock();
    let _v = Video::init();
    let Some(device) = front_end_device() else {
        return;
    };
    use TextureUsageFlags as U;

    assert_eq!(device.driver(), "metal");
    assert_eq!(
        device.shader_formats(),
        ShaderFormat::MSL | ShaderFormat::METALLIB
    );
    let name = device
        .properties()
        .get_string(PROP_GPU_DEVICE_NAME_STRING)
        .unwrap();
    assert!(!name.is_empty());

    let r = device_renderer(&device);
    // SAFETY: the device's own query, for comparison.
    let has_apple1 = unsafe {
        let b: crate::core::apple::objc::Bool =
            crate::core::apple::objc::msg_send![r.device, supportsFamily: MTL_GPU_FAMILY_APPLE1];
        b.is_true()
    };
    let sampler = U::SAMPLER;
    assert!(device.texture_supports_format(
        TextureFormat::R8G8B8A8_UNORM,
        TextureType::Texture2D,
        sampler | U::COLOR_TARGET
    ));
    // Only depth formats are depth targets.
    assert!(!device.texture_supports_format(
        TextureFormat::R8G8B8A8_UNORM,
        TextureType::Texture2D,
        U::DEPTH_STENCIL_TARGET
    ));
    assert!(device.texture_supports_format(
        TextureFormat::D32_FLOAT,
        TextureType::Texture2D,
        U::DEPTH_STENCIL_TARGET
    ));
    // Cube arrays always, the Apple GPU formats with Apple GPUs.
    assert!(device.texture_supports_format(
        TextureFormat::B5G6R5_UNORM,
        TextureType::CubeArray,
        sampler
    ));
    assert_eq!(
        device.texture_supports_format(
            TextureFormat::B5G6R5_UNORM,
            TextureType::Texture2D,
            sampler
        ),
        has_apple1
    );
    // BC formats aren't color targets.
    assert!(!device.texture_supports_format(
        TextureFormat::BC1_RGBA_UNORM,
        TextureType::Texture2D,
        U::COLOR_TARGET
    ));
    assert!(device.texture_supports_sample_count(TextureFormat::R8G8B8A8_UNORM, SampleCount::One));
    assert!(device.texture_supports_sample_count(TextureFormat::R8G8B8A8_UNORM, SampleCount::Four));

    // Every kind of texture, then released.
    for (texture_type, layers) in [
        (TextureType::Texture2D, 1),
        (TextureType::Texture2DArray, 3),
        (TextureType::Texture3D, 4),
        (TextureType::Cube, 6),
        (TextureType::CubeArray, 12),
    ] {
        let props = Properties::new();
        props
            .set(PROP_GPU_TEXTURE_CREATE_NAME_STRING, "a texture")
            .unwrap();
        let texture = device
            .create_texture(&TextureCreateInfo {
                props: Some(props),
                ..texture_info(
                    texture_type,
                    TextureFormat::R16G16B16A16_FLOAT,
                    U::SAMPLER | U::COLOR_TARGET,
                    layers,
                    2,
                )
            })
            .unwrap();
        let container = container_of(&texture);
        assert_eq!(
            lock(&container.state).debug_name.as_deref(),
            Some("a texture")
        );
        texture.set_name("renamed");
        assert_eq!(
            lock(&container.state).debug_name.as_deref(),
            Some("renamed")
        );
    }
    // The released textures wait for the next submission or wait.
    assert_eq!(lock(&r.dispose).texture_containers_to_destroy.len(), 5);
    device.wait_for_idle().unwrap();
    assert!(lock(&r.dispose).texture_containers_to_destroy.is_empty());

    // Invalid shader code is refused.
    let refused = device.create_shader(&ShaderCreateInfo {
        code: b"not a library",
        format: ShaderFormat::METALLIB,
        stage: ShaderStage::Vertex,
        ..Default::default()
    });
    assert_eq!(
        refused.unwrap_err().message(),
        "The provided shader code is not a valid Metal library!"
    );
    assert!(device
        .create_shader(&ShaderCreateInfo {
            code: b"this isn't MSL",
            format: ShaderFormat::MSL,
            stage: ShaderStage::Vertex,
            ..Default::default()
        })
        .is_err());
    // A Metal library shader (one of the blit ones), and an MSL function
    // of another name than main0.
    assert!(device
        .create_shader(&ShaderCreateInfo {
            code: &BLIT_FROM_2D,
            entrypoint: "BlitFrom2D",
            format: ShaderFormat::METALLIB,
            stage: ShaderStage::Fragment,
            num_samplers: 1,
            num_uniform_buffers: 1,
            ..Default::default()
        })
        .is_ok());
    assert!(device
        .create_shader(&ShaderCreateInfo {
            code: FRAGMENT_SOLID.replace("main0", "red").as_bytes(),
            entrypoint: "red",
            format: ShaderFormat::MSL,
            stage: ShaderStage::Fragment,
            ..Default::default()
        })
        .is_ok());

    // The blit resources were made.
    let mut blit = lock(&r.blit);
    let (shaders, linear, nearest, _) = blit.parts();
    assert!(shaders.is_some() && linear.is_some() && nearest.is_some());
}

#[test]
fn clear_a_texture_and_download_it() {
    let _l = crate::test_support::test_lock();
    let _v = Video::init();
    let Some(device) = front_end_device() else {
        return;
    };
    use TextureUsageFlags as U;

    let target = rgba_texture(&device, 4, 4, U::COLOR_TARGET | U::SAMPLER, 1);

    let mut cb = device.acquire_command_buffer().unwrap();
    {
        let color = FColor {
            r: 1.0,
            g: 0.0,
            b: 0.2,
            a: 1.0,
        };
        let pass = cb
            .begin_render_pass(&[clear_target(&target, color)], None)
            .unwrap();
        pass.end();
    }
    cb.submit().unwrap();
    device.wait_for_idle().unwrap();

    assert_eq!(
        texture_pixels(&device, &target, 0, 4, 4),
        solid(4, 4, [255, 0, 51, 255])
    );

    // A depth target (with stencil) clears too, and a depth-only pass.
    let depth = device
        .create_texture(&texture_info(
            TextureType::Texture2D,
            TextureFormat::D32_FLOAT_S8_UINT,
            U::DEPTH_STENCIL_TARGET,
            1,
            1,
        ))
        .unwrap();
    let small = rgba_texture(&device, 16, 16, U::COLOR_TARGET | U::SAMPLER, 1);
    let mut cb = device.acquire_command_buffer().unwrap();
    {
        let depth_target = DepthStencilTargetInfo {
            load_op: LoadOp::Clear,
            clear_depth: 1.0,
            store_op: StoreOp::DontCare,
            stencil_load_op: LoadOp::Clear,
            stencil_store_op: StoreOp::DontCare,
            clear_stencil: 3,
            ..DepthStencilTargetInfo::new(&depth)
        };
        cb.begin_render_pass(&[clear_target(&small, BLUE_F)], Some(&depth_target))
            .unwrap()
            .end();
        cb.begin_render_pass(&[], Some(&depth_target))
            .unwrap()
            .end();
    }
    submit_and_wait(&device, cb);
    assert_eq!(
        texture_pixels(&device, &small, 0, 16, 16),
        solid(16, 16, BLUE)
    );

    // A layer of an array texture and a depth plane of a 3D texture.
    for (texture_type, layers) in [
        (TextureType::Texture2DArray, 3),
        (TextureType::Texture3D, 3),
    ] {
        let texture = device
            .create_texture(&TextureCreateInfo {
                width: 2,
                height: 2,
                ..texture_info(
                    texture_type,
                    TextureFormat::R8G8B8A8_UNORM,
                    U::COLOR_TARGET | U::SAMPLER,
                    layers,
                    1,
                )
            })
            .unwrap();
        let mut cb = device.acquire_command_buffer().unwrap();
        {
            let info = ColorTargetInfo {
                layer_or_depth_plane: 2,
                ..clear_target(&texture, BLUE_F)
            };
            cb.begin_render_pass(&[info], None).unwrap().end();
        }
        submit_and_wait(&device, cb);
        let mut download = download_buffer(&device, 2 * 2 * 4);
        let mut cb = device.acquire_command_buffer().unwrap();
        {
            let mut copy = cb.begin_copy_pass().unwrap();
            let (layer, z) = if texture_type == TextureType::Texture3D {
                (0, 2)
            } else {
                (2, 0)
            };
            copy.download_from_texture(
                &TextureRegion {
                    texture: &texture,
                    mip_level: 0,
                    layer,
                    x: 0,
                    y: 0,
                    z,
                    w: 2,
                    h: 2,
                    d: 1,
                },
                &TextureTransferInfo {
                    transfer_buffer: &download,
                    offset: 0,
                    pixels_per_row: 0,
                    rows_per_layer: 0,
                },
            );
        }
        submit_and_wait(&device, cb);
        assert_eq!(
            read_back(&mut download),
            solid(2, 2, BLUE),
            "{texture_type:?}"
        );
    }
}

#[test]
fn upload_copy_and_download_buffers_and_textures() {
    let _l = crate::test_support::test_lock();
    let _v = Video::init();
    let Some(device) = front_end_device() else {
        return;
    };
    use crate::gpu::BufferUsageFlags as B;
    use TextureUsageFlags as U;

    // Buffers: upload, copy into the middle of another, download.
    let data: Vec<u8> = (0..256u32).map(|i| (i * 7 + 3) as u8).collect();
    let a = gpu_buffer(&device, B::VERTEX, 256);
    let b = gpu_buffer(&device, B::COMPUTE_STORAGE_READ, 512);
    fill_buffer(&device, &a, &data);
    fill_buffer(&device, &b, &[0xAA; 512]);
    let mut cb = device.acquire_command_buffer().unwrap();
    {
        let mut copy = cb.begin_copy_pass().unwrap();
        copy.copy_buffer_to_buffer(
            &BufferLocation {
                buffer: &a,
                offset: 16,
            },
            &BufferLocation {
                buffer: &b,
                offset: 100,
            },
            200,
            false,
        );
    }
    submit_and_wait(&device, cb);
    assert_eq!(buffer_contents(&device, &a, 256), data);
    let contents = buffer_contents(&device, &b, 512);
    assert!(contents[..100].iter().all(|&x| x == 0xAA));
    assert_eq!(&contents[100..300], &data[16..216]);
    assert!(contents[300..].iter().all(|&x| x == 0xAA));

    // Textures: upload a pattern, copy a region into another texture,
    // download both.
    let pattern: Vec<u8> = (0..8 * 8 * 4).map(|i| (i * 5 % 251) as u8).collect();
    let src = rgba_texture(&device, 8, 8, U::SAMPLER, 1);
    let dst = rgba_texture(&device, 8, 8, U::SAMPLER | U::COLOR_TARGET, 1);
    fill_texture(&device, &src, 0, 8, 8, &pattern);
    fill_texture(&device, &dst, 0, 8, 8, &solid(8, 8, BLUE));
    let mut cb = device.acquire_command_buffer().unwrap();
    {
        let mut copy = cb.begin_copy_pass().unwrap();
        copy.copy_texture_to_texture(
            &TextureLocation {
                texture: &src,
                mip_level: 0,
                layer: 0,
                x: 2,
                y: 1,
                z: 0,
            },
            &TextureLocation {
                texture: &dst,
                mip_level: 0,
                layer: 0,
                x: 4,
                y: 4,
                z: 0,
            },
            3,
            2,
            1,
            false,
        )
        .unwrap();
    }
    submit_and_wait(&device, cb);
    assert_eq!(texture_pixels(&device, &src, 0, 8, 8), pattern);
    let copied = texture_pixels(&device, &dst, 0, 8, 8);
    for y in 0..8usize {
        for x in 0..8usize {
            let got = &copied[(y * 8 + x) * 4..][..4];
            if (4..7).contains(&x) && (4..6).contains(&y) {
                let (sx, sy) = (x - 4 + 2, y - 4 + 1);
                assert_eq!(got, &pattern[(sy * 8 + sx) * 4..][..4], "({x}, {y})");
            } else {
                assert_eq!(got, BLUE, "({x}, {y})");
            }
        }
    }

    // Uploading twice with cycling in one command buffer: the second
    // upload goes to a new texture, as the first one is in use. (Finished
    // command buffers are cleaned by the next submission or a wait, as
    // upstream's are: until then the download still uses the texture.)
    device.wait_for_idle().unwrap();
    let upload = upload_buffer(&device, &solid(8, 8, RED));
    let mut cb = device.acquire_command_buffer().unwrap();
    {
        let mut copy = cb.begin_copy_pass().unwrap();
        for _ in 0..2 {
            copy.upload_to_texture(
                &TextureTransferInfo {
                    transfer_buffer: &upload,
                    offset: 0,
                    pixels_per_row: 8,
                    rows_per_layer: 8,
                },
                &TextureRegion {
                    texture: &dst,
                    mip_level: 0,
                    layer: 0,
                    x: 0,
                    y: 0,
                    z: 0,
                    w: 8,
                    h: 8,
                    d: 1,
                },
                true,
            );
        }
    }
    submit_and_wait(&device, cb);
    assert_eq!(lock(&container_of(&dst).state).textures.len(), 2);
    assert_eq!(texture_pixels(&device, &dst, 0, 8, 8), solid(8, 8, RED));

    // A transfer buffer mapped with cycling while in use is another one.
    let mut transfer = upload_buffer(&device, &[1; 16]);
    let target = gpu_buffer(&device, B::VERTEX, 16);
    let mut cb = device.acquire_command_buffer().unwrap();
    {
        let mut copy = cb.begin_copy_pass().unwrap();
        copy.upload_to_buffer(
            &TransferBufferLocation {
                transfer_buffer: &transfer,
                offset: 0,
            },
            &BufferRegion {
                buffer: &target,
                offset: 0,
                size: 16,
            },
            false,
        );
    }
    transfer.map(true).unwrap().copy_from_slice(&[2; 16]);
    submit_and_wait(&device, cb);
    let transfer_container = transfer.raw.downcast::<BufferContainer>().unwrap();
    assert_eq!(lock(&transfer_container.state).buffers.len(), 2);
    assert_eq!(buffer_contents(&device, &target, 16), [1; 16]);
    device.wait_for_idle().unwrap();
}

/// A buffer keeps its name, which the buffers it cycles to get too
/// (upstream's container forgets it, so they have no label).
#[test]
fn cycled_buffers_keep_the_name() {
    let _l = crate::test_support::test_lock();
    let _v = Video::init();
    let Some(device) = front_end_device() else {
        return;
    };
    let props = Properties::new();
    props
        .set(crate::gpu::PROP_GPU_BUFFER_CREATE_NAME_STRING, "a buffer")
        .unwrap();
    let buffer = device
        .create_buffer(&BufferCreateInfo {
            usage: BufferUsageFlags::VERTEX,
            size: 16,
            props: Some(props),
        })
        .unwrap();
    let upload = upload_buffer(&device, &[3; 16]);
    let mut cb = device.acquire_command_buffer().unwrap();
    {
        let mut copy = cb.begin_copy_pass().unwrap();
        // (the second upload cycles the buffer the first one uses)
        for _ in 0..2 {
            copy.upload_to_buffer(
                &TransferBufferLocation {
                    transfer_buffer: &upload,
                    offset: 0,
                },
                &BufferRegion {
                    buffer: &buffer,
                    offset: 0,
                    size: 16,
                },
                true,
            );
        }
    }
    submit_and_wait(&device, cb);
    let container = buffer.raw.downcast::<BufferContainer>().unwrap();
    let state = lock(&container.state);
    assert_eq!(state.debug_name.as_deref(), Some("a buffer"));
    assert_eq!(state.buffers.len(), 2);
    for metal_buffer in &state.buffers {
        // SAFETY: a buffer; its label is an NSString (or nil).
        let label = crate::core::apple::objc::autoreleasepool(|| unsafe {
            let label: crate::core::apple::objc::Id =
                crate::core::apple::objc::msg_send![metal_buffer.handle.get(), label];
            crate::core::apple::foundation::string_from_ns(label)
        });
        assert_eq!(label.as_deref(), Some("a buffer"));
    }
    drop(state);
    device.wait_for_idle().unwrap();
}

/// A pipeline drawing `vec2` positions from vertex buffer slot 0 into an
/// RGBA8 target.
fn position_pipeline(
    device: &Device,
    vertex: &crate::gpu::Shader,
    fragment: &crate::gpu::Shader,
) -> GraphicsPipeline {
    let color_target = [ColorTargetDescription {
        format: TextureFormat::R8G8B8A8_UNORM,
        blend_state: Default::default(),
    }];
    let vertex_buffers = [VertexBufferDescription {
        slot: 0,
        pitch: 8,
        input_rate: VertexInputRate::Vertex,
        instance_step_rate: 0,
    }];
    let vertex_attributes = [VertexAttribute {
        location: 0,
        buffer_slot: 0,
        format: VertexElementFormat::Float2,
        offset: 0,
    }];
    device
        .create_graphics_pipeline(&GraphicsPipelineCreateInfo {
            vertex_shader: vertex,
            fragment_shader: fragment,
            vertex_input_state: VertexInputState {
                vertex_buffer_descriptions: &vertex_buffers,
                vertex_attributes: &vertex_attributes,
            },
            primitive_type: PrimitiveType::TriangleList,
            rasterizer_state: Default::default(),
            multisample_state: Default::default(),
            depth_stencil_state: Default::default(),
            target_info: GraphicsPipelineTargetInfo {
                color_target_descriptions: &color_target,
                depth_stencil_format: TextureFormat::D16_UNORM,
                has_depth_stencil_target: false,
            },
            props: None,
        })
        .unwrap()
}

/// The positions pipeline with the solid red fragment shader.
fn solid_pipeline(device: &Device) -> GraphicsPipeline {
    let vertex = msl_shader(
        device,
        VERTEX_INPUT_UNIFORM,
        ShaderStage::Vertex,
        0,
        0,
        0,
        1,
    );
    let fragment = msl_shader(device, FRAGMENT_SOLID, ShaderStage::Fragment, 0, 0, 0, 0);
    position_pipeline(device, &vertex, &fragment)
}

/// The top-left quadrant of clip space (+y is up) as two triangles.
const QUAD_TOP_LEFT: [f32; 12] = [
    -1.0, 0.0, 0.0, 0.0, -1.0, 1.0, //
    -1.0, 1.0, 0.0, 0.0, 0.0, 1.0,
];

/// Which quadrants of an 8x8 image are red (top-left, top-right,
/// bottom-left, bottom-right); the rest is blue.
fn quadrants(red: [bool; 4]) -> Vec<u8> {
    let mut pixels = Vec::new();
    for y in 0..8 {
        for x in 0..8 {
            let quadrant = (y >= 4) as usize * 2 + (x >= 4) as usize;
            pixels.extend(if red[quadrant] { RED } else { BLUE });
        }
    }
    pixels
}

#[test]
fn draw_triangles_with_a_vertex_buffer_and_uniforms() {
    let _l = crate::test_support::test_lock();
    let _v = Video::init();
    let Some(device) = front_end_device() else {
        return;
    };
    use crate::gpu::BufferUsageFlags as B;
    use TextureUsageFlags as U;

    let pipeline = solid_pipeline(&device);
    let vertices = gpu_buffer(&device, B::VERTEX, 48);
    fill_buffer(&device, &vertices, &float_bytes(&QUAD_TOP_LEFT));
    let target = rgba_texture(&device, 8, 8, U::COLOR_TARGET | U::SAMPLER, 1);

    // Two draws of the quad, moved by the vertex uniform: the top-left
    // quadrant and the bottom-right one.
    let mut cb = device.acquire_command_buffer().unwrap();
    {
        let mut pass = cb
            .begin_render_pass(&[clear_target(&target, BLUE_F)], None)
            .unwrap();
        pass.bind_graphics_pipeline(&pipeline);
        pass.bind_vertex_buffers(
            0,
            &[BufferBinding {
                buffer: &vertices,
                offset: 0,
            }],
        );
        pass.command_buffer()
            .push_vertex_uniform_data(0, &float_bytes(&[0.0; 4]))
            .unwrap();
        pass.draw_primitives(6, 1, 0, 0).unwrap();
        pass.command_buffer()
            .push_vertex_uniform_data(0, &float_bytes(&[1.0, -1.0, 0.0, 0.0]))
            .unwrap();
        pass.draw_primitives(6, 1, 0, 0).unwrap();
    }
    submit_and_wait(&device, cb);
    assert_eq!(
        texture_pixels(&device, &target, 0, 8, 8),
        quadrants([true, false, false, true])
    );

    // Many draws in one command buffer move to another uniform buffer
    // (each push takes 256 bytes of a 32 KiB buffer).
    let mut cb = device.acquire_command_buffer().unwrap();
    {
        let mut pass = cb
            .begin_render_pass(&[clear_target(&target, BLUE_F)], None)
            .unwrap();
        pass.bind_graphics_pipeline(&pipeline);
        pass.bind_vertex_buffers(
            0,
            &[BufferBinding {
                buffer: &vertices,
                offset: 0,
            }],
        );
        let draws = 32768 / 256 + 4;
        for i in 0..draws {
            // (only the last draw is on screen: top-right)
            let x = if i == draws - 1 { 1.0 } else { 4.0 };
            pass.command_buffer()
                .push_vertex_uniform_data(0, &float_bytes(&[x, 0.0, 0.0, 0.0]))
                .unwrap();
            pass.draw_primitives(6, 1, 0, 0).unwrap();
        }
        let metal_cb = pass
            .command_buffer()
            .backend_mut::<MetalCommandBuffer>()
            .unwrap();
        // (the first buffer bound with the pipeline, and the next one)
        assert_eq!(metal_cb.used_uniform_buffers.len(), 2);
    }
    submit_and_wait(&device, cb);
    assert_eq!(
        texture_pixels(&device, &target, 0, 8, 8),
        quadrants([false, true, false, false])
    );
    device.wait_for_idle().unwrap();
    // The uniform buffers went back to the pool.
    assert!(lock(&device_renderer(&device).uniform_buffer_pool).len() >= 32);
}

#[test]
fn indexed_and_indirect_draws() {
    let _l = crate::test_support::test_lock();
    let _v = Video::init();
    let Some(device) = front_end_device() else {
        return;
    };
    use crate::gpu::BufferUsageFlags as B;
    use TextureUsageFlags as U;

    let pipeline = solid_pipeline(&device);
    // The quad as four corners and six 16-bit indices, and as six vertices.
    let corners = gpu_buffer(&device, B::VERTEX, 32);
    fill_buffer(
        &device,
        &corners,
        &float_bytes(&[-1.0, 0.0, 0.0, 0.0, -1.0, 1.0, 0.0, 1.0]),
    );
    let indices = gpu_buffer(&device, B::INDEX, 12);
    let index_data: Vec<u8> = [0u16, 1, 2, 2, 1, 3]
        .iter()
        .flat_map(|i| i.to_ne_bytes())
        .collect();
    fill_buffer(&device, &indices, &index_data);
    let vertices = gpu_buffer(&device, B::VERTEX, 48);
    fill_buffer(&device, &vertices, &float_bytes(&QUAD_TOP_LEFT));
    // An indirect draw (after 4 bytes of padding) and an indexed one.
    let indirect = gpu_buffer(&device, B::INDIRECT, 4 + 16 + 20);
    let indirect_data = word_bytes(&[
        0xDEAD, // (padding)
        6, 1, 0, 0, // IndirectDrawCommand
        6, 1, 0, 0, 0, // IndexedIndirectDrawCommand
    ]);
    fill_buffer(&device, &indirect, &indirect_data);
    let target = rgba_texture(&device, 8, 8, U::COLOR_TARGET | U::SAMPLER, 1);

    let mut cb = device.acquire_command_buffer().unwrap();
    {
        let mut pass = cb
            .begin_render_pass(&[clear_target(&target, BLUE_F)], None)
            .unwrap();
        pass.bind_graphics_pipeline(&pipeline);

        // Indexed: the top-left quadrant.
        pass.bind_vertex_buffers(
            0,
            &[BufferBinding {
                buffer: &corners,
                offset: 0,
            }],
        );
        pass.bind_index_buffer(
            &BufferBinding {
                buffer: &indices,
                offset: 0,
            },
            IndexElementSize::Bits16,
        );
        pass.command_buffer()
            .push_vertex_uniform_data(0, &float_bytes(&[0.0; 4]))
            .unwrap();
        pass.draw_indexed_primitives(6, 1, 0, 0, 0).unwrap();

        // Indexed indirect: the top-right quadrant.
        pass.command_buffer()
            .push_vertex_uniform_data(0, &float_bytes(&[1.0, 0.0, 0.0, 0.0]))
            .unwrap();
        pass.draw_indexed_primitives_indirect(&indirect, 20, 1)
            .unwrap();

        // Indirect: the bottom-right quadrant.
        pass.bind_vertex_buffers(
            0,
            &[BufferBinding {
                buffer: &vertices,
                offset: 0,
            }],
        );
        pass.command_buffer()
            .push_vertex_uniform_data(0, &float_bytes(&[1.0, -1.0, 0.0, 0.0]))
            .unwrap();
        pass.draw_primitives_indirect(&indirect, 4, 1).unwrap();
    }
    submit_and_wait(&device, cb);
    assert_eq!(
        texture_pixels(&device, &target, 0, 8, 8),
        quadrants([true, true, false, true])
    );
    device.wait_for_idle().unwrap();
}

#[test]
fn draw_with_samplers_storage_and_fragment_uniforms() {
    let _l = crate::test_support::test_lock();
    let _v = Video::init();
    let Some(device) = front_end_device() else {
        return;
    };
    use crate::gpu::BufferUsageFlags as B;
    use TextureUsageFlags as U;

    let vertex = msl_shader(
        &device,
        VERTEX_INPUT_UNIFORM,
        ShaderStage::Vertex,
        0,
        0,
        0,
        1,
    );
    let fragment = msl_shader(
        &device,
        FRAGMENT_SAMPLED_UNIFORM,
        ShaderStage::Fragment,
        1,
        0,
        0,
        1,
    );
    let pipeline = position_pipeline(&device, &vertex, &fragment);
    drop(fragment);

    // A triangle over the whole target.
    let vertices = gpu_buffer(&device, B::VERTEX, 24);
    fill_buffer(
        &device,
        &vertices,
        &float_bytes(&[-1.0, -1.0, 3.0, -1.0, -1.0, 3.0]),
    );
    let source = rgba_texture(&device, 2, 2, U::SAMPLER, 1);
    fill_texture(&device, &source, 0, 2, 2, &solid(2, 2, [0, 255, 0, 255]));
    let sampler = device
        .create_sampler(&SamplerCreateInfo::default())
        .unwrap();
    let target = rgba_texture(&device, 4, 4, U::COLOR_TARGET | U::SAMPLER, 1);

    let mut cb = device.acquire_command_buffer().unwrap();
    {
        let mut pass = cb
            .begin_render_pass(&[clear_target(&target, BLUE_F)], None)
            .unwrap();
        pass.bind_graphics_pipeline(&pipeline);
        pass.bind_vertex_buffers(
            0,
            &[BufferBinding {
                buffer: &vertices,
                offset: 0,
            }],
        );
        pass.bind_fragment_samplers(
            0,
            &[TextureSamplerBinding {
                texture: &source,
                sampler: &sampler,
            }],
        )
        .unwrap();
        pass.command_buffer()
            .push_vertex_uniform_data(0, &float_bytes(&[0.0; 4]))
            .unwrap();
        // (the texture times the tint)
        pass.command_buffer()
            .push_fragment_uniform_data(0, &float_bytes(&[1.0, 0.2, 1.0, 1.0]))
            .unwrap();
        pass.draw_primitives(3, 1, 0, 0).unwrap();
    }
    submit_and_wait(&device, cb);
    assert_eq!(
        texture_pixels(&device, &target, 0, 4, 4),
        solid(4, 4, [0, 51, 0, 255])
    );

    // Fragment storage: a buffer of colors and a texture of scales.
    let fragment = msl_shader(&device, FRAGMENT_STORAGE, ShaderStage::Fragment, 0, 1, 1, 0);
    let storage_pipeline = position_pipeline(&device, &vertex, &fragment);
    let colors = gpu_buffer(&device, B::GRAPHICS_STORAGE_READ, 32);
    fill_buffer(
        &device,
        &colors,
        &float_bytes(&[9.0, 9.0, 9.0, 9.0, 1.0, 0.6, 0.2, 1.0]),
    );
    let scale = rgba_texture(&device, 1, 1, U::GRAPHICS_STORAGE_READ, 1);
    fill_texture(&device, &scale, 0, 1, 1, &[255, 128, 255, 255]);
    let mut cb = device.acquire_command_buffer().unwrap();
    {
        let mut pass = cb
            .begin_render_pass(&[clear_target(&target, BLUE_F)], None)
            .unwrap();
        pass.bind_graphics_pipeline(&storage_pipeline);
        pass.bind_vertex_buffers(
            0,
            &[BufferBinding {
                buffer: &vertices,
                offset: 0,
            }],
        );
        pass.bind_fragment_storage_buffers(0, &[&colors]).unwrap();
        pass.bind_fragment_storage_textures(0, &[&scale]).unwrap();
        pass.command_buffer()
            .push_vertex_uniform_data(0, &float_bytes(&[0.0; 4]))
            .unwrap();
        pass.draw_primitives(3, 1, 0, 0).unwrap();
    }
    submit_and_wait(&device, cb);
    // (0.6 * 128/255 = 0.3012: 77; 0.2: 51)
    assert_eq!(
        texture_pixels(&device, &target, 0, 4, 4),
        solid(4, 4, [255, 77, 51, 255])
    );
    device.wait_for_idle().unwrap();
}

#[test]
fn draw_with_interpolated_colors_as_upstream() {
    let _l = crate::test_support::test_lock();
    let _v = Video::init();
    let Some(device) = front_end_device() else {
        return;
    };
    use TextureUsageFlags as U;

    let vertex = msl_shader(&device, GRADIENT_VERTEX, ShaderStage::Vertex, 0, 0, 0, 1);
    let fragment = msl_shader(
        &device,
        GRADIENT_FRAGMENT,
        ShaderStage::Fragment,
        0,
        0,
        0,
        0,
    );
    let color_target = [ColorTargetDescription {
        format: TextureFormat::R8G8B8A8_UNORM,
        blend_state: Default::default(),
    }];
    let pipeline = device
        .create_graphics_pipeline(&GraphicsPipelineCreateInfo {
            vertex_shader: &vertex,
            fragment_shader: &fragment,
            vertex_input_state: Default::default(),
            primitive_type: PrimitiveType::TriangleList,
            rasterizer_state: Default::default(),
            multisample_state: Default::default(),
            depth_stencil_state: Default::default(),
            target_info: GraphicsPipelineTargetInfo {
                color_target_descriptions: &color_target,
                depth_stencil_format: TextureFormat::INVALID,
                has_depth_stencil_target: false,
            },
            props: None,
        })
        .unwrap();
    let target = rgba_texture(&device, 8, 8, U::COLOR_TARGET | U::SAMPLER, 1);
    let mut cb = device.acquire_command_buffer().unwrap();
    {
        let mut pass = cb
            .begin_render_pass(
                &[ColorTargetInfo {
                    load_op: LoadOp::Clear,
                    ..ColorTargetInfo::new(&target)
                }],
                None,
            )
            .unwrap();
        pass.bind_graphics_pipeline(&pipeline);
        pass.command_buffer()
            .push_vertex_uniform_data(0, &float_bytes(&[1.0, 0.5, 0.75, 1.0]))
            .unwrap();
        pass.draw_primitives(3, 1, 0, 0).unwrap();
    }
    submit_and_wait(&device, cb);

    // (upstream's C on this GPU)
    let expected = hex(concat!(
        "80045aff700c5aff60145aff501c5aff40245aff302c5aff20345aff103c5aff",
        "8f044eff800c4eff70144eff601c4eff50244eff402c4eff30344eff203c4eff",
        "9f0442ff8f0c42ff801442ff701c42ff602442ff502c42ff403442ff303c42ff",
        "af0436ff9f0c36ff8f1436ff801c36ff702436ff602c36ff503436ff403c36ff",
        "bf042affaf0c2aff9f142aff8f1c2aff80242aff702c2aff60342aff503c2aff",
        "cf041effbf0c1effaf141eff9f1c1eff8f241eff802c1eff70341eff603c1eff",
        "df0412ffcf0c12ffbf1412ffaf1c12ff9f2412ff8f2c12ff803412ff703c12ff",
        "ef0406ffdf0c06ffcf1406ffbf1c06ffaf2406ff9f2c06ff8f3406ff803c06ff",
    ));
    assert_eq!(texture_pixels(&device, &target, 0, 8, 8), expected);
}

#[test]
fn compute_dispatches_write_storage_buffers_and_textures() {
    let _l = crate::test_support::test_lock();
    let _v = Video::init();
    let Some(device) = front_end_device() else {
        return;
    };
    use crate::gpu::BufferUsageFlags as B;
    use TextureUsageFlags as U;

    let pipeline = device
        .create_compute_pipeline(&ComputePipelineCreateInfo {
            code: COMPUTE_INDEX.as_bytes(),
            entrypoint: "",
            format: ShaderFormat::MSL,
            num_readwrite_storage_buffers: 1,
            num_uniform_buffers: 1,
            threadcount_x: 1,
            threadcount_y: 1,
            threadcount_z: 1,
            ..Default::default()
        })
        .unwrap();
    let storage = gpu_buffer(&device, B::COMPUTE_STORAGE_WRITE, 64);
    let indirect = gpu_buffer(&device, B::INDIRECT, 12);
    fill_buffer(&device, &indirect, &word_bytes(&[8, 1, 1]));

    // 16 invocations write i + 100, then an indirect dispatch of 8 writes
    // i + 1000 over the first ones.
    let mut cb = device.acquire_command_buffer().unwrap();
    for (add, indirect_dispatch) in [(100, false), (1000, true)] {
        let mut pass = cb
            .begin_compute_pass(
                &[],
                &[StorageBufferReadWriteBinding {
                    buffer: &storage,
                    cycle: false,
                }],
            )
            .unwrap();
        pass.bind_compute_pipeline(&pipeline);
        pass.command_buffer()
            .push_compute_uniform_data(0, &word_bytes(&[add]))
            .unwrap();
        if indirect_dispatch {
            pass.dispatch_indirect(&indirect, 0).unwrap();
        } else {
            pass.dispatch(16, 1, 1).unwrap();
        }
    }
    submit_and_wait(&device, cb);
    let expected: Vec<u32> = (0..16)
        .map(|i| if i < 8 { i + 1000 } else { i + 100 })
        .collect();
    assert_eq!(
        buffer_contents(&device, &storage, 64),
        word_bytes(&expected)
    );

    // A cycled storage buffer is a new one (the old one is in use by the
    // recorded command buffer): only the second pass's writes are there.
    // (The first pass doesn't cycle: once the finished command buffers are
    // cleaned, nothing uses the buffer.)
    device.wait_for_idle().unwrap();
    let mut cb = device.acquire_command_buffer().unwrap();
    for (add, groups) in [(5, 16), (7, 4)] {
        let mut pass = cb
            .begin_compute_pass(
                &[],
                &[StorageBufferReadWriteBinding {
                    buffer: &storage,
                    cycle: true,
                }],
            )
            .unwrap();
        pass.bind_compute_pipeline(&pipeline);
        pass.command_buffer()
            .push_compute_uniform_data(0, &word_bytes(&[add]))
            .unwrap();
        pass.dispatch(groups, 1, 1).unwrap();
    }
    submit_and_wait(&device, cb);
    let contents = buffer_contents(&device, &storage, 64);
    assert_eq!(&contents[..16], &word_bytes(&[7, 8, 9, 10])[..]);
    let storage_container = storage.raw.downcast::<BufferContainer>().unwrap();
    assert_eq!(lock(&storage_container.state).buffers.len(), 2);

    // A read-write storage texture (a view of one level), from a read-only
    // storage buffer.
    let image_pipeline = device
        .create_compute_pipeline(&ComputePipelineCreateInfo {
            code: COMPUTE_TEXTURE.as_bytes(),
            entrypoint: "",
            format: ShaderFormat::MSL,
            num_readonly_storage_buffers: 1,
            num_readwrite_storage_textures: 1,
            threadcount_x: 2,
            threadcount_y: 2,
            threadcount_z: 1,
            ..Default::default()
        })
        .unwrap();
    let scales = gpu_buffer(&device, B::COMPUTE_STORAGE_READ, 16);
    fill_buffer(&device, &scales, &float_bytes(&[0.25, 0.5, 1.0, 0.0]));
    let image = rgba_texture(&device, 4, 2, U::COMPUTE_STORAGE_WRITE | U::SAMPLER, 2);
    let mut cb = device.acquire_command_buffer().unwrap();
    {
        let mut pass = cb
            .begin_compute_pass(
                &[StorageTextureReadWriteBinding {
                    texture: &image,
                    mip_level: 0,
                    layer: 0,
                    cycle: false,
                }],
                &[],
            )
            .unwrap();
        pass.bind_compute_pipeline(&image_pipeline);
        pass.bind_compute_storage_buffers(0, &[&scales]).unwrap();
        pass.dispatch(2, 1, 1).unwrap();
    }
    submit_and_wait(&device, cb);
    let mut expected = Vec::new();
    for y in 0..2u32 {
        for x in 0..4u32 {
            let r = (x as f32 * 0.25 * 255.0).round() as u8;
            let g = (y as f32 * 0.5 * 255.0).round() as u8;
            expected.extend([r, g, 255, 255]);
        }
    }
    assert_eq!(texture_pixels(&device, &image, 0, 4, 2), expected);
    device.wait_for_idle().unwrap();
}

#[test]
fn blits_and_mipmaps_as_upstream() {
    let _l = crate::test_support::test_lock();
    let _v = Video::init();
    let Some(device) = front_end_device() else {
        return;
    };
    use TextureUsageFlags as U;

    // Mipmaps of the pattern (generateMipmapsForTexture), as upstream's.
    let mipmapped = rgba_texture(&device, 8, 8, U::SAMPLER | U::COLOR_TARGET, 4);
    fill_texture(&device, &mipmapped, 0, 8, 8, &pattern());
    let mut cb = device.acquire_command_buffer().unwrap();
    cb.generate_mipmaps(&mipmapped).unwrap();
    submit_and_wait(&device, cb);
    assert_eq!(
        texture_pixels(&device, &mipmapped, 1, 4, 4),
        hex(concat!(
            "111109fe4f1b2bfb8d254df8cb2f6ff5174b2bfb555509ec935f6fddd1694dce",
            "1d854df85b8f6fdd999909c2d7a32ba723bf6ff561c94dce9fd32ba7dddd0980",
        ))
    );
    assert_eq!(
        texture_pixels(&device, &mipmapped, 2, 2, 2),
        hex("33331af8af475ee63fa75ee6bbbb1aa4")
    );
    assert_eq!(
        texture_pixels(&device, &mipmapped, 3, 1, 1),
        hex("77773cda")
    );

    // A linear, vertically flipped blit into a 5x3 corner of a target
    // cleared to a color, through the blit pipelines, as upstream's.
    let destination = rgba_texture(&device, 8, 8, U::SAMPLER | U::COLOR_TARGET, 1);
    let mut cb = device.acquire_command_buffer().unwrap();
    cb.blit_texture(&BlitInfo {
        source: BlitRegion {
            texture: &mipmapped,
            mip_level: 0,
            layer_or_depth_plane: 0,
            x: 0,
            y: 0,
            w: 8,
            h: 8,
        },
        destination: BlitRegion {
            texture: &destination,
            mip_level: 0,
            layer_or_depth_plane: 0,
            x: 2,
            y: 1,
            w: 5,
            h: 3,
        },
        load_op: LoadOp::Clear,
        clear_color: FColor {
            r: 0.25,
            g: 0.5,
            b: 0.75,
            a: 1.0,
        },
        flip_mode: FlipMode::Vertical,
        filter: Filter::Linear,
        cycle: false,
    })
    .unwrap();
    submit_and_wait(&device, cb);
    let background = "4080bfff".repeat(8);
    let expected = hex(&[
        background.clone(),
        "4080bfff4080bfff1cb46cf94dbc4bdc7fc43cbeb1cc2ca1e2d40b834080bfff".to_owned(),
        "4080bfff4080bfff14673cfc456f3ceb77773cdaa97f3cc9da873cb94080bfff".to_owned(),
        "4080bfff4080bfff0c1a0bfe3d222cfa6f2a3cf6a1324bf2d23a6cee4080bfff".to_owned(),
        background.repeat(4),
    ]
    .concat());
    assert_eq!(texture_pixels(&device, &destination, 0, 8, 8), expected);

    // The pipeline for the destination's format was made once and cached.
    let r = device_renderer(&device);
    match &lock(&r.blit).pipelines {
        BlitPipelineCache::PerFormat(pipelines) => assert_eq!(pipelines.len(), 1),
        BlitPipelineCache::FormatAgnostic(_) => panic!("per-format pipelines"),
    }
    device.wait_for_idle().unwrap();
}

#[test]
fn fences_and_deferred_destruction() {
    let _l = crate::test_support::test_lock();
    let _v = Video::init();
    let Some(device) = front_end_device() else {
        return;
    };
    use TextureUsageFlags as U;

    let first = device
        .acquire_command_buffer()
        .unwrap()
        .submit_and_acquire_fence()
        .unwrap();
    let second = device
        .acquire_command_buffer()
        .unwrap()
        .submit_and_acquire_fence()
        .unwrap();
    device.wait_for_fences(false, &[&first, &second]).unwrap();
    device.wait_for_fences(true, &[&first, &second]).unwrap();
    assert!(first.is_signaled());
    assert!(second.is_signaled());

    // Once the command buffers are cleaned, the fences are only the
    // application's.
    let renderer = device_renderer(&device);
    device.wait_for_idle().unwrap();
    assert!(lock(&renderer.submitted_command_buffers).is_empty());
    let handle = first.raw.downcast::<MetalFence>().unwrap();
    assert_eq!(handle.reference_count.load(Ordering::SeqCst), 1);

    // A released fence goes back to the pool, and is reused.
    let pooled = lock(&renderer.available_fences).len();
    drop(first);
    assert_eq!(lock(&renderer.available_fences).len(), pooled + 1);
    let third = device
        .acquire_command_buffer()
        .unwrap()
        .submit_and_acquire_fence()
        .unwrap();
    assert!(Arc::ptr_eq(
        &third.raw.downcast::<MetalFence>().unwrap(),
        &handle
    ));
    device.wait_for_fences(true, &[&third]).unwrap();
    assert!(third.is_signaled());
    drop((second, third));

    // A texture released while a submitted command buffer uses it is
    // destroyed once that command buffer is cleaned.
    let texture = rgba_texture(&device, 64, 64, U::COLOR_TARGET | U::SAMPLER, 1);
    let mut cb = device.acquire_command_buffer().unwrap();
    cb.begin_render_pass(&[clear_target(&texture, BLUE_F)], None)
        .unwrap()
        .end();
    let metal_texture = container_of(&texture).active();
    drop(texture);
    assert_eq!(
        lock(&renderer.dispose).texture_containers_to_destroy.len(),
        1
    );
    assert_eq!(metal_texture.reference_count.load(Ordering::SeqCst), 1);
    cb.submit().unwrap();
    device.wait_for_idle().unwrap();
    assert!(lock(&renderer.dispose)
        .texture_containers_to_destroy
        .is_empty());
    assert_eq!(metal_texture.reference_count.load(Ordering::SeqCst), 0);
    // (only this test's reference is left)
    assert_eq!(Arc::strong_count(&metal_texture), 1);

    // A cancelled command buffer goes back to the pool without a fence.
    let available = lock(&renderer.available_command_buffers).buffers.len();
    device.acquire_command_buffer().unwrap().cancel().unwrap();
    assert_eq!(
        lock(&renderer.available_command_buffers).buffers.len(),
        available
    );

    // Frames in flight are taken after waiting.
    device.set_allowed_frames_in_flight(3).unwrap();
    assert_eq!(renderer.allowed_frames_in_flight.load(Ordering::SeqCst), 3);
}

#[test]
fn swapchain_queries_without_a_window() {
    let _l = crate::test_support::test_lock();
    let _v = Video::init();
    let Some(device) = front_end_device() else {
        return;
    };
    let r = device_renderer(&device);
    let window = crate::video::Window::create("metal", 32, 32, Default::default()).unwrap();

    assert!(r.supports_present_mode_internal(window, PresentMode::Vsync));
    // (immediate presentation and HDR10 swapchains: macOS only)
    assert_eq!(
        r.supports_present_mode_internal(window, PresentMode::Immediate),
        cfg!(target_os = "macos")
    );
    assert!(!r.supports_present_mode_internal(window, PresentMode::Mailbox));
    for composition in [
        SwapchainComposition::Sdr,
        SwapchainComposition::SdrLinear,
        SwapchainComposition::HdrExtendedLinear,
        SwapchainComposition::Hdr10St2084,
    ] {
        assert_eq!(
            r.supports_swapchain_composition_internal(window, composition),
            cfg!(target_os = "macos") || composition != SwapchainComposition::Hdr10St2084
        );
    }
    // An unclaimed window has no swapchain.
    assert_eq!(
        r.swapchain_texture_format_internal(window)
            .unwrap_err()
            .message(),
        "Cannot get swapchain format, window has not been claimed"
    );
    assert!(r.wait_for_swapchain_internal(window).is_err());
    // The offscreen driver has no Metal views (the front end checks for
    // them as PrepareDriver does).
    assert!(!METAL_DRIVER.name.is_empty());
    let video = crate::video::core::driver().unwrap();
    assert!(!video.implements_metal_views());
    let props = Properties::new();
    props
        .set(crate::gpu::PROP_GPU_DEVICE_CREATE_SHADERS_MSL_BOOLEAN, true)
        .unwrap();
    assert!(!device::prepare_driver(&*video, &props));
}
