// Tests of the Direct3D 12 GPU backend.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! The declarations, tables and conversions are checked against the
//! values of a C harness: a mingw program built with SDL's vendored
//! `d3d12.h` and `d3d12sdklayers.h` and mingw's DXGI headers that prints
//! the constants, GUIDs, `sizeof`/`offsetof` of the structures, the vtable
//! slots of the methods, and upstream's conversion tables (which it
//! includes from `SDL_gpu_d3d12.c`).
//!
//! The device tests make a device on the offscreen video driver through
//! the front end and directly, and create and release every kind of
//! resource, pipelines from DXBC (the blit shaders) and, on devices with
//! shader model 6, from DXIL ([`test_dxil`], made by
//! `tools/gen_gpu_test_dxil.py`). Without a Direct3D 12 device they report
//! a skip (capability `d3d12`) and pass. Wine's Direct3D 12 (vkd3d) has
//! shader model 5.1 only, so the DXIL parts skip there; Windows' WARP has
//! shader model 6.

use std::mem::{align_of, offset_of, size_of};
use std::sync::atomic::Ordering;
use std::sync::Arc;

use super::d3d::*;
use super::descriptors::StagingDescriptorPool;
use super::device::{
    driver_version, log_debug_info_msg, message_category_name, message_severity_name,
};
use super::pipelines::{
    convert_blend_state, convert_depth_stencil_state, convert_rasterizer_state,
    convert_vertex_input_state, RootParam, RootSignatureBuilder,
};
use super::resources::{
    align, calc_subresource, calc_subresource_with_plane, default_buffer_resource_state,
    default_texture_resource_state, is_valid_shader_bytecode, D3D12BufferType, D3D12Shader,
};
use super::shaders::*;
use super::tables::*;
use super::test_dxil;
use super::*;
use crate::gpu::{
    BlendFactor, BlendOp, BufferCreateInfo, ColorComponentFlags, ColorTargetBlendState,
    ColorTargetDescription, CompareOp, CullMode, DepthStencilState, Device, FillMode, Filter,
    FrontFace, GraphicsPipelineTargetInfo, MultisampleState, PrimitiveType, RasterizerState,
    SamplerAddressMode, SamplerMipmapMode, ShaderFormat, ShaderStage, StencilOp, StencilOpState,
    TransferBufferCreateInfo, VertexAttribute, VertexBufferDescription, VertexElementFormat,
    VertexInputRate, VertexInputState, PROP_GPU_DEVICE_CREATE_DEBUGMODE_BOOLEAN,
    PROP_GPU_DEVICE_CREATE_SHADERS_DXBC_BOOLEAN, PROP_GPU_DEVICE_CREATE_SHADERS_DXIL_BOOLEAN,
    PROP_GPU_DEVICE_DRIVER_VERSION_STRING, PROP_GPU_DEVICE_NAME_STRING,
    PROP_GPU_TEXTURE_CREATE_D3D12_CLEAR_R_FLOAT, PROP_GPU_TEXTURE_CREATE_NAME_STRING,
};
use crate::hints;
use crate::init::{self, InitFlags};
use crate::render::direct3d11::d3d::*;

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

// ---------------------------------------------------------------------------
// Without a device
// ---------------------------------------------------------------------------

#[test]
fn format_lookups() {
    use TextureFormat as F;
    let f = sdl_to_d3d12_texture_format;
    let d = sdl_to_d3d12_depth_format;
    let t = sdl_to_d3d12_typeless_format;
    assert_eq!(SDL_TO_D3D12_TEXTURE_FORMAT.len(), 105);
    assert_eq!(f(F::INVALID), DXGI_FORMAT_UNKNOWN);
    assert_eq!(f(F::R8G8B8A8_UNORM), DXGI_FORMAT_R8G8B8A8_UNORM);
    assert_eq!(f(F::B4G4R4A4_UNORM), DXGI_FORMAT_B4G4R4A4_UNORM);
    assert_eq!(f(F::BC6H_RGB_FLOAT), DXGI_FORMAT_BC6H_SF16);
    assert_eq!(f(F::ASTC_4x4_UNORM), DXGI_FORMAT_UNKNOWN);
    // Depth formats: the color format of their views, the depth format of
    // their targets, and a typeless format when they are sampled too.
    for (format, color, depth, typeless) in [
        (
            F::D16_UNORM,
            DXGI_FORMAT_R16_UNORM,
            DXGI_FORMAT_D16_UNORM,
            DXGI_FORMAT_R16_TYPELESS,
        ),
        (
            F::D24_UNORM,
            DXGI_FORMAT_R24_UNORM_X8_TYPELESS,
            DXGI_FORMAT_D24_UNORM_S8_UINT,
            DXGI_FORMAT_R24G8_TYPELESS,
        ),
        (
            F::D32_FLOAT,
            DXGI_FORMAT_R32_FLOAT,
            DXGI_FORMAT_D32_FLOAT,
            DXGI_FORMAT_R32_TYPELESS,
        ),
        (
            F::D24_UNORM_S8_UINT,
            DXGI_FORMAT_R24_UNORM_X8_TYPELESS,
            DXGI_FORMAT_D24_UNORM_S8_UINT,
            DXGI_FORMAT_R24G8_TYPELESS,
        ),
        (
            F::D32_FLOAT_S8_UINT,
            DXGI_FORMAT_R32_FLOAT_X8X24_TYPELESS,
            DXGI_FORMAT_D32_FLOAT_S8X24_UINT,
            DXGI_FORMAT_R32G8X24_TYPELESS,
        ),
    ] {
        assert_eq!((f(format), d(format), t(format)), (color, depth, typeless));
    }
    // Only the depth formats have depth and typeless formats.
    for i in 0..105 {
        let format = TextureFormat(i);
        assert_eq!(d(format) != DXGI_FORMAT_UNKNOWN, format.is_depth_format());
        assert_eq!(t(format) != DXGI_FORMAT_UNKNOWN, format.is_depth_format());
    }
    // (out of range: no format)
    for format in [TextureFormat(105), TextureFormat(u32::MAX)] {
        assert_eq!(f(format), DXGI_FORMAT_UNKNOWN);
        assert_eq!(d(format), DXGI_FORMAT_UNKNOWN);
        assert_eq!(t(format), DXGI_FORMAT_UNKNOWN);
    }
}

#[test]
fn enum_mappings() {
    // Each enum's lookup is its table at the enum's C value.
    use BlendFactor as B;
    for factor in [
        B::Zero,
        B::One,
        B::SrcColor,
        B::OneMinusSrcColor,
        B::DstColor,
        B::OneMinusDstColor,
        B::SrcAlpha,
        B::OneMinusSrcAlpha,
        B::DstAlpha,
        B::OneMinusDstAlpha,
        B::ConstantColor,
        B::OneMinusConstantColor,
        B::SrcAlphaSaturate,
    ] {
        assert_eq!(
            blend_factor(factor),
            SDL_TO_D3D12_BLEND_FACTOR[factor as usize]
        );
        assert_eq!(
            blend_factor_alpha(factor),
            SDL_TO_D3D12_BLEND_FACTOR_ALPHA[factor as usize]
        );
    }
    assert_eq!(blend_factor(B::OneMinusSrcColor), D3D12_BLEND_INV_SRC_COLOR);
    // (the alpha table uses the alpha factors for the color ones)
    assert_eq!(
        blend_factor_alpha(B::OneMinusSrcColor),
        D3D12_BLEND_INV_SRC_ALPHA
    );
    assert_eq!(blend_factor_alpha(B::DstColor), D3D12_BLEND_DEST_ALPHA);
    assert_eq!(
        blend_op(BlendOp::ReverseSubtract),
        D3D12_BLEND_OP_REV_SUBTRACT
    );
    assert_eq!(blend_op(BlendOp::Max), D3D12_BLEND_OP_MAX);
    assert_eq!(compare_op(CompareOp::Never), D3D12_COMPARISON_FUNC_NEVER);
    assert_eq!(
        compare_op(CompareOp::GreaterOrEqual),
        D3D12_COMPARISON_FUNC_GREATER_EQUAL
    );
    assert_eq!(compare_op(CompareOp::Always), D3D12_COMPARISON_FUNC_ALWAYS);
    assert_eq!(
        stencil_op(StencilOp::IncrementAndClamp),
        D3D12_STENCIL_OP_INCR_SAT
    );
    assert_eq!(
        stencil_op(StencilOp::DecrementAndWrap),
        D3D12_STENCIL_OP_DECR
    );
    assert_eq!(cull_mode(CullMode::None), D3D12_CULL_MODE_NONE);
    assert_eq!(cull_mode(CullMode::Back), D3D12_CULL_MODE_BACK);
    assert_eq!(fill_mode(FillMode::Fill), D3D12_FILL_MODE_SOLID);
    assert_eq!(fill_mode(FillMode::Line), D3D12_FILL_MODE_WIREFRAME);
    assert_eq!(
        input_rate(VertexInputRate::Instance),
        D3D12_INPUT_CLASSIFICATION_PER_INSTANCE_DATA
    );
    assert_eq!(
        vertex_format(VertexElementFormat::Int),
        DXGI_FORMAT_R32_SINT
    );
    assert_eq!(
        vertex_format(VertexElementFormat::Float3),
        DXGI_FORMAT_R32G32B32_FLOAT
    );
    assert_eq!(
        vertex_format(VertexElementFormat::Ubyte4Norm),
        DXGI_FORMAT_R8G8B8A8_UNORM
    );
    assert_eq!(
        vertex_format(VertexElementFormat::Half4),
        DXGI_FORMAT_R16G16B16A16_FLOAT
    );
    assert_eq!(sample_count(SampleCount::Eight), 8);
    assert_eq!(
        primitive_topology_type(PrimitiveType::LineStrip),
        D3D12_PRIMITIVE_TOPOLOGY_TYPE_LINE
    );
    assert_eq!(
        primitive_topology_type(PrimitiveType::PointList),
        D3D12_PRIMITIVE_TOPOLOGY_TYPE_POINT
    );
    assert_eq!(
        sampler_address_mode(SamplerAddressMode::MirroredRepeat),
        D3D12_TEXTURE_ADDRESS_MODE_MIRROR
    );
    assert_eq!(
        sampler_address_mode(SamplerAddressMode::ClampToEdge),
        D3D12_TEXTURE_ADDRESS_MODE_CLAMP
    );
    // Swapchain compositions (part 2 uses them)
    assert_eq!(
        SWAPCHAIN_COMPOSITION_TO_SDL_TEXTURE_FORMAT[SwapchainComposition::SdrLinear as usize],
        TextureFormat::B8G8R8A8_UNORM_SRGB
    );
    assert_eq!(
        SWAPCHAIN_COMPOSITION_TO_COLOR_SPACE[SwapchainComposition::Hdr10St2084 as usize],
        DXGI_COLOR_SPACE_RGB_FULL_G2084_NONE_P2020
    );
}

#[test]
fn resource_states_and_helpers() {
    use TextureUsageFlags as U;
    // NOTE: order matters here!
    assert_eq!(
        default_texture_resource_state(U::SAMPLER | U::COLOR_TARGET),
        D3D12_RESOURCE_STATE_ALL_SHADER_RESOURCE
    );
    assert_eq!(
        default_texture_resource_state(U::GRAPHICS_STORAGE_READ | U::DEPTH_STENCIL_TARGET),
        D3D12_RESOURCE_STATE_ALL_SHADER_RESOURCE
    );
    assert_eq!(
        default_texture_resource_state(U::COLOR_TARGET | U::COMPUTE_STORAGE_WRITE),
        D3D12_RESOURCE_STATE_RENDER_TARGET
    );
    assert_eq!(
        default_texture_resource_state(U::DEPTH_STENCIL_TARGET),
        D3D12_RESOURCE_STATE_DEPTH_WRITE
    );
    assert_eq!(
        default_texture_resource_state(U::COMPUTE_STORAGE_READ | U::COMPUTE_STORAGE_WRITE),
        D3D12_RESOURCE_STATE_NON_PIXEL_SHADER_RESOURCE
    );
    assert_eq!(
        default_texture_resource_state(U::COMPUTE_STORAGE_WRITE),
        D3D12_RESOURCE_STATE_UNORDERED_ACCESS
    );
    assert_eq!(
        default_texture_resource_state(U::COMPUTE_STORAGE_SIMULTANEOUS_READ_WRITE),
        D3D12_RESOURCE_STATE_UNORDERED_ACCESS
    );
    // ("Texture has no default usage mode!")
    assert_eq!(
        default_texture_resource_state(U::default()),
        D3D12_RESOURCE_STATE_ALL_SHADER_RESOURCE
    );

    use BufferUsageFlags as B;
    assert_eq!(
        default_buffer_resource_state(B::VERTEX | B::INDEX | B::INDIRECT),
        D3D12_RESOURCE_STATE_VERTEX_AND_CONSTANT_BUFFER
            | D3D12_RESOURCE_STATE_INDEX_BUFFER
            | D3D12_RESOURCE_STATE_INDIRECT_ARGUMENT
    );
    assert_eq!(
        default_buffer_resource_state(B::GRAPHICS_STORAGE_READ | B::COMPUTE_STORAGE_WRITE),
        D3D12_RESOURCE_STATE_ALL_SHADER_RESOURCE
    );
    assert_eq!(
        default_buffer_resource_state(B::COMPUTE_STORAGE_READ),
        D3D12_RESOURCE_STATE_NON_PIXEL_SHADER_RESOURCE
    );
    // If no read flags are set, read-write can be the default.
    assert_eq!(
        default_buffer_resource_state(B::COMPUTE_STORAGE_WRITE),
        D3D12_RESOURCE_STATE_UNORDERED_ACCESS
    );
    // ("Buffer has no default usage mode!")
    assert_eq!(
        default_buffer_resource_state(B::default()),
        D3D12_RESOURCE_STATE_VERTEX_AND_CONSTANT_BUFFER
    );

    assert_eq!(align(0, 256), 0);
    assert_eq!(align(1, 256), 256);
    assert_eq!(align(256, 256), 256);
    assert_eq!(align(257, 256), 512);
    assert_eq!(calc_subresource(2, 3, 4), 14);
    // (the stencil plane of D24S8 at level 1 of layer 2, 3 levels, 6 layers)
    assert_eq!(calc_subresource_with_plane(1, 2, 1, 3, 6), 1 + 6 + 18);

    assert!(is_valid_shader_bytecode(b"DXBC...."));
    assert!(is_valid_shader_bytecode(b"DXBC"));
    assert!(!is_valid_shader_bytecode(b"DXB"));
    assert!(!is_valid_shader_bytecode(b"\x03\x02\x23\x07"));
    assert!(!is_valid_shader_bytecode(&[]));

    // HIWORD(HighPart).LOWORD(HighPart).HIWORD(LowPart).LOWORD(LowPart)
    assert_eq!(driver_version(0x001f_0020_0003_0004), "31.32.3.4");
    assert_eq!(driver_version(-1), "65535.65535.65535.65535");
    assert_eq!(driver_version(0), "0.0.0.0");

    assert_eq!(
        message_category_name(D3D12_MESSAGE_CATEGORY_STATE_CREATION),
        "STATE_CREATION"
    );
    assert_eq!(
        message_category_name(D3D12_MESSAGE_CATEGORY_SHADER),
        "SHADER"
    );
    assert_eq!(message_category_name(99), "UNKNOWN");
    assert_eq!(
        message_severity_name(D3D12_MESSAGE_SEVERITY_CORRUPTION),
        "CORRUPTION"
    );
    assert_eq!(
        message_severity_name(D3D12_MESSAGE_SEVERITY_MESSAGE),
        "MESSAGE"
    );
    assert_eq!(message_severity_name(7), "UNKNOWN");
    log_debug_info_msg(
        D3D12_MESSAGE_CATEGORY_EXECUTION,
        D3D12_MESSAGE_SEVERITY_WARNING,
        5,
        "test",
    );
}

#[test]
fn state_conversions() {
    let rasterizer = convert_rasterizer_state(&RasterizerState {
        fill_mode: FillMode::Line,
        cull_mode: CullMode::Front,
        front_face: FrontFace::CounterClockwise,
        depth_bias_constant_factor: 2.5,
        depth_bias_clamp: 0.25,
        depth_bias_slope_factor: 1.5,
        enable_depth_bias: true,
        enable_depth_clip: true,
    });
    assert_eq!(
        rasterizer,
        RasterizerDesc {
            fill_mode: D3D12_FILL_MODE_WIREFRAME,
            cull_mode: D3D12_CULL_MODE_FRONT,
            front_counter_clockwise: 1,
            depth_bias: 3, // (SDL_lroundf(2.5f): half away from zero)
            depth_bias_clamp: 0.25,
            slope_scaled_depth_bias: 1.5,
            depth_clip_enable: 1,
            multisample_enable: 0,
            antialiased_line_enable: 0,
            forced_sample_count: 0,
            conservative_raster: D3D12_CONSERVATIVE_RASTERIZATION_MODE_OFF,
        }
    );
    let rasterizer = convert_rasterizer_state(&RasterizerState {
        front_face: FrontFace::Clockwise,
        depth_bias_constant_factor: -2.5,
        depth_bias_clamp: 0.25,
        ..Default::default()
    });
    assert_eq!(rasterizer.front_counter_clockwise, 0);
    // (no bias unless enabled)
    assert_eq!(
        (rasterizer.depth_bias, rasterizer.depth_bias_clamp),
        (0, 0.0)
    );
    assert_eq!(
        convert_rasterizer_state(&RasterizerState {
            depth_bias_constant_factor: -2.5,
            enable_depth_bias: true,
            ..Default::default()
        })
        .depth_bias,
        -3
    );

    let depth_stencil = convert_depth_stencil_state(&DepthStencilState {
        compare_op: CompareOp::LessOrEqual,
        back_stencil_state: StencilOpState {
            fail_op: StencilOp::Zero,
            pass_op: StencilOp::Replace,
            depth_fail_op: StencilOp::Invert,
            compare_op: CompareOp::Equal,
        },
        front_stencil_state: StencilOpState {
            fail_op: StencilOp::Keep,
            pass_op: StencilOp::IncrementAndWrap,
            depth_fail_op: StencilOp::DecrementAndClamp,
            compare_op: CompareOp::NotEqual,
        },
        compare_mask: 0x0f,
        write_mask: 0xf0,
        enable_depth_test: true,
        enable_depth_write: true,
        enable_stencil_test: false,
    });
    assert_eq!(
        depth_stencil,
        DepthStencilDesc {
            depth_enable: 1,
            depth_write_mask: D3D12_DEPTH_WRITE_MASK_ALL,
            depth_func: D3D12_COMPARISON_FUNC_LESS_EQUAL,
            stencil_enable: 0,
            stencil_read_mask: 0x0f,
            stencil_write_mask: 0xf0,
            front_face: DepthStencilOpDesc {
                stencil_fail_op: D3D12_STENCIL_OP_KEEP,
                stencil_depth_fail_op: D3D12_STENCIL_OP_DECR_SAT,
                stencil_pass_op: D3D12_STENCIL_OP_INCR,
                stencil_func: D3D12_COMPARISON_FUNC_NOT_EQUAL,
            },
            back_face: DepthStencilOpDesc {
                stencil_fail_op: D3D12_STENCIL_OP_ZERO,
                stencil_depth_fail_op: D3D12_STENCIL_OP_INVERT,
                stencil_pass_op: D3D12_STENCIL_OP_REPLACE,
                stencil_func: D3D12_COMPARISON_FUNC_EQUAL,
            },
        }
    );

    // A graphics pipeline description for the blend state (its shaders
    // aren't read).
    let shader = Shader::from_backend(BackendObject::new(()));
    let targets = [
        ColorTargetDescription {
            format: TextureFormat::R8G8B8A8_UNORM,
            blend_state: ColorTargetBlendState {
                src_color_blendfactor: BlendFactor::SrcAlpha,
                dst_color_blendfactor: BlendFactor::OneMinusSrcAlpha,
                color_blend_op: BlendOp::Add,
                src_alpha_blendfactor: BlendFactor::SrcColor,
                dst_alpha_blendfactor: BlendFactor::OneMinusDstColor,
                alpha_blend_op: BlendOp::Subtract,
                color_write_mask: ColorComponentFlags(0x5),
                enable_blend: true,
                enable_color_write_mask: true,
            },
        },
        ColorTargetDescription {
            format: TextureFormat::R16G16B16A16_FLOAT,
            blend_state: ColorTargetBlendState {
                color_write_mask: ColorComponentFlags(0x5),
                ..Default::default()
            },
        },
    ];
    let createinfo = GraphicsPipelineCreateInfo {
        vertex_shader: &shader,
        fragment_shader: &shader,
        vertex_input_state: Default::default(),
        primitive_type: PrimitiveType::TriangleList,
        rasterizer_state: Default::default(),
        multisample_state: MultisampleState {
            enable_alpha_to_coverage: true,
            ..Default::default()
        },
        depth_stencil_state: Default::default(),
        target_info: GraphicsPipelineTargetInfo {
            color_target_descriptions: &targets,
            depth_stencil_format: TextureFormat::D16_UNORM,
            has_depth_stencil_target: false,
        },
        props: None,
    };
    let blend = convert_blend_state(&createinfo);
    assert_eq!(blend.alpha_to_coverage_enable, 1);
    assert_eq!(blend.independent_blend_enable, 1);
    assert_eq!(
        blend.render_target[0],
        RenderTargetBlendDesc {
            blend_enable: 1,
            logic_op_enable: 0,
            src_blend: D3D12_BLEND_SRC_ALPHA,
            dest_blend: D3D12_BLEND_INV_SRC_ALPHA,
            blend_op: D3D12_BLEND_OP_ADD,
            src_blend_alpha: D3D12_BLEND_SRC_ALPHA,
            dest_blend_alpha: D3D12_BLEND_INV_DEST_ALPHA,
            blend_op_alpha: D3D12_BLEND_OP_SUBTRACT,
            logic_op: D3D12_LOGIC_OP_NOOP,
            render_target_write_mask: 0x5,
        }
    );
    // (the write mask is all channels unless enabled; INVALID factors are
    // ZERO, and an INVALID op ADD)
    assert_eq!(blend.render_target[1].render_target_write_mask, 0xf);
    assert_eq!(blend.render_target[1].src_blend, D3D12_BLEND_ZERO);
    assert_eq!(blend.render_target[2], RenderTargetBlendDesc::default());

    let descriptions = [
        VertexBufferDescription {
            slot: 0,
            pitch: 24,
            input_rate: VertexInputRate::Vertex,
            instance_step_rate: 0,
        },
        VertexBufferDescription {
            slot: 1,
            pitch: 16,
            input_rate: VertexInputRate::Instance,
            instance_step_rate: 0,
        },
    ];
    let attributes = [
        VertexAttribute {
            location: 0,
            buffer_slot: 0,
            format: VertexElementFormat::Float2,
            offset: 0,
        },
        VertexAttribute {
            location: 3,
            buffer_slot: 1,
            format: VertexElementFormat::Ubyte4Norm,
            offset: 8,
        },
        // (a buffer slot without a description: per-vertex)
        VertexAttribute {
            location: 4,
            buffer_slot: 5,
            format: VertexElementFormat::Short2,
            offset: 4,
        },
    ];
    let semantic = c"TEXCOORD";
    let elements = convert_vertex_input_state(
        &VertexInputState {
            vertex_buffer_descriptions: &descriptions,
            vertex_attributes: &attributes,
        },
        semantic.as_ptr(),
    );
    let fields: Vec<_> = elements
        .iter()
        .map(|e| {
            assert_eq!(e.semantic_name, semantic.as_ptr());
            (
                e.semantic_index,
                e.format,
                e.input_slot,
                e.aligned_byte_offset,
                e.input_slot_class,
                e.instance_data_step_rate,
            )
        })
        .collect();
    assert_eq!(
        fields,
        [
            (0, DXGI_FORMAT_R32G32_FLOAT, 0, 0, 0, 0),
            (3, DXGI_FORMAT_R8G8B8A8_UNORM, 1, 8, 1, 1),
            (4, DXGI_FORMAT_R16G16_SINT, 5, 4, 0, 0),
        ]
    );
    assert!(convert_vertex_input_state(&VertexInputState::default(), semantic.as_ptr()).is_empty());
}

#[test]
fn root_signature_parameters() {
    let mut builder = RootSignatureBuilder::default();
    assert_eq!(
        builder.table(
            D3D12_DESCRIPTOR_RANGE_TYPE_SAMPLER,
            2,
            0,
            2,
            D3D12_SHADER_VISIBILITY_PIXEL
        ),
        0
    );
    assert_eq!(builder.cbv(1, 3, D3D12_SHADER_VISIBILITY_PIXEL), 1);
    assert_eq!(
        builder.params,
        [
            RootParam::Table(
                DescriptorRange {
                    range_type: D3D12_DESCRIPTOR_RANGE_TYPE_SAMPLER,
                    num_descriptors: 2,
                    base_shader_register: 0,
                    register_space: 2,
                    offset_in_descriptors_from_table_start: D3D12_DESCRIPTOR_RANGE_OFFSET_APPEND,
                },
                D3D12_SHADER_VISIBILITY_PIXEL
            ),
            RootParam::Cbv(1, 3, D3D12_SHADER_VISIBILITY_PIXEL),
        ]
    );
}

#[test]
fn shader_blobs() {
    // The blit shaders are DXBC of shader model 5.1, the test shaders
    // signed DXIL (a DXBC container too).
    for (blob, len) in [
        (&D3D12_FULLSCREEN_VERT[..], 884),
        (&D3D12_BLIT_FROM_2D[..], 1404),
        (&D3D12_BLIT_FROM_2D_ARRAY[..], 1456),
        (&D3D12_BLIT_FROM_3D[..], 1432),
        (&D3D12_BLIT_FROM_CUBE[..], 1936),
        (&D3D12_BLIT_FROM_CUBE_ARRAY[..], 1956),
        (&test_dxil::VERTEX[..], test_dxil::VERTEX.len()),
        (
            &test_dxil::FRAGMENT_SAMPLER[..],
            test_dxil::FRAGMENT_SAMPLER.len(),
        ),
        (
            &test_dxil::FRAGMENT_SOLID[..],
            test_dxil::FRAGMENT_SOLID.len(),
        ),
        (&test_dxil::COMPUTE[..], test_dxil::COMPUTE.len()),
    ] {
        assert_eq!(blob.len(), len);
        assert!(is_valid_shader_bytecode(blob));
        assert_eq!(
            u32::from_le_bytes(blob[24..28].try_into().unwrap()) as usize,
            len
        );
        // (a hash: signed)
        assert!(blob[4..20].iter().any(|&b| b != 0));
    }
    // DXIL containers have a DXIL part.
    assert!(test_dxil::COMPUTE.windows(4).any(|w| w == b"DXIL"));
    assert!(!D3D12_BLIT_FROM_2D.windows(4).any(|w| w == b"DXIL"));
}

#[test]
fn constants_match_the_headers() {
    // The values of SDL's vendored d3d12.h and mingw's DXGI headers (harness).
    assert_eq!(D3D12_DESCRIPTOR_HEAP_TYPE_CBV_SRV_UAV, 0);
    assert_eq!(D3D12_DESCRIPTOR_HEAP_TYPE_SAMPLER, 1);
    assert_eq!(D3D12_DESCRIPTOR_HEAP_TYPE_RTV, 2);
    assert_eq!(D3D12_DESCRIPTOR_HEAP_TYPE_DSV, 3);
    assert_eq!(D3D12_DESCRIPTOR_HEAP_TYPE_NUM_TYPES, 4);
    assert_eq!(D3D12_DESCRIPTOR_HEAP_FLAG_NONE, 0);
    assert_eq!(D3D12_DESCRIPTOR_HEAP_FLAG_SHADER_VISIBLE, 1);
    assert_eq!(D3D12_BLEND_ZERO, 1);
    assert_eq!(D3D12_BLEND_ONE, 2);
    assert_eq!(D3D12_BLEND_SRC_COLOR, 3);
    assert_eq!(D3D12_BLEND_INV_SRC_COLOR, 4);
    assert_eq!(D3D12_BLEND_SRC_ALPHA, 5);
    assert_eq!(D3D12_BLEND_INV_SRC_ALPHA, 6);
    assert_eq!(D3D12_BLEND_DEST_ALPHA, 7);
    assert_eq!(D3D12_BLEND_INV_DEST_ALPHA, 8);
    assert_eq!(D3D12_BLEND_DEST_COLOR, 9);
    assert_eq!(D3D12_BLEND_INV_DEST_COLOR, 10);
    assert_eq!(D3D12_BLEND_SRC_ALPHA_SAT, 11);
    assert_eq!(D3D12_BLEND_BLEND_FACTOR, 14);
    assert_eq!(D3D12_BLEND_INV_BLEND_FACTOR, 15);
    assert_eq!(D3D12_BLEND_OP_ADD, 1);
    assert_eq!(D3D12_BLEND_OP_SUBTRACT, 2);
    assert_eq!(D3D12_BLEND_OP_REV_SUBTRACT, 3);
    assert_eq!(D3D12_BLEND_OP_MIN, 4);
    assert_eq!(D3D12_BLEND_OP_MAX, 5);
    assert_eq!(D3D12_LOGIC_OP_NOOP, 4);
    assert_eq!(D3D12_COMPARISON_FUNC_NEVER, 1);
    assert_eq!(D3D12_COMPARISON_FUNC_LESS, 2);
    assert_eq!(D3D12_COMPARISON_FUNC_EQUAL, 3);
    assert_eq!(D3D12_COMPARISON_FUNC_LESS_EQUAL, 4);
    assert_eq!(D3D12_COMPARISON_FUNC_GREATER, 5);
    assert_eq!(D3D12_COMPARISON_FUNC_NOT_EQUAL, 6);
    assert_eq!(D3D12_COMPARISON_FUNC_GREATER_EQUAL, 7);
    assert_eq!(D3D12_COMPARISON_FUNC_ALWAYS, 8);
    assert_eq!(D3D12_STENCIL_OP_KEEP, 1);
    assert_eq!(D3D12_STENCIL_OP_ZERO, 2);
    assert_eq!(D3D12_STENCIL_OP_REPLACE, 3);
    assert_eq!(D3D12_STENCIL_OP_INCR_SAT, 4);
    assert_eq!(D3D12_STENCIL_OP_DECR_SAT, 5);
    assert_eq!(D3D12_STENCIL_OP_INVERT, 6);
    assert_eq!(D3D12_STENCIL_OP_INCR, 7);
    assert_eq!(D3D12_STENCIL_OP_DECR, 8);
    assert_eq!(D3D12_CULL_MODE_NONE, 1);
    assert_eq!(D3D12_CULL_MODE_FRONT, 2);
    assert_eq!(D3D12_CULL_MODE_BACK, 3);
    assert_eq!(D3D12_FILL_MODE_WIREFRAME, 2);
    assert_eq!(D3D12_FILL_MODE_SOLID, 3);
    assert_eq!(D3D12_DEPTH_WRITE_MASK_ZERO, 0);
    assert_eq!(D3D12_DEPTH_WRITE_MASK_ALL, 1);
    assert_eq!(D3D12_CONSERVATIVE_RASTERIZATION_MODE_OFF, 0);
    assert_eq!(D3D12_INPUT_CLASSIFICATION_PER_VERTEX_DATA, 0);
    assert_eq!(D3D12_INPUT_CLASSIFICATION_PER_INSTANCE_DATA, 1);
    assert_eq!(D3D_PRIMITIVE_TOPOLOGY_POINTLIST, 1);
    assert_eq!(D3D_PRIMITIVE_TOPOLOGY_LINELIST, 2);
    assert_eq!(D3D_PRIMITIVE_TOPOLOGY_LINESTRIP, 3);
    assert_eq!(D3D_PRIMITIVE_TOPOLOGY_TRIANGLELIST, 4);
    assert_eq!(D3D_PRIMITIVE_TOPOLOGY_TRIANGLESTRIP, 5);
    assert_eq!(D3D12_PRIMITIVE_TOPOLOGY_TYPE_POINT, 1);
    assert_eq!(D3D12_PRIMITIVE_TOPOLOGY_TYPE_LINE, 2);
    assert_eq!(D3D12_PRIMITIVE_TOPOLOGY_TYPE_TRIANGLE, 3);
    assert_eq!(D3D12_TEXTURE_ADDRESS_MODE_WRAP, 1);
    assert_eq!(D3D12_TEXTURE_ADDRESS_MODE_MIRROR, 2);
    assert_eq!(D3D12_TEXTURE_ADDRESS_MODE_CLAMP, 3);
    assert_eq!(D3D12_FILTER_REDUCTION_TYPE_MASK, 3);
    assert_eq!(D3D12_FILTER_REDUCTION_TYPE_SHIFT, 7);
    assert_eq!(D3D12_FILTER_TYPE_MASK, 3);
    assert_eq!(D3D12_MIN_FILTER_SHIFT, 4);
    assert_eq!(D3D12_MAG_FILTER_SHIFT, 2);
    assert_eq!(D3D12_MIP_FILTER_SHIFT, 0);
    assert_eq!(D3D12_ANISOTROPIC_FILTERING_BIT, 64);
    assert_eq!(D3D12_RESOURCE_STATE_COMMON, 0);
    assert_eq!(D3D12_RESOURCE_STATE_VERTEX_AND_CONSTANT_BUFFER, 1);
    assert_eq!(D3D12_RESOURCE_STATE_INDEX_BUFFER, 2);
    assert_eq!(D3D12_RESOURCE_STATE_RENDER_TARGET, 4);
    assert_eq!(D3D12_RESOURCE_STATE_UNORDERED_ACCESS, 8);
    assert_eq!(D3D12_RESOURCE_STATE_DEPTH_WRITE, 16);
    assert_eq!(D3D12_RESOURCE_STATE_NON_PIXEL_SHADER_RESOURCE, 64);
    assert_eq!(D3D12_RESOURCE_STATE_INDIRECT_ARGUMENT, 512);
    assert_eq!(D3D12_RESOURCE_STATE_COPY_DEST, 1024);
    assert_eq!(D3D12_RESOURCE_STATE_GENERIC_READ, 2755);
    assert_eq!(D3D12_RESOURCE_STATE_ALL_SHADER_RESOURCE, 192);
    assert_eq!(D3D12_RESOURCE_STATE_PRESENT, 0);
    assert_eq!(D3D12_HEAP_TYPE_DEFAULT, 1);
    assert_eq!(D3D12_HEAP_TYPE_UPLOAD, 2);
    assert_eq!(D3D12_HEAP_TYPE_READBACK, 3);
    assert_eq!(D3D12_HEAP_TYPE_GPU_UPLOAD, 5);
    assert_eq!(D3D12_CPU_PAGE_PROPERTY_UNKNOWN, 0);
    assert_eq!(D3D12_MEMORY_POOL_UNKNOWN, 0);
    assert_eq!(D3D12_HEAP_FLAG_NONE, 0);
    assert_eq!(D3D12_HEAP_FLAG_ALLOW_DISPLAY, 8);
    assert_eq!(D3D12_RESOURCE_DIMENSION_BUFFER, 1);
    assert_eq!(D3D12_RESOURCE_DIMENSION_TEXTURE2D, 3);
    assert_eq!(D3D12_RESOURCE_DIMENSION_TEXTURE3D, 4);
    assert_eq!(D3D12_TEXTURE_LAYOUT_UNKNOWN, 0);
    assert_eq!(D3D12_TEXTURE_LAYOUT_ROW_MAJOR, 1);
    assert_eq!(D3D12_RESOURCE_FLAG_ALLOW_RENDER_TARGET, 1);
    assert_eq!(D3D12_RESOURCE_FLAG_ALLOW_DEPTH_STENCIL, 2);
    assert_eq!(D3D12_RESOURCE_FLAG_ALLOW_UNORDERED_ACCESS, 4);
    assert_eq!(D3D12_DEFAULT_RESOURCE_PLACEMENT_ALIGNMENT, 0x10000);
    assert_eq!(D3D12_DEFAULT_MSAA_RESOURCE_PLACEMENT_ALIGNMENT, 0x400000);
    assert_eq!(D3D12_STANDARD_MULTISAMPLE_PATTERN, 0xffffffff);
    assert_eq!(D3D12_DEFAULT_SHADER_4_COMPONENT_MAPPING, 5768);
    assert_eq!(D3D12_IA_VERTEX_INPUT_STRUCTURE_ELEMENT_COUNT, 32);
    assert_eq!(D3D12_SRV_DIMENSION_BUFFER, 1);
    assert_eq!(D3D12_SRV_DIMENSION_TEXTURE2D, 4);
    assert_eq!(D3D12_SRV_DIMENSION_TEXTURE2DARRAY, 5);
    assert_eq!(D3D12_SRV_DIMENSION_TEXTURE2DMS, 6);
    assert_eq!(D3D12_SRV_DIMENSION_TEXTURE3D, 8);
    assert_eq!(D3D12_SRV_DIMENSION_TEXTURECUBE, 9);
    assert_eq!(D3D12_SRV_DIMENSION_TEXTURECUBEARRAY, 10);
    assert_eq!(D3D12_RTV_DIMENSION_TEXTURE2D, 4);
    assert_eq!(D3D12_RTV_DIMENSION_TEXTURE2DARRAY, 5);
    assert_eq!(D3D12_RTV_DIMENSION_TEXTURE2DMS, 6);
    assert_eq!(D3D12_RTV_DIMENSION_TEXTURE3D, 8);
    assert_eq!(D3D12_DSV_DIMENSION_TEXTURE2D, 3);
    assert_eq!(D3D12_DSV_DIMENSION_TEXTURE2DARRAY, 4);
    assert_eq!(D3D12_DSV_DIMENSION_TEXTURE2DMS, 5);
    assert_eq!(D3D12_UAV_DIMENSION_BUFFER, 1);
    assert_eq!(D3D12_UAV_DIMENSION_TEXTURE2D, 4);
    assert_eq!(D3D12_UAV_DIMENSION_TEXTURE2DARRAY, 5);
    assert_eq!(D3D12_UAV_DIMENSION_TEXTURE3D, 8);
    assert_eq!(D3D12_BUFFER_UAV_FLAG_RAW, 1);
    assert_eq!(D3D12_BUFFER_SRV_FLAG_RAW, 1);
    assert_eq!(D3D12_DESCRIPTOR_RANGE_TYPE_SRV, 0);
    assert_eq!(D3D12_DESCRIPTOR_RANGE_TYPE_UAV, 1);
    assert_eq!(D3D12_DESCRIPTOR_RANGE_TYPE_SAMPLER, 3);
    assert_eq!(D3D12_DESCRIPTOR_RANGE_OFFSET_APPEND, 0xffffffff);
    assert_eq!(D3D12_ROOT_PARAMETER_TYPE_DESCRIPTOR_TABLE, 0);
    assert_eq!(D3D12_ROOT_PARAMETER_TYPE_CBV, 2);
    assert_eq!(D3D12_SHADER_VISIBILITY_ALL, 0);
    assert_eq!(D3D12_SHADER_VISIBILITY_VERTEX, 1);
    assert_eq!(D3D12_SHADER_VISIBILITY_PIXEL, 5);
    assert_eq!(D3D12_ROOT_SIGNATURE_FLAG_NONE, 0);
    assert_eq!(
        D3D12_ROOT_SIGNATURE_FLAG_ALLOW_INPUT_ASSEMBLER_INPUT_LAYOUT,
        1
    );
    assert_eq!(D3D_ROOT_SIGNATURE_VERSION_1, 1);
    assert_eq!(D3D12_PIPELINE_STATE_FLAG_NONE, 0);
    assert_eq!(D3D12_FEATURE_D3D12_OPTIONS, 0);
    assert_eq!(D3D12_FEATURE_FORMAT_SUPPORT, 3);
    assert_eq!(D3D12_FEATURE_MULTISAMPLE_QUALITY_LEVELS, 4);
    assert_eq!(D3D12_FEATURE_SHADER_MODEL, 7);
    assert_eq!(D3D12_FEATURE_D3D12_OPTIONS13, 42);
    assert_eq!(D3D12_FEATURE_D3D12_OPTIONS16, 45);
    assert_eq!(D3D12_FORMAT_SUPPORT1_NONE, 0);
    assert_eq!(D3D12_FORMAT_SUPPORT1_TEXTURE2D, 32);
    assert_eq!(D3D12_FORMAT_SUPPORT1_TEXTURE3D, 64);
    assert_eq!(D3D12_FORMAT_SUPPORT1_TEXTURECUBE, 128);
    assert_eq!(D3D12_FORMAT_SUPPORT1_SHADER_LOAD, 256);
    assert_eq!(D3D12_FORMAT_SUPPORT1_SHADER_SAMPLE, 512);
    assert_eq!(D3D12_FORMAT_SUPPORT1_RENDER_TARGET, 16384);
    assert_eq!(D3D12_FORMAT_SUPPORT1_DEPTH_STENCIL, 0x10000);
    assert_eq!(D3D12_FORMAT_SUPPORT2_NONE, 0);
    assert_eq!(D3D12_FORMAT_SUPPORT2_UAV_TYPED_LOAD, 64);
    assert_eq!(D3D12_FORMAT_SUPPORT2_UAV_TYPED_STORE, 128);
    assert_eq!(D3D_SHADER_MODEL_6_0, 96);
    assert_eq!(D3D12_RESOURCE_BINDING_TIER_2, 2);
    assert_eq!(D3D12_COMMAND_LIST_TYPE_DIRECT, 0);
    assert_eq!(D3D12_COMMAND_QUEUE_FLAG_NONE, 0);
    assert_eq!(D3D12_INDIRECT_ARGUMENT_TYPE_DRAW, 0);
    assert_eq!(D3D12_INDIRECT_ARGUMENT_TYPE_DRAW_INDEXED, 1);
    assert_eq!(D3D12_INDIRECT_ARGUMENT_TYPE_DISPATCH, 2);
    assert_eq!(D3D12_MESSAGE_CATEGORY_APPLICATION_DEFINED, 0);
    assert_eq!(D3D12_MESSAGE_CATEGORY_MISCELLANEOUS, 1);
    assert_eq!(D3D12_MESSAGE_CATEGORY_INITIALIZATION, 2);
    assert_eq!(D3D12_MESSAGE_CATEGORY_CLEANUP, 3);
    assert_eq!(D3D12_MESSAGE_CATEGORY_COMPILATION, 4);
    assert_eq!(D3D12_MESSAGE_CATEGORY_STATE_CREATION, 5);
    assert_eq!(D3D12_MESSAGE_CATEGORY_STATE_SETTING, 6);
    assert_eq!(D3D12_MESSAGE_CATEGORY_STATE_GETTING, 7);
    assert_eq!(D3D12_MESSAGE_CATEGORY_RESOURCE_MANIPULATION, 8);
    assert_eq!(D3D12_MESSAGE_CATEGORY_EXECUTION, 9);
    assert_eq!(D3D12_MESSAGE_CATEGORY_SHADER, 10);
    assert_eq!(D3D12_MESSAGE_SEVERITY_CORRUPTION, 0);
    assert_eq!(D3D12_MESSAGE_SEVERITY_ERROR, 1);
    assert_eq!(D3D12_MESSAGE_SEVERITY_WARNING, 2);
    assert_eq!(D3D12_MESSAGE_SEVERITY_INFO, 3);
    assert_eq!(D3D12_MESSAGE_SEVERITY_MESSAGE, 4);
    assert_eq!(D3D12_MESSAGE_CALLBACK_FLAG_NONE, 0);
    assert_eq!(D3D12_DEVICE_FACTORY_FLAG_ALLOW_RETURNING_EXISTING_DEVICE, 1);
    assert_eq!(DXGI_FORMAT_UNKNOWN, 0);
    assert_eq!(DXGI_FORMAT_R32G32B32A32_FLOAT, 2);
    assert_eq!(DXGI_FORMAT_R32G32B32A32_UINT, 3);
    assert_eq!(DXGI_FORMAT_R32G32B32A32_SINT, 4);
    assert_eq!(DXGI_FORMAT_R32G32B32_FLOAT, 6);
    assert_eq!(DXGI_FORMAT_R32G32B32_UINT, 7);
    assert_eq!(DXGI_FORMAT_R32G32B32_SINT, 8);
    assert_eq!(DXGI_FORMAT_R16G16B16A16_FLOAT, 10);
    assert_eq!(DXGI_FORMAT_R16G16B16A16_UNORM, 11);
    assert_eq!(DXGI_FORMAT_R16G16B16A16_UINT, 12);
    assert_eq!(DXGI_FORMAT_R16G16B16A16_SNORM, 13);
    assert_eq!(DXGI_FORMAT_R16G16B16A16_SINT, 14);
    assert_eq!(DXGI_FORMAT_R32G32_FLOAT, 16);
    assert_eq!(DXGI_FORMAT_R32G32_UINT, 17);
    assert_eq!(DXGI_FORMAT_R32G32_SINT, 18);
    assert_eq!(DXGI_FORMAT_R32G8X24_TYPELESS, 19);
    assert_eq!(DXGI_FORMAT_D32_FLOAT_S8X24_UINT, 20);
    assert_eq!(DXGI_FORMAT_R32_FLOAT_X8X24_TYPELESS, 21);
    assert_eq!(DXGI_FORMAT_R10G10B10A2_UNORM, 24);
    assert_eq!(DXGI_FORMAT_R11G11B10_FLOAT, 26);
    assert_eq!(DXGI_FORMAT_R8G8B8A8_UNORM, 28);
    assert_eq!(DXGI_FORMAT_R8G8B8A8_UNORM_SRGB, 29);
    assert_eq!(DXGI_FORMAT_R8G8B8A8_UINT, 30);
    assert_eq!(DXGI_FORMAT_R8G8B8A8_SNORM, 31);
    assert_eq!(DXGI_FORMAT_R8G8B8A8_SINT, 32);
    assert_eq!(DXGI_FORMAT_R16G16_FLOAT, 34);
    assert_eq!(DXGI_FORMAT_R16G16_UNORM, 35);
    assert_eq!(DXGI_FORMAT_R16G16_UINT, 36);
    assert_eq!(DXGI_FORMAT_R16G16_SNORM, 37);
    assert_eq!(DXGI_FORMAT_R16G16_SINT, 38);
    assert_eq!(DXGI_FORMAT_R32_TYPELESS, 39);
    assert_eq!(DXGI_FORMAT_D32_FLOAT, 40);
    assert_eq!(DXGI_FORMAT_R32_FLOAT, 41);
    assert_eq!(DXGI_FORMAT_R32_UINT, 42);
    assert_eq!(DXGI_FORMAT_R32_SINT, 43);
    assert_eq!(DXGI_FORMAT_R24G8_TYPELESS, 44);
    assert_eq!(DXGI_FORMAT_D24_UNORM_S8_UINT, 45);
    assert_eq!(DXGI_FORMAT_R24_UNORM_X8_TYPELESS, 46);
    assert_eq!(DXGI_FORMAT_R8G8_UNORM, 49);
    assert_eq!(DXGI_FORMAT_R8G8_UINT, 50);
    assert_eq!(DXGI_FORMAT_R8G8_SNORM, 51);
    assert_eq!(DXGI_FORMAT_R8G8_SINT, 52);
    assert_eq!(DXGI_FORMAT_R16_TYPELESS, 53);
    assert_eq!(DXGI_FORMAT_R16_FLOAT, 54);
    assert_eq!(DXGI_FORMAT_D16_UNORM, 55);
    assert_eq!(DXGI_FORMAT_R16_UNORM, 56);
    assert_eq!(DXGI_FORMAT_R16_UINT, 57);
    assert_eq!(DXGI_FORMAT_R16_SNORM, 58);
    assert_eq!(DXGI_FORMAT_R16_SINT, 59);
    assert_eq!(DXGI_FORMAT_R8_UNORM, 61);
    assert_eq!(DXGI_FORMAT_R8_UINT, 62);
    assert_eq!(DXGI_FORMAT_R8_SNORM, 63);
    assert_eq!(DXGI_FORMAT_R8_SINT, 64);
    assert_eq!(DXGI_FORMAT_A8_UNORM, 65);
    assert_eq!(DXGI_FORMAT_BC1_UNORM, 71);
    assert_eq!(DXGI_FORMAT_BC1_UNORM_SRGB, 72);
    assert_eq!(DXGI_FORMAT_BC2_UNORM, 74);
    assert_eq!(DXGI_FORMAT_BC2_UNORM_SRGB, 75);
    assert_eq!(DXGI_FORMAT_BC3_UNORM, 77);
    assert_eq!(DXGI_FORMAT_BC3_UNORM_SRGB, 78);
    assert_eq!(DXGI_FORMAT_BC4_UNORM, 80);
    assert_eq!(DXGI_FORMAT_BC5_UNORM, 83);
    assert_eq!(DXGI_FORMAT_B5G6R5_UNORM, 85);
    assert_eq!(DXGI_FORMAT_B5G5R5A1_UNORM, 86);
    assert_eq!(DXGI_FORMAT_B8G8R8A8_UNORM, 87);
    assert_eq!(DXGI_FORMAT_B8G8R8X8_UNORM, 88);
    assert_eq!(DXGI_FORMAT_B8G8R8A8_UNORM_SRGB, 91);
    assert_eq!(DXGI_FORMAT_B8G8R8X8_UNORM_SRGB, 93);
    assert_eq!(DXGI_FORMAT_BC6H_UF16, 95);
    assert_eq!(DXGI_FORMAT_BC6H_SF16, 96);
    assert_eq!(DXGI_FORMAT_BC7_UNORM, 98);
    assert_eq!(DXGI_FORMAT_BC7_UNORM_SRGB, 99);
    assert_eq!(DXGI_FORMAT_NV12, 103);
    assert_eq!(DXGI_FORMAT_P010, 104);
    assert_eq!(DXGI_FORMAT_B4G4R4A4_UNORM, 115);
    assert_eq!(D3D_FEATURE_LEVEL_11_0, 45056);
    assert_eq!(D3D_FEATURE_LEVEL_11_1, 45312);
    assert_eq!(DXGI_FEATURE_PRESENT_ALLOW_TEARING, 0);
    assert_eq!(DXGI_COLOR_SPACE_RGB_FULL_G22_NONE_P709, 0);
    assert_eq!(DXGI_COLOR_SPACE_RGB_FULL_G10_NONE_P709, 1);
    assert_eq!(DXGI_COLOR_SPACE_RGB_FULL_G2084_NONE_P2020, 12);
    assert_eq!(DXGI_GPU_PREFERENCE_MINIMUM_POWER, 1);
    assert_eq!(DXGI_GPU_PREFERENCE_HIGH_PERFORMANCE, 2);
    assert_eq!(DXGI_DEBUG_RLO_SUMMARY, 1);
    assert_eq!(DXGI_DEBUG_RLO_DETAIL, 2);
    assert_eq!(E_FAIL as u32, 0x80004005);
    assert_eq!(DXGI_ERROR_DEVICE_REMOVED as u32, 0x887a0005);
}

#[test]
fn guids_match_the_headers() {
    // The GUIDs of the headers (harness).
    assert_eq!(
        guid(IID_ID3D12DEVICE),
        0x189819f1_1db6_4b57_be54_1821339b85f7
    );
    assert_eq!(
        guid(IID_ID3D12DEBUG),
        0x344488b7_6846_474b_b989_f027448245e0
    );
    assert_eq!(
        guid(CLSID_ID3D12DEBUG),
        0xf2352aeb_dd84_49fe_b97b_a9dcfdcc1b4f
    );
    assert_eq!(
        guid(IID_IDXGIFACTORY1),
        0x770aae78_f26f_4dba_a829_253c83d1b387
    );
    assert_eq!(
        guid(IID_IDXGIFACTORY4),
        0x1bc6ea02_ef36_464f_bf0c_21ca39e5168a
    );
    assert_eq!(
        guid(IID_IDXGIFACTORY5),
        0x7632e1f5_ee65_4dca_87fd_84cd75f8838d
    );
    assert_eq!(
        guid(IID_IDXGIFACTORY6),
        0xc1b6694f_ff09_44a9_b03c_77900a0a1d17
    );
    assert_eq!(
        guid(IID_IDXGIADAPTER1),
        0x29038f61_3839_4626_91fd_086879011a05
    );
    assert_eq!(
        guid(IID_IDXGIDEVICE),
        0x54ec77fa_1377_44e6_8c32_88fd5f44c84c
    );
    assert_eq!(guid(IID_IDXGIDEBUG), 0x119e7452_de9e_40fe_8806_88f90c12b441);
    assert_eq!(
        guid(IID_IDXGIINFOQUEUE),
        0xd67441c7_672a_476f_9e82_cd55b44949ce
    );
    assert_eq!(guid(DXGI_DEBUG_ALL), 0xe48ae283_da80_490b_87e6_43e9a9cfda08);
}

#[test]
fn struct_layouts_match_the_headers() {
    // sizeof, _Alignof and offsetof of the harness.
    assert_eq!(
        (
            size_of::<CpuDescriptorHandle>(),
            align_of::<CpuDescriptorHandle>()
        ),
        (8, 8)
    );
    assert_eq!(offset_of!(CpuDescriptorHandle, ptr), 0);
    assert_eq!(
        (
            size_of::<GpuDescriptorHandle>(),
            align_of::<GpuDescriptorHandle>()
        ),
        (8, 8)
    );
    assert_eq!(offset_of!(GpuDescriptorHandle, ptr), 0);
    assert_eq!(
        (
            size_of::<DescriptorHeapDesc>(),
            align_of::<DescriptorHeapDesc>()
        ),
        (16, 4)
    );
    assert_eq!(offset_of!(DescriptorHeapDesc, ty), 0);
    assert_eq!(offset_of!(DescriptorHeapDesc, num_descriptors), 4);
    assert_eq!(offset_of!(DescriptorHeapDesc, flags), 8);
    assert_eq!(offset_of!(DescriptorHeapDesc, node_mask), 12);
    assert_eq!(
        (
            size_of::<CommandQueueDesc>(),
            align_of::<CommandQueueDesc>()
        ),
        (16, 4)
    );
    assert_eq!(offset_of!(CommandQueueDesc, ty), 0);
    assert_eq!(offset_of!(CommandQueueDesc, priority), 4);
    assert_eq!(offset_of!(CommandQueueDesc, flags), 8);
    assert_eq!(offset_of!(CommandQueueDesc, node_mask), 12);
    assert_eq!(
        (size_of::<HeapProperties>(), align_of::<HeapProperties>()),
        (20, 4)
    );
    assert_eq!(offset_of!(HeapProperties, ty), 0);
    assert_eq!(offset_of!(HeapProperties, cpu_page_property), 4);
    assert_eq!(offset_of!(HeapProperties, memory_pool_preference), 8);
    assert_eq!(offset_of!(HeapProperties, creation_node_mask), 12);
    assert_eq!(offset_of!(HeapProperties, visible_node_mask), 16);
    assert_eq!(
        (size_of::<ResourceDesc>(), align_of::<ResourceDesc>()),
        (56, 8)
    );
    assert_eq!(offset_of!(ResourceDesc, dimension), 0);
    assert_eq!(offset_of!(ResourceDesc, alignment), 8);
    assert_eq!(offset_of!(ResourceDesc, width), 16);
    assert_eq!(offset_of!(ResourceDesc, height), 24);
    assert_eq!(offset_of!(ResourceDesc, depth_or_array_size), 28);
    assert_eq!(offset_of!(ResourceDesc, mip_levels), 30);
    assert_eq!(offset_of!(ResourceDesc, format), 32);
    assert_eq!(offset_of!(ResourceDesc, sample_desc), 36);
    assert_eq!(offset_of!(ResourceDesc, layout), 44);
    assert_eq!(offset_of!(ResourceDesc, flags), 48);
    assert_eq!(
        (
            size_of::<DepthStencilValue>(),
            align_of::<DepthStencilValue>()
        ),
        (8, 4)
    );
    assert_eq!(offset_of!(DepthStencilValue, depth), 0);
    assert_eq!(offset_of!(DepthStencilValue, stencil), 4);
    assert_eq!((size_of::<ClearValue>(), align_of::<ClearValue>()), (20, 4));
    assert_eq!(offset_of!(ClearValue, format), 0);
    assert_eq!(offset_of!(ClearValue, u), 4);
    assert_eq!((size_of::<BufferSrv>(), align_of::<BufferSrv>()), (24, 8));
    assert_eq!(offset_of!(BufferSrv, first_element), 0);
    assert_eq!(offset_of!(BufferSrv, num_elements), 8);
    assert_eq!(offset_of!(BufferSrv, structure_byte_stride), 12);
    assert_eq!(offset_of!(BufferSrv, flags), 16);
    assert_eq!((size_of::<Tex2dSrv>(), align_of::<Tex2dSrv>()), (16, 4));
    assert_eq!(offset_of!(Tex2dSrv, most_detailed_mip), 0);
    assert_eq!(offset_of!(Tex2dSrv, mip_levels), 4);
    assert_eq!(offset_of!(Tex2dSrv, plane_slice), 8);
    assert_eq!(offset_of!(Tex2dSrv, resource_min_lod_clamp), 12);
    assert_eq!(
        (size_of::<Tex2dArraySrv>(), align_of::<Tex2dArraySrv>()),
        (24, 4)
    );
    assert_eq!(offset_of!(Tex2dArraySrv, most_detailed_mip), 0);
    assert_eq!(offset_of!(Tex2dArraySrv, mip_levels), 4);
    assert_eq!(offset_of!(Tex2dArraySrv, first_array_slice), 8);
    assert_eq!(offset_of!(Tex2dArraySrv, array_size), 12);
    assert_eq!(offset_of!(Tex2dArraySrv, plane_slice), 16);
    assert_eq!(offset_of!(Tex2dArraySrv, resource_min_lod_clamp), 20);
    assert_eq!((size_of::<Tex2dmsSrv>(), align_of::<Tex2dmsSrv>()), (4, 4));
    assert_eq!(offset_of!(Tex2dmsSrv, unused_field_nothing_to_define), 0);
    assert_eq!((size_of::<Tex3dSrv>(), align_of::<Tex3dSrv>()), (12, 4));
    assert_eq!(offset_of!(Tex3dSrv, most_detailed_mip), 0);
    assert_eq!(offset_of!(Tex3dSrv, mip_levels), 4);
    assert_eq!(offset_of!(Tex3dSrv, resource_min_lod_clamp), 8);
    assert_eq!((size_of::<Tex3dSrv>(), align_of::<Tex3dSrv>()), (12, 4));
    assert_eq!(offset_of!(Tex3dSrv, most_detailed_mip), 0);
    assert_eq!(offset_of!(Tex3dSrv, mip_levels), 4);
    assert_eq!(offset_of!(Tex3dSrv, resource_min_lod_clamp), 8);
    assert_eq!(
        (size_of::<TexCubeArraySrv>(), align_of::<TexCubeArraySrv>()),
        (20, 4)
    );
    assert_eq!(offset_of!(TexCubeArraySrv, most_detailed_mip), 0);
    assert_eq!(offset_of!(TexCubeArraySrv, mip_levels), 4);
    assert_eq!(offset_of!(TexCubeArraySrv, first_2d_array_face), 8);
    assert_eq!(offset_of!(TexCubeArraySrv, num_cubes), 12);
    assert_eq!(offset_of!(TexCubeArraySrv, resource_min_lod_clamp), 16);
    assert_eq!(
        (
            size_of::<ShaderResourceViewDesc>(),
            align_of::<ShaderResourceViewDesc>()
        ),
        (40, 8)
    );
    assert_eq!(offset_of!(ShaderResourceViewDesc, format), 0);
    assert_eq!(offset_of!(ShaderResourceViewDesc, view_dimension), 4);
    assert_eq!(
        offset_of!(ShaderResourceViewDesc, shader_4_component_mapping),
        8
    );
    assert_eq!(offset_of!(ShaderResourceViewDesc, u), 16);
    assert_eq!((size_of::<Tex2dRtv>(), align_of::<Tex2dRtv>()), (8, 4));
    assert_eq!(offset_of!(Tex2dRtv, mip_slice), 0);
    assert_eq!(offset_of!(Tex2dRtv, plane_slice), 4);
    assert_eq!((size_of::<Tex2dRtv>(), align_of::<Tex2dRtv>()), (8, 4));
    assert_eq!(offset_of!(Tex2dRtv, mip_slice), 0);
    assert_eq!(offset_of!(Tex2dRtv, plane_slice), 4);
    assert_eq!(
        (size_of::<Tex2dArrayRtv>(), align_of::<Tex2dArrayRtv>()),
        (16, 4)
    );
    assert_eq!(offset_of!(Tex2dArrayRtv, mip_slice), 0);
    assert_eq!(offset_of!(Tex2dArrayRtv, first_array_slice), 4);
    assert_eq!(offset_of!(Tex2dArrayRtv, array_size), 8);
    assert_eq!(offset_of!(Tex2dArrayRtv, plane_slice), 12);
    assert_eq!(
        (size_of::<Tex2dArrayRtv>(), align_of::<Tex2dArrayRtv>()),
        (16, 4)
    );
    assert_eq!(offset_of!(Tex2dArrayRtv, mip_slice), 0);
    assert_eq!(offset_of!(Tex2dArrayRtv, first_array_slice), 4);
    assert_eq!(offset_of!(Tex2dArrayRtv, array_size), 8);
    assert_eq!(offset_of!(Tex2dArrayRtv, plane_slice), 12);
    assert_eq!((size_of::<Tex3dRtv>(), align_of::<Tex3dRtv>()), (12, 4));
    assert_eq!(offset_of!(Tex3dRtv, mip_slice), 0);
    assert_eq!(offset_of!(Tex3dRtv, first_w_slice), 4);
    assert_eq!(offset_of!(Tex3dRtv, w_size), 8);
    assert_eq!((size_of::<Tex3dRtv>(), align_of::<Tex3dRtv>()), (12, 4));
    assert_eq!(offset_of!(Tex3dRtv, mip_slice), 0);
    assert_eq!(offset_of!(Tex3dRtv, first_w_slice), 4);
    assert_eq!(offset_of!(Tex3dRtv, w_size), 8);
    assert_eq!((size_of::<BufferRtv>(), align_of::<BufferRtv>()), (16, 8));
    assert_eq!(offset_of!(BufferRtv, first_element), 0);
    assert_eq!(offset_of!(BufferRtv, num_elements), 8);
    assert_eq!(
        (
            size_of::<RenderTargetViewDesc>(),
            align_of::<RenderTargetViewDesc>()
        ),
        (24, 8)
    );
    assert_eq!(offset_of!(RenderTargetViewDesc, format), 0);
    assert_eq!(offset_of!(RenderTargetViewDesc, view_dimension), 4);
    assert_eq!(offset_of!(RenderTargetViewDesc, u), 8);
    assert_eq!((size_of::<Tex2dDsv>(), align_of::<Tex2dDsv>()), (4, 4));
    assert_eq!(offset_of!(Tex2dDsv, mip_slice), 0);
    assert_eq!(
        (size_of::<Tex2dArrayDsv>(), align_of::<Tex2dArrayDsv>()),
        (12, 4)
    );
    assert_eq!(offset_of!(Tex2dArrayDsv, mip_slice), 0);
    assert_eq!(offset_of!(Tex2dArrayDsv, first_array_slice), 4);
    assert_eq!(offset_of!(Tex2dArrayDsv, array_size), 8);
    assert_eq!(
        (
            size_of::<DepthStencilViewDesc>(),
            align_of::<DepthStencilViewDesc>()
        ),
        (24, 4)
    );
    assert_eq!(offset_of!(DepthStencilViewDesc, format), 0);
    assert_eq!(offset_of!(DepthStencilViewDesc, view_dimension), 4);
    assert_eq!(offset_of!(DepthStencilViewDesc, flags), 8);
    assert_eq!(offset_of!(DepthStencilViewDesc, u), 12);
    assert_eq!((size_of::<BufferUav>(), align_of::<BufferUav>()), (32, 8));
    assert_eq!(offset_of!(BufferUav, first_element), 0);
    assert_eq!(offset_of!(BufferUav, num_elements), 8);
    assert_eq!(offset_of!(BufferUav, structure_byte_stride), 12);
    assert_eq!(offset_of!(BufferUav, counter_offset_in_bytes), 16);
    assert_eq!(offset_of!(BufferUav, flags), 24);
    assert_eq!(
        (
            size_of::<UnorderedAccessViewDesc>(),
            align_of::<UnorderedAccessViewDesc>()
        ),
        (40, 8)
    );
    assert_eq!(offset_of!(UnorderedAccessViewDesc, format), 0);
    assert_eq!(offset_of!(UnorderedAccessViewDesc, view_dimension), 4);
    assert_eq!(offset_of!(UnorderedAccessViewDesc, u), 8);
    assert_eq!(
        (size_of::<SamplerDesc>(), align_of::<SamplerDesc>()),
        (52, 4)
    );
    assert_eq!(offset_of!(SamplerDesc, filter), 0);
    assert_eq!(offset_of!(SamplerDesc, address_u), 4);
    assert_eq!(offset_of!(SamplerDesc, address_v), 8);
    assert_eq!(offset_of!(SamplerDesc, address_w), 12);
    assert_eq!(offset_of!(SamplerDesc, mip_lod_bias), 16);
    assert_eq!(offset_of!(SamplerDesc, max_anisotropy), 20);
    assert_eq!(offset_of!(SamplerDesc, comparison_func), 24);
    assert_eq!(offset_of!(SamplerDesc, border_color), 28);
    assert_eq!(offset_of!(SamplerDesc, min_lod), 44);
    assert_eq!(offset_of!(SamplerDesc, max_lod), 48);
    assert_eq!(
        (size_of::<DescriptorRange>(), align_of::<DescriptorRange>()),
        (20, 4)
    );
    assert_eq!(offset_of!(DescriptorRange, range_type), 0);
    assert_eq!(offset_of!(DescriptorRange, num_descriptors), 4);
    assert_eq!(offset_of!(DescriptorRange, base_shader_register), 8);
    assert_eq!(offset_of!(DescriptorRange, register_space), 12);
    assert_eq!(
        offset_of!(DescriptorRange, offset_in_descriptors_from_table_start),
        16
    );
    assert_eq!(
        (
            size_of::<RootDescriptorTable>(),
            align_of::<RootDescriptorTable>()
        ),
        (16, 8)
    );
    assert_eq!(offset_of!(RootDescriptorTable, num_descriptor_ranges), 0);
    assert_eq!(offset_of!(RootDescriptorTable, descriptor_ranges), 8);
    assert_eq!(
        (size_of::<RootDescriptor>(), align_of::<RootDescriptor>()),
        (8, 4)
    );
    assert_eq!(offset_of!(RootDescriptor, shader_register), 0);
    assert_eq!(offset_of!(RootDescriptor, register_space), 4);
    assert_eq!(
        (size_of::<RootParameter>(), align_of::<RootParameter>()),
        (32, 8)
    );
    assert_eq!(offset_of!(RootParameter, parameter_type), 0);
    assert_eq!(offset_of!(RootParameter, u), 8);
    assert_eq!(offset_of!(RootParameter, shader_visibility), 24);
    assert_eq!(
        (
            size_of::<RootSignatureDesc>(),
            align_of::<RootSignatureDesc>()
        ),
        (40, 8)
    );
    assert_eq!(offset_of!(RootSignatureDesc, num_parameters), 0);
    assert_eq!(offset_of!(RootSignatureDesc, parameters), 8);
    assert_eq!(offset_of!(RootSignatureDesc, num_static_samplers), 16);
    assert_eq!(offset_of!(RootSignatureDesc, static_samplers), 24);
    assert_eq!(offset_of!(RootSignatureDesc, flags), 32);
    assert_eq!(
        (size_of::<ShaderBytecode>(), align_of::<ShaderBytecode>()),
        (16, 8)
    );
    assert_eq!(offset_of!(ShaderBytecode, shader_bytecode), 0);
    assert_eq!(offset_of!(ShaderBytecode, bytecode_length), 8);
    assert_eq!(
        (
            size_of::<StreamOutputDesc>(),
            align_of::<StreamOutputDesc>()
        ),
        (32, 8)
    );
    assert_eq!(offset_of!(StreamOutputDesc, so_declaration), 0);
    assert_eq!(offset_of!(StreamOutputDesc, num_entries), 8);
    assert_eq!(offset_of!(StreamOutputDesc, buffer_strides), 16);
    assert_eq!(offset_of!(StreamOutputDesc, num_strides), 24);
    assert_eq!(offset_of!(StreamOutputDesc, rasterized_stream), 28);
    assert_eq!(
        (
            size_of::<RenderTargetBlendDesc>(),
            align_of::<RenderTargetBlendDesc>()
        ),
        (40, 4)
    );
    assert_eq!(offset_of!(RenderTargetBlendDesc, blend_enable), 0);
    assert_eq!(offset_of!(RenderTargetBlendDesc, logic_op_enable), 4);
    assert_eq!(offset_of!(RenderTargetBlendDesc, src_blend), 8);
    assert_eq!(offset_of!(RenderTargetBlendDesc, dest_blend), 12);
    assert_eq!(offset_of!(RenderTargetBlendDesc, blend_op), 16);
    assert_eq!(offset_of!(RenderTargetBlendDesc, src_blend_alpha), 20);
    assert_eq!(offset_of!(RenderTargetBlendDesc, dest_blend_alpha), 24);
    assert_eq!(offset_of!(RenderTargetBlendDesc, blend_op_alpha), 28);
    assert_eq!(offset_of!(RenderTargetBlendDesc, logic_op), 32);
    assert_eq!(
        offset_of!(RenderTargetBlendDesc, render_target_write_mask),
        36
    );
    assert_eq!((size_of::<BlendDesc>(), align_of::<BlendDesc>()), (328, 4));
    assert_eq!(offset_of!(BlendDesc, alpha_to_coverage_enable), 0);
    assert_eq!(offset_of!(BlendDesc, independent_blend_enable), 4);
    assert_eq!(offset_of!(BlendDesc, render_target), 8);
    assert_eq!(
        (size_of::<RasterizerDesc>(), align_of::<RasterizerDesc>()),
        (44, 4)
    );
    assert_eq!(offset_of!(RasterizerDesc, fill_mode), 0);
    assert_eq!(offset_of!(RasterizerDesc, cull_mode), 4);
    assert_eq!(offset_of!(RasterizerDesc, front_counter_clockwise), 8);
    assert_eq!(offset_of!(RasterizerDesc, depth_bias), 12);
    assert_eq!(offset_of!(RasterizerDesc, depth_bias_clamp), 16);
    assert_eq!(offset_of!(RasterizerDesc, slope_scaled_depth_bias), 20);
    assert_eq!(offset_of!(RasterizerDesc, depth_clip_enable), 24);
    assert_eq!(offset_of!(RasterizerDesc, multisample_enable), 28);
    assert_eq!(offset_of!(RasterizerDesc, antialiased_line_enable), 32);
    assert_eq!(offset_of!(RasterizerDesc, forced_sample_count), 36);
    assert_eq!(offset_of!(RasterizerDesc, conservative_raster), 40);
    assert_eq!(
        (
            size_of::<DepthStencilOpDesc>(),
            align_of::<DepthStencilOpDesc>()
        ),
        (16, 4)
    );
    assert_eq!(offset_of!(DepthStencilOpDesc, stencil_fail_op), 0);
    assert_eq!(offset_of!(DepthStencilOpDesc, stencil_depth_fail_op), 4);
    assert_eq!(offset_of!(DepthStencilOpDesc, stencil_pass_op), 8);
    assert_eq!(offset_of!(DepthStencilOpDesc, stencil_func), 12);
    assert_eq!(
        (
            size_of::<DepthStencilDesc>(),
            align_of::<DepthStencilDesc>()
        ),
        (52, 4)
    );
    assert_eq!(offset_of!(DepthStencilDesc, depth_enable), 0);
    assert_eq!(offset_of!(DepthStencilDesc, depth_write_mask), 4);
    assert_eq!(offset_of!(DepthStencilDesc, depth_func), 8);
    assert_eq!(offset_of!(DepthStencilDesc, stencil_enable), 12);
    assert_eq!(offset_of!(DepthStencilDesc, stencil_read_mask), 16);
    assert_eq!(offset_of!(DepthStencilDesc, stencil_write_mask), 17);
    assert_eq!(offset_of!(DepthStencilDesc, front_face), 20);
    assert_eq!(offset_of!(DepthStencilDesc, back_face), 36);
    assert_eq!(
        (
            size_of::<InputElementDesc>(),
            align_of::<InputElementDesc>()
        ),
        (32, 8)
    );
    assert_eq!(offset_of!(InputElementDesc, semantic_name), 0);
    assert_eq!(offset_of!(InputElementDesc, semantic_index), 8);
    assert_eq!(offset_of!(InputElementDesc, format), 12);
    assert_eq!(offset_of!(InputElementDesc, input_slot), 16);
    assert_eq!(offset_of!(InputElementDesc, aligned_byte_offset), 20);
    assert_eq!(offset_of!(InputElementDesc, input_slot_class), 24);
    assert_eq!(offset_of!(InputElementDesc, instance_data_step_rate), 28);
    assert_eq!(
        (size_of::<InputLayoutDesc>(), align_of::<InputLayoutDesc>()),
        (16, 8)
    );
    assert_eq!(offset_of!(InputLayoutDesc, input_element_descs), 0);
    assert_eq!(offset_of!(InputLayoutDesc, num_elements), 8);
    assert_eq!(
        (
            size_of::<CachedPipelineState>(),
            align_of::<CachedPipelineState>()
        ),
        (16, 8)
    );
    assert_eq!(offset_of!(CachedPipelineState, cached_blob), 0);
    assert_eq!(
        offset_of!(CachedPipelineState, cached_blob_size_in_bytes),
        8
    );
    assert_eq!(
        (
            size_of::<GraphicsPipelineStateDesc>(),
            align_of::<GraphicsPipelineStateDesc>()
        ),
        (656, 8)
    );
    assert_eq!(offset_of!(GraphicsPipelineStateDesc, root_signature), 0);
    assert_eq!(offset_of!(GraphicsPipelineStateDesc, vs), 8);
    assert_eq!(offset_of!(GraphicsPipelineStateDesc, ps), 24);
    assert_eq!(offset_of!(GraphicsPipelineStateDesc, ds), 40);
    assert_eq!(offset_of!(GraphicsPipelineStateDesc, hs), 56);
    assert_eq!(offset_of!(GraphicsPipelineStateDesc, gs), 72);
    assert_eq!(offset_of!(GraphicsPipelineStateDesc, stream_output), 88);
    assert_eq!(offset_of!(GraphicsPipelineStateDesc, blend_state), 120);
    assert_eq!(offset_of!(GraphicsPipelineStateDesc, sample_mask), 448);
    assert_eq!(offset_of!(GraphicsPipelineStateDesc, rasterizer_state), 452);
    assert_eq!(
        offset_of!(GraphicsPipelineStateDesc, depth_stencil_state),
        496
    );
    assert_eq!(offset_of!(GraphicsPipelineStateDesc, input_layout), 552);
    assert_eq!(
        offset_of!(GraphicsPipelineStateDesc, ib_strip_cut_value),
        568
    );
    assert_eq!(
        offset_of!(GraphicsPipelineStateDesc, primitive_topology_type),
        572
    );
    assert_eq!(
        offset_of!(GraphicsPipelineStateDesc, num_render_targets),
        576
    );
    assert_eq!(offset_of!(GraphicsPipelineStateDesc, rtv_formats), 580);
    assert_eq!(offset_of!(GraphicsPipelineStateDesc, dsv_format), 612);
    assert_eq!(offset_of!(GraphicsPipelineStateDesc, sample_desc), 616);
    assert_eq!(offset_of!(GraphicsPipelineStateDesc, node_mask), 624);
    assert_eq!(offset_of!(GraphicsPipelineStateDesc, cached_pso), 632);
    assert_eq!(offset_of!(GraphicsPipelineStateDesc, flags), 648);
    assert_eq!(
        (
            size_of::<ComputePipelineStateDesc>(),
            align_of::<ComputePipelineStateDesc>()
        ),
        (56, 8)
    );
    assert_eq!(offset_of!(ComputePipelineStateDesc, root_signature), 0);
    assert_eq!(offset_of!(ComputePipelineStateDesc, cs), 8);
    assert_eq!(offset_of!(ComputePipelineStateDesc, node_mask), 24);
    assert_eq!(offset_of!(ComputePipelineStateDesc, cached_pso), 32);
    assert_eq!(offset_of!(ComputePipelineStateDesc, flags), 48);
    assert_eq!(
        (
            size_of::<IndirectArgumentDesc>(),
            align_of::<IndirectArgumentDesc>()
        ),
        (16, 4)
    );
    assert_eq!(offset_of!(IndirectArgumentDesc, ty), 0);
    assert_eq!(
        (
            size_of::<CommandSignatureDesc>(),
            align_of::<CommandSignatureDesc>()
        ),
        (24, 8)
    );
    assert_eq!(offset_of!(CommandSignatureDesc, byte_stride), 0);
    assert_eq!(offset_of!(CommandSignatureDesc, num_argument_descs), 4);
    assert_eq!(offset_of!(CommandSignatureDesc, argument_descs), 8);
    assert_eq!(offset_of!(CommandSignatureDesc, node_mask), 16);
    assert_eq!(
        (
            size_of::<FeatureDataFormatSupport>(),
            align_of::<FeatureDataFormatSupport>()
        ),
        (12, 4)
    );
    assert_eq!(offset_of!(FeatureDataFormatSupport, format), 0);
    assert_eq!(offset_of!(FeatureDataFormatSupport, support1), 4);
    assert_eq!(offset_of!(FeatureDataFormatSupport, support2), 8);
    assert_eq!(
        (
            size_of::<FeatureDataMultisampleQualityLevels>(),
            align_of::<FeatureDataMultisampleQualityLevels>()
        ),
        (16, 4)
    );
    assert_eq!(offset_of!(FeatureDataMultisampleQualityLevels, format), 0);
    assert_eq!(
        offset_of!(FeatureDataMultisampleQualityLevels, sample_count),
        4
    );
    assert_eq!(offset_of!(FeatureDataMultisampleQualityLevels, flags), 8);
    assert_eq!(
        offset_of!(FeatureDataMultisampleQualityLevels, num_quality_levels),
        12
    );
    assert_eq!(
        (
            size_of::<FeatureDataD3d12Options>(),
            align_of::<FeatureDataD3d12Options>()
        ),
        (60, 4)
    );
    assert_eq!(
        offset_of!(FeatureDataD3d12Options, resource_binding_tier),
        16
    );
    assert_eq!(offset_of!(FeatureDataD3d12Options, resource_heap_tier), 56);
    assert_eq!(
        (
            size_of::<FeatureDataShaderModel>(),
            align_of::<FeatureDataShaderModel>()
        ),
        (4, 4)
    );
    assert_eq!(offset_of!(FeatureDataShaderModel, highest_shader_model), 0);
    assert_eq!(
        (
            size_of::<FeatureDataD3d12Options13>(),
            align_of::<FeatureDataD3d12Options13>()
        ),
        (24, 4)
    );
    assert_eq!(
        offset_of!(
            FeatureDataD3d12Options13,
            unrestricted_buffer_texture_copy_pitch_supported
        ),
        0
    );
    assert_eq!(
        offset_of!(FeatureDataD3d12Options13, alpha_blend_factor_supported),
        20
    );
    assert_eq!(
        (
            size_of::<FeatureDataD3d12Options16>(),
            align_of::<FeatureDataD3d12Options16>()
        ),
        (8, 4)
    );
    assert_eq!(
        offset_of!(FeatureDataD3d12Options16, dynamic_depth_bias_supported),
        0
    );
    assert_eq!(
        offset_of!(FeatureDataD3d12Options16, gpu_upload_heap_supported),
        4
    );
    assert_eq!((size_of::<Message>(), align_of::<Message>()), (32, 8));
    assert_eq!(offset_of!(Message, category), 0);
    assert_eq!(offset_of!(Message, severity), 4);
    assert_eq!(offset_of!(Message, id), 8);
    assert_eq!(offset_of!(Message, description), 16);
    assert_eq!(offset_of!(Message, description_byte_length), 24);
    assert_eq!(
        (
            size_of::<InfoQueueFilterDesc>(),
            align_of::<InfoQueueFilterDesc>()
        ),
        (48, 8)
    );
    assert_eq!(offset_of!(InfoQueueFilterDesc, num_categories), 0);
    assert_eq!(offset_of!(InfoQueueFilterDesc, category_list), 8);
    assert_eq!(offset_of!(InfoQueueFilterDesc, num_severities), 16);
    assert_eq!(offset_of!(InfoQueueFilterDesc, severity_list), 24);
    assert_eq!(offset_of!(InfoQueueFilterDesc, num_ids), 32);
    assert_eq!(offset_of!(InfoQueueFilterDesc, id_list), 40);
    assert_eq!(
        (size_of::<InfoQueueFilter>(), align_of::<InfoQueueFilter>()),
        (96, 8)
    );
    assert_eq!(offset_of!(InfoQueueFilter, allow_list), 0);
    assert_eq!(offset_of!(InfoQueueFilter, deny_list), 48);
    assert_eq!(
        (size_of::<AdapterDesc1>(), align_of::<AdapterDesc1>()),
        (312, 8)
    );
    assert_eq!(offset_of!(AdapterDesc1, description), 0);
    assert_eq!(offset_of!(AdapterDesc1, vendor_id), 256);
    assert_eq!(offset_of!(AdapterDesc1, device_id), 260);
    assert_eq!(offset_of!(AdapterDesc1, sub_sys_id), 264);
    assert_eq!(offset_of!(AdapterDesc1, revision), 268);
    assert_eq!(offset_of!(AdapterDesc1, dedicated_video_memory), 272);
    assert_eq!(offset_of!(AdapterDesc1, dedicated_system_memory), 280);
    assert_eq!(offset_of!(AdapterDesc1, shared_system_memory), 288);
    assert_eq!(offset_of!(AdapterDesc1, adapter_luid), 296);
    assert_eq!(offset_of!(AdapterDesc1, flags), 304);
    assert_eq!(size_of::<crate::gpu::IndirectDrawCommand>(), 16);
    assert_eq!(size_of::<crate::gpu::IndexedIndirectDrawCommand>(), 20);
    assert_eq!(size_of::<crate::gpu::IndirectDispatchCommand>(), 12);
}

#[test]
fn vtable_slots_match_the_headers() {
    // offsetof(<Interface>Vtbl, <Method>) / sizeof(void *) of the harness.
    assert_eq!(slot(offset_of!(ID3D12DeviceVtbl, object.set_name)), 6);
    assert_eq!(slot(offset_of!(ID3D12DeviceVtbl, create_command_queue)), 8);
    assert_eq!(
        slot(offset_of!(ID3D12DeviceVtbl, create_graphics_pipeline_state)),
        10
    );
    assert_eq!(
        slot(offset_of!(ID3D12DeviceVtbl, create_compute_pipeline_state)),
        11
    );
    assert_eq!(
        slot(offset_of!(ID3D12DeviceVtbl, check_feature_support)),
        13
    );
    assert_eq!(
        slot(offset_of!(ID3D12DeviceVtbl, create_descriptor_heap)),
        14
    );
    assert_eq!(
        slot(offset_of!(
            ID3D12DeviceVtbl,
            get_descriptor_handle_increment_size
        )),
        15
    );
    assert_eq!(
        slot(offset_of!(ID3D12DeviceVtbl, create_root_signature)),
        16
    );
    assert_eq!(
        slot(offset_of!(ID3D12DeviceVtbl, create_shader_resource_view)),
        18
    );
    assert_eq!(
        slot(offset_of!(ID3D12DeviceVtbl, create_unordered_access_view)),
        19
    );
    assert_eq!(
        slot(offset_of!(ID3D12DeviceVtbl, create_render_target_view)),
        20
    );
    assert_eq!(
        slot(offset_of!(ID3D12DeviceVtbl, create_depth_stencil_view)),
        21
    );
    assert_eq!(slot(offset_of!(ID3D12DeviceVtbl, create_sampler)), 22);
    assert_eq!(
        slot(offset_of!(ID3D12DeviceVtbl, create_committed_resource)),
        27
    );
    assert_eq!(
        slot(offset_of!(ID3D12DeviceVtbl, get_device_removed_reason)),
        37
    );
    assert_eq!(
        slot(offset_of!(ID3D12DeviceVtbl, create_command_signature)),
        41
    );
    assert_eq!(size_of::<ID3D12DeviceVtbl>(), 44 * PTR);
    assert_eq!(
        slot(offset_of!(ID3D12DescriptorHeapVtbl, object.set_name)),
        6
    );
    assert_eq!(
        slot(offset_of!(
            ID3D12DescriptorHeapVtbl,
            get_cpu_descriptor_handle_for_heap_start
        )),
        9
    );
    assert_eq!(
        slot(offset_of!(
            ID3D12DescriptorHeapVtbl,
            get_gpu_descriptor_handle_for_heap_start
        )),
        10
    );
    assert_eq!(size_of::<ID3D12DescriptorHeapVtbl>(), 11 * PTR);
    assert_eq!(slot(offset_of!(ID3D12ResourceVtbl, object.set_name)), 6);
    assert_eq!(slot(offset_of!(ID3D12ResourceVtbl, map)), 8);
    assert_eq!(slot(offset_of!(ID3D12ResourceVtbl, unmap)), 9);
    assert_eq!(
        slot(offset_of!(ID3D12ResourceVtbl, get_gpu_virtual_address)),
        11
    );
    assert_eq!(size_of::<ID3D12ResourceVtbl>(), 15 * PTR);
    assert_eq!(
        slot(offset_of!(ID3D12PipelineStateVtbl, object.set_name)),
        6
    );
    assert_eq!(slot(offset_of!(ID3DBlobVtbl, get_buffer_pointer)), 3);
    assert_eq!(slot(offset_of!(ID3DBlobVtbl, get_buffer_size)), 4);
    assert_eq!(size_of::<ID3DBlobVtbl>(), 5 * PTR);
    assert_eq!(slot(offset_of!(ID3D12DebugVtbl, enable_debug_layer)), 3);
    assert_eq!(size_of::<ID3D12DebugVtbl>(), 4 * PTR);
    assert_eq!(
        slot(offset_of!(ID3D12InfoQueueVtbl, clear_stored_messages)),
        4
    );
    assert_eq!(slot(offset_of!(ID3D12InfoQueueVtbl, get_message)), 5);
    assert_eq!(
        slot(offset_of!(ID3D12InfoQueueVtbl, get_num_stored_messages)),
        8
    );
    assert_eq!(
        slot(offset_of!(ID3D12InfoQueueVtbl, push_storage_filter)),
        17
    );
    assert_eq!(
        slot(offset_of!(ID3D12InfoQueueVtbl, set_break_on_severity)),
        31
    );
    assert_eq!(size_of::<ID3D12InfoQueueVtbl>(), 38 * PTR);
    assert_eq!(
        slot(offset_of!(ID3D12InfoQueue1Vtbl, register_message_callback)),
        38
    );
    assert_eq!(size_of::<ID3D12InfoQueue1Vtbl>(), 40 * PTR);
    assert_eq!(
        slot(offset_of!(
            ID3D12SDKConfiguration1Vtbl,
            create_device_factory
        )),
        4
    );
    assert_eq!(size_of::<ID3D12SDKConfiguration1Vtbl>(), 6 * PTR);
    assert_eq!(slot(offset_of!(ID3D12DeviceFactoryVtbl, set_flags)), 5);
    assert_eq!(
        slot(offset_of!(
            ID3D12DeviceFactoryVtbl,
            get_configuration_interface
        )),
        7
    );
    assert_eq!(slot(offset_of!(ID3D12DeviceFactoryVtbl, create_device)), 9);
    assert_eq!(size_of::<ID3D12DeviceFactoryVtbl>(), 10 * PTR);
    assert_eq!(slot(offset_of!(IDXGIFactory1Vtbl, enum_adapters)), 7);
    assert_eq!(
        slot(offset_of!(IDXGIFactory1Vtbl, make_window_association)),
        8
    );
    assert_eq!(slot(offset_of!(IDXGIFactory1Vtbl, enum_adapters1)), 12);
    assert_eq!(size_of::<IDXGIFactory1Vtbl>(), 14 * PTR);
    assert_eq!(
        slot(offset_of!(IDXGIFactory2Vtbl, factory1.enum_adapters1)),
        12
    );
    assert_eq!(
        slot(offset_of!(IDXGIFactory2Vtbl, create_swap_chain_for_hwnd)),
        15
    );
    assert_eq!(size_of::<IDXGIFactory2Vtbl>(), 25 * PTR);
    assert_eq!(size_of::<IDXGIFactory4Vtbl>(), 28 * PTR);
    assert_eq!(
        slot(offset_of!(IDXGIFactory5Vtbl, check_feature_support)),
        28
    );
    assert_eq!(size_of::<IDXGIFactory5Vtbl>(), 29 * PTR);
    assert_eq!(
        slot(offset_of!(
            IDXGIFactory6Vtbl,
            enum_adapter_by_gpu_preference
        )),
        29
    );
    assert_eq!(size_of::<IDXGIFactory6Vtbl>(), 30 * PTR);
    assert_eq!(slot(offset_of!(IDXGIAdapter1Vtbl, adapter.get_desc)), 8);
    assert_eq!(
        slot(offset_of!(
            IDXGIAdapter1Vtbl,
            adapter.check_interface_support
        )),
        9
    );
    assert_eq!(slot(offset_of!(IDXGIAdapter1Vtbl, get_desc1)), 10);
    assert_eq!(size_of::<IDXGIAdapter1Vtbl>(), 11 * PTR);
    assert_eq!(slot(offset_of!(IDXGIDebugVtbl, report_live_objects)), 3);
    assert_eq!(size_of::<IDXGIDebugVtbl>(), 4 * PTR);
}

#[test]
fn tables_match_upstream() {
    // Upstream's tables, printed by the harness, which includes them.
    // SwapchainCompositionToSDLTextureFormat
    assert_eq!(
        SWAPCHAIN_COMPOSITION_TO_SDL_TEXTURE_FORMAT.map(|f| f.0),
        [12, 53, 29, 8]
    );
    // SwapchainCompositionToTextureFormat
    assert_eq!(SWAPCHAIN_COMPOSITION_TO_TEXTURE_FORMAT, [87, 87, 10, 24]);
    // SwapchainCompositionToColorSpace
    assert_eq!(SWAPCHAIN_COMPOSITION_TO_COLOR_SPACE, [0, 0, 1, 12]);
    // SDLToD3D12_BlendFactor
    assert_eq!(
        SDL_TO_D3D12_BLEND_FACTOR,
        [1, 1, 2, 3, 4, 9, 10, 5, 6, 7, 8, 14, 15, 11]
    );
    // SDLToD3D12_BlendFactorAlpha
    assert_eq!(
        SDL_TO_D3D12_BLEND_FACTOR_ALPHA,
        [1, 1, 2, 5, 6, 7, 8, 5, 6, 7, 8, 14, 15, 11]
    );
    // SDLToD3D12_BlendOp
    assert_eq!(SDL_TO_D3D12_BLEND_OP, [1, 1, 2, 3, 4, 5]);
    // SDLToD3D12_TextureFormat
    assert_eq!(
        SDL_TO_D3D12_TEXTURE_FORMAT,
        [
            0, 65, 61, 49, 28, 56, 35, 11, 24, 85, 86, 115, 87, 71, 74, 77, 80, 83, 98, 96, 95, 63,
            51, 31, 58, 37, 13, 54, 34, 10, 41, 16, 2, 26, 62, 50, 30, 57, 36, 12, 42, 17, 3, 64,
            52, 32, 59, 38, 14, 43, 18, 4, 29, 91, 72, 75, 78, 99, 56, 46, 41, 46, 21, 0, 0, 0, 0,
            0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
            0, 0, 0, 0, 0, 0, 0, 0, 0
        ]
    );
    // SDLToD3D12_DepthFormat
    assert_eq!(
        SDL_TO_D3D12_DEPTH_FORMAT,
        [
            0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
            0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
            55, 45, 40, 45, 20, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
            0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0
        ]
    );
    // SDLToD3D12_TypelessFormat
    assert_eq!(
        SDL_TO_D3D12_TYPELESS_FORMAT,
        [
            0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
            0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
            53, 44, 39, 44, 19, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
            0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0
        ]
    );
    // SDLToD3D12_CompareOp
    assert_eq!(SDL_TO_D3D12_COMPARE_OP, [1, 1, 2, 3, 4, 5, 6, 7, 8]);
    // SDLToD3D12_StencilOp
    assert_eq!(SDL_TO_D3D12_STENCIL_OP, [1, 1, 2, 3, 4, 5, 6, 7, 8]);
    // SDLToD3D12_CullMode
    assert_eq!(SDL_TO_D3D12_CULL_MODE, [1, 2, 3]);
    // SDLToD3D12_FillMode
    assert_eq!(SDL_TO_D3D12_FILL_MODE, [3, 2]);
    // SDLToD3D12_InputRate
    assert_eq!(SDL_TO_D3D12_INPUT_RATE, [0, 1]);
    // SDLToD3D12_VertexFormat
    assert_eq!(
        SDL_TO_D3D12_VERTEX_FORMAT,
        [
            0, 43, 18, 8, 4, 42, 17, 7, 3, 41, 16, 6, 2, 52, 32, 50, 30, 51, 31, 49, 28, 38, 14,
            36, 12, 37, 13, 35, 11, 34, 10
        ]
    );
    // SDLToD3D12_SampleCount
    assert_eq!(SDL_TO_D3D12_SAMPLE_COUNT, [1, 2, 4, 8]);
    // SDLToD3D12_PrimitiveType
    assert_eq!(SDL_TO_D3D12_PRIMITIVE_TYPE, [4, 5, 2, 3, 1]);
    // SDLToD3D12_PrimitiveTopologyType
    assert_eq!(SDL_TO_D3D12_PRIMITIVE_TOPOLOGY_TYPE, [3, 3, 2, 2, 1]);
    // SDLToD3D12_SamplerAddressMode
    assert_eq!(SDL_TO_D3D12_SAMPLER_ADDRESS_MODE, [1, 2, 3]);
    // SDLToD3D12_Filter() of (min, mag, mipmap, compare, anisotropy) = the bits of i
    let expected = [
        0, 16, 4, 20, 1, 17, 5, 21, 128, 144, 132, 148, 129, 145, 133, 149, 64, 80, 68, 84, 65, 81,
        69, 85, 192, 208, 196, 212, 193, 209, 197, 213,
    ];
    for (i, &expected) in expected.iter().enumerate() {
        let linear = |bit: usize| {
            if i & bit != 0 {
                Filter::Linear
            } else {
                Filter::Nearest
            }
        };
        let mipmap = if i & 4 != 0 {
            SamplerMipmapMode::Linear
        } else {
            SamplerMipmapMode::Nearest
        };
        assert_eq!(
            sdl_to_d3d12_filter(linear(1), linear(2), mipmap, i & 8 != 0, i & 16 != 0),
            expected
        );
    }
}

// ---------------------------------------------------------------------------
// With a device
// ---------------------------------------------------------------------------

/// The offscreen video driver, for the front end's backend selection.
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

/// A debug-mode renderer made without the front end, destroyed when
/// dropped, and the shader formats it takes.
struct TestRenderer(D3D12Renderer, ShaderFormat);

impl std::ops::Deref for TestRenderer {
    type Target = D3D12Renderer;
    fn deref(&self) -> &D3D12Renderer {
        &self.0
    }
}

impl Drop for TestRenderer {
    fn drop(&mut self) {
        self.0.destroy();
    }
}

/// The creation properties of the tests: DXBC and DXIL shaders.
fn create_props() -> Properties {
    let props = Properties::new();
    props
        .set(PROP_GPU_DEVICE_CREATE_SHADERS_DXBC_BOOLEAN, true)
        .unwrap();
    props
        .set(PROP_GPU_DEVICE_CREATE_SHADERS_DXIL_BOOLEAN, true)
        .unwrap();
    props
        .set(PROP_GPU_DEVICE_CREATE_DEBUGMODE_BOOLEAN, true)
        .unwrap();
    props
}

/// What `f` returns and the GPU warnings it logs.
fn gpu_warnings<T>(f: impl FnOnce() -> T) -> (T, Vec<String>) {
    use crate::log::{self, Priority, Threshold};
    let warnings = Arc::new(std::sync::Mutex::new(Vec::new()));
    let sink = warnings.clone();
    log::set_priority(Category::Gpu, Threshold::AtLeast(Priority::Warn));
    log::set_output(move |record| {
        if record.category == Category::Gpu {
            sink.lock().unwrap().push(record.message.to_owned());
        }
        log::default_output(record);
    });
    let result = f();
    log::reset_output();
    log::reset_priorities();
    let warnings = warnings.lock().unwrap().clone();
    (result, warnings)
}

/// The renderer, or `None` (with the skip reported) without Direct3D 12.
fn renderer() -> Option<TestRenderer> {
    let props = create_props();
    let video = crate::video::core::driver().unwrap();
    let (prepared, warnings) = gpu_warnings(|| device::prepare_driver(&*video, &props));
    if !prepared {
        crate::test_support::skip(
            "d3d12",
            format_args!(
                "no Direct3D 12 device (PrepareDriver failed: {})",
                warnings.join("; ")
            ),
        );
        return None;
    }
    match device::create_device(true, false, &props) {
        Ok((renderer, formats)) => Some(TestRenderer(renderer, formats)),
        Err(e) => {
            crate::test_support::skip(
                "d3d12",
                format_args!("no Direct3D 12 device ({})", e.message()),
            );
            None
        }
    }
}

/// A device through the front end, or `None` (with the skip reported).
fn front_end_device() -> Option<Device> {
    match Device::new(
        ShaderFormat::DXBC | ShaderFormat::DXIL,
        true,
        Some("direct3d12"),
    ) {
        Ok(device) => Some(device),
        Err(e) => {
            crate::test_support::skip(
                "d3d12",
                format_args!("no Direct3D 12 device ({})", e.message()),
            );
            None
        }
    }
}

/// Whether the device takes DXIL, or `false` with the skip of the DXIL
/// checks reported.
fn has_dxil(formats: ShaderFormat) -> bool {
    if formats.contains(ShaderFormat::DXIL) {
        return true;
    }
    crate::test_support::skip(
        "d3d12",
        "the device has no shader model 6, so no DXIL (vkd3d under Wine has 5.1)",
    );
    false
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

/// The free descriptors of a staging pool and its heaps.
fn pool_counts(pool: &StagingDescriptorPool) -> (usize, usize) {
    let state = pool.state.lock().unwrap();
    (state.free_descriptors.len(), state.heaps.len())
}

fn blit_shader_info(code: &[u8], stage: ShaderStage, uniforms: u32) -> ShaderCreateInfo<'_> {
    ShaderCreateInfo {
        code,
        entrypoint: "main",
        format: ShaderFormat::DXBC,
        stage,
        num_samplers: uniforms,
        num_storage_textures: 0,
        num_storage_buffers: 0,
        num_uniform_buffers: uniforms,
        props: None,
    }
}

#[test]
fn device_formats_and_properties() {
    let _l = crate::test_support::test_lock();
    let _v = Video::init();
    let Some(device) = front_end_device() else {
        return;
    };
    assert_eq!(device.driver(), "direct3d12");
    assert!(device.shader_formats().contains(ShaderFormat::DXBC));
    assert!(crate::gpu::supports_shader_formats(
        ShaderFormat::DXBC,
        Some("direct3d12")
    ));
    // (Direct3D 12 is tried first: it is picked for DXBC.)
    let other = Device::new(ShaderFormat::DXBC, false, None).unwrap();
    assert_eq!(other.driver(), "direct3d12");
    drop(other);
    // (and never for SPIR-V)
    assert!(!crate::gpu::supports_shader_formats(
        ShaderFormat::SPIRV,
        Some("direct3d12")
    ));

    let props = device.properties();
    let name = props.get_string(PROP_GPU_DEVICE_NAME_STRING).unwrap();
    let version = props
        .get_string(PROP_GPU_DEVICE_DRIVER_VERSION_STRING)
        .unwrap();
    assert!(!name.is_empty());
    assert_eq!(version.split('.').count(), 4, "{version}");
    eprintln!(
        "note: Direct3D 12 adapter {name}, driver {version}, shader formats {:?}",
        device.shader_formats()
    );

    // The formats upstream's documentation guarantees.
    use TextureFormat as F;
    use TextureUsageFlags as U;
    let t2d = TextureType::Texture2D;
    for format in [F::R8G8B8A8_UNORM, F::B8G8R8A8_UNORM, F::R16G16B16A16_FLOAT] {
        assert!(device.texture_supports_format(format, t2d, U::SAMPLER | U::COLOR_TARGET));
    }
    assert!(device.texture_supports_format(
        F::D16_UNORM,
        t2d,
        U::SAMPLER | U::DEPTH_STENCIL_TARGET
    ));
    assert!(device.texture_supports_format(F::D32_FLOAT, t2d, U::DEPTH_STENCIL_TARGET));
    assert!(device.texture_supports_format(F::R32_FLOAT, t2d, U::COMPUTE_STORAGE_WRITE));
    assert!(device.texture_supports_format(F::R8G8B8A8_UNORM, TextureType::Cube, U::SAMPLER));
    assert!(device.texture_supports_format(F::R8G8B8A8_UNORM, TextureType::Texture3D, U::SAMPLER));
    // (no ASTC on Direct3D 12; sRGB isn't storable; a color format can't
    // be a depth target)
    assert!(!device.texture_supports_format(F::ASTC_4x4_UNORM, t2d, U::SAMPLER));
    assert!(!device.texture_supports_format(F::R8G8B8A8_UNORM_SRGB, t2d, U::COMPUTE_STORAGE_WRITE));
    assert!(!device.texture_supports_format(F::R8G8B8A8_UNORM, t2d, U::DEPTH_STENCIL_TARGET));
    // (upstream checks the color usages of a depth format with its view
    // format: D16_UNORM's R16_UNORM can be a color target)
    assert!(device.texture_supports_format(F::D16_UNORM, t2d, U::COLOR_TARGET));

    assert!(device.texture_supports_sample_count(F::R8G8B8A8_UNORM, SampleCount::One));
    assert!(device.texture_supports_sample_count(F::R8G8B8A8_UNORM, SampleCount::Four));
    assert!(device.texture_supports_sample_count(F::D32_FLOAT, SampleCount::Four));

    // A command buffer is acquired, and cancelled.
    device.acquire_command_buffer().unwrap().cancel().unwrap();
    device.wait_for_idle().unwrap();
}

#[test]
fn create_and_release_every_resource_kind() {
    let _l = crate::test_support::test_lock();
    let _v = Video::init();
    let Some(device) = front_end_device() else {
        return;
    };
    use TextureFormat as F;
    use TextureUsageFlags as U;

    // Buffers of every usage, named.
    let mut buffers = Vec::new();
    for usage in [
        BufferUsageFlags::VERTEX,
        BufferUsageFlags::INDEX,
        BufferUsageFlags::INDIRECT,
        BufferUsageFlags::GRAPHICS_STORAGE_READ,
        BufferUsageFlags::COMPUTE_STORAGE_READ,
        BufferUsageFlags::COMPUTE_STORAGE_WRITE,
        BufferUsageFlags::COMPUTE_STORAGE_READ | BufferUsageFlags::COMPUTE_STORAGE_WRITE,
    ] {
        let buffer = device
            .create_buffer(&BufferCreateInfo {
                usage,
                size: 256,
                props: None,
            })
            .unwrap();
        buffer.set_name("a buffer");
        buffers.push(buffer);
    }

    // Transfer buffers: mapped (upload buffers persistently), written,
    // read back, and cycled.
    for usage in [TransferBufferUsage::Upload, TransferBufferUsage::Download] {
        let mut transfer_buffer = device
            .create_transfer_buffer(&TransferBufferCreateInfo {
                usage,
                size: 64,
                props: None,
            })
            .unwrap();
        assert_eq!(transfer_buffer.size(), 64);
        {
            let mut mapped = transfer_buffer.map(false).unwrap();
            assert_eq!(mapped.len(), 64);
            mapped[..4].copy_from_slice(&[1, 2, 3, 4]);
        }
        {
            let mapped = transfer_buffer.map(true).unwrap();
            assert_eq!(&mapped[..4], &[1, 2, 3, 4]);
        }
    }

    // Textures of every type, as targets, sampled and stored.
    let mut textures = Vec::new();
    for (texture_type, layers, levels) in [
        (TextureType::Texture2D, 1, 4),
        (TextureType::Texture2DArray, 3, 2),
        (TextureType::Texture3D, 4, 2),
        (TextureType::Cube, 6, 1),
        (TextureType::CubeArray, 12, 1),
    ] {
        for usage in [
            U::SAMPLER,
            U::SAMPLER | U::COLOR_TARGET,
            U::COMPUTE_STORAGE_WRITE | U::COMPUTE_STORAGE_READ,
            U::GRAPHICS_STORAGE_READ | U::COMPUTE_STORAGE_SIMULTANEOUS_READ_WRITE,
        ] {
            let format = if usage.intersects(U::COMPUTE_STORAGE_WRITE) {
                F::R32_FLOAT
            } else {
                F::R8G8B8A8_UNORM
            };
            if !device.texture_supports_format(format, texture_type, usage) {
                continue;
            }
            let texture = device
                .create_texture(&texture_info(texture_type, format, usage, layers, levels))
                .unwrap_or_else(|e| panic!("{texture_type:?} {usage:?}: {}", e.message()));
            texture.set_name("a texture");
            textures.push(texture);
        }
    }
    // Depth-stencil targets, sampled or not, with the clear value
    // properties.
    for format in [
        F::D16_UNORM,
        F::D24_UNORM,
        F::D32_FLOAT,
        F::D24_UNORM_S8_UINT,
        F::D32_FLOAT_S8_UINT,
    ] {
        for usage in [
            U::DEPTH_STENCIL_TARGET,
            U::DEPTH_STENCIL_TARGET | U::SAMPLER,
        ] {
            if !device.texture_supports_format(format, TextureType::Texture2D, usage) {
                eprintln!("note: {format:?} {usage:?} unsupported");
                continue;
            }
            let props = Properties::new();
            props
                .set(
                    crate::gpu::PROP_GPU_TEXTURE_CREATE_D3D12_CLEAR_DEPTH_FLOAT,
                    1.0f32,
                )
                .unwrap();
            props
                .set(PROP_GPU_TEXTURE_CREATE_NAME_STRING, "depth")
                .unwrap();
            let mut info = texture_info(TextureType::Texture2D, format, usage, 1, 1);
            info.props = Some(props);
            textures.push(device.create_texture(&info).unwrap());
            let mut info = texture_info(TextureType::Texture2DArray, format, usage, 2, 1);
            info.props = None;
            textures.push(device.create_texture(&info).unwrap());
        }
    }
    // A multisampled color target, with a clear color.
    let props = Properties::new();
    props
        .set(PROP_GPU_TEXTURE_CREATE_D3D12_CLEAR_R_FLOAT, 0.5f32)
        .unwrap();
    let mut info = texture_info(
        TextureType::Texture2D,
        F::R8G8B8A8_UNORM,
        U::COLOR_TARGET,
        1,
        1,
    );
    info.sample_count = SampleCount::Four;
    info.props = Some(props);
    textures.push(device.create_texture(&info).unwrap());

    // Samplers of every filter.
    let mut samplers = Vec::new();
    for (filter, mipmap_mode, enable_anisotropy, enable_compare) in [
        (Filter::Nearest, SamplerMipmapMode::Nearest, false, false),
        (Filter::Linear, SamplerMipmapMode::Linear, false, false),
        (Filter::Linear, SamplerMipmapMode::Linear, true, false),
        (Filter::Nearest, SamplerMipmapMode::Linear, false, true),
    ] {
        samplers.push(
            device
                .create_sampler(&SamplerCreateInfo {
                    min_filter: filter,
                    mag_filter: filter,
                    mipmap_mode,
                    address_mode_u: SamplerAddressMode::Repeat,
                    address_mode_v: SamplerAddressMode::MirroredRepeat,
                    address_mode_w: SamplerAddressMode::ClampToEdge,
                    max_anisotropy: 4.0,
                    enable_anisotropy,
                    enable_compare,
                    compare_op: CompareOp::Less,
                    max_lod: 1000.0,
                    ..Default::default()
                })
                .unwrap(),
        );
    }

    // A graphics pipeline from DXBC: the blit shaders.
    let vertex = device
        .create_shader(&blit_shader_info(
            &D3D12_FULLSCREEN_VERT,
            ShaderStage::Vertex,
            0,
        ))
        .unwrap();
    let fragment = device
        .create_shader(&blit_shader_info(
            &D3D12_BLIT_FROM_2D,
            ShaderStage::Fragment,
            1,
        ))
        .unwrap();
    let target = ColorTargetDescription {
        format: F::R8G8B8A8_UNORM,
        blend_state: Default::default(),
    };
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
                color_target_descriptions: std::slice::from_ref(&target),
                depth_stencil_format: F::D16_UNORM,
                has_depth_stencil_target: false,
            },
            props: None,
        })
        .unwrap_or_else(|e| panic!("{}", e.message()));
    drop(pipeline);

    // Pipelines from DXIL.
    if has_dxil(device.shader_formats()) {
        dxil_pipelines(&device);
    }

    drop((buffers, textures, samplers, vertex, fragment));
    device.wait_for_idle().unwrap();
}

/// A graphics pipeline with vertex input, blending, depth and
/// multisampling, and a compute pipeline, from the DXIL test shaders.
fn dxil_pipelines(device: &Device) {
    use TextureFormat as F;
    let shader = |code: &'static [u8], stage, samplers, uniforms| {
        device
            .create_shader(&ShaderCreateInfo {
                code,
                entrypoint: "main",
                format: ShaderFormat::DXIL,
                stage,
                num_samplers: samplers,
                num_storage_textures: 0,
                num_storage_buffers: 0,
                num_uniform_buffers: uniforms,
                props: None,
            })
            .unwrap()
    };
    let vertex = shader(&test_dxil::VERTEX, ShaderStage::Vertex, 0, 1);
    let fragment = shader(&test_dxil::FRAGMENT_SAMPLER, ShaderStage::Fragment, 1, 1);
    let solid = shader(&test_dxil::FRAGMENT_SOLID, ShaderStage::Fragment, 0, 0);

    let descriptions = [VertexBufferDescription {
        slot: 0,
        pitch: 24,
        input_rate: VertexInputRate::Vertex,
        instance_step_rate: 0,
    }];
    let attributes = [
        VertexAttribute {
            location: 0,
            buffer_slot: 0,
            format: VertexElementFormat::Float2,
            offset: 0,
        },
        VertexAttribute {
            location: 1,
            buffer_slot: 0,
            format: VertexElementFormat::Float4,
            offset: 8,
        },
    ];
    let target = ColorTargetDescription {
        format: F::B8G8R8A8_UNORM,
        blend_state: ColorTargetBlendState {
            src_color_blendfactor: BlendFactor::SrcAlpha,
            dst_color_blendfactor: BlendFactor::OneMinusSrcAlpha,
            color_blend_op: BlendOp::Add,
            src_alpha_blendfactor: BlendFactor::One,
            dst_alpha_blendfactor: BlendFactor::Zero,
            alpha_blend_op: BlendOp::Add,
            enable_blend: true,
            ..Default::default()
        },
    };
    let props = Properties::new();
    props
        .set(
            crate::gpu::PROP_GPU_GRAPHICSPIPELINE_CREATE_NAME_STRING,
            "dxil",
        )
        .unwrap();
    for (fragment, sample_count, depth) in [
        (&fragment, SampleCount::One, true),
        (&solid, SampleCount::Four, false),
    ] {
        let pipeline = device
            .create_graphics_pipeline(&GraphicsPipelineCreateInfo {
                vertex_shader: &vertex,
                fragment_shader: fragment,
                vertex_input_state: VertexInputState {
                    vertex_buffer_descriptions: &descriptions,
                    vertex_attributes: &attributes,
                },
                primitive_type: PrimitiveType::TriangleStrip,
                rasterizer_state: RasterizerState {
                    cull_mode: CullMode::Back,
                    ..Default::default()
                },
                multisample_state: MultisampleState {
                    sample_count,
                    ..Default::default()
                },
                depth_stencil_state: DepthStencilState {
                    compare_op: CompareOp::Less,
                    enable_depth_test: depth,
                    enable_depth_write: depth,
                    ..Default::default()
                },
                target_info: GraphicsPipelineTargetInfo {
                    color_target_descriptions: std::slice::from_ref(&target),
                    depth_stencil_format: F::D32_FLOAT,
                    has_depth_stencil_target: depth,
                },
                props: Some(props.clone()),
            })
            .unwrap_or_else(|e| panic!("{}", e.message()));
        drop(pipeline);
    }

    let compute_props = Properties::new();
    compute_props
        .set(
            crate::gpu::PROP_GPU_COMPUTEPIPELINE_CREATE_NAME_STRING,
            "compute",
        )
        .unwrap();
    let compute = device
        .create_compute_pipeline(&ComputePipelineCreateInfo {
            code: &test_dxil::COMPUTE,
            entrypoint: "main",
            format: ShaderFormat::DXIL,
            num_samplers: 0,
            num_readonly_storage_textures: 0,
            num_readonly_storage_buffers: 0,
            num_readwrite_storage_textures: 0,
            num_readwrite_storage_buffers: 1,
            num_uniform_buffers: 1,
            threadcount_x: 64,
            threadcount_y: 1,
            threadcount_z: 1,
            props: Some(compute_props),
        })
        .unwrap_or_else(|e| panic!("{}", e.message()));
    drop(compute);
}

#[test]
fn renderer_resources_and_pools() {
    let _l = crate::test_support::test_lock();
    let _v = Video::init();
    let Some(renderer) = renderer() else {
        return;
    };
    assert!(renderer.1.contains(ShaderFormat::DXBC));
    let pools = &renderer.staging_descriptor_pools;
    // (the blit samplers hold two sampler descriptors)
    let all_free = [(1024, 1), (1022, 1), (1024, 1), (1024, 1)];
    let counts = || pools.iter().map(|p| pool_counts(p)).collect::<Vec<_>>();
    assert_eq!(counts(), all_free);

    // A texture's views: one SRV, an RTV per level, layer and depth slice
    // (a 3D texture's second level is half as deep).
    let texture = renderer
        .create_texture_container(&texture_info(
            TextureType::Texture3D,
            TextureFormat::R8G8B8A8_UNORM,
            TextureUsageFlags::SAMPLER | TextureUsageFlags::COLOR_TARGET,
            4,
            2,
        ))
        .unwrap();
    {
        let state = texture.state.lock().unwrap();
        let active = &state.active_texture;
        assert_eq!(active.subresources.len(), 2);
        assert!(active.srv_handle.is_some());
        for (i, subresource) in active.subresources.iter().enumerate() {
            assert_eq!(subresource.index as usize, i);
            assert_eq!((subresource.level, subresource.layer), (i as u32, 0));
            assert_eq!(subresource.depth, 4 >> i);
            assert_eq!(subresource.rtv_handles.len(), 4 >> i);
            assert!(subresource.uav_handle.is_none() && subresource.dsv_handle.is_none());
        }
        let container = active.container.lock().unwrap();
        let container = container.as_ref().unwrap();
        assert!(Arc::ptr_eq(
            &container.container.upgrade().unwrap(),
            &texture
        ));
    }
    assert_eq!(
        pool_counts(&pools[D3D12_DESCRIPTOR_HEAP_TYPE_RTV as usize]).0,
        1024 - 6
    );
    assert_eq!(pool_counts(&pools[0]).0, 1024 - 1);

    // Cycling: a texture in use is replaced by a new one, which an unused
    // one is then picked over.
    {
        let first = texture.state.lock().unwrap().active_texture.clone();
        first.reference_count.store(1, Ordering::SeqCst);
        renderer.cycle_active_texture(&texture);
        let second = texture.state.lock().unwrap().active_texture.clone();
        assert!(!Arc::ptr_eq(&first, &second));
        assert_eq!(texture.state.lock().unwrap().textures.len(), 2);
        assert_eq!(second.container.lock().unwrap().as_ref().unwrap().index, 1);
        first.reference_count.store(0, Ordering::SeqCst);
        second.reference_count.store(1, Ordering::SeqCst);
        renderer.cycle_active_texture(&texture);
        assert!(Arc::ptr_eq(
            &texture.state.lock().unwrap().active_texture,
            &first
        ));
        second.reference_count.store(0, Ordering::SeqCst);
    }
    assert_eq!(
        pool_counts(&pools[D3D12_DESCRIPTOR_HEAP_TYPE_RTV as usize]).0,
        1024 - 12
    );

    // Releasing defers the destruction to the next wait; the views go back
    // to their pools then.
    renderer.release_texture_container(&texture);
    drop(texture);
    assert_eq!(
        renderer.dispose.lock().unwrap().textures_to_destroy.len(),
        2
    );
    renderer.wait_internal().unwrap();
    assert!(renderer
        .dispose
        .lock()
        .unwrap()
        .textures_to_destroy
        .is_empty());
    assert_eq!(counts(), all_free);

    // Buffers: views, mapping and cycling.
    let buffer = renderer
        .create_buffer_container(
            BufferUsageFlags::COMPUTE_STORAGE_WRITE | BufferUsageFlags::GRAPHICS_STORAGE_READ,
            1024,
            D3D12BufferType::Gpu,
            Some("storage"),
        )
        .unwrap();
    {
        let state = buffer.state.lock().unwrap();
        let active = &state.active_buffer;
        assert!(active.uav_descriptor.is_some() && active.srv_descriptor.is_some());
        assert_ne!(active.virtual_address, 0);
        assert!(active.map_pointer.load(Ordering::SeqCst).is_null());
        assert!(!active.transitioned.load(Ordering::SeqCst));
        assert_eq!(state.debug_name.as_deref(), Some("storage"));
    }
    assert_eq!(pool_counts(&pools[0]).0, 1024 - 2);
    renderer.release_buffer_container(&buffer);
    drop(buffer);
    renderer.wait_internal().unwrap();
    assert_eq!(pool_counts(&pools[0]).0, 1024);

    let upload = renderer
        .create_buffer_container(
            BufferUsageFlags::default(),
            128,
            D3D12BufferType::Upload,
            None,
        )
        .unwrap();
    {
        let state = upload.state.lock().unwrap();
        // (upload buffers are persistently mapped, and in the generic read
        // state)
        assert!(!state
            .active_buffer
            .map_pointer
            .load(Ordering::SeqCst)
            .is_null());
        assert!(state.active_buffer.transitioned.load(Ordering::SeqCst));
        assert_eq!(state.active_buffer.virtual_address, 0);
    }
    let first = renderer
        .map_transfer_buffer_internal(&upload, true)
        .unwrap();
    // (not in use: no cycling)
    assert_eq!(
        renderer
            .map_transfer_buffer_internal(&upload, true)
            .unwrap(),
        first
    );
    upload
        .state
        .lock()
        .unwrap()
        .active_buffer
        .reference_count
        .store(1, Ordering::SeqCst);
    assert_eq!(
        renderer
            .map_transfer_buffer_internal(&upload, false)
            .unwrap(),
        first
    );
    let second = renderer
        .map_transfer_buffer_internal(&upload, true)
        .unwrap();
    assert_ne!(second, first);
    assert_eq!(upload.state.lock().unwrap().buffers.len(), 2);
    for buffer in &upload.state.lock().unwrap().buffers {
        buffer.reference_count.store(0, Ordering::SeqCst);
    }
    renderer.release_buffer_container(&upload);
    drop(upload);
    assert_eq!(renderer.dispose.lock().unwrap().buffers_to_destroy.len(), 2);
    renderer.wait_internal().unwrap();
    assert!(renderer
        .dispose
        .lock()
        .unwrap()
        .buffers_to_destroy
        .is_empty());

    // An unused released object waits for the GPU to be done with it.
    let sampler = renderer
        .create_sampler_internal(&SamplerCreateInfo::default())
        .unwrap();
    sampler.reference_count.store(1, Ordering::SeqCst);
    renderer.release_sampler_internal(sampler.clone());
    renderer.perform_pending_destroys();
    assert_eq!(
        renderer.dispose.lock().unwrap().samplers_to_destroy.len(),
        1
    );
    sampler.reference_count.store(0, Ordering::SeqCst);
    drop(sampler);
    renderer.perform_pending_destroys();
    assert!(renderer
        .dispose
        .lock()
        .unwrap()
        .samplers_to_destroy
        .is_empty());
    assert_eq!(counts(), all_free);

    // Uniform buffers: made mapped, then pooled.
    let uniform = renderer.acquire_uniform_buffer_from_pool().unwrap();
    assert_eq!((uniform.write_offset, uniform.draw_offset), (0, 0));
    assert!(!uniform.buffer.map_pointer.load(Ordering::SeqCst).is_null());
    assert_ne!(uniform.buffer.virtual_address, 0);
    let buffer = uniform.buffer.clone();
    D3D12Renderer::return_uniform_buffer_to_pool(
        &mut renderer.uniform_buffer_pool.lock().unwrap(),
        uniform,
    );
    let mut uniform = renderer.acquire_uniform_buffer_from_pool().unwrap();
    assert!(Arc::ptr_eq(&uniform.buffer, &buffer));
    uniform.write_offset = 256;
    D3D12Renderer::return_uniform_buffer_to_pool(
        &mut renderer.uniform_buffer_pool.lock().unwrap(),
        uniform,
    );
    assert_eq!(renderer.uniform_buffer_pool.lock().unwrap().len(), 1);
    drop(buffer);

    // Shader-visible heaps: four of each type, then new ones.
    for (heap_type, count) in [
        (D3D12_DESCRIPTOR_HEAP_TYPE_CBV_SRV_UAV, 65536),
        (D3D12_DESCRIPTOR_HEAP_TYPE_SAMPLER, 2048),
    ] {
        let mut heaps: Vec<_> = (0..5)
            .map(|_| {
                renderer
                    .acquire_gpu_descriptor_heap_from_pool(heap_type)
                    .unwrap()
            })
            .collect();
        for heap in &heaps {
            assert_eq!(heap.heap_type, heap_type);
            assert_eq!(heap.max_descriptors, count);
            assert!(!heap.staging);
            assert_ne!(heap.descriptor_heap_gpu_start.ptr, 0);
        }
        heaps[0].current_descriptor_index = 7;
        for heap in heaps {
            renderer.return_gpu_descriptor_heap_to_pool(heap);
        }
        let pool = renderer.gpu_descriptor_heap_pools[heap_type as usize]
            .heaps
            .lock()
            .unwrap();
        assert_eq!(pool.len(), 5);
        assert!(pool.iter().all(|h| h.current_descriptor_index == 0));
    }

    renderer.drain_info_queue_messages();
}

#[test]
fn staging_descriptor_pools_grow() {
    let _l = crate::test_support::test_lock();
    let _v = Video::init();
    let Some(renderer) = renderer() else {
        return;
    };
    let pool = &renderer.staging_descriptor_pools[D3D12_DESCRIPTOR_HEAP_TYPE_RTV as usize];
    // (more descriptors than a heap holds)
    let descriptors: Vec<_> = (0..1100)
        .map(|_| {
            renderer
                .assign_staging_descriptor_handle(D3D12_DESCRIPTOR_HEAP_TYPE_RTV)
                .unwrap()
        })
        .collect();
    assert_eq!(pool_counts(pool), (2048 - 1100, 2));
    // The first heap's slots come from its end.
    let state = pool.state.lock().unwrap();
    let first = &state.heaps[0];
    assert_eq!(
        descriptors[0].cpu_handle.ptr,
        first.descriptor_heap_cpu_start.ptr + 1023 * first.descriptor_size as usize
    );
    assert_eq!(
        descriptors[1023].cpu_handle.ptr,
        first.descriptor_heap_cpu_start.ptr
    );
    let second = &state.heaps[1];
    assert_eq!(
        descriptors[1024].cpu_handle.ptr,
        second.descriptor_heap_cpu_start.ptr + 1023 * second.descriptor_size as usize
    );
    drop(state);
    drop(descriptors);
    assert_eq!(pool_counts(pool), (2048, 2));
}

#[test]
fn shader_and_pipeline_errors() {
    let _l = crate::test_support::test_lock();
    let _v = Video::init();
    let Some(renderer) = renderer() else {
        return;
    };
    let mut info = blit_shader_info(b"SPIRV....", ShaderStage::Vertex, 0);
    info.format = ShaderFormat::DXIL;
    assert_eq!(
        renderer
            .create_shader_internal(&info)
            .unwrap_err()
            .message(),
        "The provided shader code is not valid DXIL!"
    );
    info.format = ShaderFormat::DXBC;
    assert_eq!(
        renderer
            .create_shader_internal(&info)
            .unwrap_err()
            .message(),
        "The provided shader code is not valid DXBC!"
    );
    let shader = renderer
        .create_shader_internal(&blit_shader_info(
            &D3D12_BLIT_FROM_3D,
            ShaderStage::Fragment,
            1,
        ))
        .unwrap();
    assert_eq!(shader.bytecode, D3D12_BLIT_FROM_3D);
    assert_eq!(
        (
            shader.stage,
            shader.num_samplers,
            shader.num_uniform_buffers
        ),
        (ShaderStage::Fragment, 1, 1)
    );

    let compute = ComputePipelineCreateInfo {
        code: b"DXB",
        entrypoint: "main",
        format: ShaderFormat::DXIL,
        num_samplers: 0,
        num_readonly_storage_textures: 0,
        num_readonly_storage_buffers: 0,
        num_readwrite_storage_textures: 0,
        num_readwrite_storage_buffers: 1,
        num_uniform_buffers: 0,
        threadcount_x: 1,
        threadcount_y: 1,
        threadcount_z: 1,
        props: None,
    };
    assert_eq!(
        renderer
            .create_compute_pipeline_internal(&compute)
            .unwrap_err()
            .message(),
        "The provided shader code is not valid DXIL!"
    );
    // (valid DXBC that isn't a compute shader: the device refuses it)
    let compute = ComputePipelineCreateInfo {
        code: &D3D12_BLIT_FROM_2D,
        format: ShaderFormat::DXBC,
        ..compute
    };
    let e = renderer
        .create_compute_pipeline_internal(&compute)
        .unwrap_err();
    assert!(
        e.message()
            .starts_with("Could not create compute pipeline state! Error Code: "),
        "{}",
        e.message()
    );

    // The root signatures of the blit pipeline: the fragment sampler and
    // texture tables in space 2, its uniform buffer in space 3.
    let vertex = D3D12Shader {
        bytecode: D3D12_FULLSCREEN_VERT.to_vec(),
        stage: ShaderStage::Vertex,
        num_samplers: 0,
        num_uniform_buffers: 0,
        num_storage_buffers: 0,
        num_storage_textures: 0,
    };
    let root_signature = renderer
        .create_graphics_root_signature(&vertex, &shader)
        .unwrap();
    assert_eq!(
        (
            root_signature.vertex_sampler_root_index,
            root_signature.fragment_sampler_root_index,
            root_signature.fragment_sampler_texture_root_index,
            root_signature.fragment_storage_texture_root_index,
            root_signature.fragment_uniform_buffer_root_index,
        ),
        (-1, 0, 1, -1, [2, -1, -1, -1])
    );
    let full = D3D12Shader {
        num_samplers: 2,
        num_uniform_buffers: 4,
        num_storage_buffers: 3,
        num_storage_textures: 1,
        ..vertex
    };
    let root_signature = renderer
        .create_graphics_root_signature(&full, &full)
        .unwrap();
    assert_eq!(
        (
            root_signature.vertex_sampler_root_index,
            root_signature.vertex_sampler_texture_root_index,
            root_signature.vertex_storage_texture_root_index,
            root_signature.vertex_storage_buffer_root_index,
            root_signature.vertex_uniform_buffer_root_index,
            root_signature.fragment_sampler_root_index,
            root_signature.fragment_uniform_buffer_root_index,
        ),
        (0, 1, 2, 3, [4, 5, 6, 7], 8, [12, 13, 14, 15])
    );
    let root_signature = renderer
        .create_compute_root_signature(&ComputePipelineCreateInfo {
            num_samplers: 1,
            num_readonly_storage_textures: 2,
            num_readonly_storage_buffers: 3,
            num_readwrite_storage_textures: 1,
            num_readwrite_storage_buffers: 2,
            num_uniform_buffers: 2,
            ..compute
        })
        .unwrap();
    assert_eq!(
        (
            root_signature.sampler_root_index,
            root_signature.sampler_texture_root_index,
            root_signature.read_only_storage_texture_root_index,
            root_signature.read_only_storage_buffer_root_index,
            root_signature.read_write_storage_texture_root_index,
            root_signature.read_write_storage_buffer_root_index,
            root_signature.uniform_buffer_root_index,
        ),
        (0, 1, 2, 3, 4, 5, [6, 7, -1, -1])
    );
}

#[test]
fn blit_resources_and_debug_names() {
    let _l = crate::test_support::test_lock();
    let _v = Video::init();
    let Some(renderer) = renderer() else {
        return;
    };
    {
        let blit = renderer.blit.lock().unwrap();
        for shader in [
            &blit.vertex_shader,
            &blit.from_2d_shader,
            &blit.from_2d_array_shader,
            &blit.from_3d_shader,
            &blit.from_cube_shader,
            &blit.from_cube_array_shader,
        ] {
            let shader = shader
                .as_ref()
                .unwrap()
                .raw
                .downcast::<D3D12Shader>()
                .unwrap();
            assert!(is_valid_shader_bytecode(&shader.bytecode));
        }
        assert!(blit.nearest_sampler.is_some() && blit.linear_sampler.is_some());
    }
    // (the blit samplers' descriptors)
    assert_eq!(
        pool_counts(
            &renderer.staging_descriptor_pools[D3D12_DESCRIPTOR_HEAP_TYPE_SAMPLER as usize]
        )
        .0,
        1022
    );

    let buffer = renderer
        .create_buffer_container(BufferUsageFlags::VERTEX, 16, D3D12BufferType::Gpu, None)
        .unwrap();
    renderer.set_buffer_name_internal(&buffer, "vertices");
    assert_eq!(
        buffer.state.lock().unwrap().debug_name.as_deref(),
        Some("vertices")
    );
    let texture = renderer
        .create_texture_container(&texture_info(
            TextureType::Texture2D,
            TextureFormat::R8G8B8A8_UNORM,
            TextureUsageFlags::SAMPLER,
            1,
            1,
        ))
        .unwrap();
    renderer.set_texture_name_internal(&texture, "texture");
    assert_eq!(
        texture.state.lock().unwrap().debug_name.as_deref(),
        Some("texture")
    );
    // (the copy of the creation properties)
    assert!(texture.info.props.is_some());
    renderer.release_buffer_container(&buffer);
    renderer.release_texture_container(&texture);
}
