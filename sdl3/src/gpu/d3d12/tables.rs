// Rust translation of the conversion tables of src/gpu/d3d12/SDL_gpu_d3d12.c
// from Simple DirectMedia Layer.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! The conversions from the GPU API's enums to Direct3D 12's
//! (`SDLToD3D12_*`), the swapchain composition tables and
//! `SDLToD3D12_Filter()`.
//!
//! The tables are indexed by the GPU enums' C values, as upstream's are.
//! The texture formats have three tables: the format of the views
//! (`SDLToD3D12_TextureFormat`), the format of the depth-stencil views and
//! targets (`SDLToD3D12_DepthFormat`) and the typeless format of depth
//! textures that are sampled too (`SDLToD3D12_TypelessFormat`).

use super::d3d::*;
use crate::gpu::sysgpu::TEXTUREFORMAT_MAX_ENUM_VALUE;
use crate::gpu::{
    BlendFactor, BlendOp, CompareOp, CullMode, FillMode, Filter, PrimitiveType, SampleCount,
    SamplerAddressMode, SamplerMipmapMode, StencilOp, SwapchainComposition, TextureFormat,
    VertexElementFormat, VertexInputRate,
};
use crate::render::direct3d11::d3d::*;

// Conversions

/// Translation of `SwapchainCompositionToSDLTextureFormat`.
pub(super) static SWAPCHAIN_COMPOSITION_TO_SDL_TEXTURE_FORMAT: [TextureFormat; 4] = [
    TextureFormat::B8G8R8A8_UNORM,      // SDR
    TextureFormat::B8G8R8A8_UNORM_SRGB, // SDR_LINEAR
    TextureFormat::R16G16B16A16_FLOAT,  // HDR_EXTENDED_LINEAR
    TextureFormat::R10G10B10A2_UNORM,   // HDR10_ST2084
];

/// Translation of `SwapchainCompositionToTextureFormat`.
pub(super) static SWAPCHAIN_COMPOSITION_TO_TEXTURE_FORMAT: [DxgiFormat; 4] = [
    DXGI_FORMAT_B8G8R8A8_UNORM,     // SDR
    DXGI_FORMAT_B8G8R8A8_UNORM,     // SDR_LINEAR (NOTE: The RTV uses the sRGB format)
    DXGI_FORMAT_R16G16B16A16_FLOAT, // HDR_EXTENDED_LINEAR
    DXGI_FORMAT_R10G10B10A2_UNORM,  // HDR10_ST2084
];

/// Translation of `SwapchainCompositionToColorSpace`.
pub(super) static SWAPCHAIN_COMPOSITION_TO_COLOR_SPACE: [DxgiColorSpaceType; 4] = [
    DXGI_COLOR_SPACE_RGB_FULL_G22_NONE_P709,    // SDR
    DXGI_COLOR_SPACE_RGB_FULL_G22_NONE_P709,    // SDR_LINEAR
    DXGI_COLOR_SPACE_RGB_FULL_G10_NONE_P709,    // HDR_EXTENDED_LINEAR
    DXGI_COLOR_SPACE_RGB_FULL_G2084_NONE_P2020, // HDR10_ST2084
];

/// The tables of a swapchain composition (indexed as upstream's are).
pub(super) fn swapchain_composition_index(composition: SwapchainComposition) -> usize {
    composition as usize
}

/// Translation of `SDLToD3D12_BlendFactor`.
pub(super) static SDL_TO_D3D12_BLEND_FACTOR: [u32; 14] = [
    D3D12_BLEND_ZERO,             // INVALID
    D3D12_BLEND_ZERO,             // ZERO
    D3D12_BLEND_ONE,              // ONE
    D3D12_BLEND_SRC_COLOR,        // SRC_COLOR
    D3D12_BLEND_INV_SRC_COLOR,    // ONE_MINUS_SRC_COLOR
    D3D12_BLEND_DEST_COLOR,       // DST_COLOR
    D3D12_BLEND_INV_DEST_COLOR,   // ONE_MINUS_DST_COLOR
    D3D12_BLEND_SRC_ALPHA,        // SRC_ALPHA
    D3D12_BLEND_INV_SRC_ALPHA,    // ONE_MINUS_SRC_ALPHA
    D3D12_BLEND_DEST_ALPHA,       // DST_ALPHA
    D3D12_BLEND_INV_DEST_ALPHA,   // ONE_MINUS_DST_ALPHA
    D3D12_BLEND_BLEND_FACTOR,     // CONSTANT_COLOR
    D3D12_BLEND_INV_BLEND_FACTOR, // ONE_MINUS_CONSTANT_COLOR
    D3D12_BLEND_SRC_ALPHA_SAT,    // SRC_ALPHA_SATURATE
];

/// Translation of `SDLToD3D12_BlendFactorAlpha`.
pub(super) static SDL_TO_D3D12_BLEND_FACTOR_ALPHA: [u32; 14] = [
    D3D12_BLEND_ZERO,             // INVALID
    D3D12_BLEND_ZERO,             // ZERO
    D3D12_BLEND_ONE,              // ONE
    D3D12_BLEND_SRC_ALPHA,        // SRC_COLOR
    D3D12_BLEND_INV_SRC_ALPHA,    // ONE_MINUS_SRC_COLOR
    D3D12_BLEND_DEST_ALPHA,       // DST_COLOR
    D3D12_BLEND_INV_DEST_ALPHA,   // ONE_MINUS_DST_COLOR
    D3D12_BLEND_SRC_ALPHA,        // SRC_ALPHA
    D3D12_BLEND_INV_SRC_ALPHA,    // ONE_MINUS_SRC_ALPHA
    D3D12_BLEND_DEST_ALPHA,       // DST_ALPHA
    D3D12_BLEND_INV_DEST_ALPHA,   // ONE_MINUS_DST_ALPHA
    D3D12_BLEND_BLEND_FACTOR,     // CONSTANT_COLOR
    D3D12_BLEND_INV_BLEND_FACTOR, // ONE_MINUS_CONSTANT_COLOR
    D3D12_BLEND_SRC_ALPHA_SAT,    // SRC_ALPHA_SATURATE
];

/// Translation of `SDLToD3D12_BlendOp`.
pub(super) static SDL_TO_D3D12_BLEND_OP: [u32; 6] = [
    D3D12_BLEND_OP_ADD,          // INVALID
    D3D12_BLEND_OP_ADD,          // ADD
    D3D12_BLEND_OP_SUBTRACT,     // SUBTRACT
    D3D12_BLEND_OP_REV_SUBTRACT, // REVERSE_SUBTRACT
    D3D12_BLEND_OP_MIN,          // MIN
    D3D12_BLEND_OP_MAX,          // MAX
];

// These are actually color formats.
// For some genius reason, D3D12 splits format capabilities for depth-stencil views.
/// Translation of `SDLToD3D12_TextureFormat`.
pub(super) static SDL_TO_D3D12_TEXTURE_FORMAT: [DxgiFormat; TEXTUREFORMAT_MAX_ENUM_VALUE as usize] = [
    DXGI_FORMAT_UNKNOWN,                  // INVALID
    DXGI_FORMAT_A8_UNORM,                 // A8_UNORM
    DXGI_FORMAT_R8_UNORM,                 // R8_UNORM
    DXGI_FORMAT_R8G8_UNORM,               // R8G8_UNORM
    DXGI_FORMAT_R8G8B8A8_UNORM,           // R8G8B8A8_UNORM
    DXGI_FORMAT_R16_UNORM,                // R16_UNORM
    DXGI_FORMAT_R16G16_UNORM,             // R16G16_UNORM
    DXGI_FORMAT_R16G16B16A16_UNORM,       // R16G16B16A16_UNORM
    DXGI_FORMAT_R10G10B10A2_UNORM,        // R10G10B10A2_UNORM
    DXGI_FORMAT_B5G6R5_UNORM,             // B5G6R5_UNORM
    DXGI_FORMAT_B5G5R5A1_UNORM,           // B5G5R5A1_UNORM
    DXGI_FORMAT_B4G4R4A4_UNORM,           // B4G4R4A4_UNORM
    DXGI_FORMAT_B8G8R8A8_UNORM,           // B8G8R8A8_UNORM
    DXGI_FORMAT_BC1_UNORM,                // BC1_UNORM
    DXGI_FORMAT_BC2_UNORM,                // BC2_UNORM
    DXGI_FORMAT_BC3_UNORM,                // BC3_UNORM
    DXGI_FORMAT_BC4_UNORM,                // BC4_UNORM
    DXGI_FORMAT_BC5_UNORM,                // BC5_UNORM
    DXGI_FORMAT_BC7_UNORM,                // BC7_UNORM
    DXGI_FORMAT_BC6H_SF16,                // BC6H_FLOAT
    DXGI_FORMAT_BC6H_UF16,                // BC6H_UFLOAT
    DXGI_FORMAT_R8_SNORM,                 // R8_SNORM
    DXGI_FORMAT_R8G8_SNORM,               // R8G8_SNORM
    DXGI_FORMAT_R8G8B8A8_SNORM,           // R8G8B8A8_SNORM
    DXGI_FORMAT_R16_SNORM,                // R16_SNORM
    DXGI_FORMAT_R16G16_SNORM,             // R16G16_SNORM
    DXGI_FORMAT_R16G16B16A16_SNORM,       // R16G16B16A16_SNORM
    DXGI_FORMAT_R16_FLOAT,                // R16_FLOAT
    DXGI_FORMAT_R16G16_FLOAT,             // R16G16_FLOAT
    DXGI_FORMAT_R16G16B16A16_FLOAT,       // R16G16B16A16_FLOAT
    DXGI_FORMAT_R32_FLOAT,                // R32_FLOAT
    DXGI_FORMAT_R32G32_FLOAT,             // R32G32_FLOAT
    DXGI_FORMAT_R32G32B32A32_FLOAT,       // R32G32B32A32_FLOAT
    DXGI_FORMAT_R11G11B10_FLOAT,          // R11G11B10_UFLOAT
    DXGI_FORMAT_R8_UINT,                  // R8_UINT
    DXGI_FORMAT_R8G8_UINT,                // R8G8_UINT
    DXGI_FORMAT_R8G8B8A8_UINT,            // R8G8B8A8_UINT
    DXGI_FORMAT_R16_UINT,                 // R16_UINT
    DXGI_FORMAT_R16G16_UINT,              // R16G16_UINT
    DXGI_FORMAT_R16G16B16A16_UINT,        // R16G16B16A16_UINT
    DXGI_FORMAT_R32_UINT,                 // R32_UINT
    DXGI_FORMAT_R32G32_UINT,              // R32G32_UINT
    DXGI_FORMAT_R32G32B32A32_UINT,        // R32G32B32A32_UINT
    DXGI_FORMAT_R8_SINT,                  // R8_INT
    DXGI_FORMAT_R8G8_SINT,                // R8G8_INT
    DXGI_FORMAT_R8G8B8A8_SINT,            // R8G8B8A8_INT
    DXGI_FORMAT_R16_SINT,                 // R16_INT
    DXGI_FORMAT_R16G16_SINT,              // R16G16_INT
    DXGI_FORMAT_R16G16B16A16_SINT,        // R16G16B16A16_INT
    DXGI_FORMAT_R32_SINT,                 // R32_INT
    DXGI_FORMAT_R32G32_SINT,              // R32G32_INT
    DXGI_FORMAT_R32G32B32A32_SINT,        // R32G32B32A32_INT
    DXGI_FORMAT_R8G8B8A8_UNORM_SRGB,      // R8G8B8A8_UNORM_SRGB
    DXGI_FORMAT_B8G8R8A8_UNORM_SRGB,      // B8G8R8A8_UNORM_SRGB
    DXGI_FORMAT_BC1_UNORM_SRGB,           // BC1_UNORM_SRGB
    DXGI_FORMAT_BC2_UNORM_SRGB,           // BC2_UNORM_SRGB
    DXGI_FORMAT_BC3_UNORM_SRGB,           // BC3_UNORM_SRGB
    DXGI_FORMAT_BC7_UNORM_SRGB,           // BC7_UNORM_SRGB
    DXGI_FORMAT_R16_UNORM,                // D16_UNORM
    DXGI_FORMAT_R24_UNORM_X8_TYPELESS,    // D24_UNORM
    DXGI_FORMAT_R32_FLOAT,                // D32_FLOAT
    DXGI_FORMAT_R24_UNORM_X8_TYPELESS,    // D24_UNORM_S8_UINT
    DXGI_FORMAT_R32_FLOAT_X8X24_TYPELESS, // D32_FLOAT_S8_UINT
    DXGI_FORMAT_UNKNOWN,                  // ASTC_4x4_UNORM
    DXGI_FORMAT_UNKNOWN,                  // ASTC_5x4_UNORM
    DXGI_FORMAT_UNKNOWN,                  // ASTC_5x5_UNORM
    DXGI_FORMAT_UNKNOWN,                  // ASTC_6x5_UNORM
    DXGI_FORMAT_UNKNOWN,                  // ASTC_6x6_UNORM
    DXGI_FORMAT_UNKNOWN,                  // ASTC_8x5_UNORM
    DXGI_FORMAT_UNKNOWN,                  // ASTC_8x6_UNORM
    DXGI_FORMAT_UNKNOWN,                  // ASTC_8x8_UNORM
    DXGI_FORMAT_UNKNOWN,                  // ASTC_10x5_UNORM
    DXGI_FORMAT_UNKNOWN,                  // ASTC_10x6_UNORM
    DXGI_FORMAT_UNKNOWN,                  // ASTC_10x8_UNORM
    DXGI_FORMAT_UNKNOWN,                  // ASTC_10x10_UNORM
    DXGI_FORMAT_UNKNOWN,                  // ASTC_12x10_UNORM
    DXGI_FORMAT_UNKNOWN,                  // ASTC_12x12_UNORM
    DXGI_FORMAT_UNKNOWN,                  // ASTC_4x4_UNORM_SRGB
    DXGI_FORMAT_UNKNOWN,                  // ASTC_5x4_UNORM_SRGB
    DXGI_FORMAT_UNKNOWN,                  // ASTC_5x5_UNORM_SRGB
    DXGI_FORMAT_UNKNOWN,                  // ASTC_6x5_UNORM_SRGB
    DXGI_FORMAT_UNKNOWN,                  // ASTC_6x6_UNORM_SRGB
    DXGI_FORMAT_UNKNOWN,                  // ASTC_8x5_UNORM_SRGB
    DXGI_FORMAT_UNKNOWN,                  // ASTC_8x6_UNORM_SRGB
    DXGI_FORMAT_UNKNOWN,                  // ASTC_8x8_UNORM_SRGB
    DXGI_FORMAT_UNKNOWN,                  // ASTC_10x5_UNORM_SRGB
    DXGI_FORMAT_UNKNOWN,                  // ASTC_10x6_UNORM_SRGB
    DXGI_FORMAT_UNKNOWN,                  // ASTC_10x8_UNORM_SRGB
    DXGI_FORMAT_UNKNOWN,                  // ASTC_10x10_UNORM_SRGB
    DXGI_FORMAT_UNKNOWN,                  // ASTC_12x10_UNORM_SRGB
    DXGI_FORMAT_UNKNOWN,                  // ASTC_12x12_UNORM_SRGB
    DXGI_FORMAT_UNKNOWN,                  // ASTC_4x4_FLOAT
    DXGI_FORMAT_UNKNOWN,                  // ASTC_5x4_FLOAT
    DXGI_FORMAT_UNKNOWN,                  // ASTC_5x5_FLOAT
    DXGI_FORMAT_UNKNOWN,                  // ASTC_6x5_FLOAT
    DXGI_FORMAT_UNKNOWN,                  // ASTC_6x6_FLOAT
    DXGI_FORMAT_UNKNOWN,                  // ASTC_8x5_FLOAT
    DXGI_FORMAT_UNKNOWN,                  // ASTC_8x6_FLOAT
    DXGI_FORMAT_UNKNOWN,                  // ASTC_8x8_FLOAT
    DXGI_FORMAT_UNKNOWN,                  // ASTC_10x5_FLOAT
    DXGI_FORMAT_UNKNOWN,                  // ASTC_10x6_FLOAT
    DXGI_FORMAT_UNKNOWN,                  // ASTC_10x8_FLOAT
    DXGI_FORMAT_UNKNOWN,                  // ASTC_10x10_FLOAT
    DXGI_FORMAT_UNKNOWN,                  // ASTC_12x10_FLOAT
    DXGI_FORMAT_UNKNOWN,                  // ASTC_12x12_FLOAT
];

/// Translation of `SDLToD3D12_DepthFormat`.
pub(super) static SDL_TO_D3D12_DEPTH_FORMAT: [DxgiFormat; TEXTUREFORMAT_MAX_ENUM_VALUE as usize] = [
    DXGI_FORMAT_UNKNOWN,              // INVALID
    DXGI_FORMAT_UNKNOWN,              // A8_UNORM
    DXGI_FORMAT_UNKNOWN,              // R8_UNORM
    DXGI_FORMAT_UNKNOWN,              // R8G8_UNORM
    DXGI_FORMAT_UNKNOWN,              // R8G8B8A8_UNORM
    DXGI_FORMAT_UNKNOWN,              // R16_UNORM
    DXGI_FORMAT_UNKNOWN,              // R16G16_UNORM
    DXGI_FORMAT_UNKNOWN,              // R16G16B16A16_UNORM
    DXGI_FORMAT_UNKNOWN,              // R10G10B10A2_UNORM
    DXGI_FORMAT_UNKNOWN,              // B5G6R5_UNORM
    DXGI_FORMAT_UNKNOWN,              // B5G5R5A1_UNORM
    DXGI_FORMAT_UNKNOWN,              // B4G4R4A4_UNORM
    DXGI_FORMAT_UNKNOWN,              // B8G8R8A8_UNORM
    DXGI_FORMAT_UNKNOWN,              // BC1_UNORM
    DXGI_FORMAT_UNKNOWN,              // BC2_UNORM
    DXGI_FORMAT_UNKNOWN,              // BC3_UNORM
    DXGI_FORMAT_UNKNOWN,              // BC4_UNORM
    DXGI_FORMAT_UNKNOWN,              // BC5_UNORM
    DXGI_FORMAT_UNKNOWN,              // BC7_UNORM
    DXGI_FORMAT_UNKNOWN,              // BC6H_FLOAT
    DXGI_FORMAT_UNKNOWN,              // BC6H_UFLOAT
    DXGI_FORMAT_UNKNOWN,              // R8_SNORM
    DXGI_FORMAT_UNKNOWN,              // R8G8_SNORM
    DXGI_FORMAT_UNKNOWN,              // R8G8B8A8_SNORM
    DXGI_FORMAT_UNKNOWN,              // R16_SNORM
    DXGI_FORMAT_UNKNOWN,              // R16G16_SNORM
    DXGI_FORMAT_UNKNOWN,              // R16G16B16A16_SNORM
    DXGI_FORMAT_UNKNOWN,              // R16_FLOAT
    DXGI_FORMAT_UNKNOWN,              // R16G16_FLOAT
    DXGI_FORMAT_UNKNOWN,              // R16G16B16A16_FLOAT
    DXGI_FORMAT_UNKNOWN,              // R32_FLOAT
    DXGI_FORMAT_UNKNOWN,              // R32G32_FLOAT
    DXGI_FORMAT_UNKNOWN,              // R32G32B32A32_FLOAT
    DXGI_FORMAT_UNKNOWN,              // R11G11B10_UFLOAT
    DXGI_FORMAT_UNKNOWN,              // R8_UINT
    DXGI_FORMAT_UNKNOWN,              // R8G8_UINT
    DXGI_FORMAT_UNKNOWN,              // R8G8B8A8_UINT
    DXGI_FORMAT_UNKNOWN,              // R16_UINT
    DXGI_FORMAT_UNKNOWN,              // R16G16_UINT
    DXGI_FORMAT_UNKNOWN,              // R16G16B16A16_UINT
    DXGI_FORMAT_UNKNOWN,              // R32_UINT
    DXGI_FORMAT_UNKNOWN,              // R32G32_UINT
    DXGI_FORMAT_UNKNOWN,              // R32G32B32A32_UINT
    DXGI_FORMAT_UNKNOWN,              // R8_INT
    DXGI_FORMAT_UNKNOWN,              // R8G8_INT
    DXGI_FORMAT_UNKNOWN,              // R8G8B8A8_INT
    DXGI_FORMAT_UNKNOWN,              // R16_INT
    DXGI_FORMAT_UNKNOWN,              // R16G16_INT
    DXGI_FORMAT_UNKNOWN,              // R16G16B16A16_INT
    DXGI_FORMAT_UNKNOWN,              // R32_INT
    DXGI_FORMAT_UNKNOWN,              // R32G32_INT
    DXGI_FORMAT_UNKNOWN,              // R32G32B32A32_INT
    DXGI_FORMAT_UNKNOWN,              // R8G8B8A8_UNORM_SRGB
    DXGI_FORMAT_UNKNOWN,              // B8G8R8A8_UNORM_SRGB
    DXGI_FORMAT_UNKNOWN,              // BC1_UNORM_SRGB
    DXGI_FORMAT_UNKNOWN,              // BC2_UNORM_SRGB
    DXGI_FORMAT_UNKNOWN,              // BC3_UNORM_SRGB
    DXGI_FORMAT_UNKNOWN,              // BC7_UNORM_SRGB
    DXGI_FORMAT_D16_UNORM,            // D16_UNORM
    DXGI_FORMAT_D24_UNORM_S8_UINT,    // D24_UNORM
    DXGI_FORMAT_D32_FLOAT,            // D32_FLOAT
    DXGI_FORMAT_D24_UNORM_S8_UINT,    // D24_UNORM_S8_UINT
    DXGI_FORMAT_D32_FLOAT_S8X24_UINT, // D32_FLOAT_S8_UINT
    DXGI_FORMAT_UNKNOWN,              // ASTC_4x4_UNORM
    DXGI_FORMAT_UNKNOWN,              // ASTC_5x4_UNORM
    DXGI_FORMAT_UNKNOWN,              // ASTC_5x5_UNORM
    DXGI_FORMAT_UNKNOWN,              // ASTC_6x5_UNORM
    DXGI_FORMAT_UNKNOWN,              // ASTC_6x6_UNORM
    DXGI_FORMAT_UNKNOWN,              // ASTC_8x5_UNORM
    DXGI_FORMAT_UNKNOWN,              // ASTC_8x6_UNORM
    DXGI_FORMAT_UNKNOWN,              // ASTC_8x8_UNORM
    DXGI_FORMAT_UNKNOWN,              // ASTC_10x5_UNORM
    DXGI_FORMAT_UNKNOWN,              // ASTC_10x6_UNORM
    DXGI_FORMAT_UNKNOWN,              // ASTC_10x8_UNORM
    DXGI_FORMAT_UNKNOWN,              // ASTC_10x10_UNORM
    DXGI_FORMAT_UNKNOWN,              // ASTC_12x10_UNORM
    DXGI_FORMAT_UNKNOWN,              // ASTC_12x12_UNORM
    DXGI_FORMAT_UNKNOWN,              // ASTC_4x4_UNORM_SRGB
    DXGI_FORMAT_UNKNOWN,              // ASTC_5x4_UNORM_SRGB
    DXGI_FORMAT_UNKNOWN,              // ASTC_5x5_UNORM_SRGB
    DXGI_FORMAT_UNKNOWN,              // ASTC_6x5_UNORM_SRGB
    DXGI_FORMAT_UNKNOWN,              // ASTC_6x6_UNORM_SRGB
    DXGI_FORMAT_UNKNOWN,              // ASTC_8x5_UNORM_SRGB
    DXGI_FORMAT_UNKNOWN,              // ASTC_8x6_UNORM_SRGB
    DXGI_FORMAT_UNKNOWN,              // ASTC_8x8_UNORM_SRGB
    DXGI_FORMAT_UNKNOWN,              // ASTC_10x5_UNORM_SRGB
    DXGI_FORMAT_UNKNOWN,              // ASTC_10x6_UNORM_SRGB
    DXGI_FORMAT_UNKNOWN,              // ASTC_10x8_UNORM_SRGB
    DXGI_FORMAT_UNKNOWN,              // ASTC_10x10_UNORM_SRGB
    DXGI_FORMAT_UNKNOWN,              // ASTC_12x10_UNORM_SRGB
    DXGI_FORMAT_UNKNOWN,              // ASTC_12x12_UNORM_SRGB
    DXGI_FORMAT_UNKNOWN,              // ASTC_4x4_FLOAT
    DXGI_FORMAT_UNKNOWN,              // ASTC_5x4_FLOAT
    DXGI_FORMAT_UNKNOWN,              // ASTC_5x5_FLOAT
    DXGI_FORMAT_UNKNOWN,              // ASTC_6x5_FLOAT
    DXGI_FORMAT_UNKNOWN,              // ASTC_6x6_FLOAT
    DXGI_FORMAT_UNKNOWN,              // ASTC_8x5_FLOAT
    DXGI_FORMAT_UNKNOWN,              // ASTC_8x6_FLOAT
    DXGI_FORMAT_UNKNOWN,              // ASTC_8x8_FLOAT
    DXGI_FORMAT_UNKNOWN,              // ASTC_10x5_FLOAT
    DXGI_FORMAT_UNKNOWN,              // ASTC_10x6_FLOAT
    DXGI_FORMAT_UNKNOWN,              // ASTC_10x8_FLOAT
    DXGI_FORMAT_UNKNOWN,              // ASTC_10x10_FLOAT
    DXGI_FORMAT_UNKNOWN,              // ASTC_12x10_FLOAT
    DXGI_FORMAT_UNKNOWN,              // ASTC_12x12_FLOAT
];

/// Translation of `SDLToD3D12_TypelessFormat`.
pub(super) static SDL_TO_D3D12_TYPELESS_FORMAT: [DxgiFormat;
    TEXTUREFORMAT_MAX_ENUM_VALUE as usize] = [
    DXGI_FORMAT_UNKNOWN,           // INVALID
    DXGI_FORMAT_UNKNOWN,           // A8_UNORM
    DXGI_FORMAT_UNKNOWN,           // R8_UNORM
    DXGI_FORMAT_UNKNOWN,           // R8G8_UNORM
    DXGI_FORMAT_UNKNOWN,           // R8G8B8A8_UNORM
    DXGI_FORMAT_UNKNOWN,           // R16_UNORM
    DXGI_FORMAT_UNKNOWN,           // R16G16_UNORM
    DXGI_FORMAT_UNKNOWN,           // R16G16B16A16_UNORM
    DXGI_FORMAT_UNKNOWN,           // R10G10B10A2_UNORM
    DXGI_FORMAT_UNKNOWN,           // B5G6R5_UNORM
    DXGI_FORMAT_UNKNOWN,           // B5G5R5A1_UNORM
    DXGI_FORMAT_UNKNOWN,           // B4G4R4A4_UNORM
    DXGI_FORMAT_UNKNOWN,           // B8G8R8A8_UNORM
    DXGI_FORMAT_UNKNOWN,           // BC1_UNORM
    DXGI_FORMAT_UNKNOWN,           // BC2_UNORM
    DXGI_FORMAT_UNKNOWN,           // BC3_UNORM
    DXGI_FORMAT_UNKNOWN,           // BC4_UNORM
    DXGI_FORMAT_UNKNOWN,           // BC5_UNORM
    DXGI_FORMAT_UNKNOWN,           // BC7_UNORM
    DXGI_FORMAT_UNKNOWN,           // BC6H_FLOAT
    DXGI_FORMAT_UNKNOWN,           // BC6H_UFLOAT
    DXGI_FORMAT_UNKNOWN,           // R8_SNORM
    DXGI_FORMAT_UNKNOWN,           // R8G8_SNORM
    DXGI_FORMAT_UNKNOWN,           // R8G8B8A8_SNORM
    DXGI_FORMAT_UNKNOWN,           // R16_SNORM
    DXGI_FORMAT_UNKNOWN,           // R16G16_SNORM
    DXGI_FORMAT_UNKNOWN,           // R16G16B16A16_SNORM
    DXGI_FORMAT_UNKNOWN,           // R16_FLOAT
    DXGI_FORMAT_UNKNOWN,           // R16G16_FLOAT
    DXGI_FORMAT_UNKNOWN,           // R16G16B16A16_FLOAT
    DXGI_FORMAT_UNKNOWN,           // R32_FLOAT
    DXGI_FORMAT_UNKNOWN,           // R32G32_FLOAT
    DXGI_FORMAT_UNKNOWN,           // R32G32B32A32_FLOAT
    DXGI_FORMAT_UNKNOWN,           // R11G11B10_UFLOAT
    DXGI_FORMAT_UNKNOWN,           // R8_UINT
    DXGI_FORMAT_UNKNOWN,           // R8G8_UINT
    DXGI_FORMAT_UNKNOWN,           // R8G8B8A8_UINT
    DXGI_FORMAT_UNKNOWN,           // R16_UINT
    DXGI_FORMAT_UNKNOWN,           // R16G16_UINT
    DXGI_FORMAT_UNKNOWN,           // R16G16B16A16_UINT
    DXGI_FORMAT_UNKNOWN,           // R32_UINT
    DXGI_FORMAT_UNKNOWN,           // R32G32_UINT
    DXGI_FORMAT_UNKNOWN,           // R32G32B32A32_UINT
    DXGI_FORMAT_UNKNOWN,           // R8_INT
    DXGI_FORMAT_UNKNOWN,           // R8G8_INT
    DXGI_FORMAT_UNKNOWN,           // R8G8B8A8_INT
    DXGI_FORMAT_UNKNOWN,           // R16_INT
    DXGI_FORMAT_UNKNOWN,           // R16G16_INT
    DXGI_FORMAT_UNKNOWN,           // R16G16B16A16_INT
    DXGI_FORMAT_UNKNOWN,           // R32_INT
    DXGI_FORMAT_UNKNOWN,           // R32G32_INT
    DXGI_FORMAT_UNKNOWN,           // R32G32B32A32_INT
    DXGI_FORMAT_UNKNOWN,           // R8G8B8A8_UNORM_SRGB
    DXGI_FORMAT_UNKNOWN,           // B8G8R8A8_UNORM_SRGB
    DXGI_FORMAT_UNKNOWN,           // BC1_UNORM_SRGB
    DXGI_FORMAT_UNKNOWN,           // BC2_UNORM_SRGB
    DXGI_FORMAT_UNKNOWN,           // BC3_UNORM_SRGB
    DXGI_FORMAT_UNKNOWN,           // BC7_UNORM_SRGB
    DXGI_FORMAT_R16_TYPELESS,      // D16_UNORM
    DXGI_FORMAT_R24G8_TYPELESS,    // D24_UNORM
    DXGI_FORMAT_R32_TYPELESS,      // D32_FLOAT
    DXGI_FORMAT_R24G8_TYPELESS,    // D24_UNORM_S8_UINT
    DXGI_FORMAT_R32G8X24_TYPELESS, // D32_FLOAT_S8_UINT
    DXGI_FORMAT_UNKNOWN,           // ASTC_4x4_UNORM
    DXGI_FORMAT_UNKNOWN,           // ASTC_5x4_UNORM
    DXGI_FORMAT_UNKNOWN,           // ASTC_5x5_UNORM
    DXGI_FORMAT_UNKNOWN,           // ASTC_6x5_UNORM
    DXGI_FORMAT_UNKNOWN,           // ASTC_6x6_UNORM
    DXGI_FORMAT_UNKNOWN,           // ASTC_8x5_UNORM
    DXGI_FORMAT_UNKNOWN,           // ASTC_8x6_UNORM
    DXGI_FORMAT_UNKNOWN,           // ASTC_8x8_UNORM
    DXGI_FORMAT_UNKNOWN,           // ASTC_10x5_UNORM
    DXGI_FORMAT_UNKNOWN,           // ASTC_10x6_UNORM
    DXGI_FORMAT_UNKNOWN,           // ASTC_10x8_UNORM
    DXGI_FORMAT_UNKNOWN,           // ASTC_10x10_UNORM
    DXGI_FORMAT_UNKNOWN,           // ASTC_12x10_UNORM
    DXGI_FORMAT_UNKNOWN,           // ASTC_12x12_UNORM
    DXGI_FORMAT_UNKNOWN,           // ASTC_4x4_UNORM_SRGB
    DXGI_FORMAT_UNKNOWN,           // ASTC_5x4_UNORM_SRGB
    DXGI_FORMAT_UNKNOWN,           // ASTC_5x5_UNORM_SRGB
    DXGI_FORMAT_UNKNOWN,           // ASTC_6x5_UNORM_SRGB
    DXGI_FORMAT_UNKNOWN,           // ASTC_6x6_UNORM_SRGB
    DXGI_FORMAT_UNKNOWN,           // ASTC_8x5_UNORM_SRGB
    DXGI_FORMAT_UNKNOWN,           // ASTC_8x6_UNORM_SRGB
    DXGI_FORMAT_UNKNOWN,           // ASTC_8x8_UNORM_SRGB
    DXGI_FORMAT_UNKNOWN,           // ASTC_10x5_UNORM_SRGB
    DXGI_FORMAT_UNKNOWN,           // ASTC_10x6_UNORM_SRGB
    DXGI_FORMAT_UNKNOWN,           // ASTC_10x8_UNORM_SRGB
    DXGI_FORMAT_UNKNOWN,           // ASTC_10x10_UNORM_SRGB
    DXGI_FORMAT_UNKNOWN,           // ASTC_12x10_UNORM_SRGB
    DXGI_FORMAT_UNKNOWN,           // ASTC_12x12_UNORM_SRGB
    DXGI_FORMAT_UNKNOWN,           // ASTC_4x4_FLOAT
    DXGI_FORMAT_UNKNOWN,           // ASTC_5x4_FLOAT
    DXGI_FORMAT_UNKNOWN,           // ASTC_5x5_FLOAT
    DXGI_FORMAT_UNKNOWN,           // ASTC_6x5_FLOAT
    DXGI_FORMAT_UNKNOWN,           // ASTC_6x6_FLOAT
    DXGI_FORMAT_UNKNOWN,           // ASTC_8x5_FLOAT
    DXGI_FORMAT_UNKNOWN,           // ASTC_8x6_FLOAT
    DXGI_FORMAT_UNKNOWN,           // ASTC_8x8_FLOAT
    DXGI_FORMAT_UNKNOWN,           // ASTC_10x5_FLOAT
    DXGI_FORMAT_UNKNOWN,           // ASTC_10x6_FLOAT
    DXGI_FORMAT_UNKNOWN,           // ASTC_10x8_FLOAT
    DXGI_FORMAT_UNKNOWN,           // ASTC_10x10_FLOAT
    DXGI_FORMAT_UNKNOWN,           // ASTC_12x10_FLOAT
    DXGI_FORMAT_UNKNOWN,           // ASTC_12x12_FLOAT
];

/// `SDLToD3D12_TextureFormat[format]`.
///
/// Note (upstream): C reads past the table for a format out of range when
/// debug mode doesn't reject it first; that is `DXGI_FORMAT_UNKNOWN` here,
/// as for the other two format tables.
pub(super) fn sdl_to_d3d12_texture_format(format: TextureFormat) -> DxgiFormat {
    format_lookup(&SDL_TO_D3D12_TEXTURE_FORMAT, format)
}

/// `SDLToD3D12_DepthFormat[format]`.
pub(super) fn sdl_to_d3d12_depth_format(format: TextureFormat) -> DxgiFormat {
    format_lookup(&SDL_TO_D3D12_DEPTH_FORMAT, format)
}

/// `SDLToD3D12_TypelessFormat[format]`.
pub(super) fn sdl_to_d3d12_typeless_format(format: TextureFormat) -> DxgiFormat {
    format_lookup(&SDL_TO_D3D12_TYPELESS_FORMAT, format)
}

fn format_lookup(
    table: &[DxgiFormat; TEXTUREFORMAT_MAX_ENUM_VALUE as usize],
    format: TextureFormat,
) -> DxgiFormat {
    table
        .get(format.0 as usize)
        .copied()
        .unwrap_or(DXGI_FORMAT_UNKNOWN)
}

/// Translation of `SDLToD3D12_CompareOp`.
pub(super) static SDL_TO_D3D12_COMPARE_OP: [u32; 9] = [
    D3D12_COMPARISON_FUNC_NEVER,         // INVALID
    D3D12_COMPARISON_FUNC_NEVER,         // NEVER
    D3D12_COMPARISON_FUNC_LESS,          // LESS
    D3D12_COMPARISON_FUNC_EQUAL,         // EQUAL
    D3D12_COMPARISON_FUNC_LESS_EQUAL,    // LESS_OR_EQUAL
    D3D12_COMPARISON_FUNC_GREATER,       // GREATER
    D3D12_COMPARISON_FUNC_NOT_EQUAL,     // NOT_EQUAL
    D3D12_COMPARISON_FUNC_GREATER_EQUAL, // GREATER_OR_EQUAL
    D3D12_COMPARISON_FUNC_ALWAYS,        // ALWAYS
];

/// Translation of `SDLToD3D12_StencilOp`.
pub(super) static SDL_TO_D3D12_STENCIL_OP: [u32; 9] = [
    D3D12_STENCIL_OP_KEEP,     // INVALID
    D3D12_STENCIL_OP_KEEP,     // KEEP
    D3D12_STENCIL_OP_ZERO,     // ZERO
    D3D12_STENCIL_OP_REPLACE,  // REPLACE
    D3D12_STENCIL_OP_INCR_SAT, // INCREMENT_AND_CLAMP
    D3D12_STENCIL_OP_DECR_SAT, // DECREMENT_AND_CLAMP
    D3D12_STENCIL_OP_INVERT,   // INVERT
    D3D12_STENCIL_OP_INCR,     // INCREMENT_AND_WRAP
    D3D12_STENCIL_OP_DECR,     // DECREMENT_AND_WRAP
];

/// Translation of `SDLToD3D12_CullMode`.
pub(super) static SDL_TO_D3D12_CULL_MODE: [u32; 3] = [
    D3D12_CULL_MODE_NONE,  // NONE
    D3D12_CULL_MODE_FRONT, // FRONT
    D3D12_CULL_MODE_BACK,  // BACK
];

/// Translation of `SDLToD3D12_FillMode`.
pub(super) static SDL_TO_D3D12_FILL_MODE: [u32; 2] = [
    D3D12_FILL_MODE_SOLID,     // FILL
    D3D12_FILL_MODE_WIREFRAME, // LINE
];

/// Translation of `SDLToD3D12_InputRate`.
pub(super) static SDL_TO_D3D12_INPUT_RATE: [u32; 2] = [
    D3D12_INPUT_CLASSIFICATION_PER_VERTEX_DATA,   // VERTEX
    D3D12_INPUT_CLASSIFICATION_PER_INSTANCE_DATA, // INSTANCE
];

/// Translation of `SDLToD3D12_VertexFormat`.
pub(super) static SDL_TO_D3D12_VERTEX_FORMAT: [DxgiFormat; 31] = [
    DXGI_FORMAT_UNKNOWN,            // UNKNOWN
    DXGI_FORMAT_R32_SINT,           // INT
    DXGI_FORMAT_R32G32_SINT,        // INT2
    DXGI_FORMAT_R32G32B32_SINT,     // INT3
    DXGI_FORMAT_R32G32B32A32_SINT,  // INT4
    DXGI_FORMAT_R32_UINT,           // UINT
    DXGI_FORMAT_R32G32_UINT,        // UINT2
    DXGI_FORMAT_R32G32B32_UINT,     // UINT3
    DXGI_FORMAT_R32G32B32A32_UINT,  // UINT4
    DXGI_FORMAT_R32_FLOAT,          // FLOAT
    DXGI_FORMAT_R32G32_FLOAT,       // FLOAT2
    DXGI_FORMAT_R32G32B32_FLOAT,    // FLOAT3
    DXGI_FORMAT_R32G32B32A32_FLOAT, // FLOAT4
    DXGI_FORMAT_R8G8_SINT,          // BYTE2
    DXGI_FORMAT_R8G8B8A8_SINT,      // BYTE4
    DXGI_FORMAT_R8G8_UINT,          // UBYTE2
    DXGI_FORMAT_R8G8B8A8_UINT,      // UBYTE4
    DXGI_FORMAT_R8G8_SNORM,         // BYTE2_NORM
    DXGI_FORMAT_R8G8B8A8_SNORM,     // BYTE4_NORM
    DXGI_FORMAT_R8G8_UNORM,         // UBYTE2_NORM
    DXGI_FORMAT_R8G8B8A8_UNORM,     // UBYTE4_NORM
    DXGI_FORMAT_R16G16_SINT,        // SHORT2
    DXGI_FORMAT_R16G16B16A16_SINT,  // SHORT4
    DXGI_FORMAT_R16G16_UINT,        // USHORT2
    DXGI_FORMAT_R16G16B16A16_UINT,  // USHORT4
    DXGI_FORMAT_R16G16_SNORM,       // SHORT2_NORM
    DXGI_FORMAT_R16G16B16A16_SNORM, // SHORT4_NORM
    DXGI_FORMAT_R16G16_UNORM,       // USHORT2_NORM
    DXGI_FORMAT_R16G16B16A16_UNORM, // USHORT4_NORM
    DXGI_FORMAT_R16G16_FLOAT,       // HALF2
    DXGI_FORMAT_R16G16B16A16_FLOAT, // HALF4
];

/// Translation of `SDLToD3D12_SampleCount`.
pub(super) static SDL_TO_D3D12_SAMPLE_COUNT: [u32; 4] = [
    1, // SDL_GPU_SAMPLECOUNT_1
    2, // SDL_GPU_SAMPLECOUNT_2
    4, // SDL_GPU_SAMPLECOUNT_4
    8, // SDL_GPU_SAMPLECOUNT_8
];

/// Translation of `SDLToD3D12_PrimitiveType`.
pub(super) static SDL_TO_D3D12_PRIMITIVE_TYPE: [u32; 5] = [
    D3D_PRIMITIVE_TOPOLOGY_TRIANGLELIST,  // TRIANGLELIST
    D3D_PRIMITIVE_TOPOLOGY_TRIANGLESTRIP, // TRIANGLESTRIP
    D3D_PRIMITIVE_TOPOLOGY_LINELIST,      // LINELIST
    D3D_PRIMITIVE_TOPOLOGY_LINESTRIP,     // LINESTRIP
    D3D_PRIMITIVE_TOPOLOGY_POINTLIST,     // POINTLIST
];

/// Translation of `SDLToD3D12_PrimitiveTopologyType`.
pub(super) static SDL_TO_D3D12_PRIMITIVE_TOPOLOGY_TYPE: [u32; 5] = [
    D3D12_PRIMITIVE_TOPOLOGY_TYPE_TRIANGLE, // TRIANGLELIST
    D3D12_PRIMITIVE_TOPOLOGY_TYPE_TRIANGLE, // TRIANGLESTRIP
    D3D12_PRIMITIVE_TOPOLOGY_TYPE_LINE,     // LINELIST
    D3D12_PRIMITIVE_TOPOLOGY_TYPE_LINE,     // LINESTRIP
    D3D12_PRIMITIVE_TOPOLOGY_TYPE_POINT,    // POINTLIST
];

/// Translation of `SDLToD3D12_SamplerAddressMode`.
pub(super) static SDL_TO_D3D12_SAMPLER_ADDRESS_MODE: [u32; 3] = [
    D3D12_TEXTURE_ADDRESS_MODE_WRAP,   // REPEAT
    D3D12_TEXTURE_ADDRESS_MODE_MIRROR, // MIRRORED_REPEAT
    D3D12_TEXTURE_ADDRESS_MODE_CLAMP,  // CLAMP_TO_EDGE
];

/// `D3D12_ENCODE_BASIC_FILTER()`
const fn d3d12_encode_basic_filter(min: u32, mag: u32, mip: u32, reduction: u32) -> u32 {
    ((min & D3D12_FILTER_TYPE_MASK) << D3D12_MIN_FILTER_SHIFT)
        | ((mag & D3D12_FILTER_TYPE_MASK) << D3D12_MAG_FILTER_SHIFT)
        | ((mip & D3D12_FILTER_TYPE_MASK) << D3D12_MIP_FILTER_SHIFT)
        | ((reduction & D3D12_FILTER_REDUCTION_TYPE_MASK) << D3D12_FILTER_REDUCTION_TYPE_SHIFT)
}

/// The `D3D12_FILTER` of a sampler. Translation of `SDLToD3D12_Filter()`.
pub(super) fn sdl_to_d3d12_filter(
    min_filter: Filter,
    mag_filter: Filter,
    mipmap_mode: SamplerMipmapMode,
    comparison_enabled: bool,
    anisotropy_enabled: bool,
) -> u32 {
    let mut result = d3d12_encode_basic_filter(
        (min_filter == Filter::Linear) as u32,
        (mag_filter == Filter::Linear) as u32,
        (mipmap_mode == SamplerMipmapMode::Linear) as u32,
        comparison_enabled as u32,
    );

    if anisotropy_enabled {
        result |= D3D12_ANISOTROPIC_FILTERING_BIT;
    }

    result
}

// The lookups of the enums whose Rust enums can't be out of range.

pub(super) fn blend_factor(factor: BlendFactor) -> u32 {
    SDL_TO_D3D12_BLEND_FACTOR[factor as usize]
}

pub(super) fn blend_factor_alpha(factor: BlendFactor) -> u32 {
    SDL_TO_D3D12_BLEND_FACTOR_ALPHA[factor as usize]
}

pub(super) fn blend_op(op: BlendOp) -> u32 {
    SDL_TO_D3D12_BLEND_OP[op as usize]
}

pub(super) fn compare_op(op: CompareOp) -> u32 {
    SDL_TO_D3D12_COMPARE_OP[op as usize]
}

pub(super) fn stencil_op(op: StencilOp) -> u32 {
    SDL_TO_D3D12_STENCIL_OP[op as usize]
}

pub(super) fn cull_mode(mode: CullMode) -> u32 {
    SDL_TO_D3D12_CULL_MODE[mode as usize]
}

pub(super) fn fill_mode(mode: FillMode) -> u32 {
    SDL_TO_D3D12_FILL_MODE[mode as usize]
}

pub(super) fn input_rate(rate: VertexInputRate) -> u32 {
    SDL_TO_D3D12_INPUT_RATE[rate as usize]
}

pub(super) fn vertex_format(format: VertexElementFormat) -> DxgiFormat {
    SDL_TO_D3D12_VERTEX_FORMAT[format as usize]
}

pub(super) fn sample_count(count: SampleCount) -> u32 {
    SDL_TO_D3D12_SAMPLE_COUNT[count as usize]
}

pub(super) fn primitive_topology_type(primitive_type: PrimitiveType) -> u32 {
    SDL_TO_D3D12_PRIMITIVE_TOPOLOGY_TYPE[primitive_type as usize]
}

pub(super) fn sampler_address_mode(mode: SamplerAddressMode) -> u32 {
    SDL_TO_D3D12_SAMPLER_ADDRESS_MODE[mode as usize]
}
