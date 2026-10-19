// Tests of the Direct3D 12 renderer.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! The tables are checked against upstream's values, the shader and root
//! signature blobs against the root parameters the renderer binds, and the
//! declarations the renderer added to the GPU API's Direct3D 12 ones (the
//! methods and constants it alone uses) against the vtable slots,
//! `sizeof`/`offsetof` values, constants and GUIDs of a C harness built
//! with SDL's vendored `<d3d12.h>` and mingw-w64's `<dxgi1_6.h>`.
//!
//! The scenes are drawn by the Direct3D 12 renderer and by the software
//! renderer, and the pixels compared, with the tolerances of the Direct3D
//! 11 and GPU renderers' tests: exactly where the math is exact (clears,
//! fills, points, 1:1 copies, palettes, render targets read back), within
//! a small tolerance where the GPU's float blending, modulation or YUV
//! conversion rounds differently. The renderer's shaders are DXIL, so the
//! device tests need a Direct3D 12 device with shader model 6, such as
//! Windows' WARP: without one (Wine's vkd3d has shader model 5.1, and the
//! renderer's pipeline states can't be made there) they report a skip
//! (capability `d3d12`) and pass. Where there is one, the renderer must be
//! made, except under a Wine whose `IDXGISwapChain1::SetRotation()` is a
//! stub (`E_NOTIMPL`, as DXVK's is), where the flip-model swap chain of
//! `D3D12_CreateWindowSizeDependentResources()` fails as upstream's does.

use std::mem::{align_of, offset_of, size_of};

use super::*;
use crate::core::windows::is_wine;
use crate::events::queue::{get_events, pump};
use crate::gpu::ShaderFormat;
use crate::init::{self, InitFlags};
use crate::render::direct3d11::d3d::{
    IDXGIFactory1Vtbl, IDXGIFactory2Vtbl, IDXGIFactory5Vtbl, IID_IDXGIFACTORY6,
};
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

const PTR: usize = size_of::<usize>();

fn slot(offset: usize) -> usize {
    offset / PTR
}

fn guid(g: windows_sys::core::GUID) -> u128 {
    let mut v = (g.data1 as u128) << 96 | (g.data2 as u128) << 80 | (g.data3 as u128) << 64;
    for (i, b) in g.data4.iter().enumerate() {
        v |= (*b as u128) << (56 - 8 * i);
    }
    v
}

fn u32_at(data: &[u8], offset: usize) -> u32 {
    u32::from_le_bytes(data[offset..offset + 4].try_into().unwrap())
}

// ---------------------------------------------------------------------------
// Without a device
// ---------------------------------------------------------------------------

#[test]
fn dxil_shaders() {
    // The sizes of the arrays in upstream's D3D12_*.h headers.
    let sizes = [
        (Shader::Solid, 4072, 4464),
        (Shader::SolidPq, 4524, 4464),
        (Shader::Rgb, 4880, 4560),
        (Shader::Advanced, 10768, 4704),
        (Shader::RgbPq, 10080, 4704),
        (Shader::RgbSimple, 4116, 4560),
    ];
    for (shader, ps_size, vs_size) in sizes {
        let (ps, vs) = (shader.pixel_shader(), shader.vertex_shader());
        assert_eq!((ps.len(), vs.len()), (ps_size, vs_size), "{shader:?}");
        for blob in [ps, vs] {
            // A DXBC container (version 1.0, its own size) with a DXIL
            // part, and signed (a nonzero digest).
            assert_eq!(&blob[..4], b"DXBC");
            assert!(blob[4..20].iter().any(|&b| b != 0));
            assert_eq!(&blob[20..24], &[1, 0, 0, 0]);
            assert_eq!(u32_at(blob, 24) as usize, blob.len());
            let parts = u32_at(blob, 28) as usize;
            assert!((0..parts).any(|i| {
                let offset = u32_at(blob, 32 + 4 * i) as usize;
                &blob[offset..offset + 4] == b"DXIL"
            }));
        }
    }
    assert_eq!(Shader::ALL.len(), Shader::COUNT);
    for (i, shader) in Shader::ALL.iter().enumerate() {
        assert_eq!(*shader as usize, i);
    }
    // (D3D12_shaders[]: the vertex shader and root signature of each)
    use RootSignature as R;
    let roots = [
        (Shader::Solid, R::Color),
        (Shader::SolidPq, R::Color),
        (Shader::Rgb, R::Texture),
        (Shader::Advanced, R::Advanced),
        (Shader::RgbPq, R::Advanced),
        (Shader::RgbSimple, R::Texture),
    ];
    for (shader, root) in roots {
        assert_eq!(shader.root_signature_type(), root, "{shader:?}");
    }
    assert_eq!(
        Shader::Solid.vertex_shader(),
        Shader::SolidPq.vertex_shader()
    );
    assert_eq!(
        Shader::Rgb.vertex_shader(),
        Shader::RgbSimple.vertex_shader()
    );
    assert_eq!(
        Shader::Advanced.vertex_shader(),
        Shader::RgbPq.vertex_shader()
    );
    assert_ne!(Shader::Solid.vertex_shader(), Shader::Rgb.vertex_shader());
    // The first and last bytes of D3D12_PixelShader_Colors.h
    let colors = Shader::Solid.pixel_shader();
    assert_eq!(
        &colors[..12],
        &[0x44, 0x58, 0x42, 0x43, 0xb5, 0x5a, 0x6a, 0x5a, 0xf5, 0x06, 0xd8, 0xa9]
    );
    assert_eq!(&colors[colors.len() - 4..], &[0x00, 0x00, 0x00, 0x00]);
}

/// A root parameter of a serialized root signature (version 1.1): its
/// type and visibility, and its root constants' register and count or its
/// descriptor table's ranges (type, count, register).
#[derive(Debug, PartialEq, Eq)]
enum Param {
    Constants(u32, u32, u32),
    Table(u32, Vec<(u32, u32, u32)>),
}

/// The root parameters of a root signature blob (its `RTS0` part).
fn root_parameters(blob: &[u8]) -> (u32, Vec<Param>) {
    let rts0 = u32_at(blob, 32) as usize + 8;
    let rs = &blob[rts0..];
    assert_eq!(u32_at(rs, 0), 2, "version 1.1");
    let (count, offset) = (u32_at(rs, 4) as usize, u32_at(rs, 8) as usize);
    assert_eq!(u32_at(rs, 12), 0, "no static samplers");
    let flags = u32_at(rs, 20);
    let params = (0..count)
        .map(|i| {
            let p = offset + 12 * i;
            let (ty, visibility, data) = (u32_at(rs, p), u32_at(rs, p + 4), u32_at(rs, p + 8));
            let data = data as usize;
            match ty {
                // D3D12_ROOT_PARAMETER_TYPE_32BIT_CONSTANTS, for all stages
                1 => {
                    assert_eq!(visibility, D3D12_SHADER_VISIBILITY_ALL);
                    assert_eq!(u32_at(rs, data + 4), 0, "register space");
                    Param::Constants(u32_at(rs, data), u32_at(rs, data + 8), visibility)
                }
                // D3D12_ROOT_PARAMETER_TYPE_DESCRIPTOR_TABLE
                0 => {
                    let (ranges, at) = (u32_at(rs, data) as usize, u32_at(rs, data + 4) as usize);
                    let ranges = (0..ranges)
                        .map(|r| {
                            let q = at + 24 * r;
                            (u32_at(rs, q), u32_at(rs, q + 4), u32_at(rs, q + 8))
                        })
                        .collect();
                    Param::Table(visibility, ranges)
                }
                _ => panic!("parameter type {ty}"),
            }
        })
        .collect();
    (flags, params)
}

#[test]
fn root_signatures_match_the_bindings() {
    // The serialized root signatures (D3D12_Shader_Common.hlsli): the
    // vertex and pixel constants D3D12_SetDrawState() sets as root
    // parameters 0 and 1, the shader resources it sets at 2 on, and the
    // samplers' tables (3 for the RGB shaders, 5 and 6 for the advanced
    // ones).
    use Param::{Constants as C, Table as T};
    let srv = |register| T(D3D12_SHADER_VISIBILITY_PIXEL, vec![(0, 1, register)]);
    let sampler = |register| T(D3D12_SHADER_VISIBILITY_PIXEL, vec![(3, 1, register)]);
    // ALLOW_INPUT_ASSEMBLER_INPUT_LAYOUT and DENY_{HULL,DOMAIN,GEOMETRY}_SHADER_ROOT_ACCESS
    let flags = D3D12_ROOT_SIGNATURE_FLAG_ALLOW_INPUT_ASSEMBLER_INPUT_LAYOUT | 0x4 | 0x8 | 0x10;
    let expected = [
        (RootSignature::Color, Vec::new()),
        (RootSignature::Texture, vec![srv(0), sampler(0)]),
        (
            RootSignature::Advanced,
            vec![srv(0), srv(1), srv(2), sampler(0), sampler(1)],
        ),
    ];
    for (root, tables) in expected {
        let blob = root.data();
        assert_eq!(&blob[..4], b"DXBC");
        assert_eq!(u32_at(blob, 24) as usize, blob.len());
        assert_eq!(u32_at(blob, 28), 1, "one part");
        assert_eq!(&blob[36..40], b"RTS0");
        let (got_flags, params) = root_parameters(blob);
        assert_eq!(got_flags, flags, "{root:?}");
        let mut want = vec![C(0, 16, 0), C(1, 28, 0)];
        want.extend(tables);
        assert_eq!(params, want, "{root:?}");
    }
    assert_eq!(RootSignature::ALL.len(), RootSignature::COUNT);
    assert_eq!(
        (
            RootSignature::Color.data().len(),
            RootSignature::Texture.data().len(),
            RootSignature::Advanced.data().len()
        ),
        (116, 204, 336)
    );
    // (the root constants are the constant structures, in 32-bit values)
    assert_eq!(size_of::<VertexShaderConstants>() / 4, 16);
    assert_eq!(size_of::<PixelShaderConstants>() / 4, 28);
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
    assert_eq!(tex(F::I4FL, srgb), DXGI_FORMAT_R16_UNORM);
    assert_eq!(tex(F::RGBA4444, srgb), DXGI_FORMAT_UNKNOWN);
    assert_eq!(tex(F::XBGR8888, srgb), DXGI_FORMAT_UNKNOWN);

    let view = sdl_pixel_format_to_dxgi_main_resource_view_format;
    assert_eq!(view(F::NV12, srgb), DXGI_FORMAT_R8_UNORM);
    assert_eq!(view(F::NV21, srgb), DXGI_FORMAT_R8_UNORM);
    assert_eq!(view(F::P010, srgb), DXGI_FORMAT_R16_UNORM);
    assert_eq!(view(F::ABGR8888, linear), DXGI_FORMAT_R8G8B8A8_UNORM_SRGB);
    assert_eq!(view(F::YV12, srgb), DXGI_FORMAT_R8_UNORM);

    assert_eq!(d3d12_align(1, 256), 256);
    assert_eq!(d3d12_align(256, 256), 256);
    assert_eq!(d3d12_align(257, 256), 512);
    assert_eq!(d3d12_align(0, 256), 0);
}

#[test]
fn blend_modes() {
    let custom = |sc, dc, co, sa, da, ao| BlendMode::compose_custom(sc, dc, co, sa, da, ao);
    let desc = create_blend_state(BlendMode::BLEND);
    assert_eq!(desc.alpha_to_coverage_enable, 0);
    assert_eq!(desc.independent_blend_enable, 0);
    let rt = desc.render_target[0];
    assert_eq!(rt.blend_enable, 1);
    assert_eq!(rt.logic_op_enable, 0);
    assert_eq!(rt.src_blend, D3D12_BLEND_SRC_ALPHA);
    assert_eq!(rt.dest_blend, D3D12_BLEND_INV_SRC_ALPHA);
    assert_eq!(rt.blend_op, D3D12_BLEND_OP_ADD);
    assert_eq!(rt.src_blend_alpha, D3D12_BLEND_ONE);
    assert_eq!(rt.dest_blend_alpha, D3D12_BLEND_INV_SRC_ALPHA);
    assert_eq!(rt.blend_op_alpha, D3D12_BLEND_OP_ADD);
    assert_eq!(rt.render_target_write_mask, D3D12_COLOR_WRITE_ENABLE_ALL);
    // (the other render targets stay zeroed)
    assert_eq!(desc.render_target[1], RenderTargetBlendDesc::default());

    let rt = create_blend_state(BlendMode::MOD).render_target[0];
    assert_eq!(
        (
            rt.src_blend,
            rt.dest_blend,
            rt.src_blend_alpha,
            rt.dest_blend_alpha
        ),
        (
            D3D12_BLEND_ZERO,
            D3D12_BLEND_SRC_COLOR,
            D3D12_BLEND_ZERO,
            D3D12_BLEND_ONE
        )
    );
    let rt = create_blend_state(BlendMode::MUL).render_target[0];
    assert_eq!(
        (rt.src_blend, rt.dest_blend),
        (D3D12_BLEND_DEST_COLOR, D3D12_BLEND_INV_SRC_ALPHA)
    );
    let rt = create_blend_state(custom(
        BF::OneMinusDstColor,
        BF::DstAlpha,
        BO::RevSubtract,
        BF::OneMinusDstAlpha,
        BF::SrcColor,
        BO::Maximum,
    ))
    .render_target[0];
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
            D3D12_BLEND_INV_DEST_COLOR,
            D3D12_BLEND_DEST_ALPHA,
            D3D12_BLEND_OP_REV_SUBTRACT,
            D3D12_BLEND_INV_DEST_ALPHA,
            D3D12_BLEND_SRC_COLOR,
            D3D12_BLEND_OP_MAX
        )
    );
    assert_eq!(get_blend_func(None), 0);
    assert_eq!(get_blend_equation(None), 0);
    assert_eq!(
        get_blend_equation(Some(BlendOperation::Minimum)),
        D3D12_BLEND_OP_MIN
    );
    assert_eq!(
        get_blend_equation(Some(BlendOperation::Subtract)),
        D3D12_BLEND_OP_SUBTRACT
    );
}

#[test]
fn samplers_and_shader_selection() {
    use TextureAddressMode as A;
    let desc = sampler_desc(ScaleMode::Nearest, A::Clamp, A::Wrap).unwrap();
    assert_eq!(desc.filter, D3D12_FILTER_MIN_MAG_MIP_POINT);
    assert_eq!(desc.address_u, D3D12_TEXTURE_ADDRESS_MODE_CLAMP);
    assert_eq!(desc.address_v, D3D12_TEXTURE_ADDRESS_MODE_WRAP);
    assert_eq!(desc.address_w, D3D12_TEXTURE_ADDRESS_MODE_CLAMP);
    assert_eq!(desc.max_anisotropy, 1);
    assert_eq!(desc.mip_lod_bias, 0.0);
    assert_eq!(desc.comparison_func, D3D12_COMPARISON_FUNC_NONE);
    assert_eq!(desc.min_lod, 0.0);
    assert_eq!(desc.max_lod.to_bits(), 0x7f7f_ffff); // (D3D12_FLOAT32_MAX)
    assert_eq!(desc.border_color, [0.0; 4]);
    let desc = sampler_desc(ScaleMode::PixelArt, A::Wrap, A::Clamp).unwrap();
    assert_eq!(desc.filter, D3D12_FILTER_MIN_MAG_MIP_LINEAR);
    let desc = sampler_desc(ScaleMode::Linear, A::Wrap, A::Wrap).unwrap();
    assert_eq!(desc.filter, D3D12_FILTER_MIN_MAG_MIP_LINEAR);
    assert_eq!(
        sampler_desc(ScaleMode::Linear, A::Auto, A::Clamp)
            .unwrap_err()
            .message(),
        "Unknown texture address mode: 0"
    );
    assert_eq!(
        sampler_desc(ScaleMode::Linear, A::Clamp, A::Auto)
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
    assert_eq!(size_of::<VertexShaderConstants>(), 64);
    assert_eq!(size_of::<VertexPositionColor>(), 32);
    assert_eq!(offset_of!(VertexPositionColor, tex), 8);
    assert_eq!(offset_of!(VertexPositionColor, color), 16);
    assert_eq!(offset_of!(PixelShaderConstants, tonemap_method), 32);
    assert_eq!(offset_of!(PixelShaderConstants, ycbcr_matrix), 48);
    let mut b = PixelShaderConstants::default();
    b.ycbcr_matrix[15] = -0.0;
    assert_eq!(b.words()[27], 0x8000_0000);
    b.sc_rgb_output = 1.0;
    assert_eq!(b.words()[0], 1.0f32.to_bits());
    let mut v = VertexShaderConstants::default();
    v.mpv.m[3][0] = -1.0;
    assert_eq!(v.words()[12], (-1.0f32).to_bits());
    assert_eq!(v.words()[0], 0);
}

#[test]
fn rotations() {
    assert!(is_display_rotated_90_degrees(DXGI_MODE_ROTATION_ROTATE90));
    assert!(is_display_rotated_90_degrees(DXGI_MODE_ROTATION_ROTATE270));
    assert!(!is_display_rotated_90_degrees(DXGI_MODE_ROTATION_ROTATE180));
    assert!(!is_display_rotated_90_degrees(DXGI_MODE_ROTATION_IDENTITY));
    assert_eq!(get_current_rotation(), DXGI_MODE_ROTATION_IDENTITY);
}

#[test]
fn renderer_declarations_match_the_headers() {
    // offsetof(<Interface>Vtbl, <Method>) / sizeof(void *) of the harness.
    assert_eq!(
        slot(offset_of!(ID3D12DeviceVtbl, get_copyable_footprints)),
        38
    );
    assert_eq!(
        slot(offset_of!(ID3D12DeviceVtbl, create_command_signature)),
        41
    );
    assert_eq!(
        slot(offset_of!(
            ID3D12GraphicsCommandListVtbl,
            set_graphics_root_32bit_constants
        )),
        36
    );
    assert_eq!(
        slot(offset_of!(
            ID3D12GraphicsCommandListVtbl,
            set_compute_root_constant_buffer_view
        )),
        37
    );
    // (IDXGISwapChain4's slots, the renderer's swap chain)
    type S = IDXGISwapChain3Vtbl;
    assert_eq!(slot(offset_of!(S, present)), 8);
    assert_eq!(slot(offset_of!(S, get_buffer)), 9);
    assert_eq!(slot(offset_of!(S, resize_buffers)), 13);
    assert_eq!(slot(offset_of!(S, set_rotation)), 27);
    assert_eq!(slot(offset_of!(S, set_maximum_frame_latency)), 31);
    assert_eq!(slot(offset_of!(S, get_current_back_buffer_index)), 36);
    assert_eq!(slot(offset_of!(S, check_color_space_support)), 37);
    assert_eq!(slot(offset_of!(S, set_color_space1)), 38);
    assert_eq!(size_of::<S>() / PTR, 40);
    // (IDXGIFactory6's)
    assert_eq!(
        slot(offset_of!(IDXGIFactory1Vtbl, make_window_association)),
        8
    );
    assert_eq!(
        slot(offset_of!(IDXGIFactory2Vtbl, create_swap_chain_for_hwnd)),
        15
    );
    assert_eq!(
        slot(offset_of!(
            IDXGIFactory6Vtbl,
            enum_adapter_by_gpu_preference
        )),
        29
    );
    assert_eq!(
        slot(offset_of!(IDXGIFactory5Vtbl, check_feature_support)),
        28
    );

    // sizeof and _Alignof
    assert_eq!((size_of::<D3d12Range>(), align_of::<D3d12Range>()), (16, 8));
    assert_eq!(
        (
            size_of::<PlacedSubresourceFootprint>(),
            align_of::<PlacedSubresourceFootprint>()
        ),
        (32, 8)
    );

    // The constants.
    assert_eq!(D3D12_FILTER_MIN_MAG_MIP_POINT, 0);
    assert_eq!(D3D12_FILTER_MIN_MAG_MIP_LINEAR, 21);
    assert_eq!(D3D12_COMPARISON_FUNC_NONE, 0);
    assert_eq!(D3D12_COLOR_WRITE_ENABLE_ALL, 15);
    assert_eq!(D3D12_RESOURCE_STATE_PIXEL_SHADER_RESOURCE, 128);
    assert_eq!(D3D12_PRIMITIVE_TOPOLOGY_TYPE_PATCH, 4);
    assert_eq!(D3D12_RESOURCE_FLAG_NONE, 0);
    assert_eq!(D3D12_RESOURCE_BARRIER_ALL_SUBRESOURCES, 4294967295);
    assert_eq!(DXGI_SWAP_CHAIN_FLAG_FRAME_LATENCY_WAITABLE_OBJECT, 64);
    assert_eq!(D3D12_FLOAT32_MAX, f32::from_bits(0x7f7f_ffff)); // (0x1.fffffep+127)

    // The GUIDs.
    assert_eq!(
        guid(IID_ID3D12DEVICE1),
        0x77acce80_638e_4e65_8895_c1f23386863e
    );
    assert_eq!(
        guid(IID_ID3D12GRAPHICSCOMMANDLIST2),
        0x38c3e585_ff17_412c_9150_4fc6f9d72a28
    );
    assert_eq!(
        guid(IID_IDXGISWAPCHAIN4),
        0x3d585d5a_bd4a_489e_b1f4_3dbcb6452ffb
    );
    assert_eq!(
        guid(IID_IDXGIADAPTER4),
        0x3c8d99d1_4fbf_4181_a82c_af66bf7bd24e
    );
    assert_eq!(
        guid(IID_IDXGIFACTORY6),
        0xc1b6694f_ff09_44a9_b03c_77900a0a1d17
    );

    // And upstream's numbers.
    assert_eq!(SDL_D3D12_NUM_BUFFERS, 2);
    assert_eq!(SDL_D3D12_NUM_VERTEX_BUFFERS, 256);
    assert_eq!(SDL_D3D12_MAX_NUM_TEXTURES, 16384);
    assert_eq!(SDL_D3D12_NUM_UPLOAD_BUFFERS, 32);
    assert_eq!(EVENT_MODIFY_STATE | SYNCHRONIZE, 0x0010_0002);
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

thread_local! {
    /// The step of the renderer's setup this thread's test makes fail.
    static FAIL_STEP: std::cell::Cell<Option<&'static str>> = const { std::cell::Cell::new(None) };
}

/// The renderer's fault injection: whether the test makes `step` fail.
pub(super) fn injected_failure(step: &str) -> bool {
    FAIL_STEP.get() == Some(step)
}

/// A window, or `None` (with the skip reported) when the windows video
/// driver can't make one.
fn window(w: i32, h: i32, flags: WindowFlags) -> Option<Window> {
    match Window::create("direct3d12", w, h, flags) {
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

/// Whether there is a Direct3D 12 device with shader model 6 (which takes
/// the renderer's DXIL), or `false` with the skip reported.
fn has_d3d12_dxil() -> bool {
    match crate::gpu::Device::new(
        ShaderFormat::DXBC | ShaderFormat::DXIL,
        false,
        Some("direct3d12"),
    ) {
        Ok(device) if device.shader_formats().contains(ShaderFormat::DXIL) => true,
        Ok(_) => {
            crate::test_support::skip(
                "d3d12",
                "the device has no shader model 6, so no DXIL (vkd3d under Wine has 5.1)",
            );
            false
        }
        Err(e) => {
            crate::test_support::skip(
                "d3d12",
                format_args!("no Direct3D 12 device ({})", e.message()),
            );
            false
        }
    }
}

/// Whether a renderer that couldn't be made failed at the swap chain's
/// `SetRotation()`, a stub under some Wines (see the module
/// documentation): then the skip is reported.
fn rotation_stub(message: &str) -> bool {
    if is_wine() && message.starts_with("IDXGISwapChain4::SetRotation") {
        crate::test_support::skip(
            "d3d12",
            format_args!("this Wine's flip-model swap chains can't be rotated ({message})"),
        );
        return true;
    }
    false
}

/// A window with a Direct3D 12 renderer, or `None` (with the skip
/// reported) when there is no device with shader model 6. Where there is
/// one, the renderer must be made.
fn d3d12_renderer(w: i32, h: i32) -> Option<(Window, Renderer)> {
    if !has_d3d12_dxil() {
        return None;
    }
    let window = window(w, h, WindowFlags::HIDDEN)?;
    match Renderer::for_window(&window, Some(D3D12_RENDERER)) {
        Ok(r) => Some((window, r)),
        Err(e) if rotation_stub(e.message()) => {
            window.destroy();
            None
        }
        Err(e) => panic!(
            "no Direct3D 12 renderer on a device with shader model 6 ({})",
            e.message()
        ),
    }
}

/// A software renderer drawing into an ARGB8888 surface (the format the
/// Direct3D 12 renderer reads its B8G8R8A8 swap chain back in).
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
            "{what}: {} pixels differ by more than {tol}; first at ({x}, {y}): d3d12 {pa:?} sw {pb:?}",
            bad.len()
        );
    }
}

/// Draw `scene` with the Direct3D 12 renderer and the software renderer,
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
fn srv_pool_allocator() {
    let _l = crate::test_support::test_lock();
    let Some(_video) = Video::init() else {
        return;
    };
    let Some(window) = window(8, 8, WindowFlags::HIDDEN) else {
        return;
    };
    // (the pool of D3D12_GetAvailableSRVIndex() and D3D12_FreeSRVIndex(),
    // without a device)
    let mut data = D3d12Renderer::new(window, Colorspace::SRGB);
    assert!(data.get_available_srv_index().is_err());
    data.init_srv_pool();
    assert_eq!(data.get_available_srv_index().unwrap(), 0);
    assert_eq!(data.get_available_srv_index().unwrap(), 1);
    assert_eq!(data.get_available_srv_index().unwrap(), 2);
    data.free_srv_index(1);
    data.free_srv_index(0);
    assert_eq!(data.get_available_srv_index().unwrap(), 0);
    assert_eq!(data.get_available_srv_index().unwrap(), 1);
    assert_eq!(data.get_available_srv_index().unwrap(), 3);
    for i in 4..SDL_D3D12_MAX_NUM_TEXTURES {
        assert_eq!(data.get_available_srv_index().unwrap(), i);
    }
    assert_eq!(
        data.get_available_srv_index().unwrap_err().message(),
        "[d3d12] Cannot allocate more than 16384 textures!"
    );
    data.free_srv_index(7);
    assert_eq!(data.get_available_srv_index().unwrap(), 7);
    // Started again (as after a device loss) with the last slot freed
    // before another, the pool hands each slot out once.
    let last = SDL_D3D12_MAX_NUM_TEXTURES - 1;
    data.free_srv_index(5);
    data.free_srv_index(last);
    data.init_srv_pool();
    for i in 0..SDL_D3D12_MAX_NUM_TEXTURES {
        assert_eq!(data.get_available_srv_index().unwrap(), i);
    }
    assert!(data.get_available_srv_index().is_err());
    // (the renderer's starting state: invalidated, tearing presents)
    assert!(data.cliprect_dirty && data.viewport_dirty);
    assert_eq!(data.present_flags, DXGI_PRESENT_ALLOW_TEARING);
    assert_eq!(data.vertex_buffers.len(), SDL_D3D12_NUM_VERTEX_BUFFERS);
    window.destroy();
}

/// Releasing the device closes the fence event (upstream's is never
/// closed).
#[test]
fn release_all_closes_the_fence_event() {
    use windows_sys::Win32::System::Threading::{CreateEventW, OpenEventW};

    let _l = crate::test_support::test_lock();
    let Some(_video) = Video::init() else {
        return;
    };
    let Some(window) = window(8, 8, WindowFlags::HIDDEN) else {
        return;
    };
    let mut data = D3d12Renderer::new(window, Colorspace::SRGB);
    // (an event of a name of the test's, which is gone once its only
    // handle is closed)
    let name: Vec<u16> = format!("sdl3-rs-d3d12-fence-{}\0", std::process::id())
        .encode_utf16()
        .collect();
    // SAFETY: no attributes; a NUL-terminated name.
    data.fence_event = unsafe { CreateEventW(null(), 0, 0, name.as_ptr()) };
    assert!(!data.fence_event.is_null());
    data.release_all();
    assert!(data.fence_event.is_null());
    // SAFETY: a NUL-terminated name; a handle it returns is closed.
    let event = unsafe { OpenEventW(SYNCHRONIZE, 0, name.as_ptr()) };
    if !event.is_null() {
        // SAFETY: the handle OpenEventW() returned.
        unsafe { CloseHandle(event) };
        panic!("the fence event wasn't closed");
    }
    window.destroy();
}

/// A fence event that can't be made fails the renderer (upstream's is
/// made without its root signatures and pipeline states).
#[test]
fn d3d12_fails_without_its_fence_event() {
    let _l = crate::test_support::test_lock();
    let Some(_video) = Video::init() else {
        return;
    };
    let Some((window, r)) = d3d12_renderer(W, H) else {
        return;
    };
    drop(r);
    FAIL_STEP.set(Some("fence event"));
    let result = D3d12Renderer::for_window(window, Colorspace::SRGB);
    FAIL_STEP.set(None);
    let message = result.err().map(|e| e.message().to_owned());
    assert!(
        message
            .as_deref()
            .is_some_and(|m| m.starts_with("CreateEventEx")),
        "{message:?}"
    );
    window.destroy();
}

/// A read of the pixels that fails (here at an unknown rotation) leaves
/// the back buffer a render target (upstream's is left a copy source), so
/// the renderer draws and reads on.
#[test]
fn d3d12_failed_read_leaves_a_render_target() {
    let _l = crate::test_support::test_lock();
    let Some(_video) = Video::init() else {
        return;
    };
    let Some((window, r)) = d3d12_renderer(W, H) else {
        return;
    };
    drop(r);
    let mut data = match D3d12Renderer::for_window(window, Colorspace::SRGB) {
        Ok(data) => data,
        Err(e) => panic!("no Direct3D 12 renderer ({})", e.message()),
    };
    let rect = Rect::new(0, 0, 4, 4);
    let rotation = data.rotation;
    data.rotation = 99;
    assert_eq!(
        data.render_read_pixels(&rect).unwrap_err().message(),
        "The physical display is in an unknown or unsupported rotation"
    );
    data.rotation = rotation;
    let read = data.render_read_pixels(&rect).unwrap();
    assert_eq!((read.width(), read.height()), (4, 4));
    data.destroy();
    drop(data);
    window.destroy();
}

/// A texture whose creation fails frees only the SRV slots it took
/// (upstream frees slot 0 for each one it didn't).
#[test]
fn failed_texture_frees_only_its_srv_slots() {
    let _l = crate::test_support::test_lock();
    let Some(_video) = Video::init() else {
        return;
    };
    let Some(window) = window(8, 8, WindowFlags::HIDDEN) else {
        return;
    };
    let mut data = D3d12Renderer::new(window, Colorspace::SRGB);
    data.init_srv_pool();
    assert_eq!(data.get_available_srv_index().unwrap(), 0);
    // (failed before any view was made, and after the main one)
    data.free_texture_srvs(&D3d12Texture {
        yuv: true,
        nv12: true,
        ..Default::default()
    });
    assert_eq!(data.get_available_srv_index().unwrap(), 1);
    data.free_texture_srvs(&D3d12Texture {
        main_srv_index: Some(1),
        yuv: true,
        ..Default::default()
    });
    assert_eq!(data.get_available_srv_index().unwrap(), 1);
    assert_eq!(data.get_available_srv_index().unwrap(), 2);
    window.destroy();
}

#[test]
fn d3d12_renderer_basics() {
    let _l = crate::test_support::test_lock();
    let Some(_video) = Video::init() else {
        return;
    };
    let Some((window, mut r)) = d3d12_renderer(W, H) else {
        return;
    };
    assert_eq!(r.name(), "direct3d12");
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
        PROP_RENDERER_D3D12_DEVICE_POINTER,
        PROP_RENDERER_D3D12_SWAPCHAIN_POINTER,
        PROP_RENDERER_D3D12_COMMAND_QUEUE_POINTER,
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
        PixelFormat::INDEX8,
        PixelFormat::IYUV,
        PixelFormat::YV12,
        PixelFormat::I444,
        PixelFormat::NV12,
        PixelFormat::NV21,
        PixelFormat::P010,
        PixelFormat::I0FL,
        PixelFormat::I4FL,
    ] {
        assert!(formats.contains(&f), "{f:?} supported");
    }

    // The textures are published as properties.
    let t = r
        .create_texture(PixelFormat::ARGB8888, TextureAccess::Static, 8, 8)
        .unwrap();
    let tp = r.texture_properties(t).unwrap();
    assert!(tp.get_number(PROP_TEXTURE_D3D12_TEXTURE_POINTER).unwrap() != 0);
    assert!(tp
        .get_number(PROP_TEXTURE_D3D12_TEXTURE_U_POINTER)
        .is_none());
    let yuv = r
        .create_texture(PixelFormat::IYUV, TextureAccess::Static, 8, 8)
        .unwrap();
    let yp = r.texture_properties(yuv).unwrap();
    assert!(yp.get_number(PROP_TEXTURE_D3D12_TEXTURE_U_POINTER).unwrap() != 0);
    assert!(yp.get_number(PROP_TEXTURE_D3D12_TEXTURE_V_POINTER).unwrap() != 0);
    // (an unsupported YUV colorspace is refused, and its SRV slots go back
    // to the pool: the next texture takes the same ones)
    assert!(r
        .create_texture_with(&TextureCreateInfo {
            format: PixelFormat::IYUV,
            width: 8,
            height: 8,
            colorspace: Some(Colorspace::SRGB),
            ..Default::default()
        })
        .is_err());
    // (upstream frees slot 0, t's, for it: the next texture would take it,
    // and t would be drawn with that texture's pixels)
    r.update_texture(t, None, &[[20, 10, 200, 255]; 64].concat(), 32)
        .unwrap();
    let other = r
        .create_texture(PixelFormat::ARGB8888, TextureAccess::Static, 8, 8)
        .unwrap();
    r.update_texture(other, None, &[[200, 30, 10, 255]; 64].concat(), 32)
        .unwrap();
    r.set_draw_color(0, 0, 0, 255);
    r.clear().unwrap();
    r.render_texture(t, None, Some(&FRect::new(0.0, 0.0, 8.0, 8.0)))
        .unwrap();
    let s = r.read_pixels(Some(&Rect::new(0, 0, 8, 8))).unwrap();
    assert_eq!(rows(&s), [[20, 10, 200, 255]; 64].concat());
    r.destroy_texture(other);
    r.destroy_texture(t);
    r.destroy_texture(yuv);

    // Custom blend modes are supported, with pipeline states made for them
    let custom = BlendMode::compose_custom(
        BF::One,
        BF::One,
        BO::RevSubtract,
        BF::Zero,
        BF::One,
        BO::Maximum,
    );
    r.set_draw_blend_mode(custom).unwrap();
    r.render_fill_rect(None).unwrap();
    r.set_draw_blend_mode(BlendMode::NONE).unwrap();
    r.flush().unwrap();

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
    r.present().unwrap();

    // The window can go first.
    window.destroy();
    assert!(r.clear().is_err());
    drop(r);
}

#[test]
fn d3d12_matches_software_for_shapes() {
    let _l = crate::test_support::test_lock();
    let Some(_video) = Video::init() else {
        return;
    };
    let Some((_window, mut r)) = d3d12_renderer(W, H) else {
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
    // tie WARP and Wine round up like the software renderer and GPUs such
    // as NVIDIA's round down, so the channels are allowed to differ by 1).
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

    // Many flushes in one frame: past the 256 vertex buffers (issuing an
    // intermediate batch), growing them on the way.
    compare("many flushes", &mut r, 0, |r| {
        r.set_draw_color(0, 0, 0, 255);
        r.clear().unwrap();
        for i in 0..300 {
            r.set_draw_color((i * 6 % 256) as u8, 100, 200, 255);
            let points: Vec<FPoint> = (0..=i % 50)
                .map(|k| FPoint {
                    x: ((i + k) % W) as f32,
                    y: (i % 40 + k % 3) as f32,
                })
                .collect();
            r.render_points(&points).unwrap();
            r.flush().unwrap();
        }
        // (and a draw bigger than a 64 KiB vertex buffer)
        r.set_draw_color(9, 99, 199, 255);
        let points: Vec<FPoint> = (0..3000)
            .map(|k| FPoint {
                x: (k % W) as f32,
                y: (44 + k / W % 3) as f32,
            })
            .collect();
        r.render_points(&points).unwrap();
    });
}

#[test]
fn d3d12_matches_software_for_textures() {
    let _l = crate::test_support::test_lock();
    let Some(_video) = Video::init() else {
        return;
    };
    let Some((_window, mut r)) = d3d12_renderer(W, H) else {
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

    // Wrapped texture coordinates (the samplers' address modes)
    let tv = pattern_texture(&mut r, PixelFormat::ABGR8888, 16, 12);
    let ts = pattern_texture(&mut sw, PixelFormat::ABGR8888, 16, 12);
    for (rr, t) in [(&mut r, tv), (&mut sw, ts)] {
        rr.set_texture_scale_mode(t, ScaleMode::Nearest).unwrap();
        rr.set_texture_blend_mode(t, BlendMode::NONE).unwrap();
        rr.set_draw_color(0, 0, 0, 255);
        rr.clear().unwrap();
        rr.render_texture_tiled(t, None, 1.0, Some(&FRect::new(2.0, 2.0, 40.0, 30.0)))
            .unwrap();
    }
    assert_close(
        "tiled texture",
        &r.read_pixels(None).unwrap(),
        &sw.read_pixels(None).unwrap(),
        0,
    );
    r.destroy_texture(tv);
    sw.destroy_texture(ts);

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
    // (as in D3D11_SetDrawState(), only the first resource is compared:
    // the texture is bound already, so without the flush, which
    // invalidates the cached state, the previous palette would stay bound)
    r.flush().unwrap();
    for rr in [&mut r, &mut sw] {
        let t = if rr.name() == "direct3d12" { tv } else { ts };
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

    // More than 32 texture updates in one frame: the upload buffers run
    // out and the batch is issued.
    let t = r
        .create_texture(PixelFormat::ABGR8888, TextureAccess::Static, 40, 1)
        .unwrap();
    for i in 0..40 {
        r.update_texture(
            t,
            Some(&Rect::new(i, 0, 1, 1)),
            &[i as u8 * 6, 0, 0, 255],
            4,
        )
        .unwrap();
    }
    r.set_texture_blend_mode(t, BlendMode::NONE).unwrap();
    r.set_texture_scale_mode(t, ScaleMode::Nearest).unwrap();
    r.render_texture(t, None, Some(&FRect::new(0.0, 0.0, 40.0, 1.0)))
        .unwrap();
    let s = r.read_pixels(Some(&Rect::new(0, 0, 40, 1))).unwrap();
    let px = rows(&s);
    for i in 0..40 {
        assert_eq!(px[i * 4..i * 4 + 4], [0, 0, i as u8 * 6, 255], "texel {i}");
    }
    r.destroy_texture(t);
}

/// A palette replaced after a draw is destroyed only once the draw is done:
/// the draw uses its colors, not those of the next palette, which takes
/// its SRV slot (upstream releases it while the draw is still recorded).
#[test]
fn d3d12_palette_outlives_its_draws() {
    let _l = crate::test_support::test_lock();
    let Some(_video) = Video::init() else {
        return;
    };
    let Some((_window, mut r)) = d3d12_renderer(W, H) else {
        return;
    };
    let scene = |r: &mut Renderer| {
        let palette = |color| {
            let mut palette = Palette::new(256).unwrap();
            palette.set_colors(0, &[color; 256]).unwrap();
            share_palette(palette)
        };
        let t = r
            .create_texture(PixelFormat::INDEX8, TextureAccess::Static, 4, 4)
            .unwrap();
        r.set_texture_palette(t, Some(palette(Color::new(200, 10, 20, 255))))
            .unwrap();
        r.update_texture(t, None, &[0; 16], 4).unwrap();
        r.set_texture_scale_mode(t, ScaleMode::Nearest).unwrap();
        r.set_draw_color(0, 0, 0, 255);
        r.clear().unwrap();
        r.render_texture(t, None, Some(&FRect::new(4.0, 4.0, 16.0, 16.0)))
            .unwrap();
        // (the first palette's last reference goes: it's destroyed)
        r.set_texture_palette(t, Some(palette(Color::new(10, 30, 220, 255))))
            .unwrap();
        r.render_texture(t, None, Some(&FRect::new(32.0, 4.0, 16.0, 16.0)))
            .unwrap();
        t
    };
    let tv = scene(&mut r);
    let mut sw = software_renderer(W, H);
    let ts = scene(&mut sw);
    assert_close(
        "replaced palette",
        &r.read_pixels(None).unwrap(),
        &sw.read_pixels(None).unwrap(),
        0,
    );
    r.destroy_texture(tv);
    sw.destroy_texture(ts);
}

#[test]
fn d3d12_matches_software_for_targets() {
    let _l = crate::test_support::test_lock();
    let Some(_video) = Video::init() else {
        return;
    };
    let Some((_window, mut r)) = d3d12_renderer(W, H) else {
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
            // A part, scaled up 2x (not down, where the pixel centers fall
            // on texel edges and rasterizers differ)
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

    // One target after another, and back
    let a = r
        .create_texture(PixelFormat::ARGB8888, TextureAccess::Target, 8, 8)
        .unwrap();
    let b = r
        .create_texture(PixelFormat::ARGB8888, TextureAccess::Target, 8, 8)
        .unwrap();
    r.set_render_target(Some(a)).unwrap();
    r.set_draw_color(200, 0, 0, 255);
    r.clear().unwrap();
    r.set_render_target(Some(b)).unwrap();
    r.set_draw_color(0, 200, 0, 255);
    r.clear().unwrap();
    r.set_texture_blend_mode(a, BlendMode::NONE).unwrap();
    r.render_texture(a, None, Some(&FRect::new(0.0, 0.0, 4.0, 8.0)))
        .unwrap();
    r.set_render_target(None).unwrap();
    r.set_texture_blend_mode(b, BlendMode::NONE).unwrap();
    r.render_texture(b, None, Some(&FRect::new(0.0, 0.0, 8.0, 8.0)))
        .unwrap();
    let px = rows(&r.read_pixels(Some(&Rect::new(0, 0, 8, 1))).unwrap());
    assert_eq!(px[..4], [0, 0, 200, 255]);
    assert_eq!(px[28..32], [0, 200, 0, 255]);
    r.destroy_texture(a);
    r.destroy_texture(b);

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
    r.present().unwrap();
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
fn d3d12_matches_software_for_yuv() {
    let _l = crate::test_support::test_lock();
    let Some(_video) = Video::init() else {
        return;
    };
    let Some((_window, mut r)) = d3d12_renderer(W, H) else {
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
    r.destroy_texture(t);
}

#[test]
fn d3d12_line_methods() {
    let _l = crate::test_support::test_lock();
    let Some(_video) = Video::init() else {
        return;
    };
    // Direct3D lines (SDL_RENDER_LINE_METHOD 2), with the end points drawn
    // as points, hit the same pixels as the software renderer's for
    // straight lines.
    hints::set(hints::RENDER_LINE_METHOD, "2").unwrap();
    let created = d3d12_renderer(W, H);
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
fn d3d12_refuses_transparent_windows_and_falls_back() {
    let _l = crate::test_support::test_lock();
    let Some(_video) = Video::init() else {
        return;
    };
    // D3D12 removed the swap effect needed to support transparent windows
    let Some(transparent) = window(W, H, WindowFlags::HIDDEN | WindowFlags::TRANSPARENT) else {
        return;
    };
    assert_eq!(
        Renderer::for_window(&transparent, Some(D3D12_RENDERER))
            .err()
            .unwrap()
            .message(),
        "The direct3d12 renderer doesn't work with transparent windows"
    );
    transparent.destroy();

    // Without a device that takes the DXIL (Wine's vkd3d), the renderer
    // fails (making its pipeline states) and the next driver is tried.
    let Some(window) = window(W, H, WindowFlags::HIDDEN) else {
        return;
    };
    let dxil = has_d3d12_dxil();
    match Renderer::for_window(&window, Some(D3D12_RENDERER)) {
        Ok(_) => assert!(dxil, "a renderer without shader model 6"),
        Err(e) if !dxil => println!("note: no Direct3D 12 renderer ({})", e.message()),
        Err(e) => assert!(rotation_stub(e.message()), "{}", e.message()),
    }
    let r = Renderer::for_window(&window, None).unwrap();
    if !dxil {
        assert_ne!(r.name(), D3D12_RENDERER);
    }
    drop(r);
    window.destroy();
}

#[test]
fn d3d12_fails_without_a_window_handle() {
    let _l = crate::test_support::test_lock();
    // The dummy driver's windows have no HWND: the renderer can't be
    // created, and the default renderer is the software one.
    init::quit();
    hints::set(hints::VIDEO_DRIVER, "dummy").unwrap();
    init::init(InitFlags::VIDEO).unwrap();
    let window = Window::create("no hwnd", 16, 16, WindowFlags::default()).unwrap();
    assert_eq!(
        Renderer::for_window(&window, Some(D3D12_RENDERER))
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

/// The reference count of a COM object (one `AddRef()` and `Release()`).
fn ref_count<V>(object: &ComPtr<V>) -> u32 {
    use crate::core::windows::com::IUnknownVtbl;
    let unknown = object.as_ptr().cast::<c_void>();
    // SAFETY: a live object; every vtable starts with IUnknown's; the
    // reference taken is given back.
    unsafe {
        let vtbl = &**unknown.cast::<*const IUnknownVtbl>();
        (vtbl.add_ref)(unknown);
        (vtbl.release)(unknown)
    }
}

/// A borrowed reference to the renderer's object in a pointer property
/// (ManuallyDrop keeps the renderer's reference).
fn renderer_object<V>(r: &Renderer, name: &str) -> std::mem::ManuallyDrop<ComPtr<V>> {
    let ptr = r.properties().get_number(name).unwrap() as usize as *mut c_void;
    // SAFETY: the renderer's live object of that property, borrowed while
    // the renderer exists.
    std::mem::ManuallyDrop::new(unsafe { ComPtr::from_raw(ptr.cast()) }.unwrap())
}

/// A texture the test makes on the renderer's device, in
/// `D3D12_RESOURCE_STATE_COPY_DEST`, with `pixels` (tightly packed rows of
/// `bpp` bytes) copied in on the renderer's queue (and waited for).
fn app_resource(
    device: &D3d12Device,
    queue: &D3d12CommandQueue,
    format: DxgiFormat,
    (w, h): (u32, u32),
    flags: u32,
    pixels: Option<(&[u8], usize)>,
) -> D3d12Resource {
    let desc = ResourceDesc {
        width: w as u64,
        height: h,
        mip_levels: 1,
        depth_or_array_size: 1,
        format,
        sample_desc: SampleDesc {
            count: 1,
            quality: 0,
        },
        dimension: D3D12_RESOURCE_DIMENSION_TEXTURE2D,
        flags,
        ..Default::default()
    };
    let texture = device
        .create_committed_resource(
            &heap_properties(D3D12_HEAP_TYPE_DEFAULT),
            D3D12_HEAP_FLAG_NONE,
            &desc,
            D3D12_RESOURCE_STATE_COPY_DEST,
            None,
        )
        .unwrap();
    let Some((pixels, bpp)) = pixels else {
        return texture;
    };

    let (footprint, num_rows, _, upload_size) = device.copyable_footprints(&desc, 0, 0);
    let upload = device
        .create_committed_resource(
            &heap_properties(D3D12_HEAP_TYPE_UPLOAD),
            D3D12_HEAP_FLAG_NONE,
            &buffer_desc(upload_size),
            D3D12_RESOURCE_STATE_GENERIC_READ,
            None,
        )
        .unwrap();
    let memory = upload.map().unwrap();
    let row = w as usize * bpp;
    assert_eq!(pixels.len(), row * h as usize);
    for y in 0..num_rows as usize {
        // SAFETY: the mapped buffer holds NumRows rows of RowPitch (>= row)
        // bytes from the footprint's offset.
        unsafe {
            std::ptr::copy_nonoverlapping(
                pixels[y * row..].as_ptr(),
                memory.add(footprint.offset as usize + y * footprint.footprint.row_pitch as usize),
                row,
            )
        };
    }
    upload.unmap();

    let allocator = device
        .create_command_allocator(D3D12_COMMAND_LIST_TYPE_DIRECT)
        .unwrap();
    let list = device
        .create_command_list(0, D3D12_COMMAND_LIST_TYPE_DIRECT, &allocator)
        .unwrap();
    list.copy_texture_region(
        &subresource_location(&texture, 0),
        0,
        0,
        0,
        &footprint_location(&upload, footprint),
        None,
    );
    assert_eq!(list.close(), 0);
    queue.execute_graphics_command_list(&list);
    let fence = device.create_fence(0, D3D12_FENCE_FLAG_NONE).unwrap();
    assert_eq!(queue.signal(&fence, 1), 0);
    while fence.completed_value() < 1 {
        std::thread::sleep(std::time::Duration::from_millis(1));
    }
    texture
}

/// Textures wrapping resources the test makes on the renderer's device
/// (`SDL_PROP_TEXTURE_CREATE_D3D12_TEXTURE*_POINTER`): drawn from (a BGRA
/// and an IYUV one, filled by the test on the renderer's queue), and drawn
/// into as a target, as the software renderer draws; the renderer holds a
/// reference to them while it uses them.
#[test]
fn d3d12_textures_from_existing_d3d12_resources() {
    use crate::render::NativeObject;

    let _l = crate::test_support::test_lock();
    let Some(_video) = Video::init() else {
        return;
    };
    let Some((_window, mut r)) = d3d12_renderer(W, H) else {
        return;
    };
    let device: std::mem::ManuallyDrop<D3d12Device> =
        renderer_object(&r, PROP_RENDERER_D3D12_DEVICE_POINTER);
    let queue: std::mem::ManuallyDrop<D3d12CommandQueue> =
        renderer_object(&r, PROP_RENDERER_D3D12_COMMAND_QUEUE_POINTER);
    let mut sw = software_renderer(W, H);
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

    // A BGRA texture filled by the test.
    let (w, h) = (16u32, 12u32);
    let bgra: Vec<u8> = pattern(w as i32, h as i32)
        .chunks(4)
        .flat_map(|p| [p[2], p[1], p[0], p[3]])
        .collect();
    let ours = app_resource(
        &device,
        &queue,
        DXGI_FORMAT_B8G8R8A8_UNORM,
        (w, h),
        D3D12_RESOURCE_FLAG_NONE,
        Some((&bgra, 4)),
    );
    let count = ref_count(&ours);
    // SAFETY: the test's live resource of the renderer's device, alive
    // until after the renderer's texture is destroyed.
    let object = unsafe { NativeObject::from_ptr(raw(&ours)) };
    let t = r
        .create_texture_with(&TextureCreateInfo {
            format: PixelFormat::ARGB8888,
            width: w as i32,
            height: h as i32,
            d3d12_texture: object,
            ..TextureCreateInfo::default()
        })
        .unwrap();
    assert_eq!(
        r.texture_properties(t)
            .unwrap()
            .get_number(PROP_TEXTURE_D3D12_TEXTURE_POINTER),
        Some(raw(&ours) as i64)
    );
    assert!(ref_count(&ours) > count, "the renderer holds a reference");
    let st = pattern_texture(&mut sw, PixelFormat::ARGB8888, w as i32, h as i32);
    assert_close(
        "existing D3D12 texture",
        &draw(&mut r, t),
        &draw(&mut sw, st),
        0,
    );
    r.destroy_texture(t);
    sw.destroy_texture(st);
    r.flush().unwrap();
    // (the renderer releases its references once the GPU is done)
    r.present().unwrap();
    r.set_draw_color(0, 0, 0, 255);
    r.clear().unwrap();
    r.read_pixels(None).unwrap();
    assert_eq!(
        ref_count(&ours),
        count,
        "the renderer's references are given back"
    );

    // An object that isn't a resource fails as upstream's QueryInterface().
    // SAFETY: the renderer's live queue, alive while the renderer is.
    let not_a_resource = unsafe { NativeObject::from_ptr(raw(&*queue)) };
    let e = r
        .create_texture_with(&TextureCreateInfo {
            format: PixelFormat::ARGB8888,
            width: 4,
            height: 4,
            d3d12_texture: not_a_resource,
            ..TextureCreateInfo::default()
        })
        .unwrap_err();
    assert!(
        e.message().starts_with("SDL.texture.create.d3d12.texture"),
        "{}",
        e.message()
    );

    // A target of the test's resource: what's drawn into it reads back as
    // the software renderer's target.
    let (tw, th) = (24u32, 18u32);
    let target_resource = app_resource(
        &device,
        &queue,
        DXGI_FORMAT_B8G8R8A8_UNORM,
        (tw, th),
        D3D12_RESOURCE_FLAG_ALLOW_RENDER_TARGET,
        None,
    );
    // SAFETY: as above.
    let object = unsafe { NativeObject::from_ptr(raw(&target_resource)) };
    let target = r
        .create_texture_with(&TextureCreateInfo {
            format: PixelFormat::ARGB8888,
            access: TextureAccess::Target,
            width: tw as i32,
            height: th as i32,
            d3d12_texture: object,
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
        "existing D3D12 target",
        &into_target(&mut r, target),
        &into_target(&mut sw, sw_target),
        0,
    );
    assert_close(
        "existing D3D12 target drawn",
        &draw(&mut r, target),
        &draw(&mut sw, sw_target),
        0,
    );
    r.destroy_texture(target);

    // An IYUV texture of three planes filled by the test.
    let (y, u, v) = yuv_planes(w as usize, h as usize);
    let plane = |data: &[u8], size: (u32, u32)| {
        app_resource(
            &device,
            &queue,
            DXGI_FORMAT_R8_UNORM,
            size,
            D3D12_RESOURCE_FLAG_NONE,
            Some((data, 1)),
        )
    };
    let planes = [
        plane(&y, (w, h)),
        plane(&u, (w / 2, h / 2)),
        plane(&v, (w / 2, h / 2)),
    ];
    // SAFETY: as above.
    let objects = planes
        .each_ref()
        .map(|p| unsafe { NativeObject::from_ptr(raw(p)) });
    let t = r
        .create_texture_with(&TextureCreateInfo {
            format: PixelFormat::IYUV,
            width: w as i32,
            height: h as i32,
            d3d12_texture: objects[0],
            d3d12_texture_u: objects[1],
            d3d12_texture_v: objects[2],
            ..TextureCreateInfo::default()
        })
        .unwrap();
    let props = r.texture_properties(t).unwrap();
    assert_eq!(
        props.get_number(PROP_TEXTURE_D3D12_TEXTURE_U_POINTER),
        Some(raw(&planes[1]) as i64)
    );
    assert_eq!(
        props.get_number(PROP_TEXTURE_D3D12_TEXTURE_V_POINTER),
        Some(raw(&planes[2]) as i64)
    );
    let st = sw
        .create_texture(PixelFormat::IYUV, TextureAccess::Static, w as i32, h as i32)
        .unwrap();
    sw.update_yuv_texture(st, None, &y, w as i32, &u, w as i32 / 2, &v, w as i32 / 2)
        .unwrap();
    // (the shaders convert in floats, the software path in fixed point)
    assert_close(
        "existing D3D12 IYUV planes",
        &draw(&mut r, t),
        &draw(&mut sw, st),
        3,
    );
    r.destroy_texture(t);
    r.present().unwrap();
    // The test's textures before the renderer: destroying it releases
    // the device, and the last reference unloads the runtime's DLL.
    drop((ours, target_resource, planes));
    drop(r);
}
