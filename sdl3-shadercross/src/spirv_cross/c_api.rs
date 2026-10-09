// Rust translation of spirv_cross_c.h and spirv_cross_c.cpp from
// SPIRV-Cross (the parts SDL_shadercross calls).
// Copyright 2019-2021 Hans-Kristian Arntzen
// SPDX-License-Identifier: Apache-2.0 OR MIT
// This is an altered (translated to Rust) version of the original
// software; see LICENSE.txt.

//! The C API, `spvc_*`, as SDL_shadercross uses it.
//!
//! The opaque handles become owned values: a [`SpvcContext`] keeps the
//! last error (`spvc_context_get_last_error_string()`), and the parsed IR,
//! compilers, option sets, variable sets and resource lists it would own
//! (and free in `spvc_context_destroy()`) are returned to the caller. The
//! `SPVC_BEGIN_SAFE_SCOPE`/`SPVC_END_SAFE_SCOPE` exception handlers are
//! the `Err` arms that report the error to the context.

use std::collections::HashSet;

use super::common::*;
use super::cross::*;
use super::glsl::*;
use super::hlsl::*;
use super::msl::*;
use super::parsed_ir::ParsedIR;
use super::parser::Parser;
use super::spirv::*;

/// `spvc_result`.
pub type SpvcResult = i32;
/// Success.
pub const SPVC_SUCCESS: SpvcResult = 0;
/// The SPIR-V is invalid. Should have been caught by validation ideally.
pub const SPVC_ERROR_INVALID_SPIRV: SpvcResult = -1;
/// The SPIR-V might be valid or invalid, but SPIRV-Cross currently cannot correctly translate this to your target language.
pub const SPVC_ERROR_UNSUPPORTED_SPIRV: SpvcResult = -2;
/// If for some reason we hit this, new or malloc failed.
pub const SPVC_ERROR_OUT_OF_MEMORY: SpvcResult = -3;
/// Invalid API argument.
pub const SPVC_ERROR_INVALID_ARGUMENT: SpvcResult = -4;

/// `spvc_backend`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(i32)]
pub enum SpvcBackend {
    /// This backend can only perform reflection, no compiler options are supported. Maps to spirv_cross::Compiler.
    None = 0,
    /// spirv_cross::CompilerGLSL
    Glsl = 1,
    /// CompilerHLSL
    Hlsl = 2,
    /// CompilerMSL
    Msl = 3,
}

/// `spvc_resource_type`.
pub type SpvcResourceType = u32;
pub const SPVC_RESOURCE_TYPE_UNKNOWN: SpvcResourceType = 0;
pub const SPVC_RESOURCE_TYPE_UNIFORM_BUFFER: SpvcResourceType = 1;
pub const SPVC_RESOURCE_TYPE_STORAGE_BUFFER: SpvcResourceType = 2;
pub const SPVC_RESOURCE_TYPE_STAGE_INPUT: SpvcResourceType = 3;
pub const SPVC_RESOURCE_TYPE_STAGE_OUTPUT: SpvcResourceType = 4;
pub const SPVC_RESOURCE_TYPE_SUBPASS_INPUT: SpvcResourceType = 5;
pub const SPVC_RESOURCE_TYPE_STORAGE_IMAGE: SpvcResourceType = 6;
pub const SPVC_RESOURCE_TYPE_SAMPLED_IMAGE: SpvcResourceType = 7;
pub const SPVC_RESOURCE_TYPE_ATOMIC_COUNTER: SpvcResourceType = 8;
pub const SPVC_RESOURCE_TYPE_PUSH_CONSTANT: SpvcResourceType = 9;
pub const SPVC_RESOURCE_TYPE_SEPARATE_IMAGE: SpvcResourceType = 10;
pub const SPVC_RESOURCE_TYPE_SEPARATE_SAMPLERS: SpvcResourceType = 11;
pub const SPVC_RESOURCE_TYPE_ACCELERATION_STRUCTURE: SpvcResourceType = 12;
pub const SPVC_RESOURCE_TYPE_RAY_QUERY: SpvcResourceType = 13;
pub const SPVC_RESOURCE_TYPE_SHADER_RECORD_BUFFER: SpvcResourceType = 14;
pub const SPVC_RESOURCE_TYPE_GL_PLAIN_UNIFORM: SpvcResourceType = 15;
pub const SPVC_RESOURCE_TYPE_TENSOR: SpvcResourceType = 16;

/// `spvc_basetype` (the values of `SPIRType::BaseType`).
pub type SpvcBasetype = u32;
pub const SPVC_BASETYPE_UNKNOWN: SpvcBasetype = 0;
pub const SPVC_BASETYPE_VOID: SpvcBasetype = 1;
pub const SPVC_BASETYPE_BOOLEAN: SpvcBasetype = 2;
pub const SPVC_BASETYPE_INT8: SpvcBasetype = 3;
pub const SPVC_BASETYPE_UINT8: SpvcBasetype = 4;
pub const SPVC_BASETYPE_INT16: SpvcBasetype = 5;
pub const SPVC_BASETYPE_UINT16: SpvcBasetype = 6;
pub const SPVC_BASETYPE_INT32: SpvcBasetype = 7;
pub const SPVC_BASETYPE_UINT32: SpvcBasetype = 8;
pub const SPVC_BASETYPE_INT64: SpvcBasetype = 9;
pub const SPVC_BASETYPE_UINT64: SpvcBasetype = 10;
pub const SPVC_BASETYPE_ATOMIC_COUNTER: SpvcBasetype = 11;
pub const SPVC_BASETYPE_FP16: SpvcBasetype = 12;
pub const SPVC_BASETYPE_FP32: SpvcBasetype = 13;
pub const SPVC_BASETYPE_FP64: SpvcBasetype = 14;
pub const SPVC_BASETYPE_STRUCT: SpvcBasetype = 15;
pub const SPVC_BASETYPE_IMAGE: SpvcBasetype = 16;
pub const SPVC_BASETYPE_SAMPLED_IMAGE: SpvcBasetype = 17;
pub const SPVC_BASETYPE_SAMPLER: SpvcBasetype = 18;
pub const SPVC_BASETYPE_ACCELERATION_STRUCTURE: SpvcBasetype = 19;

pub const SPVC_COMPILER_OPTION_COMMON_BIT: u32 = 0x1000000;
pub const SPVC_COMPILER_OPTION_GLSL_BIT: u32 = 0x2000000;
pub const SPVC_COMPILER_OPTION_HLSL_BIT: u32 = 0x4000000;
pub const SPVC_COMPILER_OPTION_MSL_BIT: u32 = 0x8000000;
pub const SPVC_COMPILER_OPTION_LANG_BITS: u32 = 0x0f000000;

/// `spvc_compiler_option`.
pub type SpvcCompilerOption = u32;
pub const SPVC_COMPILER_OPTION_FORCE_TEMPORARY: SpvcCompilerOption =
    1 | SPVC_COMPILER_OPTION_COMMON_BIT;
pub const SPVC_COMPILER_OPTION_FLATTEN_MULTIDIMENSIONAL_ARRAYS: SpvcCompilerOption =
    2 | SPVC_COMPILER_OPTION_COMMON_BIT;
pub const SPVC_COMPILER_OPTION_FIXUP_DEPTH_CONVENTION: SpvcCompilerOption =
    3 | SPVC_COMPILER_OPTION_COMMON_BIT;
pub const SPVC_COMPILER_OPTION_FLIP_VERTEX_Y: SpvcCompilerOption =
    4 | SPVC_COMPILER_OPTION_COMMON_BIT;
pub const SPVC_COMPILER_OPTION_GLSL_SUPPORT_NONZERO_BASE_INSTANCE: SpvcCompilerOption =
    5 | SPVC_COMPILER_OPTION_GLSL_BIT;
pub const SPVC_COMPILER_OPTION_GLSL_SEPARATE_SHADER_OBJECTS: SpvcCompilerOption =
    6 | SPVC_COMPILER_OPTION_GLSL_BIT;
pub const SPVC_COMPILER_OPTION_GLSL_ENABLE_420PACK_EXTENSION: SpvcCompilerOption =
    7 | SPVC_COMPILER_OPTION_GLSL_BIT;
pub const SPVC_COMPILER_OPTION_GLSL_VERSION: SpvcCompilerOption = 8 | SPVC_COMPILER_OPTION_GLSL_BIT;
pub const SPVC_COMPILER_OPTION_GLSL_ES: SpvcCompilerOption = 9 | SPVC_COMPILER_OPTION_GLSL_BIT;
pub const SPVC_COMPILER_OPTION_GLSL_VULKAN_SEMANTICS: SpvcCompilerOption =
    10 | SPVC_COMPILER_OPTION_GLSL_BIT;
pub const SPVC_COMPILER_OPTION_GLSL_ES_DEFAULT_FLOAT_PRECISION_HIGHP: SpvcCompilerOption =
    11 | SPVC_COMPILER_OPTION_GLSL_BIT;
pub const SPVC_COMPILER_OPTION_GLSL_ES_DEFAULT_INT_PRECISION_HIGHP: SpvcCompilerOption =
    12 | SPVC_COMPILER_OPTION_GLSL_BIT;
pub const SPVC_COMPILER_OPTION_HLSL_SHADER_MODEL: SpvcCompilerOption =
    13 | SPVC_COMPILER_OPTION_HLSL_BIT;
pub const SPVC_COMPILER_OPTION_HLSL_POINT_SIZE_COMPAT: SpvcCompilerOption =
    14 | SPVC_COMPILER_OPTION_HLSL_BIT;
pub const SPVC_COMPILER_OPTION_HLSL_POINT_COORD_COMPAT: SpvcCompilerOption =
    15 | SPVC_COMPILER_OPTION_HLSL_BIT;
pub const SPVC_COMPILER_OPTION_HLSL_SUPPORT_NONZERO_BASE_VERTEX_BASE_INSTANCE: SpvcCompilerOption =
    16 | SPVC_COMPILER_OPTION_HLSL_BIT;
pub const SPVC_COMPILER_OPTION_MSL_VERSION: SpvcCompilerOption = 17 | SPVC_COMPILER_OPTION_MSL_BIT;
pub const SPVC_COMPILER_OPTION_MSL_TEXEL_BUFFER_TEXTURE_WIDTH: SpvcCompilerOption =
    18 | SPVC_COMPILER_OPTION_MSL_BIT;
// Obsolete, use SWIZZLE_BUFFER_INDEX instead.
pub const SPVC_COMPILER_OPTION_MSL_AUX_BUFFER_INDEX: SpvcCompilerOption =
    19 | SPVC_COMPILER_OPTION_MSL_BIT;
pub const SPVC_COMPILER_OPTION_MSL_SWIZZLE_BUFFER_INDEX: SpvcCompilerOption =
    19 | SPVC_COMPILER_OPTION_MSL_BIT;
pub const SPVC_COMPILER_OPTION_MSL_INDIRECT_PARAMS_BUFFER_INDEX: SpvcCompilerOption =
    20 | SPVC_COMPILER_OPTION_MSL_BIT;
pub const SPVC_COMPILER_OPTION_MSL_SHADER_OUTPUT_BUFFER_INDEX: SpvcCompilerOption =
    21 | SPVC_COMPILER_OPTION_MSL_BIT;
pub const SPVC_COMPILER_OPTION_MSL_SHADER_PATCH_OUTPUT_BUFFER_INDEX: SpvcCompilerOption =
    22 | SPVC_COMPILER_OPTION_MSL_BIT;
pub const SPVC_COMPILER_OPTION_MSL_SHADER_TESS_FACTOR_OUTPUT_BUFFER_INDEX: SpvcCompilerOption =
    23 | SPVC_COMPILER_OPTION_MSL_BIT;
pub const SPVC_COMPILER_OPTION_MSL_SHADER_INPUT_WORKGROUP_INDEX: SpvcCompilerOption =
    24 | SPVC_COMPILER_OPTION_MSL_BIT;
pub const SPVC_COMPILER_OPTION_MSL_ENABLE_POINT_SIZE_BUILTIN: SpvcCompilerOption =
    25 | SPVC_COMPILER_OPTION_MSL_BIT;
pub const SPVC_COMPILER_OPTION_MSL_DISABLE_RASTERIZATION: SpvcCompilerOption =
    26 | SPVC_COMPILER_OPTION_MSL_BIT;
pub const SPVC_COMPILER_OPTION_MSL_CAPTURE_OUTPUT_TO_BUFFER: SpvcCompilerOption =
    27 | SPVC_COMPILER_OPTION_MSL_BIT;
pub const SPVC_COMPILER_OPTION_MSL_SWIZZLE_TEXTURE_SAMPLES: SpvcCompilerOption =
    28 | SPVC_COMPILER_OPTION_MSL_BIT;
pub const SPVC_COMPILER_OPTION_MSL_PAD_FRAGMENT_OUTPUT_COMPONENTS: SpvcCompilerOption =
    29 | SPVC_COMPILER_OPTION_MSL_BIT;
pub const SPVC_COMPILER_OPTION_MSL_TESS_DOMAIN_ORIGIN_LOWER_LEFT: SpvcCompilerOption =
    30 | SPVC_COMPILER_OPTION_MSL_BIT;
pub const SPVC_COMPILER_OPTION_MSL_PLATFORM: SpvcCompilerOption = 31 | SPVC_COMPILER_OPTION_MSL_BIT;
pub const SPVC_COMPILER_OPTION_MSL_ARGUMENT_BUFFERS: SpvcCompilerOption =
    32 | SPVC_COMPILER_OPTION_MSL_BIT;
pub const SPVC_COMPILER_OPTION_GLSL_EMIT_PUSH_CONSTANT_AS_UNIFORM_BUFFER: SpvcCompilerOption =
    33 | SPVC_COMPILER_OPTION_GLSL_BIT;
pub const SPVC_COMPILER_OPTION_MSL_TEXTURE_BUFFER_NATIVE: SpvcCompilerOption =
    34 | SPVC_COMPILER_OPTION_MSL_BIT;
pub const SPVC_COMPILER_OPTION_GLSL_EMIT_UNIFORM_BUFFER_AS_PLAIN_UNIFORMS: SpvcCompilerOption =
    35 | SPVC_COMPILER_OPTION_GLSL_BIT;
pub const SPVC_COMPILER_OPTION_MSL_BUFFER_SIZE_BUFFER_INDEX: SpvcCompilerOption =
    36 | SPVC_COMPILER_OPTION_MSL_BIT;
pub const SPVC_COMPILER_OPTION_EMIT_LINE_DIRECTIVES: SpvcCompilerOption =
    37 | SPVC_COMPILER_OPTION_COMMON_BIT;
pub const SPVC_COMPILER_OPTION_MSL_MULTIVIEW: SpvcCompilerOption =
    38 | SPVC_COMPILER_OPTION_MSL_BIT;
pub const SPVC_COMPILER_OPTION_MSL_VIEW_MASK_BUFFER_INDEX: SpvcCompilerOption =
    39 | SPVC_COMPILER_OPTION_MSL_BIT;
pub const SPVC_COMPILER_OPTION_MSL_DEVICE_INDEX: SpvcCompilerOption =
    40 | SPVC_COMPILER_OPTION_MSL_BIT;
pub const SPVC_COMPILER_OPTION_MSL_VIEW_INDEX_FROM_DEVICE_INDEX: SpvcCompilerOption =
    41 | SPVC_COMPILER_OPTION_MSL_BIT;
pub const SPVC_COMPILER_OPTION_MSL_DISPATCH_BASE: SpvcCompilerOption =
    42 | SPVC_COMPILER_OPTION_MSL_BIT;
pub const SPVC_COMPILER_OPTION_MSL_DYNAMIC_OFFSETS_BUFFER_INDEX: SpvcCompilerOption =
    43 | SPVC_COMPILER_OPTION_MSL_BIT;
pub const SPVC_COMPILER_OPTION_MSL_TEXTURE_1D_AS_2D: SpvcCompilerOption =
    44 | SPVC_COMPILER_OPTION_MSL_BIT;
pub const SPVC_COMPILER_OPTION_MSL_ENABLE_BASE_INDEX_ZERO: SpvcCompilerOption =
    45 | SPVC_COMPILER_OPTION_MSL_BIT;
// Obsolete. Use MSL_FRAMEBUFFER_FETCH_SUBPASS instead.
pub const SPVC_COMPILER_OPTION_MSL_IOS_FRAMEBUFFER_FETCH_SUBPASS: SpvcCompilerOption =
    46 | SPVC_COMPILER_OPTION_MSL_BIT;
pub const SPVC_COMPILER_OPTION_MSL_FRAMEBUFFER_FETCH_SUBPASS: SpvcCompilerOption =
    46 | SPVC_COMPILER_OPTION_MSL_BIT;
pub const SPVC_COMPILER_OPTION_MSL_INVARIANT_FP_MATH: SpvcCompilerOption =
    47 | SPVC_COMPILER_OPTION_MSL_BIT;
pub const SPVC_COMPILER_OPTION_MSL_EMULATE_CUBEMAP_ARRAY: SpvcCompilerOption =
    48 | SPVC_COMPILER_OPTION_MSL_BIT;
pub const SPVC_COMPILER_OPTION_MSL_ENABLE_DECORATION_BINDING: SpvcCompilerOption =
    49 | SPVC_COMPILER_OPTION_MSL_BIT;
pub const SPVC_COMPILER_OPTION_MSL_FORCE_ACTIVE_ARGUMENT_BUFFER_RESOURCES: SpvcCompilerOption =
    50 | SPVC_COMPILER_OPTION_MSL_BIT;
pub const SPVC_COMPILER_OPTION_MSL_FORCE_NATIVE_ARRAYS: SpvcCompilerOption =
    51 | SPVC_COMPILER_OPTION_MSL_BIT;
pub const SPVC_COMPILER_OPTION_ENABLE_STORAGE_IMAGE_QUALIFIER_DEDUCTION: SpvcCompilerOption =
    52 | SPVC_COMPILER_OPTION_COMMON_BIT;
pub const SPVC_COMPILER_OPTION_HLSL_FORCE_STORAGE_BUFFER_AS_UAV: SpvcCompilerOption =
    53 | SPVC_COMPILER_OPTION_HLSL_BIT;
pub const SPVC_COMPILER_OPTION_FORCE_ZERO_INITIALIZED_VARIABLES: SpvcCompilerOption =
    54 | SPVC_COMPILER_OPTION_COMMON_BIT;
pub const SPVC_COMPILER_OPTION_HLSL_NONWRITABLE_UAV_TEXTURE_AS_SRV: SpvcCompilerOption =
    55 | SPVC_COMPILER_OPTION_HLSL_BIT;
pub const SPVC_COMPILER_OPTION_MSL_ENABLE_FRAG_OUTPUT_MASK: SpvcCompilerOption =
    56 | SPVC_COMPILER_OPTION_MSL_BIT;
pub const SPVC_COMPILER_OPTION_MSL_ENABLE_FRAG_DEPTH_BUILTIN: SpvcCompilerOption =
    57 | SPVC_COMPILER_OPTION_MSL_BIT;
pub const SPVC_COMPILER_OPTION_MSL_ENABLE_FRAG_STENCIL_REF_BUILTIN: SpvcCompilerOption =
    58 | SPVC_COMPILER_OPTION_MSL_BIT;
pub const SPVC_COMPILER_OPTION_MSL_ENABLE_CLIP_DISTANCE_USER_VARYING: SpvcCompilerOption =
    59 | SPVC_COMPILER_OPTION_MSL_BIT;
pub const SPVC_COMPILER_OPTION_HLSL_ENABLE_16BIT_TYPES: SpvcCompilerOption =
    60 | SPVC_COMPILER_OPTION_HLSL_BIT;
pub const SPVC_COMPILER_OPTION_MSL_MULTI_PATCH_WORKGROUP: SpvcCompilerOption =
    61 | SPVC_COMPILER_OPTION_MSL_BIT;
pub const SPVC_COMPILER_OPTION_MSL_SHADER_INPUT_BUFFER_INDEX: SpvcCompilerOption =
    62 | SPVC_COMPILER_OPTION_MSL_BIT;
pub const SPVC_COMPILER_OPTION_MSL_SHADER_INDEX_BUFFER_INDEX: SpvcCompilerOption =
    63 | SPVC_COMPILER_OPTION_MSL_BIT;
pub const SPVC_COMPILER_OPTION_MSL_VERTEX_FOR_TESSELLATION: SpvcCompilerOption =
    64 | SPVC_COMPILER_OPTION_MSL_BIT;
pub const SPVC_COMPILER_OPTION_MSL_VERTEX_INDEX_TYPE: SpvcCompilerOption =
    65 | SPVC_COMPILER_OPTION_MSL_BIT;
pub const SPVC_COMPILER_OPTION_GLSL_FORCE_FLATTENED_IO_BLOCKS: SpvcCompilerOption =
    66 | SPVC_COMPILER_OPTION_GLSL_BIT;
pub const SPVC_COMPILER_OPTION_MSL_MULTIVIEW_LAYERED_RENDERING: SpvcCompilerOption =
    67 | SPVC_COMPILER_OPTION_MSL_BIT;
pub const SPVC_COMPILER_OPTION_MSL_ARRAYED_SUBPASS_INPUT: SpvcCompilerOption =
    68 | SPVC_COMPILER_OPTION_MSL_BIT;
pub const SPVC_COMPILER_OPTION_MSL_R32UI_LINEAR_TEXTURE_ALIGNMENT: SpvcCompilerOption =
    69 | SPVC_COMPILER_OPTION_MSL_BIT;
pub const SPVC_COMPILER_OPTION_MSL_R32UI_ALIGNMENT_CONSTANT_ID: SpvcCompilerOption =
    70 | SPVC_COMPILER_OPTION_MSL_BIT;
pub const SPVC_COMPILER_OPTION_HLSL_FLATTEN_MATRIX_VERTEX_INPUT_SEMANTICS: SpvcCompilerOption =
    71 | SPVC_COMPILER_OPTION_HLSL_BIT;
pub const SPVC_COMPILER_OPTION_MSL_IOS_USE_SIMDGROUP_FUNCTIONS: SpvcCompilerOption =
    72 | SPVC_COMPILER_OPTION_MSL_BIT;
pub const SPVC_COMPILER_OPTION_MSL_EMULATE_SUBGROUPS: SpvcCompilerOption =
    73 | SPVC_COMPILER_OPTION_MSL_BIT;
pub const SPVC_COMPILER_OPTION_MSL_FIXED_SUBGROUP_SIZE: SpvcCompilerOption =
    74 | SPVC_COMPILER_OPTION_MSL_BIT;
pub const SPVC_COMPILER_OPTION_MSL_FORCE_SAMPLE_RATE_SHADING: SpvcCompilerOption =
    75 | SPVC_COMPILER_OPTION_MSL_BIT;
pub const SPVC_COMPILER_OPTION_MSL_IOS_SUPPORT_BASE_VERTEX_INSTANCE: SpvcCompilerOption =
    76 | SPVC_COMPILER_OPTION_MSL_BIT;
pub const SPVC_COMPILER_OPTION_GLSL_OVR_MULTIVIEW_VIEW_COUNT: SpvcCompilerOption =
    77 | SPVC_COMPILER_OPTION_GLSL_BIT;
pub const SPVC_COMPILER_OPTION_RELAX_NAN_CHECKS: SpvcCompilerOption =
    78 | SPVC_COMPILER_OPTION_COMMON_BIT;
pub const SPVC_COMPILER_OPTION_MSL_RAW_BUFFER_TESE_INPUT: SpvcCompilerOption =
    79 | SPVC_COMPILER_OPTION_MSL_BIT;
pub const SPVC_COMPILER_OPTION_MSL_SHADER_PATCH_INPUT_BUFFER_INDEX: SpvcCompilerOption =
    80 | SPVC_COMPILER_OPTION_MSL_BIT;
pub const SPVC_COMPILER_OPTION_MSL_MANUAL_HELPER_INVOCATION_UPDATES: SpvcCompilerOption =
    81 | SPVC_COMPILER_OPTION_MSL_BIT;
pub const SPVC_COMPILER_OPTION_MSL_CHECK_DISCARDED_FRAG_STORES: SpvcCompilerOption =
    82 | SPVC_COMPILER_OPTION_MSL_BIT;
pub const SPVC_COMPILER_OPTION_GLSL_ENABLE_ROW_MAJOR_LOAD_WORKAROUND: SpvcCompilerOption =
    83 | SPVC_COMPILER_OPTION_GLSL_BIT;
pub const SPVC_COMPILER_OPTION_MSL_ARGUMENT_BUFFERS_TIER: SpvcCompilerOption =
    84 | SPVC_COMPILER_OPTION_MSL_BIT;
pub const SPVC_COMPILER_OPTION_MSL_SAMPLE_DREF_LOD_ARRAY_AS_GRAD: SpvcCompilerOption =
    85 | SPVC_COMPILER_OPTION_MSL_BIT;
pub const SPVC_COMPILER_OPTION_MSL_READWRITE_TEXTURE_FENCES: SpvcCompilerOption =
    86 | SPVC_COMPILER_OPTION_MSL_BIT;
pub const SPVC_COMPILER_OPTION_MSL_REPLACE_RECURSIVE_INPUTS: SpvcCompilerOption =
    87 | SPVC_COMPILER_OPTION_MSL_BIT;
pub const SPVC_COMPILER_OPTION_MSL_AGX_MANUAL_CUBE_GRAD_FIXUP: SpvcCompilerOption =
    88 | SPVC_COMPILER_OPTION_MSL_BIT;
pub const SPVC_COMPILER_OPTION_MSL_FORCE_FRAGMENT_WITH_SIDE_EFFECTS_EXECUTION: SpvcCompilerOption =
    89 | SPVC_COMPILER_OPTION_MSL_BIT;
pub const SPVC_COMPILER_OPTION_HLSL_USE_ENTRY_POINT_NAME: SpvcCompilerOption =
    90 | SPVC_COMPILER_OPTION_HLSL_BIT;
pub const SPVC_COMPILER_OPTION_HLSL_PRESERVE_STRUCTURED_BUFFERS: SpvcCompilerOption =
    91 | SPVC_COMPILER_OPTION_HLSL_BIT;
pub const SPVC_COMPILER_OPTION_MSL_AUTO_DISABLE_RASTERIZATION: SpvcCompilerOption =
    92 | SPVC_COMPILER_OPTION_MSL_BIT;
pub const SPVC_COMPILER_OPTION_MSL_ENABLE_POINT_SIZE_DEFAULT: SpvcCompilerOption =
    93 | SPVC_COMPILER_OPTION_MSL_BIT;
pub const SPVC_COMPILER_OPTION_HLSL_USER_SEMANTIC: SpvcCompilerOption =
    94 | SPVC_COMPILER_OPTION_HLSL_BIT;

/// `spvc_reflected_resource`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SpvcReflectedResource {
    pub id: u32,
    pub base_type_id: u32,
    pub type_id: u32,
    pub name: String,
}

/// `spvc_msl_resource_binding_2`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SpvcMslResourceBinding2 {
    pub stage: ExecutionModel,
    pub desc_set: u32,
    pub binding: u32,
    pub count: u32,
    pub msl_buffer: u32,
    pub msl_texture: u32,
    pub msl_sampler: u32,
}

/// `struct spvc_context_s`.
#[derive(Debug, Default)]
pub struct SpvcContext {
    last_error: String,
}

/// `struct spvc_compiler_s`.
#[derive(Debug)]
pub struct SpvcCompiler {
    pub compiler: Compiler,
    pub backend: SpvcBackend,
}

/// `struct spvc_compiler_options_s`.
#[derive(Clone, Debug)]
pub struct SpvcCompilerOptions {
    backend_flags: u32,
    pub glsl: GlslOptions,
    pub hlsl: HlslOptions,
    pub msl: MslOptions,
}

/// `struct spvc_set_s`.
#[derive(Clone, Debug, Default)]
pub struct SpvcSet {
    pub set: HashSet<VariableID>,
}

/// `struct spvc_resources_s`.
#[derive(Clone, Debug, Default)]
pub struct SpvcResources {
    uniform_buffers: Vec<SpvcReflectedResource>,
    storage_buffers: Vec<SpvcReflectedResource>,
    stage_inputs: Vec<SpvcReflectedResource>,
    stage_outputs: Vec<SpvcReflectedResource>,
    subpass_inputs: Vec<SpvcReflectedResource>,
    storage_images: Vec<SpvcReflectedResource>,
    sampled_images: Vec<SpvcReflectedResource>,
    atomic_counters: Vec<SpvcReflectedResource>,
    push_constant_buffers: Vec<SpvcReflectedResource>,
    shader_record_buffers: Vec<SpvcReflectedResource>,
    separate_images: Vec<SpvcReflectedResource>,
    separate_samplers: Vec<SpvcReflectedResource>,
    acceleration_structures: Vec<SpvcReflectedResource>,
    gl_plain_uniforms: Vec<SpvcReflectedResource>,
    tensors: Vec<SpvcReflectedResource>,
}

impl SpvcResources {
    fn copy_resources(outputs: &mut Vec<SpvcReflectedResource>, inputs: &[Resource]) {
        for i in inputs {
            outputs.push(SpvcReflectedResource {
                base_type_id: i.base_type_id,
                type_id: i.type_id,
                id: i.id,
                name: i.name.clone(),
            });
        }
    }

    fn copy_all(&mut self, resources: &ShaderResources) {
        Self::copy_resources(&mut self.uniform_buffers, &resources.uniform_buffers);
        Self::copy_resources(&mut self.storage_buffers, &resources.storage_buffers);
        Self::copy_resources(&mut self.stage_inputs, &resources.stage_inputs);
        Self::copy_resources(&mut self.stage_outputs, &resources.stage_outputs);
        Self::copy_resources(&mut self.subpass_inputs, &resources.subpass_inputs);
        Self::copy_resources(&mut self.storage_images, &resources.storage_images);
        Self::copy_resources(&mut self.sampled_images, &resources.sampled_images);
        Self::copy_resources(&mut self.atomic_counters, &resources.atomic_counters);
        Self::copy_resources(
            &mut self.push_constant_buffers,
            &resources.push_constant_buffers,
        );
        Self::copy_resources(
            &mut self.shader_record_buffers,
            &resources.shader_record_buffers,
        );
        Self::copy_resources(&mut self.separate_images, &resources.separate_images);
        Self::copy_resources(&mut self.separate_samplers, &resources.separate_samplers);
        Self::copy_resources(
            &mut self.acceleration_structures,
            &resources.acceleration_structures,
        );
        Self::copy_resources(&mut self.gl_plain_uniforms, &resources.gl_plain_uniforms);
        Self::copy_resources(&mut self.tensors, &resources.tensors);
    }
}

impl SpvcContext {
    /// `spvc_context_create()`.
    pub fn create() -> SpvcContext {
        SpvcContext::default()
    }

    fn report_error(&mut self, msg: impl Into<String>) {
        self.last_error = msg.into();
    }

    /// `spvc_context_get_last_error_string()`.
    pub fn get_last_error_string(&self) -> &str {
        &self.last_error
    }

    /// `spvc_context_parse_spirv()`.
    pub fn parse_spirv(&mut self, spirv: &[u32]) -> std::result::Result<ParsedIR, SpvcResult> {
        let mut words = Vec::new();
        if words.try_reserve_exact(spirv.len()).is_err() {
            self.report_error("Out of memory.");
            return Err(SPVC_ERROR_OUT_OF_MEMORY);
        }
        words.extend_from_slice(spirv);
        let mut parser = Parser::new(words);
        match parser.parse() {
            Ok(()) => Ok(parser.into_parsed_ir()),
            Err(e) => {
                self.report_error(e.to_string());
                Err(SPVC_ERROR_INVALID_SPIRV)
            }
        }
    }

    /// `spvc_context_create_compiler()` (with `SPVC_CAPTURE_MODE_TAKE_OWNERSHIP`).
    pub fn create_compiler(
        &mut self,
        backend: SpvcBackend,
        parsed_ir: ParsedIR,
    ) -> std::result::Result<SpvcCompiler, SpvcResult> {
        let b = match backend {
            SpvcBackend::None => Backend::None,
            SpvcBackend::Glsl => Backend::Glsl,
            SpvcBackend::Hlsl => Backend::Hlsl,
            SpvcBackend::Msl => Backend::Msl,
        };
        match Compiler::with_backend(parsed_ir, b) {
            Ok(compiler) => Ok(SpvcCompiler { compiler, backend }),
            Err(e) => {
                self.report_error(e.to_string());
                Err(SPVC_ERROR_OUT_OF_MEMORY)
            }
        }
    }
}

impl SpvcCompiler {
    /// `spvc_compiler_create_compiler_options()`.
    pub fn create_compiler_options(&self) -> SpvcCompilerOptions {
        let mut opt = SpvcCompilerOptions {
            backend_flags: 0,
            glsl: GlslOptions::default(),
            hlsl: HlslOptions::default(),
            msl: MslOptions::default(),
        };
        match self.backend {
            SpvcBackend::Msl => {
                opt.backend_flags |= SPVC_COMPILER_OPTION_MSL_BIT | SPVC_COMPILER_OPTION_COMMON_BIT;
                opt.glsl = *self.compiler.get_common_options();
                opt.msl = *self.compiler.get_msl_options();
            }
            SpvcBackend::Hlsl => {
                opt.backend_flags |=
                    SPVC_COMPILER_OPTION_HLSL_BIT | SPVC_COMPILER_OPTION_COMMON_BIT;
                opt.glsl = *self.compiler.get_common_options();
                opt.hlsl = *self.compiler.get_hlsl_options();
            }
            SpvcBackend::Glsl => {
                opt.backend_flags |=
                    SPVC_COMPILER_OPTION_GLSL_BIT | SPVC_COMPILER_OPTION_COMMON_BIT;
                opt.glsl = *self.compiler.get_common_options();
            }
            SpvcBackend::None => {}
        }
        opt
    }

    /// `spvc_compiler_install_compiler_options()`.
    pub fn install_compiler_options(&mut self, options: &SpvcCompilerOptions) -> SpvcResult {
        match self.backend {
            SpvcBackend::Glsl => self.compiler.set_common_options(&options.glsl),
            SpvcBackend::Hlsl => {
                self.compiler.set_common_options(&options.glsl);
                self.compiler.set_hlsl_options(&options.hlsl);
            }
            SpvcBackend::Msl => {
                self.compiler.set_common_options(&options.glsl);
                self.compiler.set_msl_options(&options.msl);
            }
            SpvcBackend::None => {}
        }
        SPVC_SUCCESS
    }

    /// `spvc_compiler_compile()`.
    pub fn compile(
        &mut self,
        context: &mut SpvcContext,
    ) -> std::result::Result<String, SpvcResult> {
        match self.compiler.compile() {
            Ok(result) => {
                if result.is_empty() {
                    context.report_error("Unsupported SPIR-V.");
                    return Err(SPVC_ERROR_UNSUPPORTED_SPIRV);
                }
                Ok(result)
            }
            Err(e) => {
                context.report_error(e.to_string());
                Err(SPVC_ERROR_UNSUPPORTED_SPIRV)
            }
        }
    }

    /// `spvc_compiler_get_active_interface_variables()`.
    pub fn get_active_interface_variables(
        &mut self,
        context: &mut SpvcContext,
    ) -> std::result::Result<SpvcSet, SpvcResult> {
        match self.compiler.get_active_interface_variables() {
            Ok(set) => Ok(SpvcSet { set }),
            Err(e) => {
                context.report_error(e.to_string());
                Err(SPVC_ERROR_INVALID_ARGUMENT)
            }
        }
    }

    /// `spvc_compiler_set_enabled_interface_variables()`.
    pub fn set_enabled_interface_variables(&mut self, set: &SpvcSet) -> SpvcResult {
        self.compiler
            .set_enabled_interface_variables(set.set.clone());
        SPVC_SUCCESS
    }

    /// `spvc_compiler_create_shader_resources_for_active_variables()`.
    pub fn create_shader_resources_for_active_variables(
        &mut self,
        context: &mut SpvcContext,
        set: &SpvcSet,
    ) -> std::result::Result<SpvcResources, SpvcResult> {
        match self.compiler.get_shader_resources_for(&set.set) {
            Ok(accessed_resources) => {
                let mut res = SpvcResources::default();
                res.copy_all(&accessed_resources);
                Ok(res)
            }
            Err(e) => {
                context.report_error(e.to_string());
                Err(SPVC_ERROR_OUT_OF_MEMORY)
            }
        }
    }

    /// `spvc_compiler_create_shader_resources()`.
    pub fn create_shader_resources(
        &mut self,
        context: &mut SpvcContext,
    ) -> std::result::Result<SpvcResources, SpvcResult> {
        match self.compiler.get_shader_resources() {
            Ok(accessed_resources) => {
                let mut res = SpvcResources::default();
                res.copy_all(&accessed_resources);
                Ok(res)
            }
            Err(e) => {
                context.report_error(e.to_string());
                Err(SPVC_ERROR_OUT_OF_MEMORY)
            }
        }
    }

    /// `spvc_compiler_msl_add_resource_binding_2()`.
    pub fn msl_add_resource_binding_2(
        &mut self,
        context: &mut SpvcContext,
        binding: &SpvcMslResourceBinding2,
    ) -> SpvcResult {
        if self.backend != SpvcBackend::Msl {
            context.report_error("MSL function used on a non-MSL backend.");
            return SPVC_ERROR_INVALID_ARGUMENT;
        }

        let bind = MSLResourceBinding {
            binding: binding.binding,
            desc_set: binding.desc_set,
            stage: binding.stage,
            msl_buffer: binding.msl_buffer,
            msl_texture: binding.msl_texture,
            msl_sampler: binding.msl_sampler,
            count: binding.count,
            ..Default::default()
        };
        // (C++'s add_msl_resource_binding() can't fail; SPVC_BEGIN_SAFE_SCOPE
        // isn't used here either.)
        match self.compiler.add_msl_resource_binding(&bind) {
            Ok(()) => SPVC_SUCCESS,
            Err(e) => {
                context.report_error(e.to_string());
                SPVC_ERROR_INVALID_ARGUMENT
            }
        }
    }

    /// `spvc_compiler_has_decoration()`.
    pub fn has_decoration(&self, id: u32, decoration: Decoration) -> bool {
        self.compiler.has_decoration(id, decoration)
    }

    /// `spvc_compiler_get_decoration()`.
    pub fn get_decoration(&self, id: u32, decoration: Decoration) -> u32 {
        self.compiler.get_decoration(id, decoration)
    }

    /// `spvc_compiler_get_execution_mode_argument_by_index()`.
    ///
    /// (C++ doesn't catch the exceptions this can throw on malformed
    /// SPIR-V; here they read as 0.)
    pub fn get_execution_mode_argument_by_index(&self, mode: ExecutionMode, index: u32) -> u32 {
        self.compiler
            .get_execution_mode_argument(mode, index)
            .unwrap_or(0)
    }

    /// `spvc_compiler_get_execution_model()`.
    pub fn get_execution_model(&self) -> ExecutionModel {
        self.compiler.get_execution_model()
    }

    /// `spvc_compiler_get_cleansed_entry_point_name()`.
    pub fn get_cleansed_entry_point_name(
        &self,
        context: &mut SpvcContext,
        name: &str,
        model: ExecutionModel,
    ) -> Option<String> {
        match self.compiler.get_cleansed_entry_point_name(name, model) {
            Ok(n) => Some(n.clone()),
            Err(e) => {
                context.report_error(e.to_string());
                None
            }
        }
    }

    /// `spvc_compiler_get_type_handle()`: `None` where C++ returns null.
    pub fn get_type_handle(&self, context: &mut SpvcContext, id: u32) -> Option<&SPIRType> {
        match self.compiler.get_type(id) {
            Ok(t) => Some(t),
            Err(e) => {
                context.report_error(e.to_string());
                None
            }
        }
    }
}

impl SpvcCompilerOptions {
    /// `spvc_compiler_options_set_bool()`.
    pub fn set_bool(
        &mut self,
        context: &mut SpvcContext,
        option: SpvcCompilerOption,
        value: bool,
    ) -> SpvcResult {
        self.set_uint(context, option, if value { 1 } else { 0 })
    }

    /// `spvc_compiler_options_set_uint()`.
    pub fn set_uint(
        &mut self,
        context: &mut SpvcContext,
        option: SpvcCompilerOption,
        value: u32,
    ) -> SpvcResult {
        let supported_mask = self.backend_flags;
        let required_mask = option & SPVC_COMPILER_OPTION_LANG_BITS;
        if (required_mask | supported_mask) != supported_mask {
            context.report_error("Option is not supported by current backend.");
            return SPVC_ERROR_INVALID_ARGUMENT;
        }

        match option {
            SPVC_COMPILER_OPTION_FORCE_TEMPORARY => self.glsl.force_temporary = value != 0,
            SPVC_COMPILER_OPTION_FLATTEN_MULTIDIMENSIONAL_ARRAYS => {
                self.glsl.flatten_multidimensional_arrays = value != 0
            }
            SPVC_COMPILER_OPTION_FIXUP_DEPTH_CONVENTION => {
                self.glsl.vertex.fixup_clipspace = value != 0
            }
            SPVC_COMPILER_OPTION_FLIP_VERTEX_Y => self.glsl.vertex.flip_vert_y = value != 0,
            SPVC_COMPILER_OPTION_EMIT_LINE_DIRECTIVES => {
                self.glsl.emit_line_directives = value != 0
            }
            SPVC_COMPILER_OPTION_ENABLE_STORAGE_IMAGE_QUALIFIER_DEDUCTION => {
                self.glsl.enable_storage_image_qualifier_deduction = value != 0
            }
            SPVC_COMPILER_OPTION_FORCE_ZERO_INITIALIZED_VARIABLES => {
                self.glsl.force_zero_initialized_variables = value != 0
            }

            SPVC_COMPILER_OPTION_GLSL_SUPPORT_NONZERO_BASE_INSTANCE => {
                self.glsl.vertex.support_nonzero_base_instance = value != 0
            }
            SPVC_COMPILER_OPTION_GLSL_SEPARATE_SHADER_OBJECTS => {
                self.glsl.separate_shader_objects = value != 0
            }
            SPVC_COMPILER_OPTION_GLSL_ENABLE_420PACK_EXTENSION => {
                self.glsl.enable_420pack_extension = value != 0
            }
            SPVC_COMPILER_OPTION_GLSL_VERSION => self.glsl.version = value,
            SPVC_COMPILER_OPTION_GLSL_ES => self.glsl.es = value != 0,
            SPVC_COMPILER_OPTION_GLSL_VULKAN_SEMANTICS => self.glsl.vulkan_semantics = value != 0,
            SPVC_COMPILER_OPTION_GLSL_ES_DEFAULT_FLOAT_PRECISION_HIGHP => {
                self.glsl.fragment.default_float_precision = if value != 0 {
                    Precision::Highp
                } else {
                    Precision::Mediump
                }
            }
            SPVC_COMPILER_OPTION_GLSL_ES_DEFAULT_INT_PRECISION_HIGHP => {
                self.glsl.fragment.default_int_precision = if value != 0 {
                    Precision::Highp
                } else {
                    Precision::Mediump
                }
            }
            SPVC_COMPILER_OPTION_GLSL_EMIT_PUSH_CONSTANT_AS_UNIFORM_BUFFER => {
                self.glsl.emit_push_constant_as_uniform_buffer = value != 0
            }
            SPVC_COMPILER_OPTION_GLSL_EMIT_UNIFORM_BUFFER_AS_PLAIN_UNIFORMS => {
                self.glsl.emit_uniform_buffer_as_plain_uniforms = value != 0
            }
            SPVC_COMPILER_OPTION_GLSL_FORCE_FLATTENED_IO_BLOCKS => {
                self.glsl.force_flattened_io_blocks = value != 0
            }
            SPVC_COMPILER_OPTION_GLSL_OVR_MULTIVIEW_VIEW_COUNT => {
                self.glsl.ovr_multiview_view_count = value
            }
            SPVC_COMPILER_OPTION_RELAX_NAN_CHECKS => self.glsl.relax_nan_checks = value != 0,
            SPVC_COMPILER_OPTION_GLSL_ENABLE_ROW_MAJOR_LOAD_WORKAROUND => {
                self.glsl.enable_row_major_load_workaround = value != 0
            }

            SPVC_COMPILER_OPTION_HLSL_SHADER_MODEL => self.hlsl.shader_model = value,
            SPVC_COMPILER_OPTION_HLSL_POINT_SIZE_COMPAT => self.hlsl.point_size_compat = value != 0,
            SPVC_COMPILER_OPTION_HLSL_POINT_COORD_COMPAT => {
                self.hlsl.point_coord_compat = value != 0
            }
            SPVC_COMPILER_OPTION_HLSL_SUPPORT_NONZERO_BASE_VERTEX_BASE_INSTANCE => {
                self.hlsl.support_nonzero_base_vertex_base_instance = value != 0
            }
            SPVC_COMPILER_OPTION_HLSL_FORCE_STORAGE_BUFFER_AS_UAV => {
                self.hlsl.force_storage_buffer_as_uav = value != 0
            }
            SPVC_COMPILER_OPTION_HLSL_NONWRITABLE_UAV_TEXTURE_AS_SRV => {
                self.hlsl.nonwritable_uav_texture_as_srv = value != 0
            }
            SPVC_COMPILER_OPTION_HLSL_ENABLE_16BIT_TYPES => {
                self.hlsl.enable_16bit_types = value != 0
            }
            SPVC_COMPILER_OPTION_HLSL_FLATTEN_MATRIX_VERTEX_INPUT_SEMANTICS => {
                self.hlsl.flatten_matrix_vertex_input_semantics = value != 0
            }
            SPVC_COMPILER_OPTION_HLSL_USE_ENTRY_POINT_NAME => {
                self.hlsl.use_entry_point_name = value != 0
            }
            SPVC_COMPILER_OPTION_HLSL_PRESERVE_STRUCTURED_BUFFERS => {
                self.hlsl.preserve_structured_buffers = value != 0
            }
            SPVC_COMPILER_OPTION_HLSL_USER_SEMANTIC => self.hlsl.user_semantic = value != 0,

            SPVC_COMPILER_OPTION_MSL_VERSION => self.msl.msl_version = value,
            SPVC_COMPILER_OPTION_MSL_TEXEL_BUFFER_TEXTURE_WIDTH => {
                self.msl.texel_buffer_texture_width = value
            }
            SPVC_COMPILER_OPTION_MSL_SWIZZLE_BUFFER_INDEX => self.msl.swizzle_buffer_index = value,
            SPVC_COMPILER_OPTION_MSL_INDIRECT_PARAMS_BUFFER_INDEX => {
                self.msl.indirect_params_buffer_index = value
            }
            SPVC_COMPILER_OPTION_MSL_SHADER_OUTPUT_BUFFER_INDEX => {
                self.msl.shader_output_buffer_index = value
            }
            SPVC_COMPILER_OPTION_MSL_SHADER_PATCH_OUTPUT_BUFFER_INDEX => {
                self.msl.shader_patch_output_buffer_index = value
            }
            SPVC_COMPILER_OPTION_MSL_SHADER_TESS_FACTOR_OUTPUT_BUFFER_INDEX => {
                self.msl.shader_tess_factor_buffer_index = value
            }
            SPVC_COMPILER_OPTION_MSL_SHADER_INPUT_WORKGROUP_INDEX => {
                self.msl.shader_input_wg_index = value
            }
            SPVC_COMPILER_OPTION_MSL_ENABLE_POINT_SIZE_BUILTIN => {
                self.msl.enable_point_size_builtin = value != 0
            }
            SPVC_COMPILER_OPTION_MSL_ENABLE_POINT_SIZE_DEFAULT => {
                self.msl.enable_point_size_default = value != 0
            }
            SPVC_COMPILER_OPTION_MSL_DISABLE_RASTERIZATION => {
                self.msl.disable_rasterization = value != 0
            }
            SPVC_COMPILER_OPTION_MSL_AUTO_DISABLE_RASTERIZATION => {
                self.msl.auto_disable_rasterization = value != 0
            }
            SPVC_COMPILER_OPTION_MSL_CAPTURE_OUTPUT_TO_BUFFER => {
                self.msl.capture_output_to_buffer = value != 0
            }
            SPVC_COMPILER_OPTION_MSL_SWIZZLE_TEXTURE_SAMPLES => {
                self.msl.swizzle_texture_samples = value != 0
            }
            SPVC_COMPILER_OPTION_MSL_PAD_FRAGMENT_OUTPUT_COMPONENTS => {
                self.msl.pad_fragment_output_components = value != 0
            }
            SPVC_COMPILER_OPTION_MSL_TESS_DOMAIN_ORIGIN_LOWER_LEFT => {
                self.msl.tess_domain_origin_lower_left = value != 0
            }
            SPVC_COMPILER_OPTION_MSL_PLATFORM => {
                self.msl.platform = if value == 0 {
                    MslPlatform::IOS
                } else {
                    MslPlatform::MacOS
                }
            }
            SPVC_COMPILER_OPTION_MSL_ARGUMENT_BUFFERS => self.msl.argument_buffers = value != 0,
            SPVC_COMPILER_OPTION_MSL_TEXTURE_BUFFER_NATIVE => {
                self.msl.texture_buffer_native = value != 0
            }
            SPVC_COMPILER_OPTION_MSL_BUFFER_SIZE_BUFFER_INDEX => {
                self.msl.buffer_size_buffer_index = value
            }
            SPVC_COMPILER_OPTION_MSL_MULTIVIEW => self.msl.multiview = value != 0,
            SPVC_COMPILER_OPTION_MSL_VIEW_MASK_BUFFER_INDEX => {
                self.msl.view_mask_buffer_index = value
            }
            SPVC_COMPILER_OPTION_MSL_DEVICE_INDEX => self.msl.device_index = value,
            SPVC_COMPILER_OPTION_MSL_VIEW_INDEX_FROM_DEVICE_INDEX => {
                self.msl.view_index_from_device_index = value != 0
            }
            SPVC_COMPILER_OPTION_MSL_DISPATCH_BASE => self.msl.dispatch_base = value != 0,
            SPVC_COMPILER_OPTION_MSL_DYNAMIC_OFFSETS_BUFFER_INDEX => {
                self.msl.dynamic_offsets_buffer_index = value
            }
            SPVC_COMPILER_OPTION_MSL_TEXTURE_1D_AS_2D => self.msl.texture_1D_as_2D = value != 0,
            SPVC_COMPILER_OPTION_MSL_ENABLE_BASE_INDEX_ZERO => {
                self.msl.enable_base_index_zero = value != 0
            }
            SPVC_COMPILER_OPTION_MSL_FRAMEBUFFER_FETCH_SUBPASS => {
                self.msl.use_framebuffer_fetch_subpasses = value != 0
            }
            SPVC_COMPILER_OPTION_MSL_INVARIANT_FP_MATH => {
                self.msl.invariant_float_math = value != 0
            }
            SPVC_COMPILER_OPTION_MSL_EMULATE_CUBEMAP_ARRAY => {
                self.msl.emulate_cube_array = value != 0
            }
            SPVC_COMPILER_OPTION_MSL_ENABLE_DECORATION_BINDING => {
                self.msl.enable_decoration_binding = value != 0
            }
            SPVC_COMPILER_OPTION_MSL_FORCE_ACTIVE_ARGUMENT_BUFFER_RESOURCES => {
                self.msl.force_active_argument_buffer_resources = value != 0
            }
            SPVC_COMPILER_OPTION_MSL_FORCE_NATIVE_ARRAYS => {
                self.msl.force_native_arrays = value != 0
            }
            SPVC_COMPILER_OPTION_MSL_ENABLE_FRAG_OUTPUT_MASK => {
                self.msl.enable_frag_output_mask = value
            }
            SPVC_COMPILER_OPTION_MSL_ENABLE_FRAG_DEPTH_BUILTIN => {
                self.msl.enable_frag_depth_builtin = value != 0
            }
            SPVC_COMPILER_OPTION_MSL_ENABLE_FRAG_STENCIL_REF_BUILTIN => {
                self.msl.enable_frag_stencil_ref_builtin = value != 0
            }
            SPVC_COMPILER_OPTION_MSL_ENABLE_CLIP_DISTANCE_USER_VARYING => {
                self.msl.enable_clip_distance_user_varying = value != 0
            }
            SPVC_COMPILER_OPTION_MSL_MULTI_PATCH_WORKGROUP => {
                self.msl.multi_patch_workgroup = value != 0
            }
            SPVC_COMPILER_OPTION_MSL_SHADER_INPUT_BUFFER_INDEX => {
                self.msl.shader_input_buffer_index = value
            }
            SPVC_COMPILER_OPTION_MSL_SHADER_INDEX_BUFFER_INDEX => {
                self.msl.shader_index_buffer_index = value
            }
            SPVC_COMPILER_OPTION_MSL_VERTEX_FOR_TESSELLATION => {
                self.msl.vertex_for_tessellation = value != 0
            }
            SPVC_COMPILER_OPTION_MSL_VERTEX_INDEX_TYPE => {
                self.msl.vertex_index_type = match value {
                    0 => IndexType::None,
                    1 => IndexType::UInt16,
                    _ => IndexType::UInt32,
                }
            }
            SPVC_COMPILER_OPTION_MSL_MULTIVIEW_LAYERED_RENDERING => {
                self.msl.multiview_layered_rendering = value != 0
            }
            SPVC_COMPILER_OPTION_MSL_ARRAYED_SUBPASS_INPUT => {
                self.msl.arrayed_subpass_input = value != 0
            }
            SPVC_COMPILER_OPTION_MSL_R32UI_LINEAR_TEXTURE_ALIGNMENT => {
                self.msl.r32ui_linear_texture_alignment = value
            }
            SPVC_COMPILER_OPTION_MSL_R32UI_ALIGNMENT_CONSTANT_ID => {
                self.msl.r32ui_alignment_constant_id = value
            }
            SPVC_COMPILER_OPTION_MSL_IOS_USE_SIMDGROUP_FUNCTIONS => {
                self.msl.ios_use_simdgroup_functions = value != 0
            }
            SPVC_COMPILER_OPTION_MSL_EMULATE_SUBGROUPS => self.msl.emulate_subgroups = value != 0,
            SPVC_COMPILER_OPTION_MSL_FIXED_SUBGROUP_SIZE => self.msl.fixed_subgroup_size = value,
            SPVC_COMPILER_OPTION_MSL_FORCE_SAMPLE_RATE_SHADING => {
                self.msl.force_sample_rate_shading = value != 0
            }
            SPVC_COMPILER_OPTION_MSL_IOS_SUPPORT_BASE_VERTEX_INSTANCE => {
                self.msl.ios_support_base_vertex_instance = value != 0
            }
            SPVC_COMPILER_OPTION_MSL_RAW_BUFFER_TESE_INPUT => {
                self.msl.raw_buffer_tese_input = value != 0
            }
            SPVC_COMPILER_OPTION_MSL_SHADER_PATCH_INPUT_BUFFER_INDEX => {
                self.msl.shader_patch_input_buffer_index = value
            }
            SPVC_COMPILER_OPTION_MSL_MANUAL_HELPER_INVOCATION_UPDATES => {
                self.msl.manual_helper_invocation_updates = value != 0
            }
            SPVC_COMPILER_OPTION_MSL_CHECK_DISCARDED_FRAG_STORES => {
                self.msl.check_discarded_frag_stores = value != 0
            }
            SPVC_COMPILER_OPTION_MSL_ARGUMENT_BUFFERS_TIER => {
                self.msl.argument_buffers_tier = if value == 0 {
                    ArgumentBuffersTier::Tier1
                } else {
                    ArgumentBuffersTier::Tier2
                }
            }
            SPVC_COMPILER_OPTION_MSL_SAMPLE_DREF_LOD_ARRAY_AS_GRAD => {
                self.msl.sample_dref_lod_array_as_grad = value != 0
            }
            SPVC_COMPILER_OPTION_MSL_READWRITE_TEXTURE_FENCES => {
                self.msl.readwrite_texture_fences = value != 0
            }
            SPVC_COMPILER_OPTION_MSL_REPLACE_RECURSIVE_INPUTS => {
                self.msl.replace_recursive_inputs = value != 0
            }
            SPVC_COMPILER_OPTION_MSL_AGX_MANUAL_CUBE_GRAD_FIXUP => {
                self.msl.agx_manual_cube_grad_fixup = value != 0
            }
            SPVC_COMPILER_OPTION_MSL_FORCE_FRAGMENT_WITH_SIDE_EFFECTS_EXECUTION => {
                self.msl.force_fragment_with_side_effects_execution = value != 0
            }

            _ => {
                context.report_error("Unknown option.");
                return SPVC_ERROR_INVALID_ARGUMENT;
            }
        }

        SPVC_SUCCESS
    }
}

impl SpvcResources {
    /// `spvc_resources_get_resource_list_for_type()`.
    pub fn get_resource_list_for_type(
        &self,
        context: &mut SpvcContext,
        type_: SpvcResourceType,
    ) -> std::result::Result<&[SpvcReflectedResource], SpvcResult> {
        let list = match type_ {
            SPVC_RESOURCE_TYPE_UNIFORM_BUFFER => &self.uniform_buffers,
            SPVC_RESOURCE_TYPE_STORAGE_BUFFER => &self.storage_buffers,
            SPVC_RESOURCE_TYPE_STAGE_INPUT => &self.stage_inputs,
            SPVC_RESOURCE_TYPE_STAGE_OUTPUT => &self.stage_outputs,
            SPVC_RESOURCE_TYPE_SUBPASS_INPUT => &self.subpass_inputs,
            SPVC_RESOURCE_TYPE_STORAGE_IMAGE => &self.storage_images,
            SPVC_RESOURCE_TYPE_SAMPLED_IMAGE => &self.sampled_images,
            SPVC_RESOURCE_TYPE_ATOMIC_COUNTER => &self.atomic_counters,
            SPVC_RESOURCE_TYPE_PUSH_CONSTANT => &self.push_constant_buffers,
            SPVC_RESOURCE_TYPE_SEPARATE_IMAGE => &self.separate_images,
            SPVC_RESOURCE_TYPE_SEPARATE_SAMPLERS => &self.separate_samplers,
            SPVC_RESOURCE_TYPE_ACCELERATION_STRUCTURE => &self.acceleration_structures,
            SPVC_RESOURCE_TYPE_SHADER_RECORD_BUFFER => &self.shader_record_buffers,
            SPVC_RESOURCE_TYPE_GL_PLAIN_UNIFORM => &self.gl_plain_uniforms,
            SPVC_RESOURCE_TYPE_TENSOR => &self.tensors,
            _ => {
                context.report_error("Invalid argument.");
                return Err(SPVC_ERROR_INVALID_ARGUMENT);
            }
        };
        Ok(list)
    }
}

/// `spvc_type_get_basetype()`.
pub fn spvc_type_get_basetype(type_: &SPIRType) -> SpvcBasetype {
    // For now the enums match up.
    type_.basetype.to_u32()
}

/// `spvc_type_get_vector_size()`.
pub fn spvc_type_get_vector_size(type_: &SPIRType) -> u32 {
    type_.vecsize
}
