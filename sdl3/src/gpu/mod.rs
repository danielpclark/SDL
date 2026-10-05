// Rust translation of src/gpu/SDL_gpu.c and include/SDL3/SDL_gpu.h from
// Simple DirectMedia Layer.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! The GPU API: a cross-platform interface to modern graphics hardware
//! (Vulkan, Direct3D 12 and Metal), for 3D graphics and compute.
//!
//! A [`Device`] is created for the shader formats the application ships
//! ([`Device::new`], or [`Device::with_properties`] for the
//! `SDL.gpu.device.create.*` options); it picks the first backend that works
//! (or the one [`hints::GPU_DRIVER`] or the name property asks for). The
//! device creates the GPU resources ([`Texture`], [`Buffer`],
//! [`TransferBuffer`], [`Sampler`], [`Shader`], [`GraphicsPipeline`],
//! [`ComputePipeline`]): owned handles that release the resource when they
//! are dropped, which is safe while the GPU still uses it (upstream's
//! release functions defer the destruction). The device itself is destroyed
//! once it and everything created from it are dropped.
//!
//! Work is recorded into a [`CommandBuffer`]: render passes ([`RenderPass`]),
//! compute passes ([`ComputePass`]) and copy passes ([`CopyPass`]) borrow the
//! command buffer and end when they are dropped (or with `end()`), and the
//! command buffer is consumed by [`CommandBuffer::submit`] (or
//! [`CommandBuffer::submit_and_acquire_fence`], or
//! [`CommandBuffer::cancel`]). A command buffer dropped without that is
//! cancelled, or submitted if it acquired a swapchain texture (which can't
//! be cancelled). A window is drawn to after [`Device::claim_window`], by
//! rendering into the swapchain texture of
//! [`CommandBuffer::acquire_swapchain_texture`].
//!
//! In debug mode (the default) the front end validates each call as
//! upstream does: a failed check triggers an
//! [`sdl_assert_release!`](crate::sdl_assert_release) with upstream's
//! message and the call returns that message as an error. The checks C needs
//! for null pointers, submitted command buffers, ended passes and enum
//! values out of range can't fail here: handles are references, passes
//! borrow their command buffer, and the enums can't hold other values.
//!
//! The only backend is "vulkan", translated in part: its devices and
//! resources work, its command buffers, passes, swapchains and submission
//! fail with "not translated yet" (see the backend's module). Direct3D 12
//! and Metal come later. The OpenXR functions are not translated.

pub(crate) mod sysgpu;
#[cfg(test)]
mod tests;
pub(crate) mod vulkan;

use std::ptr::NonNull;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;

use crate::error::{Error, Result};
use crate::events::window::WindowFlags;
use crate::hints;
use crate::properties::Properties;
use crate::video::{FColor, FlipMode, PixelFormat, Rect, Window};

use sysgpu::{
    BackendCommandBuffer, BackendDevice, BackendObject, BackendSwapchainTexture,
    CommandBufferHeader, ComputePipelineHeader, GpuBootstrap, GpuDriver, GraphicsPipelineHeader,
    MAX_COLOR_TARGET_BINDINGS, MAX_COMPUTE_WRITE_BUFFERS, MAX_COMPUTE_WRITE_TEXTURES,
    MAX_FRAMES_IN_FLIGHT, MAX_STORAGE_BUFFERS_PER_STAGE, MAX_STORAGE_TEXTURES_PER_STAGE,
    MAX_TEXTURE_SAMPLERS_PER_STAGE, MAX_UNIFORM_BUFFERS_PER_STAGE, MAX_VERTEX_ATTRIBUTES,
    MAX_VERTEX_BUFFERS, TEXTUREFORMAT_MAX_ENUM_VALUE,
};

/// `SDL_assert_release(!"message")` for a failed debug-mode check, and the
/// error the call then returns.
macro_rules! debug_fail {
    ($msg:literal) => {{
        $crate::sdl_assert_release!(!$msg);
        $crate::error::Error::new($msg)
    }};
}

// Flag types

macro_rules! gpu_flags {
    (
        $(#[$m:meta])*
        $name:ident($ty:ty) {
            $($(#[$fm:meta])* $flag:ident = $val:expr;)*
        }
    ) => {
        $(#[$m])*
        #[repr(transparent)]
        #[derive(Clone, Copy, PartialEq, Eq, Hash, Default)]
        pub struct $name(pub $ty);

        impl $name {
            $($(#[$fm])* pub const $flag: $name = $name($val);)*

            /// The raw bits.
            pub const fn bits(self) -> $ty {
                self.0
            }
            /// True if every flag in `other` is set.
            pub const fn contains(self, other: $name) -> bool {
                self.0 & other.0 == other.0
            }
            /// True if any flag in `other` is set.
            pub const fn intersects(self, other: $name) -> bool {
                self.0 & other.0 != 0
            }
            /// True if no flag is set.
            pub const fn is_empty(self) -> bool {
                self.0 == 0
            }
        }

        impl std::ops::BitOr for $name {
            type Output = $name;
            fn bitor(self, rhs: $name) -> $name {
                $name(self.0 | rhs.0)
            }
        }

        impl std::ops::BitOrAssign for $name {
            fn bitor_assign(&mut self, rhs: $name) {
                self.0 |= rhs.0;
            }
        }

        impl std::ops::BitAnd for $name {
            type Output = $name;
            fn bitand(self, rhs: $name) -> $name {
                $name(self.0 & rhs.0)
            }
        }

        impl std::ops::Not for $name {
            type Output = $name;
            fn not(self) -> $name {
                $name(!self.0)
            }
        }

        impl std::fmt::Debug for $name {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                let mut rest = self.0;
                let mut first = true;
                write!(f, concat!(stringify!($name), "("))?;
                $(
                    #[allow(clippy::bad_bit_mask)] // (a zero flag is skipped at run time)
                    {
                        let v: $ty = $val;
                        if v != 0 && self.0 & v == v {
                            if !first {
                                write!(f, " | ")?;
                            }
                            write!(f, stringify!($flag))?;
                            first = false;
                            rest &= !v;
                        }
                    }
                )*
                if rest != 0 || first {
                    if !first {
                        write!(f, " | ")?;
                    }
                    write!(f, "{rest:#x}")?;
                }
                write!(f, ")")
            }
        }
    };
}

gpu_flags! {
    /// Specifies how a texture is intended to be used by the client.
    /// Translation of `SDL_GPUTextureUsageFlags`.
    ///
    /// A texture must have at least one usage flag. Note that some usage
    /// flag combinations are invalid. With regards to compute storage usage,
    /// READ | WRITE means that you can have shader A that only writes into
    /// the texture and shader B that only reads from the texture and bind
    /// the same texture to either shader respectively. SIMULTANEOUS means
    /// that you can do reads and writes within the same shader or compute
    /// pass.
    TextureUsageFlags(u32) {
        /// Texture supports sampling.
        SAMPLER = 1 << 0;
        /// Texture is a color render target.
        COLOR_TARGET = 1 << 1;
        /// Texture is a depth stencil target.
        DEPTH_STENCIL_TARGET = 1 << 2;
        /// Texture supports storage reads in graphics stages.
        GRAPHICS_STORAGE_READ = 1 << 3;
        /// Texture supports storage reads in the compute stage.
        COMPUTE_STORAGE_READ = 1 << 4;
        /// Texture supports storage writes in the compute stage.
        COMPUTE_STORAGE_WRITE = 1 << 5;
        /// Texture supports reads and writes in the same compute shader.
        /// This is NOT equivalent to READ | WRITE.
        COMPUTE_STORAGE_SIMULTANEOUS_READ_WRITE = 1 << 6;
    }
}

gpu_flags! {
    /// Specifies how a buffer is intended to be used by the client.
    /// Translation of `SDL_GPUBufferUsageFlags`.
    ///
    /// A buffer must have at least one usage flag. Note that some usage
    /// flag combinations are invalid. Unlike textures, READ | WRITE can be
    /// used for simultaneous read-write usage.
    BufferUsageFlags(u32) {
        /// Buffer is a vertex buffer.
        VERTEX = 1 << 0;
        /// Buffer is an index buffer.
        INDEX = 1 << 1;
        /// Buffer is an indirect buffer.
        INDIRECT = 1 << 2;
        /// Buffer supports storage reads in graphics stages.
        GRAPHICS_STORAGE_READ = 1 << 3;
        /// Buffer supports storage reads in the compute stage.
        COMPUTE_STORAGE_READ = 1 << 4;
        /// Buffer supports storage writes in the compute stage.
        COMPUTE_STORAGE_WRITE = 1 << 5;
    }
}

gpu_flags! {
    /// Specifies the format of shader code. Translation of
    /// `SDL_GPUShaderFormat`.
    ///
    /// Each format corresponds to a specific backend that accepts it.
    ShaderFormat(u32) {
        /// No format (`SDL_GPU_SHADERFORMAT_INVALID`).
        INVALID = 0;
        /// Shaders for NDA'd platforms.
        PRIVATE = 1 << 0;
        /// SPIR-V shaders for Vulkan.
        SPIRV = 1 << 1;
        /// DXBC SM5_1 shaders for D3D12.
        DXBC = 1 << 2;
        /// DXIL SM6_0 shaders for D3D12.
        DXIL = 1 << 3;
        /// MSL shaders for Metal.
        MSL = 1 << 4;
        /// Precompiled metallib shaders for Metal.
        METALLIB = 1 << 5;
    }
}

gpu_flags! {
    /// Specifies which color components are written in a graphics pipeline.
    /// Translation of `SDL_GPUColorComponentFlags`.
    ColorComponentFlags(u8) {
        /// the red component
        R = 1 << 0;
        /// the green component
        G = 1 << 1;
        /// the blue component
        B = 1 << 2;
        /// the alpha component
        A = 1 << 3;
    }
}

// Texture formats

/// Specifies the pixel format of a texture. Translation of
/// `SDL_GPUTextureFormat`.
///
/// Texture format support varies depending on driver, hardware, and usage
/// flags. In general, you should use [`Device::texture_supports_format`] to
/// query if a format is supported before using it. However, there are a few
/// guaranteed formats (see upstream's documentation).
///
/// Unless D16_UNORM is sufficient for your purposes, always check which of
/// D24/D32 is supported before creating a depth-stencil texture!
///
/// A newtype rather than an enum: values round-trip as in C, and
/// [`TextureFormat::INVALID`] (or a value out of range) is what debug mode
/// rejects.
#[repr(transparent)]
#[derive(Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct TextureFormat(pub u32);

macro_rules! texture_formats {
    ($($name:ident = $val:expr,)*) => {
        #[allow(non_upper_case_globals)]
        impl TextureFormat {
            $(
                #[doc = concat!("`SDL_GPU_TEXTUREFORMAT_", stringify!($name), "`")]
                pub const $name: TextureFormat = TextureFormat($val);
            )*

            /// The format's name, without the `SDL_GPU_TEXTUREFORMAT_` prefix.
            fn name(self) -> Option<&'static str> {
                match self.0 {
                    $($val => Some(stringify!($name)),)*
                    _ => None,
                }
            }
        }
    };
}

texture_formats! {
    INVALID = 0,

    // Unsigned Normalized Float Color Formats
    A8_UNORM = 1,
    R8_UNORM = 2,
    R8G8_UNORM = 3,
    R8G8B8A8_UNORM = 4,
    R16_UNORM = 5,
    R16G16_UNORM = 6,
    R16G16B16A16_UNORM = 7,
    R10G10B10A2_UNORM = 8,
    B5G6R5_UNORM = 9,
    B5G5R5A1_UNORM = 10,
    B4G4R4A4_UNORM = 11,
    B8G8R8A8_UNORM = 12,
    // Compressed Unsigned Normalized Float Color Formats
    BC1_RGBA_UNORM = 13,
    BC2_RGBA_UNORM = 14,
    BC3_RGBA_UNORM = 15,
    BC4_R_UNORM = 16,
    BC5_RG_UNORM = 17,
    BC7_RGBA_UNORM = 18,
    // Compressed Signed Float Color Formats
    BC6H_RGB_FLOAT = 19,
    // Compressed Unsigned Float Color Formats
    BC6H_RGB_UFLOAT = 20,
    // Signed Normalized Float Color Formats
    R8_SNORM = 21,
    R8G8_SNORM = 22,
    R8G8B8A8_SNORM = 23,
    R16_SNORM = 24,
    R16G16_SNORM = 25,
    R16G16B16A16_SNORM = 26,
    // Signed Float Color Formats
    R16_FLOAT = 27,
    R16G16_FLOAT = 28,
    R16G16B16A16_FLOAT = 29,
    R32_FLOAT = 30,
    R32G32_FLOAT = 31,
    R32G32B32A32_FLOAT = 32,
    // Unsigned Float Color Formats
    R11G11B10_UFLOAT = 33,
    // Unsigned Integer Color Formats
    R8_UINT = 34,
    R8G8_UINT = 35,
    R8G8B8A8_UINT = 36,
    R16_UINT = 37,
    R16G16_UINT = 38,
    R16G16B16A16_UINT = 39,
    R32_UINT = 40,
    R32G32_UINT = 41,
    R32G32B32A32_UINT = 42,
    // Signed Integer Color Formats
    R8_INT = 43,
    R8G8_INT = 44,
    R8G8B8A8_INT = 45,
    R16_INT = 46,
    R16G16_INT = 47,
    R16G16B16A16_INT = 48,
    R32_INT = 49,
    R32G32_INT = 50,
    R32G32B32A32_INT = 51,
    // SRGB Unsigned Normalized Color Formats
    R8G8B8A8_UNORM_SRGB = 52,
    B8G8R8A8_UNORM_SRGB = 53,
    // Compressed SRGB Unsigned Normalized Color Formats
    BC1_RGBA_UNORM_SRGB = 54,
    BC2_RGBA_UNORM_SRGB = 55,
    BC3_RGBA_UNORM_SRGB = 56,
    BC7_RGBA_UNORM_SRGB = 57,
    // Depth Formats
    D16_UNORM = 58,
    D24_UNORM = 59,
    D32_FLOAT = 60,
    D24_UNORM_S8_UINT = 61,
    D32_FLOAT_S8_UINT = 62,
    // Compressed ASTC Normalized Float Color Formats
    ASTC_4x4_UNORM = 63,
    ASTC_5x4_UNORM = 64,
    ASTC_5x5_UNORM = 65,
    ASTC_6x5_UNORM = 66,
    ASTC_6x6_UNORM = 67,
    ASTC_8x5_UNORM = 68,
    ASTC_8x6_UNORM = 69,
    ASTC_8x8_UNORM = 70,
    ASTC_10x5_UNORM = 71,
    ASTC_10x6_UNORM = 72,
    ASTC_10x8_UNORM = 73,
    ASTC_10x10_UNORM = 74,
    ASTC_12x10_UNORM = 75,
    ASTC_12x12_UNORM = 76,
    // Compressed SRGB ASTC Normalized Float Color Formats
    ASTC_4x4_UNORM_SRGB = 77,
    ASTC_5x4_UNORM_SRGB = 78,
    ASTC_5x5_UNORM_SRGB = 79,
    ASTC_6x5_UNORM_SRGB = 80,
    ASTC_6x6_UNORM_SRGB = 81,
    ASTC_8x5_UNORM_SRGB = 82,
    ASTC_8x6_UNORM_SRGB = 83,
    ASTC_8x8_UNORM_SRGB = 84,
    ASTC_10x5_UNORM_SRGB = 85,
    ASTC_10x6_UNORM_SRGB = 86,
    ASTC_10x8_UNORM_SRGB = 87,
    ASTC_10x10_UNORM_SRGB = 88,
    ASTC_12x10_UNORM_SRGB = 89,
    ASTC_12x12_UNORM_SRGB = 90,
    // Compressed ASTC Signed Float Color Formats
    ASTC_4x4_FLOAT = 91,
    ASTC_5x4_FLOAT = 92,
    ASTC_5x5_FLOAT = 93,
    ASTC_6x5_FLOAT = 94,
    ASTC_6x6_FLOAT = 95,
    ASTC_8x5_FLOAT = 96,
    ASTC_8x6_FLOAT = 97,
    ASTC_8x8_FLOAT = 98,
    ASTC_10x5_FLOAT = 99,
    ASTC_10x6_FLOAT = 100,
    ASTC_10x8_FLOAT = 101,
    ASTC_10x10_FLOAT = 102,
    ASTC_12x10_FLOAT = 103,
    ASTC_12x12_FLOAT = 104,
}

impl std::fmt::Debug for TextureFormat {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self.name() {
            Some(name) => write!(f, "TextureFormat::{name}"),
            None => write!(f, "TextureFormat({})", self.0),
        }
    }
}

// Enums

/// Specifies the primitive topology of a graphics pipeline. Translation of
/// `SDL_GPUPrimitiveType`.
#[repr(u32)]
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Default)]
pub enum PrimitiveType {
    /// A series of separate triangles.
    #[default]
    TriangleList,
    /// A series of connected triangles.
    TriangleStrip,
    /// A series of separate lines.
    LineList,
    /// A series of connected lines.
    LineStrip,
    /// A series of separate points.
    PointList,
}

/// Specifies how the contents of a texture attached to a render pass are
/// treated at the beginning of the render pass. Translation of
/// `SDL_GPULoadOp`.
#[repr(u32)]
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Default)]
pub enum LoadOp {
    /// The previous contents of the texture will be preserved.
    #[default]
    Load,
    /// The contents of the texture will be cleared to a color.
    Clear,
    /// The previous contents of the texture need not be preserved. The
    /// contents will be undefined.
    DontCare,
}

/// Specifies how the contents of a texture attached to a render pass are
/// treated at the end of the render pass. Translation of `SDL_GPUStoreOp`.
#[repr(u32)]
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Default)]
pub enum StoreOp {
    /// The contents generated during the render pass will be written to
    /// memory.
    #[default]
    Store,
    /// The contents generated during the render pass are not needed and may
    /// be discarded. The contents will be undefined.
    DontCare,
    /// The multisample contents generated during the render pass will be
    /// resolved to a non-multisample texture. The contents in the multisample
    /// texture may then be discarded and will be undefined.
    Resolve,
    /// The multisample contents generated during the render pass will be
    /// resolved to a non-multisample texture. The contents in the multisample
    /// texture will be written to memory.
    ResolveAndStore,
}

/// Specifies the size of elements in an index buffer. Translation of
/// `SDL_GPUIndexElementSize`.
#[repr(u32)]
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Default)]
pub enum IndexElementSize {
    /// The index elements are 16-bit.
    #[default]
    Bits16,
    /// The index elements are 32-bit.
    Bits32,
}

/// Specifies the type of a texture. Translation of `SDL_GPUTextureType`.
#[repr(u32)]
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Default)]
pub enum TextureType {
    /// The texture is a 2-dimensional image.
    #[default]
    Texture2D,
    /// The texture is a 2-dimensional array image.
    Texture2DArray,
    /// The texture is a 3-dimensional image.
    Texture3D,
    /// The texture is a cube image.
    Cube,
    /// The texture is a cube array image.
    CubeArray,
}

/// Specifies the sample count of a texture. Translation of
/// `SDL_GPUSampleCount`.
///
/// Used in multisampling. Note that this value only applies when the
/// texture is used as a render target.
#[repr(u32)]
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug, Default)]
pub enum SampleCount {
    /// No multisampling.
    #[default]
    One,
    /// MSAA 2x
    Two,
    /// MSAA 4x
    Four,
    /// MSAA 8x
    Eight,
}

/// Specifies the face of a cube map. Translation of `SDL_GPUCubeMapFace`.
///
/// Can be passed in as the layer field in texture-related structs.
#[repr(u32)]
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum CubeMapFace {
    /// the positive X face
    PositiveX,
    /// the negative X face
    NegativeX,
    /// the positive Y face
    PositiveY,
    /// the negative Y face
    NegativeY,
    /// the positive Z face
    PositiveZ,
    /// the negative Z face
    NegativeZ,
}

/// Specifies how a transfer buffer is intended to be used by the client.
/// Translation of `SDL_GPUTransferBufferUsage`.
///
/// Note that mapping and copying FROM an upload transfer buffer or TO a
/// download transfer buffer is undefined behavior.
#[repr(u32)]
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Default)]
pub enum TransferBufferUsage {
    /// for uploads to the GPU
    #[default]
    Upload,
    /// for downloads from the GPU
    Download,
}

/// Specifies which stage a shader program corresponds to. Translation of
/// `SDL_GPUShaderStage`.
#[repr(u32)]
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Default)]
pub enum ShaderStage {
    /// the vertex stage
    #[default]
    Vertex,
    /// the fragment stage
    Fragment,
}

/// Specifies the format of a vertex attribute. Translation of
/// `SDL_GPUVertexElementFormat`.
#[repr(u32)]
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum VertexElementFormat {
    // 32-bit Signed Integers
    /// one 32-bit signed integer
    Int = 1,
    /// two 32-bit signed integers
    Int2,
    /// three 32-bit signed integers
    Int3,
    /// four 32-bit signed integers
    Int4,

    // 32-bit Unsigned Integers
    /// one 32-bit unsigned integer
    Uint,
    /// two 32-bit unsigned integers
    Uint2,
    /// three 32-bit unsigned integers
    Uint3,
    /// four 32-bit unsigned integers
    Uint4,

    // 32-bit Floats
    /// one 32-bit float
    Float,
    /// two 32-bit floats
    Float2,
    /// three 32-bit floats
    Float3,
    /// four 32-bit floats
    Float4,

    // 8-bit Signed Integers
    /// two 8-bit signed integers
    Byte2,
    /// four 8-bit signed integers
    Byte4,

    // 8-bit Unsigned Integers
    /// two 8-bit unsigned integers
    Ubyte2,
    /// four 8-bit unsigned integers
    Ubyte4,

    // 8-bit Signed Normalized
    /// two 8-bit signed normalized values
    Byte2Norm,
    /// four 8-bit signed normalized values
    Byte4Norm,

    // 8-bit Unsigned Normalized
    /// two 8-bit unsigned normalized values
    Ubyte2Norm,
    /// four 8-bit unsigned normalized values
    Ubyte4Norm,

    // 16-bit Signed Integers
    /// two 16-bit signed integers
    Short2,
    /// four 16-bit signed integers
    Short4,

    // 16-bit Unsigned Integers
    /// two 16-bit unsigned integers
    Ushort2,
    /// four 16-bit unsigned integers
    Ushort4,

    // 16-bit Signed Normalized
    /// two 16-bit signed normalized values
    Short2Norm,
    /// four 16-bit signed normalized values
    Short4Norm,

    // 16-bit Unsigned Normalized
    /// two 16-bit unsigned normalized values
    Ushort2Norm,
    /// four 16-bit unsigned normalized values
    Ushort4Norm,

    // 16-bit Floats
    /// two 16-bit floats
    Half2,
    /// four 16-bit floats
    Half4,
}

/// Specifies the rate at which vertex attributes are pulled from buffers.
/// Translation of `SDL_GPUVertexInputRate`.
#[repr(u32)]
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Default)]
pub enum VertexInputRate {
    /// Attribute addressing is a function of the vertex index.
    #[default]
    Vertex,
    /// Attribute addressing is a function of the instance index.
    Instance,
}

/// Specifies the fill mode of the graphics pipeline. Translation of
/// `SDL_GPUFillMode`.
#[repr(u32)]
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Default)]
pub enum FillMode {
    /// Polygons will be rendered via rasterization.
    #[default]
    Fill,
    /// Polygon edges will be drawn as line segments.
    Line,
}

/// Specifies the facing direction in which triangle faces will be culled.
/// Translation of `SDL_GPUCullMode`.
#[repr(u32)]
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Default)]
pub enum CullMode {
    /// No triangles are culled.
    #[default]
    None,
    /// Front-facing triangles are culled.
    Front,
    /// Back-facing triangles are culled.
    Back,
}

/// Specifies the vertex winding that will cause a triangle to be determined
/// to be front-facing. Translation of `SDL_GPUFrontFace`.
#[repr(u32)]
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Default)]
pub enum FrontFace {
    /// A triangle with counter-clockwise vertex winding will be considered
    /// front-facing.
    #[default]
    CounterClockwise,
    /// A triangle with clockwise vertex winding will be considered
    /// front-facing.
    Clockwise,
}

/// Specifies a comparison operator for depth, stencil and sampler
/// operations. Translation of `SDL_GPUCompareOp`.
///
/// The default is the first operator, which is what backends make of C's
/// zeroed `SDL_GPU_COMPAREOP_INVALID`.
#[repr(u32)]
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Default)]
pub enum CompareOp {
    /// The comparison always evaluates false.
    #[default]
    Never = 1,
    /// The comparison evaluates reference < test.
    Less,
    /// The comparison evaluates reference == test.
    Equal,
    /// The comparison evaluates reference <= test.
    LessOrEqual,
    /// The comparison evaluates reference > test.
    Greater,
    /// The comparison evaluates reference != test.
    NotEqual,
    /// The comparison evaluates reference >= test.
    GreaterOrEqual,
    /// The comparison always evaluates true.
    Always,
}

/// Specifies what happens to a stored stencil value if stencil tests fail
/// or pass. Translation of `SDL_GPUStencilOp`.
#[repr(u32)]
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Default)]
pub enum StencilOp {
    /// Keeps the current value.
    #[default]
    Keep = 1,
    /// Sets the value to 0.
    Zero,
    /// Sets the value to reference.
    Replace,
    /// Increments the current value and clamps to the maximum value.
    IncrementAndClamp,
    /// Decrements the current value and clamps to 0.
    DecrementAndClamp,
    /// Bitwise-inverts the current value.
    Invert,
    /// Increments the current value and wraps back to 0.
    IncrementAndWrap,
    /// Decrements the current value and wraps to the maximum value.
    DecrementAndWrap,
}

/// Specifies the operator to be used when pixels in a render target are
/// blended with existing pixels in the texture. Translation of
/// `SDL_GPUBlendOp`.
///
/// The source color is the value written by the fragment shader. The
/// destination color is the value currently existing in the texture.
#[repr(u32)]
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Default)]
pub enum BlendOp {
    /// (source * source_factor) + (destination * destination_factor)
    #[default]
    Add = 1,
    /// (source * source_factor) - (destination * destination_factor)
    Subtract,
    /// (destination * destination_factor) - (source * source_factor)
    ReverseSubtract,
    /// min(source, destination)
    Min,
    /// max(source, destination)
    Max,
}

/// Specifies a blending factor to be used when pixels in a render target
/// are blended with existing pixels in the texture. Translation of
/// `SDL_GPUBlendFactor`.
///
/// The source color is the value written by the fragment shader. The
/// destination color is the value currently existing in the texture.
#[repr(u32)]
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Default)]
pub enum BlendFactor {
    /// 0
    #[default]
    Zero = 1,
    /// 1
    One,
    /// source color
    SrcColor,
    /// 1 - source color
    OneMinusSrcColor,
    /// destination color
    DstColor,
    /// 1 - destination color
    OneMinusDstColor,
    /// source alpha
    SrcAlpha,
    /// 1 - source alpha
    OneMinusSrcAlpha,
    /// destination alpha
    DstAlpha,
    /// 1 - destination alpha
    OneMinusDstAlpha,
    /// blend constant
    ConstantColor,
    /// 1 - blend constant
    OneMinusConstantColor,
    /// min(source alpha, 1 - destination alpha)
    SrcAlphaSaturate,
}

/// Specifies a filter operation used by a sampler. Translation of
/// `SDL_GPUFilter`.
#[repr(u32)]
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Default)]
pub enum Filter {
    /// Point filtering.
    #[default]
    Nearest,
    /// Linear filtering.
    Linear,
}

/// Specifies a mipmap mode used by a sampler. Translation of
/// `SDL_GPUSamplerMipmapMode`.
#[repr(u32)]
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Default)]
pub enum SamplerMipmapMode {
    /// Point filtering.
    #[default]
    Nearest,
    /// Linear filtering.
    Linear,
}

/// Specifies behavior of texture sampling when the coordinates exceed the
/// 0-1 range. Translation of `SDL_GPUSamplerAddressMode`.
#[repr(u32)]
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Default)]
pub enum SamplerAddressMode {
    /// Specifies that the coordinates will wrap around.
    #[default]
    Repeat,
    /// Specifies that the coordinates will wrap around mirrored.
    MirroredRepeat,
    /// Specifies that the coordinates will clamp to the 0-1 range.
    ClampToEdge,
}

/// Specifies the timing that will be used to present swapchain textures to
/// the OS. Translation of `SDL_GPUPresentMode`.
///
/// VSYNC mode will always be supported. IMMEDIATE and MAILBOX modes may not
/// be supported on certain systems.
#[repr(u32)]
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Default)]
pub enum PresentMode {
    /// Waits for vblank before presenting. No tearing is possible. If there
    /// is a pending image to present, the new image is enqueued for
    /// presentation. Disallows tearing at the cost of visual latency.
    #[default]
    Vsync,
    /// Immediately presents. Lowest latency option, but tearing may occur.
    Immediate,
    /// Waits for vblank before presenting. No tearing is possible. If there
    /// is a pending image to present, the pending image is replaced by the
    /// new image. Similar to VSYNC, but with reduced visual latency.
    Mailbox,
}

/// Specifies the texture format and colorspace of the swapchain textures.
/// Translation of `SDL_GPUSwapchainComposition`.
///
/// SDR will always be supported. Other compositions may not be supported
/// on certain systems.
#[repr(u32)]
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Default)]
pub enum SwapchainComposition {
    /// B8G8R8A8 or R8G8B8A8 swapchain. Pixel values are in sRGB encoding.
    #[default]
    Sdr,
    /// B8G8R8A8_SRGB or R8G8B8A8_SRGB swapchain. Pixel values are stored in
    /// memory in sRGB encoding but accessed in shaders in "linear sRGB"
    /// encoding which is sRGB but with a linear transfer function.
    SdrLinear,
    /// R16G16B16A16_FLOAT swapchain. Pixel values are in extended linear
    /// sRGB encoding and permits values outside of the [0, 1] range.
    HdrExtendedLinear,
    /// A2R10G10B10 or A2B10G10R10 swapchain. Pixel values are in BT.2020 ST2084
    /// (PQ) encoding.
    Hdr10St2084,
}

// Structures

/// A structure specifying a viewport. Translation of `SDL_GPUViewport`.
#[derive(Clone, Copy, PartialEq, Debug, Default)]
pub struct Viewport {
    /// The left offset of the viewport.
    pub x: f32,
    /// The top offset of the viewport.
    pub y: f32,
    /// The width of the viewport.
    pub w: f32,
    /// The height of the viewport.
    pub h: f32,
    /// The minimum depth of the viewport.
    pub min_depth: f32,
    /// The maximum depth of the viewport.
    pub max_depth: f32,
}

/// A structure specifying parameters related to transferring data to or
/// from a texture. Translation of `SDL_GPUTextureTransferInfo`.
///
/// If either of `pixels_per_row` or `rows_per_layer` is zero, then width
/// and height of passed [`TextureRegion`] to [`CopyPass::upload_to_texture`]
/// or [`CopyPass::download_from_texture`] are used as default values
/// respectively and data is considered to be tightly packed.
#[derive(Clone, Copy, Debug)]
pub struct TextureTransferInfo<'a> {
    /// The transfer buffer used in the transfer operation.
    pub transfer_buffer: &'a TransferBuffer,
    /// The starting byte of the image data in the transfer buffer.
    pub offset: u32,
    /// The number of pixels from one row to the next.
    pub pixels_per_row: u32,
    /// The number of rows from one layer/depth-slice to the next.
    pub rows_per_layer: u32,
}

/// A structure specifying a location in a transfer buffer. Translation of
/// `SDL_GPUTransferBufferLocation`.
#[derive(Clone, Copy, Debug)]
pub struct TransferBufferLocation<'a> {
    /// The transfer buffer used in the transfer operation.
    pub transfer_buffer: &'a TransferBuffer,
    /// The starting byte of the buffer data in the transfer buffer.
    pub offset: u32,
}

/// A structure specifying a location in a texture. Translation of
/// `SDL_GPUTextureLocation`.
#[derive(Clone, Copy, Debug)]
pub struct TextureLocation<'a> {
    /// The texture used in the copy operation.
    pub texture: &'a Texture,
    /// The mip level index of the location.
    pub mip_level: u32,
    /// The layer index of the location.
    pub layer: u32,
    /// The left offset of the location.
    pub x: u32,
    /// The top offset of the location.
    pub y: u32,
    /// The front offset of the location.
    pub z: u32,
}

/// A structure specifying a region of a texture. Translation of
/// `SDL_GPUTextureRegion`.
#[derive(Clone, Copy, Debug)]
pub struct TextureRegion<'a> {
    /// The texture used in the copy operation.
    pub texture: &'a Texture,
    /// The mip level index to transfer.
    pub mip_level: u32,
    /// The layer index to transfer.
    pub layer: u32,
    /// The left offset of the region.
    pub x: u32,
    /// The top offset of the region.
    pub y: u32,
    /// The front offset of the region.
    pub z: u32,
    /// The width of the region.
    pub w: u32,
    /// The height of the region.
    pub h: u32,
    /// The depth of the region.
    pub d: u32,
}

/// A structure specifying a region of a texture used in the blit operation.
/// Translation of `SDL_GPUBlitRegion`.
#[derive(Clone, Copy, Debug)]
pub struct BlitRegion<'a> {
    /// The texture.
    pub texture: &'a Texture,
    /// The mip level index of the region.
    pub mip_level: u32,
    /// The layer index or depth plane of the region. This value is treated
    /// as a layer index on 2D array and cube textures, and as a depth plane
    /// on 3D textures.
    pub layer_or_depth_plane: u32,
    /// The left offset of the region.
    pub x: u32,
    /// The top offset of the region.
    pub y: u32,
    /// The width of the region.
    pub w: u32,
    /// The height of the region.
    pub h: u32,
}

/// A structure specifying a location in a buffer. Translation of
/// `SDL_GPUBufferLocation`.
#[derive(Clone, Copy, Debug)]
pub struct BufferLocation<'a> {
    /// The buffer.
    pub buffer: &'a Buffer,
    /// The starting byte within the buffer.
    pub offset: u32,
}

/// A structure specifying a region of a buffer. Translation of
/// `SDL_GPUBufferRegion`.
#[derive(Clone, Copy, Debug)]
pub struct BufferRegion<'a> {
    /// The buffer.
    pub buffer: &'a Buffer,
    /// The starting byte within the buffer.
    pub offset: u32,
    /// The size in bytes of the region.
    pub size: u32,
}

/// A structure specifying the parameters of an indirect draw command, as
/// laid out in an indirect buffer. Translation of
/// `SDL_GPUIndirectDrawCommand`.
#[repr(C)]
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Default)]
pub struct IndirectDrawCommand {
    /// The number of vertices to draw.
    pub num_vertices: u32,
    /// The number of instances to draw.
    pub num_instances: u32,
    /// The index of the first vertex to draw.
    pub first_vertex: u32,
    /// The ID of the first instance to draw.
    pub first_instance: u32,
}

/// A structure specifying the parameters of an indexed indirect draw
/// command, as laid out in an indirect buffer. Translation of
/// `SDL_GPUIndexedIndirectDrawCommand`.
#[repr(C)]
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Default)]
pub struct IndexedIndirectDrawCommand {
    /// The number of indices to draw per instance.
    pub num_indices: u32,
    /// The number of instances to draw.
    pub num_instances: u32,
    /// The base index within the index buffer.
    pub first_index: u32,
    /// The value added to the vertex index before indexing into the vertex
    /// buffer.
    pub vertex_offset: i32,
    /// The ID of the first instance to draw.
    pub first_instance: u32,
}

/// A structure specifying the parameters of an indexed dispatch command, as
/// laid out in an indirect buffer. Translation of
/// `SDL_GPUIndirectDispatchCommand`.
#[repr(C)]
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Default)]
pub struct IndirectDispatchCommand {
    /// The number of local workgroups to dispatch in the X dimension.
    pub groupcount_x: u32,
    /// The number of local workgroups to dispatch in the Y dimension.
    pub groupcount_y: u32,
    /// The number of local workgroups to dispatch in the Z dimension.
    pub groupcount_z: u32,
}

// State structures

/// A structure specifying the parameters of a sampler. Translation of
/// `SDL_GPUSamplerCreateInfo`.
#[derive(Clone, Debug, Default)]
pub struct SamplerCreateInfo {
    /// The minification filter to apply to lookups.
    pub min_filter: Filter,
    /// The magnification filter to apply to lookups.
    pub mag_filter: Filter,
    /// The mipmap filter to apply to lookups.
    pub mipmap_mode: SamplerMipmapMode,
    /// The addressing mode for U coordinates outside [0, 1).
    pub address_mode_u: SamplerAddressMode,
    /// The addressing mode for V coordinates outside [0, 1).
    pub address_mode_v: SamplerAddressMode,
    /// The addressing mode for W coordinates outside [0, 1).
    pub address_mode_w: SamplerAddressMode,
    /// The bias to be added to mipmap LOD calculation.
    pub mip_lod_bias: f32,
    /// The anisotropy value clamp used by the sampler. If enable_anisotropy
    /// is false, this is ignored.
    pub max_anisotropy: f32,
    /// The comparison operator to apply to fetched data before filtering.
    pub compare_op: CompareOp,
    /// Clamps the minimum of the computed LOD value.
    pub min_lod: f32,
    /// Clamps the maximum of the computed LOD value.
    pub max_lod: f32,
    /// true to enable anisotropic filtering.
    pub enable_anisotropy: bool,
    /// true to enable comparison against a reference value during lookups.
    pub enable_compare: bool,
    /// A properties group for extensions ([`PROP_GPU_SAMPLER_CREATE_NAME_STRING`]).
    pub props: Option<Properties>,
}

/// A structure specifying the parameters of vertex buffers used in a
/// graphics pipeline. Translation of `SDL_GPUVertexBufferDescription`.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Default)]
pub struct VertexBufferDescription {
    /// The binding slot of the vertex buffer.
    pub slot: u32,
    /// The size of a single element + the offset between elements.
    pub pitch: u32,
    /// Whether attribute addressing is a function of the vertex index or
    /// instance index.
    pub input_rate: VertexInputRate,
    /// Reserved for future use. Must be set to 0.
    pub instance_step_rate: u32,
}

/// A structure specifying a vertex attribute. Translation of
/// `SDL_GPUVertexAttribute`.
///
/// All vertex attribute locations provided to a [`VertexInputState`] must
/// be unique.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct VertexAttribute {
    /// The shader input location index.
    pub location: u32,
    /// The binding slot of the associated vertex buffer.
    pub buffer_slot: u32,
    /// The size and type of the attribute data.
    pub format: VertexElementFormat,
    /// The byte offset of this attribute relative to the start of the
    /// vertex element.
    pub offset: u32,
}

/// A structure specifying the parameters of a graphics pipeline vertex
/// input state. Translation of `SDL_GPUVertexInputState`.
#[derive(Clone, Copy, Debug, Default)]
pub struct VertexInputState<'a> {
    /// The vertex buffer descriptions.
    pub vertex_buffer_descriptions: &'a [VertexBufferDescription],
    /// The vertex attribute descriptions.
    pub vertex_attributes: &'a [VertexAttribute],
}

/// A structure specifying the stencil operation state of a graphics
/// pipeline. Translation of `SDL_GPUStencilOpState`.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Default)]
pub struct StencilOpState {
    /// The action performed on samples that fail the stencil test.
    pub fail_op: StencilOp,
    /// The action performed on samples that pass the depth and stencil
    /// tests.
    pub pass_op: StencilOp,
    /// The action performed on samples that pass the stencil test and fail
    /// the depth test.
    pub depth_fail_op: StencilOp,
    /// The comparison operator used in the stencil test.
    pub compare_op: CompareOp,
}

/// A structure specifying the blend state of a color target. Translation of
/// `SDL_GPUColorTargetBlendState`.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Default)]
pub struct ColorTargetBlendState {
    /// The value to be multiplied by the source RGB value.
    pub src_color_blendfactor: BlendFactor,
    /// The value to be multiplied by the destination RGB value.
    pub dst_color_blendfactor: BlendFactor,
    /// The blend operation for the RGB components.
    pub color_blend_op: BlendOp,
    /// The value to be multiplied by the source alpha.
    pub src_alpha_blendfactor: BlendFactor,
    /// The value to be multiplied by the destination alpha.
    pub dst_alpha_blendfactor: BlendFactor,
    /// The blend operation for the alpha component.
    pub alpha_blend_op: BlendOp,
    /// A bitmask specifying which of the RGBA components are enabled for
    /// writing. Writes to all channels if enable_color_write_mask is false.
    pub color_write_mask: ColorComponentFlags,
    /// Whether blending is enabled for the color target.
    pub enable_blend: bool,
    /// Whether the color write mask is enabled.
    pub enable_color_write_mask: bool,
}

/// A structure specifying code and metadata for creating a shader object.
/// Translation of `SDL_GPUShaderCreateInfo`.
#[derive(Clone, Debug, Default)]
pub struct ShaderCreateInfo<'a> {
    /// The shader code.
    pub code: &'a [u8],
    /// The entry point function name for the shader.
    pub entrypoint: &'a str,
    /// The format of the shader code.
    pub format: ShaderFormat,
    /// The stage the shader program corresponds to.
    pub stage: ShaderStage,
    /// The number of samplers defined in the shader.
    pub num_samplers: u32,
    /// The number of storage textures defined in the shader.
    pub num_storage_textures: u32,
    /// The number of storage buffers defined in the shader.
    pub num_storage_buffers: u32,
    /// The number of uniform buffers defined in the shader.
    pub num_uniform_buffers: u32,
    /// A properties group for extensions ([`PROP_GPU_SHADER_CREATE_NAME_STRING`]).
    pub props: Option<Properties>,
}

/// A structure specifying the parameters of a texture. Translation of
/// `SDL_GPUTextureCreateInfo`.
///
/// Usage flags can be bitwise OR'd together for combinations of usages.
/// Note that certain usage combinations are invalid, for example SAMPLER
/// and GRAPHICS_STORAGE.
#[derive(Clone, Debug, Default)]
pub struct TextureCreateInfo {
    /// The base dimensionality of the texture.
    pub texture_type: TextureType,
    /// The pixel format of the texture.
    pub format: TextureFormat,
    /// How the texture is intended to be used by the client.
    pub usage: TextureUsageFlags,
    /// The width of the texture.
    pub width: u32,
    /// The height of the texture.
    pub height: u32,
    /// The layer count or depth of the texture. This value is treated as a
    /// layer count on 2D array textures, and as a depth value on 3D
    /// textures.
    pub layer_count_or_depth: u32,
    /// The number of mip levels in the texture.
    pub num_levels: u32,
    /// The number of samples per texel. Only applies if the texture is used
    /// as a render target.
    pub sample_count: SampleCount,
    /// A properties group for extensions ([`PROP_GPU_TEXTURE_CREATE_NAME_STRING`]...).
    pub props: Option<Properties>,
}

/// A structure specifying the parameters of a buffer. Translation of
/// `SDL_GPUBufferCreateInfo`.
///
/// Usage flags can be bitwise OR'd together for combinations of usages.
/// Note that certain combinations are invalid, for example VERTEX and
/// INDEX.
#[derive(Clone, Debug, Default)]
pub struct BufferCreateInfo {
    /// How the buffer is intended to be used by the client.
    pub usage: BufferUsageFlags,
    /// The size in bytes of the buffer.
    pub size: u32,
    /// A properties group for extensions ([`PROP_GPU_BUFFER_CREATE_NAME_STRING`]).
    pub props: Option<Properties>,
}

/// A structure specifying the parameters of a transfer buffer. Translation
/// of `SDL_GPUTransferBufferCreateInfo`.
#[derive(Clone, Debug, Default)]
pub struct TransferBufferCreateInfo {
    /// How the transfer buffer is intended to be used by the client.
    pub usage: TransferBufferUsage,
    /// The size in bytes of the transfer buffer.
    pub size: u32,
    /// A properties group for extensions ([`PROP_GPU_TRANSFERBUFFER_CREATE_NAME_STRING`]).
    pub props: Option<Properties>,
}

// Pipeline state structures

/// A structure specifying the parameters of the graphics pipeline
/// rasterizer state. Translation of `SDL_GPURasterizerState`.
///
/// Note that [`FillMode::Line`] is not supported on many Android devices.
/// For those devices, the fill mode will automatically fall back to FILL.
///
/// Also note that the D3D12 driver will enable depth clamping even if
/// enable_depth_clip is true. If you need this clamp+clip behavior, consider
/// enabling depth clip and then manually clamping depth in your fragment
/// shaders on Metal and Vulkan.
#[derive(Clone, Copy, PartialEq, Debug, Default)]
pub struct RasterizerState {
    /// Whether polygons will be filled in or drawn as lines.
    pub fill_mode: FillMode,
    /// The facing direction in which triangles will be culled.
    pub cull_mode: CullMode,
    /// The vertex winding that will cause a triangle to be determined as
    /// front-facing.
    pub front_face: FrontFace,
    /// A scalar factor controlling the depth value added to each fragment.
    pub depth_bias_constant_factor: f32,
    /// The maximum depth bias of a fragment.
    pub depth_bias_clamp: f32,
    /// A scalar factor applied to a fragment's slope in depth calculations.
    pub depth_bias_slope_factor: f32,
    /// true to bias fragment depth values.
    pub enable_depth_bias: bool,
    /// true to enable depth clip, false to enable depth clamp.
    pub enable_depth_clip: bool,
}

/// A structure specifying the parameters of the graphics pipeline
/// multisample state. Translation of `SDL_GPUMultisampleState`.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Default)]
pub struct MultisampleState {
    /// The number of samples to be used in rasterization.
    pub sample_count: SampleCount,
    /// Reserved for future use. Must be set to 0.
    pub sample_mask: u32,
    /// Reserved for future use. Must be set to false.
    pub enable_mask: bool,
    /// true enables the alpha-to-coverage feature.
    pub enable_alpha_to_coverage: bool,
}

/// A structure specifying the parameters of the graphics pipeline depth
/// stencil state. Translation of `SDL_GPUDepthStencilState`.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Default)]
pub struct DepthStencilState {
    /// The comparison operator used for depth testing.
    pub compare_op: CompareOp,
    /// The stencil op state for back-facing triangles.
    pub back_stencil_state: StencilOpState,
    /// The stencil op state for front-facing triangles.
    pub front_stencil_state: StencilOpState,
    /// Selects the bits of the stencil values participating in the stencil
    /// test.
    pub compare_mask: u8,
    /// Selects the bits of the stencil values updated by the stencil test.
    pub write_mask: u8,
    /// true enables the depth test.
    pub enable_depth_test: bool,
    /// true enables depth writes. Depth writes are always disabled when
    /// enable_depth_test is false.
    pub enable_depth_write: bool,
    /// true enables the stencil test.
    pub enable_stencil_test: bool,
}

/// A structure specifying the parameters of color targets used in a
/// graphics pipeline. Translation of `SDL_GPUColorTargetDescription`.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Default)]
pub struct ColorTargetDescription {
    /// The pixel format of the texture to be used as a color target.
    pub format: TextureFormat,
    /// The blend state to be used for the color target.
    pub blend_state: ColorTargetBlendState,
}

/// A structure specifying the descriptions of render targets used in a
/// graphics pipeline. Translation of `SDL_GPUGraphicsPipelineTargetInfo`.
#[derive(Clone, Copy, Debug, Default)]
pub struct GraphicsPipelineTargetInfo<'a> {
    /// The color target descriptions.
    pub color_target_descriptions: &'a [ColorTargetDescription],
    /// The pixel format of the depth-stencil target. Ignored if
    /// has_depth_stencil_target is false.
    pub depth_stencil_format: TextureFormat,
    /// true specifies that the pipeline uses a depth-stencil target.
    pub has_depth_stencil_target: bool,
}

/// A structure specifying the parameters of a graphics pipeline state.
/// Translation of `SDL_GPUGraphicsPipelineCreateInfo`.
#[derive(Clone, Debug)]
pub struct GraphicsPipelineCreateInfo<'a> {
    /// The vertex shader used by the graphics pipeline.
    pub vertex_shader: &'a Shader,
    /// The fragment shader used by the graphics pipeline.
    pub fragment_shader: &'a Shader,
    /// The vertex layout of the graphics pipeline.
    pub vertex_input_state: VertexInputState<'a>,
    /// The primitive topology of the graphics pipeline.
    pub primitive_type: PrimitiveType,
    /// The rasterizer state of the graphics pipeline.
    pub rasterizer_state: RasterizerState,
    /// The multisample state of the graphics pipeline.
    pub multisample_state: MultisampleState,
    /// The depth-stencil state of the graphics pipeline.
    pub depth_stencil_state: DepthStencilState,
    /// Formats and blend modes for the render targets of the graphics
    /// pipeline.
    pub target_info: GraphicsPipelineTargetInfo<'a>,
    /// A properties group for extensions ([`PROP_GPU_GRAPHICSPIPELINE_CREATE_NAME_STRING`]).
    pub props: Option<Properties>,
}

/// A structure specifying the parameters of a compute pipeline state.
/// Translation of `SDL_GPUComputePipelineCreateInfo`.
#[derive(Clone, Debug, Default)]
pub struct ComputePipelineCreateInfo<'a> {
    /// The compute shader code.
    pub code: &'a [u8],
    /// The entry point function name for the shader.
    pub entrypoint: &'a str,
    /// The format of the compute shader code.
    pub format: ShaderFormat,
    /// The number of samplers defined in the shader.
    pub num_samplers: u32,
    /// The number of readonly storage textures defined in the shader.
    pub num_readonly_storage_textures: u32,
    /// The number of readonly storage buffers defined in the shader.
    pub num_readonly_storage_buffers: u32,
    /// The number of read-write storage textures defined in the shader.
    pub num_readwrite_storage_textures: u32,
    /// The number of read-write storage buffers defined in the shader.
    pub num_readwrite_storage_buffers: u32,
    /// The number of uniform buffers defined in the shader.
    pub num_uniform_buffers: u32,
    /// The number of threads in the X dimension.
    pub threadcount_x: u32,
    /// The number of threads in the Y dimension.
    pub threadcount_y: u32,
    /// The number of threads in the Z dimension.
    pub threadcount_z: u32,
    /// A properties group for extensions ([`PROP_GPU_COMPUTEPIPELINE_CREATE_NAME_STRING`]).
    pub props: Option<Properties>,
}

/// A structure specifying the parameters of a color target used by a render
/// pass. Translation of `SDL_GPUColorTargetInfo`.
///
/// The load_op field determines what is done with the texture at the
/// beginning of the render pass (LOAD, CLEAR or DONT_CARE), the store_op
/// field what is done with the color results of the render pass (STORE,
/// DONT_CARE, RESOLVE or RESOLVE_AND_STORE). Cycling (`cycle`) is only
/// allowed when the load op isn't LOAD.
#[derive(Clone, Copy, Debug)]
pub struct ColorTargetInfo<'a> {
    /// The texture that will be used as a color target by a render pass.
    pub texture: &'a Texture,
    /// The mip level to use as a color target.
    pub mip_level: u32,
    /// The layer index or depth plane to use as a color target. This value
    /// is treated as a layer index on 2D array and cube textures, and as a
    /// depth plane on 3D textures.
    pub layer_or_depth_plane: u32,
    /// The color to clear the color target to at the start of the render
    /// pass. Ignored if [`LoadOp::Clear`] is not used.
    pub clear_color: FColor,
    /// What is done with the contents of the color target at the beginning
    /// of the render pass.
    pub load_op: LoadOp,
    /// What is done with the results of the render pass.
    pub store_op: StoreOp,
    /// The texture that will receive the results of a multisample resolve
    /// operation. Ignored if a RESOLVE* store_op is not used.
    pub resolve_texture: Option<&'a Texture>,
    /// The mip level of the resolve texture to use for the resolve
    /// operation. Ignored if a RESOLVE* store_op is not used.
    pub resolve_mip_level: u32,
    /// The layer index of the resolve texture to use for the resolve
    /// operation. Ignored if a RESOLVE* store_op is not used.
    pub resolve_layer: u32,
    /// true cycles the texture if the texture is bound and load_op is not
    /// LOAD.
    pub cycle: bool,
    /// true cycles the resolve texture if the resolve texture is bound.
    /// Ignored if a RESOLVE* store_op is not used.
    pub cycle_resolve_texture: bool,
}

impl<'a> ColorTargetInfo<'a> {
    /// `texture` as a color target, with the other fields as C's zeroed
    /// structure has them (load, store, no resolve texture, no cycling).
    pub fn new(texture: &'a Texture) -> ColorTargetInfo<'a> {
        ColorTargetInfo {
            texture,
            mip_level: 0,
            layer_or_depth_plane: 0,
            clear_color: FColor::default(),
            load_op: LoadOp::Load,
            store_op: StoreOp::Store,
            resolve_texture: None,
            resolve_mip_level: 0,
            resolve_layer: 0,
            cycle: false,
            cycle_resolve_texture: false,
        }
    }
}

/// A structure specifying the parameters of a depth-stencil target used by
/// a render pass. Translation of `SDL_GPUDepthStencilTargetInfo`.
///
/// Note that depth/stencil targets do not support multisample resolves.
#[derive(Clone, Copy, Debug)]
pub struct DepthStencilTargetInfo<'a> {
    /// The texture that will be used as the depth stencil target by the
    /// render pass.
    pub texture: &'a Texture,
    /// The value to clear the depth component to at the beginning of the
    /// render pass. Ignored if [`LoadOp::Clear`] is not used.
    pub clear_depth: f32,
    /// What is done with the depth contents at the beginning of the render
    /// pass.
    pub load_op: LoadOp,
    /// What is done with the depth results of the render pass.
    pub store_op: StoreOp,
    /// What is done with the stencil contents at the beginning of the render
    /// pass.
    pub stencil_load_op: LoadOp,
    /// What is done with the stencil results of the render pass.
    pub stencil_store_op: StoreOp,
    /// true cycles the texture if the texture is bound and any load ops are
    /// not LOAD
    pub cycle: bool,
    /// The value to clear the stencil component to at the beginning of the
    /// render pass. Ignored if [`LoadOp::Clear`] is not used.
    pub clear_stencil: u8,
    /// The mip level to use as the depth stencil target.
    pub mip_level: u8,
    /// The layer index to use as the depth stencil target.
    pub layer: u8,
}

impl<'a> DepthStencilTargetInfo<'a> {
    /// `texture` as a depth-stencil target, with the other fields as C's
    /// zeroed structure has them (load and store both parts, no cycling).
    pub fn new(texture: &'a Texture) -> DepthStencilTargetInfo<'a> {
        DepthStencilTargetInfo {
            texture,
            clear_depth: 0.0,
            load_op: LoadOp::Load,
            store_op: StoreOp::Store,
            stencil_load_op: LoadOp::Load,
            stencil_store_op: StoreOp::Store,
            cycle: false,
            clear_stencil: 0,
            mip_level: 0,
            layer: 0,
        }
    }
}

/// A structure containing parameters for a blit command. Translation of
/// `SDL_GPUBlitInfo`.
#[derive(Clone, Copy, Debug)]
pub struct BlitInfo<'a> {
    /// The source region for the blit.
    pub source: BlitRegion<'a>,
    /// The destination region for the blit.
    pub destination: BlitRegion<'a>,
    /// What is done with the contents of the destination before the blit.
    pub load_op: LoadOp,
    /// The color to clear the destination region to before the blit.
    /// Ignored if load_op is not [`LoadOp::Clear`].
    pub clear_color: FColor,
    /// The flip mode for the source region.
    pub flip_mode: FlipMode,
    /// The filter mode used when blitting.
    pub filter: Filter,
    /// true cycles the destination texture if it is already bound.
    pub cycle: bool,
}

// Binding structs

/// A structure specifying parameters in a buffer binding call. Translation
/// of `SDL_GPUBufferBinding`.
#[derive(Clone, Copy, Debug)]
pub struct BufferBinding<'a> {
    /// The buffer to bind. Must have been created with
    /// [`BufferUsageFlags::VERTEX`] for [`RenderPass::bind_vertex_buffers`],
    /// or [`BufferUsageFlags::INDEX`] for [`RenderPass::bind_index_buffer`].
    pub buffer: &'a Buffer,
    /// The starting byte of the data to bind in the buffer.
    pub offset: u32,
}

/// A structure specifying parameters in a sampler binding call.
/// Translation of `SDL_GPUTextureSamplerBinding`.
#[derive(Clone, Copy, Debug)]
pub struct TextureSamplerBinding<'a> {
    /// The texture to bind. Must have been created with
    /// [`TextureUsageFlags::SAMPLER`].
    pub texture: &'a Texture,
    /// The sampler to bind.
    pub sampler: &'a Sampler,
}

/// A structure specifying parameters related to binding buffers in a
/// compute pass. Translation of `SDL_GPUStorageBufferReadWriteBinding`.
#[derive(Clone, Copy, Debug)]
pub struct StorageBufferReadWriteBinding<'a> {
    /// The buffer to bind. Must have been created with
    /// [`BufferUsageFlags::COMPUTE_STORAGE_WRITE`].
    pub buffer: &'a Buffer,
    /// true cycles the buffer if it is already bound.
    pub cycle: bool,
}

/// A structure specifying parameters related to binding textures in a
/// compute pass. Translation of `SDL_GPUStorageTextureReadWriteBinding`.
#[derive(Clone, Copy, Debug)]
pub struct StorageTextureReadWriteBinding<'a> {
    /// The texture to bind. Must have been created with
    /// [`TextureUsageFlags::COMPUTE_STORAGE_WRITE`] or
    /// [`TextureUsageFlags::COMPUTE_STORAGE_SIMULTANEOUS_READ_WRITE`].
    pub texture: &'a Texture,
    /// The mip level index to bind.
    pub mip_level: u32,
    /// The layer index to bind.
    pub layer: u32,
    /// true cycles the texture if it is already bound.
    pub cycle: bool,
}

/// A Vulkan feature structure in [`VulkanOptions::feature_list`]: its
/// `sType` and its `VkBool32` members (after `sType` and `pNext`), in
/// declaration order. Translation of an element of the
/// `SDL_GPUVulkanOptions::feature_list` chain (`VkBaseOutStructure`).
///
/// The structures taken are `VkPhysicalDeviceFeatures2` (whose members are
/// the 55 of its `VkPhysicalDeviceFeatures`), the Vulkan 1.1 ones
/// (`VkPhysicalDevice16BitStorageFeatures`,
/// `VkPhysicalDeviceMultiviewFeatures`,
/// `VkPhysicalDeviceProtectedMemoryFeatures`,
/// `VkPhysicalDeviceSamplerYcbcrConversionFeatures`,
/// `VkPhysicalDeviceShaderDrawParametersFeatures`,
/// `VkPhysicalDeviceVariablePointersFeatures`) and, with API version 1.2 or
/// higher, `VkPhysicalDeviceVulkan11Features`,
/// `VkPhysicalDeviceVulkan12Features` and (1.3) `VkPhysicalDeviceVulkan13Features`;
/// others are ignored. Members past the end of `features` are false.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct VulkanFeatureStructure {
    /// The structure's `VkStructureType`.
    pub s_type: i32,
    /// The structure's features, in order.
    pub features: Vec<bool>,
}

/// A structure specifying additional options when using Vulkan, for
/// [`PROP_GPU_DEVICE_CREATE_VULKAN_OPTIONS_POINTER`] (set it with
/// [`Properties::set_any`]). Translation of `SDL_GPUVulkanOptions`.
///
/// When no such structure is provided, SDL will use Vulkan API version 1.0
/// and a minimal set of features. The requested API version influences how
/// the feature_list is processed by SDL. When requesting API version 1.0,
/// the feature_list is ignored. Only the vulkan_10_physical_device_features
/// and the extension lists are used. When requesting API version 1.1, the
/// feature_list is scanned for feature structures introduced in Vulkan 1.1.
/// When requesting Vulkan 1.2 or higher, the feature_list is additionally
/// scanned for compound feature structs such as
/// VkPhysicalDeviceVulkan11Features. The device and instance extension
/// lists, as well as vulkan_10_physical_device_features, are always
/// processed.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct VulkanOptions {
    /// The Vulkan API version to request for the instance. Use Vulkan's
    /// VK_MAKE_VERSION or VK_MAKE_API_VERSION.
    pub vulkan_api_version: u32,
    /// Vulkan feature structs to enable. (Requires API version 1.1 or
    /// higher.)
    pub feature_list: Vec<VulkanFeatureStructure>,
    /// The `VkBool32` members of a `VkPhysicalDeviceFeatures`, in order, to
    /// enable additional Vulkan 1.0 features.
    pub vulkan_10_physical_device_features: Option<Vec<bool>>,
    /// Additional device extensions to require.
    pub device_extension_names: Vec<String>,
    /// Additional instance extensions to require.
    pub instance_extension_names: Vec<String>,
}

// Properties

/// Enable debug mode properties and validations, defaults to true.
/// Translation of `SDL_PROP_GPU_DEVICE_CREATE_DEBUGMODE_BOOLEAN`.
pub const PROP_GPU_DEVICE_CREATE_DEBUGMODE_BOOLEAN: &str = "SDL.gpu.device.create.debugmode";
/// Enable to prefer energy efficiency over maximum GPU performance,
/// defaults to false. Translation of
/// `SDL_PROP_GPU_DEVICE_CREATE_PREFERLOWPOWER_BOOLEAN`.
pub const PROP_GPU_DEVICE_CREATE_PREFERLOWPOWER_BOOLEAN: &str =
    "SDL.gpu.device.create.preferlowpower";
/// Enable to automatically log useful debug information on device creation,
/// defaults to true. Translation of
/// `SDL_PROP_GPU_DEVICE_CREATE_VERBOSE_BOOLEAN`.
pub const PROP_GPU_DEVICE_CREATE_VERBOSE_BOOLEAN: &str = "SDL.gpu.device.create.verbose";
/// The name of the GPU driver to use, if a specific one is desired.
/// Translation of `SDL_PROP_GPU_DEVICE_CREATE_NAME_STRING`.
pub const PROP_GPU_DEVICE_CREATE_NAME_STRING: &str = "SDL.gpu.device.create.name";
/// Enable Vulkan device feature shaderClipDistance. If disabled, clip
/// distances are not supported in shader code: gl_ClipDistance[] built-ins
/// of GLSL, SV_ClipDistance0/1 semantics of HLSL and `[[clip_distance]]`
/// attribute of Metal. Disabling optional features allows the application
/// to run on some older Android devices. Defaults to true. Translation of
/// `SDL_PROP_GPU_DEVICE_CREATE_FEATURE_CLIP_DISTANCE_BOOLEAN`.
pub const PROP_GPU_DEVICE_CREATE_FEATURE_CLIP_DISTANCE_BOOLEAN: &str =
    "SDL.gpu.device.create.feature.clip_distance";
/// Enable Vulkan device feature depthClamp. If disabled, there is no depth
/// clamp support and enable_depth_clip in [`RasterizerState`] must always
/// be set to true. Disabling optional features allows the application to
/// run on some older Android devices. Defaults to true. Translation of
/// `SDL_PROP_GPU_DEVICE_CREATE_FEATURE_DEPTH_CLAMPING_BOOLEAN`.
pub const PROP_GPU_DEVICE_CREATE_FEATURE_DEPTH_CLAMPING_BOOLEAN: &str =
    "SDL.gpu.device.create.feature.depth_clamping";
/// Enable Vulkan device feature drawIndirectFirstInstance. If disabled, the
/// argument first_instance of [`IndirectDrawCommand`] must be set to zero.
/// Disabling optional features allows the application to run on some older
/// Android devices. Defaults to true. Translation of
/// `SDL_PROP_GPU_DEVICE_CREATE_FEATURE_INDIRECT_DRAW_FIRST_INSTANCE_BOOLEAN`.
pub const PROP_GPU_DEVICE_CREATE_FEATURE_INDIRECT_DRAW_FIRST_INSTANCE_BOOLEAN: &str =
    "SDL.gpu.device.create.feature.indirect_draw_first_instance";
/// Enable Vulkan device feature samplerAnisotropy. If disabled,
/// enable_anisotropy of [`SamplerCreateInfo`] must be set to false.
/// Disabling optional features allows the application to run on some older
/// Android devices. Defaults to true. Translation of
/// `SDL_PROP_GPU_DEVICE_CREATE_FEATURE_ANISOTROPY_BOOLEAN`.
pub const PROP_GPU_DEVICE_CREATE_FEATURE_ANISOTROPY_BOOLEAN: &str =
    "SDL.gpu.device.create.feature.anisotropy";
/// The app is able to provide shaders for an NDA platform. Translation of
/// `SDL_PROP_GPU_DEVICE_CREATE_SHADERS_PRIVATE_BOOLEAN`.
pub const PROP_GPU_DEVICE_CREATE_SHADERS_PRIVATE_BOOLEAN: &str =
    "SDL.gpu.device.create.shaders.private";
/// The app is able to provide SPIR-V shaders if applicable. Translation of
/// `SDL_PROP_GPU_DEVICE_CREATE_SHADERS_SPIRV_BOOLEAN`.
pub const PROP_GPU_DEVICE_CREATE_SHADERS_SPIRV_BOOLEAN: &str =
    "SDL.gpu.device.create.shaders.spirv";
/// The app is able to provide DXBC shaders if applicable. Translation of
/// `SDL_PROP_GPU_DEVICE_CREATE_SHADERS_DXBC_BOOLEAN`.
pub const PROP_GPU_DEVICE_CREATE_SHADERS_DXBC_BOOLEAN: &str = "SDL.gpu.device.create.shaders.dxbc";
/// The app is able to provide DXIL shaders if applicable. Translation of
/// `SDL_PROP_GPU_DEVICE_CREATE_SHADERS_DXIL_BOOLEAN`.
pub const PROP_GPU_DEVICE_CREATE_SHADERS_DXIL_BOOLEAN: &str = "SDL.gpu.device.create.shaders.dxil";
/// The app is able to provide MSL shaders if applicable. Translation of
/// `SDL_PROP_GPU_DEVICE_CREATE_SHADERS_MSL_BOOLEAN`.
pub const PROP_GPU_DEVICE_CREATE_SHADERS_MSL_BOOLEAN: &str = "SDL.gpu.device.create.shaders.msl";
/// The app is able to provide Metal shader libraries if applicable.
/// Translation of `SDL_PROP_GPU_DEVICE_CREATE_SHADERS_METALLIB_BOOLEAN`.
pub const PROP_GPU_DEVICE_CREATE_SHADERS_METALLIB_BOOLEAN: &str =
    "SDL.gpu.device.create.shaders.metallib";
/// Allows the D3D12 backend to run on hardware with fewer resource slots,
/// defaults to false. Translation of
/// `SDL_PROP_GPU_DEVICE_CREATE_D3D12_ALLOW_FEWER_RESOURCE_SLOTS_BOOLEAN`.
pub const PROP_GPU_DEVICE_CREATE_D3D12_ALLOW_FEWER_RESOURCE_SLOTS_BOOLEAN: &str =
    "SDL.gpu.device.create.d3d12.allowtier1resourcebinding";
/// The prefix to use for all vertex semantics, default is "TEXCOORD".
/// Translation of `SDL_PROP_GPU_DEVICE_CREATE_D3D12_SEMANTIC_NAME_STRING`.
pub const PROP_GPU_DEVICE_CREATE_D3D12_SEMANTIC_NAME_STRING: &str =
    "SDL.gpu.device.create.d3d12.semantic";
/// Certain feature checks are only possible on Windows 11 by default. By
/// setting this alongside the path, the D3D12 Agility SDK can be used on
/// Windows 10. Translation of
/// `SDL_PROP_GPU_DEVICE_CREATE_D3D12_AGILITY_SDK_VERSION_NUMBER`.
pub const PROP_GPU_DEVICE_CREATE_D3D12_AGILITY_SDK_VERSION_NUMBER: &str =
    "SDL.gpu.device.create.d3d12.agility_sdk_version";
/// The path to the D3D12 Agility SDK. Translation of
/// `SDL_PROP_GPU_DEVICE_CREATE_D3D12_AGILITY_SDK_PATH_STRING`.
pub const PROP_GPU_DEVICE_CREATE_D3D12_AGILITY_SDK_PATH_STRING: &str =
    "SDL.gpu.device.create.d3d12.agility_sdk_path";
/// By default, Vulkan device enumeration includes drivers of all types,
/// including software renderers (for example, the Lavapipe Mesa driver).
/// This can be useful if your application _requires_ SDL_GPU, but if you
/// can provide your own fallback renderer, you may want to set this
/// property to true. Translation of
/// `SDL_PROP_GPU_DEVICE_CREATE_VULKAN_REQUIRE_HARDWARE_ACCELERATION_BOOLEAN`.
pub const PROP_GPU_DEVICE_CREATE_VULKAN_REQUIRE_HARDWARE_ACCELERATION_BOOLEAN: &str =
    "SDL.gpu.device.create.vulkan.requirehardwareacceleration";
/// Additional Vulkan instance and device options (`SDL_GPUVulkanOptions`).
/// Translation of `SDL_PROP_GPU_DEVICE_CREATE_VULKAN_OPTIONS_POINTER`.
pub const PROP_GPU_DEVICE_CREATE_VULKAN_OPTIONS_POINTER: &str =
    "SDL.gpu.device.create.vulkan.options";
/// Enable Metal on macOS 10.14 GPU family 1. Translation of
/// `SDL_PROP_GPU_DEVICE_CREATE_METAL_ALLOW_MACFAMILY1_BOOLEAN`.
pub const PROP_GPU_DEVICE_CREATE_METAL_ALLOW_MACFAMILY1_BOOLEAN: &str =
    "SDL.gpu.device.create.metal.allowmacfamily1";
/// Enable OpenXR support for this GPU device, defaults to false (OpenXR is
/// not translated: setting it makes device creation fail). Translation of
/// `SDL_PROP_GPU_DEVICE_CREATE_XR_ENABLE_BOOLEAN`.
pub const PROP_GPU_DEVICE_CREATE_XR_ENABLE_BOOLEAN: &str = "SDL.gpu.device.create.xr.enable";

/// Contains the name of the underlying device as reported by the system
/// driver. Translation of `SDL_PROP_GPU_DEVICE_NAME_STRING`.
pub const PROP_GPU_DEVICE_NAME_STRING: &str = "SDL.gpu.device.name";
/// Contains the self-reported name of the underlying system driver.
/// Translation of `SDL_PROP_GPU_DEVICE_DRIVER_NAME_STRING`.
pub const PROP_GPU_DEVICE_DRIVER_NAME_STRING: &str = "SDL.gpu.device.driver_name";
/// Contains the self-reported version of the underlying system driver.
/// Translation of `SDL_PROP_GPU_DEVICE_DRIVER_VERSION_STRING`.
pub const PROP_GPU_DEVICE_DRIVER_VERSION_STRING: &str = "SDL.gpu.device.driver_version";
/// Contains the detailed version information of the underlying system
/// driver as reported by the driver. Translation of
/// `SDL_PROP_GPU_DEVICE_DRIVER_INFO_STRING`.
pub const PROP_GPU_DEVICE_DRIVER_INFO_STRING: &str = "SDL.gpu.device.driver_info";

/// A name that can be displayed in debugging tools. Translation of
/// `SDL_PROP_GPU_COMPUTEPIPELINE_CREATE_NAME_STRING`.
pub const PROP_GPU_COMPUTEPIPELINE_CREATE_NAME_STRING: &str = "SDL.gpu.computepipeline.create.name";
/// A name that can be displayed in debugging tools. Translation of
/// `SDL_PROP_GPU_GRAPHICSPIPELINE_CREATE_NAME_STRING`.
pub const PROP_GPU_GRAPHICSPIPELINE_CREATE_NAME_STRING: &str =
    "SDL.gpu.graphicspipeline.create.name";
/// A name that can be displayed in debugging tools. Translation of
/// `SDL_PROP_GPU_SAMPLER_CREATE_NAME_STRING`.
pub const PROP_GPU_SAMPLER_CREATE_NAME_STRING: &str = "SDL.gpu.sampler.create.name";
/// A name that can be displayed in debugging tools. Translation of
/// `SDL_PROP_GPU_SHADER_CREATE_NAME_STRING`.
pub const PROP_GPU_SHADER_CREATE_NAME_STRING: &str = "SDL.gpu.shader.create.name";
/// (Direct3D 12 only) if the texture usage is COLOR_TARGET, clear to this
/// red intensity. Defaults to zero. Translation of
/// `SDL_PROP_GPU_TEXTURE_CREATE_D3D12_CLEAR_R_FLOAT`.
pub const PROP_GPU_TEXTURE_CREATE_D3D12_CLEAR_R_FLOAT: &str =
    "SDL.gpu.texture.create.d3d12.clear.r";
/// (Direct3D 12 only) if the texture usage is COLOR_TARGET, clear to this
/// green intensity. Defaults to zero. Translation of
/// `SDL_PROP_GPU_TEXTURE_CREATE_D3D12_CLEAR_G_FLOAT`.
pub const PROP_GPU_TEXTURE_CREATE_D3D12_CLEAR_G_FLOAT: &str =
    "SDL.gpu.texture.create.d3d12.clear.g";
/// (Direct3D 12 only) if the texture usage is COLOR_TARGET, clear to this
/// blue intensity. Defaults to zero. Translation of
/// `SDL_PROP_GPU_TEXTURE_CREATE_D3D12_CLEAR_B_FLOAT`.
pub const PROP_GPU_TEXTURE_CREATE_D3D12_CLEAR_B_FLOAT: &str =
    "SDL.gpu.texture.create.d3d12.clear.b";
/// (Direct3D 12 only) if the texture usage is COLOR_TARGET, clear to this
/// alpha intensity. Defaults to zero. Translation of
/// `SDL_PROP_GPU_TEXTURE_CREATE_D3D12_CLEAR_A_FLOAT`.
pub const PROP_GPU_TEXTURE_CREATE_D3D12_CLEAR_A_FLOAT: &str =
    "SDL.gpu.texture.create.d3d12.clear.a";
/// (Direct3D 12 only) if the texture usage is DEPTH_STENCIL_TARGET, clear
/// the texture to a depth of this value. Defaults to zero. Translation of
/// `SDL_PROP_GPU_TEXTURE_CREATE_D3D12_CLEAR_DEPTH_FLOAT`.
pub const PROP_GPU_TEXTURE_CREATE_D3D12_CLEAR_DEPTH_FLOAT: &str =
    "SDL.gpu.texture.create.d3d12.clear.depth";
/// (Direct3D 12 only) if the texture usage is DEPTH_STENCIL_TARGET, clear
/// the texture to a stencil of this value. Defaults to zero. Translation of
/// `SDL_PROP_GPU_TEXTURE_CREATE_D3D12_CLEAR_STENCIL_NUMBER`.
pub const PROP_GPU_TEXTURE_CREATE_D3D12_CLEAR_STENCIL_NUMBER: &str =
    "SDL.gpu.texture.create.d3d12.clear.stencil";
/// A name that can be displayed in debugging tools. Translation of
/// `SDL_PROP_GPU_TEXTURE_CREATE_NAME_STRING`.
pub const PROP_GPU_TEXTURE_CREATE_NAME_STRING: &str = "SDL.gpu.texture.create.name";
/// A name that can be displayed in debugging tools. Translation of
/// `SDL_PROP_GPU_BUFFER_CREATE_NAME_STRING`.
pub const PROP_GPU_BUFFER_CREATE_NAME_STRING: &str = "SDL.gpu.buffer.create.name";
/// A name that can be displayed in debugging tools. Translation of
/// `SDL_PROP_GPU_TRANSFERBUFFER_CREATE_NAME_STRING`.
pub const PROP_GPU_TRANSFERBUFFER_CREATE_NAME_STRING: &str = "SDL.gpu.transferbuffer.create.name";

// Front-end tables

/// Whether a texture format can be written by compute shaders, by format.
/// Translation of `TextureFormatIsComputeWritable`.
static TEXTURE_FORMAT_IS_COMPUTE_WRITABLE: [bool; TEXTUREFORMAT_MAX_ENUM_VALUE as usize] = [
    false, // INVALID
    false, // A8_UNORM
    true,  // R8_UNORM
    true,  // R8G8_UNORM
    true,  // R8G8B8A8_UNORM
    true,  // R16_UNORM
    true,  // R16G16_UNORM
    true,  // R16G16B16A16_UNORM
    true,  // R10G10B10A2_UNORM
    false, // B5G6R5_UNORM
    false, // B5G5R5A1_UNORM
    false, // B4G4R4A4_UNORM
    false, // B8G8R8A8_UNORM
    false, // BC1_UNORM
    false, // BC2_UNORM
    false, // BC3_UNORM
    false, // BC4_UNORM
    false, // BC5_UNORM
    false, // BC7_UNORM
    false, // BC6H_FLOAT
    false, // BC6H_UFLOAT
    true,  // R8_SNORM
    true,  // R8G8_SNORM
    true,  // R8G8B8A8_SNORM
    true,  // R16_SNORM
    true,  // R16G16_SNORM
    true,  // R16G16B16A16_SNORM
    true,  // R16_FLOAT
    true,  // R16G16_FLOAT
    true,  // R16G16B16A16_FLOAT
    true,  // R32_FLOAT
    true,  // R32G32_FLOAT
    true,  // R32G32B32A32_FLOAT
    true,  // R11G11B10_UFLOAT
    true,  // R8_UINT
    true,  // R8G8_UINT
    true,  // R8G8B8A8_UINT
    true,  // R16_UINT
    true,  // R16G16_UINT
    true,  // R16G16B16A16_UINT
    true,  // R32_UINT
    true,  // R32G32_UINT
    true,  // R32G32B32A32_UINT
    true,  // R8_INT
    true,  // R8G8_INT
    true,  // R8G8B8A8_INT
    true,  // R16_INT
    true,  // R16G16_INT
    true,  // R16G16B16A16_INT
    true,  // R32_INT
    true,  // R32G32_INT
    true,  // R32G32B32A32_INT
    false, // R8G8B8A8_UNORM_SRGB
    false, // B8G8R8A8_UNORM_SRGB
    false, // BC1_UNORM_SRGB
    false, // BC3_UNORM_SRGB
    false, // BC3_UNORM_SRGB
    false, // BC7_UNORM_SRGB
    false, // D16_UNORM
    false, // D24_UNORM
    false, // D32_FLOAT
    false, // D24_UNORM_S8_UINT
    false, // D32_FLOAT_S8_UINT
    false, // ASTC_4x4_UNORM
    false, // ASTC_5x4_UNORM
    false, // ASTC_5x5_UNORM
    false, // ASTC_6x5_UNORM
    false, // ASTC_6x6_UNORM
    false, // ASTC_8x5_UNORM
    false, // ASTC_8x6_UNORM
    false, // ASTC_8x8_UNORM
    false, // ASTC_10x5_UNORM
    false, // ASTC_10x6_UNORM
    false, // ASTC_10x8_UNORM
    false, // ASTC_10x10_UNORM
    false, // ASTC_12x10_UNORM
    false, // ASTC_12x12_UNORM
    false, // ASTC_4x4_UNORM_SRGB
    false, // ASTC_5x4_UNORM_SRGB
    false, // ASTC_5x5_UNORM_SRGB
    false, // ASTC_6x5_UNORM_SRGB
    false, // ASTC_6x6_UNORM_SRGB
    false, // ASTC_8x5_UNORM_SRGB
    false, // ASTC_8x6_UNORM_SRGB
    false, // ASTC_8x8_UNORM_SRGB
    false, // ASTC_10x5_UNORM_SRGB
    false, // ASTC_10x6_UNORM_SRGB
    false, // ASTC_10x8_UNORM_SRGB
    false, // ASTC_10x10_UNORM_SRGB
    false, // ASTC_12x10_UNORM_SRGB
    false, // ASTC_12x12_UNORM_SRGB
    false, // ASTC_4x4_FLOAT
    false, // ASTC_5x4_FLOAT
    false, // ASTC_5x5_FLOAT
    false, // ASTC_6x5_FLOAT
    false, // ASTC_6x6_FLOAT
    false, // ASTC_8x5_FLOAT
    false, // ASTC_8x6_FLOAT
    false, // ASTC_8x8_FLOAT
    false, // ASTC_10x5_FLOAT
    false, // ASTC_10x6_FLOAT
    false, // ASTC_10x8_FLOAT
    false, // ASTC_10x10_FLOAT
    false, // ASTC_12x10_FLOAT
    false, // ASTC_12x12_FLOAT
];

impl TextureFormat {
    /// Whether compute shaders can write the format
    /// (`TextureFormatIsComputeWritable[format]`).
    ///
    /// Note (upstream): C reads past the table for a format out of range
    /// when debug mode doesn't reject it first; that is false here.
    pub(crate) fn is_compute_writable(self) -> bool {
        TEXTURE_FORMAT_IS_COMPUTE_WRITABLE
            .get(self.0 as usize)
            .copied()
            .unwrap_or(false)
    }

    /// `CHECK_TEXTUREFORMAT_ENUM_INVALID`: whether the format is out of the
    /// enum's range (or INVALID).
    fn is_invalid_enum(self) -> bool {
        self.0 == TextureFormat::INVALID.0 || self.0 >= TEXTUREFORMAT_MAX_ENUM_VALUE
    }
}

// Drivers

/// The GPU backends compiled in, in order of preference (`backends[]`).
///
/// Upstream's list is the private (console) driver, Metal, Direct3D 12 and
/// Vulkan; only Vulkan is translated yet (its devices and resources: see
/// [`vulkan`]).
static BACKENDS: &[&GpuBootstrap] = &[&vulkan::VULKAN_DRIVER];

/// The backends tests use in place of [`BACKENDS`], if set.
#[cfg(test)]
pub(crate) static TEST_BACKENDS: std::sync::Mutex<Option<Vec<&'static GpuBootstrap>>> =
    std::sync::Mutex::new(None);

/// The backend list.
fn backends() -> Vec<&'static GpuBootstrap> {
    #[cfg(test)]
    {
        let test = TEST_BACKENDS.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(list) = test.as_ref() {
            return list.clone();
        }
    }
    BACKENDS.to_vec()
}

/// Translation of `SDL_GPUSelectBackend()`.
fn select_backend(props: &Properties) -> Result<&'static GpuBootstrap> {
    let video = match crate::video::core::driver() {
        Ok(video) => video,
        Err(_) => return Err(Error::new("Video subsystem not initialized")),
    };

    // (HAVE_GPU_OPENXR: OpenXR is not translated.)
    if props
        .get_bool(PROP_GPU_DEVICE_CREATE_XR_ENABLE_BOOLEAN)
        .unwrap_or(false)
    {
        return Err(Error::new("OpenXR is not enabled in this build of SDL"));
    }

    let gpudriver = hints::get(hints::GPU_DRIVER)
        .or_else(|| props.get_string(PROP_GPU_DEVICE_CREATE_NAME_STRING));

    let backends = backends();

    // Environment/Properties override...
    if let Some(gpudriver) = gpudriver {
        for backend in &backends {
            if crate::stdlib::string::strcasecmp(&gpudriver, backend.name).is_eq()
                && (backend.prepare_driver)(&*video, props)
            {
                return Ok(backend);
            }
        }

        return Err(Error::new(format!(
            "SDL_HINT_GPU_DRIVER {gpudriver} unsupported!"
        )));
    }

    for backend in &backends {
        if (backend.prepare_driver)(&*video, props) {
            return Ok(backend);
        }
    }

    Err(Error::new("No supported SDL_GPU backend found!"))
}

/// The creation properties for `format_flags`, `debug_mode` and `name`.
/// Translation of `SDL_GPU_FillProperties()`.
fn fill_properties(
    props: &Properties,
    format_flags: ShaderFormat,
    debug_mode: bool,
    name: Option<&str>,
) -> Result<()> {
    if format_flags.contains(ShaderFormat::PRIVATE) {
        props.set(PROP_GPU_DEVICE_CREATE_SHADERS_PRIVATE_BOOLEAN, true)?;
    }
    if format_flags.contains(ShaderFormat::SPIRV) {
        props.set(PROP_GPU_DEVICE_CREATE_SHADERS_SPIRV_BOOLEAN, true)?;
    }
    if format_flags.contains(ShaderFormat::DXBC) {
        props.set(PROP_GPU_DEVICE_CREATE_SHADERS_DXBC_BOOLEAN, true)?;
    }
    if format_flags.contains(ShaderFormat::DXIL) {
        props.set(PROP_GPU_DEVICE_CREATE_SHADERS_DXIL_BOOLEAN, true)?;
    }
    if format_flags.contains(ShaderFormat::MSL) {
        props.set(PROP_GPU_DEVICE_CREATE_SHADERS_MSL_BOOLEAN, true)?;
    }
    if format_flags.contains(ShaderFormat::METALLIB) {
        props.set(PROP_GPU_DEVICE_CREATE_SHADERS_METALLIB_BOOLEAN, true)?;
    }
    props.set(PROP_GPU_DEVICE_CREATE_DEBUGMODE_BOOLEAN, debug_mode)?;
    if let Some(name) = name {
        props.set(PROP_GPU_DEVICE_CREATE_NAME_STRING, name)?;
    }
    Ok(())
}

/// Whether a backend works for the given shader formats (and, with `name`,
/// whether that backend does). Translation of
/// `SDL_GPUSupportsShaderFormats()`.
pub fn supports_shader_formats(format_flags: ShaderFormat, name: Option<&str>) -> bool {
    let props = Properties::new();
    if fill_properties(&props, format_flags, false, name).is_err() {
        return false;
    }
    supports_properties(&props)
}

/// Whether a backend works with the given creation properties (the
/// `SDL.gpu.device.create.*` properties of [`Device::with_properties`]).
/// Translation of `SDL_GPUSupportsProperties()`.
pub fn supports_properties(props: &Properties) -> bool {
    select_backend(props).is_ok()
}

/// The number of GPU drivers compiled in. Translation of
/// `SDL_GetNumGPUDrivers()`.
pub fn num_gpu_drivers() -> usize {
    backends().len()
}

/// The name of a built in GPU driver ("vulkan", "direct3d12", "metal"...).
/// Translation of `SDL_GetGPUDriver()`.
pub fn gpu_driver(index: usize) -> Result<&'static str> {
    backends()
        .get(index)
        .map(|b| b.name)
        .ok_or_else(|| Error::invalid_param("index"))
}

// The device

/// What a device and every object made from it share: the backend driver
/// and the front end's `SDL_GPUDevice` fields.
pub(crate) struct DeviceShared {
    pub(crate) driver: Box<dyn GpuDriver>,

    /// Store this for SDL_GetGPUDeviceDriver()
    backend: &'static str,

    /// Store this for SDL_GetGPUShaderFormats()
    shader_formats: ShaderFormat,

    // Store this for SDL_gpu.c's debug layer
    pub(crate) debug_mode: bool,
    pub(crate) default_enable_depth_clip: bool,
    validate_feature_depth_clamp_disabled: bool,
    validate_feature_anisotropy_disabled: bool,
    /// The smallest color target of the last render pass begun on any
    /// command buffer (upstream keeps these in the device).
    max_viewport_width: AtomicU32,
    max_viewport_height: AtomicU32,
}

impl Drop for DeviceShared {
    fn drop(&mut self) {
        self.driver.destroy();
    }
}

impl std::fmt::Debug for DeviceShared {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DeviceShared")
            .field("backend", &self.backend)
            .field("shader_formats", &self.shader_formats)
            .field("debug_mode", &self.debug_mode)
            .finish_non_exhaustive()
    }
}

/// Who a handle belongs to.
#[derive(Clone)]
enum Owner {
    /// Created by the application: released when dropped.
    Device(Arc<DeviceShared>),
    /// A swapchain's texture: never released by the front end.
    Swapchain(Arc<DeviceShared>),
    /// Created by a backend for its own use (blit pipelines and their
    /// shaders and samplers): the backend releases it.
    Backend,
}

impl Owner {
    fn device(&self) -> Option<&Arc<DeviceShared>> {
        match self {
            Owner::Device(device) | Owner::Swapchain(device) => Some(device),
            Owner::Backend => None,
        }
    }

    /// The device of an application handle, to release it with.
    fn releasing(&self) -> Option<&Arc<DeviceShared>> {
        match self {
            Owner::Device(device) => Some(device),
            _ => None,
        }
    }
}

impl std::fmt::Debug for Owner {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Owner::Device(_) => "Device",
            Owner::Swapchain(_) => "Swapchain",
            Owner::Backend => "Backend",
        })
    }
}

/// A GPU context. Translation of `SDL_GPUDevice *`.
///
/// Dropping it destroys the device (`SDL_DestroyGPUDevice()`) once every
/// resource, fence and command buffer created from it is dropped too.
pub struct Device {
    shared: Arc<DeviceShared>,
}

impl std::fmt::Debug for Device {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Device")
            .field("driver", &self.shared.backend)
            .field("debug_mode", &self.shared.debug_mode)
            .finish_non_exhaustive()
    }
}

/// A buffer for vertices, indices, indirect draw commands or storage data.
/// Translation of `SDL_GPUBuffer *`; dropping it releases it
/// (`SDL_ReleaseGPUBuffer()`).
pub struct Buffer {
    owner: Owner,
    pub(crate) raw: BackendObject,
}

/// A buffer for transferring data to and from the GPU. Translation of
/// `SDL_GPUTransferBuffer *`; dropping it releases it
/// (`SDL_ReleaseGPUTransferBuffer()`).
pub struct TransferBuffer {
    owner: Owner,
    pub(crate) raw: BackendObject,
    size: u32,
}

/// A texture. Translation of `SDL_GPUTexture *`; dropping it releases it
/// (`SDL_ReleaseGPUTexture()`), except for a swapchain's texture, which
/// belongs to the swapchain.
pub struct Texture {
    owner: Owner,
    pub(crate) raw: BackendObject,
    /// The texture's creation info (`TextureCommonHeader`).
    pub(crate) info: TextureCreateInfo,
}

/// A sampler. Translation of `SDL_GPUSampler *`; dropping it releases it
/// (`SDL_ReleaseGPUSampler()`).
pub struct Sampler {
    owner: Owner,
    pub(crate) raw: BackendObject,
}

/// A compiled shader, used to create pipelines. Translation of
/// `SDL_GPUShader *`; dropping it releases it (`SDL_ReleaseGPUShader()`),
/// which is fine once the pipelines are created.
pub struct Shader {
    owner: Owner,
    pub(crate) raw: BackendObject,
}

/// A compute pipeline. Translation of `SDL_GPUComputePipeline *`; dropping
/// it releases it (`SDL_ReleaseGPUComputePipeline()`).
pub struct ComputePipeline {
    owner: Owner,
    pub(crate) raw: BackendObject,
    pub(crate) header: ComputePipelineHeader,
}

/// A graphics pipeline. Translation of `SDL_GPUGraphicsPipeline *`;
/// dropping it releases it (`SDL_ReleaseGPUGraphicsPipeline()`).
pub struct GraphicsPipeline {
    owner: Owner,
    pub(crate) raw: BackendObject,
    pub(crate) header: GraphicsPipelineHeader,
}

/// A fence, signaled when the command buffer it was acquired with is done.
/// Translation of `SDL_GPUFence *`; dropping it releases it
/// (`SDL_ReleaseGPUFence()`).
pub struct Fence {
    owner: Owner,
    pub(crate) raw: BackendObject,
}

macro_rules! handle_impls {
    ($($ty:ident => $release:ident;)*) => {
        $(
            impl Drop for $ty {
                fn drop(&mut self) {
                    if let Some(device) = self.owner.releasing() {
                        device.driver.$release(&self.raw);
                    }
                }
            }

            impl std::fmt::Debug for $ty {
                fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                    f.debug_struct(stringify!($ty))
                        .field("owner", &self.owner)
                        .field("raw", &self.raw)
                        .finish_non_exhaustive()
                }
            }
        )*
    };
}

handle_impls! {
    Buffer => release_buffer;
    TransferBuffer => release_transfer_buffer;
    Texture => release_texture;
    Sampler => release_sampler;
    Shader => release_shader;
    ComputePipeline => release_compute_pipeline;
    GraphicsPipeline => release_graphics_pipeline;
    Fence => release_fence;
}

impl Shader {
    /// A shader a backend made for its own use (which it releases itself).
    #[allow(dead_code)] // (for the backends)
    pub(crate) fn from_backend(raw: BackendObject) -> Shader {
        Shader {
            owner: Owner::Backend,
            raw,
        }
    }
}

impl Sampler {
    /// A sampler a backend made for its own use (which it releases itself).
    #[allow(dead_code)] // (for the backends)
    pub(crate) fn from_backend(raw: BackendObject) -> Sampler {
        Sampler {
            owner: Owner::Backend,
            raw,
        }
    }
}

impl GraphicsPipeline {
    /// The pipeline, for a backend to keep for its own use (it releases it
    /// when it is destroyed; the handle no longer holds the device).
    pub(crate) fn into_backend_owned(mut self) -> GraphicsPipeline {
        self.owner = Owner::Backend;
        self
    }
}

impl Texture {
    /// Set an arbitrary string constant to label a texture, for debugging
    /// tools. Translation of `SDL_SetGPUTextureName()`.
    ///
    /// You should use [`PROP_GPU_TEXTURE_CREATE_NAME_STRING`] with
    /// [`Device::create_texture`] instead of this function to avoid thread
    /// safety issues.
    pub fn set_name(&self, text: &str) {
        if let Some(device) = self.owner.device() {
            device.driver.set_texture_name(&self.raw, text);
        }
    }
}

impl Buffer {
    /// Set an arbitrary string constant to label a buffer, for debugging
    /// tools. Translation of `SDL_SetGPUBufferName()`.
    ///
    /// You should use [`PROP_GPU_BUFFER_CREATE_NAME_STRING`] with
    /// [`Device::create_buffer`] instead of this function to avoid thread
    /// safety issues.
    pub fn set_name(&self, text: &str) {
        if let Some(device) = self.owner.device() {
            device.driver.set_buffer_name(&self.raw, text);
        }
    }
}

impl TransferBuffer {
    /// Map the transfer buffer into application address space. Translation
    /// of `SDL_MapGPUTransferBuffer()`; the mapping is unmapped
    /// (`SDL_UnmapGPUTransferBuffer()`) when it is dropped.
    ///
    /// You must unmap the transfer buffer before encoding upload commands.
    /// The memory is owned by the graphics driver. With `cycle`, a transfer
    /// buffer still in use by the GPU is cycled (a fresh one is mapped).
    pub fn map(&mut self, cycle: bool) -> Result<MappedTransferBuffer<'_>> {
        let device = self
            .owner
            .device()
            .ok_or_else(|| Error::invalid_param("transfer_buffer"))?;
        let ptr = device.driver.map_transfer_buffer(&self.raw, cycle)?;
        Ok(MappedTransferBuffer { buffer: self, ptr })
    }

    /// The size in bytes of the transfer buffer.
    pub fn size(&self) -> u32 {
        self.size
    }
}

/// The memory of a mapped [`TransferBuffer`], unmapped when dropped.
pub struct MappedTransferBuffer<'a> {
    buffer: &'a mut TransferBuffer,
    ptr: NonNull<u8>,
}

impl std::fmt::Debug for MappedTransferBuffer<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MappedTransferBuffer")
            .field("size", &self.buffer.size)
            .finish_non_exhaustive()
    }
}

impl std::ops::Deref for MappedTransferBuffer<'_> {
    type Target = [u8];
    fn deref(&self) -> &[u8] {
        // SAFETY: the backend's mapping is valid for the buffer's size until
        // it is unmapped (the `map_transfer_buffer` contract), which happens
        // when this guard is dropped; the guard borrows the transfer buffer
        // mutably, so there is no other mapping.
        unsafe { std::slice::from_raw_parts(self.ptr.as_ptr(), self.buffer.size as usize) }
    }
}

impl std::ops::DerefMut for MappedTransferBuffer<'_> {
    fn deref_mut(&mut self) -> &mut [u8] {
        // SAFETY: as in `deref`, and `&mut self` makes this the only
        // reference to the memory.
        unsafe { std::slice::from_raw_parts_mut(self.ptr.as_ptr(), self.buffer.size as usize) }
    }
}

impl Drop for MappedTransferBuffer<'_> {
    fn drop(&mut self) {
        if let Some(device) = self.buffer.owner.device() {
            device.driver.unmap_transfer_buffer(&self.buffer.raw);
        }
    }
}

impl Fence {
    /// Whether the fence is signaled (its command buffer is done).
    /// Translation of `SDL_QueryGPUFence()`.
    pub fn is_signaled(&self) -> bool {
        match self.owner.device() {
            Some(device) => device.driver.query_fence(&self.raw),
            None => false,
        }
    }
}

impl Device {
    /// Create a GPU context for the given shader formats. Translation of
    /// `SDL_CreateGPUDevice()`.
    ///
    /// `debug_mode` enables the validation of every call; `name` picks a
    /// driver ("vulkan"...), or `None` for the best one available.
    pub fn new(format_flags: ShaderFormat, debug_mode: bool, name: Option<&str>) -> Result<Device> {
        let props = Properties::new();
        fill_properties(&props, format_flags, debug_mode, name)?;
        Device::with_properties(&props)
    }

    /// Create a GPU context with the `SDL.gpu.device.create.*` properties
    /// (`PROP_GPU_DEVICE_CREATE_*`). Translation of
    /// `SDL_CreateGPUDeviceWithProperties()`.
    pub fn with_properties(props: &Properties) -> Result<Device> {
        let selected_backend = select_backend(props)?;
        crate::debug!(
            crate::log::Category::System,
            "SDL chose gpu backend '{}'",
            selected_backend.name
        );
        let debug_mode = props
            .get_bool(PROP_GPU_DEVICE_CREATE_DEBUGMODE_BOOLEAN)
            .unwrap_or(true);
        let prefer_low_power = props
            .get_bool(PROP_GPU_DEVICE_CREATE_PREFERLOWPOWER_BOOLEAN)
            .unwrap_or(false);

        let BackendDevice {
            driver,
            shader_formats,
        } = (selected_backend.create_device)(debug_mode, prefer_low_power, props)?;

        let (default_enable_depth_clip, validate_feature_depth_clamp_disabled) = if props
            .get_bool(PROP_GPU_DEVICE_CREATE_FEATURE_DEPTH_CLAMPING_BOOLEAN)
            .unwrap_or(true)
        {
            (false, false)
        } else {
            (true, true)
        };
        let validate_feature_anisotropy_disabled = !props
            .get_bool(PROP_GPU_DEVICE_CREATE_FEATURE_ANISOTROPY_BOOLEAN)
            .unwrap_or(true);
        let max_viewport = if debug_mode { u32::MAX } else { 0 };

        Ok(Device {
            shared: Arc::new(DeviceShared {
                driver,
                backend: selected_backend.name,
                shader_formats,
                debug_mode,
                default_enable_depth_clip,
                validate_feature_depth_clamp_disabled,
                validate_feature_anisotropy_disabled,
                max_viewport_width: AtomicU32::new(max_viewport),
                max_viewport_height: AtomicU32::new(max_viewport),
            }),
        })
    }

    /// The name of the backend used to create this GPU context.
    /// Translation of `SDL_GetGPUDeviceDriver()`.
    pub fn driver(&self) -> &'static str {
        self.shared.backend
    }

    /// The shader formats supported by this GPU context. Translation of
    /// `SDL_GetGPUShaderFormats()`.
    pub fn shader_formats(&self) -> ShaderFormat {
        self.shared.shader_formats
    }

    /// The properties of the GPU device ([`PROP_GPU_DEVICE_NAME_STRING`]...).
    /// Translation of `SDL_GetGPUDeviceProperties()`.
    pub fn properties(&self) -> Properties {
        self.shared.driver.properties()
    }

    /// Whether a texture format is supported for a given type and usage.
    /// Translation of `SDL_GPUTextureSupportsFormat()`.
    pub fn texture_supports_format(
        &self,
        format: TextureFormat,
        texture_type: TextureType,
        usage: TextureUsageFlags,
    ) -> bool {
        self.shared
            .texture_supports_format(format, texture_type, usage)
    }

    /// Whether a sample count for a texture format is supported.
    /// Translation of `SDL_GPUTextureSupportsSampleCount()`.
    pub fn texture_supports_sample_count(
        &self,
        format: TextureFormat,
        sample_count: SampleCount,
    ) -> bool {
        if self.shared.debug_mode && format.is_invalid_enum() {
            crate::sdl_assert_release!(!"Invalid texture format enum!");
            return false;
        }

        self.shared
            .driver
            .supports_sample_count(format, sample_count)
    }

    /// Create a pipeline object to be used in a compute workflow.
    /// Translation of `SDL_CreateGPUComputePipeline()`.
    pub fn create_compute_pipeline(
        &self,
        createinfo: &ComputePipelineCreateInfo<'_>,
    ) -> Result<ComputePipeline> {
        self.shared.create_compute_pipeline(createinfo)
    }

    /// Create a pipeline object to be used in a graphics workflow.
    /// Translation of `SDL_CreateGPUGraphicsPipeline()`.
    pub fn create_graphics_pipeline(
        &self,
        createinfo: &GraphicsPipelineCreateInfo<'_>,
    ) -> Result<GraphicsPipeline> {
        self.shared.create_graphics_pipeline(createinfo)
    }

    /// Create a sampler object to be used when binding textures in a
    /// graphics workflow. Translation of `SDL_CreateGPUSampler()`.
    pub fn create_sampler(&self, createinfo: &SamplerCreateInfo) -> Result<Sampler> {
        if self.shared.debug_mode
            && self.shared.validate_feature_anisotropy_disabled
            && createinfo.enable_anisotropy
        {
            return Err(debug_fail!(
                "enable_anisotropy must be set to false (FEATURE_ANISOTROPY disabled)"
            ));
        }

        let raw = self.shared.driver.create_sampler(createinfo)?;
        Ok(Sampler {
            owner: Owner::Device(self.shared.clone()),
            raw,
        })
    }

    /// Create a shader to be used when creating a graphics pipeline.
    /// Translation of `SDL_CreateGPUShader()`.
    pub fn create_shader(&self, createinfo: &ShaderCreateInfo<'_>) -> Result<Shader> {
        if self.shared.debug_mode {
            if createinfo.format == ShaderFormat::INVALID {
                return Err(debug_fail!("Shader format cannot be INVALID!"));
            }
            if !createinfo.format.intersects(self.shared.shader_formats) {
                return Err(debug_fail!("Incompatible shader format for GPU backend"));
            }
            const _: () = assert!(MAX_TEXTURE_SAMPLERS_PER_STAGE == 16);
            if createinfo.num_samplers > MAX_TEXTURE_SAMPLERS_PER_STAGE {
                return Err(debug_fail!(
                    "Shader sampler count cannot be higher than 16!"
                ));
            }
            const _: () = assert!(MAX_STORAGE_TEXTURES_PER_STAGE == 8);
            if createinfo.num_storage_textures > MAX_STORAGE_TEXTURES_PER_STAGE {
                return Err(debug_fail!(
                    "Shader storage texture count cannot be higher than 8!"
                ));
            }
            const _: () = assert!(MAX_STORAGE_BUFFERS_PER_STAGE == 8);
            if createinfo.num_storage_buffers > MAX_STORAGE_BUFFERS_PER_STAGE {
                return Err(debug_fail!(
                    "Shader storage buffer count cannot be higher than 8!"
                ));
            }
            const _: () = assert!(MAX_UNIFORM_BUFFERS_PER_STAGE == 4);
            if createinfo.num_uniform_buffers > MAX_UNIFORM_BUFFERS_PER_STAGE {
                return Err(debug_fail!(
                    "Shader uniform buffer count cannot be higher than 4!"
                ));
            }
        }

        let raw = self.shared.driver.create_shader(createinfo)?;
        Ok(Shader {
            owner: Owner::Device(self.shared.clone()),
            raw,
        })
    }

    /// Create a texture object to be used in graphics or compute workflows.
    /// Translation of `SDL_CreateGPUTexture()`.
    pub fn create_texture(&self, createinfo: &TextureCreateInfo) -> Result<Texture> {
        if self.shared.debug_mode {
            let mut failed: Option<Error> = None;
            macro_rules! fail {
                ($msg:literal) => {{
                    let e = debug_fail!($msg);
                    failed.get_or_insert(e);
                }};
            }

            const MAX_2D_DIMENSION: u32 = 16384;
            const MAX_3D_DIMENSION: u32 = 2048;

            // Common checks for all texture types
            if createinfo.format.is_invalid_enum() {
                return Err(debug_fail!("Invalid texture format enum!"));
            }

            let usage = createinfo.usage;
            if createinfo.width == 0
                || createinfo.height == 0
                || createinfo.layer_count_or_depth == 0
            {
                fail!("For any texture: width, height, and layer_count_or_depth must be >= 1");
            }
            if createinfo.num_levels == 0 {
                fail!("For any texture: num_levels must be >= 1");
            }
            if createinfo.texture_type == TextureType::Texture2D
                && createinfo.layer_count_or_depth != 1
            {
                fail!("2D textures must have a layer count of 1");
            }
            if usage.contains(TextureUsageFlags::GRAPHICS_STORAGE_READ)
                && usage.contains(TextureUsageFlags::SAMPLER)
            {
                fail!(
                    "For any texture: usage cannot contain both GRAPHICS_STORAGE_READ and SAMPLER"
                );
            }
            if createinfo.sample_count > SampleCount::One
                && usage.contains(TextureUsageFlags::COMPUTE_STORAGE_WRITE)
            {
                fail!("For multisample textures: usage cannot contain COMPUTE_STORAGE_WRITE flag");
            }
            if createinfo.format.is_depth_format()
                && !(usage
                    & !(TextureUsageFlags::DEPTH_STENCIL_TARGET | TextureUsageFlags::SAMPLER))
                    .is_empty()
            {
                fail!("For depth textures: usage cannot contain any flags except for DEPTH_STENCIL_TARGET and SAMPLER");
            }
            if createinfo.format.is_integer_format() && usage.contains(TextureUsageFlags::SAMPLER) {
                fail!("For any texture: usage cannot contain SAMPLER for textures with an integer format");
            }

            let supports = |texture_type| {
                self.shared
                    .texture_supports_format(createinfo.format, texture_type, usage)
            };
            if createinfo.texture_type == TextureType::Cube {
                // Cubemap validation
                if createinfo.width != createinfo.height {
                    fail!("For cube textures: width and height must be identical");
                }
                if createinfo.width > MAX_2D_DIMENSION || createinfo.height > MAX_2D_DIMENSION {
                    fail!("For cube textures: width and height must be <= 16384");
                }
                if createinfo.layer_count_or_depth != 6 {
                    fail!("For cube textures: layer_count_or_depth must be 6");
                }
                if createinfo.sample_count > SampleCount::One {
                    fail!("For cube textures: sample_count must be SDL_GPU_SAMPLECOUNT_1");
                }
                if !supports(TextureType::Cube) {
                    fail!("For cube textures: the format is unsupported for the given usage");
                }
            } else if createinfo.texture_type == TextureType::CubeArray {
                // Cubemap array validation
                if createinfo.width != createinfo.height {
                    fail!("For cube array textures: width and height must be identical");
                }
                if createinfo.width > MAX_2D_DIMENSION || createinfo.height > MAX_2D_DIMENSION {
                    fail!("For cube array textures: width and height must be <= 16384");
                }
                if !createinfo.layer_count_or_depth.is_multiple_of(6) {
                    fail!("For cube array textures: layer_count_or_depth must be a multiple of 6");
                }
                if createinfo.sample_count > SampleCount::One {
                    fail!("For cube array textures: sample_count must be SDL_GPU_SAMPLECOUNT_1");
                }
                if !supports(TextureType::CubeArray) {
                    fail!("For cube array textures: the format is unsupported for the given usage");
                }
            } else if createinfo.texture_type == TextureType::Texture3D {
                // 3D Texture Validation
                if createinfo.width > MAX_3D_DIMENSION
                    || createinfo.height > MAX_3D_DIMENSION
                    || createinfo.layer_count_or_depth > MAX_3D_DIMENSION
                {
                    fail!(
                        "For 3D textures: width, height, and layer_count_or_depth must be <= 2048"
                    );
                }
                if usage.contains(TextureUsageFlags::DEPTH_STENCIL_TARGET) {
                    fail!("For 3D textures: usage must not contain DEPTH_STENCIL_TARGET");
                }
                if createinfo.sample_count > SampleCount::One {
                    fail!("For 3D textures: sample_count must be SDL_GPU_SAMPLECOUNT_1");
                }
                if !supports(TextureType::Texture3D) {
                    fail!("For 3D textures: the format is unsupported for the given usage");
                }
            } else {
                if createinfo.texture_type == TextureType::Texture2DArray {
                    // Array Texture Validation
                    if createinfo.sample_count > SampleCount::One {
                        fail!("For array textures: sample_count must be SDL_GPU_SAMPLECOUNT_1");
                    }
                }
                if createinfo.sample_count > SampleCount::One && createinfo.num_levels > 1 {
                    fail!("For 2D multisample textures: num_levels must be 1");
                }
                if !supports(TextureType::Texture2D) {
                    fail!("For 2D textures: the format is unsupported for the given usage");
                }
            }

            if let Some(e) = failed {
                return Err(e);
            }
        }

        let raw = self.shared.driver.create_texture(createinfo)?;
        Ok(Texture {
            owner: Owner::Device(self.shared.clone()),
            raw,
            info: createinfo.clone(),
        })
    }

    /// Create a buffer object to be used in graphics or compute workflows.
    /// Translation of `SDL_CreateGPUBuffer()`.
    pub fn create_buffer(&self, createinfo: &BufferCreateInfo) -> Result<Buffer> {
        if self.shared.debug_mode && createinfo.size < 4 {
            crate::sdl_assert_release!(!"Cannot create a buffer with size less than 4 bytes!");
        }

        let debug_name = createinfo
            .props
            .as_ref()
            .and_then(|p| p.get_string(PROP_GPU_BUFFER_CREATE_NAME_STRING));

        let raw = self.shared.driver.create_buffer(
            createinfo.usage,
            createinfo.size,
            debug_name.as_deref(),
        )?;
        Ok(Buffer {
            owner: Owner::Device(self.shared.clone()),
            raw,
        })
    }

    /// Create a transfer buffer to be used when uploading to or downloading
    /// from graphics resources. Translation of
    /// `SDL_CreateGPUTransferBuffer()`.
    ///
    /// (Upstream's debug check of the usage can't fail: the enum holds
    /// only upload and download.)
    pub fn create_transfer_buffer(
        &self,
        createinfo: &TransferBufferCreateInfo,
    ) -> Result<TransferBuffer> {
        let debug_name = createinfo
            .props
            .as_ref()
            .and_then(|p| p.get_string(PROP_GPU_TRANSFERBUFFER_CREATE_NAME_STRING));

        let raw = self.shared.driver.create_transfer_buffer(
            createinfo.usage,
            createinfo.size,
            debug_name.as_deref(),
        )?;
        Ok(TransferBuffer {
            owner: Owner::Device(self.shared.clone()),
            raw,
            size: createinfo.size,
        })
    }

    /// Acquire a command buffer, to record commands into and submit.
    /// Translation of `SDL_AcquireGPUCommandBuffer()`.
    ///
    /// A command buffer is not thread safe: record it on one thread at a
    /// time (it can be acquired, recorded and submitted on any thread).
    pub fn acquire_command_buffer(&self) -> Result<CommandBuffer> {
        let raw = self.shared.driver.acquire_command_buffer()?;
        // (The header is new for each command buffer, which is what
        // upstream's debug mode resets it to; outside debug mode it is
        // only read for flags the same command buffer sets.)
        Ok(CommandBuffer {
            device: self.shared.clone(),
            raw: Some(raw),
            header: CommandBufferHeader::default(),
        })
    }

    /// Whether a swapchain composition is supported by the window. The
    /// window must be claimed before calling this function. Translation of
    /// `SDL_WindowSupportsGPUSwapchainComposition()`.
    pub fn window_supports_swapchain_composition(
        &self,
        window: &Window,
        swapchain_composition: SwapchainComposition,
    ) -> bool {
        self.shared
            .driver
            .supports_swapchain_composition(*window, swapchain_composition)
    }

    /// Whether a present mode is supported by the window. The window must
    /// be claimed before calling this function. Translation of
    /// `SDL_WindowSupportsGPUPresentMode()`.
    pub fn window_supports_present_mode(&self, window: &Window, present_mode: PresentMode) -> bool {
        self.shared
            .driver
            .supports_present_mode(*window, present_mode)
    }

    /// Claim a window, creating a swapchain structure for it. Translation
    /// of `SDL_ClaimWindowForGPUDevice()`.
    ///
    /// This must be called before acquiring swapchain textures for the
    /// window. The swapchain has the SDR composition and the VSYNC present
    /// mode.
    pub fn claim_window(&self, window: &Window) -> Result<()> {
        if window.flags()?.contains(WindowFlags::TRANSPARENT) {
            return Err(Error::new(
                "The GPU API doesn't support transparent windows",
            ));
        }

        self.shared.driver.claim_window(*window)
    }

    /// Unclaim a window, destroying its swapchain structure. Translation of
    /// `SDL_ReleaseWindowFromGPUDevice()`.
    pub fn release_window(&self, window: &Window) {
        self.shared.driver.release_window(*window);
    }

    /// Change the swapchain parameters for the given claimed window.
    /// Translation of `SDL_SetGPUSwapchainParameters()`.
    ///
    /// This function will fail if the requested present mode or swapchain
    /// composition are unsupported by the device.
    pub fn set_swapchain_parameters(
        &self,
        window: &Window,
        swapchain_composition: SwapchainComposition,
        present_mode: PresentMode,
    ) -> Result<()> {
        self.shared
            .driver
            .set_swapchain_parameters(*window, swapchain_composition, present_mode)
    }

    /// Configure the maximum allowed number of frames in flight (1 to 3,
    /// default 2). Translation of `SDL_SetGPUAllowedFramesInFlight()`.
    ///
    /// The value is clamped; debug mode asserts when it is out of range.
    pub fn set_allowed_frames_in_flight(&self, allowed_frames_in_flight: u32) -> Result<()> {
        if self.shared.debug_mode {
            const _: () = assert!(MAX_FRAMES_IN_FLIGHT == 3);
            if !(1..=3).contains(&allowed_frames_in_flight) {
                crate::sdl_assert_release!(
                    !"allowed_frames_in_flight value must be between 1 and 3!"
                );
            }
        }

        // (MAX_FRAMES_IN_FLIGHT is 3)
        let allowed_frames_in_flight = allowed_frames_in_flight.clamp(1, MAX_FRAMES_IN_FLIGHT);
        self.shared
            .driver
            .set_allowed_frames_in_flight(allowed_frames_in_flight)
    }

    /// The texture format of the swapchain for the given window.
    /// Translation of `SDL_GetGPUSwapchainTextureFormat()`.
    ///
    /// Note that this format can change if the swapchain parameters change.
    pub fn swapchain_texture_format(&self, window: &Window) -> Result<TextureFormat> {
        self.shared.driver.swapchain_texture_format(*window)
    }

    /// Block the thread until a swapchain texture is available to be
    /// acquired. Translation of `SDL_WaitForGPUSwapchain()`.
    pub fn wait_for_swapchain(&self, window: &Window) -> Result<()> {
        self.shared.driver.wait_for_swapchain(*window)
    }

    /// Block the thread until the GPU is completely idle. Translation of
    /// `SDL_WaitForGPUIdle()`.
    pub fn wait_for_idle(&self) -> Result<()> {
        self.shared.driver.wait()
    }

    /// Block the thread until the given fences are signaled (all of them,
    /// or any with `wait_all` false). Translation of
    /// `SDL_WaitForGPUFences()`.
    pub fn wait_for_fences(&self, wait_all: bool, fences: &[&Fence]) -> Result<()> {
        if fences.is_empty() {
            return Ok(());
        }

        let raw: Vec<&BackendObject> = fences.iter().map(|f| &f.raw).collect();
        self.shared.driver.wait_for_fences(wait_all, &raw)
    }
}

impl DeviceShared {
    /// Translation of `SDL_GPUTextureSupportsFormat()`.
    pub(crate) fn texture_supports_format(
        &self,
        format: TextureFormat,
        texture_type: TextureType,
        usage: TextureUsageFlags,
    ) -> bool {
        if self.debug_mode && format.is_invalid_enum() {
            crate::sdl_assert_release!(!"Invalid texture format enum!");
            return false;
        }

        if (usage.contains(TextureUsageFlags::COMPUTE_STORAGE_WRITE)
            || usage.contains(TextureUsageFlags::COMPUTE_STORAGE_SIMULTANEOUS_READ_WRITE))
            && !format.is_compute_writable()
        {
            return false;
        }

        self.driver
            .supports_texture_format(format, texture_type, usage)
    }

    /// Translation of `SDL_CreateGPUComputePipeline()`.
    pub(crate) fn create_compute_pipeline(
        self: &Arc<Self>,
        createinfo: &ComputePipelineCreateInfo<'_>,
    ) -> Result<ComputePipeline> {
        if self.debug_mode {
            if createinfo.format == ShaderFormat::INVALID {
                return Err(debug_fail!("Shader format cannot be INVALID!"));
            }
            if !createinfo.format.intersects(self.shader_formats) {
                return Err(debug_fail!("Incompatible shader format for GPU backend"));
            }
            const _: () = assert!(MAX_COMPUTE_WRITE_TEXTURES == 8);
            if createinfo.num_readwrite_storage_textures > MAX_COMPUTE_WRITE_TEXTURES {
                return Err(debug_fail!(
                    "Compute pipeline write-only texture count cannot be higher than 8!"
                ));
            }
            const _: () = assert!(MAX_COMPUTE_WRITE_BUFFERS == 8);
            if createinfo.num_readwrite_storage_buffers > MAX_COMPUTE_WRITE_BUFFERS {
                return Err(debug_fail!(
                    "Compute pipeline write-only buffer count cannot be higher than 8!"
                ));
            }
            if createinfo.num_samplers > MAX_TEXTURE_SAMPLERS_PER_STAGE {
                return Err(debug_fail!(
                    "Compute pipeline sampler count cannot be higher than 16!"
                ));
            }
            if createinfo.num_readonly_storage_textures > MAX_STORAGE_TEXTURES_PER_STAGE {
                return Err(debug_fail!(
                    "Compute pipeline readonly storage texture count cannot be higher than 8!"
                ));
            }
            if createinfo.num_readonly_storage_buffers > MAX_STORAGE_BUFFERS_PER_STAGE {
                return Err(debug_fail!(
                    "Compute pipeline readonly storage buffer count cannot be higher than 8!"
                ));
            }
            if createinfo.num_uniform_buffers > MAX_UNIFORM_BUFFERS_PER_STAGE {
                return Err(debug_fail!(
                    "Compute pipeline uniform buffer count cannot be higher than 4!"
                ));
            }
            if createinfo.threadcount_x == 0
                || createinfo.threadcount_y == 0
                || createinfo.threadcount_z == 0
            {
                return Err(debug_fail!(
                    "Compute pipeline threadCount dimensions must be at least 1!"
                ));
            }
        }

        let (raw, header) = self.driver.create_compute_pipeline(createinfo)?;
        Ok(ComputePipeline {
            owner: Owner::Device(self.clone()),
            raw,
            header,
        })
    }

    /// Translation of `SDL_CreateGPUGraphicsPipeline()`.
    ///
    /// (The shaders can't be NULL, and the arrays' pointers are their
    /// slices; the enum checks of the blend, compare and stencil ops and the
    /// vertex formats can't fail.)
    pub(crate) fn create_graphics_pipeline(
        self: &Arc<Self>,
        info: &GraphicsPipelineCreateInfo<'_>,
    ) -> Result<GraphicsPipeline> {
        if self.debug_mode {
            let target_info = &info.target_info;
            for desc in target_info.color_target_descriptions {
                if desc.format.is_invalid_enum() {
                    return Err(debug_fail!("Invalid texture format enum!"));
                }
                if desc.format.is_depth_format() {
                    return Err(debug_fail!(
                        "Color target formats cannot be a depth format!"
                    ));
                }
                if !self.texture_supports_format(
                    desc.format,
                    TextureType::Texture2D,
                    TextureUsageFlags::COLOR_TARGET,
                ) {
                    return Err(debug_fail!(
                        "Format is not supported for color targets on this device!"
                    ));
                }
                if desc.blend_state.enable_blend {
                    // (CHECK_BLENDFACTOR_ENUM_INVALID / CHECK_BLENDOP_ENUM_INVALID
                    // can't fail.)

                    // TODO: validate that format support blending?
                }
            }
            if target_info.has_depth_stencil_target {
                if target_info.depth_stencil_format.is_invalid_enum() {
                    return Err(debug_fail!("Invalid texture format enum!"));
                }
                if !target_info.depth_stencil_format.is_depth_format() {
                    return Err(debug_fail!(
                        "Depth-stencil target format must be a depth format!"
                    ));
                }
                if !self.texture_supports_format(
                    target_info.depth_stencil_format,
                    TextureType::Texture2D,
                    TextureUsageFlags::DEPTH_STENCIL_TARGET,
                ) {
                    return Err(debug_fail!(
                        "Format is not supported for depth targets on this device!"
                    ));
                }
            }
            if info.multisample_state.enable_alpha_to_coverage {
                let Some(first) = target_info.color_target_descriptions.first() else {
                    return Err(debug_fail!(
                        "Alpha-to-coverage enabled but no color targets present!"
                    ));
                };
                if !first.format.has_alpha() {
                    return Err(debug_fail!(
                        "Format is not compatible with alpha-to-coverage!"
                    ));
                }

                // TODO: validate that format supports belnding? This is only required on Metal.
            }
            let vertex_input_state = &info.vertex_input_state;
            const _: () = assert!(MAX_VERTEX_BUFFERS == 16);
            if vertex_input_state.vertex_buffer_descriptions.len() > MAX_VERTEX_BUFFERS as usize {
                return Err(debug_fail!(
                    "The number of vertex buffer descriptions in a vertex input state must not exceed 16!"
                ));
            }
            const _: () = assert!(MAX_VERTEX_ATTRIBUTES == 16);
            if vertex_input_state.vertex_attributes.len() > MAX_VERTEX_ATTRIBUTES as usize {
                return Err(debug_fail!(
                    "The number of vertex attributes in a vertex input state must not exceed 16!"
                ));
            }
            for desc in vertex_input_state.vertex_buffer_descriptions {
                if desc.instance_step_rate != 0 {
                    return Err(debug_fail!(
                        "For all vertex buffer descriptions, instance_step_rate must be 0!"
                    ));
                }
            }
            let attributes = vertex_input_state.vertex_attributes;
            for (i, attribute) in attributes.iter().enumerate() {
                if attributes[..i]
                    .iter()
                    .any(|other| other.location == attribute.location)
                {
                    return Err(debug_fail!(
                        "Each vertex attribute location in a vertex input state must be unique!"
                    ));
                }
            }
            if info.multisample_state.enable_mask {
                return Err(debug_fail!(
                    "For multisample states, enable_mask must be false!"
                ));
            }
            if info.multisample_state.sample_mask != 0 {
                return Err(debug_fail!(
                    "For multisample states, sample_mask must be 0!"
                ));
            }

            if self.validate_feature_depth_clamp_disabled
                && !info.rasterizer_state.enable_depth_clip
            {
                return Err(debug_fail!(
                    "Rasterizer state enable_depth_clip must be set to true (FEATURE_DEPTH_CLAMPING disabled)"
                ));
            }
        }

        let (raw, header) = self.driver.create_graphics_pipeline(info)?;
        Ok(GraphicsPipeline {
            owner: Owner::Device(self.clone()),
            raw,
            header,
        })
    }
}

// Command Buffer

/// A command buffer, to record GPU commands into and submit. Translation of
/// `SDL_GPUCommandBuffer *`.
///
/// Dropping a command buffer that wasn't submitted or cancelled cancels it,
/// or submits it if it acquired a swapchain texture.
pub struct CommandBuffer {
    pub(crate) device: Arc<DeviceShared>,
    /// The backend's command buffer, until it is submitted or cancelled.
    raw: Option<Box<BackendCommandBuffer>>,
    pub(crate) header: CommandBufferHeader,
}

impl std::fmt::Debug for CommandBuffer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CommandBuffer")
            .field("header", &self.header)
            .finish_non_exhaustive()
    }
}

/// The message for a command buffer used after being submitted, which the
/// API makes impossible.
const SUBMITTED: &str = "command buffer already submitted";

/// A swapchain texture acquired for a window: the texture to render to and
/// its size. Translation of the outputs of
/// `SDL_AcquireGPUSwapchainTexture()`.
///
/// The texture belongs to the swapchain: it is valid until the command
/// buffer it was acquired with is submitted, and dropping it does nothing.
#[derive(Debug)]
pub struct SwapchainTexture {
    /// The swapchain texture.
    pub texture: Texture,
    /// The width of the swapchain texture.
    pub width: u32,
    /// The height of the swapchain texture.
    pub height: u32,
}

impl CommandBuffer {
    /// The device, the backend's command buffer and the front end's state.
    fn parts(
        &mut self,
    ) -> (
        &DeviceShared,
        &mut BackendCommandBuffer,
        &mut CommandBufferHeader,
    ) {
        (
            &self.device,
            self.raw.as_deref_mut().expect(SUBMITTED),
            &mut self.header,
        )
    }

    /// The backend's command buffer, if it is a `T` (for the backends'
    /// `generate_mipmaps` and `blit`).
    #[allow(dead_code)] // (for the backends)
    pub(crate) fn backend_mut<T: 'static>(&mut self) -> Option<&mut T> {
        self.raw
            .as_deref_mut()
            .and_then(|raw| raw.downcast_mut::<T>())
    }

    /// Insert an arbitrary string label into the command buffer, for
    /// debugging tools. Translation of `SDL_InsertGPUDebugLabel()`.
    pub fn insert_debug_label(&mut self, text: &str) {
        let (device, raw, _) = self.parts();
        device.driver.insert_debug_label(raw, text);
    }

    /// Begin a debug group with an arbitrary name, for debugging tools.
    /// Translation of `SDL_PushGPUDebugGroup()`.
    pub fn push_debug_group(&mut self, name: &str) {
        let (device, raw, _) = self.parts();
        device.driver.push_debug_group(raw, name);
    }

    /// End the most-recently pushed debug group. Translation of
    /// `SDL_PopGPUDebugGroup()`.
    pub fn pop_debug_group(&mut self) {
        let (device, raw, _) = self.parts();
        device.driver.pop_debug_group(raw);
    }

    /// Push data to a vertex uniform slot, for the following draw calls.
    /// Translation of `SDL_PushGPUVertexUniformData()`.
    ///
    /// The data being pushed must respect std140 layout conventions. In
    /// practical terms this means you must ensure that vec3 and vec4 fields
    /// are 16-byte aligned.
    pub fn push_vertex_uniform_data(&mut self, slot_index: u32, data: &[u8]) -> Result<()> {
        if slot_index >= MAX_UNIFORM_BUFFERS_PER_STAGE {
            return Err(Error::new(
                "slot_index exceeds MAX_UNIFORM_BUFFERS_PER_STAGE",
            ));
        }

        let (device, raw, _) = self.parts();
        device
            .driver
            .push_vertex_uniform_data(raw, slot_index, data);
        Ok(())
    }

    /// Push data to a fragment uniform slot, for the following draw calls.
    /// Translation of `SDL_PushGPUFragmentUniformData()`.
    ///
    /// The data being pushed must respect std140 layout conventions.
    pub fn push_fragment_uniform_data(&mut self, slot_index: u32, data: &[u8]) -> Result<()> {
        if slot_index >= MAX_UNIFORM_BUFFERS_PER_STAGE {
            return Err(Error::new(
                "slot_index exceeds MAX_UNIFORM_BUFFERS_PER_STAGE",
            ));
        }

        let (device, raw, _) = self.parts();
        device
            .driver
            .push_fragment_uniform_data(raw, slot_index, data);
        Ok(())
    }

    /// Push data to a uniform slot, for the following compute dispatches.
    /// Translation of `SDL_PushGPUComputeUniformData()`.
    ///
    /// The data being pushed must respect std140 layout conventions.
    pub fn push_compute_uniform_data(&mut self, slot_index: u32, data: &[u8]) -> Result<()> {
        if slot_index >= MAX_UNIFORM_BUFFERS_PER_STAGE {
            return Err(Error::new(
                "slot_index exceeds MAX_UNIFORM_BUFFERS_PER_STAGE",
            ));
        }

        let (device, raw, _) = self.parts();
        device
            .driver
            .push_compute_uniform_data(raw, slot_index, data);
        Ok(())
    }

    /// Begin a render pass on the command buffer, with up to 8 color
    /// targets and an optional depth-stencil target. Translation of
    /// `SDL_BeginGPURenderPass()`.
    ///
    /// The pass ends when the returned [`RenderPass`] is dropped. It is
    /// invalid to begin another pass, or to acquire a swapchain texture,
    /// while it is in progress.
    pub fn begin_render_pass<'a>(
        &'a mut self,
        color_target_infos: &[ColorTargetInfo<'_>],
        depth_stencil_target_info: Option<&DepthStencilTargetInfo<'_>>,
    ) -> Result<RenderPass<'a>> {
        if color_target_infos.len() > MAX_COLOR_TARGET_BINDINGS as usize {
            return Err(Error::new(
                "num_color_targets exceeds MAX_COLOR_TARGET_BINDINGS",
            ));
        }

        let (device, raw, header) = self.parts();
        if device.debug_mode {
            if header.any_pass_in_progress() {
                return Err(debug_fail!("Cannot begin render pass during another pass!"));
            }

            device.max_viewport_width.store(u32::MAX, Ordering::Relaxed);
            device
                .max_viewport_height
                .store(u32::MAX, Ordering::Relaxed);

            for color_target_info in color_target_infos {
                let texture_header = &color_target_info.texture.info;

                device
                    .max_viewport_width
                    .fetch_min(texture_header.width, Ordering::Relaxed);
                device
                    .max_viewport_height
                    .fetch_min(texture_header.height, Ordering::Relaxed);

                if color_target_info.cycle && color_target_info.load_op == LoadOp::Load {
                    return Err(debug_fail!(
                        "Cannot cycle color target when load op is LOAD!"
                    ));
                }

                if color_target_info.store_op == StoreOp::Resolve
                    || color_target_info.store_op == StoreOp::ResolveAndStore
                {
                    let Some(resolve_texture) = color_target_info.resolve_texture else {
                        return Err(debug_fail!(
                            "Store op is RESOLVE or RESOLVE_AND_STORE but resolve_texture is NULL!"
                        ));
                    };
                    let resolve_texture_header = &resolve_texture.info;
                    if texture_header.sample_count == SampleCount::One {
                        return Err(debug_fail!(
                            "Store op is RESOLVE or RESOLVE_AND_STORE but texture is not multisample!"
                        ));
                    }
                    if resolve_texture_header.sample_count != SampleCount::One {
                        return Err(debug_fail!(
                            "Resolve texture must have a sample count of 1!"
                        ));
                    }
                    if resolve_texture_header.format != texture_header.format {
                        return Err(debug_fail!(
                            "Resolve texture must have the same format as its corresponding color target!"
                        ));
                    }
                    if resolve_texture_header.texture_type == TextureType::Texture3D {
                        return Err(debug_fail!(
                            "Resolve texture must not be of TEXTURETYPE_3D!"
                        ));
                    }
                    if !resolve_texture_header
                        .usage
                        .contains(TextureUsageFlags::COLOR_TARGET)
                    {
                        return Err(debug_fail!(
                            "Resolve texture usage must include COLOR_TARGET!"
                        ));
                    }
                }

                if color_target_info.layer_or_depth_plane >= texture_header.layer_count_or_depth {
                    return Err(debug_fail!(
                        "Color target layer index must be less than the texture's layer count!"
                    ));
                }

                if color_target_info.mip_level >= texture_header.num_levels {
                    return Err(debug_fail!(
                        "Color target mip level must be less than the texture's level count!"
                    ));
                }
            }

            if let Some(depth_stencil_target_info) = depth_stencil_target_info {
                let texture_header = &depth_stencil_target_info.texture.info;
                if !texture_header
                    .usage
                    .contains(TextureUsageFlags::DEPTH_STENCIL_TARGET)
                {
                    return Err(debug_fail!(
                        "Depth target must have been created with the DEPTH_STENCIL_TARGET usage flag!"
                    ));
                }

                if texture_header.layer_count_or_depth > 255 {
                    return Err(debug_fail!(
                        "Cannot bind a depth texture with more than 255 layers!"
                    ));
                }

                if depth_stencil_target_info.cycle
                    && (depth_stencil_target_info.load_op == LoadOp::Load
                        || depth_stencil_target_info.stencil_load_op == LoadOp::Load)
                {
                    return Err(debug_fail!(
                        "Cannot cycle depth target when load op or stencil load op is LOAD!"
                    ));
                }

                if depth_stencil_target_info.store_op == StoreOp::Resolve
                    || depth_stencil_target_info.stencil_store_op == StoreOp::Resolve
                    || depth_stencil_target_info.store_op == StoreOp::ResolveAndStore
                    || depth_stencil_target_info.stencil_store_op == StoreOp::ResolveAndStore
                {
                    return Err(debug_fail!(
                        "RESOLVE store ops are not supported for depth-stencil targets!"
                    ));
                }
            }
        }

        device
            .driver
            .begin_render_pass(raw, color_target_infos, depth_stencil_target_info);

        if device.debug_mode {
            header.render_pass.in_progress = true;
            header.render_pass.num_color_targets = color_target_infos.len() as u32;
        }

        Ok(RenderPass { cmd: self })
    }

    /// Begin a compute pass on the command buffer, with the storage
    /// textures and buffers the compute shaders write. Translation of
    /// `SDL_BeginGPUComputePass()`.
    ///
    /// The pass ends when the returned [`ComputePass`] is dropped.
    pub fn begin_compute_pass<'a>(
        &'a mut self,
        storage_texture_bindings: &[StorageTextureReadWriteBinding<'_>],
        storage_buffer_bindings: &[StorageBufferReadWriteBinding<'_>],
    ) -> Result<ComputePass<'a>> {
        if storage_texture_bindings.len() > MAX_COMPUTE_WRITE_TEXTURES as usize {
            return Err(Error::invalid_param("num_storage_texture_bindings"));
        }
        if storage_buffer_bindings.len() > MAX_COMPUTE_WRITE_BUFFERS as usize {
            return Err(Error::invalid_param("num_storage_buffer_bindings"));
        }

        let (device, raw, header) = self.parts();
        if device.debug_mode {
            if header.any_pass_in_progress() {
                return Err(debug_fail!(
                    "Cannot begin compute pass during another pass!"
                ));
            }

            for binding in storage_texture_bindings {
                let header = &binding.texture.info;
                if !header
                    .usage
                    .contains(TextureUsageFlags::COMPUTE_STORAGE_WRITE)
                    && !header
                        .usage
                        .contains(TextureUsageFlags::COMPUTE_STORAGE_SIMULTANEOUS_READ_WRITE)
                {
                    return Err(debug_fail!(
                        "Texture must be created with COMPUTE_STORAGE_WRITE or COMPUTE_STORAGE_SIMULTANEOUS_READ_WRITE flag"
                    ));
                }

                if binding.layer >= header.layer_count_or_depth {
                    return Err(debug_fail!(
                        "Storage texture layer index must be less than the texture's layer count!"
                    ));
                }

                if binding.mip_level >= header.num_levels {
                    return Err(debug_fail!(
                        "Storage texture mip level must be less than the texture's level count!"
                    ));
                }
            }

            // TODO: validate buffer usage?
        }

        device
            .driver
            .begin_compute_pass(raw, storage_texture_bindings, storage_buffer_bindings);

        if device.debug_mode {
            header.compute_pass.in_progress = true;

            for i in 0..storage_texture_bindings.len() {
                header.compute_pass.read_write_storage_texture_bound[i] = true;
            }

            for i in 0..storage_buffer_bindings.len() {
                header.compute_pass.read_write_storage_buffer_bound[i] = true;
            }
        }

        Ok(ComputePass { cmd: self })
    }

    /// Begin a copy pass on the command buffer. Translation of
    /// `SDL_BeginGPUCopyPass()`.
    ///
    /// The pass ends when the returned [`CopyPass`] is dropped.
    pub fn begin_copy_pass(&mut self) -> Result<CopyPass<'_>> {
        let (device, raw, header) = self.parts();
        if device.debug_mode && header.any_pass_in_progress() {
            return Err(debug_fail!("Cannot begin copy pass during another pass!"));
        }

        device.driver.begin_copy_pass(raw);

        if device.debug_mode {
            header.copy_pass.in_progress = true;
        }

        Ok(CopyPass { cmd: self })
    }

    /// Generate mipmaps for the given texture, outside any pass.
    /// Translation of `SDL_GenerateMipmapsForGPUTexture()`.
    ///
    /// The texture must have more than one mip level and be created with
    /// the SAMPLER and COLOR_TARGET usage flags.
    pub fn generate_mipmaps(&mut self, texture: &Texture) -> Result<()> {
        let device = self.device.clone();
        if device.debug_mode {
            if self.header.any_pass_in_progress() {
                return Err(debug_fail!("Cannot generate mipmaps during a pass!"));
            }

            let header = &texture.info;
            if header.num_levels <= 1 {
                return Err(debug_fail!(
                    "Cannot generate mipmaps for texture with num_levels <= 1!"
                ));
            }

            if !header.usage.contains(TextureUsageFlags::SAMPLER)
                || !header.usage.contains(TextureUsageFlags::COLOR_TARGET)
            {
                return Err(debug_fail!(
                    "GenerateMipmaps texture must be created with SAMPLER and COLOR_TARGET usage flags!"
                ));
            }

            self.header.ignore_render_pass_texture_validation = true;
        }

        device.driver.generate_mipmaps(self, texture);

        if device.debug_mode {
            self.header.ignore_render_pass_texture_validation = false;
        }
        Ok(())
    }

    /// Blit from a source texture region to a destination texture region,
    /// outside any pass. Translation of `SDL_BlitGPUTexture()`.
    pub fn blit_texture(&mut self, info: &BlitInfo<'_>) -> Result<()> {
        let device = self.device.clone();
        if device.debug_mode {
            if self.header.any_pass_in_progress() {
                return Err(debug_fail!("Cannot blit during a pass!"));
            }

            // Validation
            let mut failed: Option<Error> = None;
            macro_rules! fail {
                ($msg:literal) => {{
                    let e = debug_fail!($msg);
                    failed.get_or_insert(e);
                }};
            }
            let src_header = &info.source.texture.info;
            let dst_header = &info.destination.texture.info;

            // (The textures can't be NULL.)
            if src_header.sample_count != SampleCount::One {
                fail!("Blit source texture must have a sample count of 1");
            }
            if !src_header.usage.contains(TextureUsageFlags::SAMPLER) {
                fail!("Blit source texture must be created with the SAMPLER usage flag");
            }
            if !dst_header.usage.contains(TextureUsageFlags::COLOR_TARGET) {
                fail!("Blit destination texture must be created with the COLOR_TARGET usage flag");
            }
            if src_header.format.is_depth_format() {
                fail!("Blit source texture cannot have a depth format");
            }
            if info.source.w == 0
                || info.source.h == 0
                || info.destination.w == 0
                || info.destination.h == 0
            {
                fail!(
                    "Blit source/destination regions must have non-zero width, height, and depth"
                );
            }

            if let Some(e) = failed {
                return Err(e);
            }
        }

        device.driver.blit(self, info);
        Ok(())
    }

    /// Acquire a texture to use in presentation, outside any pass.
    /// Translation of `SDL_AcquireGPUSwapchainTexture()`.
    ///
    /// When a swapchain texture is acquired, it will automatically be
    /// presented when the command buffer is submitted. `None` (without an
    /// error) means no texture is available right now, for instance
    /// because too many frames are in flight or the window is minimized;
    /// the command buffer can still be submitted.
    ///
    /// The window must have been claimed with [`Device::claim_window`].
    pub fn acquire_swapchain_texture(
        &mut self,
        window: &Window,
    ) -> Result<Option<SwapchainTexture>> {
        let (device, raw, header) = self.parts();
        if device.debug_mode && header.any_pass_in_progress() {
            return Err(debug_fail!(
                "Cannot acquire a swapchain texture during a pass!"
            ));
        }

        let result = device.driver.acquire_swapchain_texture(raw, *window);
        self.swapchain_texture(result)
    }

    /// Block the thread until a swapchain texture is available to be
    /// acquired, and then acquire it. Translation of
    /// `SDL_WaitAndAcquireGPUSwapchainTexture()`.
    ///
    /// `None` (without an error) means the window can't present now, for
    /// instance because it is minimized.
    pub fn wait_and_acquire_swapchain_texture(
        &mut self,
        window: &Window,
    ) -> Result<Option<SwapchainTexture>> {
        let (device, raw, header) = self.parts();
        if device.debug_mode && header.any_pass_in_progress() {
            return Err(debug_fail!(
                "Cannot acquire a swapchain texture during a pass!"
            ));
        }

        let result = device
            .driver
            .wait_and_acquire_swapchain_texture(raw, *window);
        self.swapchain_texture(result)
    }

    /// The front-end handle of an acquired swapchain texture.
    fn swapchain_texture(
        &mut self,
        result: Result<Option<BackendSwapchainTexture>>,
    ) -> Result<Option<SwapchainTexture>> {
        let acquired = result?;
        if acquired.is_some() {
            self.header.swapchain_texture_acquired = true;
        }

        Ok(acquired.map(|t| SwapchainTexture {
            texture: Texture {
                owner: Owner::Swapchain(self.device.clone()),
                raw: t.raw,
                info: t.info,
            },
            width: t.width,
            height: t.height,
        }))
    }

    /// Whether the command buffer can be submitted (no pass in progress).
    fn check_submit(&self) -> Result<()> {
        if self.device.debug_mode && self.header.any_pass_in_progress() {
            return Err(debug_fail!(
                "Cannot submit command buffer while a pass is in progress!"
            ));
        }
        Ok(())
    }

    /// Submit the command buffer for GPU processing (presenting the
    /// swapchain textures it acquired). Translation of
    /// `SDL_SubmitGPUCommandBuffer()`.
    pub fn submit(mut self) -> Result<()> {
        self.check_submit()?;

        let raw = self.raw.take().expect(SUBMITTED);
        self.device.driver.submit(raw)
    }

    /// Submit the command buffer and acquire a fence, signaled when the
    /// GPU is done with it. Translation of
    /// `SDL_SubmitGPUCommandBufferAndAcquireFence()`.
    pub fn submit_and_acquire_fence(mut self) -> Result<Fence> {
        self.check_submit()?;

        let raw = self.raw.take().expect(SUBMITTED);
        let fence = self.device.driver.submit_and_acquire_fence(raw)?;
        Ok(Fence {
            owner: Owner::Device(self.device.clone()),
            raw: fence,
        })
    }

    /// Cancel the command buffer: none of the enqueued commands are
    /// executed. Translation of `SDL_CancelGPUCommandBuffer()`.
    ///
    /// It is an error to cancel a command buffer after a swapchain texture
    /// has been acquired (that one is submitted when it is dropped).
    pub fn cancel(mut self) -> Result<()> {
        if self.device.debug_mode && self.header.swapchain_texture_acquired {
            return Err(debug_fail!(
                "Cannot cancel command buffer after a swapchain texture has been acquired!"
            ));
        }

        let raw = self.raw.take().expect(SUBMITTED);
        self.device.driver.cancel(raw)
    }
}

impl Drop for CommandBuffer {
    fn drop(&mut self) {
        if let Some(raw) = self.raw.take() {
            // (Upstream leaves a command buffer that is neither submitted
            // nor cancelled to the application; a swapchain texture can't
            // be cancelled.)
            let _ = if self.header.swapchain_texture_acquired {
                self.device.driver.submit(raw)
            } else {
                self.device.driver.cancel(raw)
            };
        }
    }
}

/// Whether `first_slot + num_bindings` exceeds `max`, computed without the
/// wrap-around of C's `Uint32` sum.
///
/// Note (upstream): in C a huge `first_slot` wraps the sum below the limit
/// and the debug layer then writes past its binding arrays.
fn exceeds(first_slot: u32, num_bindings: usize, max: u32) -> bool {
    first_slot as u64 + num_bindings as u64 > max as u64
}

/// `SDL_GPU_CheckGraphicsBindings()`.
fn check_graphics_bindings(header: &CommandBufferHeader) {
    let rp = &header.render_pass;
    let Some(pipeline) = rp.graphics_pipeline else {
        return;
    };
    for i in 0..pipeline.num_vertex_samplers as usize {
        if !rp.vertex_sampler_bound.get(i).copied().unwrap_or(false) {
            crate::sdl_assert_release!(!"Missing vertex sampler binding!");
        }
    }
    for i in 0..pipeline.num_vertex_storage_textures as usize {
        if !rp
            .vertex_storage_texture_bound
            .get(i)
            .copied()
            .unwrap_or(false)
        {
            crate::sdl_assert_release!(!"Missing vertex storage texture binding!");
        }
    }
    for i in 0..pipeline.num_vertex_storage_buffers as usize {
        if !rp
            .vertex_storage_buffer_bound
            .get(i)
            .copied()
            .unwrap_or(false)
        {
            crate::sdl_assert_release!(!"Missing vertex storage buffer binding!");
        }
    }
    for i in 0..pipeline.num_fragment_samplers as usize {
        if !rp.fragment_sampler_bound.get(i).copied().unwrap_or(false) {
            crate::sdl_assert_release!(!"Missing fragment sampler binding!");
        }
    }
    for i in 0..pipeline.num_fragment_storage_textures as usize {
        if !rp
            .fragment_storage_texture_bound
            .get(i)
            .copied()
            .unwrap_or(false)
        {
            crate::sdl_assert_release!(!"Missing fragment storage texture binding!");
        }
    }
    for i in 0..pipeline.num_fragment_storage_buffers as usize {
        if !rp
            .fragment_storage_buffer_bound
            .get(i)
            .copied()
            .unwrap_or(false)
        {
            crate::sdl_assert_release!(!"Missing fragment storage buffer binding!");
        }
    }
}

/// `SDL_GPU_CheckComputeBindings()`.
fn check_compute_bindings(header: &CommandBufferHeader) {
    let cp = &header.compute_pass;
    let Some(pipeline) = cp.compute_pipeline else {
        return;
    };
    for i in 0..pipeline.num_samplers as usize {
        if !cp.sampler_bound.get(i).copied().unwrap_or(false) {
            crate::sdl_assert_release!(!"Missing compute sampler binding!");
        }
    }
    for i in 0..pipeline.num_readonly_storage_textures as usize {
        if !cp
            .read_only_storage_texture_bound
            .get(i)
            .copied()
            .unwrap_or(false)
        {
            crate::sdl_assert_release!(!"Missing compute readonly storage texture binding!");
        }
    }
    for i in 0..pipeline.num_readonly_storage_buffers as usize {
        if !cp
            .read_only_storage_buffer_bound
            .get(i)
            .copied()
            .unwrap_or(false)
        {
            crate::sdl_assert_release!(!"Missing compute readonly storage buffer binding!");
        }
    }
    for i in 0..pipeline.num_readwrite_storage_textures as usize {
        if !cp
            .read_write_storage_texture_bound
            .get(i)
            .copied()
            .unwrap_or(false)
        {
            crate::sdl_assert_release!(!"Missing compute read-write storage texture binding!");
        }
    }
    for i in 0..pipeline.num_readwrite_storage_buffers as usize {
        if !cp
            .read_write_storage_buffer_bound
            .get(i)
            .copied()
            .unwrap_or(false)
        {
            crate::sdl_assert_release!(!"Missing compute read-write storage buffer bbinding!");
        }
    }
}

// Render Pass

/// A render pass in progress on a [`CommandBuffer`]. Translation of
/// `SDL_GPURenderPass *`; dropping it ends the pass
/// (`SDL_EndGPURenderPass()`).
///
/// (Upstream's "Render pass not in progress!" checks can't fail: the pass
/// exists only while it is in progress.)
pub struct RenderPass<'a> {
    cmd: &'a mut CommandBuffer,
}

impl std::fmt::Debug for RenderPass<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RenderPass")
            .field("state", &self.cmd.header.render_pass)
            .finish_non_exhaustive()
    }
}

impl RenderPass<'_> {
    /// The command buffer the pass is recorded into: for pushing uniform
    /// data and debug labels during the pass.
    pub fn command_buffer(&mut self) -> &mut CommandBuffer {
        self.cmd
    }

    /// Bind a graphics pipeline for use in rendering. Translation of
    /// `SDL_BindGPUGraphicsPipeline()`.
    pub fn bind_graphics_pipeline(&mut self, graphics_pipeline: &GraphicsPipeline) {
        let (device, raw, header) = self.cmd.parts();
        device
            .driver
            .bind_graphics_pipeline(raw, &graphics_pipeline.raw);

        if device.debug_mode {
            header.render_pass.graphics_pipeline = Some(graphics_pipeline.header);
        }
    }

    /// Set the current viewport state. Translation of
    /// `SDL_SetGPUViewport()`.
    pub fn set_viewport(&mut self, viewport: &Viewport) -> Result<()> {
        let (device, raw, _) = self.cmd.parts();
        if device.debug_mode {
            let max_w = device.max_viewport_width.load(Ordering::Relaxed);
            let max_h = device.max_viewport_height.load(Ordering::Relaxed);
            if (viewport.x + viewport.w) > max_w as f32 || (viewport.y + viewport.h) > max_h as f32
            {
                return Err(debug_fail!(
                    "Viewport size exceeds current render target dimensions"
                ));
            }
        }

        device.driver.set_viewport(raw, viewport);
        Ok(())
    }

    /// Set the current scissor state. Translation of `SDL_SetGPUScissor()`.
    pub fn set_scissor(&mut self, scissor: &Rect) -> Result<()> {
        let (device, raw, _) = self.cmd.parts();
        if device.debug_mode {
            let max_w = device.max_viewport_width.load(Ordering::Relaxed);
            let max_h = device.max_viewport_height.load(Ordering::Relaxed);
            if (scissor.x.wrapping_add(scissor.w) as u32) > max_w
                || (scissor.y.wrapping_add(scissor.h) as u32) > max_h
            {
                return Err(debug_fail!(
                    "Scissor rectangle size exceeds current render target dimensions"
                ));
            }
        }

        device.driver.set_scissor(raw, scissor);
        Ok(())
    }

    /// Set the current blend constants. Translation of
    /// `SDL_SetGPUBlendConstants()`.
    pub fn set_blend_constants(&mut self, blend_constants: FColor) {
        let (device, raw, _) = self.cmd.parts();
        device.driver.set_blend_constants(raw, blend_constants);
    }

    /// Set the current stencil reference value. Translation of
    /// `SDL_SetGPUStencilReference()`.
    pub fn set_stencil_reference(&mut self, reference: u8) {
        let (device, raw, _) = self.cmd.parts();
        device.driver.set_stencil_reference(raw, reference);
    }

    /// Bind vertex buffers for use with subsequent draw calls, from
    /// `first_slot` on. Translation of `SDL_BindGPUVertexBuffers()`.
    pub fn bind_vertex_buffers(&mut self, first_slot: u32, bindings: &[BufferBinding<'_>]) {
        let (device, raw, _) = self.cmd.parts();
        device.driver.bind_vertex_buffers(raw, first_slot, bindings);
    }

    /// Bind an index buffer for use with subsequent draw calls. Translation
    /// of `SDL_BindGPUIndexBuffer()`.
    pub fn bind_index_buffer(
        &mut self,
        binding: &BufferBinding<'_>,
        index_element_size: IndexElementSize,
    ) {
        let (device, raw, _) = self.cmd.parts();
        device
            .driver
            .bind_index_buffer(raw, binding, index_element_size);
    }

    /// Bind texture-sampler pairs for use on the vertex shader, from
    /// `first_slot` on. Translation of `SDL_BindGPUVertexSamplers()`.
    pub fn bind_vertex_samplers(
        &mut self,
        first_slot: u32,
        texture_sampler_bindings: &[TextureSamplerBinding<'_>],
    ) -> Result<()> {
        if exceeds(
            first_slot,
            texture_sampler_bindings.len(),
            MAX_TEXTURE_SAMPLERS_PER_STAGE,
        ) {
            return Err(Error::new(
                "first_slot + num_bindings exceeds MAX_TEXTURE_SAMPLERS_PER_STAGE",
            ));
        }

        let (device, raw, header) = self.cmd.parts();
        if device.debug_mode {
            if !header.ignore_render_pass_texture_validation {
                // (CHECK_SAMPLER_TEXTURES is disabled upstream.)

                for binding in texture_sampler_bindings {
                    if binding.texture.info.sample_count > SampleCount::One {
                        crate::sdl_assert_release!(
                            !"Multisample textures cannot be bound as samplers!"
                        );
                    }
                }
            }

            for i in 0..texture_sampler_bindings.len() {
                header.render_pass.vertex_sampler_bound[first_slot as usize + i] = true;
            }
        }

        if texture_sampler_bindings.is_empty() {
            return Ok(());
        }

        device
            .driver
            .bind_vertex_samplers(raw, first_slot, texture_sampler_bindings);
        Ok(())
    }

    /// Bind storage textures for use on the vertex shader, from
    /// `first_slot` on. Translation of `SDL_BindGPUVertexStorageTextures()`.
    pub fn bind_vertex_storage_textures(
        &mut self,
        first_slot: u32,
        storage_textures: &[&Texture],
    ) -> Result<()> {
        if exceeds(
            first_slot,
            storage_textures.len(),
            MAX_STORAGE_TEXTURES_PER_STAGE,
        ) {
            return Err(Error::new(
                "first_slot + num_bindings exceeds MAX_STORAGE_TEXTURES_PER_STAGE",
            ));
        }

        let (device, raw, header) = self.cmd.parts();
        if device.debug_mode {
            // (CHECK_STORAGE_TEXTURES is disabled upstream.)

            for i in 0..storage_textures.len() {
                header.render_pass.vertex_storage_texture_bound[first_slot as usize + i] = true;
            }
        }

        device
            .driver
            .bind_vertex_storage_textures(raw, first_slot, storage_textures);
        Ok(())
    }

    /// Bind storage buffers for use on the vertex shader, from
    /// `first_slot` on. Translation of `SDL_BindGPUVertexStorageBuffers()`.
    pub fn bind_vertex_storage_buffers(
        &mut self,
        first_slot: u32,
        storage_buffers: &[&Buffer],
    ) -> Result<()> {
        if exceeds(
            first_slot,
            storage_buffers.len(),
            MAX_STORAGE_BUFFERS_PER_STAGE,
        ) {
            return Err(Error::new(
                "first_slot + num_bindings exceeds MAX_STORAGE_BUFFERS_PER_STAGE",
            ));
        }

        let (device, raw, header) = self.cmd.parts();
        if device.debug_mode {
            for i in 0..storage_buffers.len() {
                header.render_pass.vertex_storage_buffer_bound[first_slot as usize + i] = true;
            }
        }

        device
            .driver
            .bind_vertex_storage_buffers(raw, first_slot, storage_buffers);
        Ok(())
    }

    /// Bind texture-sampler pairs for use on the fragment shader, from
    /// `first_slot` on. Translation of `SDL_BindGPUFragmentSamplers()`.
    pub fn bind_fragment_samplers(
        &mut self,
        first_slot: u32,
        texture_sampler_bindings: &[TextureSamplerBinding<'_>],
    ) -> Result<()> {
        if exceeds(
            first_slot,
            texture_sampler_bindings.len(),
            MAX_TEXTURE_SAMPLERS_PER_STAGE,
        ) {
            return Err(Error::new(
                "first_slot + num_bindings exceeds MAX_TEXTURE_SAMPLERS_PER_STAGE",
            ));
        }

        let (device, raw, header) = self.cmd.parts();
        if device.debug_mode {
            // (CHECK_SAMPLER_TEXTURES, unless ignore_render_pass_texture_validation,
            // is disabled upstream.)

            for binding in texture_sampler_bindings {
                if binding.texture.info.sample_count > SampleCount::One {
                    crate::sdl_assert_release!(
                        !"Multisample textures cannot be bound as samplers!"
                    );
                }
            }

            for i in 0..texture_sampler_bindings.len() {
                header.render_pass.fragment_sampler_bound[first_slot as usize + i] = true;
            }
        }

        if texture_sampler_bindings.is_empty() {
            return Ok(());
        }

        device
            .driver
            .bind_fragment_samplers(raw, first_slot, texture_sampler_bindings);
        Ok(())
    }

    /// Bind storage textures for use on the fragment shader, from
    /// `first_slot` on. Translation of
    /// `SDL_BindGPUFragmentStorageTextures()`.
    pub fn bind_fragment_storage_textures(
        &mut self,
        first_slot: u32,
        storage_textures: &[&Texture],
    ) -> Result<()> {
        if exceeds(
            first_slot,
            storage_textures.len(),
            MAX_STORAGE_TEXTURES_PER_STAGE,
        ) {
            return Err(Error::new(
                "first_slot + num_bindings exceeds MAX_STORAGE_TEXTURES_PER_STAGE",
            ));
        }

        let (device, raw, header) = self.cmd.parts();
        if device.debug_mode {
            // (CHECK_STORAGE_TEXTURES is disabled upstream.)

            for i in 0..storage_textures.len() {
                header.render_pass.fragment_storage_texture_bound[first_slot as usize + i] = true;
            }
        }

        device
            .driver
            .bind_fragment_storage_textures(raw, first_slot, storage_textures);
        Ok(())
    }

    /// Bind storage buffers for use on the fragment shader, from
    /// `first_slot` on. Translation of `SDL_BindGPUFragmentStorageBuffers()`.
    pub fn bind_fragment_storage_buffers(
        &mut self,
        first_slot: u32,
        storage_buffers: &[&Buffer],
    ) -> Result<()> {
        if exceeds(
            first_slot,
            storage_buffers.len(),
            MAX_STORAGE_BUFFERS_PER_STAGE,
        ) {
            return Err(Error::new(
                "first_slot + num_bindings exceeds MAX_STORAGE_BUFFERS_PER_STAGE",
            ));
        }

        let (device, raw, header) = self.cmd.parts();
        if device.debug_mode {
            for i in 0..storage_buffers.len() {
                header.render_pass.fragment_storage_buffer_bound[first_slot as usize + i] = true;
            }
        }

        device
            .driver
            .bind_fragment_storage_buffers(raw, first_slot, storage_buffers);
        Ok(())
    }

    /// The debug checks before a draw: a bound pipeline (an error) and its
    /// bindings (assertions).
    fn check_draw(header: &CommandBufferHeader) -> Result<()> {
        if header.render_pass.graphics_pipeline.is_none() {
            return Err(debug_fail!("Graphics pipeline not bound!"));
        }
        check_graphics_bindings(header);
        Ok(())
    }

    /// Draw data using bound graphics state with an index buffer and
    /// instancing enabled. Translation of `SDL_DrawGPUIndexedPrimitives()`.
    pub fn draw_indexed_primitives(
        &mut self,
        num_indices: u32,
        num_instances: u32,
        first_index: u32,
        vertex_offset: i32,
        first_instance: u32,
    ) -> Result<()> {
        let (device, raw, header) = self.cmd.parts();
        if device.debug_mode {
            Self::check_draw(header)?;
        }

        device.driver.draw_indexed_primitives(
            raw,
            num_indices,
            num_instances,
            first_index,
            vertex_offset,
            first_instance,
        );
        Ok(())
    }

    /// Draw data using bound graphics state. Translation of
    /// `SDL_DrawGPUPrimitives()`.
    pub fn draw_primitives(
        &mut self,
        num_vertices: u32,
        num_instances: u32,
        first_vertex: u32,
        first_instance: u32,
    ) -> Result<()> {
        let (device, raw, header) = self.cmd.parts();
        if device.debug_mode {
            Self::check_draw(header)?;
        }

        device.driver.draw_primitives(
            raw,
            num_vertices,
            num_instances,
            first_vertex,
            first_instance,
        );
        Ok(())
    }

    /// Draw data using bound graphics state and with draw parameters set
    /// from a buffer ([`IndirectDrawCommand`]s). Translation of
    /// `SDL_DrawGPUPrimitivesIndirect()`.
    pub fn draw_primitives_indirect(
        &mut self,
        buffer: &Buffer,
        offset: u32,
        draw_count: u32,
    ) -> Result<()> {
        let (device, raw, header) = self.cmd.parts();
        if device.debug_mode {
            Self::check_draw(header)?;
        }

        device
            .driver
            .draw_primitives_indirect(raw, &buffer.raw, offset, draw_count);
        Ok(())
    }

    /// Draw data using bound graphics state with an index buffer enabled
    /// and with draw parameters set from a buffer
    /// ([`IndexedIndirectDrawCommand`]s). Translation of
    /// `SDL_DrawGPUIndexedPrimitivesIndirect()`.
    pub fn draw_indexed_primitives_indirect(
        &mut self,
        buffer: &Buffer,
        offset: u32,
        draw_count: u32,
    ) -> Result<()> {
        let (device, raw, header) = self.cmd.parts();
        if device.debug_mode {
            Self::check_draw(header)?;
        }

        device
            .driver
            .draw_indexed_primitives_indirect(raw, &buffer.raw, offset, draw_count);
        Ok(())
    }

    /// End the render pass. Translation of `SDL_EndGPURenderPass()`; the
    /// same as dropping the pass.
    pub fn end(self) {}
}

impl Drop for RenderPass<'_> {
    fn drop(&mut self) {
        let (device, raw, header) = self.cmd.parts();
        device.driver.end_render_pass(raw);

        if device.debug_mode {
            let rp = &mut header.render_pass;
            rp.in_progress = false;
            rp.num_color_targets = 0;
            rp.graphics_pipeline = None;
            rp.vertex_sampler_bound = Default::default();
            rp.vertex_storage_texture_bound = Default::default();
            rp.vertex_storage_buffer_bound = Default::default();
            rp.fragment_sampler_bound = Default::default();
            rp.fragment_storage_texture_bound = Default::default();
            rp.fragment_storage_buffer_bound = Default::default();
        }
    }
}

// Compute Pass

/// A compute pass in progress on a [`CommandBuffer`]. Translation of
/// `SDL_GPUComputePass *`; dropping it ends the pass
/// (`SDL_EndGPUComputePass()`).
pub struct ComputePass<'a> {
    cmd: &'a mut CommandBuffer,
}

impl std::fmt::Debug for ComputePass<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ComputePass")
            .field("state", &self.cmd.header.compute_pass)
            .finish_non_exhaustive()
    }
}

impl ComputePass<'_> {
    /// The command buffer the pass is recorded into: for pushing uniform
    /// data and debug labels during the pass.
    pub fn command_buffer(&mut self) -> &mut CommandBuffer {
        self.cmd
    }

    /// Bind a compute pipeline for use in dispatch. Translation of
    /// `SDL_BindGPUComputePipeline()`.
    pub fn bind_compute_pipeline(&mut self, compute_pipeline: &ComputePipeline) {
        let (device, raw, header) = self.cmd.parts();
        device
            .driver
            .bind_compute_pipeline(raw, &compute_pipeline.raw);

        if device.debug_mode {
            header.compute_pass.compute_pipeline = Some(compute_pipeline.header);
        }
    }

    /// Bind texture-sampler pairs for use on the compute shader, from
    /// `first_slot` on. Translation of `SDL_BindGPUComputeSamplers()`.
    pub fn bind_compute_samplers(
        &mut self,
        first_slot: u32,
        texture_sampler_bindings: &[TextureSamplerBinding<'_>],
    ) -> Result<()> {
        if exceeds(
            first_slot,
            texture_sampler_bindings.len(),
            MAX_TEXTURE_SAMPLERS_PER_STAGE,
        ) {
            return Err(Error::new(
                "first_slot + num_bindings exceeds MAX_TEXTURE_SAMPLERS_PER_STAGE",
            ));
        }

        let (device, raw, header) = self.cmd.parts();
        if device.debug_mode {
            for binding in texture_sampler_bindings {
                if binding.texture.info.sample_count > SampleCount::One {
                    crate::sdl_assert_release!(
                        !"Multisample textures cannot be bound as samplers!"
                    );
                }
            }

            for i in 0..texture_sampler_bindings.len() {
                header.compute_pass.sampler_bound[first_slot as usize + i] = true;
            }
        }

        device
            .driver
            .bind_compute_samplers(raw, first_slot, texture_sampler_bindings);
        Ok(())
    }

    /// Bind storage textures as readonly for use on the compute pipeline,
    /// from `first_slot` on. Translation of
    /// `SDL_BindGPUComputeStorageTextures()`.
    pub fn bind_compute_storage_textures(
        &mut self,
        first_slot: u32,
        storage_textures: &[&Texture],
    ) -> Result<()> {
        if exceeds(
            first_slot,
            storage_textures.len(),
            MAX_STORAGE_TEXTURES_PER_STAGE,
        ) {
            return Err(Error::new(
                "first_slot + num_bindings exceeds MAX_STORAGE_TEXTURES_PER_STAGE",
            ));
        }

        let (device, raw, header) = self.cmd.parts();
        if device.debug_mode {
            for i in 0..storage_textures.len() {
                header.compute_pass.read_only_storage_texture_bound[first_slot as usize + i] = true;
            }
        }

        device
            .driver
            .bind_compute_storage_textures(raw, first_slot, storage_textures);
        Ok(())
    }

    /// Bind storage buffers as readonly for use on the compute pipeline,
    /// from `first_slot` on. Translation of
    /// `SDL_BindGPUComputeStorageBuffers()`.
    pub fn bind_compute_storage_buffers(
        &mut self,
        first_slot: u32,
        storage_buffers: &[&Buffer],
    ) -> Result<()> {
        if exceeds(
            first_slot,
            storage_buffers.len(),
            MAX_STORAGE_BUFFERS_PER_STAGE,
        ) {
            return Err(Error::new(
                "first_slot + num_bindings exceeds MAX_STORAGE_BUFFERS_PER_STAGE",
            ));
        }

        let (device, raw, header) = self.cmd.parts();
        if device.debug_mode {
            for i in 0..storage_buffers.len() {
                header.compute_pass.read_only_storage_buffer_bound[first_slot as usize + i] = true;
            }
        }

        device
            .driver
            .bind_compute_storage_buffers(raw, first_slot, storage_buffers);
        Ok(())
    }

    /// The debug checks before a dispatch: a bound pipeline (an error) and
    /// its bindings (assertions).
    fn check_dispatch(header: &CommandBufferHeader) -> Result<()> {
        if header.compute_pass.compute_pipeline.is_none() {
            return Err(debug_fail!("Compute pipeline not bound!"));
        }
        check_compute_bindings(header);
        Ok(())
    }

    /// Dispatch compute work. Translation of `SDL_DispatchGPUCompute()`.
    pub fn dispatch(
        &mut self,
        groupcount_x: u32,
        groupcount_y: u32,
        groupcount_z: u32,
    ) -> Result<()> {
        let (device, raw, header) = self.cmd.parts();
        if device.debug_mode {
            Self::check_dispatch(header)?;
        }

        device
            .driver
            .dispatch_compute(raw, groupcount_x, groupcount_y, groupcount_z);
        Ok(())
    }

    /// Dispatch compute work with parameters set from a buffer (an
    /// [`IndirectDispatchCommand`]). Translation of
    /// `SDL_DispatchGPUComputeIndirect()`.
    pub fn dispatch_indirect(&mut self, buffer: &Buffer, offset: u32) -> Result<()> {
        let (device, raw, header) = self.cmd.parts();
        if device.debug_mode {
            Self::check_dispatch(header)?;
        }

        device
            .driver
            .dispatch_compute_indirect(raw, &buffer.raw, offset);
        Ok(())
    }

    /// End the compute pass. Translation of `SDL_EndGPUComputePass()`; the
    /// same as dropping the pass.
    pub fn end(self) {}
}

impl Drop for ComputePass<'_> {
    fn drop(&mut self) {
        let (device, raw, header) = self.cmd.parts();
        device.driver.end_compute_pass(raw);

        if device.debug_mode {
            let cp = &mut header.compute_pass;
            cp.in_progress = false;
            cp.compute_pipeline = None;
            cp.sampler_bound = Default::default();
            cp.read_only_storage_texture_bound = Default::default();
            cp.read_only_storage_buffer_bound = Default::default();
            cp.read_write_storage_texture_bound = Default::default();
            cp.read_write_storage_buffer_bound = Default::default();
        }
    }
}

// Copy Pass

/// A copy pass in progress on a [`CommandBuffer`]. Translation of
/// `SDL_GPUCopyPass *`; dropping it ends the pass (`SDL_EndGPUCopyPass()`).
///
/// (Upstream's checks for NULL transfer buffers, buffers and textures can't
/// fail: the regions hold references.)
pub struct CopyPass<'a> {
    cmd: &'a mut CommandBuffer,
}

impl std::fmt::Debug for CopyPass<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CopyPass")
            .field("state", &self.cmd.header.copy_pass)
            .finish_non_exhaustive()
    }
}

impl CopyPass<'_> {
    /// The command buffer the pass is recorded into: for debug labels
    /// during the pass.
    pub fn command_buffer(&mut self) -> &mut CommandBuffer {
        self.cmd
    }

    /// Upload data from a transfer buffer to a texture. Translation of
    /// `SDL_UploadToGPUTexture()`.
    pub fn upload_to_texture(
        &mut self,
        source: &TextureTransferInfo<'_>,
        destination: &TextureRegion<'_>,
        cycle: bool,
    ) {
        let (device, raw, _) = self.cmd.parts();
        device
            .driver
            .upload_to_texture(raw, source, destination, cycle);
    }

    /// Upload data from a transfer buffer to a buffer. Translation of
    /// `SDL_UploadToGPUBuffer()`.
    pub fn upload_to_buffer(
        &mut self,
        source: &TransferBufferLocation<'_>,
        destination: &BufferRegion<'_>,
        cycle: bool,
    ) {
        let (device, raw, _) = self.cmd.parts();
        device
            .driver
            .upload_to_buffer(raw, source, destination, cycle);
    }

    /// Perform a texture-to-texture copy of a `w` x `h` x `d` region.
    /// Translation of `SDL_CopyGPUTextureToTexture()`.
    #[allow(clippy::too_many_arguments)]
    pub fn copy_texture_to_texture(
        &mut self,
        source: &TextureLocation<'_>,
        destination: &TextureLocation<'_>,
        w: u32,
        h: u32,
        d: u32,
        cycle: bool,
    ) -> Result<()> {
        let (device, raw, _) = self.cmd.parts();
        if device.debug_mode && source.texture.info.format != destination.texture.info.format {
            return Err(debug_fail!(
                "Source and destination textures must have the same format!"
            ));
        }

        device
            .driver
            .copy_texture_to_texture(raw, source, destination, w, h, d, cycle);
        Ok(())
    }

    /// Perform a buffer-to-buffer copy of `size` bytes. Translation of
    /// `SDL_CopyGPUBufferToBuffer()`.
    pub fn copy_buffer_to_buffer(
        &mut self,
        source: &BufferLocation<'_>,
        destination: &BufferLocation<'_>,
        size: u32,
        cycle: bool,
    ) {
        let (device, raw, _) = self.cmd.parts();
        device
            .driver
            .copy_buffer_to_buffer(raw, source, destination, size, cycle);
    }

    /// Copy data from a texture to a transfer buffer on the GPU timeline.
    /// Translation of `SDL_DownloadFromGPUTexture()`.
    ///
    /// This data is not guaranteed to be copied until the command buffer
    /// fence is signaled.
    pub fn download_from_texture(
        &mut self,
        source: &TextureRegion<'_>,
        destination: &TextureTransferInfo<'_>,
    ) {
        let (device, raw, _) = self.cmd.parts();
        device
            .driver
            .download_from_texture(raw, source, destination);
    }

    /// Copy data from a buffer to a transfer buffer on the GPU timeline.
    /// Translation of `SDL_DownloadFromGPUBuffer()`.
    ///
    /// This data is not guaranteed to be copied until the command buffer
    /// fence is signaled.
    pub fn download_from_buffer(
        &mut self,
        source: &BufferRegion<'_>,
        destination: &TransferBufferLocation<'_>,
    ) {
        let (device, raw, _) = self.cmd.parts();
        device.driver.download_from_buffer(raw, source, destination);
    }

    /// End the copy pass. Translation of `SDL_EndGPUCopyPass()`; the same
    /// as dropping the pass.
    pub fn end(self) {}
}

impl Drop for CopyPass<'_> {
    fn drop(&mut self) {
        let (device, raw, header) = self.cmd.parts();
        device.driver.end_copy_pass(raw);

        if device.debug_mode {
            header.copy_pass.in_progress = false;
        }
    }
}

// Texture format queries

impl TextureFormat {
    /// The texel block size for a texture format, in bytes. Translation of
    /// `SDL_GPUTextureFormatTexelBlockSize()`.
    ///
    /// An unrecognized format triggers an assertion and returns 0.
    pub fn texel_block_size(self) -> u32 {
        use TextureFormat as F;
        match self {
            F::BC1_RGBA_UNORM | F::BC1_RGBA_UNORM_SRGB | F::BC4_R_UNORM => 8,
            F::BC2_RGBA_UNORM
            | F::BC3_RGBA_UNORM
            | F::BC5_RG_UNORM
            | F::BC7_RGBA_UNORM
            | F::BC6H_RGB_FLOAT
            | F::BC6H_RGB_UFLOAT
            | F::BC2_RGBA_UNORM_SRGB
            | F::BC3_RGBA_UNORM_SRGB
            | F::BC7_RGBA_UNORM_SRGB => 16,
            F::R8_UNORM | F::R8_SNORM | F::A8_UNORM | F::R8_UINT | F::R8_INT => 1,
            F::B5G6R5_UNORM
            | F::B4G4R4A4_UNORM
            | F::B5G5R5A1_UNORM
            | F::R16_FLOAT
            | F::R8G8_SNORM
            | F::R8G8_UNORM
            | F::R8G8_UINT
            | F::R8G8_INT
            | F::R16_UNORM
            | F::R16_SNORM
            | F::R16_UINT
            | F::R16_INT
            | F::D16_UNORM => 2,
            F::R8G8B8A8_UNORM
            | F::B8G8R8A8_UNORM
            | F::R8G8B8A8_UNORM_SRGB
            | F::B8G8R8A8_UNORM_SRGB
            | F::R32_FLOAT
            | F::R16G16_FLOAT
            | F::R11G11B10_UFLOAT
            | F::R8G8B8A8_SNORM
            | F::R10G10B10A2_UNORM
            | F::R8G8B8A8_UINT
            | F::R8G8B8A8_INT
            | F::R16G16_UINT
            | F::R16G16_INT
            | F::R16G16_UNORM
            | F::R16G16_SNORM
            | F::D24_UNORM
            | F::D32_FLOAT
            | F::R32_UINT
            | F::R32_INT
            | F::D24_UNORM_S8_UINT => 4,
            F::D32_FLOAT_S8_UINT => 5,
            F::R16G16B16A16_FLOAT
            | F::R16G16B16A16_UNORM
            | F::R16G16B16A16_SNORM
            | F::R16G16B16A16_UINT
            | F::R16G16B16A16_INT
            | F::R32G32_FLOAT
            | F::R32G32_UINT
            | F::R32G32_INT => 8,
            F::R32G32B32A32_FLOAT | F::R32G32B32A32_INT | F::R32G32B32A32_UINT => 16,
            F::ASTC_4x4_UNORM
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
            | F::ASTC_12x12_FLOAT => 16,
            _ => {
                crate::sdl_assert_release!(!"Unrecognized TextureFormat!");
                0
            }
        }
    }

    /// The size in bytes of a texture of this format with the given
    /// dimensions (whole blocks). Translation of
    /// `SDL_CalculateGPUTextureFormatSize()`.
    ///
    /// The arithmetic wraps around in 32 bits, as in C.
    pub fn calculate_size(self, width: u32, height: u32, depth_or_layer_count: u32) -> u32 {
        let block_width = self.block_width().max(1) as u32;
        let block_height = self.block_height().max(1) as u32;
        let blocks_per_row = width.wrapping_add(block_width).wrapping_sub(1) / block_width;
        let blocks_per_column = height.wrapping_add(block_height).wrapping_sub(1) / block_height;
        depth_or_layer_count
            .wrapping_mul(blocks_per_row)
            .wrapping_mul(blocks_per_column)
            .wrapping_mul(self.texel_block_size())
    }

    /// The pixel format corresponding to this texture format, if there is
    /// one. Translation of `SDL_GetPixelFormatFromGPUTextureFormat()`.
    pub fn pixel_format(self) -> Option<PixelFormat> {
        use TextureFormat as F;
        Some(match self {
            F::B4G4R4A4_UNORM => PixelFormat::ARGB4444,
            F::B5G6R5_UNORM => PixelFormat::RGB565,
            F::B5G5R5A1_UNORM => PixelFormat::ARGB1555,
            F::R8G8B8A8_UINT => PixelFormat::RGBA32,
            F::R8G8B8A8_SNORM => PixelFormat::RGBA32,
            F::R8G8B8A8_UNORM => PixelFormat::RGBA32,
            F::R8G8B8A8_UNORM_SRGB => PixelFormat::RGBA32,
            F::B8G8R8A8_UNORM => PixelFormat::BGRA32,
            F::B8G8R8A8_UNORM_SRGB => PixelFormat::BGRA32,
            F::R10G10B10A2_UNORM => PixelFormat::ABGR2101010,
            F::R16G16B16A16_UINT => PixelFormat::RGBA64,
            F::R16G16B16A16_UNORM => PixelFormat::RGBA64,
            F::R16G16B16A16_FLOAT => PixelFormat::RGBA64_FLOAT,
            F::R32G32B32A32_FLOAT => PixelFormat::RGBA128_FLOAT,
            _ => return None,
        })
    }

    /// The texture format corresponding to a pixel format, if there is one.
    /// Translation of `SDL_GetGPUTextureFormatFromPixelFormat()`.
    pub fn from_pixel_format(format: PixelFormat) -> Option<TextureFormat> {
        Some(match format {
            PixelFormat::ARGB4444 => TextureFormat::B4G4R4A4_UNORM,
            PixelFormat::RGB565 => TextureFormat::B5G6R5_UNORM,
            PixelFormat::ARGB1555 => TextureFormat::B5G5R5A1_UNORM,
            PixelFormat::BGRA32 | PixelFormat::BGRX32 => TextureFormat::B8G8R8A8_UNORM,
            PixelFormat::RGBA32 | PixelFormat::RGBX32 => TextureFormat::R8G8B8A8_UNORM,
            PixelFormat::ABGR2101010 => TextureFormat::R10G10B10A2_UNORM,
            PixelFormat::RGBA64 => TextureFormat::R16G16B16A16_UNORM,
            PixelFormat::RGBA64_FLOAT => TextureFormat::R16G16B16A16_FLOAT,
            PixelFormat::RGBA128_FLOAT => TextureFormat::R32G32B32A32_FLOAT,
            _ => return None,
        })
    }
}
