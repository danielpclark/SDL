// Rust translation of include/SDL3_shadercross/SDL_shadercross.h from
// SDL_shadercross.
// Copyright (C) 2024 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! # sdl3-shadercross — SDL_shadercross, translated to Rust
//!
//! The pure-Rust translation of
//! [SDL_shadercross](https://github.com/libsdl-org/SDL_shadercross) 3, the
//! shader translation library for SDL3's GPU API, with the parts of
//! [SPIRV-Cross](https://github.com/KhronosGroup/SPIRV-Cross) it builds on
//! (the `spirv_cross` module): SPIR-V shaders are reflected for their
//! resource counts and I/O ([`reflect_graphics_spirv`],
//! [`reflect_compute_spirv`]), transpiled to Metal Shading Language
//! ([`transpile_msl_from_spirv`]) or HLSL ([`transpile_hlsl_from_spirv`]),
//! and turned straight into [`sdl3::gpu`] shaders and compute pipelines in
//! whichever format the device takes
//! ([`compile_graphics_shader_from_spirv`],
//! [`compile_compute_pipeline_from_spirv`]).
//!
//! DXBC is compiled from HLSL by Windows' `d3dcompiler_47.dll`, which
//! [`init`] loads as upstream does; elsewhere (where upstream loads the
//! non-system vkd3d-utils library) DXBC isn't available. DXIL and SPIR-V
//! from HLSL need DXC, which this build doesn't have: those functions
//! fail with upstream's messages for a build without DXC.
//!
//! As in the [`sdl3`] crate, the implementation is a line-by-line
//! translation and the API is designed for Rust: buffers are [`Vec`]s and
//! [`String`]s, metadata owns its arrays, errors are
//! [`Result`](sdl3::Result)s, and every item names the C symbol it
//! translates. The `shadercross` command-line tool is the crate's binary.

#![deny(unsafe_code)]
#![warn(missing_debug_implementations)]

use sdl3::properties::Properties;

mod shadercross;
pub use shadercross::*;

#[doc(hidden)]
pub mod spirv_cross;

#[cfg(test)]
mod tests;

/// `SDL_SHADERCROSS_MAJOR_VERSION`.
pub const MAJOR_VERSION: i32 = 3;
/// `SDL_SHADERCROSS_MINOR_VERSION`.
pub const MINOR_VERSION: i32 = 0;
/// `SDL_SHADERCROSS_MICRO_VERSION`.
pub const MICRO_VERSION: i32 = 0;

/// The type of a shader input or output variable. Translation of
/// `SDL_ShaderCross_IOVarType`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum IOVarType {
    /// `SDL_SHADERCROSS_IOVAR_TYPE_UNKNOWN`.
    #[default]
    Unknown,
    /// `SDL_SHADERCROSS_IOVAR_TYPE_INT8`.
    Int8,
    /// `SDL_SHADERCROSS_IOVAR_TYPE_UINT8`.
    UInt8,
    /// `SDL_SHADERCROSS_IOVAR_TYPE_INT16`.
    Int16,
    /// `SDL_SHADERCROSS_IOVAR_TYPE_UINT16`.
    UInt16,
    /// `SDL_SHADERCROSS_IOVAR_TYPE_INT32`.
    Int32,
    /// `SDL_SHADERCROSS_IOVAR_TYPE_UINT32`.
    UInt32,
    /// `SDL_SHADERCROSS_IOVAR_TYPE_INT64`.
    Int64,
    /// `SDL_SHADERCROSS_IOVAR_TYPE_UINT64`.
    UInt64,
    /// `SDL_SHADERCROSS_IOVAR_TYPE_FLOAT16`.
    Float16,
    /// `SDL_SHADERCROSS_IOVAR_TYPE_FLOAT32`.
    Float32,
    /// `SDL_SHADERCROSS_IOVAR_TYPE_FLOAT64`.
    Float64,
}

/// A shader stage. Translation of `SDL_ShaderCross_ShaderStage`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum ShaderStage {
    /// `SDL_SHADERCROSS_SHADERSTAGE_VERTEX`.
    #[default]
    Vertex,
    /// `SDL_SHADERCROSS_SHADERSTAGE_FRAGMENT`.
    Fragment,
    /// `SDL_SHADERCROSS_SHADERSTAGE_COMPUTE`.
    Compute,
}

/// A shader input or output variable. Translation of
/// `SDL_ShaderCross_IOVarMetadata`.
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub struct IOVarMetadata {
    /// The UTF-8 name of the variable.
    pub name: String,
    /// The location of the variable.
    pub location: u32,
    /// The vector type of the variable.
    pub vector_type: IOVarType,
    /// The number of components in the vector type of the variable.
    pub vector_size: u32,
}

/// The resource counts of a graphics shader. Translation of
/// `SDL_ShaderCross_GraphicsShaderResourceInfo`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct GraphicsShaderResourceInfo {
    /// The number of samplers defined in the shader.
    pub num_samplers: u32,
    /// The number of storage textures defined in the shader.
    pub num_storage_textures: u32,
    /// The number of storage buffers defined in the shader.
    pub num_storage_buffers: u32,
    /// The number of uniform buffers defined in the shader.
    pub num_uniform_buffers: u32,
}

/// The metadata of a graphics shader. Translation of
/// `SDL_ShaderCross_GraphicsShaderMetadata` (whose `num_inputs` and
/// `num_outputs` are the lengths of `inputs` and `outputs`).
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub struct GraphicsShaderMetadata {
    /// Sub-struct containing the resource info of the shader.
    pub resource_info: GraphicsShaderResourceInfo,
    /// The inputs defined in the shader.
    pub inputs: Vec<IOVarMetadata>,
    /// The outputs defined in the shader.
    pub outputs: Vec<IOVarMetadata>,
}

/// The metadata of a compute pipeline. Translation of
/// `SDL_ShaderCross_ComputePipelineMetadata`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct ComputePipelineMetadata {
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
}

/// A SPIR-V shader to translate. Translation of `SDL_ShaderCross_SPIRV_Info`
/// (`bytecode_size` is the slice's length).
#[derive(Clone, Copy, Debug, Default)]
pub struct SpirvInfo<'a> {
    /// The SPIRV bytecode.
    pub bytecode: &'a [u8],
    /// The entry point function name for the shader in UTF-8.
    pub entrypoint: &'a str,
    /// The shader stage to transpile the shader with.
    pub shader_stage: ShaderStage,
    /// A properties group for extensions; `None` if no extensions are needed.
    pub props: Option<&'a Properties>,
}

/// Enable debug information in the compiled shader. Translation of
/// `SDL_SHADERCROSS_PROP_SHADER_DEBUG_ENABLE_BOOLEAN`.
pub const PROP_SHADER_DEBUG_ENABLE_BOOLEAN: &str = "SDL_shadercross.spirv.debug.enable";
/// A name for the shader, shown in debugging tools. Translation of
/// `SDL_SHADERCROSS_PROP_SHADER_DEBUG_NAME_STRING`.
pub const PROP_SHADER_DEBUG_NAME_STRING: &str = "SDL_shadercross.spirv.debug.name";
/// Cull unused bindings. Translation of
/// `SDL_SHADERCROSS_PROP_SHADER_CULL_UNUSED_BINDINGS_BOOLEAN`.
pub const PROP_SHADER_CULL_UNUSED_BINDINGS_BOOLEAN: &str =
    "SDL_shadercross.spirv.cull_unused_bindings";
/// Emit HLSL compatible with PSSL (shader model 5.0, `main` entry points).
/// Translation of `SDL_SHADERCROSS_PROP_SPIRV_PSSL_COMPATIBILITY_BOOLEAN`.
pub const PROP_SPIRV_PSSL_COMPATIBILITY_BOOLEAN: &str = "SDL_shadercross.spirv.pssl.compatibility";
/// The MSL version to emit, as "major.minor.patch" (default "1.2.0").
/// Translation of `SDL_SHADERCROSS_PROP_SPIRV_MSL_VERSION_STRING`.
pub const PROP_SPIRV_MSL_VERSION_STRING: &str = "SDL_shadercross.spirv.msl.version";
/// Compile HLSL directly instead of round-tripping it through SPIR-V.
/// Translation of `SDL_SHADERCROSS_PROP_HLSL_SKIP_SPIRV_ROUNDTRIP_BOOLEAN`.
pub const PROP_HLSL_SKIP_SPIRV_ROUNDTRIP_BOOLEAN: &str =
    "SDL_shadercross.hlsl.skip_spirv_roundtrip";

/// An HLSL preprocessor define. Translation of
/// `SDL_ShaderCross_HLSL_Define`.
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub struct HlslDefine {
    /// The define name.
    pub name: String,
    /// An optional value for the define.
    pub value: Option<String>,
}

/// An HLSL shader to compile. Translation of `SDL_ShaderCross_HLSL_Info`
/// (the define list is a slice instead of a NULL-terminated array).
#[derive(Clone, Copy, Debug, Default)]
pub struct HlslInfo<'a> {
    /// The HLSL source code for the shader.
    pub source: &'a str,
    /// The entry point function name for the shader in UTF-8.
    pub entrypoint: &'a str,
    /// The include directory for shader code. Optional.
    pub include_dir: Option<&'a str>,
    /// The defines. Optional.
    pub defines: Option<&'a [HlslDefine]>,
    /// The shader stage to compile the shader with.
    pub shader_stage: ShaderStage,
    /// A properties group for extensions; `None` if no extensions are needed.
    pub props: Option<&'a Properties>,
}
