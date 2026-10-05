// Tests of the Direct3D 11 renderer.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! The tables are checked against upstream's values, and the declarations
//! against the `sizeof`/`offsetof` values and vtable slots of a C harness
//! built with mingw-w64's `<d3d11_1.h>`, `<dxgi1_5.h>` and `<dxgidebug.h>`.
//! The scenes are drawn by the Direct3D 11 renderer on a real device (on
//! Linux CI, Wine's d3d11 over wined3d and Mesa) and by the software
//! renderer, and the pixels compared: exactly where the math is exact
//! (clears, fills, points, 1:1 copies, palettes), within a small tolerance
//! where the GPU's float blending or YUV conversion rounds differently.
//! Without a device (or a desktop to put the window on) the device tests
//! report a skip (capability `d3d11`, or `desktop`) and pass.
//!
//! Wine's `IDXGISwapChain1::SetRotation()` is a stub that fails with
//! `E_NOTIMPL`, which makes the flip-model swap chain of an ordinary
//! window fail at creation, as upstream's does (checked by
//! [`flip_model_swap_chain_under_wine`]). Under Wine the scenes are drawn
//! into transparent windows instead, whose swap chains use
//! `DXGI_SWAP_EFFECT_DISCARD` and aren't rotated.

use std::mem::{offset_of, size_of};

use super::shaders::Shader;
use super::*;
use crate::core::windows::is_wine;
use crate::events::queue::{get_events, pump};
use crate::init::{self, InitFlags};
use crate::render::{
    num_render_drivers, render_driver, Renderer, Texture, TextureAccess, TextureCreateInfo, Vertex,
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
fn driver_list_follows_upstreams_order() {
    // (render_drivers[] in SDL_render.c: D3D11, D3D12, D3D, METAL, ...,
    // OGL, OGL_ES2, ..., VULKAN, GPU, SW)
    assert_eq!(num_render_drivers(), 6);
    assert_eq!(render_driver(0).unwrap(), "direct3d11");
    assert_eq!(render_driver(1).unwrap(), "opengl");
    assert_eq!(render_driver(2).unwrap(), "opengles2");
    assert_eq!(render_driver(3).unwrap(), "vulkan");
    assert_eq!(render_driver(4).unwrap(), "gpu");
    assert_eq!(render_driver(5).unwrap(), SOFTWARE_RENDERER);
    assert!(render_driver(6).is_err());
}

#[test]
fn dxbc_shaders() {
    // The sizes of the arrays in upstream's D3D11_*.h headers.
    let sizes = [
        (Shader::Solid, 1248),
        (Shader::SolidPq, 1908),
        (Shader::Rgb, 1440),
        (Shader::Advanced, 9196),
        (Shader::RgbPq, 8436),
        (Shader::RgbSimple, 724),
    ];
    let vs = shaders::vertex_shader();
    assert_eq!(vs.len(), 1420);
    for (shader, bytes) in sizes {
        let ps = shader.pixel_shader();
        assert_eq!(ps.len(), bytes, "{shader:?}");
        for blob in [ps, vs] {
            // DXBC magic, version 1, and the container's own size
            assert_eq!(&blob[..4], b"DXBC");
            assert_eq!(&blob[20..24], &[1, 0, 0, 0]);
            assert_eq!(
                u32::from_le_bytes(blob[24..28].try_into().unwrap()) as usize,
                blob.len()
            );
        }
    }
    assert_eq!(Shader::ALL.len(), Shader::COUNT);
    for (i, shader) in Shader::ALL.iter().enumerate() {
        assert_eq!(*shader as usize, i);
    }
    // The first and last bytes of D3D11_PixelShader_Colors.h
    let colors = Shader::Solid.pixel_shader();
    assert_eq!(
        &colors[..16],
        &[68, 88, 66, 67, 131, 2, 46, 215, 253, 13, 129, 98, 132, 106, 250, 166]
    );
    assert_eq!(
        &colors[colors.len() - 12..],
        &[83, 86, 95, 84, 65, 82, 71, 69, 84, 0, 171, 171]
    );
}

#[test]
fn format_tables() {
    use PixelFormat as F;
    assert_eq!(
        dxgi_format_to_sdl_pixel_format(DXGI_FORMAT_B8G8R8A8_UNORM),
        F::ARGB8888
    );
    assert_eq!(
        dxgi_format_to_sdl_pixel_format(DXGI_FORMAT_B8G8R8A8_UNORM_SRGB),
        F::ARGB8888
    );
    assert_eq!(
        dxgi_format_to_sdl_pixel_format(DXGI_FORMAT_R8G8B8A8_UNORM_SRGB),
        F::ABGR8888
    );
    assert_eq!(
        dxgi_format_to_sdl_pixel_format(DXGI_FORMAT_B8G8R8X8_UNORM),
        F::XRGB8888
    );
    assert_eq!(
        dxgi_format_to_sdl_pixel_format(DXGI_FORMAT_B4G4R4A4_UNORM),
        F::ARGB4444
    );
    assert_eq!(
        dxgi_format_to_sdl_pixel_format(DXGI_FORMAT_R16G16B16A16_FLOAT),
        F::RGBA64_FLOAT
    );
    assert_eq!(
        dxgi_format_to_sdl_pixel_format(DXGI_FORMAT_R8_UNORM),
        F::UNKNOWN
    );

    let srgb = Colorspace::SRGB;
    let linear = Colorspace::SRGB_LINEAR;
    let tex = sdl_pixel_format_to_dxgi_texture_format;
    assert_eq!(tex(F::ARGB8888, srgb), DXGI_FORMAT_B8G8R8A8_UNORM);
    assert_eq!(tex(F::ARGB8888, linear), DXGI_FORMAT_B8G8R8A8_UNORM_SRGB);
    assert_eq!(
        tex(F::XRGB8888, Colorspace::HDR10),
        DXGI_FORMAT_B8G8R8X8_UNORM_SRGB
    );
    assert_eq!(tex(F::ABGR2101010, linear), DXGI_FORMAT_R10G10B10A2_UNORM);
    assert_eq!(tex(F::RGB565, srgb), DXGI_FORMAT_B5G6R5_UNORM);
    assert_eq!(tex(F::INDEX8, srgb), DXGI_FORMAT_R8_UNORM);
    assert_eq!(tex(F::IYUV, srgb), DXGI_FORMAT_R8_UNORM);
    assert_eq!(tex(F::I444, srgb), DXGI_FORMAT_R8_UNORM);
    assert_eq!(tex(F::NV21, srgb), DXGI_FORMAT_NV12);
    assert_eq!(tex(F::P010, srgb), DXGI_FORMAT_P010);
    assert_eq!(tex(F::I0FL, srgb), DXGI_FORMAT_R16_UNORM);
    assert_eq!(tex(F::RGBA4444, srgb), DXGI_FORMAT_UNKNOWN);
    assert_eq!(tex(F::XBGR8888, srgb), DXGI_FORMAT_UNKNOWN);

    let view = sdl_pixel_format_to_dxgi_main_resource_view_format;
    assert_eq!(view(F::NV12, srgb), DXGI_FORMAT_R8_UNORM);
    assert_eq!(view(F::NV21, srgb), DXGI_FORMAT_R8_UNORM);
    assert_eq!(view(F::P010, srgb), DXGI_FORMAT_R16_UNORM);
    assert_eq!(view(F::ABGR8888, linear), DXGI_FORMAT_R8G8B8A8_UNORM_SRGB);
    assert_eq!(view(F::YV12, srgb), DXGI_FORMAT_R8_UNORM);
}

#[test]
fn blend_modes() {
    let custom = |sc, dc, co, sa, da, ao| BlendMode::compose_custom(sc, dc, co, sa, da, ao);
    let desc = blend_desc(BlendMode::BLEND);
    assert_eq!(desc.alpha_to_coverage_enable, 0);
    assert_eq!(desc.independent_blend_enable, 0);
    let rt = desc.render_target[0];
    assert_eq!(rt.blend_enable, 1);
    assert_eq!(rt.src_blend, D3D11_BLEND_SRC_ALPHA);
    assert_eq!(rt.dest_blend, D3D11_BLEND_INV_SRC_ALPHA);
    assert_eq!(rt.blend_op, D3D11_BLEND_OP_ADD);
    assert_eq!(rt.src_blend_alpha, D3D11_BLEND_ONE);
    assert_eq!(rt.dest_blend_alpha, D3D11_BLEND_INV_SRC_ALPHA);
    assert_eq!(rt.blend_op_alpha, D3D11_BLEND_OP_ADD);
    assert_eq!(rt.render_target_write_mask, D3D11_COLOR_WRITE_ENABLE_ALL);
    // (the other render targets stay zeroed)
    assert_eq!(desc.render_target[1].blend_enable, 0);

    let rt = blend_desc(BlendMode::MOD).render_target[0];
    assert_eq!(
        (
            rt.src_blend,
            rt.dest_blend,
            rt.src_blend_alpha,
            rt.dest_blend_alpha
        ),
        (
            D3D11_BLEND_ZERO,
            D3D11_BLEND_SRC_COLOR,
            D3D11_BLEND_ZERO,
            D3D11_BLEND_ONE
        )
    );
    let rt = blend_desc(BlendMode::MUL).render_target[0];
    assert_eq!(
        (rt.src_blend, rt.dest_blend),
        (D3D11_BLEND_DEST_COLOR, D3D11_BLEND_INV_SRC_ALPHA)
    );

    assert_eq!(
        get_blend_func(Some(BF::OneMinusDstColor)),
        D3D11_BLEND_INV_DEST_COLOR
    );
    assert_eq!(get_blend_func(Some(BF::DstAlpha)), D3D11_BLEND_DEST_ALPHA);
    assert_eq!(get_blend_func(None), 0);
    assert_eq!(
        get_blend_equation(Some(BO::Subtract)),
        D3D11_BLEND_OP_SUBTRACT
    );
    assert_eq!(get_blend_equation(Some(BO::Minimum)), D3D11_BLEND_OP_MIN);
    assert_eq!(get_blend_equation(Some(BO::Maximum)), D3D11_BLEND_OP_MAX);
    assert_eq!(get_blend_equation(None), 0);

    let mode = custom(
        BF::DstColor,
        BF::Zero,
        BO::RevSubtract,
        BF::One,
        BF::OneMinusDstAlpha,
        BO::Maximum,
    );
    let rt = blend_desc(mode).render_target[0];
    assert_eq!(
        (
            rt.src_blend,
            rt.dest_blend,
            rt.blend_op,
            rt.src_blend_alpha,
            rt.dest_blend_alpha,
            rt.blend_op_alpha
        ),
        (
            D3D11_BLEND_DEST_COLOR,
            D3D11_BLEND_ZERO,
            D3D11_BLEND_OP_REV_SUBTRACT,
            D3D11_BLEND_ONE,
            D3D11_BLEND_INV_DEST_ALPHA,
            D3D11_BLEND_OP_MAX
        )
    );
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

    let desc = sampler_desc(ScaleMode::Nearest, A::Clamp, A::Wrap).unwrap();
    assert_eq!(desc.filter, D3D11_FILTER_MIN_MAG_MIP_POINT);
    assert_eq!(desc.address_u, D3D11_TEXTURE_ADDRESS_CLAMP);
    assert_eq!(desc.address_v, D3D11_TEXTURE_ADDRESS_WRAP);
    assert_eq!(desc.address_w, D3D11_TEXTURE_ADDRESS_CLAMP);
    assert_eq!(desc.max_anisotropy, 1);
    assert_eq!(desc.comparison_func, D3D11_COMPARISON_ALWAYS);
    assert_eq!(desc.max_lod.to_bits(), 0x7f7f_ffff); // (FLT_MAX)
    let desc = sampler_desc(ScaleMode::PixelArt, A::Wrap, A::Clamp).unwrap();
    assert_eq!(desc.filter, D3D11_FILTER_MIN_MAG_MIP_LINEAR);
    assert_eq!(
        sampler_desc(ScaleMode::Linear, A::Auto, A::Clamp)
            .unwrap_err()
            .message(),
        "Unknown texture address mode: 0"
    );

    let mut constants = PixelShaderConstants {
        color_scale: 1.0,
        sdr_white_point: 80.0,
        texture_type: TEXTURETYPE_RGB,
        ..Default::default()
    };
    assert!(!pq_shader_scales_input(&constants));
    let srgb = Colorspace::SRGB;
    let hdr10 = Colorspace::HDR10;
    assert_eq!(select_shader(srgb, None), Shader::Solid);
    assert_eq!(select_shader(hdr10, None), Shader::SolidPq);
    assert_eq!(select_shader(srgb, Some(&constants)), Shader::Rgb);
    assert_eq!(select_shader(hdr10, Some(&constants)), Shader::RgbPq);
    constants.input_type = INPUTTYPE_HDR10;
    assert_eq!(select_shader(hdr10, Some(&constants)), Shader::RgbSimple);
    assert_eq!(select_shader(srgb, Some(&constants)), Shader::Advanced);
    constants.sdr_white_point = 203.0;
    assert!(pq_shader_scales_input(&constants));
    assert_eq!(select_shader(hdr10, Some(&constants)), Shader::RgbPq);
    constants.sdr_white_point = 80.0;
    constants.tonemap_method = TONEMAP_CHROME;
    assert!(pq_shader_scales_input(&constants));
    constants.input_type = INPUTTYPE_UNSPECIFIED;
    assert_eq!(select_shader(srgb, Some(&constants)), Shader::Advanced);
    constants.tonemap_method = TONEMAP_NONE;
    constants.texture_type = TEXTURETYPE_PALETTE_NEAREST;
    assert_eq!(select_shader(srgb, Some(&constants)), Shader::Advanced);

    // (the layouts the shaders read)
    assert_eq!(size_of::<PixelShaderConstants>(), 112);
    assert_eq!(size_of::<VertexShaderConstants>(), 128);
    assert_eq!(size_of::<VertexPositionColor>(), 32);
    assert_eq!(offset_of!(VertexPositionColor, tex), 8);
    assert_eq!(offset_of!(VertexPositionColor, color), 16);
    assert_eq!(offset_of!(PixelShaderConstants, tonemap_method), 32);
    assert_eq!(offset_of!(PixelShaderConstants, ycbcr_matrix), 48);
    let a = PixelShaderConstants::default();
    let mut b = a;
    assert!(a.same_bits(&b));
    b.ycbcr_matrix[15] = -0.0;
    assert!(!a.same_bits(&b));
    assert_eq!(b.words()[27], 0x8000_0000);
}

#[test]
fn matrices_and_rotations() {
    let id = Float4X4::identity();
    let r = Float4X4::rotation_z(std::f32::consts::PI * 0.5);
    assert!(Float4X4::multiply(&id, &r).same_bits(&r));
    assert!(Float4X4::multiply(&r, &id).same_bits(&r));
    assert_eq!(r.m[0][1], 1.0);
    assert_eq!(r.m[1][0], -1.0);
    let rr = Float4X4::multiply(&r, &r);
    assert!((rr.m[0][0] + 1.0).abs() < 1e-6);
    assert!(is_display_rotated_90_degrees(DXGI_MODE_ROTATION_ROTATE90));
    assert!(is_display_rotated_90_degrees(DXGI_MODE_ROTATION_ROTATE270));
    assert!(!is_display_rotated_90_degrees(DXGI_MODE_ROTATION_ROTATE180));
    assert!(!is_display_rotated_90_degrees(DXGI_MODE_ROTATION_IDENTITY));
    assert_eq!(get_current_rotation(), DXGI_MODE_ROTATION_IDENTITY);
}

#[test]
fn constants_match_the_headers() {
    // The values a mingw-w64 C harness prints for the headers' names.
    assert_eq!(
        [
            DXGI_FORMAT_R32G32B32A32_FLOAT,
            DXGI_FORMAT_R16G16B16A16_FLOAT,
            DXGI_FORMAT_R32G32_FLOAT,
            DXGI_FORMAT_R10G10B10A2_UNORM,
            DXGI_FORMAT_R8G8B8A8_UNORM,
            DXGI_FORMAT_R8G8B8A8_UNORM_SRGB,
            DXGI_FORMAT_R16G16_UNORM,
            DXGI_FORMAT_R8G8_UNORM,
            DXGI_FORMAT_R16_UNORM,
            DXGI_FORMAT_R8_UNORM,
            DXGI_FORMAT_B5G6R5_UNORM,
            DXGI_FORMAT_B5G5R5A1_UNORM,
            DXGI_FORMAT_B8G8R8A8_UNORM,
            DXGI_FORMAT_B8G8R8X8_UNORM,
            DXGI_FORMAT_B8G8R8A8_UNORM_SRGB,
            DXGI_FORMAT_B8G8R8X8_UNORM_SRGB,
            DXGI_FORMAT_NV12,
            DXGI_FORMAT_P010,
            DXGI_FORMAT_B4G4R4A4_UNORM,
        ],
        [
            0x2, 0xa, 0x10, 0x18, 0x1c, 0x1d, 0x23, 0x31, 0x38, 0x3d, 0x55, 0x56, 0x57, 0x58, 0x5b,
            0x5d, 0x67, 0x68, 0x73
        ]
    );
    assert_eq!(
        [
            D3D11_BIND_VERTEX_BUFFER,
            D3D11_BIND_CONSTANT_BUFFER,
            D3D11_BIND_SHADER_RESOURCE,
            D3D11_BIND_RENDER_TARGET,
            D3D11_CPU_ACCESS_WRITE,
            D3D11_CPU_ACCESS_READ,
            D3D11_FILTER_MIN_MAG_MIP_LINEAR,
            D3D11_FORMAT_SUPPORT_TEXTURE2D,
            D3D11_CREATE_DEVICE_BGRA_SUPPORT,
            D3D_FEATURE_LEVEL_11_0,
            D3D_FEATURE_LEVEL_11_1,
            DXGI_SWAP_CHAIN_FLAG_ALLOW_TEARING,
            DXGI_PRESENT_DO_NOT_WAIT,
            DXGI_PRESENT_ALLOW_TEARING,
            DXGI_USAGE_RENDER_TARGET_OUTPUT,
            DXGI_COLOR_SPACE_RGB_FULL_G2084_NONE_P2020,
        ],
        [
            0x1, 0x4, 0x8, 0x20, 0x10000, 0x20000, 0x15, 0x20, 0x20, 0xb000, 0xb100, 0x800, 0x8,
            0x200, 0x20, 0xc
        ]
    );
    assert_eq!(
        [
            DXGI_ERROR_INVALID_CALL,
            DXGI_ERROR_DEVICE_REMOVED,
            DXGI_ERROR_WAS_STILL_DRAWING,
            E_FAIL
        ]
        .map(|hr| hr as u32),
        [0x887a0001, 0x887a0005, 0x887a000a, 0x80004005]
    );
    assert_eq!(D3D11_FLOAT32_MAX, f32::from_bits(0x7f7f_ffff));
    // A GUID's fields, as SDL_render_d3d11.c spells them out.
    assert_eq!(DXGI_DEBUG_ALL.data1, 0xe48ae283);
    assert_eq!(
        DXGI_DEBUG_ALL.data4,
        [0x87, 0xe6, 0x43, 0xe9, 0xa9, 0xcf, 0xda, 0x8]
    );
    assert_eq!(IID_IDXGIINFOQUEUE.data2, 0x672A);
    assert_eq!(IID_ID3D11DEVICE1.data3, 0x43d6);
}

#[test]
fn structures_match_the_headers() {
    // sizeof/offsetof of a mingw-w64 C harness (x86_64)
    assert_eq!(size_of::<Texture2dDesc>(), 44);
    assert_eq!(offset_of!(Texture2dDesc, format), 16);
    assert_eq!(offset_of!(Texture2dDesc, sample_desc), 20);
    assert_eq!(offset_of!(Texture2dDesc, usage), 28);
    assert_eq!(offset_of!(Texture2dDesc, bind_flags), 32);
    assert_eq!(offset_of!(Texture2dDesc, misc_flags), 40);
    assert_eq!(size_of::<BufferDesc>(), 24);
    assert_eq!(offset_of!(BufferDesc, structure_byte_stride), 20);
    assert_eq!(size_of::<SubresourceData>(), 16);
    assert_eq!(offset_of!(SubresourceData, sys_mem_pitch), 8);
    assert_eq!(size_of::<MappedSubresource>(), 16);
    assert_eq!(offset_of!(MappedSubresource, row_pitch), 8);
    assert_eq!(size_of::<ShaderResourceViewDesc>(), 24);
    assert_eq!(offset_of!(ShaderResourceViewDesc, view_dimension), 4);
    assert_eq!(offset_of!(ShaderResourceViewDesc, most_detailed_mip), 8);
    assert_eq!(size_of::<RenderTargetViewDesc>(), 20);
    assert_eq!(offset_of!(RenderTargetViewDesc, mip_slice), 8);
    assert_eq!(size_of::<BlendDesc>(), 264);
    assert_eq!(offset_of!(BlendDesc, render_target), 8);
    assert_eq!(size_of::<RenderTargetBlendDesc>(), 32);
    assert_eq!(
        offset_of!(RenderTargetBlendDesc, render_target_write_mask),
        28
    );
    assert_eq!(size_of::<RasterizerDesc>(), 40);
    assert_eq!(offset_of!(RasterizerDesc, depth_bias_clamp), 16);
    assert_eq!(offset_of!(RasterizerDesc, antialiased_line_enable), 36);
    assert_eq!(size_of::<SamplerDesc>(), 52);
    assert_eq!(offset_of!(SamplerDesc, mip_lod_bias), 16);
    assert_eq!(offset_of!(SamplerDesc, border_color), 28);
    assert_eq!(offset_of!(SamplerDesc, max_lod), 48);
    assert_eq!(size_of::<Viewport>(), 24);
    assert_eq!(size_of::<D3d11Rect>(), 16);
    assert_eq!(size_of::<D3d11Box>(), 24);
    assert_eq!(size_of::<InputElementDesc>(), 32);
    assert_eq!(offset_of!(InputElementDesc, semantic_index), 8);
    assert_eq!(offset_of!(InputElementDesc, aligned_byte_offset), 20);
    assert_eq!(offset_of!(InputElementDesc, instance_data_step_rate), 28);
    assert_eq!(size_of::<SwapChainDesc1>(), 48);
    assert_eq!(offset_of!(SwapChainDesc1, stereo), 12);
    assert_eq!(offset_of!(SwapChainDesc1, buffer_usage), 24);
    assert_eq!(offset_of!(SwapChainDesc1, scaling), 32);
    assert_eq!(offset_of!(SwapChainDesc1, flags), 44);
    assert_eq!(size_of::<PresentParameters>(), 32);
    assert_eq!(offset_of!(PresentParameters, dirty_rects), 8);
    assert_eq!(offset_of!(PresentParameters, scroll_rect), 16);
    assert_eq!(offset_of!(PresentParameters, scroll_offset), 24);
    assert_eq!(size_of::<AdapterDesc>(), 304);
    assert_eq!(offset_of!(AdapterDesc, vendor_id), 256);
    assert_eq!(offset_of!(AdapterDesc, dedicated_video_memory), 272);
    assert_eq!(offset_of!(AdapterDesc, adapter_luid), 296);
}

#[test]
fn vtable_slots_match_the_headers() {
    // offsetof(<Interface>Vtbl, <Method>) / sizeof(void *) of the harness.
    let ptr = size_of::<usize>();
    let slot = |offset: usize| offset / ptr;
    assert_eq!(slot(offset_of!(IDXGIFactory2Vtbl, enum_adapters)), 7);
    assert_eq!(
        slot(offset_of!(IDXGIFactory2Vtbl, make_window_association)),
        8
    );
    assert_eq!(
        slot(offset_of!(IDXGIFactory2Vtbl, create_swap_chain_for_hwnd)),
        15
    );
    assert_eq!(size_of::<IDXGIFactory2Vtbl>(), 25 * ptr);
    assert_eq!(
        slot(offset_of!(IDXGIFactory5Vtbl, check_feature_support)),
        28
    );
    assert_eq!(size_of::<IDXGIFactory5Vtbl>(), 29 * ptr);
    assert_eq!(slot(offset_of!(IDXGIAdapterVtbl, get_desc)), 8);
    assert_eq!(slot(offset_of!(IDXGIDevice1Vtbl, get_adapter)), 7);
    assert_eq!(
        slot(offset_of!(IDXGIDevice1Vtbl, set_maximum_frame_latency)),
        12
    );
    assert_eq!(slot(offset_of!(IDXGISwapChain1Vtbl, get_buffer)), 9);
    assert_eq!(
        slot(offset_of!(IDXGISwapChain1Vtbl, get_fullscreen_state)),
        11
    );
    assert_eq!(slot(offset_of!(IDXGISwapChain1Vtbl, resize_buffers)), 13);
    assert_eq!(slot(offset_of!(IDXGISwapChain1Vtbl, present1)), 22);
    assert_eq!(slot(offset_of!(IDXGISwapChain1Vtbl, set_rotation)), 27);
    assert_eq!(size_of::<IDXGISwapChain1Vtbl>(), 29 * ptr);
    assert_eq!(
        slot(offset_of!(IDXGISwapChain3Vtbl, check_color_space_support)),
        37
    );
    assert_eq!(slot(offset_of!(IDXGISwapChain3Vtbl, set_color_space1)), 38);
    assert_eq!(size_of::<IDXGISwapChain3Vtbl>(), 40 * ptr);
    assert_eq!(slot(offset_of!(IDXGIDebugVtbl, report_live_objects)), 3);
    assert_eq!(size_of::<IDXGIDebugVtbl>(), 4 * ptr);
    assert_eq!(
        slot(offset_of!(IDXGIInfoQueueVtbl, set_break_on_severity)),
        33
    );

    assert_eq!(slot(offset_of!(ID3D11DeviceVtbl, create_buffer)), 3);
    assert_eq!(slot(offset_of!(ID3D11DeviceVtbl, create_texture_2d)), 5);
    assert_eq!(
        slot(offset_of!(ID3D11DeviceVtbl, create_shader_resource_view)),
        7
    );
    assert_eq!(
        slot(offset_of!(ID3D11DeviceVtbl, create_render_target_view)),
        9
    );
    assert_eq!(slot(offset_of!(ID3D11DeviceVtbl, create_input_layout)), 11);
    assert_eq!(slot(offset_of!(ID3D11DeviceVtbl, create_vertex_shader)), 12);
    assert_eq!(slot(offset_of!(ID3D11DeviceVtbl, create_pixel_shader)), 15);
    assert_eq!(slot(offset_of!(ID3D11DeviceVtbl, create_blend_state)), 20);
    assert_eq!(
        slot(offset_of!(ID3D11DeviceVtbl, create_rasterizer_state)),
        22
    );
    assert_eq!(slot(offset_of!(ID3D11DeviceVtbl, create_sampler_state)), 23);
    assert_eq!(slot(offset_of!(ID3D11DeviceVtbl, check_format_support)), 29);

    let ctx = |offset| slot(offset);
    assert_eq!(
        ctx(offset_of!(ID3D11DeviceContextVtbl, vs_set_constant_buffers)),
        7
    );
    assert_eq!(
        ctx(offset_of!(ID3D11DeviceContextVtbl, ps_set_shader_resources)),
        8
    );
    assert_eq!(ctx(offset_of!(ID3D11DeviceContextVtbl, ps_set_shader)), 9);
    assert_eq!(
        ctx(offset_of!(ID3D11DeviceContextVtbl, ps_set_samplers)),
        10
    );
    assert_eq!(ctx(offset_of!(ID3D11DeviceContextVtbl, vs_set_shader)), 11);
    assert_eq!(ctx(offset_of!(ID3D11DeviceContextVtbl, draw)), 13);
    assert_eq!(ctx(offset_of!(ID3D11DeviceContextVtbl, map)), 14);
    assert_eq!(ctx(offset_of!(ID3D11DeviceContextVtbl, unmap)), 15);
    assert_eq!(
        ctx(offset_of!(ID3D11DeviceContextVtbl, ps_set_constant_buffers)),
        16
    );
    assert_eq!(
        ctx(offset_of!(ID3D11DeviceContextVtbl, ia_set_input_layout)),
        17
    );
    assert_eq!(
        ctx(offset_of!(ID3D11DeviceContextVtbl, ia_set_vertex_buffers)),
        18
    );
    assert_eq!(
        ctx(offset_of!(
            ID3D11DeviceContextVtbl,
            ia_set_primitive_topology
        )),
        24
    );
    assert_eq!(
        ctx(offset_of!(ID3D11DeviceContextVtbl, om_set_render_targets)),
        33
    );
    assert_eq!(
        ctx(offset_of!(ID3D11DeviceContextVtbl, om_set_blend_state)),
        35
    );
    assert_eq!(ctx(offset_of!(ID3D11DeviceContextVtbl, rs_set_state)), 43);
    assert_eq!(
        ctx(offset_of!(ID3D11DeviceContextVtbl, rs_set_viewports)),
        44
    );
    assert_eq!(
        ctx(offset_of!(ID3D11DeviceContextVtbl, rs_set_scissor_rects)),
        45
    );
    assert_eq!(
        ctx(offset_of!(ID3D11DeviceContextVtbl, copy_subresource_region)),
        46
    );
    assert_eq!(
        ctx(offset_of!(ID3D11DeviceContextVtbl, update_subresource)),
        48
    );
    assert_eq!(
        ctx(offset_of!(
            ID3D11DeviceContextVtbl,
            clear_render_target_view
        )),
        50
    );
    assert_eq!(ctx(offset_of!(ID3D11DeviceContextVtbl, clear_state)), 110);
    assert_eq!(ctx(offset_of!(ID3D11DeviceContextVtbl, flush)), 111);

    assert_eq!(slot(offset_of!(ID3D11Texture2DVtbl, get_desc)), 10);
    assert_eq!(size_of::<ID3D11Texture2DVtbl>(), 11 * ptr);
    assert_eq!(slot(offset_of!(ID3D11ViewVtbl, get_resource)), 7);
}

#[test]
fn upload_rows_are_bounds_checked() {
    // (the copies into mapped memory refuse a source that's too short,
    // where the C code trusts the caller)
    let mut dst = [0u8; 16];
    let src = [1u8, 2, 3, 4, 5, 6, 7, 8, 9, 10];
    // SAFETY: `dst` has 2 rows of 3 bytes, 8 apart.
    unsafe { copy_rows(dst.as_mut_ptr(), 8, &src, 5, 3, 2) }.unwrap();
    assert_eq!(dst, [1, 2, 3, 0, 0, 0, 0, 0, 6, 7, 8, 0, 0, 0, 0, 0]);
    // SAFETY: as above, but nothing is written.
    assert!(unsafe { copy_rows(dst.as_mut_ptr(), 8, &src, 5, 6, 2) }.is_err());
    assert_eq!(plane(&src, 10).unwrap().len(), 0);
    assert!(plane(&src, 11).is_err());
}

// ---------------------------------------------------------------------------
// With a device
// ---------------------------------------------------------------------------

/// Video up on the windows driver; quit when dropped.
struct Video;

impl Video {
    /// The windows video driver, or `None` (with the skip reported) when
    /// there's no desktop.
    fn init() -> Option<Video> {
        init::quit();
        hints::set(hints::VIDEO_DRIVER, "windows").unwrap();
        let result = init::init(InitFlags::VIDEO);
        hints::reset(hints::VIDEO_DRIVER);
        if let Err(e) = result {
            crate::test_support::skip(
                "desktop",
                format_args!("no desktop for the windows video driver ({e})"),
            );
            return None;
        }
        Some(Video)
    }
}

impl Drop for Video {
    fn drop(&mut self) {
        init::quit();
    }
}

/// The flags of the test windows: hidden, and under Wine transparent (see
/// the module documentation).
fn window_flags() -> WindowFlags {
    if is_wine() {
        WindowFlags::HIDDEN | WindowFlags::TRANSPARENT
    } else {
        WindowFlags::HIDDEN
    }
}

/// A window, or `None` (with the skip reported) when the windows video
/// driver can't make one.
fn window(w: i32, h: i32, flags: WindowFlags) -> Option<Window> {
    match Window::create("direct3d11", w, h, flags) {
        Ok(window) => Some(window),
        Err(e) => {
            crate::test_support::skip(
                "desktop",
                format_args!("the windows video driver can't create windows ({e})"),
            );
            None
        }
    }
}

/// A window with a Direct3D 11 renderer, or `None` (with the skip
/// reported) when there is no device.
fn d3d11_renderer(w: i32, h: i32) -> Option<(Window, Renderer)> {
    let window = window(w, h, window_flags())?;
    match Renderer::for_window(&window, Some(D3D11_RENDERER)) {
        Ok(r) => Some((window, r)),
        Err(e) => {
            crate::test_support::skip(
                "d3d11",
                format_args!("no Direct3D 11 renderer ({})", e.message()),
            );
            window.destroy();
            None
        }
    }
}

/// A software renderer drawing into an ARGB8888 surface (the format the
/// Direct3D 11 renderer reads its B8G8R8A8 swap chain back in).
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
/// converted to the Direct3D image's format first).
fn assert_close(what: &str, d3d: &Surface<'_>, sw: &Surface<'_>, tol: u8) {
    let converted;
    let sw = if sw.format() != d3d.format() {
        converted = sw.convert(d3d.format()).unwrap();
        &converted
    } else {
        sw
    };
    assert_eq!(
        (d3d.width(), d3d.height()),
        (sw.width(), sw.height()),
        "{what}: sizes"
    );
    let bad = differences(d3d, sw, tol);
    if let Some(((x, y), pa, pb)) = bad.first() {
        panic!(
            "{what}: {} pixels differ by more than {tol}; first at ({x}, {y}): d3d11 {pa:?} sw {pb:?}",
            bad.len()
        );
    }
}

/// Draw `scene` with the Direct3D 11 renderer and the software renderer,
/// and compare the results (`tol` per channel).
fn compare(what: &str, r: &mut Renderer, tol: u8, scene: impl Fn(&mut Renderer)) {
    scene(r);
    let d3d = r.read_pixels(None).unwrap();
    let mut sw = software_renderer(W, H);
    scene(&mut sw);
    let sw = sw.read_pixels(None).unwrap();
    assert_close(what, &d3d, &sw, tol);
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
fn d3d11_renderer_basics() {
    let _l = crate::test_support::test_lock();
    let Some(_video) = Video::init() else {
        return;
    };
    let Some((window, mut r)) = d3d11_renderer(W, H) else {
        return;
    };
    assert_eq!(r.name(), "direct3d11");
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
        PROP_RENDERER_D3D11_DEVICE_POINTER,
        PROP_RENDERER_D3D11_SWAPCHAIN_POINTER,
    ] {
        assert!(props.get_number(prop).unwrap_or(0) != 0, "{prop}");
    }
    let formats = r.texture_formats();
    assert_eq!(
        &formats[..2],
        &[PixelFormat::ARGB8888, PixelFormat::ABGR8888]
    );
    for f in [
        PixelFormat::XRGB8888,
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

    // The textures are published as properties.
    let t = r
        .create_texture(PixelFormat::ARGB8888, TextureAccess::Static, 8, 8)
        .unwrap();
    let tp = r.texture_properties(t).unwrap();
    assert!(tp.get_number(PROP_TEXTURE_D3D11_TEXTURE_POINTER).unwrap() != 0);
    assert!(tp
        .get_number(PROP_TEXTURE_D3D11_TEXTURE_U_POINTER)
        .is_none());
    let yuv = r
        .create_texture(PixelFormat::IYUV, TextureAccess::Static, 8, 8)
        .unwrap();
    let yp = r.texture_properties(yuv).unwrap();
    assert!(yp.get_number(PROP_TEXTURE_D3D11_TEXTURE_U_POINTER).unwrap() != 0);
    assert!(yp.get_number(PROP_TEXTURE_D3D11_TEXTURE_V_POINTER).unwrap() != 0);
    // (an unsupported YUV colorspace is refused)
    assert!(r
        .create_texture_with(&TextureCreateInfo {
            format: PixelFormat::IYUV,
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

    // Presenting several frames
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
    assert!(r.set_vsync(-1).is_err());

    // A resized window gets resized buffers
    window.set_size(40, 30).unwrap();
    for _ in 0..10 {
        pump();
    }
    let _ = get_events(EventType::FIRST, EventType::LAST, 100000);
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
fn d3d11_matches_software_for_shapes() {
    let _l = crate::test_support::test_lock();
    let Some(_video) = Video::init() else {
        return;
    };
    let Some((_window, mut r)) = d3d11_renderer(W, H) else {
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

    // Many flushes in one frame: around the eight vertex buffers, growing
    // them on the way.
    compare("many flushes", &mut r, 0, |r| {
        r.set_draw_color(0, 0, 0, 255);
        r.clear().unwrap();
        for i in 0..40 {
            r.set_draw_color((i * 6 % 256) as u8, 100, 200, 255);
            let points: Vec<FPoint> = (0..=i)
                .map(|k| FPoint {
                    x: ((i + k) % W) as f32,
                    y: (i / 2 + k % 3) as f32,
                })
                .collect();
            r.render_points(&points).unwrap();
            r.flush().unwrap();
        }
    });
}

#[test]
fn d3d11_matches_software_for_textures() {
    let _l = crate::test_support::test_lock();
    let Some(_video) = Video::init() else {
        return;
    };
    let Some((_window, mut r)) = d3d11_renderer(W, H) else {
        return;
    };
    let mut sw = software_renderer(W, H);

    for format in [
        PixelFormat::ARGB8888,
        PixelFormat::ABGR8888,
        PixelFormat::XRGB8888,
        PixelFormat::RGB565,
        PixelFormat::ARGB4444,
        PixelFormat::RGBA4444, // (through a native texture)
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

    // A palettized texture, nearest (sampled in the shader)
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
    // Another palette is used
    let mut palette2 = Palette::new(256).unwrap();
    palette2
        .set_colors(0, &[Color::new(200, 10, 20, 255); 256])
        .unwrap();
    {
        let shared = share_palette(palette2);
        r.set_texture_palette(tv, Some(shared.clone())).unwrap();
        sw.set_texture_palette(ts, Some(shared)).unwrap();
    }
    // (FIXME (upstream) in D3D11_SetDrawState(): the texture is bound
    // already, so without the flush, which invalidates the cached state,
    // the previous palette would stay bound)
    r.flush().unwrap();
    for rr in [&mut r, &mut sw] {
        let t = if rr.name() == "direct3d11" { tv } else { ts };
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
    let d3d_streaming = r.read_pixels(Some(&Rect::new(10, 10, 20, 20))).unwrap();
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
        &crop(&d3d_streaming),
        &crop(&sw_streaming),
        0,
    );
    r.destroy_texture(tv);
    sw.destroy_texture(ts);

    // Linear scaling and pixel art go through the advanced shader.
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
fn d3d11_matches_software_for_targets() {
    let _l = crate::test_support::test_lock();
    let Some(_video) = Video::init() else {
        return;
    };
    let Some((_window, mut r)) = d3d11_renderer(W, H) else {
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
            r.render_texture(target, None, Some(&FRect::new(0.0, 0.0, 32.0, 24.0)))
                .unwrap();
            // A part, scaled up 2x: nearest sampling then reads texels at
            // a quarter and three quarters across. (Scaled down 2x, every
            // pixel center falls exactly on a texel edge, and which texel
            // a rasterizer picks there depends on its texture coordinate
            // precision: WARP picks the other one than wined3d and the
            // software renderer do.)
            r.render_texture(
                target,
                Some(&FRect::new(12.0, 8.0, 16.0, 12.0)),
                Some(&FRect::new(32.0, 24.0, 32.0, 24.0)),
            )
            .unwrap();
            let out = r.read_pixels(None).unwrap();
            r.destroy_texture(target);
            (read, out)
        };
        let tv = pattern_texture(&mut r, PixelFormat::ABGR8888, 16, 12);
        let ts = pattern_texture(&mut sw, PixelFormat::ABGR8888, 16, 12);
        let (d3d_read, d3d_out) = scene(&mut r, tv);
        let (sw_read, sw_out) = scene(&mut sw, ts);
        let what = format!("{} target", format.name());
        assert_eq!(d3d_read.format(), format, "{what} read format");
        assert_close(&format!("{what} read"), &d3d_read, &sw_read, 0);
        // (the target is drawn blended, rounding differently)
        assert_close(&what, &d3d_out, &sw_out, 2);
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
    // (a texture that isn't a target is refused)
    let t = r
        .create_texture(PixelFormat::ARGB8888, TextureAccess::Static, 8, 8)
        .unwrap();
    assert!(r.set_render_target(Some(t)).is_err());
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
fn d3d11_matches_software_for_yuv() {
    let _l = crate::test_support::test_lock();
    let Some(_video) = Video::init() else {
        return;
    };
    let Some((_window, mut r)) = d3d11_renderer(W, H) else {
        return;
    };
    let mut sw = software_renderer(W, H);
    let (w, h) = (16usize, 12usize);
    let (y, u, v) = yuv_planes(w, h);
    let uv: Vec<u8> = u.iter().zip(&v).flat_map(|(a, b)| [*a, *b]).collect();
    let vu: Vec<u8> = u.iter().zip(&v).flat_map(|(a, b)| [*b, *a]).collect();

    // (Wine's d3d11 has no NV12 textures it can sample: the renderer
    // offers them, as upstream's does, and fails to create them there; the
    // hardware check draws one)
    let nv12 = match r.create_texture(PixelFormat::NV12, TextureAccess::Static, 16, 12) {
        Ok(t) => {
            r.destroy_texture(t);
            true
        }
        Err(e) => {
            println!(
                "note: no NV12 textures on this device ({}): the NV12 and NV21 checks don't run",
                e.message()
            );
            false
        }
    };
    let has =
        |format: &PixelFormat| nv12 || !matches!(*format, PixelFormat::NV12 | PixelFormat::NV21);

    for format in [
        PixelFormat::IYUV,
        PixelFormat::YV12,
        PixelFormat::NV12,
        PixelFormat::NV21,
    ]
    .iter()
    .filter(|f| has(f))
    .copied()
    {
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
    for (format, data) in [(PixelFormat::IYUV, &all), (PixelFormat::NV12, &nv)]
        .into_iter()
        .filter(|(f, _)| has(f))
    {
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

    // I444: three full planes
    let (y4, u4, v4) = (
        y.clone(),
        (0..w * h)
            .map(|i| (110 + (i % w) * 36 / w) as u8)
            .collect::<Vec<u8>>(),
        (0..w * h)
            .map(|i| (146 - (i / w) * 36 / h) as u8)
            .collect::<Vec<u8>>(),
    );
    let scene = |r: &mut Renderer| {
        let t = r
            .create_texture(PixelFormat::I444, TextureAccess::Static, w as i32, h as i32)
            .unwrap();
        r.set_texture_scale_mode(t, ScaleMode::Nearest).unwrap();
        r.update_yuv_texture(t, None, &y4, w as i32, &u4, w as i32, &v4, w as i32)
            .unwrap();
        r.set_draw_color(0, 0, 0, 255);
        r.clear().unwrap();
        r.render_texture(t, None, Some(&FRect::new(3.0, 3.0, 32.0, 24.0)))
            .unwrap();
        r.destroy_texture(t);
        r.read_pixels(None).unwrap()
    };
    assert_close("I444 texture", &scene(&mut r), &scene(&mut sw), 3);

    // (a buffer without the chroma planes is refused, not read past its end)
    let t = r
        .create_texture(PixelFormat::IYUV, TextureAccess::Static, w as i32, h as i32)
        .unwrap();
    assert!(r.update_texture(t, None, &y, w as i32).is_err());
}

#[test]
fn d3d11_line_methods() {
    let _l = crate::test_support::test_lock();
    let Some(_video) = Video::init() else {
        return;
    };
    // Direct3D lines (SDL_RENDER_LINE_METHOD 2), with the end points drawn
    // as points, hit the same pixels as the software renderer's for
    // straight lines.
    hints::set(hints::RENDER_LINE_METHOD, "2").unwrap();
    let created = d3d11_renderer(W, H);
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
    let d3d = scene(&mut r);
    let sw = scene(&mut software_renderer(W, H));
    assert_close("lines", &d3d, &sw, 0);
}

#[test]
fn flip_model_swap_chain_under_wine() {
    let _l = crate::test_support::test_lock();
    let Some(_video) = Video::init() else {
        return;
    };
    // An ordinary window gets a flip-model swap chain, rotated with
    // IDXGISwapChain1::SetRotation() on Windows 8 and later.
    let Some(window) = window(W, H, WindowFlags::HIDDEN) else {
        return;
    };
    let result = Renderer::for_window(&window, Some(D3D11_RENDERER));
    if is_wine() {
        // Wine's SetRotation() is a stub (E_NOTIMPL): the renderer fails,
        // as upstream's does there, unless Wine has no device at all.
        match result {
            Ok(_) => println!("note: this Wine rotates flip-model swap chains"),
            Err(e) => {
                let message = e.message();
                if !message.starts_with("IDXGISwapChain1::SetRotation") {
                    crate::test_support::skip(
                        "d3d11",
                        format_args!("no Direct3D 11 renderer ({message})"),
                    );
                }
            }
        }
        // and the default renderer is the next one.
        let r = Renderer::for_window(&window, None).unwrap();
        assert_ne!(r.name(), D3D11_RENDERER);
    } else {
        match result {
            Ok(mut r) => {
                r.set_draw_color(1, 2, 3, 255);
                r.clear().unwrap();
                r.present().unwrap();
            }
            Err(e) => crate::test_support::skip(
                "d3d11",
                format_args!("no Direct3D 11 renderer ({})", e.message()),
            ),
        }
    }
    window.destroy();
}

#[test]
fn d3d11_fails_without_a_window_handle() {
    let _l = crate::test_support::test_lock();
    // The dummy driver's windows have no HWND: the renderer can't be
    // created, and the default renderer is the software one.
    init::quit();
    hints::set(hints::VIDEO_DRIVER, "dummy").unwrap();
    init::init(InitFlags::VIDEO).unwrap();
    let window = Window::create("no hwnd", 16, 16, WindowFlags::default()).unwrap();
    assert_eq!(
        Renderer::for_window(&window, Some(D3D11_RENDERER))
            .err()
            .unwrap()
            .message(),
        "Couldn't get window handle"
    );
    let r = Renderer::for_window(&window, None).unwrap();
    assert_eq!(r.name(), SOFTWARE_RENDERER);
    drop(r);
    window.destroy();
    init::quit();
    hints::reset(hints::VIDEO_DRIVER);
}

/// The Direct3D 11 renderer on the machine's GPU: run on Windows with
/// `--ignored --nocapture` (see docs/HARDWARE_TESTING.md).
#[test]
#[ignore]
fn hardware_gpu_d3d11_renderer() {
    let _l = crate::test_support::test_lock();
    let Some(_video) = Video::init() else {
        return;
    };
    let Some(window) = window(64, 48, WindowFlags::default()) else {
        return;
    };

    // The backend itself, for the adapter it runs on.
    let mut backend = match D3d11Renderer::for_window(window, Colorspace::SRGB) {
        Ok(backend) => backend,
        Err(e) => {
            crate::test_support::skip(
                "d3d11",
                format_args!("no Direct3D 11 device ({})", e.message()),
            );
            window.destroy();
            return;
        }
    };
    let device = backend.device().unwrap();
    let dxgi_device = device.query::<IDXGIDevice1Vtbl>(&IID_IDXGIDEVICE1).unwrap();
    let desc = dxgi_device.adapter().unwrap().desc().unwrap();
    let description = crate::core::windows::wide_to_utf8(&desc.description);
    println!(
        "adapter: {description} (vendor {:04x}, device {:04x}, {} MB dedicated video memory)",
        desc.vendor_id,
        desc.device_id,
        desc.dedicated_video_memory >> 20
    );
    println!("feature level: {:#x}", backend.feature_level);
    println!("swap chain flags: {:#x}", backend.swap_chain_flags);
    drop(dxgi_device);
    backend.destroy();
    drop(backend);

    // The 2D renderer, chosen by default.
    let mut r = Renderer::for_window(&window, None).unwrap();
    println!("default renderer: {}", r.name());
    assert_eq!(r.name(), "direct3d11");
    r.set_draw_color(10, 200, 30, 255);
    r.clear().unwrap();
    let s = r.read_pixels(Some(&Rect::new(0, 0, 1, 1))).unwrap();
    let c = s.read_pixel(0, 0).unwrap();
    assert_eq!((c.r, c.g, c.b), (10, 200, 30));
    r.present().unwrap();

    // An NV12 texture (which Wine's d3d11 can't sample), as the software
    // renderer draws it.
    let (w, h) = (16usize, 12usize);
    let (y, u, v) = yuv_planes(w, h);
    let uv: Vec<u8> = u.iter().zip(&v).flat_map(|(a, b)| [*a, *b]).collect();
    let scene = |r: &mut Renderer| {
        let t = r
            .create_texture(PixelFormat::NV12, TextureAccess::Static, w as i32, h as i32)
            .unwrap();
        r.set_texture_scale_mode(t, ScaleMode::Nearest).unwrap();
        r.update_nv_texture(t, None, &y, w as i32, &uv, w as i32)
            .unwrap();
        r.set_draw_color(0, 0, 0, 255);
        r.clear().unwrap();
        r.render_texture(t, None, Some(&FRect::new(3.0, 3.0, 32.0, 24.0)))
            .unwrap();
        r.destroy_texture(t);
        r.read_pixels(Some(&Rect::new(0, 0, W, H))).unwrap()
    };
    let d3d = scene(&mut r);
    let sw = scene(&mut software_renderer(W, H));
    assert_close("NV12 texture", &d3d, &sw, 3);
    println!("NV12 texture: matches the software renderer");
    r.present().unwrap();
    drop(r);
    window.destroy();
}
