// Metal, QuartzCore and CoreGraphics declarations for the Metal GPU backend
// and the Metal renderer.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! What `SDL_gpu_metal.m` (and the Metal renderer, `SDL_render_metal.m`)
//! takes from `<Metal/Metal.h>`,
//! `<QuartzCore/CoreAnimation.h>` and CoreGraphics, declared by hand: the
//! enum values (checked against the macOS SDK's headers by a C program
//! when these were written, and by the tests' tables), the structures
//! passed by value, the C functions, and `@available()`. The classes and
//! their methods are reached through the Objective-C runtime
//! (`core::apple::objc`).

use std::ffi::c_void;

use crate::core::apple::block::DispatchQueue;
use crate::core::apple::foundation::CFStringRef;
use crate::core::apple::objc::{Id, NSInteger, NSUInteger};

// MTLPixelFormat
pub(crate) const MTL_PIXEL_FORMAT_INVALID: NSUInteger = 0;
pub(crate) const MTL_PIXEL_FORMAT_A8_UNORM: NSUInteger = 1;
pub(crate) const MTL_PIXEL_FORMAT_R8_UNORM: NSUInteger = 10;
pub(crate) const MTL_PIXEL_FORMAT_R8_SNORM: NSUInteger = 12;
pub(crate) const MTL_PIXEL_FORMAT_R8_UINT: NSUInteger = 13;
pub(crate) const MTL_PIXEL_FORMAT_R8_SINT: NSUInteger = 14;
pub(crate) const MTL_PIXEL_FORMAT_R16_UNORM: NSUInteger = 20;
pub(crate) const MTL_PIXEL_FORMAT_R16_SNORM: NSUInteger = 22;
pub(crate) const MTL_PIXEL_FORMAT_R16_UINT: NSUInteger = 23;
pub(crate) const MTL_PIXEL_FORMAT_R16_SINT: NSUInteger = 24;
pub(crate) const MTL_PIXEL_FORMAT_R16_FLOAT: NSUInteger = 25;
pub(crate) const MTL_PIXEL_FORMAT_RG8_UNORM: NSUInteger = 30;
pub(crate) const MTL_PIXEL_FORMAT_RG8_SNORM: NSUInteger = 32;
pub(crate) const MTL_PIXEL_FORMAT_RG8_UINT: NSUInteger = 33;
pub(crate) const MTL_PIXEL_FORMAT_RG8_SINT: NSUInteger = 34;
pub(crate) const MTL_PIXEL_FORMAT_B5G6R5_UNORM: NSUInteger = 40;
pub(crate) const MTL_PIXEL_FORMAT_A1BGR5_UNORM: NSUInteger = 41;
pub(crate) const MTL_PIXEL_FORMAT_ABGR4_UNORM: NSUInteger = 42;
pub(crate) const MTL_PIXEL_FORMAT_BGR5A1_UNORM: NSUInteger = 43;
pub(crate) const MTL_PIXEL_FORMAT_R32_UINT: NSUInteger = 53;
pub(crate) const MTL_PIXEL_FORMAT_R32_SINT: NSUInteger = 54;
pub(crate) const MTL_PIXEL_FORMAT_R32_FLOAT: NSUInteger = 55;
pub(crate) const MTL_PIXEL_FORMAT_RG16_UNORM: NSUInteger = 60;
pub(crate) const MTL_PIXEL_FORMAT_RG16_SNORM: NSUInteger = 62;
pub(crate) const MTL_PIXEL_FORMAT_RG16_UINT: NSUInteger = 63;
pub(crate) const MTL_PIXEL_FORMAT_RG16_SINT: NSUInteger = 64;
pub(crate) const MTL_PIXEL_FORMAT_RG16_FLOAT: NSUInteger = 65;
pub(crate) const MTL_PIXEL_FORMAT_RGBA8_UNORM: NSUInteger = 70;
pub(crate) const MTL_PIXEL_FORMAT_RGBA8_UNORM_SRGB: NSUInteger = 71;
pub(crate) const MTL_PIXEL_FORMAT_RGBA8_SNORM: NSUInteger = 72;
pub(crate) const MTL_PIXEL_FORMAT_RGBA8_UINT: NSUInteger = 73;
pub(crate) const MTL_PIXEL_FORMAT_RGBA8_SINT: NSUInteger = 74;
pub(crate) const MTL_PIXEL_FORMAT_BGRA8_UNORM: NSUInteger = 80;
pub(crate) const MTL_PIXEL_FORMAT_BGRA8_UNORM_SRGB: NSUInteger = 81;
pub(crate) const MTL_PIXEL_FORMAT_RGB10A2_UNORM: NSUInteger = 90;
pub(crate) const MTL_PIXEL_FORMAT_RG11B10_FLOAT: NSUInteger = 92;
pub(crate) const MTL_PIXEL_FORMAT_RG32_UINT: NSUInteger = 103;
pub(crate) const MTL_PIXEL_FORMAT_RG32_SINT: NSUInteger = 104;
pub(crate) const MTL_PIXEL_FORMAT_RG32_FLOAT: NSUInteger = 105;
pub(crate) const MTL_PIXEL_FORMAT_RGBA16_UNORM: NSUInteger = 110;
pub(crate) const MTL_PIXEL_FORMAT_RGBA16_SNORM: NSUInteger = 112;
pub(crate) const MTL_PIXEL_FORMAT_RGBA16_UINT: NSUInteger = 113;
pub(crate) const MTL_PIXEL_FORMAT_RGBA16_SINT: NSUInteger = 114;
pub(crate) const MTL_PIXEL_FORMAT_RGBA16_FLOAT: NSUInteger = 115;
pub(crate) const MTL_PIXEL_FORMAT_RGBA32_UINT: NSUInteger = 123;
pub(crate) const MTL_PIXEL_FORMAT_RGBA32_SINT: NSUInteger = 124;
pub(crate) const MTL_PIXEL_FORMAT_RGBA32_FLOAT: NSUInteger = 125;
pub(crate) const MTL_PIXEL_FORMAT_BC1_RGBA: NSUInteger = 130;
pub(crate) const MTL_PIXEL_FORMAT_BC1_RGBA_SRGB: NSUInteger = 131;
pub(crate) const MTL_PIXEL_FORMAT_BC2_RGBA: NSUInteger = 132;
pub(crate) const MTL_PIXEL_FORMAT_BC2_RGBA_SRGB: NSUInteger = 133;
pub(crate) const MTL_PIXEL_FORMAT_BC3_RGBA: NSUInteger = 134;
pub(crate) const MTL_PIXEL_FORMAT_BC3_RGBA_SRGB: NSUInteger = 135;
pub(crate) const MTL_PIXEL_FORMAT_BC4_R_UNORM: NSUInteger = 140;
pub(crate) const MTL_PIXEL_FORMAT_BC5_RG_UNORM: NSUInteger = 142;
pub(crate) const MTL_PIXEL_FORMAT_BC6H_RGB_FLOAT: NSUInteger = 150;
pub(crate) const MTL_PIXEL_FORMAT_BC6H_RGB_UFLOAT: NSUInteger = 151;
pub(crate) const MTL_PIXEL_FORMAT_BC7_RGBA_UNORM: NSUInteger = 152;
pub(crate) const MTL_PIXEL_FORMAT_BC7_RGBA_UNORM_SRGB: NSUInteger = 153;
pub(crate) const MTL_PIXEL_FORMAT_ASTC_4X4_SRGB: NSUInteger = 186;
pub(crate) const MTL_PIXEL_FORMAT_ASTC_5X4_SRGB: NSUInteger = 187;
pub(crate) const MTL_PIXEL_FORMAT_ASTC_5X5_SRGB: NSUInteger = 188;
pub(crate) const MTL_PIXEL_FORMAT_ASTC_6X5_SRGB: NSUInteger = 189;
pub(crate) const MTL_PIXEL_FORMAT_ASTC_6X6_SRGB: NSUInteger = 190;
pub(crate) const MTL_PIXEL_FORMAT_ASTC_8X5_SRGB: NSUInteger = 192;
pub(crate) const MTL_PIXEL_FORMAT_ASTC_8X6_SRGB: NSUInteger = 193;
pub(crate) const MTL_PIXEL_FORMAT_ASTC_8X8_SRGB: NSUInteger = 194;
pub(crate) const MTL_PIXEL_FORMAT_ASTC_10X5_SRGB: NSUInteger = 195;
pub(crate) const MTL_PIXEL_FORMAT_ASTC_10X6_SRGB: NSUInteger = 196;
pub(crate) const MTL_PIXEL_FORMAT_ASTC_10X8_SRGB: NSUInteger = 197;
pub(crate) const MTL_PIXEL_FORMAT_ASTC_10X10_SRGB: NSUInteger = 198;
pub(crate) const MTL_PIXEL_FORMAT_ASTC_12X10_SRGB: NSUInteger = 199;
pub(crate) const MTL_PIXEL_FORMAT_ASTC_12X12_SRGB: NSUInteger = 200;
pub(crate) const MTL_PIXEL_FORMAT_ASTC_4X4_LDR: NSUInteger = 204;
pub(crate) const MTL_PIXEL_FORMAT_ASTC_5X4_LDR: NSUInteger = 205;
pub(crate) const MTL_PIXEL_FORMAT_ASTC_5X5_LDR: NSUInteger = 206;
pub(crate) const MTL_PIXEL_FORMAT_ASTC_6X5_LDR: NSUInteger = 207;
pub(crate) const MTL_PIXEL_FORMAT_ASTC_6X6_LDR: NSUInteger = 208;
pub(crate) const MTL_PIXEL_FORMAT_ASTC_8X5_LDR: NSUInteger = 210;
pub(crate) const MTL_PIXEL_FORMAT_ASTC_8X6_LDR: NSUInteger = 211;
pub(crate) const MTL_PIXEL_FORMAT_ASTC_8X8_LDR: NSUInteger = 212;
pub(crate) const MTL_PIXEL_FORMAT_ASTC_10X5_LDR: NSUInteger = 213;
pub(crate) const MTL_PIXEL_FORMAT_ASTC_10X6_LDR: NSUInteger = 214;
pub(crate) const MTL_PIXEL_FORMAT_ASTC_10X8_LDR: NSUInteger = 215;
pub(crate) const MTL_PIXEL_FORMAT_ASTC_10X10_LDR: NSUInteger = 216;
pub(crate) const MTL_PIXEL_FORMAT_ASTC_12X10_LDR: NSUInteger = 217;
pub(crate) const MTL_PIXEL_FORMAT_ASTC_12X12_LDR: NSUInteger = 218;
pub(crate) const MTL_PIXEL_FORMAT_ASTC_4X4_HDR: NSUInteger = 222;
pub(crate) const MTL_PIXEL_FORMAT_ASTC_5X4_HDR: NSUInteger = 223;
pub(crate) const MTL_PIXEL_FORMAT_ASTC_5X5_HDR: NSUInteger = 224;
pub(crate) const MTL_PIXEL_FORMAT_ASTC_6X5_HDR: NSUInteger = 225;
pub(crate) const MTL_PIXEL_FORMAT_ASTC_6X6_HDR: NSUInteger = 226;
pub(crate) const MTL_PIXEL_FORMAT_ASTC_8X5_HDR: NSUInteger = 228;
pub(crate) const MTL_PIXEL_FORMAT_ASTC_8X6_HDR: NSUInteger = 229;
pub(crate) const MTL_PIXEL_FORMAT_ASTC_8X8_HDR: NSUInteger = 230;
pub(crate) const MTL_PIXEL_FORMAT_ASTC_10X5_HDR: NSUInteger = 231;
pub(crate) const MTL_PIXEL_FORMAT_ASTC_10X6_HDR: NSUInteger = 232;
pub(crate) const MTL_PIXEL_FORMAT_ASTC_10X8_HDR: NSUInteger = 233;
pub(crate) const MTL_PIXEL_FORMAT_ASTC_10X10_HDR: NSUInteger = 234;
pub(crate) const MTL_PIXEL_FORMAT_ASTC_12X10_HDR: NSUInteger = 235;
pub(crate) const MTL_PIXEL_FORMAT_ASTC_12X12_HDR: NSUInteger = 236;
pub(crate) const MTL_PIXEL_FORMAT_DEPTH16_UNORM: NSUInteger = 250;
pub(crate) const MTL_PIXEL_FORMAT_DEPTH32_FLOAT: NSUInteger = 252;
pub(crate) const MTL_PIXEL_FORMAT_DEPTH24_UNORM_STENCIL8: NSUInteger = 255;
pub(crate) const MTL_PIXEL_FORMAT_DEPTH32_FLOAT_STENCIL8: NSUInteger = 260;

// MTLVertexFormat
pub(crate) const MTL_VERTEX_FORMAT_INVALID: NSUInteger = 0;
pub(crate) const MTL_VERTEX_FORMAT_UCHAR2: NSUInteger = 1;
pub(crate) const MTL_VERTEX_FORMAT_UCHAR4: NSUInteger = 3;
pub(crate) const MTL_VERTEX_FORMAT_CHAR2: NSUInteger = 4;
pub(crate) const MTL_VERTEX_FORMAT_CHAR4: NSUInteger = 6;
pub(crate) const MTL_VERTEX_FORMAT_UCHAR2_NORMALIZED: NSUInteger = 7;
pub(crate) const MTL_VERTEX_FORMAT_UCHAR4_NORMALIZED: NSUInteger = 9;
pub(crate) const MTL_VERTEX_FORMAT_CHAR2_NORMALIZED: NSUInteger = 10;
pub(crate) const MTL_VERTEX_FORMAT_CHAR4_NORMALIZED: NSUInteger = 12;
pub(crate) const MTL_VERTEX_FORMAT_USHORT2: NSUInteger = 13;
pub(crate) const MTL_VERTEX_FORMAT_USHORT4: NSUInteger = 15;
pub(crate) const MTL_VERTEX_FORMAT_SHORT2: NSUInteger = 16;
pub(crate) const MTL_VERTEX_FORMAT_SHORT4: NSUInteger = 18;
pub(crate) const MTL_VERTEX_FORMAT_USHORT2_NORMALIZED: NSUInteger = 19;
pub(crate) const MTL_VERTEX_FORMAT_USHORT4_NORMALIZED: NSUInteger = 21;
pub(crate) const MTL_VERTEX_FORMAT_SHORT2_NORMALIZED: NSUInteger = 22;
pub(crate) const MTL_VERTEX_FORMAT_SHORT4_NORMALIZED: NSUInteger = 24;
pub(crate) const MTL_VERTEX_FORMAT_HALF2: NSUInteger = 25;
pub(crate) const MTL_VERTEX_FORMAT_HALF4: NSUInteger = 27;
pub(crate) const MTL_VERTEX_FORMAT_FLOAT: NSUInteger = 28;
pub(crate) const MTL_VERTEX_FORMAT_FLOAT2: NSUInteger = 29;
pub(crate) const MTL_VERTEX_FORMAT_FLOAT3: NSUInteger = 30;
pub(crate) const MTL_VERTEX_FORMAT_FLOAT4: NSUInteger = 31;
pub(crate) const MTL_VERTEX_FORMAT_INT: NSUInteger = 32;
pub(crate) const MTL_VERTEX_FORMAT_INT2: NSUInteger = 33;
pub(crate) const MTL_VERTEX_FORMAT_INT3: NSUInteger = 34;
pub(crate) const MTL_VERTEX_FORMAT_INT4: NSUInteger = 35;
pub(crate) const MTL_VERTEX_FORMAT_UINT: NSUInteger = 36;
pub(crate) const MTL_VERTEX_FORMAT_UINT2: NSUInteger = 37;
pub(crate) const MTL_VERTEX_FORMAT_UINT3: NSUInteger = 38;
pub(crate) const MTL_VERTEX_FORMAT_UINT4: NSUInteger = 39;

// MTLIndexType
pub(crate) const MTL_INDEX_TYPE_UINT16: NSUInteger = 0;
pub(crate) const MTL_INDEX_TYPE_UINT32: NSUInteger = 1;

// MTLPrimitiveType
pub(crate) const MTL_PRIMITIVE_TYPE_POINT: NSUInteger = 0;
pub(crate) const MTL_PRIMITIVE_TYPE_LINE: NSUInteger = 1;
pub(crate) const MTL_PRIMITIVE_TYPE_LINE_STRIP: NSUInteger = 2;
pub(crate) const MTL_PRIMITIVE_TYPE_TRIANGLE: NSUInteger = 3;
pub(crate) const MTL_PRIMITIVE_TYPE_TRIANGLE_STRIP: NSUInteger = 4;

// MTLTriangleFillMode
pub(crate) const MTL_TRIANGLE_FILL_MODE_FILL: NSUInteger = 0;
pub(crate) const MTL_TRIANGLE_FILL_MODE_LINES: NSUInteger = 1;

// MTLCullMode
pub(crate) const MTL_CULL_MODE_NONE: NSUInteger = 0;
pub(crate) const MTL_CULL_MODE_FRONT: NSUInteger = 1;
pub(crate) const MTL_CULL_MODE_BACK: NSUInteger = 2;

// MTLWinding
pub(crate) const MTL_WINDING_CLOCKWISE: NSUInteger = 0;
pub(crate) const MTL_WINDING_COUNTER_CLOCKWISE: NSUInteger = 1;

// MTLBlendFactor
pub(crate) const MTL_BLEND_FACTOR_ZERO: NSUInteger = 0;
pub(crate) const MTL_BLEND_FACTOR_ONE: NSUInteger = 1;
pub(crate) const MTL_BLEND_FACTOR_SOURCE_COLOR: NSUInteger = 2;
pub(crate) const MTL_BLEND_FACTOR_ONE_MINUS_SOURCE_COLOR: NSUInteger = 3;
pub(crate) const MTL_BLEND_FACTOR_SOURCE_ALPHA: NSUInteger = 4;
pub(crate) const MTL_BLEND_FACTOR_ONE_MINUS_SOURCE_ALPHA: NSUInteger = 5;
pub(crate) const MTL_BLEND_FACTOR_DESTINATION_COLOR: NSUInteger = 6;
pub(crate) const MTL_BLEND_FACTOR_ONE_MINUS_DESTINATION_COLOR: NSUInteger = 7;
pub(crate) const MTL_BLEND_FACTOR_DESTINATION_ALPHA: NSUInteger = 8;
pub(crate) const MTL_BLEND_FACTOR_ONE_MINUS_DESTINATION_ALPHA: NSUInteger = 9;
pub(crate) const MTL_BLEND_FACTOR_SOURCE_ALPHA_SATURATED: NSUInteger = 10;
pub(crate) const MTL_BLEND_FACTOR_BLEND_COLOR: NSUInteger = 11;
pub(crate) const MTL_BLEND_FACTOR_ONE_MINUS_BLEND_COLOR: NSUInteger = 12;

// MTLBlendOperation
pub(crate) const MTL_BLEND_OPERATION_ADD: NSUInteger = 0;
pub(crate) const MTL_BLEND_OPERATION_SUBTRACT: NSUInteger = 1;
pub(crate) const MTL_BLEND_OPERATION_REVERSE_SUBTRACT: NSUInteger = 2;
pub(crate) const MTL_BLEND_OPERATION_MIN: NSUInteger = 3;
pub(crate) const MTL_BLEND_OPERATION_MAX: NSUInteger = 4;

// MTLCompareFunction
pub(crate) const MTL_COMPARE_FUNCTION_NEVER: NSUInteger = 0;
pub(crate) const MTL_COMPARE_FUNCTION_LESS: NSUInteger = 1;
pub(crate) const MTL_COMPARE_FUNCTION_EQUAL: NSUInteger = 2;
pub(crate) const MTL_COMPARE_FUNCTION_LESS_EQUAL: NSUInteger = 3;
pub(crate) const MTL_COMPARE_FUNCTION_GREATER: NSUInteger = 4;
pub(crate) const MTL_COMPARE_FUNCTION_NOT_EQUAL: NSUInteger = 5;
pub(crate) const MTL_COMPARE_FUNCTION_GREATER_EQUAL: NSUInteger = 6;
pub(crate) const MTL_COMPARE_FUNCTION_ALWAYS: NSUInteger = 7;

// MTLStencilOperation
pub(crate) const MTL_STENCIL_OPERATION_KEEP: NSUInteger = 0;
pub(crate) const MTL_STENCIL_OPERATION_ZERO: NSUInteger = 1;
pub(crate) const MTL_STENCIL_OPERATION_REPLACE: NSUInteger = 2;
pub(crate) const MTL_STENCIL_OPERATION_INCREMENT_CLAMP: NSUInteger = 3;
pub(crate) const MTL_STENCIL_OPERATION_DECREMENT_CLAMP: NSUInteger = 4;
pub(crate) const MTL_STENCIL_OPERATION_INVERT: NSUInteger = 5;
pub(crate) const MTL_STENCIL_OPERATION_INCREMENT_WRAP: NSUInteger = 6;
pub(crate) const MTL_STENCIL_OPERATION_DECREMENT_WRAP: NSUInteger = 7;

// MTLSamplerAddressMode
pub(crate) const MTL_SAMPLER_ADDRESS_MODE_CLAMP_TO_EDGE: NSUInteger = 0;
pub(crate) const MTL_SAMPLER_ADDRESS_MODE_REPEAT: NSUInteger = 2;
pub(crate) const MTL_SAMPLER_ADDRESS_MODE_MIRROR_REPEAT: NSUInteger = 3;

// MTLSamplerMinMagFilter
pub(crate) const MTL_SAMPLER_MIN_MAG_FILTER_NEAREST: NSUInteger = 0;
pub(crate) const MTL_SAMPLER_MIN_MAG_FILTER_LINEAR: NSUInteger = 1;

// MTLSamplerMipFilter
pub(crate) const MTL_SAMPLER_MIP_FILTER_NEAREST: NSUInteger = 1;
pub(crate) const MTL_SAMPLER_MIP_FILTER_LINEAR: NSUInteger = 2;

// MTLLoadAction
pub(crate) const MTL_LOAD_ACTION_DONT_CARE: NSUInteger = 0;
pub(crate) const MTL_LOAD_ACTION_LOAD: NSUInteger = 1;
pub(crate) const MTL_LOAD_ACTION_CLEAR: NSUInteger = 2;

// MTLStoreAction
pub(crate) const MTL_STORE_ACTION_DONT_CARE: NSUInteger = 0;
pub(crate) const MTL_STORE_ACTION_STORE: NSUInteger = 1;
pub(crate) const MTL_STORE_ACTION_MULTISAMPLE_RESOLVE: NSUInteger = 2;
pub(crate) const MTL_STORE_ACTION_STORE_AND_MULTISAMPLE_RESOLVE: NSUInteger = 3;

// MTLVertexStepFunction
pub(crate) const MTL_VERTEX_STEP_FUNCTION_PER_VERTEX: NSUInteger = 1;
pub(crate) const MTL_VERTEX_STEP_FUNCTION_PER_INSTANCE: NSUInteger = 2;

// MTLTextureType
pub(crate) const MTL_TEXTURE_TYPE_2D: NSUInteger = 2;
pub(crate) const MTL_TEXTURE_TYPE_2D_ARRAY: NSUInteger = 3;
pub(crate) const MTL_TEXTURE_TYPE_2D_MULTISAMPLE: NSUInteger = 4;
pub(crate) const MTL_TEXTURE_TYPE_CUBE: NSUInteger = 5;
pub(crate) const MTL_TEXTURE_TYPE_CUBE_ARRAY: NSUInteger = 6;
pub(crate) const MTL_TEXTURE_TYPE_3D: NSUInteger = 7;

// MTLColorWriteMask
pub(crate) const MTL_COLOR_WRITE_MASK_ALPHA: NSUInteger = 1;
pub(crate) const MTL_COLOR_WRITE_MASK_BLUE: NSUInteger = 2;
pub(crate) const MTL_COLOR_WRITE_MASK_GREEN: NSUInteger = 4;
pub(crate) const MTL_COLOR_WRITE_MASK_RED: NSUInteger = 8;

// MTLDepthClipMode (no depth clip mode on visionOS, where it's always
// clipped)
#[cfg(not(target_os = "visionos"))]
pub(crate) const MTL_DEPTH_CLIP_MODE_CLIP: NSUInteger = 0;
#[cfg(not(target_os = "visionos"))]
pub(crate) const MTL_DEPTH_CLIP_MODE_CLAMP: NSUInteger = 1;

// MTLStorageMode
#[cfg(target_os = "macos")]
pub(crate) const MTL_STORAGE_MODE_MANAGED: NSUInteger = 1;
pub(crate) const MTL_STORAGE_MODE_PRIVATE: NSUInteger = 2;

// MTLTextureUsage
pub(crate) const MTL_TEXTURE_USAGE_SHADER_READ: NSUInteger = 1;
pub(crate) const MTL_TEXTURE_USAGE_SHADER_WRITE: NSUInteger = 2;
pub(crate) const MTL_TEXTURE_USAGE_RENDER_TARGET: NSUInteger = 4;

// MTLResourceOptions
pub(crate) const MTL_RESOURCE_CPU_CACHE_MODE_DEFAULT_CACHE: NSUInteger = 0;
pub(crate) const MTL_RESOURCE_CPU_CACHE_MODE_WRITE_COMBINED: NSUInteger = 1;
pub(crate) const MTL_RESOURCE_STORAGE_MODE_SHARED: NSUInteger = 0;
pub(crate) const MTL_RESOURCE_STORAGE_MODE_PRIVATE: NSUInteger = 32;

// MTLTextureSwizzle
pub(crate) const MTL_TEXTURE_SWIZZLE_RED: u8 = 2;
pub(crate) const MTL_TEXTURE_SWIZZLE_GREEN: u8 = 3;
pub(crate) const MTL_TEXTURE_SWIZZLE_BLUE: u8 = 4;
pub(crate) const MTL_TEXTURE_SWIZZLE_ALPHA: u8 = 5;

// MTLPipelineOption
pub(crate) const MTL_PIPELINE_OPTION_NONE: NSUInteger = 0;

// MTLCommandBufferStatus
pub(crate) const MTL_COMMAND_BUFFER_STATUS_COMMITTED: NSUInteger = 2;
pub(crate) const MTL_COMMAND_BUFFER_STATUS_SCHEDULED: NSUInteger = 3;

// MTLGPUFamily
pub(crate) const MTL_GPU_FAMILY_APPLE1: NSInteger = 1001;
// (the other UIKit platforms' device check; visionOS's tests')
#[cfg(any(
    not(any(target_os = "macos", target_os = "visionos")),
    all(test, target_os = "visionos")
))]
pub(crate) const MTL_GPU_FAMILY_APPLE3: NSInteger = 1003;
pub(crate) const MTL_GPU_FAMILY_APPLE4: NSInteger = 1004;
pub(crate) const MTL_GPU_FAMILY_APPLE6: NSInteger = 1006;
pub(crate) const MTL_GPU_FAMILY_APPLE7: NSInteger = 1007;
#[cfg(target_os = "macos")]
pub(crate) const MTL_GPU_FAMILY_MAC1: NSInteger = 2001;
#[cfg(target_os = "macos")]
pub(crate) const MTL_GPU_FAMILY_MAC2: NSInteger = 2002;
pub(crate) const MTL_GPU_FAMILY_COMMON2: NSInteger = 3002;

// MTLFeatureSet
#[cfg(target_os = "macos")]
pub(crate) const MTL_FEATURE_SET_MACOS_GPU_FAMILY1_V4: NSUInteger = 10004;
#[cfg(target_os = "macos")]
pub(crate) const MTL_FEATURE_SET_MACOS_GPU_FAMILY2_V1: NSUInteger = 10005;
// (the Metal renderer's, on the UIKit platforms)
#[cfg(not(any(target_os = "macos", target_abi = "macabi", target_os = "tvos")))]
pub(crate) const MTL_FEATURE_SET_IOS_GPU_FAMILY1_V2: NSUInteger = 2;
#[cfg(not(any(target_os = "macos", target_abi = "macabi", target_os = "tvos")))]
pub(crate) const MTL_FEATURE_SET_IOS_GPU_FAMILY2_V2: NSUInteger = 3;
#[cfg(not(any(target_os = "macos", target_abi = "macabi", target_os = "tvos")))]
pub(crate) const MTL_FEATURE_SET_IOS_GPU_FAMILY3_V1: NSUInteger = 4;
#[cfg(not(any(target_os = "macos", target_abi = "macabi", target_os = "tvos")))]
pub(crate) const MTL_FEATURE_SET_IOS_GPU_FAMILY4_V1: NSUInteger = 11;
#[cfg(target_os = "tvos")]
pub(crate) const MTL_FEATURE_SET_TVOS_GPU_FAMILY2_V1: NSUInteger = 30003;

/// `MTLOrigin`.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct MTLOrigin {
    pub(crate) x: NSUInteger,
    pub(crate) y: NSUInteger,
    pub(crate) z: NSUInteger,
}

/// `MTLOriginMake()`.
pub(crate) fn mtl_origin_make(x: u32, y: u32, z: u32) -> MTLOrigin {
    MTLOrigin {
        x: x as NSUInteger,
        y: y as NSUInteger,
        z: z as NSUInteger,
    }
}

/// `MTLSize`.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct MTLSize {
    pub(crate) width: NSUInteger,
    pub(crate) height: NSUInteger,
    pub(crate) depth: NSUInteger,
}

/// `MTLSizeMake()`.
pub(crate) fn mtl_size_make(width: u32, height: u32, depth: u32) -> MTLSize {
    MTLSize {
        width: width as NSUInteger,
        height: height as NSUInteger,
        depth: depth as NSUInteger,
    }
}

/// `MTLRegion`.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct MTLRegion {
    pub(crate) origin: MTLOrigin,
    pub(crate) size: MTLSize,
}

/// `MTLRegionMake2D()`.
pub(crate) fn mtl_region_make_2d(x: i32, y: i32, width: i32, height: i32) -> MTLRegion {
    MTLRegion {
        origin: MTLOrigin {
            x: x as NSUInteger,
            y: y as NSUInteger,
            z: 0,
        },
        size: MTLSize {
            width: width as NSUInteger,
            height: height as NSUInteger,
            depth: 1,
        },
    }
}

/// `MTLViewport`.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) struct MTLViewport {
    pub(crate) origin_x: f64,
    pub(crate) origin_y: f64,
    pub(crate) width: f64,
    pub(crate) height: f64,
    pub(crate) znear: f64,
    pub(crate) zfar: f64,
}

/// `MTLScissorRect`.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct MTLScissorRect {
    pub(crate) x: NSUInteger,
    pub(crate) y: NSUInteger,
    pub(crate) width: NSUInteger,
    pub(crate) height: NSUInteger,
}

/// `MTLClearColor`.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) struct MTLClearColor {
    pub(crate) red: f64,
    pub(crate) green: f64,
    pub(crate) blue: f64,
    pub(crate) alpha: f64,
}

/// `MTLTextureSwizzleChannels` (`MTLTextureSwizzleChannelsMake()` is the
/// struct literal).
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct MTLTextureSwizzleChannels {
    pub(crate) red: u8,
    pub(crate) green: u8,
    pub(crate) blue: u8,
    pub(crate) alpha: u8,
}

/// `CGColorSpaceRef`.
pub(crate) type CGColorSpaceRef = *mut c_void;

/// `dispatch_data_t`.
pub(crate) type DispatchData = *mut c_void;

/// `DISPATCH_DATA_DESTRUCTOR_DEFAULT`: the data is copied.
pub(crate) const DISPATCH_DATA_DESTRUCTOR_DEFAULT: *const c_void = std::ptr::null();

#[link(name = "Metal", kind = "framework")]
unsafe extern "C" {
    /// Returns a `+1` device (`NS_RETURNS_RETAINED`), or nil.
    pub(crate) fn MTLCreateSystemDefaultDevice() -> Id;
    /// Returns a `+1` `NSArray` of the devices (macOS only).
    #[cfg(target_os = "macos")]
    pub(crate) fn MTLCopyAllDevices() -> Id;
}

// CAMetalLayer is QuartzCore's; its class is looked up by name.
#[link(name = "QuartzCore", kind = "framework")]
unsafe extern "C" {}

#[link(name = "CoreGraphics", kind = "framework")]
unsafe extern "C" {
    pub(crate) static kCGColorSpaceSRGB: CFStringRef;
    pub(crate) static kCGColorSpaceExtendedLinearSRGB: CFStringRef;
    pub(crate) static kCGColorSpaceITUR_2100_PQ: CFStringRef;
    pub(crate) fn CGColorSpaceCreateWithName(name: CFStringRef) -> CGColorSpaceRef;
    pub(crate) fn CGColorSpaceRelease(space: CGColorSpaceRef);
}

#[link(name = "System", kind = "dylib")]
unsafe extern "C" {
    /// `dispatch_data_create()`: with `DISPATCH_DATA_DESTRUCTOR_DEFAULT`,
    /// the bytes are copied.
    pub(crate) fn dispatch_data_create(
        buffer: *const c_void,
        size: usize,
        queue: DispatchQueue,
        destructor: *const c_void,
    ) -> DispatchData;
}
