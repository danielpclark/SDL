// Rust translation of the conversion tables of src/gpu/metal/SDL_gpu_metal.m
// from Simple DirectMedia Layer.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! The conversions from the GPU API's enums to Metal's (`SDLToMetal_*`),
//! and the swapchain composition tables.
//!
//! The tables are indexed by the GPU enums' C values, as upstream's are.
//! Upstream's `@available()` checks are made at run time; the ones that
//! only name iOS and tvOS are true on macOS.

use super::sys::*;
use crate::core::apple::foundation::{available, CFStringRef};
use crate::core::apple::objc::NSUInteger;
use crate::gpu::{
    BlendFactor, BlendOp, ColorComponentFlags, CompareOp, CullMode, FillMode, Filter, FrontFace,
    IndexElementSize, LoadOp, PrimitiveType, SampleCount, SamplerAddressMode, SamplerMipmapMode,
    StencilOp, StoreOp, SwapchainComposition, TextureFormat, TextureType, VertexElementFormat,
    VertexInputRate,
};

/// Translation of `SDLToMetal_TextureFormat()`.
pub(super) fn sdl_to_metal_texture_format(format: TextureFormat) -> NSUInteger {
    use TextureFormat as F;

    // RETURN_FORMAT(availability, format)
    let gated = |availability: bool, format: NSUInteger| {
        if availability {
            format
        } else {
            MTL_PIXEL_FORMAT_INVALID
        }
    };
    let macos11 = || available!(macos(11, 0));

    match format {
        F::INVALID => MTL_PIXEL_FORMAT_INVALID,
        F::A8_UNORM => MTL_PIXEL_FORMAT_A8_UNORM,
        F::R8_UNORM => MTL_PIXEL_FORMAT_R8_UNORM,
        F::R8G8_UNORM => MTL_PIXEL_FORMAT_RG8_UNORM,
        F::R8G8B8A8_UNORM => MTL_PIXEL_FORMAT_RGBA8_UNORM,
        F::R16_UNORM => MTL_PIXEL_FORMAT_R16_UNORM,
        F::R16G16_UNORM => MTL_PIXEL_FORMAT_RG16_UNORM,
        F::R16G16B16A16_UNORM => MTL_PIXEL_FORMAT_RGBA16_UNORM,
        F::R10G10B10A2_UNORM => MTL_PIXEL_FORMAT_RGB10A2_UNORM,
        F::B5G6R5_UNORM => gated(macos11(), MTL_PIXEL_FORMAT_B5G6R5_UNORM),
        F::B5G5R5A1_UNORM => gated(macos11(), MTL_PIXEL_FORMAT_BGR5A1_UNORM),
        F::B4G4R4A4_UNORM => gated(macos11(), MTL_PIXEL_FORMAT_ABGR4_UNORM),
        F::B8G8R8A8_UNORM => MTL_PIXEL_FORMAT_BGRA8_UNORM,
        // (@available(iOS 16.4, tvOS 16.4, *): always on macOS)
        F::BC1_RGBA_UNORM => MTL_PIXEL_FORMAT_BC1_RGBA,
        F::BC2_RGBA_UNORM => MTL_PIXEL_FORMAT_BC2_RGBA,
        F::BC3_RGBA_UNORM => MTL_PIXEL_FORMAT_BC3_RGBA,
        F::BC4_R_UNORM => MTL_PIXEL_FORMAT_BC4_R_UNORM,
        F::BC5_RG_UNORM => MTL_PIXEL_FORMAT_BC5_RG_UNORM,
        F::BC7_RGBA_UNORM => MTL_PIXEL_FORMAT_BC7_RGBA_UNORM,
        F::BC6H_RGB_FLOAT => MTL_PIXEL_FORMAT_BC6H_RGB_FLOAT,
        F::BC6H_RGB_UFLOAT => MTL_PIXEL_FORMAT_BC6H_RGB_UFLOAT,
        F::R8_SNORM => MTL_PIXEL_FORMAT_R8_SNORM,
        F::R8G8_SNORM => MTL_PIXEL_FORMAT_RG8_SNORM,
        F::R8G8B8A8_SNORM => MTL_PIXEL_FORMAT_RGBA8_SNORM,
        F::R16_SNORM => MTL_PIXEL_FORMAT_R16_SNORM,
        F::R16G16_SNORM => MTL_PIXEL_FORMAT_RG16_SNORM,
        F::R16G16B16A16_SNORM => MTL_PIXEL_FORMAT_RGBA16_SNORM,
        F::R16_FLOAT => MTL_PIXEL_FORMAT_R16_FLOAT,
        F::R16G16_FLOAT => MTL_PIXEL_FORMAT_RG16_FLOAT,
        F::R16G16B16A16_FLOAT => MTL_PIXEL_FORMAT_RGBA16_FLOAT,
        F::R32_FLOAT => MTL_PIXEL_FORMAT_R32_FLOAT,
        F::R32G32_FLOAT => MTL_PIXEL_FORMAT_RG32_FLOAT,
        F::R32G32B32A32_FLOAT => MTL_PIXEL_FORMAT_RGBA32_FLOAT,
        F::R11G11B10_UFLOAT => MTL_PIXEL_FORMAT_RG11B10_FLOAT,
        F::R8_UINT => MTL_PIXEL_FORMAT_R8_UINT,
        F::R8G8_UINT => MTL_PIXEL_FORMAT_RG8_UINT,
        F::R8G8B8A8_UINT => MTL_PIXEL_FORMAT_RGBA8_UINT,
        F::R16_UINT => MTL_PIXEL_FORMAT_R16_UINT,
        F::R16G16_UINT => MTL_PIXEL_FORMAT_RG16_UINT,
        F::R16G16B16A16_UINT => MTL_PIXEL_FORMAT_RGBA16_UINT,
        F::R32_UINT => MTL_PIXEL_FORMAT_R32_UINT,
        F::R32G32_UINT => MTL_PIXEL_FORMAT_RG32_UINT,
        F::R32G32B32A32_UINT => MTL_PIXEL_FORMAT_RGBA32_UINT,
        F::R8_INT => MTL_PIXEL_FORMAT_R8_SINT,
        F::R8G8_INT => MTL_PIXEL_FORMAT_RG8_SINT,
        F::R8G8B8A8_INT => MTL_PIXEL_FORMAT_RGBA8_SINT,
        F::R16_INT => MTL_PIXEL_FORMAT_R16_SINT,
        F::R16G16_INT => MTL_PIXEL_FORMAT_RG16_SINT,
        F::R16G16B16A16_INT => MTL_PIXEL_FORMAT_RGBA16_SINT,
        F::R32_INT => MTL_PIXEL_FORMAT_R32_SINT,
        F::R32G32_INT => MTL_PIXEL_FORMAT_RG32_SINT,
        F::R32G32B32A32_INT => MTL_PIXEL_FORMAT_RGBA32_SINT,
        F::R8G8B8A8_UNORM_SRGB => MTL_PIXEL_FORMAT_RGBA8_UNORM_SRGB,
        F::B8G8R8A8_UNORM_SRGB => MTL_PIXEL_FORMAT_BGRA8_UNORM_SRGB,
        F::BC1_RGBA_UNORM_SRGB => MTL_PIXEL_FORMAT_BC1_RGBA_SRGB,
        F::BC2_RGBA_UNORM_SRGB => MTL_PIXEL_FORMAT_BC2_RGBA_SRGB,
        F::BC3_RGBA_UNORM_SRGB => MTL_PIXEL_FORMAT_BC3_RGBA_SRGB,
        F::BC7_RGBA_UNORM_SRGB => MTL_PIXEL_FORMAT_BC7_RGBA_UNORM_SRGB,
        // (@available(iOS 13.0, tvOS 13.0, *): always on macOS)
        F::D16_UNORM => MTL_PIXEL_FORMAT_DEPTH16_UNORM,
        // (SDL_PLATFORM_MACOS)
        F::D24_UNORM => MTL_PIXEL_FORMAT_DEPTH24_UNORM_STENCIL8,
        F::D32_FLOAT => MTL_PIXEL_FORMAT_DEPTH32_FLOAT,
        // (SDL_PLATFORM_MACOS)
        F::D24_UNORM_S8_UINT => MTL_PIXEL_FORMAT_DEPTH24_UNORM_STENCIL8,
        F::D32_FLOAT_S8_UINT => MTL_PIXEL_FORMAT_DEPTH32_FLOAT_STENCIL8,
        F::ASTC_4x4_UNORM => gated(macos11(), MTL_PIXEL_FORMAT_ASTC_4X4_LDR),
        F::ASTC_5x4_UNORM => gated(macos11(), MTL_PIXEL_FORMAT_ASTC_5X4_LDR),
        F::ASTC_5x5_UNORM => gated(macos11(), MTL_PIXEL_FORMAT_ASTC_5X5_LDR),
        F::ASTC_6x5_UNORM => gated(macos11(), MTL_PIXEL_FORMAT_ASTC_6X5_LDR),
        F::ASTC_6x6_UNORM => gated(macos11(), MTL_PIXEL_FORMAT_ASTC_6X6_LDR),
        F::ASTC_8x5_UNORM => gated(macos11(), MTL_PIXEL_FORMAT_ASTC_8X5_LDR),
        F::ASTC_8x6_UNORM => gated(macos11(), MTL_PIXEL_FORMAT_ASTC_8X6_LDR),
        F::ASTC_8x8_UNORM => gated(macos11(), MTL_PIXEL_FORMAT_ASTC_8X8_LDR),
        F::ASTC_10x5_UNORM => gated(macos11(), MTL_PIXEL_FORMAT_ASTC_10X5_LDR),
        F::ASTC_10x6_UNORM => gated(macos11(), MTL_PIXEL_FORMAT_ASTC_10X6_LDR),
        F::ASTC_10x8_UNORM => gated(macos11(), MTL_PIXEL_FORMAT_ASTC_10X8_LDR),
        F::ASTC_10x10_UNORM => gated(macos11(), MTL_PIXEL_FORMAT_ASTC_10X10_LDR),
        F::ASTC_12x10_UNORM => gated(macos11(), MTL_PIXEL_FORMAT_ASTC_12X10_LDR),
        F::ASTC_12x12_UNORM => gated(macos11(), MTL_PIXEL_FORMAT_ASTC_12X12_LDR),
        F::ASTC_4x4_UNORM_SRGB => gated(macos11(), MTL_PIXEL_FORMAT_ASTC_4X4_SRGB),
        F::ASTC_5x4_UNORM_SRGB => gated(macos11(), MTL_PIXEL_FORMAT_ASTC_5X4_SRGB),
        F::ASTC_5x5_UNORM_SRGB => gated(macos11(), MTL_PIXEL_FORMAT_ASTC_5X5_SRGB),
        F::ASTC_6x5_UNORM_SRGB => gated(macos11(), MTL_PIXEL_FORMAT_ASTC_6X5_SRGB),
        F::ASTC_6x6_UNORM_SRGB => gated(macos11(), MTL_PIXEL_FORMAT_ASTC_6X6_SRGB),
        F::ASTC_8x5_UNORM_SRGB => gated(macos11(), MTL_PIXEL_FORMAT_ASTC_8X5_SRGB),
        F::ASTC_8x6_UNORM_SRGB => gated(macos11(), MTL_PIXEL_FORMAT_ASTC_8X6_SRGB),
        F::ASTC_8x8_UNORM_SRGB => gated(macos11(), MTL_PIXEL_FORMAT_ASTC_8X8_SRGB),
        F::ASTC_10x5_UNORM_SRGB => gated(macos11(), MTL_PIXEL_FORMAT_ASTC_10X5_SRGB),
        F::ASTC_10x6_UNORM_SRGB => gated(macos11(), MTL_PIXEL_FORMAT_ASTC_10X6_SRGB),
        F::ASTC_10x8_UNORM_SRGB => gated(macos11(), MTL_PIXEL_FORMAT_ASTC_10X8_SRGB),
        F::ASTC_10x10_UNORM_SRGB => gated(macos11(), MTL_PIXEL_FORMAT_ASTC_10X10_SRGB),
        F::ASTC_12x10_UNORM_SRGB => gated(macos11(), MTL_PIXEL_FORMAT_ASTC_12X10_SRGB),
        F::ASTC_12x12_UNORM_SRGB => gated(macos11(), MTL_PIXEL_FORMAT_ASTC_12X12_SRGB),
        F::ASTC_4x4_FLOAT => gated(macos11(), MTL_PIXEL_FORMAT_ASTC_4X4_HDR),
        F::ASTC_5x4_FLOAT => gated(macos11(), MTL_PIXEL_FORMAT_ASTC_5X4_HDR),
        F::ASTC_5x5_FLOAT => gated(macos11(), MTL_PIXEL_FORMAT_ASTC_5X5_HDR),
        F::ASTC_6x5_FLOAT => gated(macos11(), MTL_PIXEL_FORMAT_ASTC_6X5_HDR),
        F::ASTC_6x6_FLOAT => gated(macos11(), MTL_PIXEL_FORMAT_ASTC_6X6_HDR),
        F::ASTC_8x5_FLOAT => gated(macos11(), MTL_PIXEL_FORMAT_ASTC_8X5_HDR),
        F::ASTC_8x6_FLOAT => gated(macos11(), MTL_PIXEL_FORMAT_ASTC_8X6_HDR),
        F::ASTC_8x8_FLOAT => gated(macos11(), MTL_PIXEL_FORMAT_ASTC_8X8_HDR),
        F::ASTC_10x5_FLOAT => gated(macos11(), MTL_PIXEL_FORMAT_ASTC_10X5_HDR),
        F::ASTC_10x6_FLOAT => gated(macos11(), MTL_PIXEL_FORMAT_ASTC_10X6_HDR),
        F::ASTC_10x8_FLOAT => gated(macos11(), MTL_PIXEL_FORMAT_ASTC_10X8_HDR),
        F::ASTC_10x10_FLOAT => gated(macos11(), MTL_PIXEL_FORMAT_ASTC_10X10_HDR),
        F::ASTC_12x10_FLOAT => gated(macos11(), MTL_PIXEL_FORMAT_ASTC_12X10_HDR),
        F::ASTC_12x12_FLOAT => gated(macos11(), MTL_PIXEL_FORMAT_ASTC_12X12_HDR),
        // Note (upstream): C falls off the end of the switch for values
        // outside the enum (undefined behavior); no format here.
        _ => MTL_PIXEL_FORMAT_INVALID,
    }
}

/// Translation of `SDLToMetal_VertexFormat`.
pub(super) static SDL_TO_METAL_VERTEX_FORMAT: [NSUInteger; 31] = [
    MTL_VERTEX_FORMAT_INVALID,            // INVALID
    MTL_VERTEX_FORMAT_INT,                // INT
    MTL_VERTEX_FORMAT_INT2,               // INT2
    MTL_VERTEX_FORMAT_INT3,               // INT3
    MTL_VERTEX_FORMAT_INT4,               // INT4
    MTL_VERTEX_FORMAT_UINT,               // UINT
    MTL_VERTEX_FORMAT_UINT2,              // UINT2
    MTL_VERTEX_FORMAT_UINT3,              // UINT3
    MTL_VERTEX_FORMAT_UINT4,              // UINT4
    MTL_VERTEX_FORMAT_FLOAT,              // FLOAT
    MTL_VERTEX_FORMAT_FLOAT2,             // FLOAT2
    MTL_VERTEX_FORMAT_FLOAT3,             // FLOAT3
    MTL_VERTEX_FORMAT_FLOAT4,             // FLOAT4
    MTL_VERTEX_FORMAT_CHAR2,              // BYTE2
    MTL_VERTEX_FORMAT_CHAR4,              // BYTE4
    MTL_VERTEX_FORMAT_UCHAR2,             // UBYTE2
    MTL_VERTEX_FORMAT_UCHAR4,             // UBYTE4
    MTL_VERTEX_FORMAT_CHAR2_NORMALIZED,   // BYTE2_NORM
    MTL_VERTEX_FORMAT_CHAR4_NORMALIZED,   // BYTE4_NORM
    MTL_VERTEX_FORMAT_UCHAR2_NORMALIZED,  // UBYTE2_NORM
    MTL_VERTEX_FORMAT_UCHAR4_NORMALIZED,  // UBYTE4_NORM
    MTL_VERTEX_FORMAT_SHORT2,             // SHORT2
    MTL_VERTEX_FORMAT_SHORT4,             // SHORT4
    MTL_VERTEX_FORMAT_USHORT2,            // USHORT2
    MTL_VERTEX_FORMAT_USHORT4,            // USHORT4
    MTL_VERTEX_FORMAT_SHORT2_NORMALIZED,  // SHORT2_NORM
    MTL_VERTEX_FORMAT_SHORT4_NORMALIZED,  // SHORT4_NORM
    MTL_VERTEX_FORMAT_USHORT2_NORMALIZED, // USHORT2_NORM
    MTL_VERTEX_FORMAT_USHORT4_NORMALIZED, // USHORT4_NORM
    MTL_VERTEX_FORMAT_HALF2,              // HALF2
    MTL_VERTEX_FORMAT_HALF4,              // HALF4
];

pub(super) fn sdl_to_metal_vertex_format(format: VertexElementFormat) -> NSUInteger {
    SDL_TO_METAL_VERTEX_FORMAT[format as usize]
}

/// Translation of `SDLToMetal_IndexType`.
pub(super) fn sdl_to_metal_index_type(size: IndexElementSize) -> NSUInteger {
    [
        MTL_INDEX_TYPE_UINT16, // 16BIT
        MTL_INDEX_TYPE_UINT32, // 32BIT
    ][size as usize]
}

/// Translation of `SDLToMetal_PrimitiveType`.
pub(super) fn sdl_to_metal_primitive_type(primitive_type: PrimitiveType) -> NSUInteger {
    [
        MTL_PRIMITIVE_TYPE_TRIANGLE,       // TRIANGLELIST
        MTL_PRIMITIVE_TYPE_TRIANGLE_STRIP, // TRIANGLESTRIP
        MTL_PRIMITIVE_TYPE_LINE,           // LINELIST
        MTL_PRIMITIVE_TYPE_LINE_STRIP,     // LINESTRIP
        MTL_PRIMITIVE_TYPE_POINT,          // POINTLIST
    ][primitive_type as usize]
}

/// Translation of `SDLToMetal_PolygonMode`.
pub(super) fn sdl_to_metal_polygon_mode(fill_mode: FillMode) -> NSUInteger {
    [
        MTL_TRIANGLE_FILL_MODE_FILL,  // FILL
        MTL_TRIANGLE_FILL_MODE_LINES, // LINE
    ][fill_mode as usize]
}

/// Translation of `SDLToMetal_CullMode`.
pub(super) fn sdl_to_metal_cull_mode(cull_mode: CullMode) -> NSUInteger {
    [
        MTL_CULL_MODE_NONE,  // NONE
        MTL_CULL_MODE_FRONT, // FRONT
        MTL_CULL_MODE_BACK,  // BACK
    ][cull_mode as usize]
}

/// Translation of `SDLToMetal_FrontFace`.
pub(super) fn sdl_to_metal_front_face(front_face: FrontFace) -> NSUInteger {
    [
        MTL_WINDING_COUNTER_CLOCKWISE, // COUNTER_CLOCKWISE
        MTL_WINDING_CLOCKWISE,         // CLOCKWISE
    ][front_face as usize]
}

/// Translation of `SDLToMetal_BlendFactor`.
pub(super) static SDL_TO_METAL_BLEND_FACTOR: [NSUInteger; 14] = [
    MTL_BLEND_FACTOR_ZERO,                        // INVALID
    MTL_BLEND_FACTOR_ZERO,                        // ZERO
    MTL_BLEND_FACTOR_ONE,                         // ONE
    MTL_BLEND_FACTOR_SOURCE_COLOR,                // SRC_COLOR
    MTL_BLEND_FACTOR_ONE_MINUS_SOURCE_COLOR,      // ONE_MINUS_SRC_COLOR
    MTL_BLEND_FACTOR_DESTINATION_COLOR,           // DST_COLOR
    MTL_BLEND_FACTOR_ONE_MINUS_DESTINATION_COLOR, // ONE_MINUS_DST_COLOR
    MTL_BLEND_FACTOR_SOURCE_ALPHA,                // SRC_ALPHA
    MTL_BLEND_FACTOR_ONE_MINUS_SOURCE_ALPHA,      // ONE_MINUS_SRC_ALPHA
    MTL_BLEND_FACTOR_DESTINATION_ALPHA,           // DST_ALPHA
    MTL_BLEND_FACTOR_ONE_MINUS_DESTINATION_ALPHA, // ONE_MINUS_DST_ALPHA
    MTL_BLEND_FACTOR_BLEND_COLOR,                 // CONSTANT_COLOR
    MTL_BLEND_FACTOR_ONE_MINUS_BLEND_COLOR,       // ONE_MINUS_CONSTANT_COLOR
    MTL_BLEND_FACTOR_SOURCE_ALPHA_SATURATED,      // SRC_ALPHA_SATURATE
];

pub(super) fn sdl_to_metal_blend_factor(factor: BlendFactor) -> NSUInteger {
    SDL_TO_METAL_BLEND_FACTOR[factor as usize]
}

/// Translation of `SDLToMetal_BlendOp`.
pub(super) static SDL_TO_METAL_BLEND_OP: [NSUInteger; 6] = [
    MTL_BLEND_OPERATION_ADD,              // INVALID
    MTL_BLEND_OPERATION_ADD,              // ADD
    MTL_BLEND_OPERATION_SUBTRACT,         // SUBTRACT
    MTL_BLEND_OPERATION_REVERSE_SUBTRACT, // REVERSE_SUBTRACT
    MTL_BLEND_OPERATION_MIN,              // MIN
    MTL_BLEND_OPERATION_MAX,              // MAX
];

pub(super) fn sdl_to_metal_blend_op(op: BlendOp) -> NSUInteger {
    SDL_TO_METAL_BLEND_OP[op as usize]
}

/// Translation of `SDLToMetal_CompareOp`.
pub(super) static SDL_TO_METAL_COMPARE_OP: [NSUInteger; 9] = [
    MTL_COMPARE_FUNCTION_NEVER,         // INVALID
    MTL_COMPARE_FUNCTION_NEVER,         // NEVER
    MTL_COMPARE_FUNCTION_LESS,          // LESS
    MTL_COMPARE_FUNCTION_EQUAL,         // EQUAL
    MTL_COMPARE_FUNCTION_LESS_EQUAL,    // LESS_OR_EQUAL
    MTL_COMPARE_FUNCTION_GREATER,       // GREATER
    MTL_COMPARE_FUNCTION_NOT_EQUAL,     // NOT_EQUAL
    MTL_COMPARE_FUNCTION_GREATER_EQUAL, // GREATER_OR_EQUAL
    MTL_COMPARE_FUNCTION_ALWAYS,        // ALWAYS
];

pub(super) fn sdl_to_metal_compare_op(op: CompareOp) -> NSUInteger {
    SDL_TO_METAL_COMPARE_OP[op as usize]
}

/// Translation of `SDLToMetal_StencilOp`.
pub(super) static SDL_TO_METAL_STENCIL_OP: [NSUInteger; 9] = [
    MTL_STENCIL_OPERATION_KEEP,            // INVALID
    MTL_STENCIL_OPERATION_KEEP,            // KEEP
    MTL_STENCIL_OPERATION_ZERO,            // ZERO
    MTL_STENCIL_OPERATION_REPLACE,         // REPLACE
    MTL_STENCIL_OPERATION_INCREMENT_CLAMP, // INCREMENT_AND_CLAMP
    MTL_STENCIL_OPERATION_DECREMENT_CLAMP, // DECREMENT_AND_CLAMP
    MTL_STENCIL_OPERATION_INVERT,          // INVERT
    MTL_STENCIL_OPERATION_INCREMENT_WRAP,  // INCREMENT_AND_WRAP
    MTL_STENCIL_OPERATION_DECREMENT_WRAP,  // DECREMENT_AND_WRAP
];

pub(super) fn sdl_to_metal_stencil_op(op: StencilOp) -> NSUInteger {
    SDL_TO_METAL_STENCIL_OP[op as usize]
}

/// Translation of `SDLToMetal_SamplerAddressMode`.
pub(super) fn sdl_to_metal_sampler_address_mode(mode: SamplerAddressMode) -> NSUInteger {
    [
        MTL_SAMPLER_ADDRESS_MODE_REPEAT,        // REPEAT
        MTL_SAMPLER_ADDRESS_MODE_MIRROR_REPEAT, // MIRRORED_REPEAT
        MTL_SAMPLER_ADDRESS_MODE_CLAMP_TO_EDGE, // CLAMP_TO_EDGE
    ][mode as usize]
}

/// Translation of `SDLToMetal_MinMagFilter`.
pub(super) fn sdl_to_metal_min_mag_filter(filter: Filter) -> NSUInteger {
    [
        MTL_SAMPLER_MIN_MAG_FILTER_NEAREST, // NEAREST
        MTL_SAMPLER_MIN_MAG_FILTER_LINEAR,  // LINEAR
    ][filter as usize]
}

/// Translation of `SDLToMetal_MipFilter`.
pub(super) fn sdl_to_metal_mip_filter(mode: SamplerMipmapMode) -> NSUInteger {
    [
        MTL_SAMPLER_MIP_FILTER_NEAREST, // NEAREST
        MTL_SAMPLER_MIP_FILTER_LINEAR,  // LINEAR
    ][mode as usize]
}

/// Translation of `SDLToMetal_LoadOp`.
pub(super) fn sdl_to_metal_load_op(op: LoadOp) -> NSUInteger {
    [
        MTL_LOAD_ACTION_LOAD,      // LOAD
        MTL_LOAD_ACTION_CLEAR,     // CLEAR
        MTL_LOAD_ACTION_DONT_CARE, // DONT_CARE
    ][op as usize]
}

/// Translation of `SDLToMetal_StoreOp`.
pub(super) fn sdl_to_metal_store_op(op: StoreOp) -> NSUInteger {
    [
        MTL_STORE_ACTION_STORE,
        MTL_STORE_ACTION_DONT_CARE,
        MTL_STORE_ACTION_MULTISAMPLE_RESOLVE,
        MTL_STORE_ACTION_STORE_AND_MULTISAMPLE_RESOLVE,
    ][op as usize]
}

/// Translation of `SDLToMetal_StepFunction`.
pub(super) fn sdl_to_metal_step_function(rate: VertexInputRate) -> NSUInteger {
    [
        MTL_VERTEX_STEP_FUNCTION_PER_VERTEX,
        MTL_VERTEX_STEP_FUNCTION_PER_INSTANCE,
    ][rate as usize]
}

/// Translation of `SDLToMetal_SampleCount`.
pub(super) fn sdl_to_metal_sample_count(sample_count: SampleCount) -> NSUInteger {
    [
        1, // SDL_GPU_SAMPLECOUNT_1
        2, // SDL_GPU_SAMPLECOUNT_2
        4, // SDL_GPU_SAMPLECOUNT_4
        8, // SDL_GPU_SAMPLECOUNT_8
    ][sample_count as usize]
}

/// Translation of `SwapchainCompositionToFormat`.
pub(super) fn swapchain_composition_to_format(composition: SwapchainComposition) -> TextureFormat {
    [
        TextureFormat::B8G8R8A8_UNORM,      // SDR
        TextureFormat::B8G8R8A8_UNORM_SRGB, // SDR_LINEAR
        TextureFormat::R16G16B16A16_FLOAT,  // HDR_EXTENDED_LINEAR
        TextureFormat::R10G10B10A2_UNORM,   // HDR10_ST2084
    ][composition as usize]
}

/// Translation of `SwapchainCompositionToColorSpace` (upstream fills the
/// array when a device is created; the names are the framework's
/// constants).
pub(super) fn swapchain_composition_to_color_space(
    composition: SwapchainComposition,
) -> CFStringRef {
    // SAFETY: reading CoreGraphics' constant strings.
    unsafe {
        match composition {
            SwapchainComposition::Sdr | SwapchainComposition::SdrLinear => kCGColorSpaceSRGB,
            SwapchainComposition::HdrExtendedLinear => kCGColorSpaceExtendedLinearSRGB,
            SwapchainComposition::Hdr10St2084 => {
                if available!(macos(11, 0)) {
                    kCGColorSpaceITUR_2100_PQ
                } else {
                    std::ptr::null()
                }
            }
        }
    }
}

/// Translation of `SDLToMetal_TextureType()`.
pub(super) fn sdl_to_metal_texture_type(texture_type: TextureType, is_msaa: bool) -> NSUInteger {
    match texture_type {
        TextureType::Texture2D => {
            if is_msaa {
                MTL_TEXTURE_TYPE_2D_MULTISAMPLE
            } else {
                MTL_TEXTURE_TYPE_2D
            }
        }
        TextureType::Texture2DArray => MTL_TEXTURE_TYPE_2D_ARRAY,
        TextureType::Texture3D => MTL_TEXTURE_TYPE_3D,
        TextureType::Cube => MTL_TEXTURE_TYPE_CUBE,
        TextureType::CubeArray => MTL_TEXTURE_TYPE_CUBE_ARRAY,
    }
}

/// Translation of `SDLToMetal_ColorWriteMask()`.
pub(super) fn sdl_to_metal_color_write_mask(mask: ColorComponentFlags) -> NSUInteger {
    let mut result = 0;
    if mask.contains(ColorComponentFlags::R) {
        result |= MTL_COLOR_WRITE_MASK_RED;
    }
    if mask.contains(ColorComponentFlags::G) {
        result |= MTL_COLOR_WRITE_MASK_GREEN;
    }
    if mask.contains(ColorComponentFlags::B) {
        result |= MTL_COLOR_WRITE_MASK_BLUE;
    }
    if mask.contains(ColorComponentFlags::A) {
        result |= MTL_COLOR_WRITE_MASK_ALPHA;
    }
    result
}

/// Translation of `SDLToMetal_DepthClipMode()`.
pub(super) fn sdl_to_metal_depth_clip_mode(enable_depth_clip: bool) -> NSUInteger {
    if enable_depth_clip {
        MTL_DEPTH_CLIP_MODE_CLIP
    } else {
        MTL_DEPTH_CLIP_MODE_CLAMP
    }
}
