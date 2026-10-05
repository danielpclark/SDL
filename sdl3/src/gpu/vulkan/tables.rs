// Rust translation of the conversion tables and error messages of
// src/gpu/vulkan/SDL_gpu_vulkan.c from Simple DirectMedia Layer.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! The conversions from the GPU API's enums to Vulkan's (`SDLToVK_*`), the
//! swapchain composition tables and `VkErrorMessages()`.
//!
//! The tables are indexed by the GPU enums' C values, as upstream's are.

#![allow(dead_code)] // (part 2 uses the swapchain, load/store and index tables)

use crate::gpu::sysgpu::TEXTUREFORMAT_MAX_ENUM_VALUE;
use crate::gpu::{
    BlendFactor, BlendOp, CompareOp, CullMode, Filter, FrontFace, IndexElementSize, LoadOp,
    PresentMode, PrimitiveType, SampleCount, SamplerAddressMode, SamplerMipmapMode, StencilOp,
    StoreOp, SwapchainComposition, TextureFormat, VertexElementFormat, VertexInputRate,
};
use crate::video::vk::*;

/// Translation of `SDLToVK_PresentMode`.
pub(super) static SDL_TO_VK_PRESENT_MODE: [VkPresentModeKHR; 3] = [
    VK_PRESENT_MODE_FIFO_KHR,
    VK_PRESENT_MODE_IMMEDIATE_KHR,
    VK_PRESENT_MODE_MAILBOX_KHR,
];

/// Translation of `SDLToVK_TextureFormat`.
pub(super) static SDL_TO_VK_TEXTURE_FORMAT: [VkFormat; TEXTUREFORMAT_MAX_ENUM_VALUE as usize] = [
    VK_FORMAT_UNDEFINED,                   // INVALID
    VK_FORMAT_R8_UNORM,                    // A8_UNORM
    VK_FORMAT_R8_UNORM,                    // R8_UNORM
    VK_FORMAT_R8G8_UNORM,                  // R8G8_UNORM
    VK_FORMAT_R8G8B8A8_UNORM,              // R8G8B8A8_UNORM
    VK_FORMAT_R16_UNORM,                   // R16_UNORM
    VK_FORMAT_R16G16_UNORM,                // R16G16_UNORM
    VK_FORMAT_R16G16B16A16_UNORM,          // R16G16B16A16_UNORM
    VK_FORMAT_A2B10G10R10_UNORM_PACK32,    // R10G10B10A2_UNORM
    VK_FORMAT_R5G6B5_UNORM_PACK16,         // B5G6R5_UNORM
    VK_FORMAT_A1R5G5B5_UNORM_PACK16,       // B5G5R5A1_UNORM
    VK_FORMAT_B4G4R4A4_UNORM_PACK16,       // B4G4R4A4_UNORM
    VK_FORMAT_B8G8R8A8_UNORM,              // B8G8R8A8_UNORM
    VK_FORMAT_BC1_RGBA_UNORM_BLOCK,        // BC1_UNORM
    VK_FORMAT_BC2_UNORM_BLOCK,             // BC2_UNORM
    VK_FORMAT_BC3_UNORM_BLOCK,             // BC3_UNORM
    VK_FORMAT_BC4_UNORM_BLOCK,             // BC4_UNORM
    VK_FORMAT_BC5_UNORM_BLOCK,             // BC5_UNORM
    VK_FORMAT_BC7_UNORM_BLOCK,             // BC7_UNORM
    VK_FORMAT_BC6H_SFLOAT_BLOCK,           // BC6H_FLOAT
    VK_FORMAT_BC6H_UFLOAT_BLOCK,           // BC6H_UFLOAT
    VK_FORMAT_R8_SNORM,                    // R8_SNORM
    VK_FORMAT_R8G8_SNORM,                  // R8G8_SNORM
    VK_FORMAT_R8G8B8A8_SNORM,              // R8G8B8A8_SNORM
    VK_FORMAT_R16_SNORM,                   // R16_SNORM
    VK_FORMAT_R16G16_SNORM,                // R16G16_SNORM
    VK_FORMAT_R16G16B16A16_SNORM,          // R16G16B16A16_SNORM
    VK_FORMAT_R16_SFLOAT,                  // R16_FLOAT
    VK_FORMAT_R16G16_SFLOAT,               // R16G16_FLOAT
    VK_FORMAT_R16G16B16A16_SFLOAT,         // R16G16B16A16_FLOAT
    VK_FORMAT_R32_SFLOAT,                  // R32_FLOAT
    VK_FORMAT_R32G32_SFLOAT,               // R32G32_FLOAT
    VK_FORMAT_R32G32B32A32_SFLOAT,         // R32G32B32A32_FLOAT
    VK_FORMAT_B10G11R11_UFLOAT_PACK32,     // R11G11B10_UFLOAT
    VK_FORMAT_R8_UINT,                     // R8_UINT
    VK_FORMAT_R8G8_UINT,                   // R8G8_UINT
    VK_FORMAT_R8G8B8A8_UINT,               // R8G8B8A8_UINT
    VK_FORMAT_R16_UINT,                    // R16_UINT
    VK_FORMAT_R16G16_UINT,                 // R16G16_UINT
    VK_FORMAT_R16G16B16A16_UINT,           // R16G16B16A16_UINT
    VK_FORMAT_R32_UINT,                    // R32_UINT
    VK_FORMAT_R32G32_UINT,                 // R32G32_UINT
    VK_FORMAT_R32G32B32A32_UINT,           // R32G32B32A32_UINT
    VK_FORMAT_R8_SINT,                     // R8_INT
    VK_FORMAT_R8G8_SINT,                   // R8G8_INT
    VK_FORMAT_R8G8B8A8_SINT,               // R8G8B8A8_INT
    VK_FORMAT_R16_SINT,                    // R16_INT
    VK_FORMAT_R16G16_SINT,                 // R16G16_INT
    VK_FORMAT_R16G16B16A16_SINT,           // R16G16B16A16_INT
    VK_FORMAT_R32_SINT,                    // R32_INT
    VK_FORMAT_R32G32_SINT,                 // R32G32_INT
    VK_FORMAT_R32G32B32A32_SINT,           // R32G32B32A32_INT
    VK_FORMAT_R8G8B8A8_SRGB,               // R8G8B8A8_UNORM_SRGB
    VK_FORMAT_B8G8R8A8_SRGB,               // B8G8R8A8_UNORM_SRGB
    VK_FORMAT_BC1_RGBA_SRGB_BLOCK,         // BC1_UNORM_SRGB
    VK_FORMAT_BC2_SRGB_BLOCK,              // BC3_UNORM_SRGB
    VK_FORMAT_BC3_SRGB_BLOCK,              // BC3_UNORM_SRGB
    VK_FORMAT_BC7_SRGB_BLOCK,              // BC7_UNORM_SRGB
    VK_FORMAT_D16_UNORM,                   // D16_UNORM
    VK_FORMAT_X8_D24_UNORM_PACK32,         // D24_UNORM
    VK_FORMAT_D32_SFLOAT,                  // D32_FLOAT
    VK_FORMAT_D24_UNORM_S8_UINT,           // D24_UNORM_S8_UINT
    VK_FORMAT_D32_SFLOAT_S8_UINT,          // D32_FLOAT_S8_UINT
    VK_FORMAT_ASTC_4x4_UNORM_BLOCK,        // ASTC_4x4_UNORM
    VK_FORMAT_ASTC_5x4_UNORM_BLOCK,        // ASTC_5x4_UNORM
    VK_FORMAT_ASTC_5x5_UNORM_BLOCK,        // ASTC_5x5_UNORM
    VK_FORMAT_ASTC_6x5_UNORM_BLOCK,        // ASTC_6x5_UNORM
    VK_FORMAT_ASTC_6x6_UNORM_BLOCK,        // ASTC_6x6_UNORM
    VK_FORMAT_ASTC_8x5_UNORM_BLOCK,        // ASTC_8x5_UNORM
    VK_FORMAT_ASTC_8x6_UNORM_BLOCK,        // ASTC_8x6_UNORM
    VK_FORMAT_ASTC_8x8_UNORM_BLOCK,        // ASTC_8x8_UNORM
    VK_FORMAT_ASTC_10x5_UNORM_BLOCK,       // ASTC_10x5_UNORM
    VK_FORMAT_ASTC_10x6_UNORM_BLOCK,       // ASTC_10x6_UNORM
    VK_FORMAT_ASTC_10x8_UNORM_BLOCK,       // ASTC_10x8_UNORM
    VK_FORMAT_ASTC_10x10_UNORM_BLOCK,      // ASTC_10x10_UNORM
    VK_FORMAT_ASTC_12x10_UNORM_BLOCK,      // ASTC_12x10_UNORM
    VK_FORMAT_ASTC_12x12_UNORM_BLOCK,      // ASTC_12x12_UNORM
    VK_FORMAT_ASTC_4x4_SRGB_BLOCK,         // ASTC_4x4_UNORM_SRGB
    VK_FORMAT_ASTC_5x4_SRGB_BLOCK,         // ASTC_5x4_UNORM_SRGB
    VK_FORMAT_ASTC_5x5_SRGB_BLOCK,         // ASTC_5x5_UNORM_SRGB
    VK_FORMAT_ASTC_6x5_SRGB_BLOCK,         // ASTC_6x5_UNORM_SRGB
    VK_FORMAT_ASTC_6x6_SRGB_BLOCK,         // ASTC_6x6_UNORM_SRGB
    VK_FORMAT_ASTC_8x5_SRGB_BLOCK,         // ASTC_8x5_UNORM_SRGB
    VK_FORMAT_ASTC_8x6_SRGB_BLOCK,         // ASTC_8x6_UNORM_SRGB
    VK_FORMAT_ASTC_8x8_SRGB_BLOCK,         // ASTC_8x8_UNORM_SRGB
    VK_FORMAT_ASTC_10x5_SRGB_BLOCK,        // ASTC_10x5_UNORM_SRGB
    VK_FORMAT_ASTC_10x6_SRGB_BLOCK,        // ASTC_10x6_UNORM_SRGB
    VK_FORMAT_ASTC_10x8_SRGB_BLOCK,        // ASTC_10x8_UNORM_SRGB
    VK_FORMAT_ASTC_10x10_SRGB_BLOCK,       // ASTC_10x10_UNORM_SRGB
    VK_FORMAT_ASTC_12x10_SRGB_BLOCK,       // ASTC_12x10_UNORM_SRGB
    VK_FORMAT_ASTC_12x12_SRGB_BLOCK,       // ASTC_12x12_UNORM_SRGB
    VK_FORMAT_ASTC_4x4_SFLOAT_BLOCK_EXT,   // ASTC_4x4_FLOAT
    VK_FORMAT_ASTC_5x4_SFLOAT_BLOCK_EXT,   // ASTC_5x4_FLOAT
    VK_FORMAT_ASTC_5x5_SFLOAT_BLOCK_EXT,   // ASTC_5x5_FLOAT
    VK_FORMAT_ASTC_6x5_SFLOAT_BLOCK_EXT,   // ASTC_6x5_FLOAT
    VK_FORMAT_ASTC_6x6_SFLOAT_BLOCK_EXT,   // ASTC_6x6_FLOAT
    VK_FORMAT_ASTC_8x5_SFLOAT_BLOCK_EXT,   // ASTC_8x5_FLOAT
    VK_FORMAT_ASTC_8x6_SFLOAT_BLOCK_EXT,   // ASTC_8x6_FLOAT
    VK_FORMAT_ASTC_8x8_SFLOAT_BLOCK_EXT,   // ASTC_8x8_FLOAT
    VK_FORMAT_ASTC_10x5_SFLOAT_BLOCK_EXT,  // ASTC_10x5_FLOAT
    VK_FORMAT_ASTC_10x6_SFLOAT_BLOCK_EXT,  // ASTC_10x6_FLOAT
    VK_FORMAT_ASTC_10x8_SFLOAT_BLOCK_EXT,  // ASTC_10x8_FLOAT
    VK_FORMAT_ASTC_10x10_SFLOAT_BLOCK_EXT, // ASTC_10x10_FLOAT
    VK_FORMAT_ASTC_12x10_SFLOAT_BLOCK_EXT, // ASTC_12x10_FLOAT
    VK_FORMAT_ASTC_12x12_SFLOAT_BLOCK,     // ASTC_12x12_FLOAT
];

/// `SDLToVK_TextureFormat[format]`.
///
/// Note (upstream): C reads past the table for a format out of range when
/// debug mode doesn't reject it first; that is `VK_FORMAT_UNDEFINED` here.
pub(super) fn sdl_to_vk_texture_format(format: TextureFormat) -> VkFormat {
    SDL_TO_VK_TEXTURE_FORMAT
        .get(format.0 as usize)
        .copied()
        .unwrap_or(VK_FORMAT_UNDEFINED)
}

/// `IDENTITY_SWIZZLE`
pub(super) const IDENTITY_SWIZZLE: VkComponentMapping = VkComponentMapping {
    r: VK_COMPONENT_SWIZZLE_IDENTITY,
    g: VK_COMPONENT_SWIZZLE_IDENTITY,
    b: VK_COMPONENT_SWIZZLE_IDENTITY,
    a: VK_COMPONENT_SWIZZLE_IDENTITY,
};

/// The swizzle of the views of a texture format. Translation of
/// `SwizzleForSDLFormat()`.
pub(super) fn swizzle_for_sdl_format(format: TextureFormat) -> VkComponentMapping {
    if format == TextureFormat::A8_UNORM {
        // TODO: use VK_FORMAT_A8_UNORM_KHR from VK_KHR_maintenance5 when available
        return VkComponentMapping {
            r: VK_COMPONENT_SWIZZLE_ZERO,
            g: VK_COMPONENT_SWIZZLE_ZERO,
            b: VK_COMPONENT_SWIZZLE_ZERO,
            a: VK_COMPONENT_SWIZZLE_R,
        };
    }

    if format == TextureFormat::B4G4R4A4_UNORM {
        // ARGB -> BGRA
        // TODO: use VK_FORMAT_A4R4G4B4_UNORM_PACK16_EXT from VK_EXT_4444_formats when available
        return VkComponentMapping {
            r: VK_COMPONENT_SWIZZLE_G,
            g: VK_COMPONENT_SWIZZLE_R,
            b: VK_COMPONENT_SWIZZLE_A,
            a: VK_COMPONENT_SWIZZLE_B,
        };
    }

    IDENTITY_SWIZZLE
}

/// Translation of `SwapchainCompositionToFormat`.
pub(super) static SWAPCHAIN_COMPOSITION_TO_FORMAT: [VkFormat; 4] = [
    VK_FORMAT_B8G8R8A8_UNORM,           // SDR
    VK_FORMAT_B8G8R8A8_SRGB,            // SDR_LINEAR
    VK_FORMAT_R16G16B16A16_SFLOAT,      // HDR_EXTENDED_LINEAR
    VK_FORMAT_A2B10G10R10_UNORM_PACK32, // HDR10_ST2084
];

/// Translation of `SwapchainCompositionToFallbackFormat`.
pub(super) static SWAPCHAIN_COMPOSITION_TO_FALLBACK_FORMAT: [VkFormat; 4] = [
    VK_FORMAT_R8G8B8A8_UNORM, // SDR
    VK_FORMAT_R8G8B8A8_SRGB,  // SDR_LINEAR
    VK_FORMAT_UNDEFINED,      // HDR_EXTENDED_LINEAR (no fallback)
    VK_FORMAT_UNDEFINED,      // HDR10_ST2084 (no fallback)
];

/// The texture format of a swapchain composition. Translation of
/// `SwapchainCompositionToSDLFormat()` (the enum has no other values, so
/// there's no `INVALID` case).
pub(super) fn swapchain_composition_to_sdl_format(
    composition: SwapchainComposition,
    using_fallback: bool,
) -> TextureFormat {
    match composition {
        SwapchainComposition::Sdr => {
            if using_fallback {
                TextureFormat::R8G8B8A8_UNORM
            } else {
                TextureFormat::B8G8R8A8_UNORM
            }
        }
        SwapchainComposition::SdrLinear => {
            if using_fallback {
                TextureFormat::R8G8B8A8_UNORM_SRGB
            } else {
                TextureFormat::B8G8R8A8_UNORM_SRGB
            }
        }
        SwapchainComposition::HdrExtendedLinear => TextureFormat::R16G16B16A16_FLOAT,
        SwapchainComposition::Hdr10St2084 => TextureFormat::R10G10B10A2_UNORM,
    }
}

/// Translation of `SwapchainCompositionToColorSpace`.
pub(super) static SWAPCHAIN_COMPOSITION_TO_COLOR_SPACE: [VkColorSpaceKHR; 4] = [
    VK_COLOR_SPACE_SRGB_NONLINEAR_KHR,       // SDR
    VK_COLOR_SPACE_SRGB_NONLINEAR_KHR,       // SDR_LINEAR
    VK_COLOR_SPACE_EXTENDED_SRGB_LINEAR_EXT, // HDR_EXTENDED_LINEAR
    VK_COLOR_SPACE_HDR10_ST2084_EXT,         // HDR10_ST2084
];

/// Translation of `SwapchainCompositionSwizzle`.
pub(super) static SWAPCHAIN_COMPOSITION_SWIZZLE: [VkComponentMapping; 4] = [
    IDENTITY_SWIZZLE, // SDR
    IDENTITY_SWIZZLE, // SDR_LINEAR
    IDENTITY_SWIZZLE, // HDR_EXTENDED_LINEAR
    // HDR10_ST2084
    VkComponentMapping {
        r: VK_COMPONENT_SWIZZLE_R,
        g: VK_COMPONENT_SWIZZLE_G,
        b: VK_COMPONENT_SWIZZLE_B,
        a: VK_COMPONENT_SWIZZLE_A,
    },
];

/// Translation of `SDLToVK_VertexFormat` (indexed by the C value, which
/// starts at 1 after `INVALID`).
pub(super) static SDL_TO_VK_VERTEX_FORMAT: [VkFormat; 31] = [
    VK_FORMAT_UNDEFINED,           // INVALID
    VK_FORMAT_R32_SINT,            // INT
    VK_FORMAT_R32G32_SINT,         // INT2
    VK_FORMAT_R32G32B32_SINT,      // INT3
    VK_FORMAT_R32G32B32A32_SINT,   // INT4
    VK_FORMAT_R32_UINT,            // UINT
    VK_FORMAT_R32G32_UINT,         // UINT2
    VK_FORMAT_R32G32B32_UINT,      // UINT3
    VK_FORMAT_R32G32B32A32_UINT,   // UINT4
    VK_FORMAT_R32_SFLOAT,          // FLOAT
    VK_FORMAT_R32G32_SFLOAT,       // FLOAT2
    VK_FORMAT_R32G32B32_SFLOAT,    // FLOAT3
    VK_FORMAT_R32G32B32A32_SFLOAT, // FLOAT4
    VK_FORMAT_R8G8_SINT,           // BYTE2
    VK_FORMAT_R8G8B8A8_SINT,       // BYTE4
    VK_FORMAT_R8G8_UINT,           // UBYTE2
    VK_FORMAT_R8G8B8A8_UINT,       // UBYTE4
    VK_FORMAT_R8G8_SNORM,          // BYTE2_NORM
    VK_FORMAT_R8G8B8A8_SNORM,      // BYTE4_NORM
    VK_FORMAT_R8G8_UNORM,          // UBYTE2_NORM
    VK_FORMAT_R8G8B8A8_UNORM,      // UBYTE4_NORM
    VK_FORMAT_R16G16_SINT,         // SHORT2
    VK_FORMAT_R16G16B16A16_SINT,   // SHORT4
    VK_FORMAT_R16G16_UINT,         // USHORT2
    VK_FORMAT_R16G16B16A16_UINT,   // USHORT4
    VK_FORMAT_R16G16_SNORM,        // SHORT2_NORM
    VK_FORMAT_R16G16B16A16_SNORM,  // SHORT4_NORM
    VK_FORMAT_R16G16_UNORM,        // USHORT2_NORM
    VK_FORMAT_R16G16B16A16_UNORM,  // USHORT4_NORM
    VK_FORMAT_R16G16_SFLOAT,       // HALF2
    VK_FORMAT_R16G16B16A16_SFLOAT, // HALF4
];

/// `SDLToVK_VertexFormat[format]`.
pub(super) fn sdl_to_vk_vertex_format(format: VertexElementFormat) -> VkFormat {
    SDL_TO_VK_VERTEX_FORMAT[format as usize]
}

/// Translation of `SDLToVK_IndexType`.
pub(super) fn sdl_to_vk_index_type(size: IndexElementSize) -> i32 {
    [VK_INDEX_TYPE_UINT16, VK_INDEX_TYPE_UINT32][size as usize]
}

/// Translation of `SDLToVK_PrimitiveType`.
pub(super) fn sdl_to_vk_primitive_type(primitive_type: PrimitiveType) -> VkPrimitiveTopology {
    [
        VK_PRIMITIVE_TOPOLOGY_TRIANGLE_LIST,
        VK_PRIMITIVE_TOPOLOGY_TRIANGLE_STRIP,
        VK_PRIMITIVE_TOPOLOGY_LINE_LIST,
        VK_PRIMITIVE_TOPOLOGY_LINE_STRIP,
        VK_PRIMITIVE_TOPOLOGY_POINT_LIST,
    ][primitive_type as usize]
}

/// Translation of `SDLToVK_CullMode` (whose last entry,
/// `VK_CULL_MODE_FRONT_AND_BACK`, no `SDL_GPUCullMode` reaches).
pub(super) fn sdl_to_vk_cull_mode(cull_mode: CullMode) -> VkFlags {
    [
        VK_CULL_MODE_NONE,
        VK_CULL_MODE_FRONT_BIT,
        VK_CULL_MODE_BACK_BIT,
        VK_CULL_MODE_FRONT_AND_BACK,
    ][cull_mode as usize]
}

/// Translation of `SDLToVK_FrontFace`.
pub(super) fn sdl_to_vk_front_face(front_face: FrontFace) -> i32 {
    [VK_FRONT_FACE_COUNTER_CLOCKWISE, VK_FRONT_FACE_CLOCKWISE][front_face as usize]
}

/// Translation of `SDLToVK_BlendFactor`.
pub(super) static SDL_TO_VK_BLEND_FACTOR: [VkBlendFactor; 14] = [
    VK_BLEND_FACTOR_ZERO, // INVALID
    VK_BLEND_FACTOR_ZERO,
    VK_BLEND_FACTOR_ONE,
    VK_BLEND_FACTOR_SRC_COLOR,
    VK_BLEND_FACTOR_ONE_MINUS_SRC_COLOR,
    VK_BLEND_FACTOR_DST_COLOR,
    VK_BLEND_FACTOR_ONE_MINUS_DST_COLOR,
    VK_BLEND_FACTOR_SRC_ALPHA,
    VK_BLEND_FACTOR_ONE_MINUS_SRC_ALPHA,
    VK_BLEND_FACTOR_DST_ALPHA,
    VK_BLEND_FACTOR_ONE_MINUS_DST_ALPHA,
    VK_BLEND_FACTOR_CONSTANT_COLOR,
    VK_BLEND_FACTOR_ONE_MINUS_CONSTANT_COLOR,
    VK_BLEND_FACTOR_SRC_ALPHA_SATURATE,
];

/// `SDLToVK_BlendFactor[factor]`.
pub(super) fn sdl_to_vk_blend_factor(factor: BlendFactor) -> VkBlendFactor {
    SDL_TO_VK_BLEND_FACTOR[factor as usize]
}

/// Translation of `SDLToVK_BlendOp`.
pub(super) static SDL_TO_VK_BLEND_OP: [VkBlendOp; 6] = [
    VK_BLEND_OP_ADD, // INVALID
    VK_BLEND_OP_ADD,
    VK_BLEND_OP_SUBTRACT,
    VK_BLEND_OP_REVERSE_SUBTRACT,
    VK_BLEND_OP_MIN,
    VK_BLEND_OP_MAX,
];

/// `SDLToVK_BlendOp[op]`.
pub(super) fn sdl_to_vk_blend_op(op: BlendOp) -> VkBlendOp {
    SDL_TO_VK_BLEND_OP[op as usize]
}

/// Translation of `SDLToVK_CompareOp`.
pub(super) static SDL_TO_VK_COMPARE_OP: [i32; 9] = [
    VK_COMPARE_OP_NEVER, // INVALID
    VK_COMPARE_OP_NEVER,
    VK_COMPARE_OP_LESS,
    VK_COMPARE_OP_EQUAL,
    VK_COMPARE_OP_LESS_OR_EQUAL,
    VK_COMPARE_OP_GREATER,
    VK_COMPARE_OP_NOT_EQUAL,
    VK_COMPARE_OP_GREATER_OR_EQUAL,
    VK_COMPARE_OP_ALWAYS,
];

/// `SDLToVK_CompareOp[op]`.
pub(super) fn sdl_to_vk_compare_op(op: CompareOp) -> i32 {
    SDL_TO_VK_COMPARE_OP[op as usize]
}

/// Translation of `SDLToVK_StencilOp`.
pub(super) static SDL_TO_VK_STENCIL_OP: [i32; 9] = [
    VK_STENCIL_OP_KEEP, // INVALID
    VK_STENCIL_OP_KEEP,
    VK_STENCIL_OP_ZERO,
    VK_STENCIL_OP_REPLACE,
    VK_STENCIL_OP_INCREMENT_AND_CLAMP,
    VK_STENCIL_OP_DECREMENT_AND_CLAMP,
    VK_STENCIL_OP_INVERT,
    VK_STENCIL_OP_INCREMENT_AND_WRAP,
    VK_STENCIL_OP_DECREMENT_AND_WRAP,
];

/// `SDLToVK_StencilOp[op]`.
pub(super) fn sdl_to_vk_stencil_op(op: StencilOp) -> i32 {
    SDL_TO_VK_STENCIL_OP[op as usize]
}

/// Translation of `SDLToVK_LoadOp`.
pub(super) fn sdl_to_vk_load_op(op: LoadOp) -> VkAttachmentLoadOp {
    [
        VK_ATTACHMENT_LOAD_OP_LOAD,
        VK_ATTACHMENT_LOAD_OP_CLEAR,
        VK_ATTACHMENT_LOAD_OP_DONT_CARE,
    ][op as usize]
}

/// Translation of `SDLToVK_StoreOp`.
pub(super) fn sdl_to_vk_store_op(op: StoreOp) -> i32 {
    [
        VK_ATTACHMENT_STORE_OP_STORE,
        VK_ATTACHMENT_STORE_OP_DONT_CARE,
        VK_ATTACHMENT_STORE_OP_DONT_CARE,
        VK_ATTACHMENT_STORE_OP_STORE,
    ][op as usize]
}

/// Translation of `SDLToVK_SampleCount`.
pub(super) fn sdl_to_vk_sample_count(sample_count: SampleCount) -> VkFlags {
    [
        VK_SAMPLE_COUNT_1_BIT,
        VK_SAMPLE_COUNT_2_BIT,
        VK_SAMPLE_COUNT_4_BIT,
        VK_SAMPLE_COUNT_8_BIT,
    ][sample_count as usize]
}

/// Translation of `SDLToVK_VertexInputRate`.
pub(super) fn sdl_to_vk_vertex_input_rate(rate: VertexInputRate) -> i32 {
    [VK_VERTEX_INPUT_RATE_VERTEX, VK_VERTEX_INPUT_RATE_INSTANCE][rate as usize]
}

/// Translation of `SDLToVK_Filter`.
pub(super) fn sdl_to_vk_filter(filter: Filter) -> i32 {
    [VK_FILTER_NEAREST, VK_FILTER_LINEAR][filter as usize]
}

/// Translation of `SDLToVK_SamplerMipmapMode`.
pub(super) fn sdl_to_vk_sampler_mipmap_mode(mode: SamplerMipmapMode) -> i32 {
    [
        VK_SAMPLER_MIPMAP_MODE_NEAREST,
        VK_SAMPLER_MIPMAP_MODE_LINEAR,
    ][mode as usize]
}

/// Translation of `SDLToVK_SamplerAddressMode`.
pub(super) fn sdl_to_vk_sampler_address_mode(mode: SamplerAddressMode) -> i32 {
    [
        VK_SAMPLER_ADDRESS_MODE_REPEAT,
        VK_SAMPLER_ADDRESS_MODE_MIRRORED_REPEAT,
        VK_SAMPLER_ADDRESS_MODE_CLAMP_TO_EDGE,
    ][mode as usize]
}

/// `SDLToVK_PresentMode[mode]`.
pub(super) fn sdl_to_vk_present_mode(mode: PresentMode) -> VkPresentModeKHR {
    SDL_TO_VK_PRESENT_MODE[mode as usize]
}

/// The name of a `VkResult`. Translation of `VkErrorMessages()`.
pub(super) fn vk_error_messages(code: VkResult) -> &'static str {
    match code {
        VK_ERROR_OUT_OF_HOST_MEMORY => "VK_ERROR_OUT_OF_HOST_MEMORY",
        VK_ERROR_OUT_OF_DEVICE_MEMORY => "VK_ERROR_OUT_OF_DEVICE_MEMORY",
        VK_ERROR_FRAGMENTED_POOL => "VK_ERROR_FRAGMENTED_POOL",
        VK_ERROR_OUT_OF_POOL_MEMORY => "VK_ERROR_OUT_OF_POOL_MEMORY",
        VK_ERROR_INITIALIZATION_FAILED => "VK_ERROR_INITIALIZATION_FAILED",
        VK_ERROR_LAYER_NOT_PRESENT => "VK_ERROR_LAYER_NOT_PRESENT",
        VK_ERROR_EXTENSION_NOT_PRESENT => "VK_ERROR_EXTENSION_NOT_PRESENT",
        VK_ERROR_FEATURE_NOT_PRESENT => "VK_ERROR_FEATURE_NOT_PRESENT",
        VK_ERROR_TOO_MANY_OBJECTS => "VK_ERROR_TOO_MANY_OBJECTS",
        VK_ERROR_DEVICE_LOST => "VK_ERROR_DEVICE_LOST",
        VK_ERROR_INCOMPATIBLE_DRIVER => "VK_ERROR_INCOMPATIBLE_DRIVER",
        VK_ERROR_OUT_OF_DATE_KHR => "VK_ERROR_OUT_OF_DATE_KHR",
        VK_ERROR_SURFACE_LOST_KHR => "VK_ERROR_SURFACE_LOST_KHR",
        VK_ERROR_FULL_SCREEN_EXCLUSIVE_MODE_LOST_EXT => {
            "VK_ERROR_FULL_SCREEN_EXCLUSIVE_MODE_LOST_EXT"
        }
        VK_SUBOPTIMAL_KHR => "VK_SUBOPTIMAL_KHR",
        VK_ERROR_NATIVE_WINDOW_IN_USE_KHR => "VK_ERROR_NATIVE_WINDOW_IN_USE_KHR",
        VK_ERROR_INVALID_SHADER_NV => "VK_ERROR_INVALID_SHADER_NV",
        _ => "Unhandled VkResult!",
    }
}
