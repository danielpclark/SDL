// Rust translation of src/gpu/SDL_sysgpu.h (and the backend helpers of
// src/gpu/SDL_gpu.c) from Simple DirectMedia Layer.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! The internal structures of the GPU API and the interface between its
//! front end ([`crate::gpu`]) and its backends.
//!
//! A backend is a [`GpuBootstrap`] (`SDL_GPUBootstrap`) in the front end's
//! driver list, whose `create_device` returns a [`GpuDriver`]: the function
//! table of `struct SDL_GPUDevice` over the backend's own state (upstream's
//! `SDL_GPURenderer` driver data). The front end does the parameter checks
//! and the debug-mode validation, then calls the driver.
//!
//! The objects a backend creates (textures, buffers, pipelines, fences...)
//! are [`BackendObject`]s: reference-counted `Any` values the front-end
//! handles hold and pass back to the driver, which downcasts them to its own
//! types (upstream casts `SDL_GPUTexture *` to its `VulkanTextureContainer
//! *`). A backend that must keep an object alive while the GPU uses it
//! clones the `Arc` (upstream's reference counts). Command buffers are
//! uniquely owned ([`BackendCommandBuffer`]): the driver hands one out in
//! `acquire_command_buffer`, records into it, and gets it back in `submit`
//! or `cancel`.
//!
//! The binding and region structures the driver receives are the public
//! ones, which name front-end handles; a backend reads their `raw` objects
//! and their creation info (`texture.info`, upstream's
//! `TextureCommonHeader`).

use std::any::Any;
use std::ptr::NonNull;
use std::sync::Arc;

use super::{
    BlitInfo, BufferBinding, BufferLocation, BufferRegion, BufferUsageFlags,
    ColorTargetDescription, ColorTargetInfo, CommandBuffer, ComputePipelineCreateInfo,
    DepthStencilTargetInfo, DeviceShared, Filter, GraphicsPipeline, GraphicsPipelineCreateInfo,
    GraphicsPipelineTargetInfo, IndexElementSize, MultisampleState, PresentMode, PrimitiveType,
    RasterizerState, SampleCount, Sampler, SamplerCreateInfo, Shader, ShaderCreateInfo,
    ShaderFormat, StorageBufferReadWriteBinding, StorageTextureReadWriteBinding, StoreOp,
    SwapchainComposition, Texture, TextureCreateInfo, TextureFormat, TextureLocation,
    TextureRegion, TextureSamplerBinding, TextureTransferInfo, TextureType, TextureUsageFlags,
    TransferBufferLocation, TransferBufferUsage, Viewport,
};
use crate::error::{Error, Result};
use crate::properties::Properties;
use crate::video::sysvideo::VideoDriver;
use crate::video::{FColor, FlipMode, Rect, Window};

// GraphicsDevice Limits

pub(crate) const MAX_TEXTURE_SAMPLERS_PER_STAGE: u32 = 16;
pub(crate) const MAX_STORAGE_TEXTURES_PER_STAGE: u32 = 8;
pub(crate) const MAX_STORAGE_BUFFERS_PER_STAGE: u32 = 8;
pub(crate) const MAX_UNIFORM_BUFFERS_PER_STAGE: u32 = 4;
pub(crate) const MAX_COMPUTE_WRITE_TEXTURES: u32 = 8;
pub(crate) const MAX_COMPUTE_WRITE_BUFFERS: u32 = 8;
pub(crate) const UNIFORM_BUFFER_SIZE: u32 = 32768;
pub(crate) const MAX_VERTEX_BUFFERS: u32 = 16;
pub(crate) const MAX_VERTEX_ATTRIBUTES: u32 = 16;
pub(crate) const MAX_COLOR_TARGET_BINDINGS: u32 = 8;
pub(crate) const MAX_PRESENT_COUNT: u32 = 16;
pub(crate) const MAX_FRAMES_IN_FLIGHT: u32 = 3;

// Common Structs

/// The debug-mode state of a copy pass. Translation of `Pass` (its
/// `command_buffer` back pointer is the borrow a pass handle holds).
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct PassState {
    pub(crate) in_progress: bool,
}

/// The debug-mode state of a compute pass. Translation of `ComputePass`.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct ComputePassState {
    pub(crate) in_progress: bool,

    /// The bound pipeline's binding counts.
    pub(crate) compute_pipeline: Option<ComputePipelineHeader>,

    pub(crate) sampler_bound: [bool; MAX_TEXTURE_SAMPLERS_PER_STAGE as usize],
    pub(crate) read_only_storage_texture_bound: [bool; MAX_STORAGE_TEXTURES_PER_STAGE as usize],
    pub(crate) read_only_storage_buffer_bound: [bool; MAX_STORAGE_BUFFERS_PER_STAGE as usize],
    pub(crate) read_write_storage_texture_bound: [bool; MAX_COMPUTE_WRITE_TEXTURES as usize],
    pub(crate) read_write_storage_buffer_bound: [bool; MAX_COMPUTE_WRITE_BUFFERS as usize],
}

/// The debug-mode state of a render pass. Translation of `RenderPass`.
///
/// Upstream also records the color and depth-stencil targets, for the
/// sampler and storage texture checks it has disabled (`#if 0`, see
/// <https://github.com/libsdl-org/SDL/issues/13871>); they aren't kept here.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct RenderPassState {
    pub(crate) in_progress: bool,
    pub(crate) num_color_targets: u32,

    /// The bound pipeline's binding counts.
    pub(crate) graphics_pipeline: Option<GraphicsPipelineHeader>,

    pub(crate) vertex_sampler_bound: [bool; MAX_TEXTURE_SAMPLERS_PER_STAGE as usize],
    pub(crate) vertex_storage_texture_bound: [bool; MAX_STORAGE_TEXTURES_PER_STAGE as usize],
    pub(crate) vertex_storage_buffer_bound: [bool; MAX_STORAGE_BUFFERS_PER_STAGE as usize],

    pub(crate) fragment_sampler_bound: [bool; MAX_TEXTURE_SAMPLERS_PER_STAGE as usize],
    pub(crate) fragment_storage_texture_bound: [bool; MAX_STORAGE_TEXTURES_PER_STAGE as usize],
    pub(crate) fragment_storage_buffer_bound: [bool; MAX_STORAGE_BUFFERS_PER_STAGE as usize],
}

/// The front end's state of a command buffer. Translation of
/// `CommandBufferCommonHeader`.
///
/// Its `device` is the command buffer's device handle, and `submitted`
/// isn't needed: submitting or cancelling consumes the [`CommandBuffer`].
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct CommandBufferHeader {
    pub(crate) render_pass: RenderPassState,
    pub(crate) compute_pass: ComputePassState,

    pub(crate) copy_pass: PassState,
    pub(crate) swapchain_texture_acquired: bool,
    /// used to avoid tripping assert on GenerateMipmaps
    pub(crate) ignore_render_pass_texture_validation: bool,
}

impl CommandBufferHeader {
    /// Whether any pass is in progress (`CHECK_ANY_PASS_IN_PROGRESS`).
    pub(crate) fn any_pass_in_progress(&self) -> bool {
        self.render_pass.in_progress || self.compute_pass.in_progress || self.copy_pass.in_progress
    }
}

/// The binding counts of a graphics pipeline, which a backend fills in when
/// it creates one. Translation of `GraphicsPipelineCommonHeader`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct GraphicsPipelineHeader {
    pub(crate) num_vertex_samplers: u32,
    pub(crate) num_vertex_storage_textures: u32,
    pub(crate) num_vertex_storage_buffers: u32,
    pub(crate) num_vertex_uniform_buffers: u32,

    pub(crate) num_fragment_samplers: u32,
    pub(crate) num_fragment_storage_textures: u32,
    pub(crate) num_fragment_storage_buffers: u32,
    pub(crate) num_fragment_uniform_buffers: u32,
}

/// The binding counts of a compute pipeline, which a backend fills in when
/// it creates one. Translation of `ComputePipelineCommonHeader`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct ComputePipelineHeader {
    pub(crate) num_samplers: u32,
    pub(crate) num_readonly_storage_textures: u32,
    pub(crate) num_readonly_storage_buffers: u32,
    pub(crate) num_readwrite_storage_textures: u32,
    pub(crate) num_readwrite_storage_buffers: u32,
    pub(crate) num_uniform_buffers: u32,
}

/// The fragment uniforms of the blit shaders. Translation of
/// `BlitFragmentUniforms`.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) struct BlitFragmentUniforms {
    // texcoord space
    pub(crate) left: f32,
    pub(crate) top: f32,
    pub(crate) width: f32,
    pub(crate) height: f32,

    pub(crate) mip_level: u32,
    pub(crate) layer_or_depth: f32,
}

impl BlitFragmentUniforms {
    /// The uniforms as the shaders read them (the C struct's bytes).
    pub(crate) fn to_bytes(self) -> [u8; 24] {
        let mut out = [0u8; 24];
        let words = [
            self.left.to_bits(),
            self.top.to_bits(),
            self.width.to_bits(),
            self.height.to_bits(),
            self.mip_level,
            self.layer_or_depth.to_bits(),
        ];
        for (chunk, word) in out.chunks_exact_mut(4).zip(words) {
            chunk.copy_from_slice(&word.to_ne_bytes());
        }
        out
    }
}

/// A cached blit pipeline. Translation of `BlitPipelineCacheEntry`.
#[derive(Debug)]
pub(crate) struct BlitPipelineCacheEntry {
    pub(crate) texture_type: TextureType,
    pub(crate) format: TextureFormat,
    /// Owned by the backend (it releases the pipelines when it is destroyed).
    pub(crate) pipeline: GraphicsPipeline,
}

/// A backend's blit pipelines (`blit_pipelines` and its count and capacity).
#[derive(Debug)]
#[allow(dead_code)] // (for the backends)
pub(crate) enum BlitPipelineCache {
    /// Pre-created, format-agnostic pipelines, indexed by source texture
    /// type (upstream passes a NULL count).
    FormatAgnostic(Vec<BlitPipelineCacheEntry>),
    /// Pipelines per source texture type and destination format, created
    /// when first needed.
    PerFormat(Vec<BlitPipelineCacheEntry>),
}

// Internal Helper Utilities

/// `SDL_GPU_TEXTUREFORMAT_MAX_ENUM_VALUE`.
pub(crate) const TEXTUREFORMAT_MAX_ENUM_VALUE: u32 = TextureFormat::ASTC_12x12_FLOAT.0 + 1;

// (The other `*_MAX_ENUM_VALUE`s bound C enums whose Rust enums can't hold
// anything else.)

impl TextureFormat {
    /// The width of the format's blocks in texels. Translation of
    /// `Texture_GetBlockWidth()`.
    pub(crate) fn block_width(self) -> i32 {
        use TextureFormat as F;
        match self {
            F::ASTC_12x10_UNORM
            | F::ASTC_12x12_UNORM
            | F::ASTC_12x10_UNORM_SRGB
            | F::ASTC_12x12_UNORM_SRGB
            | F::ASTC_12x10_FLOAT
            | F::ASTC_12x12_FLOAT => 12,
            F::ASTC_10x5_UNORM
            | F::ASTC_10x6_UNORM
            | F::ASTC_10x8_UNORM
            | F::ASTC_10x10_UNORM
            | F::ASTC_10x5_UNORM_SRGB
            | F::ASTC_10x6_UNORM_SRGB
            | F::ASTC_10x8_UNORM_SRGB
            | F::ASTC_10x10_UNORM_SRGB
            | F::ASTC_10x5_FLOAT
            | F::ASTC_10x6_FLOAT
            | F::ASTC_10x8_FLOAT
            | F::ASTC_10x10_FLOAT => 10,
            F::ASTC_8x5_UNORM
            | F::ASTC_8x6_UNORM
            | F::ASTC_8x8_UNORM
            | F::ASTC_8x5_UNORM_SRGB
            | F::ASTC_8x6_UNORM_SRGB
            | F::ASTC_8x8_UNORM_SRGB
            | F::ASTC_8x5_FLOAT
            | F::ASTC_8x6_FLOAT
            | F::ASTC_8x8_FLOAT => 8,
            F::ASTC_6x5_UNORM
            | F::ASTC_6x6_UNORM
            | F::ASTC_6x5_UNORM_SRGB
            | F::ASTC_6x6_UNORM_SRGB
            | F::ASTC_6x5_FLOAT
            | F::ASTC_6x6_FLOAT => 6,
            F::ASTC_5x4_UNORM
            | F::ASTC_5x5_UNORM
            | F::ASTC_5x4_UNORM_SRGB
            | F::ASTC_5x5_UNORM_SRGB
            | F::ASTC_5x4_FLOAT
            | F::ASTC_5x5_FLOAT => 5,
            F::BC1_RGBA_UNORM
            | F::BC2_RGBA_UNORM
            | F::BC3_RGBA_UNORM
            | F::BC4_R_UNORM
            | F::BC5_RG_UNORM
            | F::BC7_RGBA_UNORM
            | F::BC6H_RGB_FLOAT
            | F::BC6H_RGB_UFLOAT
            | F::BC1_RGBA_UNORM_SRGB
            | F::BC2_RGBA_UNORM_SRGB
            | F::BC3_RGBA_UNORM_SRGB
            | F::BC7_RGBA_UNORM_SRGB
            | F::ASTC_4x4_UNORM
            | F::ASTC_4x4_UNORM_SRGB
            | F::ASTC_4x4_FLOAT => 4,
            F::R8G8B8A8_UNORM
            | F::B8G8R8A8_UNORM
            | F::B5G6R5_UNORM
            | F::B5G5R5A1_UNORM
            | F::B4G4R4A4_UNORM
            | F::R10G10B10A2_UNORM
            | F::R8G8_UNORM
            | F::R16G16_UNORM
            | F::R16G16B16A16_UNORM
            | F::R8_UNORM
            | F::R16_UNORM
            | F::A8_UNORM
            | F::R8_SNORM
            | F::R8G8_SNORM
            | F::R8G8B8A8_SNORM
            | F::R16_SNORM
            | F::R16G16_SNORM
            | F::R16G16B16A16_SNORM
            | F::R16_FLOAT
            | F::R16G16_FLOAT
            | F::R16G16B16A16_FLOAT
            | F::R32_FLOAT
            | F::R32G32_FLOAT
            | F::R32G32B32A32_FLOAT
            | F::R11G11B10_UFLOAT
            | F::R8_UINT
            | F::R8G8_UINT
            | F::R8G8B8A8_UINT
            | F::R16_UINT
            | F::R16G16_UINT
            | F::R16G16B16A16_UINT
            | F::R32_UINT
            | F::R32G32_UINT
            | F::R32G32B32A32_UINT
            | F::R8_INT
            | F::R8G8_INT
            | F::R8G8B8A8_INT
            | F::R16_INT
            | F::R16G16_INT
            | F::R16G16B16A16_INT
            | F::R32_INT
            | F::R32G32_INT
            | F::R32G32B32A32_INT
            | F::R8G8B8A8_UNORM_SRGB
            | F::B8G8R8A8_UNORM_SRGB
            | F::D16_UNORM
            | F::D24_UNORM
            | F::D32_FLOAT
            | F::D24_UNORM_S8_UINT
            | F::D32_FLOAT_S8_UINT => 1,
            _ => {
                crate::sdl_assert_release!(!"Unrecognized TextureFormat!");
                0
            }
        }
    }

    /// The height of the format's blocks in texels. Translation of
    /// `Texture_GetBlockHeight()`.
    pub(crate) fn block_height(self) -> i32 {
        use TextureFormat as F;
        match self {
            F::ASTC_12x12_UNORM | F::ASTC_12x12_UNORM_SRGB | F::ASTC_12x12_FLOAT => 12,
            F::ASTC_12x10_UNORM
            | F::ASTC_12x10_UNORM_SRGB
            | F::ASTC_12x10_FLOAT
            | F::ASTC_10x10_UNORM
            | F::ASTC_10x10_UNORM_SRGB
            | F::ASTC_10x10_FLOAT => 10,
            F::ASTC_10x8_UNORM
            | F::ASTC_10x8_UNORM_SRGB
            | F::ASTC_10x8_FLOAT
            | F::ASTC_8x8_UNORM
            | F::ASTC_8x8_UNORM_SRGB
            | F::ASTC_8x8_FLOAT => 8,
            F::ASTC_10x6_UNORM
            | F::ASTC_10x6_UNORM_SRGB
            | F::ASTC_10x6_FLOAT
            | F::ASTC_8x6_UNORM
            | F::ASTC_8x6_UNORM_SRGB
            | F::ASTC_8x6_FLOAT
            | F::ASTC_6x6_UNORM
            | F::ASTC_6x6_UNORM_SRGB
            | F::ASTC_6x6_FLOAT => 6,
            F::ASTC_10x5_UNORM
            | F::ASTC_10x5_UNORM_SRGB
            | F::ASTC_10x5_FLOAT
            | F::ASTC_8x5_UNORM
            | F::ASTC_8x5_UNORM_SRGB
            | F::ASTC_8x5_FLOAT
            | F::ASTC_6x5_UNORM
            | F::ASTC_6x5_UNORM_SRGB
            | F::ASTC_6x5_FLOAT
            | F::ASTC_5x5_UNORM
            | F::ASTC_5x5_UNORM_SRGB
            | F::ASTC_5x5_FLOAT => 5,
            F::BC1_RGBA_UNORM
            | F::BC2_RGBA_UNORM
            | F::BC3_RGBA_UNORM
            | F::BC4_R_UNORM
            | F::BC5_RG_UNORM
            | F::BC7_RGBA_UNORM
            | F::BC6H_RGB_FLOAT
            | F::BC6H_RGB_UFLOAT
            | F::BC1_RGBA_UNORM_SRGB
            | F::BC2_RGBA_UNORM_SRGB
            | F::BC3_RGBA_UNORM_SRGB
            | F::BC7_RGBA_UNORM_SRGB
            | F::ASTC_5x4_UNORM
            | F::ASTC_5x4_UNORM_SRGB
            | F::ASTC_5x4_FLOAT
            | F::ASTC_4x4_UNORM
            | F::ASTC_4x4_UNORM_SRGB
            | F::ASTC_4x4_FLOAT => 4,
            F::R8G8B8A8_UNORM
            | F::B8G8R8A8_UNORM
            | F::B5G6R5_UNORM
            | F::B5G5R5A1_UNORM
            | F::B4G4R4A4_UNORM
            | F::R10G10B10A2_UNORM
            | F::R8G8_UNORM
            | F::R16G16_UNORM
            | F::R16G16B16A16_UNORM
            | F::R8_UNORM
            | F::R16_UNORM
            | F::A8_UNORM
            | F::R8_SNORM
            | F::R8G8_SNORM
            | F::R8G8B8A8_SNORM
            | F::R16_SNORM
            | F::R16G16_SNORM
            | F::R16G16B16A16_SNORM
            | F::R16_FLOAT
            | F::R16G16_FLOAT
            | F::R16G16B16A16_FLOAT
            | F::R32_FLOAT
            | F::R32G32_FLOAT
            | F::R32G32B32A32_FLOAT
            | F::R11G11B10_UFLOAT
            | F::R8_UINT
            | F::R8G8_UINT
            | F::R8G8B8A8_UINT
            | F::R16_UINT
            | F::R16G16_UINT
            | F::R16G16B16A16_UINT
            | F::R32_UINT
            | F::R32G32_UINT
            | F::R32G32B32A32_UINT
            | F::R8_INT
            | F::R8G8_INT
            | F::R8G8B8A8_INT
            | F::R16_INT
            | F::R16G16_INT
            | F::R16G16B16A16_INT
            | F::R32_INT
            | F::R32G32_INT
            | F::R32G32B32A32_INT
            | F::R8G8B8A8_UNORM_SRGB
            | F::B8G8R8A8_UNORM_SRGB
            | F::D16_UNORM
            | F::D24_UNORM
            | F::D32_FLOAT
            | F::D24_UNORM_S8_UINT
            | F::D32_FLOAT_S8_UINT => 1,
            _ => {
                crate::sdl_assert_release!(!"Unrecognized TextureFormat!");
                0
            }
        }
    }

    /// Whether this is a depth format. Translation of `IsDepthFormat()`.
    pub(crate) fn is_depth_format(self) -> bool {
        use TextureFormat as F;
        matches!(
            self,
            F::D16_UNORM
                | F::D24_UNORM
                | F::D32_FLOAT
                | F::D24_UNORM_S8_UINT
                | F::D32_FLOAT_S8_UINT
        )
    }

    /// Whether this is a depth format with stencil. Translation of
    /// `IsStencilFormat()`.
    #[allow(dead_code)] // (for the backends)
    pub(crate) fn is_stencil_format(self) -> bool {
        matches!(
            self,
            TextureFormat::D24_UNORM_S8_UINT | TextureFormat::D32_FLOAT_S8_UINT
        )
    }

    /// Whether this is an 8 or 16 bit integer format. Translation of
    /// `IsIntegerFormat()`.
    pub(crate) fn is_integer_format(self) -> bool {
        use TextureFormat as F;
        matches!(
            self,
            F::R8_UINT
                | F::R8G8_UINT
                | F::R8G8B8A8_UINT
                | F::R16_UINT
                | F::R16G16_UINT
                | F::R16G16B16A16_UINT
                | F::R8_INT
                | F::R8G8_INT
                | F::R8G8B8A8_INT
                | F::R16_INT
                | F::R16G16_INT
                | F::R16G16B16A16_INT
        )
    }

    /// Whether this is a block-compressed format. Translation of
    /// `IsCompressedFormat()`.
    #[allow(dead_code)] // (for the backends)
    pub(crate) fn is_compressed_format(self) -> bool {
        use TextureFormat as F;
        matches!(
            self,
            F::BC1_RGBA_UNORM
                | F::BC1_RGBA_UNORM_SRGB
                | F::BC2_RGBA_UNORM
                | F::BC2_RGBA_UNORM_SRGB
                | F::BC3_RGBA_UNORM
                | F::BC3_RGBA_UNORM_SRGB
                | F::BC4_R_UNORM
                | F::BC5_RG_UNORM
                | F::BC6H_RGB_FLOAT
                | F::BC6H_RGB_UFLOAT
                | F::BC7_RGBA_UNORM
                | F::BC7_RGBA_UNORM_SRGB
                | F::ASTC_4x4_UNORM
                | F::ASTC_5x4_UNORM
                | F::ASTC_5x5_UNORM
                | F::ASTC_6x5_UNORM
                | F::ASTC_6x6_UNORM
                | F::ASTC_8x5_UNORM
                | F::ASTC_8x6_UNORM
                | F::ASTC_8x8_UNORM
                | F::ASTC_10x5_UNORM
                | F::ASTC_10x6_UNORM
                | F::ASTC_10x8_UNORM
                | F::ASTC_10x10_UNORM
                | F::ASTC_12x10_UNORM
                | F::ASTC_12x12_UNORM
                | F::ASTC_4x4_UNORM_SRGB
                | F::ASTC_5x4_UNORM_SRGB
                | F::ASTC_5x5_UNORM_SRGB
                | F::ASTC_6x5_UNORM_SRGB
                | F::ASTC_6x6_UNORM_SRGB
                | F::ASTC_8x5_UNORM_SRGB
                | F::ASTC_8x6_UNORM_SRGB
                | F::ASTC_8x8_UNORM_SRGB
                | F::ASTC_10x5_UNORM_SRGB
                | F::ASTC_10x6_UNORM_SRGB
                | F::ASTC_10x8_UNORM_SRGB
                | F::ASTC_10x10_UNORM_SRGB
                | F::ASTC_12x10_UNORM_SRGB
                | F::ASTC_12x12_UNORM_SRGB
                | F::ASTC_4x4_FLOAT
                | F::ASTC_5x4_FLOAT
                | F::ASTC_5x5_FLOAT
                | F::ASTC_6x5_FLOAT
                | F::ASTC_6x6_FLOAT
                | F::ASTC_8x5_FLOAT
                | F::ASTC_8x6_FLOAT
                | F::ASTC_8x8_FLOAT
                | F::ASTC_10x5_FLOAT
                | F::ASTC_10x6_FLOAT
                | F::ASTC_10x8_FLOAT
                | F::ASTC_10x10_FLOAT
                | F::ASTC_12x10_FLOAT
                | F::ASTC_12x12_FLOAT
        )
    }

    /// Whether the format has an alpha channel. Translation of
    /// `FormatHasAlpha()`.
    pub(crate) fn has_alpha(self) -> bool {
        use TextureFormat as F;
        match self {
            F::ASTC_12x10_UNORM
            | F::ASTC_12x12_UNORM
            | F::ASTC_12x10_UNORM_SRGB
            | F::ASTC_12x12_UNORM_SRGB
            | F::ASTC_12x10_FLOAT
            | F::ASTC_12x12_FLOAT
            | F::ASTC_10x5_UNORM
            | F::ASTC_10x6_UNORM
            | F::ASTC_10x8_UNORM
            | F::ASTC_10x10_UNORM
            | F::ASTC_10x5_UNORM_SRGB
            | F::ASTC_10x6_UNORM_SRGB
            | F::ASTC_10x8_UNORM_SRGB
            | F::ASTC_10x10_UNORM_SRGB
            | F::ASTC_10x5_FLOAT
            | F::ASTC_10x6_FLOAT
            | F::ASTC_10x8_FLOAT
            | F::ASTC_10x10_FLOAT
            | F::ASTC_8x5_UNORM
            | F::ASTC_8x6_UNORM
            | F::ASTC_8x8_UNORM
            | F::ASTC_8x5_UNORM_SRGB
            | F::ASTC_8x6_UNORM_SRGB
            | F::ASTC_8x8_UNORM_SRGB
            | F::ASTC_8x5_FLOAT
            | F::ASTC_8x6_FLOAT
            | F::ASTC_8x8_FLOAT
            | F::ASTC_6x5_UNORM
            | F::ASTC_6x6_UNORM
            | F::ASTC_6x5_UNORM_SRGB
            | F::ASTC_6x6_UNORM_SRGB
            | F::ASTC_6x5_FLOAT
            | F::ASTC_6x6_FLOAT
            | F::ASTC_5x4_UNORM
            | F::ASTC_5x5_UNORM
            | F::ASTC_5x4_UNORM_SRGB
            | F::ASTC_5x5_UNORM_SRGB
            | F::ASTC_5x4_FLOAT
            | F::ASTC_5x5_FLOAT
            | F::ASTC_4x4_UNORM
            | F::ASTC_4x4_UNORM_SRGB
            | F::ASTC_4x4_FLOAT => {
                // ASTC textures may or may not have alpha; return true as this is mainly intended for validation
                true
            }

            F::BC1_RGBA_UNORM
            | F::BC2_RGBA_UNORM
            | F::BC3_RGBA_UNORM
            | F::BC7_RGBA_UNORM
            | F::BC1_RGBA_UNORM_SRGB
            | F::BC2_RGBA_UNORM_SRGB
            | F::BC3_RGBA_UNORM_SRGB
            | F::BC7_RGBA_UNORM_SRGB
            | F::R8G8B8A8_UNORM
            | F::B8G8R8A8_UNORM
            | F::B5G5R5A1_UNORM
            | F::B4G4R4A4_UNORM
            | F::R10G10B10A2_UNORM
            | F::R16G16B16A16_UNORM
            | F::A8_UNORM
            | F::R8G8B8A8_SNORM
            | F::R16G16B16A16_SNORM
            | F::R16G16B16A16_FLOAT
            | F::R32G32B32A32_FLOAT
            | F::R8G8B8A8_UINT
            | F::R16G16B16A16_UINT
            | F::R32G32B32A32_UINT
            | F::R8G8B8A8_INT
            | F::R16G16B16A16_INT
            | F::R32G32B32A32_INT
            | F::R8G8B8A8_UNORM_SRGB
            | F::B8G8R8A8_UNORM_SRGB => true,

            _ => false,
        }
    }

    /// The bytes in a row of `width` texels (whole blocks). Translation of
    /// `BytesPerRow()`; as there, an unrecognized format (block width 0)
    /// divides by zero, so backends call it with valid formats only.
    #[allow(dead_code)] // (for the backends)
    pub(crate) fn bytes_per_row(self, width: i32) -> u32 {
        let block_width = self.block_width() as u32;
        let blocks_per_row = (width as u32).wrapping_add(block_width).wrapping_sub(1) / block_width;
        blocks_per_row.wrapping_mul(self.texel_block_size())
    }
}

/// The size of an index. Translation of `IndexSize()`.
#[allow(dead_code)] // (for the backends)
pub(crate) fn index_size(size: IndexElementSize) -> u32 {
    if size == IndexElementSize::Bits16 {
        2
    } else {
        4
    }
}

// Backend objects

/// An object a backend created: a texture, buffer, transfer buffer,
/// sampler, shader, pipeline or fence (the opaque `SDL_GPUTexture *` and
/// friends), downcast by the backend to its own type.
#[derive(Clone)]
pub(crate) struct BackendObject(pub(crate) Arc<dyn Any + Send + Sync>);

impl BackendObject {
    #[allow(dead_code)] // (for the backends)
    pub(crate) fn new<T: Any + Send + Sync>(value: T) -> BackendObject {
        BackendObject(Arc::new(value))
    }

    /// The backend's object, if it is a `T`.
    #[allow(dead_code)] // (for the backends)
    pub(crate) fn downcast_ref<T: Any>(&self) -> Option<&T> {
        self.0.downcast_ref::<T>()
    }

    /// Another reference to the backend's object, if it is a `T`.
    #[allow(dead_code)] // (for the backends)
    pub(crate) fn downcast<T: Any + Send + Sync>(&self) -> Option<Arc<T>> {
        self.0.clone().downcast::<T>().ok()
    }
}

impl std::fmt::Debug for BackendObject {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "BackendObject({:p})", Arc::as_ptr(&self.0))
    }
}

/// A backend's command buffer (`SDL_GPUCommandBuffer *`), downcast by the
/// backend to its own type. The owned form is `Box<BackendCommandBuffer>`.
pub(crate) type BackendCommandBuffer = dyn Any + Send;

/// A swapchain texture a backend acquired: the texture, the creation info
/// the backend gives it (upstream's `TextureCommonHeader` of the
/// swapchain's texture containers) and its size.
#[derive(Debug)]
pub(crate) struct BackendSwapchainTexture {
    pub(crate) raw: BackendObject,
    pub(crate) info: TextureCreateInfo,
    pub(crate) width: u32,
    pub(crate) height: u32,
}

/// A device a backend created: its driver and the shader formats it takes
/// (`result->shader_formats`).
pub(crate) struct BackendDevice {
    pub(crate) driver: Box<dyn GpuDriver>,
    pub(crate) shader_formats: ShaderFormat,
}

impl std::fmt::Debug for BackendDevice {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("BackendDevice")
            .field("shader_formats", &self.shader_formats)
            .finish_non_exhaustive()
    }
}

// SDL_GPUDevice Definition

/// The functions a GPU backend implements: the function pointers of
/// `struct SDL_GPUDevice`, called with the backend's driver data
/// (`SDL_GPURenderer *`) as `self`. The front end has done the parameter
/// checks and (in debug mode) the validation before it calls them.
///
/// A device is shared between threads, so the driver locks its own state as
/// upstream's backends do. Functions that return `bool` plus an error in C
/// return a `Result`; creation functions return the new object.
///
/// Not translated: the OpenXR entry points (`DestroyXRSwapchain`,
/// `CreateXRSession`, `GetXRSwapchainFormats`, `CreateXRSwapchain`).
#[allow(clippy::too_many_arguments)]
pub(crate) trait GpuDriver: Send + Sync {
    // Device

    /// `DestroyDevice`: called once the device and everything created from
    /// it are dropped. Releases the claimed windows and the backend's own
    /// objects.
    fn destroy(&mut self);

    /// `GetDeviceProperties`.
    fn properties(&self) -> Properties;

    // State Creation

    /// `CreateComputePipeline`: the pipeline and its binding counts.
    fn create_compute_pipeline(
        &self,
        createinfo: &ComputePipelineCreateInfo<'_>,
    ) -> Result<(BackendObject, ComputePipelineHeader)>;

    /// `CreateGraphicsPipeline`: the pipeline and its binding counts.
    fn create_graphics_pipeline(
        &self,
        createinfo: &GraphicsPipelineCreateInfo<'_>,
    ) -> Result<(BackendObject, GraphicsPipelineHeader)>;

    /// `CreateSampler`.
    fn create_sampler(&self, createinfo: &SamplerCreateInfo) -> Result<BackendObject>;

    /// `CreateShader`.
    fn create_shader(&self, createinfo: &ShaderCreateInfo<'_>) -> Result<BackendObject>;

    /// `CreateTexture`. The front end keeps `createinfo` as the texture's
    /// info (upstream's backends copy it into the texture's header).
    fn create_texture(&self, createinfo: &TextureCreateInfo) -> Result<BackendObject>;

    /// `CreateBuffer`.
    fn create_buffer(
        &self,
        usage_flags: BufferUsageFlags,
        size: u32,
        debug_name: Option<&str>,
    ) -> Result<BackendObject>;

    /// `CreateTransferBuffer`.
    fn create_transfer_buffer(
        &self,
        usage: TransferBufferUsage,
        size: u32,
        debug_name: Option<&str>,
    ) -> Result<BackendObject>;

    // Debug Naming

    /// `SetBufferName`.
    fn set_buffer_name(&self, buffer: &BackendObject, text: &str);

    /// `SetTextureName`.
    fn set_texture_name(&self, texture: &BackendObject, text: &str);

    /// `InsertDebugLabel`.
    fn insert_debug_label(&self, command_buffer: &mut BackendCommandBuffer, text: &str);

    /// `PushDebugGroup`.
    fn push_debug_group(&self, command_buffer: &mut BackendCommandBuffer, name: &str);

    /// `PopDebugGroup`.
    fn pop_debug_group(&self, command_buffer: &mut BackendCommandBuffer);

    // Disposal

    /// `ReleaseTexture`: the application no longer uses the texture (the
    /// backend destroys it once the GPU is done with it).
    fn release_texture(&self, texture: &BackendObject);

    /// `ReleaseSampler`.
    fn release_sampler(&self, sampler: &BackendObject);

    /// `ReleaseBuffer`.
    fn release_buffer(&self, buffer: &BackendObject);

    /// `ReleaseTransferBuffer`.
    fn release_transfer_buffer(&self, transfer_buffer: &BackendObject);

    /// `ReleaseShader`.
    fn release_shader(&self, shader: &BackendObject);

    /// `ReleaseComputePipeline`.
    fn release_compute_pipeline(&self, compute_pipeline: &BackendObject);

    /// `ReleaseGraphicsPipeline`.
    fn release_graphics_pipeline(&self, graphics_pipeline: &BackendObject);

    // Render Pass

    /// `BeginRenderPass`.
    fn begin_render_pass(
        &self,
        command_buffer: &mut BackendCommandBuffer,
        color_target_infos: &[ColorTargetInfo<'_>],
        depth_stencil_target_info: Option<&DepthStencilTargetInfo<'_>>,
    );

    /// `BindGraphicsPipeline`.
    fn bind_graphics_pipeline(
        &self,
        command_buffer: &mut BackendCommandBuffer,
        graphics_pipeline: &BackendObject,
    );

    /// `SetViewport`.
    fn set_viewport(&self, command_buffer: &mut BackendCommandBuffer, viewport: &Viewport);

    /// `SetScissor`.
    fn set_scissor(&self, command_buffer: &mut BackendCommandBuffer, scissor: &Rect);

    /// `SetBlendConstants`.
    fn set_blend_constants(
        &self,
        command_buffer: &mut BackendCommandBuffer,
        blend_constants: FColor,
    );

    /// `SetStencilReference`.
    fn set_stencil_reference(&self, command_buffer: &mut BackendCommandBuffer, reference: u8);

    /// `BindVertexBuffers`.
    fn bind_vertex_buffers(
        &self,
        command_buffer: &mut BackendCommandBuffer,
        first_slot: u32,
        bindings: &[BufferBinding<'_>],
    );

    /// `BindIndexBuffer`.
    fn bind_index_buffer(
        &self,
        command_buffer: &mut BackendCommandBuffer,
        binding: &BufferBinding<'_>,
        index_element_size: IndexElementSize,
    );

    /// `BindVertexSamplers`.
    fn bind_vertex_samplers(
        &self,
        command_buffer: &mut BackendCommandBuffer,
        first_slot: u32,
        texture_sampler_bindings: &[TextureSamplerBinding<'_>],
    );

    /// `BindVertexStorageTextures`.
    fn bind_vertex_storage_textures(
        &self,
        command_buffer: &mut BackendCommandBuffer,
        first_slot: u32,
        storage_textures: &[&Texture],
    );

    /// `BindVertexStorageBuffers`.
    fn bind_vertex_storage_buffers(
        &self,
        command_buffer: &mut BackendCommandBuffer,
        first_slot: u32,
        storage_buffers: &[&super::Buffer],
    );

    /// `BindFragmentSamplers`.
    fn bind_fragment_samplers(
        &self,
        command_buffer: &mut BackendCommandBuffer,
        first_slot: u32,
        texture_sampler_bindings: &[TextureSamplerBinding<'_>],
    );

    /// `BindFragmentStorageTextures`.
    fn bind_fragment_storage_textures(
        &self,
        command_buffer: &mut BackendCommandBuffer,
        first_slot: u32,
        storage_textures: &[&Texture],
    );

    /// `BindFragmentStorageBuffers`.
    fn bind_fragment_storage_buffers(
        &self,
        command_buffer: &mut BackendCommandBuffer,
        first_slot: u32,
        storage_buffers: &[&super::Buffer],
    );

    /// `PushVertexUniformData`.
    fn push_vertex_uniform_data(
        &self,
        command_buffer: &mut BackendCommandBuffer,
        slot_index: u32,
        data: &[u8],
    );

    /// `PushFragmentUniformData`.
    fn push_fragment_uniform_data(
        &self,
        command_buffer: &mut BackendCommandBuffer,
        slot_index: u32,
        data: &[u8],
    );

    /// `DrawIndexedPrimitives`.
    fn draw_indexed_primitives(
        &self,
        command_buffer: &mut BackendCommandBuffer,
        num_indices: u32,
        num_instances: u32,
        first_index: u32,
        vertex_offset: i32,
        first_instance: u32,
    );

    /// `DrawPrimitives`.
    fn draw_primitives(
        &self,
        command_buffer: &mut BackendCommandBuffer,
        num_vertices: u32,
        num_instances: u32,
        first_vertex: u32,
        first_instance: u32,
    );

    /// `DrawPrimitivesIndirect`.
    fn draw_primitives_indirect(
        &self,
        command_buffer: &mut BackendCommandBuffer,
        buffer: &BackendObject,
        offset: u32,
        draw_count: u32,
    );

    /// `DrawIndexedPrimitivesIndirect`.
    fn draw_indexed_primitives_indirect(
        &self,
        command_buffer: &mut BackendCommandBuffer,
        buffer: &BackendObject,
        offset: u32,
        draw_count: u32,
    );

    /// `EndRenderPass`.
    fn end_render_pass(&self, command_buffer: &mut BackendCommandBuffer);

    // Compute Pass

    /// `BeginComputePass`.
    fn begin_compute_pass(
        &self,
        command_buffer: &mut BackendCommandBuffer,
        storage_texture_bindings: &[StorageTextureReadWriteBinding<'_>],
        storage_buffer_bindings: &[StorageBufferReadWriteBinding<'_>],
    );

    /// `BindComputePipeline`.
    fn bind_compute_pipeline(
        &self,
        command_buffer: &mut BackendCommandBuffer,
        compute_pipeline: &BackendObject,
    );

    /// `BindComputeSamplers`.
    fn bind_compute_samplers(
        &self,
        command_buffer: &mut BackendCommandBuffer,
        first_slot: u32,
        texture_sampler_bindings: &[TextureSamplerBinding<'_>],
    );

    /// `BindComputeStorageTextures`.
    fn bind_compute_storage_textures(
        &self,
        command_buffer: &mut BackendCommandBuffer,
        first_slot: u32,
        storage_textures: &[&Texture],
    );

    /// `BindComputeStorageBuffers`.
    fn bind_compute_storage_buffers(
        &self,
        command_buffer: &mut BackendCommandBuffer,
        first_slot: u32,
        storage_buffers: &[&super::Buffer],
    );

    /// `PushComputeUniformData`.
    fn push_compute_uniform_data(
        &self,
        command_buffer: &mut BackendCommandBuffer,
        slot_index: u32,
        data: &[u8],
    );

    /// `DispatchCompute`.
    fn dispatch_compute(
        &self,
        command_buffer: &mut BackendCommandBuffer,
        groupcount_x: u32,
        groupcount_y: u32,
        groupcount_z: u32,
    );

    /// `DispatchComputeIndirect`.
    fn dispatch_compute_indirect(
        &self,
        command_buffer: &mut BackendCommandBuffer,
        buffer: &BackendObject,
        offset: u32,
    );

    /// `EndComputePass`.
    fn end_compute_pass(&self, command_buffer: &mut BackendCommandBuffer);

    // TransferBuffer Data

    /// `MapTransferBuffer`: the transfer buffer's memory, which must stay
    /// valid for reads and writes of the buffer's whole size, and not be
    /// touched by the backend, until `unmap_transfer_buffer`.
    fn map_transfer_buffer(
        &self,
        transfer_buffer: &BackendObject,
        cycle: bool,
    ) -> Result<NonNull<u8>>;

    /// `UnmapTransferBuffer`.
    fn unmap_transfer_buffer(&self, transfer_buffer: &BackendObject);

    // Copy Pass

    /// `BeginCopyPass`.
    fn begin_copy_pass(&self, command_buffer: &mut BackendCommandBuffer);

    /// `UploadToTexture`.
    fn upload_to_texture(
        &self,
        command_buffer: &mut BackendCommandBuffer,
        source: &TextureTransferInfo<'_>,
        destination: &TextureRegion<'_>,
        cycle: bool,
    );

    /// `UploadToBuffer`.
    fn upload_to_buffer(
        &self,
        command_buffer: &mut BackendCommandBuffer,
        source: &TransferBufferLocation<'_>,
        destination: &BufferRegion<'_>,
        cycle: bool,
    );

    /// `CopyTextureToTexture`.
    fn copy_texture_to_texture(
        &self,
        command_buffer: &mut BackendCommandBuffer,
        source: &TextureLocation<'_>,
        destination: &TextureLocation<'_>,
        w: u32,
        h: u32,
        d: u32,
        cycle: bool,
    );

    /// `CopyBufferToBuffer`.
    fn copy_buffer_to_buffer(
        &self,
        command_buffer: &mut BackendCommandBuffer,
        source: &BufferLocation<'_>,
        destination: &BufferLocation<'_>,
        size: u32,
        cycle: bool,
    );

    /// `GenerateMipmaps`. It gets the front-end command buffer, as a
    /// backend may blit through the front end
    /// ([`CommandBuffer::blit_texture`]); its own command buffer is
    /// [`CommandBuffer::backend_mut`].
    fn generate_mipmaps(&self, command_buffer: &mut CommandBuffer, texture: &Texture);

    /// `DownloadFromTexture`.
    fn download_from_texture(
        &self,
        command_buffer: &mut BackendCommandBuffer,
        source: &TextureRegion<'_>,
        destination: &TextureTransferInfo<'_>,
    );

    /// `DownloadFromBuffer`.
    fn download_from_buffer(
        &self,
        command_buffer: &mut BackendCommandBuffer,
        source: &BufferRegion<'_>,
        destination: &TransferBufferLocation<'_>,
    );

    /// `EndCopyPass`.
    fn end_copy_pass(&self, command_buffer: &mut BackendCommandBuffer);

    /// `Blit`. It gets the front-end command buffer, for [`blit_common`].
    fn blit(&self, command_buffer: &mut CommandBuffer, info: &BlitInfo<'_>);

    // Submission/Presentation

    /// `SupportsSwapchainComposition`.
    fn supports_swapchain_composition(
        &self,
        window: Window,
        swapchain_composition: SwapchainComposition,
    ) -> bool;

    /// `SupportsPresentMode`.
    fn supports_present_mode(&self, window: Window, present_mode: PresentMode) -> bool;

    /// `ClaimWindow`.
    fn claim_window(&self, window: Window) -> Result<()>;

    /// `ReleaseWindow`.
    fn release_window(&self, window: Window);

    /// `SetSwapchainParameters`.
    fn set_swapchain_parameters(
        &self,
        window: Window,
        swapchain_composition: SwapchainComposition,
        present_mode: PresentMode,
    ) -> Result<()>;

    /// `SetAllowedFramesInFlight` (1 to 3).
    fn set_allowed_frames_in_flight(&self, allowed_frames_in_flight: u32) -> Result<()>;

    /// `GetSwapchainTextureFormat`.
    fn swapchain_texture_format(&self, window: Window) -> Result<TextureFormat>;

    /// `AcquireCommandBuffer`.
    fn acquire_command_buffer(&self) -> Result<Box<BackendCommandBuffer>>;

    /// `AcquireSwapchainTexture`: `None` (with success) when no texture is
    /// available, for instance while the window is minimized.
    fn acquire_swapchain_texture(
        &self,
        command_buffer: &mut BackendCommandBuffer,
        window: Window,
    ) -> Result<Option<BackendSwapchainTexture>>;

    /// `WaitForSwapchain`.
    fn wait_for_swapchain(&self, window: Window) -> Result<()>;

    /// `WaitAndAcquireSwapchainTexture`.
    fn wait_and_acquire_swapchain_texture(
        &self,
        command_buffer: &mut BackendCommandBuffer,
        window: Window,
    ) -> Result<Option<BackendSwapchainTexture>>;

    /// `Submit`.
    fn submit(&self, command_buffer: Box<BackendCommandBuffer>) -> Result<()>;

    /// `SubmitAndAcquireFence`.
    fn submit_and_acquire_fence(
        &self,
        command_buffer: Box<BackendCommandBuffer>,
    ) -> Result<BackendObject>;

    /// `Cancel`.
    fn cancel(&self, command_buffer: Box<BackendCommandBuffer>) -> Result<()>;

    /// `Wait`.
    fn wait(&self) -> Result<()>;

    /// `WaitForFences`.
    fn wait_for_fences(&self, wait_all: bool, fences: &[&BackendObject]) -> Result<()>;

    /// `QueryFence`.
    fn query_fence(&self, fence: &BackendObject) -> bool;

    /// `ReleaseFence`.
    fn release_fence(&self, fence: &BackendObject);

    // Feature Queries

    /// `SupportsTextureFormat`.
    fn supports_texture_format(
        &self,
        format: TextureFormat,
        texture_type: TextureType,
        usage: TextureUsageFlags,
    ) -> bool;

    /// `SupportsSampleCount`.
    fn supports_sample_count(
        &self,
        format: TextureFormat,
        desired_sample_count: SampleCount,
    ) -> bool;
}

/// A GPU backend in the front end's driver list. Translation of
/// `SDL_GPUBootstrap`.
#[derive(Debug)]
pub(crate) struct GpuBootstrap {
    pub(crate) name: &'static str,
    /// `PrepareDriver`: whether the backend can work, with the video
    /// subsystem's driver and the creation properties.
    pub(crate) prepare_driver: fn(video: &dyn VideoDriver, props: &Properties) -> bool,
    /// `CreateDevice`.
    pub(crate) create_device:
        fn(debug_mode: bool, prefer_low_power: bool, props: &Properties) -> Result<BackendDevice>,
}

// Internal Utility Functions

/// The six blit shaders a backend made for [`fetch_blit_pipeline`] and
/// [`blit_common`] (`blit_vertex_shader`, `blit_from_2d_shader`, ...).
#[derive(Clone, Copy, Debug)]
#[allow(dead_code)] // (for the backends)
pub(crate) struct BlitShaders<'a> {
    pub(crate) vertex: &'a Shader,
    pub(crate) from_2d: &'a Shader,
    pub(crate) from_2d_array: &'a Shader,
    pub(crate) from_3d: &'a Shader,
    pub(crate) from_cube: &'a Shader,
    pub(crate) from_cube_array: &'a Shader,
}

/// The blit pipeline for a source texture type and destination format,
/// from `cache` or made (and cached) through the front end. Translation of
/// `SDL_GPU_FetchBlitPipeline()`.
#[allow(dead_code)] // (for the backends)
pub(crate) fn fetch_blit_pipeline<'c>(
    device: &Arc<DeviceShared>,
    source_texture_type: TextureType,
    destination_format: TextureFormat,
    shaders: &BlitShaders<'_>,
    cache: &'c mut BlitPipelineCache,
) -> Result<&'c GraphicsPipeline> {
    let blit_pipelines = match cache {
        BlitPipelineCache::FormatAgnostic(pipelines) => {
            // use pre-created, format-agnostic pipelines
            // Note (upstream): C indexes the array without a bounds check.
            return pipelines
                .get(source_texture_type as usize)
                .map(|entry| &entry.pipeline)
                .ok_or_else(|| Error::new("Failed to create GPU pipeline for blit"));
        }
        BlitPipelineCache::PerFormat(pipelines) => pipelines,
    };

    if let Some(i) = blit_pipelines
        .iter()
        .position(|e| e.texture_type == source_texture_type && e.format == destination_format)
    {
        return Ok(&blit_pipelines[i].pipeline);
    }

    // No pipeline found, we'll need to make one!
    let mut color_target_desc = ColorTargetDescription {
        format: destination_format,
        blend_state: Default::default(),
    };
    color_target_desc.blend_state.color_write_mask = super::ColorComponentFlags(0xF);

    let fragment_shader = if source_texture_type == TextureType::Cube {
        shaders.from_cube
    } else if source_texture_type == TextureType::CubeArray {
        shaders.from_cube_array
    } else if source_texture_type == TextureType::Texture2DArray {
        shaders.from_2d_array
    } else if source_texture_type == TextureType::Texture3D {
        shaders.from_3d
    } else {
        shaders.from_2d
    };

    let blit_pipeline_create_info = GraphicsPipelineCreateInfo {
        vertex_shader: shaders.vertex,
        fragment_shader,
        vertex_input_state: Default::default(),
        primitive_type: PrimitiveType::TriangleList,
        rasterizer_state: RasterizerState {
            enable_depth_clip: device.default_enable_depth_clip,
            ..Default::default()
        },
        multisample_state: MultisampleState {
            sample_count: SampleCount::One,
            enable_mask: false,
            ..Default::default()
        },
        depth_stencil_state: Default::default(),
        target_info: GraphicsPipelineTargetInfo {
            color_target_descriptions: std::slice::from_ref(&color_target_desc),
            depth_stencil_format: TextureFormat::D16_UNORM, // arbitrary
            has_depth_stencil_target: false,
        },
        props: None,
    };

    let pipeline = match device.create_graphics_pipeline(&blit_pipeline_create_info) {
        Ok(pipeline) => pipeline.into_backend_owned(),
        Err(_) => return Err(Error::new("Failed to create GPU pipeline for blit")),
    };

    // Cache the new pipeline
    blit_pipelines.push(BlitPipelineCacheEntry {
        texture_type: source_texture_type,
        format: destination_format,
        pipeline,
    });

    Ok(&blit_pipelines.last().expect("just pushed").pipeline)
}

/// A blit drawn as a render pass with a blit pipeline, for backends without
/// a blit of their own. Translation of `SDL_GPU_BlitCommon()`.
///
/// Like upstream, the calls after beginning the render pass go on when one
/// of them fails; a failure to fetch the pipeline or begin the pass is
/// returned (upstream goes on with NULL, which the calls then reject).
#[allow(dead_code)] // (for the backends)
pub(crate) fn blit_common(
    command_buffer: &mut CommandBuffer,
    info: &BlitInfo<'_>,
    blit_linear_sampler: &Sampler,
    blit_nearest_sampler: &Sampler,
    shaders: &BlitShaders<'_>,
    cache: &mut BlitPipelineCache,
) -> Result<()> {
    let device = command_buffer.device.clone();
    let src_header = &info.source.texture.info;
    let dst_header = &info.destination.texture.info;

    let blit_pipeline = fetch_blit_pipeline(
        &device,
        src_header.texture_type,
        dst_header.format,
        shaders,
        cache,
    );

    crate::sdl_assert!(blit_pipeline.is_ok());
    let blit_pipeline = blit_pipeline?;

    // Note (upstream): the resolve fields are left uninitialized in C; they
    // are unused with the STORE store op.
    let color_target_info = ColorTargetInfo {
        load_op: info.load_op,
        clear_color: info.clear_color,
        store_op: StoreOp::Store,

        texture: info.destination.texture,
        mip_level: info.destination.mip_level,
        layer_or_depth_plane: info.destination.layer_or_depth_plane,
        cycle: info.cycle,
        resolve_texture: None,
        resolve_mip_level: 0,
        resolve_layer: 0,
        cycle_resolve_texture: false,
    };

    let mut render_pass =
        command_buffer.begin_render_pass(std::slice::from_ref(&color_target_info), None)?;

    let viewport = Viewport {
        x: info.destination.x as f32,
        y: info.destination.y as f32,
        w: info.destination.w as f32,
        h: info.destination.h as f32,
        min_depth: 0.0,
        max_depth: 1.0,
    };

    let _ = render_pass.set_viewport(&viewport);

    render_pass.bind_graphics_pipeline(blit_pipeline);

    let texture_sampler_binding = TextureSamplerBinding {
        texture: info.source.texture,
        sampler: if info.filter == Filter::Nearest {
            blit_nearest_sampler
        } else {
            blit_linear_sampler
        },
    };

    let _ = render_pass.bind_fragment_samplers(0, std::slice::from_ref(&texture_sampler_binding));

    // Note (upstream): a mip level of 32 or more shifts by the type's width
    // or more, which is undefined in C; the shift count wraps here.
    let src_width = src_header.width.wrapping_shr(info.source.mip_level) as f32;
    let src_height = src_header.height.wrapping_shr(info.source.mip_level) as f32;
    let mut blit_fragment_uniforms = BlitFragmentUniforms {
        left: info.source.x as f32 / src_width,
        top: info.source.y as f32 / src_height,
        width: info.source.w as f32 / src_width,
        height: info.source.h as f32 / src_height,
        mip_level: info.source.mip_level,
        layer_or_depth: 0.0,
    };

    let layer_divisor = if src_header.texture_type == TextureType::Texture3D {
        src_header.layer_count_or_depth
    } else {
        1
    };
    blit_fragment_uniforms.layer_or_depth =
        info.source.layer_or_depth_plane as f32 / layer_divisor as f32;

    if matches!(
        info.flip_mode,
        FlipMode::Horizontal | FlipMode::HorizontalAndVertical
    ) {
        blit_fragment_uniforms.left += blit_fragment_uniforms.width;
        blit_fragment_uniforms.width *= -1.0;
    }

    if matches!(
        info.flip_mode,
        FlipMode::Vertical | FlipMode::HorizontalAndVertical
    ) {
        blit_fragment_uniforms.top += blit_fragment_uniforms.height;
        blit_fragment_uniforms.height *= -1.0;
    }

    let _ = render_pass
        .command_buffer()
        .push_fragment_uniform_data(0, &blit_fragment_uniforms.to_bytes());

    let _ = render_pass.draw_primitives(3, 1, 0, 0);
    render_pass.end();
    Ok(())
}
