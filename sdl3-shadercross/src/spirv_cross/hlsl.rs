// Rust translation of spirv_hlsl.hpp and spirv_hlsl.cpp from SPIRV-Cross.
// Copyright 2016-2021 Robert Konrad
// SPDX-License-Identifier: Apache-2.0 OR MIT
// This is an altered (translated to Rust) version of the original
// software; see LICENSE.txt.

//! `CompilerHLSL`: decompiles SPIR-V to HLSL.
//!
//! CompilerHLSL's overrides of CompilerGLSL's virtual methods are the
//! `hlsl_*` targets of the dispatchers in the GLSL backend; its own
//! methods that share a name with one of CompilerGLSL's (which C++ hides)
//! are prefixed `hlsl_` as well.

use std::collections::{HashMap, HashSet};

use super::common::*;
use super::cross::*;
use super::glsl::*;
use super::parsed_ir::*;
use super::spirv::*;

use BufferPackingStandard::*;

// Interface which remaps vertex inputs to a fixed semantic name to make linking easier.
/// `HLSLVertexAttributeRemap`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct HLSLVertexAttributeRemap {
    pub location: u32,
    pub semantic: String,
}

// Specifying a root constant (d3d12) or push constant range (vulkan).
//
// `start` and `end` denotes the range of the root constant in bytes.
// Both values need to be multiple of 4.
/// `RootConstants`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct RootConstants {
    pub start: u32,
    pub end: u32,

    pub binding: u32,
    pub space: u32,
}

// For finer control, decorations may be removed from specific resources instead with unset_decoration().
/// `HLSLBindingFlagBits`.
pub type HLSLBindingFlagBits = u32;
pub const HLSL_BINDING_AUTO_NONE_BIT: HLSLBindingFlagBits = 0;

// Push constant (root constant) resources will be declared as CBVs (b-space) without a register() declaration.
// A register will be automatically assigned by the D3D compiler, but must therefore be reflected in D3D-land.
// Push constants do not normally have a DecorationBinding set, but if they do, this can be used to ignore it.
pub const HLSL_BINDING_AUTO_PUSH_CONSTANT_BIT: HLSLBindingFlagBits = 1 << 0;

// cbuffer resources will be declared as CBVs (b-space) without a register() declaration.
// A register will be automatically assigned, but must be reflected in D3D-land.
pub const HLSL_BINDING_AUTO_CBV_BIT: HLSLBindingFlagBits = 1 << 1;

// All SRVs (t-space) will be declared without a register() declaration.
pub const HLSL_BINDING_AUTO_SRV_BIT: HLSLBindingFlagBits = 1 << 2;

// All UAVs (u-space) will be declared without a register() declaration.
pub const HLSL_BINDING_AUTO_UAV_BIT: HLSLBindingFlagBits = 1 << 3;

// All samplers (s-space) will be declared without a register() declaration.
pub const HLSL_BINDING_AUTO_SAMPLER_BIT: HLSLBindingFlagBits = 1 << 4;

// No resources will be declared with register().
pub const HLSL_BINDING_AUTO_ALL: HLSLBindingFlagBits = 0x7fffffff;

/// `HLSLBindingFlags`.
pub type HLSLBindingFlags = u32;

/// `HLSLResourceBinding::Binding`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct HLSLResourceBindingRegister {
    pub register_space: u32,
    pub register_binding: u32,
}

// By matching stage, desc_set and binding for a SPIR-V resource,
// register bindings are set based on whether the HLSL resource is a
// CBV, UAV, SRV or Sampler. A single binding in SPIR-V might contain multiple
// resource types, e.g. COMBINED_IMAGE_SAMPLER, and SRV/Sampler bindings will be used respectively.
// On SM 5.0 and lower, register_space is ignored.
//
// To remap a push constant block which does not have any desc_set/binding associated with it,
// use ResourceBindingPushConstant{DescriptorSet,Binding} as values for desc_set/binding.
// For deeper control of push constants, set_root_constant_layouts() can be used instead.
/// `HLSLResourceBinding`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct HLSLResourceBinding {
    pub stage: ExecutionModel,
    pub desc_set: u32,
    pub binding: u32,

    pub cbv: HLSLResourceBindingRegister,
    pub uav: HLSLResourceBindingRegister,
    pub srv: HLSLResourceBindingRegister,
    pub sampler: HLSLResourceBindingRegister,
}

impl Default for HLSLResourceBinding {
    fn default() -> Self {
        HLSLResourceBinding {
            stage: ExecutionModelMax,
            desc_set: 0,
            binding: 0,
            cbv: HLSLResourceBindingRegister::default(),
            uav: HLSLResourceBindingRegister::default(),
            srv: HLSLResourceBindingRegister::default(),
            sampler: HLSLResourceBindingRegister::default(),
        }
    }
}

/// `HLSLAuxBinding`.
pub type HLSLAuxBinding = u32;
pub const HLSL_AUX_BINDING_BASE_VERTEX_INSTANCE: HLSLAuxBinding = 0;

/// `CompilerHLSL::Options`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct HlslOptions {
    pub shader_model: u32, // TODO: map ps_4_0_level_9_0,... somehow

    // Allows the PointSize builtin in SM 4.0+, and ignores it, as PointSize is not supported in SM 4+.
    pub point_size_compat: bool,

    // Allows the PointCoord builtin, returns float2(0.5, 0.5), as PointCoord is not supported in HLSL.
    pub point_coord_compat: bool,

    // If true, the backend will assume that VertexIndex and InstanceIndex will need to apply
    // a base offset, and you will need to fill in a cbuffer with offsets.
    // Set to false if you know you will never use base instance or base vertex
    // functionality as it might remove an internal cbuffer.
    pub support_nonzero_base_vertex_base_instance: bool,

    // Forces a storage buffer to always be declared as UAV, even if the readonly decoration is used.
    // By default, a readonly storage buffer will be declared as ByteAddressBuffer (SRV) instead.
    // Alternatively, use set_hlsl_force_storage_buffer_as_uav to specify individually.
    pub force_storage_buffer_as_uav: bool,

    // Forces any storage image type marked as NonWritable to be considered an SRV instead.
    // For this to work with function call parameters, NonWritable must be considered to be part of the type system
    // so that NonWritable image arguments are also translated to Texture rather than RWTexture.
    pub nonwritable_uav_texture_as_srv: bool,

    // Enables native 16-bit types. Needs SM 6.2.
    // Uses half/int16_t/uint16_t instead of min16* types.
    // Also adds support for 16-bit load-store from (RW)ByteAddressBuffer.
    pub enable_16bit_types: bool,

    // If matrices are used as IO variables, flatten the attribute declaration to use
    // TEXCOORD{N,N+1,N+2,...} rather than TEXCOORDN_{0,1,2,3}.
    // If add_vertex_attribute_remap is used and this feature is used,
    // the semantic name will be queried once per active location.
    pub flatten_matrix_vertex_input_semantics: bool,

    // Rather than emitting main() for the entry point, use the name in SPIR-V.
    pub use_entry_point_name: bool,

    // Preserve (RW)StructuredBuffer types if the input source was HLSL.
    // This relies on UserTypeGOOGLE to encode the buffer type either as "structuredbuffer" or "rwstructuredbuffer"
    // whereas the type can be extended with an optional subtype, e.g. "structuredbuffer:int".
    pub preserve_structured_buffers: bool,

    // Use UserSemantic decoration info (if specified), otherwise use default mechanism (such as add_vertex_attribute_remap or TEXCOORD#).
    pub user_semantic: bool,
}

impl Default for HlslOptions {
    fn default() -> Self {
        HlslOptions {
            shader_model: 30,
            point_size_compat: false,
            point_coord_compat: false,
            support_nonzero_base_vertex_base_instance: false,
            force_storage_buffer_as_uav: false,
            nonwritable_uav_texture_as_srv: false,
            enable_16bit_types: false,
            flatten_matrix_vertex_input_semantics: false,
            use_entry_point_name: false,
            preserve_structured_buffers: false,
            user_semantic: false,
        }
    }
}

/// `CompilerHLSL::TextureSizeVariants`.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct TextureSizeVariants {
    pub srv: u64,
    pub uav: [[u64; 4]; 3],
}

/// `CompilerHLSL::TextureQueryVariantDim`.
pub(crate) type TextureQueryVariantDim = u32;
pub(crate) const Query1D: TextureQueryVariantDim = 0;
pub(crate) const Query1DArray: TextureQueryVariantDim = 1;
pub(crate) const Query2D: TextureQueryVariantDim = 2;
pub(crate) const Query2DArray: TextureQueryVariantDim = 3;
pub(crate) const Query3D: TextureQueryVariantDim = 4;
pub(crate) const QueryBuffer: TextureQueryVariantDim = 5;
pub(crate) const QueryCube: TextureQueryVariantDim = 6;
pub(crate) const QueryCubeArray: TextureQueryVariantDim = 7;
pub(crate) const Query2DMS: TextureQueryVariantDim = 8;
pub(crate) const Query2DMSArray: TextureQueryVariantDim = 9;
pub(crate) const QueryDimCount: TextureQueryVariantDim = 10;

/// `CompilerHLSL::TextureQueryVariantType`.
pub(crate) type TextureQueryVariantType = u32;
pub(crate) const QueryTypeFloat: TextureQueryVariantType = 0;
pub(crate) const QueryTypeInt: TextureQueryVariantType = 16;
pub(crate) const QueryTypeUInt: TextureQueryVariantType = 32;
pub(crate) const QueryTypeCount: TextureQueryVariantType = 3;

/// `CompilerHLSL::BitcastType`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum BitcastType {
    TypeNormal,
    TypePackUint2x32,
    TypeUnpackUint64,
}

/// `CompilerHLSL::base_vertex_info`.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct BaseVertexInfo {
    pub register_index: u32,
    pub register_space: u32,
    pub explicit_binding: bool,
    pub used: bool,
}

/// CompilerHLSL's state.
#[derive(Debug, Default)]
pub struct HlslState {
    pub(crate) options: HlslOptions,

    // TODO: Refactor this to be more similar to MSL, maybe have some common system in place?
    pub(crate) requires_op_fmod: bool,
    pub(crate) requires_fp16_packing: bool,
    pub(crate) requires_uint2_packing: bool,
    pub(crate) requires_explicit_fp16_packing: bool,
    pub(crate) requires_unorm8_packing: bool,
    pub(crate) requires_snorm8_packing: bool,
    pub(crate) requires_unorm16_packing: bool,
    pub(crate) requires_snorm16_packing: bool,
    pub(crate) requires_bitfield_insert: bool,
    pub(crate) requires_bitfield_extract: bool,
    pub(crate) requires_inverse_2x2: bool,
    pub(crate) requires_inverse_3x3: bool,
    pub(crate) requires_inverse_4x4: bool,
    pub(crate) requires_scalar_reflect: bool,
    pub(crate) requires_scalar_refract: bool,
    pub(crate) requires_scalar_faceforward: bool,

    pub(crate) required_texture_size_variants: TextureSizeVariants,

    pub(crate) require_output: bool,
    pub(crate) require_input: bool,
    pub(crate) remap_vertex_attributes: Vec<HLSLVertexAttributeRemap>,

    pub(crate) num_workgroups_builtin: u32,
    pub(crate) resource_binding_flags: HLSLBindingFlags,

    // Custom root constant layout, which should be emitted
    // when translating push constant ranges.
    pub(crate) root_constants_layout: Vec<RootConstants>,

    pub(crate) unique_identifier_count: u32,

    pub(crate) resource_bindings: HashMap<StageSetBinding, (HLSLResourceBinding, bool)>,

    pub(crate) force_uav_buffer_bindings: HashSet<SetBindingPair>,

    pub(crate) base_vertex_info: BaseVertexInfo,

    pub(crate) composite_selection_workaround_types: Vec<TypeID>,
}

impl Compiler {
    /// `CompilerHLSL::get_hlsl_options()`.
    pub fn get_hlsl_options(&self) -> &HlslOptions {
        &self.hlsl.options
    }

    /// `CompilerHLSL::set_hlsl_options()`.
    pub fn set_hlsl_options(&mut self, opts: &HlslOptions) {
        self.hlsl.options = *opts;
    }

    /// The `CompilerHLSL` constructors, which only construct the `CompilerGLSL` base.
    pub(crate) fn hlsl_init(&mut self) -> Result<()> {
        Ok(())
    }
}

/// `ImageFormatNormalizedState`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ImageFormatNormalizedState {
    None = 0,
    Unorm = 1,
    Snorm = 2,
}

fn image_format_to_normalized_state(fmt: ImageFormat) -> ImageFormatNormalizedState {
    match fmt {
        ImageFormatR8 | ImageFormatR16 | ImageFormatRg8 | ImageFormatRg16 | ImageFormatRgba8
        | ImageFormatRgba16 | ImageFormatRgb10A2 => ImageFormatNormalizedState::Unorm,

        ImageFormatR8Snorm
        | ImageFormatR16Snorm
        | ImageFormatRg8Snorm
        | ImageFormatRg16Snorm
        | ImageFormatRgba8Snorm
        | ImageFormatRgba16Snorm => ImageFormatNormalizedState::Snorm,

        _ => ImageFormatNormalizedState::None,
    }
}

fn image_format_to_components(fmt: ImageFormat) -> Result<u32> {
    match fmt {
        ImageFormatR8 | ImageFormatR16 | ImageFormatR8Snorm | ImageFormatR16Snorm
        | ImageFormatR16f | ImageFormatR32f | ImageFormatR8i | ImageFormatR16i
        | ImageFormatR32i | ImageFormatR8ui | ImageFormatR16ui | ImageFormatR32ui => Ok(1),

        ImageFormatRg8 | ImageFormatRg16 | ImageFormatRg8Snorm | ImageFormatRg16Snorm
        | ImageFormatRg16f | ImageFormatRg32f | ImageFormatRg8i | ImageFormatRg16i
        | ImageFormatRg32i | ImageFormatRg8ui | ImageFormatRg16ui | ImageFormatRg32ui => Ok(2),

        ImageFormatR11fG11fB10f => Ok(3),

        ImageFormatRgba8
        | ImageFormatRgba16
        | ImageFormatRgb10A2
        | ImageFormatRgba8Snorm
        | ImageFormatRgba16Snorm
        | ImageFormatRgba16f
        | ImageFormatRgba32f
        | ImageFormatRgba8i
        | ImageFormatRgba16i
        | ImageFormatRgba32i
        | ImageFormatRgba8ui
        | ImageFormatRgba16ui
        | ImageFormatRgba32ui
        | ImageFormatRgb10a2ui => Ok(4),

        ImageFormatUnknown => Ok(4), // Assume 4.

        _ => spirv_cross_throw!("Unrecognized typed image format."),
    }
}

fn image_format_to_type(fmt: ImageFormat, basetype: BaseType) -> Result<&'static str> {
    let expect = |b: BaseType, s: &'static str| -> Result<&'static str> {
        if basetype != b {
            spirv_cross_throw!("Mismatch in image type and base type of image.");
        }
        Ok(s)
    };
    match fmt {
        ImageFormatR8 | ImageFormatR16 => expect(BaseType::Float, "unorm float"),
        ImageFormatRg8 | ImageFormatRg16 => expect(BaseType::Float, "unorm float2"),
        ImageFormatRgba8 | ImageFormatRgba16 => expect(BaseType::Float, "unorm float4"),
        ImageFormatRgb10A2 => expect(BaseType::Float, "unorm float4"),

        ImageFormatR8Snorm | ImageFormatR16Snorm => expect(BaseType::Float, "snorm float"),
        ImageFormatRg8Snorm | ImageFormatRg16Snorm => expect(BaseType::Float, "snorm float2"),
        ImageFormatRgba8Snorm | ImageFormatRgba16Snorm => expect(BaseType::Float, "snorm float4"),

        ImageFormatR16f | ImageFormatR32f => expect(BaseType::Float, "float"),
        ImageFormatRg16f | ImageFormatRg32f => expect(BaseType::Float, "float2"),
        ImageFormatRgba16f | ImageFormatRgba32f => expect(BaseType::Float, "float4"),

        ImageFormatR11fG11fB10f => expect(BaseType::Float, "float3"),

        ImageFormatR8i | ImageFormatR16i | ImageFormatR32i => expect(BaseType::Int, "int"),
        ImageFormatRg8i | ImageFormatRg16i | ImageFormatRg32i => expect(BaseType::Int, "int2"),
        ImageFormatRgba8i | ImageFormatRgba16i | ImageFormatRgba32i => {
            expect(BaseType::Int, "int4")
        }

        ImageFormatR8ui | ImageFormatR16ui | ImageFormatR32ui => expect(BaseType::UInt, "uint"),
        ImageFormatRg8ui | ImageFormatRg16ui | ImageFormatRg32ui => expect(BaseType::UInt, "uint2"),
        ImageFormatRgba8ui | ImageFormatRgba16ui | ImageFormatRgba32ui => {
            expect(BaseType::UInt, "uint4")
        }
        ImageFormatRgb10a2ui => expect(BaseType::UInt, "uint4"),

        ImageFormatUnknown => match basetype {
            BaseType::Float => Ok("float4"),
            BaseType::Int => Ok("int4"),
            BaseType::UInt => Ok("uint4"),
            _ => spirv_cross_throw!("Unsupported base type for image."),
        },

        _ => spirv_cross_throw!("Unrecognized typed image format."),
    }
}

impl Compiler {
    pub(crate) fn image_type_hlsl_modern(&mut self, type_: &SPIRType, id: u32) -> Result<String> {
        let imagetype = self.get_rc::<SPIRType>(type_.image.type_)?;
        let dim: &str;
        let mut typed_load = false;
        let components = 4;

        let force_image_srv = self.hlsl.options.nonwritable_uav_texture_as_srv
            && self.has_decoration(id, DecorationNonWritable);

        match type_.image.dim {
            Dim1D => {
                typed_load = type_.image.sampled == 2;
                dim = "1D";
            }
            Dim2D => {
                typed_load = type_.image.sampled == 2;
                dim = "2D";
            }
            Dim3D => {
                typed_load = type_.image.sampled == 2;
                dim = "3D";
            }
            DimCube => {
                if type_.image.sampled == 2 {
                    spirv_cross_throw!("RWTextureCube does not exist in HLSL.");
                }
                dim = "Cube";
            }
            DimRect => {
                spirv_cross_throw!("Rectangle texture support is not yet implemented for HLSL.");
                // TODO
            }
            DimBuffer => {
                if type_.image.sampled == 1 {
                    return Ok(join!(
                        "Buffer<",
                        self.type_to_glsl(&imagetype, 0)?,
                        components,
                        ">"
                    ));
                } else if type_.image.sampled == 2 {
                    if self.interlocked_resources.contains(&id) {
                        return Ok(join!(
                            "RasterizerOrderedBuffer<",
                            image_format_to_type(type_.image.format, imagetype.basetype)?,
                            ">"
                        ));
                    }

                    typed_load = !force_image_srv && type_.image.sampled == 2;

                    let rw = if force_image_srv { "" } else { "RW" };
                    let elem = if typed_load {
                        image_format_to_type(type_.image.format, imagetype.basetype)?.to_string()
                    } else {
                        join!(self.type_to_glsl(&imagetype, 0)?, components)
                    };
                    return Ok(join!(rw, "Buffer<", elem, ">"));
                } else {
                    spirv_cross_throw!("Sampler buffers must be either sampled or unsampled. Cannot deduce in runtime.");
                }
            }
            DimSubpassData => {
                dim = "2D";
                typed_load = false;
            }
            _ => {
                spirv_cross_throw!("Invalid dimension.");
            }
        }
        let arrayed = if type_.image.arrayed { "Array" } else { "" };
        let ms = if type_.image.ms { "MS" } else { "" };
        let mut rw = if typed_load && !force_image_srv {
            "RW"
        } else {
            ""
        };

        if force_image_srv {
            typed_load = false;
        }

        if typed_load && self.interlocked_resources.contains(&id) {
            rw = "RasterizerOrdered";
        }

        let elem = if typed_load {
            image_format_to_type(type_.image.format, imagetype.basetype)?.to_string()
        } else {
            join!(self.type_to_glsl(&imagetype, 0)?, components)
        };
        Ok(join!(rw, "Texture", dim, ms, arrayed, "<", elem, ">"))
    }

    pub(crate) fn image_type_hlsl_legacy(&mut self, type_: &SPIRType, _id: u32) -> Result<String> {
        let imagetype_basetype = self.get::<SPIRType>(type_.image.type_)?.basetype;
        let mut res = String::new();

        match imagetype_basetype {
            BaseType::Int => res = "i".into(),
            BaseType::UInt => res = "u".into(),
            _ => {}
        }

        if type_.basetype == BaseType::Image && type_.image.dim == DimSubpassData {
            return Ok(res + "subpassInput" + if type_.image.ms { "MS" } else { "" });
        }

        // If we're emulating subpassInput with samplers, force sampler2D
        // so we don't have to specify format.
        if type_.basetype == BaseType::Image && type_.image.dim != DimSubpassData {
            // Sampler buffers are always declared as samplerBuffer even though they might be separate images in the SPIR-V.
            if type_.image.dim == DimBuffer && type_.image.sampled == 1 {
                res += "sampler";
            } else {
                res += if type_.image.sampled == 2 {
                    "image"
                } else {
                    "texture"
                };
            }
        } else {
            res += "sampler";
        }

        match type_.image.dim {
            Dim1D => res += "1D",
            Dim2D => res += "2D",
            Dim3D => res += "3D",
            DimCube => res += "CUBE",

            DimBuffer => res += "Buffer",

            DimSubpassData => res += "2D",
            _ => {
                spirv_cross_throw!(
                    "Only 1D, 2D, 3D, Buffer, InputTarget and Cube textures supported."
                );
            }
        }

        if type_.image.ms {
            res += "MS";
        }
        if type_.image.arrayed {
            res += "Array";
        }

        Ok(res)
    }

    pub(crate) fn image_type_hlsl(&mut self, type_: &SPIRType, id: u32) -> Result<String> {
        if self.hlsl.options.shader_model <= 30 {
            self.image_type_hlsl_legacy(type_, id)
        } else {
            self.image_type_hlsl_modern(type_, id)
        }
    }

    // The optional id parameter indicates the object whose type we are trying
    // to find the description for. It is optional. Most type descriptions do not
    // depend on a specific object's use of that type.
    pub(crate) fn hlsl_type_to_glsl(&mut self, type_: &SPIRType, id: u32) -> Result<String> {
        // Ignore the pointer type since GLSL doesn't have pointers.

        match type_.basetype {
            BaseType::Struct => {
                // Need OpName lookup here to get a "sensible" name for a struct.
                if self.glsl.backend.explicit_struct_type {
                    return Ok(join!("struct ", self.to_name(type_.self_, true)?));
                } else {
                    return self.to_name(type_.self_, true);
                }
            }

            BaseType::Image | BaseType::SampledImage => return self.image_type_hlsl(type_, id),

            BaseType::Sampler => {
                return Ok(if self.comparison_ids.contains(&id) {
                    "SamplerComparisonState".into()
                } else {
                    "SamplerState".into()
                });
            }

            BaseType::Void => return Ok("void".into()),

            _ => {}
        }

        let sixteen = self.hlsl.options.enable_16bit_types;
        let half = if sixteen { "half" } else { "min16float" };
        let short = if sixteen { "int16_t" } else { "min16int" };
        let ushort = if sixteen { "uint16_t" } else { "min16uint" };

        if type_.vecsize == 1 && type_.columns == 1 {
            // Scalar builtin
            Ok(match type_.basetype {
                BaseType::Boolean => "bool".into(),
                BaseType::Int => self.glsl.backend.basic_int_type.into(),
                BaseType::UInt => self.glsl.backend.basic_uint_type.into(),
                BaseType::AtomicCounter => "atomic_uint".into(),
                BaseType::Half => half.into(),
                BaseType::Short => short.into(),
                BaseType::UShort => ushort.into(),
                BaseType::Float => "float".into(),
                BaseType::Double => "double".into(),
                BaseType::Int64 => {
                    if self.hlsl.options.shader_model < 60 {
                        spirv_cross_throw!("64-bit integers only supported in SM 6.0.");
                    }
                    "int64_t".into()
                }
                BaseType::UInt64 => {
                    if self.hlsl.options.shader_model < 60 {
                        spirv_cross_throw!("64-bit integers only supported in SM 6.0.");
                    }
                    "uint64_t".into()
                }
                BaseType::AccelerationStructure => "RaytracingAccelerationStructure".into(),
                BaseType::RayQuery => "RayQuery<RAY_FLAG_NONE>".into(),
                _ => "???".into(),
            })
        } else if type_.vecsize > 1 && type_.columns == 1 {
            // Vector builtin
            let v = type_.vecsize;
            Ok(match type_.basetype {
                BaseType::Boolean => join!("bool", v),
                BaseType::Int => join!("int", v),
                BaseType::UInt => join!("uint", v),
                BaseType::Half => join!(half, v),
                BaseType::Short => join!(short, v),
                BaseType::UShort => join!(ushort, v),
                BaseType::Float => join!("float", v),
                BaseType::Double => join!("double", v),
                BaseType::Int64 => join!("int64_t", v),
                BaseType::UInt64 => join!("uint64_t", v),
                _ => "???".into(),
            })
        } else {
            let c = type_.columns;
            let v = type_.vecsize;
            Ok(match type_.basetype {
                BaseType::Boolean => join!("bool", c, "x", v),
                BaseType::Int => join!("int", c, "x", v),
                BaseType::UInt => join!("uint", c, "x", v),
                BaseType::Half => join!(half, c, "x", v),
                BaseType::Short => join!(short, c, "x", v),
                BaseType::UShort => join!(ushort, c, "x", v),
                BaseType::Float => join!("float", c, "x", v),
                BaseType::Double => join!("double", c, "x", v),
                // Matrix types not supported for int64/uint64.
                _ => "???".into(),
            })
        }
    }

    pub(crate) fn hlsl_emit_header(&mut self) -> Result<()> {
        for header in self.glsl.header_lines.clone() {
            statement!(self, header);
        }

        if !self.glsl.header_lines.is_empty() {
            statement!(self, "");
        }
        Ok(())
    }

    pub(crate) fn emit_interface_block_globally(&mut self, var: &SPIRVariable) -> Result<()> {
        self.add_resource_name(var.self_)?;

        // The global copies of I/O variables should not contain interpolation qualifiers.
        // These are emitted inside the interface structs.
        let old_flags = std::mem::take(&mut self.meta_mut(var.self_).decoration.decoration_flags);
        let decl = self.variable_decl(var);
        self.meta_mut(var.self_).decoration.decoration_flags = old_flags;
        statement!(self, "static ", decl?, ";");
        Ok(())
    }

    pub(crate) fn hlsl_to_storage_qualifiers_glsl(
        &mut self,
        var: &SPIRVariable,
    ) -> Result<&'static str> {
        // Input and output variables are handled specially in HLSL backend.
        // The variables are declared as global, private variables, and do not need any qualifiers.
        if var.storage == StorageClassUniformConstant
            || var.storage == StorageClassUniform
            || var.storage == StorageClassPushConstant
        {
            return Ok("uniform ");
        }

        Ok("")
    }

    pub(crate) fn emit_builtin_outputs_in_struct(&mut self) -> Result<()> {
        let execution_model = self.get_entry_point().model;
        let execution_flags = self.get_entry_point().flags.clone();

        let legacy = self.hlsl.options.shader_model <= 30;
        for i in self.active_output_builtins.bits() {
            let mut type_: Option<&str> = None;
            let mut semantic: Option<&str> = None;
            let builtin = i as BuiltIn;
            match builtin {
                BuiltInPosition => {
                    type_ = Some(
                        if self.is_position_invariant()
                            && self.glsl.backend.support_precise_qualifier
                        {
                            "precise float4"
                        } else {
                            "float4"
                        },
                    );
                    semantic = Some(if legacy { "POSITION" } else { "SV_Position" });
                }

                BuiltInSampleMask => {
                    if self.hlsl.options.shader_model < 41
                        || execution_model != ExecutionModelFragment
                    {
                        spirv_cross_throw!(
                            "Sample Mask output is only supported in PS 4.1 or higher."
                        );
                    }
                    type_ = Some("uint");
                    semantic = Some("SV_Coverage");
                }

                BuiltInFragDepth => {
                    type_ = Some("float");
                    if legacy {
                        semantic = Some("DEPTH");
                    } else if self.hlsl.options.shader_model >= 50
                        && execution_flags.get(ExecutionModeDepthGreater)
                    {
                        semantic = Some("SV_DepthGreaterEqual");
                    } else if self.hlsl.options.shader_model >= 50
                        && execution_flags.get(ExecutionModeDepthLess)
                    {
                        semantic = Some("SV_DepthLessEqual");
                    } else {
                        semantic = Some("SV_Depth");
                    }
                }

                BuiltInClipDistance => {
                    const TYPES: [&str; 4] = ["float", "float2", "float3", "float4"];

                    // HLSL is a bit weird here, use SV_ClipDistance0, SV_ClipDistance1 and so on with vectors.
                    if execution_model == ExecutionModelMeshEXT {
                        if self.clip_distance_count > 4 {
                            spirv_cross_throw!(
                                "Clip distance count > 4 not supported for mesh shaders."
                            );
                        }

                        if self.clip_distance_count == 1 {
                            // Avoids having to hack up access_chain code. Makes it trivially indexable.
                            statement!(self, "float gl_ClipDistance[1] : SV_ClipDistance;");
                        } else {
                            // Replace array with vector directly, avoids any weird fixup path.
                            // FIXME (upstream): a count of 0 indexes types[-1].
                            let t = TYPES[(self.clip_distance_count as usize).wrapping_sub(1) % 4];
                            statement!(self, t, " gl_ClipDistance : SV_ClipDistance;");
                        }
                    } else {
                        let mut clip = 0;
                        while clip < self.clip_distance_count {
                            let mut to_declare = self.clip_distance_count - clip;
                            if to_declare > 4 {
                                to_declare = 4;
                            }

                            let semantic_index = clip / 4;

                            statement!(
                                self,
                                TYPES[(to_declare - 1) as usize],
                                " ",
                                self.builtin_to_glsl(builtin, StorageClassOutput)?,
                                semantic_index,
                                " : SV_ClipDistance",
                                semantic_index,
                                ";"
                            );
                            clip += 4;
                        }
                    }
                }

                BuiltInCullDistance => {
                    const TYPES: [&str; 4] = ["float", "float2", "float3", "float4"];

                    // HLSL is a bit weird here, use SV_CullDistance0, SV_CullDistance1 and so on with vectors.
                    if execution_model == ExecutionModelMeshEXT {
                        if self.cull_distance_count > 4 {
                            spirv_cross_throw!(
                                "Cull distance count > 4 not supported for mesh shaders."
                            );
                        }

                        if self.cull_distance_count == 1 {
                            // Avoids having to hack up access_chain code. Makes it trivially indexable.
                            statement!(self, "float gl_CullDistance[1] : SV_CullDistance;");
                        } else {
                            // Replace array with vector directly, avoids any weird fixup path.
                            // FIXME (upstream): a count of 0 indexes types[-1].
                            let t = TYPES[(self.cull_distance_count as usize).wrapping_sub(1) % 4];
                            statement!(self, t, " gl_CullDistance : SV_CullDistance;");
                        }
                    } else {
                        let mut cull = 0;
                        while cull < self.cull_distance_count {
                            let mut to_declare = self.cull_distance_count - cull;
                            if to_declare > 4 {
                                to_declare = 4;
                            }

                            let semantic_index = cull / 4;

                            statement!(
                                self,
                                TYPES[(to_declare - 1) as usize],
                                " ",
                                self.builtin_to_glsl(builtin, StorageClassOutput)?,
                                semantic_index,
                                " : SV_CullDistance",
                                semantic_index,
                                ";"
                            );
                            cull += 4;
                        }
                    }
                }

                BuiltInPointSize => {
                    // If point_size_compat is enabled, just ignore PointSize.
                    // PointSize does not exist in HLSL, but some code bases might want to be able to use these shaders,
                    // even if it means working around the missing feature.
                    if legacy {
                        type_ = Some("float");
                        semantic = Some("PSIZE");
                    } else if !self.hlsl.options.point_size_compat {
                        spirv_cross_throw!("Unsupported builtin in HLSL.");
                    }
                }

                BuiltInLayer
                | BuiltInPrimitiveId
                | BuiltInViewportIndex
                | BuiltInPrimitiveShadingRateKHR
                | BuiltInCullPrimitiveEXT => {
                    // per-primitive attributes handled separatly
                }

                BuiltInPrimitivePointIndicesEXT
                | BuiltInPrimitiveLineIndicesEXT
                | BuiltInPrimitiveTriangleIndicesEXT => {
                    // meshlet local-index buffer handled separatly
                }

                _ => spirv_cross_throw!("Unsupported builtin in HLSL."),
            }

            if let (Some(t), Some(s)) = (type_, semantic) {
                statement!(
                    self,
                    t,
                    " ",
                    self.builtin_to_glsl(builtin, StorageClassOutput)?,
                    " : ",
                    s,
                    ";"
                );
            }
        }
        Ok(())
    }

    pub(crate) fn emit_builtin_primitive_outputs_in_struct(&mut self) -> Result<()> {
        for i in self.active_output_builtins.bits() {
            let mut type_: Option<&str> = None;
            let mut semantic: Option<&str> = None;
            let builtin = i as BuiltIn;
            match builtin {
                BuiltInLayer => {
                    if self.hlsl.options.shader_model < 50 {
                        spirv_cross_throw!(
                            "Render target array index output is only supported in SM 5.0 or higher."
                        );
                    }
                    type_ = Some("uint");
                    semantic = Some("SV_RenderTargetArrayIndex");
                }

                BuiltInPrimitiveId => {
                    type_ = Some("uint");
                    semantic = Some("SV_PrimitiveID");
                }

                BuiltInViewportIndex => {
                    type_ = Some("uint");
                    semantic = Some("SV_ViewportArrayIndex");
                }

                BuiltInPrimitiveShadingRateKHR => {
                    type_ = Some("uint");
                    semantic = Some("SV_ShadingRate");
                }

                BuiltInCullPrimitiveEXT => {
                    type_ = Some("bool");
                    semantic = Some("SV_CullPrimitive");
                }

                _ => {}
            }

            if let (Some(t), Some(s)) = (type_, semantic) {
                statement!(
                    self,
                    t,
                    " ",
                    self.builtin_to_glsl(builtin, StorageClassOutput)?,
                    " : ",
                    s,
                    ";"
                );
            }
        }
        Ok(())
    }

    pub(crate) fn emit_builtin_inputs_in_struct(&mut self) -> Result<()> {
        let legacy = self.hlsl.options.shader_model <= 30;
        let model = self.get_entry_point().model;
        for i in self.active_input_builtins.bits() {
            let mut type_: Option<&str> = None;
            let mut semantic: Option<&str> = None;
            let builtin = i as BuiltIn;
            match builtin {
                BuiltInPosition => {
                    type_ = Some("float4");
                    semantic = Some(if legacy { "POSITION" } else { "SV_Position" });
                }
                BuiltInFragCoord => {
                    type_ = Some("float4");
                    semantic = Some(if legacy { "VPOS" } else { "SV_Position" });
                }

                BuiltInVertexId | BuiltInVertexIndex => {
                    if legacy {
                        spirv_cross_throw!("Vertex index not supported in SM 3.0 or lower.");
                    }
                    type_ = Some("uint");
                    semantic = Some("SV_VertexID");
                }

                BuiltInPrimitiveId => {
                    // For geometry shaders, PrimitiveId is a direct function parameter
                    // (SV_PrimitiveID), not part of the input struct.
                    if model != ExecutionModelGeometry {
                        type_ = Some("uint");
                        semantic = Some("SV_PrimitiveID");
                    }
                }

                BuiltInInvocationId => {
                    if model == ExecutionModelGeometry {
                        type_ = Some("uint");
                        semantic = Some("SV_GSInstanceID");
                    } else if model != ExecutionModelTessellationControl {
                        // For tesc, InvocationId is a direct function parameter (SV_OutputControlPointID),
                        // not part of the input struct.
                        spirv_cross_throw!(
                            "InvocationId is only supported in geometry and tessellation control shaders."
                        );
                    }
                }

                BuiltInInstanceId | BuiltInInstanceIndex => {
                    if legacy {
                        spirv_cross_throw!("Instance index not supported in SM 3.0 or lower.");
                    }
                    type_ = Some("uint");
                    semantic = Some("SV_InstanceID");
                }

                BuiltInSampleId => {
                    if legacy {
                        spirv_cross_throw!("Sample ID not supported in SM 3.0 or lower.");
                    }
                    type_ = Some("uint");
                    semantic = Some("SV_SampleIndex");
                }

                BuiltInSampleMask => {
                    if self.hlsl.options.shader_model < 50 || model != ExecutionModelFragment {
                        spirv_cross_throw!(
                            "Sample Mask input is only supported in PS 5.0 or higher."
                        );
                    }
                    type_ = Some("uint");
                    semantic = Some("SV_Coverage");
                }

                BuiltInGlobalInvocationId => {
                    type_ = Some("uint3");
                    semantic = Some("SV_DispatchThreadID");
                }

                BuiltInLocalInvocationId => {
                    type_ = Some("uint3");
                    semantic = Some("SV_GroupThreadID");
                }

                BuiltInLocalInvocationIndex => {
                    type_ = Some("uint");
                    semantic = Some("SV_GroupIndex");
                }

                BuiltInWorkgroupId => {
                    type_ = Some("uint3");
                    semantic = Some("SV_GroupID");
                }

                BuiltInFrontFacing => {
                    type_ = Some("bool");
                    semantic = Some("SV_IsFrontFace");
                }

                BuiltInViewIndex => {
                    if self.hlsl.options.shader_model < 61
                        || (model != ExecutionModelVertex && model != ExecutionModelFragment)
                    {
                        spirv_cross_throw!(
                            "View Index input is only supported in VS and PS 6.1 or higher."
                        );
                    }
                    type_ = Some("uint");
                    semantic = Some("SV_ViewID");
                }

                BuiltInNumWorkgroups
                | BuiltInSubgroupSize
                | BuiltInSubgroupLocalInvocationId
                | BuiltInSubgroupEqMask
                | BuiltInSubgroupLtMask
                | BuiltInSubgroupLeMask
                | BuiltInSubgroupGtMask
                | BuiltInSubgroupGeMask => {
                    // Handled specially.
                }

                BuiltInBaseVertex => {
                    if self.hlsl.options.shader_model >= 68 {
                        type_ = Some("uint");
                        semantic = Some("SV_StartVertexLocation");
                    }
                }

                BuiltInBaseInstance => {
                    if self.hlsl.options.shader_model >= 68 {
                        type_ = Some("uint");
                        semantic = Some("SV_StartInstanceLocation");
                    }
                }

                BuiltInHelperInvocation => {
                    if self.hlsl.options.shader_model < 50 || model != ExecutionModelFragment {
                        spirv_cross_throw!(
                            "Helper Invocation input is only supported in PS 5.0 or higher."
                        );
                    }
                }

                BuiltInClipDistance => {
                    // HLSL is a bit weird here, use SV_ClipDistance0, SV_ClipDistance1 and so on with vectors.
                    let mut clip = 0;
                    while clip < self.clip_distance_count {
                        let mut to_declare = self.clip_distance_count - clip;
                        if to_declare > 4 {
                            to_declare = 4;
                        }

                        let semantic_index = clip / 4;

                        const TYPES: [&str; 4] = ["float", "float2", "float3", "float4"];
                        statement!(
                            self,
                            TYPES[(to_declare - 1) as usize],
                            " ",
                            self.builtin_to_glsl(builtin, StorageClassInput)?,
                            semantic_index,
                            " : SV_ClipDistance",
                            semantic_index,
                            ";"
                        );
                        clip += 4;
                    }
                }

                BuiltInCullDistance => {
                    // HLSL is a bit weird here, use SV_CullDistance0, SV_CullDistance1 and so on with vectors.
                    let mut cull = 0;
                    while cull < self.cull_distance_count {
                        let mut to_declare = self.cull_distance_count - cull;
                        if to_declare > 4 {
                            to_declare = 4;
                        }

                        let semantic_index = cull / 4;

                        const TYPES: [&str; 4] = ["float", "float2", "float3", "float4"];
                        statement!(
                            self,
                            TYPES[(to_declare - 1) as usize],
                            " ",
                            self.builtin_to_glsl(builtin, StorageClassInput)?,
                            semantic_index,
                            " : SV_CullDistance",
                            semantic_index,
                            ";"
                        );
                        cull += 4;
                    }
                }

                BuiltInPointCoord => {
                    // PointCoord is not supported, but provide a way to just ignore that, similar to PointSize.
                    if !self.hlsl.options.point_coord_compat {
                        spirv_cross_throw!("Unsupported builtin in HLSL.");
                    }
                }

                BuiltInLayer => {
                    if self.hlsl.options.shader_model < 50 || model != ExecutionModelFragment {
                        spirv_cross_throw!(
                            "Render target array index input is only supported in PS 5.0 or higher."
                        );
                    }
                    type_ = Some("uint");
                    semantic = Some("SV_RenderTargetArrayIndex");
                }

                BuiltInBaryCoordKHR | BuiltInBaryCoordNoPerspKHR => {
                    if self.hlsl.options.shader_model < 61 {
                        spirv_cross_throw!("SM 6.1 is required for barycentrics.");
                    }
                    type_ = Some(if builtin == BuiltInBaryCoordNoPerspKHR {
                        "noperspective float3"
                    } else {
                        "float3"
                    });
                    if self.active_input_builtins.get(BuiltInBaryCoordKHR)
                        && self.active_input_builtins.get(BuiltInBaryCoordNoPerspKHR)
                    {
                        semantic = Some(if builtin == BuiltInBaryCoordKHR {
                            "SV_Barycentrics0"
                        } else {
                            "SV_Barycentrics1"
                        });
                    } else {
                        semantic = Some("SV_Barycentrics");
                    }
                }

                _ => spirv_cross_throw!("Unsupported builtin in HLSL."),
            }

            if let (Some(t), Some(s)) = (type_, semantic) {
                statement!(
                    self,
                    t,
                    " ",
                    self.builtin_to_glsl(builtin, StorageClassInput)?,
                    " : ",
                    s,
                    ";"
                );
            }
        }
        Ok(())
    }

    pub(crate) fn type_to_consumed_locations(&self, type_: &SPIRType) -> Result<u32> {
        self.type_to_consumed_locations_d(type_, 0)
    }

    fn type_to_consumed_locations_d(&self, type_: &SPIRType, depth: u32) -> Result<u32> {
        if depth > 1024 {
            spirv_cross_throw!("Type recursion too deep.");
        }
        // TODO: Need to verify correctness.
        let mut elements: u32 = 0;

        if type_.basetype == BaseType::Struct {
            for &m in &type_.member_types {
                elements = elements.wrapping_add(
                    self.type_to_consumed_locations_d(self.get::<SPIRType>(m)?, depth + 1)?,
                );
            }
        } else {
            let mut array_multiplier: u32 = 1;
            for i in 0..type_.array.len() {
                if type_.array_size_literal[i] {
                    array_multiplier = array_multiplier.wrapping_mul(type_.array[i]);
                } else {
                    array_multiplier =
                        array_multiplier.wrapping_mul(self.evaluate_constant_u32(type_.array[i])?);
                }
            }
            elements = elements.wrapping_add(array_multiplier.wrapping_mul(type_.columns));
        }
        Ok(elements)
    }

    pub(crate) fn hlsl_to_interpolation_qualifiers(&mut self, flags: &Bitset) -> Result<String> {
        let mut res = String::new();
        //if (flags & (1ull << DecorationSmooth))
        //    res += "linear ";
        if flags.get(DecorationFlat) || flags.get(DecorationPerVertexKHR) {
            res += "nointerpolation ";
        }
        if flags.get(DecorationNoPerspective) {
            res += "noperspective ";
        }
        if flags.get(DecorationCentroid) {
            res += "centroid ";
        }
        if flags.get(DecorationPatch) {
            res += "patch "; // Seems to be different in actual HLSL.
        }
        if flags.get(DecorationSample) {
            res += "sample ";
        }
        if flags.get(DecorationInvariant) && self.glsl.backend.support_precise_qualifier {
            res += "precise "; // Not supported?
        }

        Ok(res)
    }

    pub(crate) fn to_semantic(
        &self,
        location: u32,
        em: ExecutionModel,
        sc: StorageClass,
    ) -> String {
        if em == ExecutionModelVertex && sc == StorageClassInput {
            // We have a vertex attribute - we should look at remapping it if the user provided
            // vertex attribute hints.
            for attribute in &self.hlsl.remap_vertex_attributes {
                if attribute.location == location {
                    return attribute.semantic.clone();
                }
            }
        }

        // Not a vertex attribute, or no remap_vertex_attributes entry.
        join!("TEXCOORD", location)
    }

    pub(crate) fn hlsl_to_initializer_expression(&mut self, var: &SPIRVariable) -> Result<String> {
        // We cannot emit static const initializer for block constants for practical reasons,
        // so just inline the initializer.
        // FIXME: There is a theoretical problem here if someone tries to composite extract
        // into this initializer since we don't declare it properly, but that is somewhat non-sensical.
        let type_self = self.get::<SPIRType>(var.basetype)?.self_;
        let is_block = self.has_decoration(type_self, DecorationBlock);
        let c = self.maybe_get_rc::<SPIRConstant>(var.initializer);
        if let (true, Some(c)) = (is_block, c) {
            self.constant_expression(&c, false, false)
        } else {
            self.to_unpacked_expression(var.initializer, true)
        }
    }

    pub(crate) fn emit_interface_block_member_in_struct(
        &mut self,
        var: &SPIRVariable,
        member_index: u32,
        location: u32,
        active_locations: &mut HashSet<u32>,
    ) -> Result<()> {
        let execution_model = self.get_entry_point().model;
        let type_ = self.get::<SPIRType>(var.basetype)?.clone();

        let semantic = if self.hlsl.options.user_semantic
            && self.has_member_decoration(var.self_, member_index, DecorationUserSemantic)
        {
            self.get_member_decoration_string(var.self_, member_index, DecorationUserSemantic)
                .clone()
        } else {
            self.to_semantic(location, execution_model, var.storage)
        };

        let mbr_name = join!(
            self.to_name(type_.self_, true)?,
            "_",
            self.to_member_name(&type_, member_index)?
        );
        let Some(&mbr_type_id) = type_.member_types.get(member_index as usize) else {
            spirv_cross_throw!("Member index is out of range.");
        };
        let mbr_type = self.get_rc::<SPIRType>(mbr_type_id)?;

        let mut member_decorations = self
            .get_member_decoration_bitset(type_.self_, member_index)
            .clone();
        if self.has_decoration(var.self_, DecorationPerVertexKHR) {
            member_decorations.set(DecorationPerVertexKHR);
        }

        statement!(
            self,
            self.to_interpolation_qualifiers(&member_decorations)?,
            self.type_to_glsl(&mbr_type, 0)?,
            " ",
            mbr_name,
            self.type_to_array_glsl(&mbr_type, var.self_)?,
            " : ",
            semantic,
            ";"
        );

        // Structs and arrays should consume more locations.
        let consumed_locations = self.type_to_consumed_locations(&mbr_type)?;
        for i in 0..consumed_locations {
            active_locations.insert(location.wrapping_add(i));
        }
        Ok(())
    }

    pub(crate) fn emit_interface_block_in_struct(
        &mut self,
        var: &SPIRVariable,
        active_locations: &mut HashSet<u32>,
    ) -> Result<()> {
        let execution_model = self.get_entry_point().model;
        let mut type_ = self.get::<SPIRType>(var.basetype)?.clone();

        let mut binding = String::new();
        let mut use_location_number = true;
        let mut need_matrix_unroll = false;
        let legacy = self.hlsl.options.shader_model <= 30;
        if execution_model == ExecutionModelFragment && var.storage == StorageClassOutput {
            // Dual-source blending is achieved in HLSL by emitting to SV_Target0 and 1.
            let index = self.get_decoration(var.self_, DecorationIndex);
            let location = self.get_decoration(var.self_, DecorationLocation);

            if index != 0 && location != 0 {
                spirv_cross_throw!("Dual-source blending is only supported on MRT #0 in HLSL.");
            }

            binding = join!(
                if legacy { "COLOR" } else { "SV_Target" },
                location.wrapping_add(index)
            );
            use_location_number = false;
            if legacy {
                // COLOR must be a four-component vector on legacy shader model targets (HLSL ERR_COLOR_4COMP)
                type_.vecsize = 4;
            }
        } else if var.storage == StorageClassInput && execution_model == ExecutionModelVertex {
            need_matrix_unroll = true;
            if legacy {
                // Inputs must be floating-point in legacy targets.
                type_.basetype = BaseType::Float;
            }
        }

        let get_vacant_location = |active_locations: &HashSet<u32>| -> Result<u32> {
            for i in 0..64 {
                if !active_locations.contains(&i) {
                    return Ok(i);
                }
            }
            spirv_cross_throw!("All locations from 0 to 63 are exhausted.");
        };

        let name = self.to_name(var.self_, true)?;
        if use_location_number {
            let mut location_number = u32::MAX;

            let semantic;
            let mut has_user_semantic = false;

            if self.hlsl.options.user_semantic
                && self.has_decoration(var.self_, DecorationUserSemantic)
            {
                semantic = self
                    .get_decoration_string(var.self_, DecorationUserSemantic)
                    .clone();
                has_user_semantic = true;
            } else {
                // If an explicit location exists, use it with TEXCOORD[N] semantic.
                // Otherwise, pick a vacant location.
                if self.has_decoration(var.self_, DecorationLocation) {
                    location_number = self.get_decoration(var.self_, DecorationLocation);
                } else {
                    location_number = get_vacant_location(active_locations)?;
                }

                // Allow semantic remap if specified.
                semantic = self.to_semantic(location_number, execution_model, var.storage);
            }

            if need_matrix_unroll && type_.columns > 1 {
                if !type_.array.is_empty() {
                    spirv_cross_throw!(
                        "Arrays of matrices used as input/output. This is not supported."
                    );
                }

                // Unroll matrices.
                for i in 0..type_.columns {
                    let mut newtype = type_.clone();
                    newtype.columns = 1;

                    let effective_semantic =
                        if self.hlsl.options.flatten_matrix_vertex_input_semantics
                            && !has_user_semantic
                        {
                            self.to_semantic(location_number, execution_model, var.storage)
                        } else {
                            join!(semantic, "_", i)
                        };

                    let flags = self.get_decoration_bitset(var.self_).clone();
                    statement!(
                        self,
                        self.to_interpolation_qualifiers(&flags)?,
                        self.variable_decl_type(&newtype, &join!(name, "_", i), 0)?,
                        " : ",
                        effective_semantic,
                        ";"
                    );
                    if location_number != u32::MAX {
                        active_locations.insert(location_number);
                        location_number = location_number.wrapping_add(1);
                    }
                }
            } else {
                let mut decl_type = type_.clone();
                if execution_model == ExecutionModelMeshEXT
                    || (execution_model == ExecutionModelGeometry
                        && var.storage == StorageClassInput)
                    || self.has_decoration(var.self_, DecorationPerVertexKHR)
                {
                    // The per-vertex/per-CP dimension is the outermost (last element in array vector).
                    // FIXME (upstream): pop_back() on an empty array is undefined.
                    if decl_type.array.pop().is_none()
                        || decl_type.array_size_literal.pop().is_none()
                    {
                        spirv_cross_throw!("Per-vertex interface variable is not an array.");
                    }
                }
                let flags = self.get_decoration_bitset(var.self_).clone();
                statement!(
                    self,
                    self.to_interpolation_qualifiers(&flags)?,
                    self.variable_decl_type(&decl_type, &name, 0)?,
                    " : ",
                    semantic,
                    ";"
                );

                if location_number != u32::MAX {
                    // Structs and arrays should consume more locations.
                    let consumed_locations = self.type_to_consumed_locations(&decl_type)?;
                    for i in 0..consumed_locations {
                        active_locations.insert(location_number.wrapping_add(i));
                    }
                }
            }
        } else {
            statement!(
                self,
                self.variable_decl_type(&type_, &name, 0)?,
                " : ",
                binding,
                ";"
            );
        }
        Ok(())
    }

    pub(crate) fn hlsl_builtin_to_glsl(
        &mut self,
        builtin: BuiltIn,
        storage: StorageClass,
    ) -> Result<String> {
        match builtin {
            BuiltInPosition => {
                // We want to avoid clash between input/output for geometry shader
                Ok(if storage == StorageClassInput {
                    "gl_PositionIn"
                } else {
                    "gl_Position"
                }
                .into())
            }
            BuiltInVertexId => Ok("gl_VertexID".into()),
            BuiltInInstanceId => Ok("gl_InstanceID".into()),
            BuiltInNumWorkgroups => {
                if self.hlsl.num_workgroups_builtin == 0 {
                    spirv_cross_throw!(
                        "NumWorkgroups builtin is used, but remap_num_workgroups_builtin() was not called. \
                         Cannot emit code for this builtin."
                    );
                }

                let nwb = self.hlsl.num_workgroups_builtin;
                let var_basetype = self.get::<SPIRVariable>(nwb)?.basetype;
                let type_self = self.get::<SPIRType>(var_basetype)?.self_;
                let mut ret = join!(
                    self.to_name(nwb, true)?,
                    "_",
                    self.get_member_name(type_self, 0)
                );
                ParsedIR::sanitize_underscores(&mut ret);
                Ok(ret)
            }
            BuiltInPointCoord => {
                // Crude hack, but there is no real alternative. This path is only enabled if point_coord_compat is set.
                Ok("float2(0.5f, 0.5f)".into())
            }
            BuiltInSubgroupLocalInvocationId => Ok("WaveGetLaneIndex()".into()),
            BuiltInSubgroupSize => Ok("WaveGetLaneCount()".into()),
            BuiltInHelperInvocation => Ok("IsHelperLane()".into()),

            _ => self.glsl_builtin_to_glsl(builtin, storage),
        }
    }

    pub(crate) fn emit_builtin_variables(&mut self) -> Result<()> {
        let mut builtins = self.active_input_builtins.clone();
        builtins.merge_or(&self.active_output_builtins);

        let mut builtin_to_initializer: HashMap<u32, ID> = HashMap::new();

        // We need to declare sample mask with the same type that module declares it.
        // Sample mask is somewhat special in that SPIR-V has an array, and we can copy that array, so we need to
        // match sign.
        let mut sample_mask_in_basetype = BaseType::Void;
        let mut sample_mask_out_basetype = BaseType::Void;

        for id in self.ir.typed_ids(TypeVariable) {
            let var = self.get_rc::<SPIRVariable>(id)?;
            if !self.is_builtin_variable(&var)? {
                continue;
            }

            let type_ = self.get_rc::<SPIRType>(var.basetype)?;
            let builtin = self.get_decoration(var.self_, DecorationBuiltIn) as BuiltIn;

            if var.storage == StorageClassInput && builtin == BuiltInSampleMask {
                sample_mask_in_basetype = type_.basetype;
            } else if var.storage == StorageClassOutput && builtin == BuiltInSampleMask {
                sample_mask_out_basetype = type_.basetype;
            }

            if var.initializer != 0 && var.storage == StorageClassOutput {
                let Some(c) = self.maybe_get_rc::<SPIRConstant>(var.initializer) else {
                    continue;
                };

                if type_.basetype == BaseType::Struct {
                    let member_count = type_.member_types.len() as u32;
                    for i in 0..member_count {
                        if self.has_member_decoration(type_.self_, i, DecorationBuiltIn) {
                            let Some(&sub) = c.subconstants.get(i as usize) else {
                                spirv_cross_throw!("Constant initializer has too few members.");
                            };
                            builtin_to_initializer.insert(
                                self.get_member_decoration(type_.self_, i, DecorationBuiltIn),
                                sub,
                            );
                        }
                    }
                } else if self.has_decoration(var.self_, DecorationBuiltIn) {
                    builtin_to_initializer.insert(builtin, var.initializer);
                }
            }
        }

        // Emit global variables for the interface variables which are statically used by the shader.
        for i in builtins.bits() {
            let builtin = i as BuiltIn;

            let mut init_expr = String::new();
            if let Some(&init) = builtin_to_initializer.get(&builtin) {
                init_expr = join!(" = ", self.to_expression(init, true)?);
            }

            if self.get_execution_model() == ExecutionModelMeshEXT
                && (builtin == BuiltInPosition
                    || builtin == BuiltInPointSize
                    || builtin == BuiltInClipDistance
                    || builtin == BuiltInCullDistance
                    || builtin == BuiltInLayer
                    || builtin == BuiltInPrimitiveId
                    || builtin == BuiltInViewportIndex
                    || builtin == BuiltInCullPrimitiveEXT
                    || builtin == BuiltInPrimitiveShadingRateKHR
                    || builtin == BuiltInPrimitivePointIndicesEXT
                    || builtin == BuiltInPrimitiveLineIndicesEXT
                    || builtin == BuiltInPrimitiveTriangleIndicesEXT)
            {
                continue;
            }

            // If we need to emit 2 separate variables (for both input & output), we'll update this value
            let mut has_separate_input_output = false;
            let mut variable_index = 0;
            while variable_index < (if has_separate_input_output { 2 } else { 1 }) {
                let mut array_size: u32 = 0;
                let storage = if self.active_input_builtins.get(i) && variable_index == 0 {
                    StorageClassInput
                } else {
                    StorageClassOutput
                };
                let mut type_: Option<&str> = None;
                match builtin {
                    BuiltInFragCoord => type_ = Some("float4"),

                    BuiltInPosition => {
                        type_ = Some("float4");
                        if storage == StorageClassInput
                            && (self.get_execution_model() == ExecutionModelGeometry
                                || self.get_execution_model() == ExecutionModelTessellationControl)
                        {
                            array_size =
                                self.input_vertices_from_execution_mode(self.get_entry_point())?;
                        }
                    }

                    BuiltInFragDepth => type_ = Some("float"),

                    BuiltInVertexId | BuiltInVertexIndex | BuiltInInstanceIndex => {
                        type_ = Some("int");
                        if self.hlsl.options.support_nonzero_base_vertex_base_instance
                            || self.hlsl.options.shader_model >= 68
                        {
                            self.hlsl.base_vertex_info.used = true;
                        }
                    }

                    BuiltInBaseVertex | BuiltInBaseInstance => {
                        type_ = Some("int");
                        self.hlsl.base_vertex_info.used = true;
                    }

                    BuiltInInstanceId | BuiltInSampleId => type_ = Some("int"),

                    BuiltInPointSize => {
                        if self.hlsl.options.point_size_compat
                            || self.hlsl.options.shader_model <= 30
                        {
                            // Just emit the global variable, it will be ignored.
                            type_ = Some("float");
                        } else {
                            spirv_cross_throw!(join!("Unsupported builtin in HLSL: ", builtin));
                        }
                    }

                    BuiltInGlobalInvocationId | BuiltInLocalInvocationId | BuiltInWorkgroupId => {
                        type_ = Some("uint3")
                    }

                    BuiltInLocalInvocationIndex => type_ = Some("uint"),

                    BuiltInFrontFacing => type_ = Some("bool"),

                    BuiltInNumWorkgroups | BuiltInPointCoord => {
                        // Handled specially.
                    }

                    BuiltInSubgroupLocalInvocationId | BuiltInSubgroupSize => {
                        if self.hlsl.options.shader_model < 60 {
                            spirv_cross_throw!("Need SM 6.0 for Wave ops.");
                        }
                    }

                    BuiltInSubgroupEqMask
                    | BuiltInSubgroupLtMask
                    | BuiltInSubgroupLeMask
                    | BuiltInSubgroupGtMask
                    | BuiltInSubgroupGeMask => {
                        if self.hlsl.options.shader_model < 60 {
                            spirv_cross_throw!("Need SM 6.0 for Wave ops.");
                        }
                        type_ = Some("uint4");
                    }

                    BuiltInHelperInvocation => {
                        if self.hlsl.options.shader_model < 50 {
                            spirv_cross_throw!("Need SM 5.0 for Helper Invocation.");
                        }
                    }

                    BuiltInClipDistance => {
                        array_size = self.clip_distance_count;
                        type_ = Some("float");
                    }

                    BuiltInCullDistance => {
                        array_size = self.cull_distance_count;
                        type_ = Some("float");
                    }

                    BuiltInSampleMask => {
                        if storage == StorageClassInput {
                            type_ = Some(if sample_mask_in_basetype == BaseType::UInt {
                                "uint"
                            } else {
                                "int"
                            });
                        } else {
                            type_ = Some(if sample_mask_out_basetype == BaseType::UInt {
                                "uint"
                            } else {
                                "int"
                            });
                        }
                        array_size = 1;
                    }

                    BuiltInPrimitiveId | BuiltInViewIndex | BuiltInLayer => type_ = Some("uint"),

                    BuiltInViewportIndex
                    | BuiltInPrimitiveShadingRateKHR
                    | BuiltInPrimitiveLineIndicesEXT
                    | BuiltInCullPrimitiveEXT => type_ = Some("uint"),

                    BuiltInBaryCoordKHR | BuiltInBaryCoordNoPerspKHR => {
                        if self.hlsl.options.shader_model < 61 {
                            spirv_cross_throw!("Need SM 6.1 for barycentrics.");
                        }
                        type_ = Some("float3");
                    }

                    _ => spirv_cross_throw!(join!("Unsupported builtin in HLSL: ", builtin)),
                }

                if let Some(t) = type_ {
                    let builtin_name = self.builtin_to_glsl(builtin, storage)?;
                    if array_size != 0 {
                        statement!(
                            self,
                            "static ",
                            t,
                            " ",
                            builtin_name,
                            "[",
                            array_size,
                            "]",
                            init_expr,
                            ";"
                        );
                    } else {
                        statement!(self, "static ", t, " ", builtin_name, init_expr, ";");
                    }

                    if storage == StorageClassInput && self.active_output_builtins.get(i) {
                        let out_builtin_name = self.builtin_to_glsl(builtin, StorageClassOutput)?;
                        if out_builtin_name != builtin_name {
                            // If built-in name differs, we need to output it again
                            // (we reevaluate type and array size in case they are different)
                            has_separate_input_output = true;
                        }
                    }
                }
                variable_index += 1;
            }
        }

        if self.hlsl.base_vertex_info.used && self.hlsl.options.shader_model < 68 {
            let mut binding_info = String::new();
            let bvi = self.hlsl.base_vertex_info;
            if bvi.explicit_binding {
                binding_info = join!(" : register(b", bvi.register_index);
                if bvi.register_space != 0 {
                    binding_info += &join!(", space", bvi.register_space);
                }
                binding_info += ")";
            }
            statement!(self, "cbuffer SPIRV_Cross_VertexInfo", binding_info);
            self.begin_scope();
            statement!(self, "int SPIRV_Cross_BaseVertex;");
            statement!(self, "int SPIRV_Cross_BaseInstance;");
            self.end_scope_decl()?;
            statement!(self, "");
        }
        Ok(())
    }

    /// `CompilerHLSL::set_hlsl_aux_buffer_binding()`.
    pub fn set_hlsl_aux_buffer_binding(
        &mut self,
        binding: HLSLAuxBinding,
        register_index: u32,
        register_space: u32,
    ) {
        if binding == HLSL_AUX_BINDING_BASE_VERTEX_INSTANCE {
            self.hlsl.base_vertex_info.explicit_binding = true;
            self.hlsl.base_vertex_info.register_space = register_space;
            self.hlsl.base_vertex_info.register_index = register_index;
        }
    }

    /// `CompilerHLSL::unset_hlsl_aux_buffer_binding()`.
    pub fn unset_hlsl_aux_buffer_binding(&mut self, binding: HLSLAuxBinding) {
        if binding == HLSL_AUX_BINDING_BASE_VERTEX_INSTANCE {
            self.hlsl.base_vertex_info.explicit_binding = false;
        }
    }

    /// `CompilerHLSL::is_hlsl_aux_buffer_binding_used()`.
    pub fn is_hlsl_aux_buffer_binding_used(&self, binding: HLSLAuxBinding) -> bool {
        if binding == HLSL_AUX_BINDING_BASE_VERTEX_INSTANCE {
            self.hlsl.base_vertex_info.used
        } else {
            false
        }
    }

    pub(crate) fn emit_composite_constants(&mut self) -> Result<()> {
        // HLSL cannot declare structs or arrays inline, so we must move them out to
        // global constants directly.
        let mut emitted = false;

        for id in self.ir.typed_ids(TypeConstant) {
            let c = self.get_rc::<SPIRConstant>(id)?;
            if c.specialization {
                continue;
            }

            let type_ = self.get_rc::<SPIRType>(c.constant_type)?;

            if type_.basetype == BaseType::Struct && self.is_builtin_type(&type_) {
                continue;
            }

            if type_.basetype == BaseType::Struct || !type_.array.is_empty() {
                self.add_resource_name(c.self_)?;
                let name = self.to_name(c.self_, true)?;
                statement!(
                    self,
                    "static const ",
                    self.variable_decl_type(&type_, &name, 0)?,
                    " = ",
                    self.constant_expression(&c, false, false)?,
                    ";"
                );
                emitted = true;
            }
        }

        if emitted {
            statement!(self, "");
        }
        Ok(())
    }

    pub(crate) fn hlsl_emit_specialization_constants_and_structs(&mut self) -> Result<()> {
        let mut emitted = false;
        let (workgroup_size_id, _wg_x, _wg_y, _wg_z) =
            self.get_work_group_size_specialization_constants()?;

        let mut io_block_types: HashSet<TypeID> = HashSet::new();
        for id in self.ir.typed_ids(TypeVariable) {
            let var = self.get_rc::<SPIRVariable>(id)?;
            let type_ = self.get_rc::<SPIRType>(var.basetype)?;
            if (var.storage == StorageClassInput || var.storage == StorageClassOutput)
                && !var.remapped_variable
                && type_.pointer
                && !self.is_builtin_variable(&var)?
                && self.interface_variable_exists_in_entry_point(var.self_)?
                && self.has_decoration(type_.self_, DecorationBlock)
            {
                io_block_types.insert(type_.self_);
            }
        }

        for id_ in self.ir.ids_for_constant_undef_or_type.clone() {
            let id_type = self.id_type(id_);

            if id_type == TypeConstant {
                let c = self.get_rc::<SPIRConstant>(id_)?;

                if c.self_ == workgroup_size_id {
                    let wg = self.get_rc::<SPIRConstant>(workgroup_size_id)?;
                    statement!(
                        self,
                        "static const uint3 gl_WorkGroupSize = ",
                        self.constant_expression(&wg, false, false)?,
                        ";"
                    );
                    emitted = true;
                } else if c.specialization {
                    let type_ = self.get_rc::<SPIRType>(c.constant_type)?;
                    self.add_resource_name(c.self_)?;
                    let name = self.to_name(c.self_, true)?;

                    if self.has_decoration(c.self_, DecorationSpecId) {
                        // HLSL does not support specialization constants, so fallback to macros.
                        let macro_name = self.constant_value_macro_name(
                            self.get_decoration(c.self_, DecorationSpecId),
                        );
                        self.get_mut::<SPIRConstant>(id_)?
                            .specialization_constant_macro_name = macro_name.clone();
                        let c = self.get_rc::<SPIRConstant>(id_)?;

                        statement!(self, "#ifndef ", macro_name);
                        statement!(
                            self,
                            "#define ",
                            macro_name,
                            " ",
                            self.constant_expression(&c, false, false)?
                        );
                        statement!(self, "#endif");
                        statement!(
                            self,
                            "static const ",
                            self.variable_decl_type(&type_, &name, 0)?,
                            " = ",
                            macro_name,
                            ";"
                        );
                    } else {
                        statement!(
                            self,
                            "static const ",
                            self.variable_decl_type(&type_, &name, 0)?,
                            " = ",
                            self.constant_expression(&c, false, false)?,
                            ";"
                        );
                    }

                    emitted = true;
                }
            } else if id_type == TypeConstantOp {
                let c = self.get_rc::<SPIRConstantOp>(id_)?;
                let type_ = self.get_rc::<SPIRType>(c.basetype)?;
                self.add_resource_name(c.self_)?;
                let name = self.to_name(c.self_, true)?;
                statement!(
                    self,
                    "static const ",
                    self.variable_decl_type(&type_, &name, 0)?,
                    " = ",
                    self.constant_op_expression(&c)?,
                    ";"
                );
                emitted = true;
            } else if id_type == TypeType {
                let type_ = self.get_rc::<SPIRType>(id_)?;
                let is_non_io_block = self.has_decoration(type_.self_, DecorationBlock)
                    && !io_block_types.contains(&type_.self_);
                let is_buffer_block = self.has_decoration(type_.self_, DecorationBufferBlock);
                if type_.basetype == BaseType::Struct
                    && type_.array.is_empty()
                    && !type_.pointer
                    && !is_non_io_block
                    && !is_buffer_block
                {
                    if emitted {
                        statement!(self, "");
                    }
                    emitted = false;

                    self.emit_struct(id_)?;
                }
            } else if id_type == TypeUndef {
                let undef = self.get_rc::<SPIRUndef>(id_)?;
                let type_ = self.get_rc::<SPIRType>(undef.basetype)?;
                // OpUndef can be void for some reason ...
                // FIXME (upstream): this return leaves the whole function (it's a
                // loop, not a lambda), skipping the remaining declarations.
                if type_.basetype == BaseType::Void {
                    return Ok(());
                }

                let mut initializer = String::new();
                if self.glsl.options.force_zero_initialized_variables
                    && self.type_can_zero_initialize(&type_)?
                {
                    initializer =
                        join!(" = ", self.to_zero_initialized_expression(undef.basetype)?);
                }

                let name = self.to_name(undef.self_, true)?;
                statement!(
                    self,
                    "static ",
                    self.variable_decl_type(&type_, &name, undef.self_)?,
                    initializer,
                    ";"
                );
                emitted = true;
            }
        }

        if emitted {
            statement!(self, "");
        }
        Ok(())
    }

    pub(crate) fn hlsl_replace_illegal_names(&mut self) -> Result<()> {
        static KEYWORDS: &[&str] = &[
            // Additional HLSL specific keywords.
            // From https://docs.microsoft.com/en-US/windows/win32/direct3dhlsl/dx-graphics-hlsl-appendix-keywords
            "AppendStructuredBuffer",
            "asm",
            "asm_fragment",
            "BlendState",
            "bool",
            "break",
            "Buffer",
            "ByteAddressBuffer",
            "case",
            "cbuffer",
            "centroid",
            "class",
            "column_major",
            "compile",
            "compile_fragment",
            "CompileShader",
            "const",
            "continue",
            "ComputeShader",
            "ConsumeStructuredBuffer",
            "default",
            "DepthStencilState",
            "DepthStencilView",
            "discard",
            "do",
            "double",
            "DomainShader",
            "dword",
            "else",
            "export",
            "false",
            "float",
            "for",
            "fxgroup",
            "GeometryShader",
            "groupshared",
            "half",
            "HullShader",
            "indices",
            "if",
            "in",
            "inline",
            "inout",
            "InputPatch",
            "int",
            "interface",
            "line",
            "lineadj",
            "linear",
            "LineStream",
            "matrix",
            "min16float",
            "min10float",
            "min16int",
            "min16uint",
            "namespace",
            "nointerpolation",
            "noperspective",
            "NULL",
            "out",
            "OutputPatch",
            "payload",
            "packoffset",
            "pass",
            "pixelfragment",
            "PixelShader",
            "point",
            "PointStream",
            "precise",
            "RasterizerState",
            "RenderTargetView",
            "return",
            "register",
            "row_major",
            "RWBuffer",
            "RWByteAddressBuffer",
            "RWStructuredBuffer",
            "RWTexture1D",
            "RWTexture1DArray",
            "RWTexture2D",
            "RWTexture2DArray",
            "RWTexture3D",
            "sample",
            "sampler",
            "SamplerState",
            "SamplerComparisonState",
            "shared",
            "snorm",
            "stateblock",
            "stateblock_state",
            "static",
            "string",
            "struct",
            "switch",
            "StructuredBuffer",
            "tbuffer",
            "technique",
            "technique10",
            "technique11",
            "texture",
            "Texture1D",
            "Texture1DArray",
            "Texture2D",
            "Texture2DArray",
            "Texture2DMS",
            "Texture2DMSArray",
            "Texture3D",
            "TextureCube",
            "TextureCubeArray",
            "true",
            "typedef",
            "triangle",
            "triangleadj",
            "TriangleStream",
            "uint",
            "uniform",
            "unorm",
            "unsigned",
            "vector",
            "vertexfragment",
            "VertexShader",
            "vertices",
            "void",
            "volatile",
            "while",
            "signed",
        ];

        let keywords: HashSet<&'static str> = KEYWORDS.iter().copied().collect();
        self.replace_illegal_names_with(&keywords)?;
        self.glsl_replace_illegal_names()
    }

    pub(crate) fn hlsl_get_builtin_basetype(
        &mut self,
        builtin: BuiltIn,
        default_type: BaseType,
    ) -> BaseType {
        match builtin {
            BuiltInSampleMask => {
                // We declare sample mask array with module type, so always use default_type here.
                default_type
            }
            _ => self.glsl_get_builtin_basetype(builtin, default_type),
        }
    }

    pub(crate) fn hlsl_emit_resources(&mut self) -> Result<()> {
        let execution_model = self.get_entry_point().model;

        self.replace_illegal_names()?;

        match execution_model {
            ExecutionModelGeometry
            | ExecutionModelTessellationControl
            | ExecutionModelTessellationEvaluation
            | ExecutionModelMeshEXT => {
                self.fixup_implicit_builtin_block_names(execution_model)?;
            }

            _ => {}
        }

        self.hlsl_emit_specialization_constants_and_structs()?;
        self.emit_composite_constants()?;

        let mut emitted = false;

        // Output UBOs and SSBOs
        for id in self.ir.typed_ids(TypeVariable) {
            let var = self.get_rc::<SPIRVariable>(id)?;
            let type_ = self.get_rc::<SPIRType>(var.basetype)?;

            let is_block_storage =
                type_.storage == StorageClassStorageBuffer || type_.storage == StorageClassUniform;
            let has_block_flags = self
                .ir
                .get_decoration_bitset(type_.self_)
                .get(DecorationBlock)
                || self
                    .ir
                    .get_decoration_bitset(type_.self_)
                    .get(DecorationBufferBlock);

            if var.storage != StorageClassFunction
                && type_.pointer
                && is_block_storage
                && !self.is_hidden_variable(&var, false)?
                && has_block_flags
            {
                self.emit_buffer_block(&var)?;
                emitted = true;
            }
        }

        // Output push constant blocks
        for id in self.ir.typed_ids(TypeVariable) {
            let var = self.get_rc::<SPIRVariable>(id)?;
            let type_ = self.get_rc::<SPIRType>(var.basetype)?;
            if var.storage != StorageClassFunction
                && type_.pointer
                && type_.storage == StorageClassPushConstant
                && !self.is_hidden_variable(&var, false)?
            {
                self.emit_push_constant_block(&var)?;
                emitted = true;
            }
        }

        if execution_model == ExecutionModelVertex
            && self.hlsl.options.shader_model <= 30
            && self.active_output_builtins.get(BuiltInPosition)
        {
            statement!(self, "uniform float4 gl_HalfPixel;");
            emitted = true;
        }

        let skip_separate_image_sampler =
            !self.combined_image_samplers.is_empty() || self.hlsl.options.shader_model <= 30;

        // Output Uniform Constants (values, samplers, images, etc).
        for id in self.ir.typed_ids(TypeVariable) {
            let var = self.get_rc::<SPIRVariable>(id)?;
            let type_ = self.get_rc::<SPIRType>(var.basetype)?;

            // If we're remapping separate samplers and images, only emit the combined samplers.
            if skip_separate_image_sampler {
                // Sampler buffers are always used without a sampler, and they will also work in regular D3D.
                let sampler_buffer =
                    type_.basetype == BaseType::Image && type_.image.dim == DimBuffer;
                let separate_image = type_.basetype == BaseType::Image && type_.image.sampled == 1;
                let separate_sampler = type_.basetype == BaseType::Sampler;
                if !sampler_buffer && (separate_image || separate_sampler) {
                    continue;
                }
            }

            if var.storage != StorageClassFunction
                && !self.is_builtin_variable(&var)?
                && !var.remapped_variable
                && type_.pointer
                && (type_.storage == StorageClassUniformConstant
                    || type_.storage == StorageClassAtomicCounter)
                && !self.is_hidden_variable(&var, false)?
            {
                self.emit_uniform(&var)?;
                emitted = true;
            }
        }

        if emitted {
            statement!(self, "");
        }
        emitted = false;

        // Emit builtin input and output variables here.
        self.emit_builtin_variables()?;

        if execution_model != ExecutionModelMeshEXT {
            for id in self.ir.typed_ids(TypeVariable) {
                let var = self.get_rc::<SPIRVariable>(id)?;
                let type_ = self.get_rc::<SPIRType>(var.basetype)?;

                let is_hidden = self.is_hidden_io_variable(&var)?;

                if var.storage != StorageClassFunction
                    && !var.remapped_variable
                    && type_.pointer
                    && (var.storage == StorageClassInput || var.storage == StorageClassOutput)
                    && !self.is_builtin_variable(&var)?
                    && self.interface_variable_exists_in_entry_point(var.self_)?
                    && !is_hidden
                {
                    // Builtin variables are handled separately.
                    self.emit_interface_block_globally(&var)?;
                    emitted = true;
                }
            }
        }

        if emitted {
            statement!(self, "");
        }
        emitted = false;

        self.hlsl.require_input = false;
        self.hlsl.require_output = false;
        let mut active_inputs: HashSet<u32> = HashSet::new();
        let mut active_outputs: HashSet<u32> = HashSet::new();

        struct IOVariable {
            var: u32,
            location: u32,
            block_member_index: u32,
            block: bool,
            // The sort keys the C++ comparator computes.
            has_location: bool,
            name: String,
        }

        let mut input_variables: Vec<IOVariable> = Vec::new();
        let mut output_variables: Vec<IOVariable> = Vec::new();

        for id in self.ir.typed_ids(TypeVariable) {
            let var = self.get_rc::<SPIRVariable>(id)?;
            let type_ = self.get_rc::<SPIRType>(var.basetype)?;
            let block = self.has_decoration(type_.self_, DecorationBlock);

            if var.storage != StorageClassInput && var.storage != StorageClassOutput {
                continue;
            }

            let is_hidden = self.is_hidden_io_variable(&var)?;

            if !var.remapped_variable
                && type_.pointer
                && !self.is_builtin_variable(&var)?
                && self.interface_variable_exists_in_entry_point(var.self_)?
                && !is_hidden
            {
                let name = self.to_name(var.self_, true)?;
                if block {
                    for i in 0..type_.member_types.len() as u32 {
                        let location = self.get_declared_member_location(&var, i, false)?;
                        let v = IOVariable {
                            var: var.self_,
                            location,
                            block_member_index: i,
                            block: true,
                            has_location: true,
                            name: name.clone(),
                        };
                        if var.storage == StorageClassInput {
                            input_variables.push(v);
                        } else {
                            output_variables.push(v);
                        }
                    }
                } else {
                    let location = self.get_decoration(var.self_, DecorationLocation);
                    let v = IOVariable {
                        var: var.self_,
                        location,
                        block_member_index: 0,
                        block: false,
                        has_location: self.has_decoration(var.self_, DecorationLocation),
                        name,
                    };
                    if var.storage == StorageClassInput {
                        input_variables.push(v);
                    } else {
                        output_variables.push(v);
                    }
                }
            }
        }

        let variable_compare = |a: &IOVariable, b: &IOVariable| -> std::cmp::Ordering {
            use std::cmp::Ordering;
            // Sort input and output variables based on, from more robust to less robust:
            // - Location
            // - Variable has a location
            // - Name comparison
            // - Variable has a name
            // - Fallback: ID
            let less = |a: &IOVariable, b: &IOVariable| -> bool {
                let has_location_a = a.block || a.has_location;
                let has_location_b = b.block || b.has_location;

                if has_location_a && has_location_b {
                    return a.location < b.location;
                } else if has_location_a && !has_location_b {
                    return true;
                } else if !has_location_a && has_location_b {
                    return false;
                }

                let name1 = &a.name;
                let name2 = &b.name;

                if name1.is_empty() && name2.is_empty() {
                    return a.var < b.var;
                } else if name1.is_empty() {
                    return true;
                } else if name2.is_empty() {
                    return false;
                }

                name1.as_str() < name2.as_str()
            };
            if less(a, b) {
                Ordering::Less
            } else if less(b, a) {
                Ordering::Greater
            } else {
                Ordering::Equal
            }
        };

        let mut input_builtins = self.active_input_builtins.clone();
        input_builtins.clear(BuiltInNumWorkgroups);
        input_builtins.clear(BuiltInPointCoord);
        input_builtins.clear(BuiltInSubgroupSize);
        input_builtins.clear(BuiltInSubgroupLocalInvocationId);
        input_builtins.clear(BuiltInSubgroupEqMask);
        input_builtins.clear(BuiltInSubgroupLtMask);
        input_builtins.clear(BuiltInSubgroupLeMask);
        input_builtins.clear(BuiltInSubgroupGtMask);
        input_builtins.clear(BuiltInSubgroupGeMask);

        if !input_variables.is_empty() || !input_builtins.empty() {
            self.hlsl.require_input = true;
            statement!(self, "struct SPIRV_Cross_Input");

            self.begin_scope();
            input_variables.sort_by(variable_compare);
            for var in &input_variables {
                let v = self.get_rc::<SPIRVariable>(var.var)?;
                if var.block {
                    self.emit_interface_block_member_in_struct(
                        &v,
                        var.block_member_index,
                        var.location,
                        &mut active_inputs,
                    )?;
                } else {
                    self.emit_interface_block_in_struct(&v, &mut active_inputs)?;
                }
            }
            self.emit_builtin_inputs_in_struct()?;
            self.end_scope_decl()?;
            statement!(self, "");
        }

        let is_mesh_shader = execution_model == ExecutionModelMeshEXT;
        if !output_variables.is_empty() || !self.active_output_builtins.empty() {
            output_variables.sort_by(variable_compare);
            self.hlsl.require_output =
                !(is_mesh_shader || execution_model == ExecutionModelGeometry);

            statement!(
                self,
                if is_mesh_shader {
                    "struct gl_MeshPerVertexEXT"
                } else {
                    "struct SPIRV_Cross_Output"
                }
            );
            self.begin_scope();
            for var in &output_variables {
                let v = self.get_rc::<SPIRVariable>(var.var)?;
                if self.is_per_primitive_variable(&v)? {
                    continue;
                }
                if var.block && is_mesh_shader && var.block_member_index != 0 {
                    continue;
                }
                if var.block && !is_mesh_shader {
                    self.emit_interface_block_member_in_struct(
                        &v,
                        var.block_member_index,
                        var.location,
                        &mut active_outputs,
                    )?;
                } else {
                    self.emit_interface_block_in_struct(&v, &mut active_outputs)?;
                }
            }
            self.emit_builtin_outputs_in_struct()?;
            if !is_mesh_shader {
                self.emit_builtin_primitive_outputs_in_struct()?;
            }
            self.end_scope_decl()?;
            statement!(self, "");

            if is_mesh_shader {
                statement!(self, "struct gl_MeshPerPrimitiveEXT");
                self.begin_scope();
                for var in &output_variables {
                    let v = self.get_rc::<SPIRVariable>(var.var)?;
                    if !self.is_per_primitive_variable(&v)? {
                        continue;
                    }
                    if var.block && var.block_member_index != 0 {
                        continue;
                    }

                    self.emit_interface_block_in_struct(&v, &mut active_outputs)?;
                }
                self.emit_builtin_primitive_outputs_in_struct()?;
                self.end_scope_decl()?;
                statement!(self, "");
            }
        }

        // Global variables.
        for global in self.global_variables.clone() {
            let var = self.get_rc::<SPIRVariable>(global)?;
            if self.is_hidden_variable(&var, true)? {
                continue;
            }

            if var.storage == StorageClassTaskPayloadWorkgroupEXT && is_mesh_shader {
                continue;
            }

            if var.storage != StorageClassOutput && !self.variable_is_lut(&var) {
                self.add_resource_name(var.self_)?;

                let storage = match var.storage {
                    StorageClassWorkgroup | StorageClassTaskPayloadWorkgroupEXT => "groupshared",

                    _ => "static",
                };

                let mut initializer = String::new();
                if self.glsl.options.force_zero_initialized_variables
                    && var.storage == StorageClassPrivate
                    && var.initializer == 0
                    && var.static_expression == 0
                    && self.type_can_zero_initialize(self.get_variable_data_type(&var)?)?
                {
                    initializer = join!(
                        " = ",
                        self.to_zero_initialized_expression(self.get_variable_data_type_id(&var)?)?
                    );
                }
                statement!(
                    self,
                    storage,
                    " ",
                    self.variable_decl(&var)?,
                    initializer,
                    ";"
                );

                emitted = true;
            }
        }

        if emitted {
            statement!(self, "");
        }

        if self.hlsl.requires_op_fmod {
            static TYPES: [&str; 4] = ["float", "float2", "float3", "float4"];

            for type_ in TYPES.iter() {
                statement!(self, type_, " mod(", type_, " x, ", type_, " y)");
                self.begin_scope();
                statement!(self, "return x - y * floor(x / y);");
                self.end_scope()?;
                statement!(self, "");
            }
        }

        self.emit_texture_size_variants(
            self.hlsl.required_texture_size_variants.srv,
            "4",
            false,
            "",
        )?;
        for norm in 0..3 {
            for comp in 0..4 {
                static QUALIFIERS: [&str; 3] = ["", "unorm ", "snorm "];
                static VECSIZES: [&str; 4] = ["", "2", "3", "4"];
                self.emit_texture_size_variants(
                    self.hlsl.required_texture_size_variants.uav[norm as usize][comp as usize],
                    VECSIZES[comp as usize],
                    true,
                    QUALIFIERS[norm as usize],
                )?;
            }
        }

        if self.hlsl.requires_fp16_packing {
            // HLSL does not pack into a single word sadly :(
            statement!(self, "uint spvPackHalf2x16(float2 value)");
            self.begin_scope();
            statement!(self, "uint2 Packed = f32tof16(value);");
            statement!(self, "return Packed.x | (Packed.y << 16);");
            self.end_scope()?;
            statement!(self, "");

            statement!(self, "float2 spvUnpackHalf2x16(uint value)");
            self.begin_scope();
            statement!(self, "return f16tof32(uint2(value & 0xffff, value >> 16));");
            self.end_scope()?;
            statement!(self, "");
        }

        if self.hlsl.requires_uint2_packing {
            statement!(self, "uint64_t spvPackUint2x32(uint2 value)");
            self.begin_scope();
            statement!(
                self,
                "return (uint64_t(value.y) << 32) | uint64_t(value.x);"
            );
            self.end_scope()?;
            statement!(self, "");

            statement!(self, "uint2 spvUnpackUint2x32(uint64_t value)");
            self.begin_scope();
            statement!(self, "uint2 Unpacked;");
            statement!(self, "Unpacked.x = uint(value & 0xffffffff);");
            statement!(self, "Unpacked.y = uint(value >> 32);");
            statement!(self, "return Unpacked;");
            self.end_scope()?;
            statement!(self, "");
        }

        if self.hlsl.requires_explicit_fp16_packing {
            // HLSL does not pack into a single word sadly :(
            statement!(self, "uint spvPackFloat2x16(min16float2 value)");
            self.begin_scope();
            statement!(self, "uint2 Packed = f32tof16(value);");
            statement!(self, "return Packed.x | (Packed.y << 16);");
            self.end_scope()?;
            statement!(self, "");

            statement!(self, "min16float2 spvUnpackFloat2x16(uint value)");
            self.begin_scope();
            statement!(
                self,
                "return min16float2(f16tof32(uint2(value & 0xffff, value >> 16)));"
            );
            self.end_scope()?;
            statement!(self, "");
        }

        // HLSL does not seem to have builtins for these operation, so roll them by hand ...
        if self.hlsl.requires_unorm8_packing {
            statement!(self, "uint spvPackUnorm4x8(float4 value)");
            self.begin_scope();
            statement!(
                self,
                "uint4 Packed = uint4(round(saturate(value) * 255.0));"
            );
            statement!(
                self,
                "return Packed.x | (Packed.y << 8) | (Packed.z << 16) | (Packed.w << 24);"
            );
            self.end_scope()?;
            statement!(self, "");

            statement!(self, "float4 spvUnpackUnorm4x8(uint value)");
            self.begin_scope();
            statement!(self, "uint4 Packed = uint4(value & 0xff, (value >> 8) & 0xff, (value >> 16) & 0xff, value >> 24);");
            statement!(self, "return float4(Packed) / 255.0;");
            self.end_scope()?;
            statement!(self, "");
        }

        if self.hlsl.requires_snorm8_packing {
            statement!(self, "uint spvPackSnorm4x8(float4 value)");
            self.begin_scope();
            statement!(
                self,
                "int4 Packed = int4(round(clamp(value, -1.0, 1.0) * 127.0)) & 0xff;"
            );
            statement!(
                self,
                "return uint(Packed.x | (Packed.y << 8) | (Packed.z << 16) | (Packed.w << 24));"
            );
            self.end_scope()?;
            statement!(self, "");

            statement!(self, "float4 spvUnpackSnorm4x8(uint value)");
            self.begin_scope();
            statement!(self, "int SignedValue = int(value);");
            statement!(self, "int4 Packed = int4(SignedValue << 24, SignedValue << 16, SignedValue << 8, SignedValue) >> 24;");
            statement!(self, "return clamp(float4(Packed) / 127.0, -1.0, 1.0);");
            self.end_scope()?;
            statement!(self, "");
        }

        if self.hlsl.requires_unorm16_packing {
            statement!(self, "uint spvPackUnorm2x16(float2 value)");
            self.begin_scope();
            statement!(
                self,
                "uint2 Packed = uint2(round(saturate(value) * 65535.0));"
            );
            statement!(self, "return Packed.x | (Packed.y << 16);");
            self.end_scope()?;
            statement!(self, "");

            statement!(self, "float2 spvUnpackUnorm2x16(uint value)");
            self.begin_scope();
            statement!(self, "uint2 Packed = uint2(value & 0xffff, value >> 16);");
            statement!(self, "return float2(Packed) / 65535.0;");
            self.end_scope()?;
            statement!(self, "");
        }

        if self.hlsl.requires_snorm16_packing {
            statement!(self, "uint spvPackSnorm2x16(float2 value)");
            self.begin_scope();
            statement!(
                self,
                "int2 Packed = int2(round(clamp(value, -1.0, 1.0) * 32767.0)) & 0xffff;"
            );
            statement!(self, "return uint(Packed.x | (Packed.y << 16));");
            self.end_scope()?;
            statement!(self, "");

            statement!(self, "float2 spvUnpackSnorm2x16(uint value)");
            self.begin_scope();
            statement!(self, "int SignedValue = int(value);");
            statement!(
                self,
                "int2 Packed = int2(SignedValue << 16, SignedValue) >> 16;"
            );
            statement!(self, "return clamp(float2(Packed) / 32767.0, -1.0, 1.0);");
            self.end_scope()?;
            statement!(self, "");
        }

        if self.hlsl.requires_bitfield_insert {
            static TYPES: [&str; 4] = ["uint", "uint2", "uint3", "uint4"];
            for type_ in TYPES.iter() {
                statement!(
                    self,
                    type_,
                    " spvBitfieldInsert(",
                    type_,
                    " Base, ",
                    type_,
                    " Insert, uint Offset, uint Count)"
                );
                self.begin_scope();
                statement!(self, "uint Mask = Count == 32 ? 0xffffffff : (((1u << Count) - 1) << (Offset & 31));");
                statement!(self, "return (Base & ~Mask) | ((Insert << Offset) & Mask);");
                self.end_scope()?;
                statement!(self, "");
            }
        }

        if self.hlsl.requires_bitfield_extract {
            static UNSIGNED_TYPES: [&str; 4] = ["uint", "uint2", "uint3", "uint4"];
            for type_ in UNSIGNED_TYPES.iter() {
                statement!(
                    self,
                    type_,
                    " spvBitfieldUExtract(",
                    type_,
                    " Base, uint Offset, uint Count)"
                );
                self.begin_scope();
                statement!(
                    self,
                    "uint Mask = Count == 32 ? 0xffffffff : ((1 << Count) - 1);"
                );
                statement!(self, "return (Base >> Offset) & Mask;");
                self.end_scope()?;
                statement!(self, "");
            }

            // In this overload, we will have to do sign-extension, which we will emulate by shifting up and down.
            static SIGNED_TYPES: [&str; 4] = ["int", "int2", "int3", "int4"];
            for type_ in SIGNED_TYPES.iter() {
                statement!(
                    self,
                    type_,
                    " spvBitfieldSExtract(",
                    type_,
                    " Base, int Offset, int Count)"
                );
                self.begin_scope();
                statement!(self, "int Mask = Count == 32 ? -1 : ((1 << Count) - 1);");
                statement!(self, type_, " Masked = (Base >> Offset) & Mask;");
                statement!(self, "int ExtendShift = (32 - Count) & 31;");
                statement!(self, "return (Masked << ExtendShift) >> ExtendShift;");
                self.end_scope()?;
                statement!(self, "");
            }
        }

        if self.hlsl.requires_inverse_2x2 {
            statement!(self, "// Returns the inverse of a matrix, by using the algorithm of calculating the classical");
            statement!(self, "// adjoint and dividing by the determinant. The contents of the matrix are changed.");
            statement!(self, "float2x2 spvInverse(float2x2 m)");
            self.begin_scope();
            statement!(
                self,
                "float2x2 adj;	// The adjoint matrix (inverse after dividing by determinant)"
            );
            statement_no_indent!(self, "");
            statement!(
                self,
                "// Create the transpose of the cofactors, as the classical adjoint of the matrix."
            );
            statement!(self, "adj[0][0] =  m[1][1];");
            statement!(self, "adj[0][1] = -m[0][1];");
            statement_no_indent!(self, "");
            statement!(self, "adj[1][0] = -m[1][0];");
            statement!(self, "adj[1][1] =  m[0][0];");
            statement_no_indent!(self, "");
            statement!(
                self,
                "// Calculate the determinant as a combination of the cofactors of the first row."
            );
            statement!(
                self,
                "float det = (adj[0][0] * m[0][0]) + (adj[0][1] * m[1][0]);"
            );
            statement_no_indent!(self, "");
            statement!(
                self,
                "// Divide the classical adjoint matrix by the determinant."
            );
            statement!(
                self,
                "// If determinant is zero, matrix is not invertable, so leave it unchanged."
            );
            statement!(self, "return (det != 0.0f) ? (adj * (1.0f / det)) : m;");
            self.end_scope()?;
            statement!(self, "");
        }

        if self.hlsl.requires_inverse_3x3 {
            statement!(self, "// Returns the determinant of a 2x2 matrix.");
            statement!(
                self,
                "float spvDet2x2(float a1, float a2, float b1, float b2)"
            );
            self.begin_scope();
            statement!(self, "return a1 * b2 - b1 * a2;");
            self.end_scope()?;
            statement_no_indent!(self, "");
            statement!(self, "// Returns the inverse of a matrix, by using the algorithm of calculating the classical");
            statement!(self, "// adjoint and dividing by the determinant. The contents of the matrix are changed.");
            statement!(self, "float3x3 spvInverse(float3x3 m)");
            self.begin_scope();
            statement!(
                self,
                "float3x3 adj;	// The adjoint matrix (inverse after dividing by determinant)"
            );
            statement_no_indent!(self, "");
            statement!(
                self,
                "// Create the transpose of the cofactors, as the classical adjoint of the matrix."
            );
            statement!(
                self,
                "adj[0][0] =  spvDet2x2(m[1][1], m[1][2], m[2][1], m[2][2]);"
            );
            statement!(
                self,
                "adj[0][1] = -spvDet2x2(m[0][1], m[0][2], m[2][1], m[2][2]);"
            );
            statement!(
                self,
                "adj[0][2] =  spvDet2x2(m[0][1], m[0][2], m[1][1], m[1][2]);"
            );
            statement_no_indent!(self, "");
            statement!(
                self,
                "adj[1][0] = -spvDet2x2(m[1][0], m[1][2], m[2][0], m[2][2]);"
            );
            statement!(
                self,
                "adj[1][1] =  spvDet2x2(m[0][0], m[0][2], m[2][0], m[2][2]);"
            );
            statement!(
                self,
                "adj[1][2] = -spvDet2x2(m[0][0], m[0][2], m[1][0], m[1][2]);"
            );
            statement_no_indent!(self, "");
            statement!(
                self,
                "adj[2][0] =  spvDet2x2(m[1][0], m[1][1], m[2][0], m[2][1]);"
            );
            statement!(
                self,
                "adj[2][1] = -spvDet2x2(m[0][0], m[0][1], m[2][0], m[2][1]);"
            );
            statement!(
                self,
                "adj[2][2] =  spvDet2x2(m[0][0], m[0][1], m[1][0], m[1][1]);"
            );
            statement_no_indent!(self, "");
            statement!(
                self,
                "// Calculate the determinant as a combination of the cofactors of the first row."
            );
            statement!(self, "float det = (adj[0][0] * m[0][0]) + (adj[0][1] * m[1][0]) + (adj[0][2] * m[2][0]);");
            statement_no_indent!(self, "");
            statement!(
                self,
                "// Divide the classical adjoint matrix by the determinant."
            );
            statement!(
                self,
                "// If determinant is zero, matrix is not invertable, so leave it unchanged."
            );
            statement!(self, "return (det != 0.0f) ? (adj * (1.0f / det)) : m;");
            self.end_scope()?;
            statement!(self, "");
        }

        if self.hlsl.requires_inverse_4x4 {
            if !self.hlsl.requires_inverse_3x3 {
                statement!(self, "// Returns the determinant of a 2x2 matrix.");
                statement!(
                    self,
                    "float spvDet2x2(float a1, float a2, float b1, float b2)"
                );
                self.begin_scope();
                statement!(self, "return a1 * b2 - b1 * a2;");
                self.end_scope()?;
                statement!(self, "");
            }

            statement!(self, "// Returns the determinant of a 3x3 matrix.");
            statement!(self, "float spvDet3x3(float a1, float a2, float a3, float b1, float b2, float b3, float c1, float c2, float c3)");
            self.begin_scope();
            statement!(self, "return a1 * spvDet2x2(b2, b3, c2, c3) - b1 * spvDet2x2(a2, a3, c2, c3) + c1 * spvDet2x2(a2, a3, b2, b3);");
            self.end_scope()?;
            statement_no_indent!(self, "");
            statement!(self, "// Returns the inverse of a matrix, by using the algorithm of calculating the classical");
            statement!(self, "// adjoint and dividing by the determinant. The contents of the matrix are changed.");
            statement!(self, "float4x4 spvInverse(float4x4 m)");
            self.begin_scope();
            statement!(
                self,
                "float4x4 adj;	// The adjoint matrix (inverse after dividing by determinant)"
            );
            statement_no_indent!(self, "");
            statement!(
                self,
                "// Create the transpose of the cofactors, as the classical adjoint of the matrix."
            );
            statement!(self, "adj[0][0] =  spvDet3x3(m[1][1], m[1][2], m[1][3], m[2][1], m[2][2], m[2][3], m[3][1], m[3][2], m[3][3]);");
            statement!(self, "adj[0][1] = -spvDet3x3(m[0][1], m[0][2], m[0][3], m[2][1], m[2][2], m[2][3], m[3][1], m[3][2], m[3][3]);");
            statement!(self, "adj[0][2] =  spvDet3x3(m[0][1], m[0][2], m[0][3], m[1][1], m[1][2], m[1][3], m[3][1], m[3][2], m[3][3]);");
            statement!(self, "adj[0][3] = -spvDet3x3(m[0][1], m[0][2], m[0][3], m[1][1], m[1][2], m[1][3], m[2][1], m[2][2], m[2][3]);");
            statement_no_indent!(self, "");
            statement!(self, "adj[1][0] = -spvDet3x3(m[1][0], m[1][2], m[1][3], m[2][0], m[2][2], m[2][3], m[3][0], m[3][2], m[3][3]);");
            statement!(self, "adj[1][1] =  spvDet3x3(m[0][0], m[0][2], m[0][3], m[2][0], m[2][2], m[2][3], m[3][0], m[3][2], m[3][3]);");
            statement!(self, "adj[1][2] = -spvDet3x3(m[0][0], m[0][2], m[0][3], m[1][0], m[1][2], m[1][3], m[3][0], m[3][2], m[3][3]);");
            statement!(self, "adj[1][3] =  spvDet3x3(m[0][0], m[0][2], m[0][3], m[1][0], m[1][2], m[1][3], m[2][0], m[2][2], m[2][3]);");
            statement_no_indent!(self, "");
            statement!(self, "adj[2][0] =  spvDet3x3(m[1][0], m[1][1], m[1][3], m[2][0], m[2][1], m[2][3], m[3][0], m[3][1], m[3][3]);");
            statement!(self, "adj[2][1] = -spvDet3x3(m[0][0], m[0][1], m[0][3], m[2][0], m[2][1], m[2][3], m[3][0], m[3][1], m[3][3]);");
            statement!(self, "adj[2][2] =  spvDet3x3(m[0][0], m[0][1], m[0][3], m[1][0], m[1][1], m[1][3], m[3][0], m[3][1], m[3][3]);");
            statement!(self, "adj[2][3] = -spvDet3x3(m[0][0], m[0][1], m[0][3], m[1][0], m[1][1], m[1][3], m[2][0], m[2][1], m[2][3]);");
            statement_no_indent!(self, "");
            statement!(self, "adj[3][0] = -spvDet3x3(m[1][0], m[1][1], m[1][2], m[2][0], m[2][1], m[2][2], m[3][0], m[3][1], m[3][2]);");
            statement!(self, "adj[3][1] =  spvDet3x3(m[0][0], m[0][1], m[0][2], m[2][0], m[2][1], m[2][2], m[3][0], m[3][1], m[3][2]);");
            statement!(self, "adj[3][2] = -spvDet3x3(m[0][0], m[0][1], m[0][2], m[1][0], m[1][1], m[1][2], m[3][0], m[3][1], m[3][2]);");
            statement!(self, "adj[3][3] =  spvDet3x3(m[0][0], m[0][1], m[0][2], m[1][0], m[1][1], m[1][2], m[2][0], m[2][1], m[2][2]);");
            statement_no_indent!(self, "");
            statement!(
                self,
                "// Calculate the determinant as a combination of the cofactors of the first row."
            );
            statement!(self, "float det = (adj[0][0] * m[0][0]) + (adj[0][1] * m[1][0]) + (adj[0][2] * m[2][0]) + (adj[0][3] * m[3][0]);");
            statement_no_indent!(self, "");
            statement!(
                self,
                "// Divide the classical adjoint matrix by the determinant."
            );
            statement!(
                self,
                "// If determinant is zero, matrix is not invertable, so leave it unchanged."
            );
            statement!(self, "return (det != 0.0f) ? (adj * (1.0f / det)) : m;");
            self.end_scope()?;
            statement!(self, "");
        }

        if self.hlsl.requires_scalar_reflect {
            // FP16/FP64? No templates in HLSL.
            statement!(self, "float spvReflect(float i, float n)");
            self.begin_scope();
            statement!(self, "return i - 2.0 * dot(n, i) * n;");
            self.end_scope()?;
            statement!(self, "");
        }

        if self.hlsl.requires_scalar_refract {
            // FP16/FP64? No templates in HLSL.
            statement!(self, "float spvRefract(float i, float n, float eta)");
            self.begin_scope();
            statement!(self, "float NoI = n * i;");
            statement!(self, "float NoI2 = NoI * NoI;");
            statement!(self, "float k = 1.0 - eta * eta * (1.0 - NoI2);");
            statement!(self, "if (k < 0.0)");
            self.begin_scope();
            statement!(self, "return 0.0;");
            self.end_scope()?;
            statement!(self, "else");
            self.begin_scope();
            statement!(self, "return eta * i - (eta * NoI + sqrt(k)) * n;");
            self.end_scope()?;
            self.end_scope()?;
            statement!(self, "");
        }

        if self.hlsl.requires_scalar_faceforward {
            // FP16/FP64? No templates in HLSL.
            statement!(self, "float spvFaceForward(float n, float i, float nref)");
            self.begin_scope();
            statement!(self, "return i * nref < 0.0 ? n : -n;");
            self.end_scope()?;
            statement!(self, "");
        }

        for type_id in self.hlsl.composite_selection_workaround_types.clone() {
            // Need out variable since HLSL does not support returning arrays.
            let type_ = self.get_rc::<SPIRType>(type_id)?;
            let type_str = self.type_to_glsl(&type_, 0)?;
            let type_arr_str = self.type_to_array_glsl(&type_, 0)?;
            statement!(
                self,
                "void spvSelectComposite(out ",
                type_str,
                " out_value",
                type_arr_str,
                ", bool cond, ",
                type_str,
                " true_val",
                type_arr_str,
                ", ",
                type_str,
                " false_val",
                type_arr_str,
                ")"
            );
            self.begin_scope();
            statement!(self, "if (cond)");
            self.begin_scope();
            statement!(self, "out_value = true_val;");
            self.end_scope()?;
            statement!(self, "else");
            self.begin_scope();
            statement!(self, "out_value = false_val;");
            self.end_scope()?;
            self.end_scope()?;
            statement!(self, "");
        }

        if is_mesh_shader && self.glsl.options.vertex.flip_vert_y {
            statement!(self, "float4 spvFlipVertY(float4 v)");
            self.begin_scope();
            statement!(self, "return float4(v.x, -v.y, v.z, v.w);");
            self.end_scope()?;
            statement!(self, "");
            statement!(self, "float spvFlipVertY(float v)");
            self.begin_scope();
            statement!(self, "return -v;");
            self.end_scope()?;
            statement!(self, "");
        }
        Ok(())
    }

    pub(crate) fn emit_texture_size_variants(
        &mut self,
        variant_mask: u64,
        vecsize_qualifier: &str,
        uav: bool,
        type_qualifier: &str,
    ) -> Result<()> {
        if variant_mask == 0 {
            return Ok(());
        }

        static TYPES: [&str; QueryTypeCount as usize] = ["float", "int", "uint"];
        static DIMS: [&str; QueryDimCount as usize] = [
            "Texture1D",
            "Texture1DArray",
            "Texture2D",
            "Texture2DArray",
            "Texture3D",
            "Buffer",
            "TextureCube",
            "TextureCubeArray",
            "Texture2DMS",
            "Texture2DMSArray",
        ];

        static HAS_LOD: [bool; QueryDimCount as usize] = [
            true, true, true, true, true, false, true, true, false, false,
        ];

        static RET_TYPES: [&str; QueryDimCount as usize] = [
            "uint", "uint2", "uint2", "uint3", "uint3", "uint", "uint2", "uint3", "uint2", "uint3",
        ];

        static RETURN_ARGUMENTS: [u32; QueryDimCount as usize] = [1, 2, 2, 3, 3, 1, 2, 3, 2, 3];

        for index in 0..QueryDimCount as usize {
            for type_index in 0..QueryTypeCount as usize {
                let bit = 16 * type_index + index;
                let mask = 1u64 << bit;

                if (variant_mask & mask) == 0 {
                    continue;
                }

                statement!(
                    self,
                    RET_TYPES[index],
                    " spv",
                    if uav { "Image" } else { "Texture" },
                    "Size(",
                    if uav { "RW" } else { "" },
                    DIMS[index],
                    "<",
                    type_qualifier,
                    TYPES[type_index],
                    vecsize_qualifier,
                    "> Tex, ",
                    if uav { "" } else { "uint Level, " },
                    "out uint Param)"
                );
                self.begin_scope();
                statement!(self, RET_TYPES[index], " ret;");
                match RETURN_ARGUMENTS[index] {
                    1 => {
                        if HAS_LOD[index] && !uav {
                            statement!(self, "Tex.GetDimensions(Level, ret.x, Param);");
                        } else {
                            statement!(self, "Tex.GetDimensions(ret.x);");
                            statement!(self, "Param = 0u;");
                        }
                    }
                    2 => {
                        if HAS_LOD[index] && !uav {
                            statement!(self, "Tex.GetDimensions(Level, ret.x, ret.y, Param);");
                        } else if !uav {
                            statement!(self, "Tex.GetDimensions(ret.x, ret.y, Param);");
                        } else {
                            statement!(self, "Tex.GetDimensions(ret.x, ret.y);");
                            statement!(self, "Param = 0u;");
                        }
                    }
                    3 => {
                        if HAS_LOD[index] && !uav {
                            statement!(
                                self,
                                "Tex.GetDimensions(Level, ret.x, ret.y, ret.z, Param);"
                            );
                        } else if !uav {
                            statement!(self, "Tex.GetDimensions(ret.x, ret.y, ret.z, Param);");
                        } else {
                            statement!(self, "Tex.GetDimensions(ret.x, ret.y, ret.z);");
                            statement!(self, "Param = 0u;");
                        }
                    }
                    _ => {}
                }

                statement!(self, "return ret;");
                self.end_scope()?;
                statement!(self, "");
            }
        }
        Ok(())
    }

    pub(crate) fn analyze_meshlet_writes(&mut self) -> Result<()> {
        let mut id_per_vertex: u32 = 0;
        let mut id_per_primitive: u32 = 0;
        let mut need_per_primitive = false;
        let mut need_per_vertex = false;

        for id in self.ir.typed_ids(TypeVariable) {
            let var = self.get_rc::<SPIRVariable>(id)?;
            let type_self = self.get::<SPIRType>(var.basetype)?.self_;
            let block = self.has_decoration(type_self, DecorationBlock);
            if var.storage == StorageClassOutput && block && self.is_builtin_variable(&var)? {
                let flags = self.get_buffer_block_flags(var.self_)?;
                if flags.get(DecorationPerPrimitiveEXT) {
                    id_per_primitive = var.self_;
                } else {
                    id_per_vertex = var.self_;
                }
            } else if var.storage == StorageClassOutput {
                let flags = if block {
                    self.get_buffer_block_flags(var.self_)?
                } else {
                    self.get_decoration_bitset(var.self_).clone()
                };

                if flags.get(DecorationPerPrimitiveEXT) {
                    need_per_primitive = true;
                } else {
                    need_per_vertex = true;
                }
            }
        }

        // If we have per-primitive outputs, and no per-primitive builtins,
        // empty version of gl_MeshPerPrimitiveEXT will be emitted.
        // If we don't use block IO for vertex output, we'll also need to synthesize the PerVertex block.

        let generate_block = |this: &mut Compiler,
                              block_name: &str,
                              instance_name: &str,
                              per_primitive: bool|
         -> Result<u32> {
            let output_primitives = this.get_entry_point().output_primitives;
            let output_vertices = this.get_entry_point().output_vertices;

            let op_type = this.ir.increase_bound_by(4)?;
            let op_arr = op_type + 1;
            let op_ptr = op_type + 2;
            let op_var = op_type + 3;

            let type_ = this.set::<SPIRType>(op_type, SPIRType::new(OpTypeStruct))?;
            type_.basetype = BaseType::Struct;
            let type_copy = type_.clone();
            this.set_name(op_type, block_name);
            this.set_decoration(op_type, DecorationBlock, 0);
            if per_primitive {
                this.set_decoration(op_type, DecorationPerPrimitiveEXT, 0);
            }

            let arr = this.set::<SPIRType>(op_arr, type_copy.clone())?;
            arr.op = OpTypeArray;
            arr.parent_type = type_copy.self_;
            arr.array.push(if per_primitive {
                output_primitives
            } else {
                output_vertices
            });
            arr.array_size_literal.push(true);
            let arr_copy = arr.clone();

            let ptr = this.set::<SPIRType>(op_ptr, arr_copy.clone())?;
            ptr.parent_type = arr_copy.self_;
            ptr.op = OpTypePointer;
            ptr.pointer = true;
            ptr.pointer_depth += 1;
            ptr.storage = StorageClassOutput;
            this.set_decoration(op_ptr, DecorationBlock, 0);
            this.set_name(op_ptr, block_name);

            let var = this
                .set::<SPIRVariable>(op_var, SPIRVariable::new(op_ptr, StorageClassOutput, 0, 0))?;
            let var_self = var.self_;
            if per_primitive {
                this.set_decoration(op_var, DecorationPerPrimitiveEXT, 0);
            }
            this.set_name(op_var, instance_name);
            this.get_entry_point_mut()
                .interface_variables
                .push(var_self);

            Ok(op_var)
        };

        if id_per_vertex == 0 && need_per_vertex {
            id_per_vertex =
                generate_block(self, "gl_MeshPerVertexEXT", "gl_MeshVerticesEXT", false)?;
        }
        if id_per_primitive == 0 && need_per_primitive {
            id_per_primitive =
                generate_block(self, "gl_MeshPerPrimitiveEXT", "gl_MeshPrimitivesEXT", true)?;
        }

        let mut processed_func_ids: HashSet<u32> = HashSet::new();
        let entry = self.ir.default_entry_point;
        self.analyze_meshlet_writes_func(
            entry,
            id_per_vertex,
            id_per_primitive,
            &mut processed_func_ids,
        )
    }

    pub(crate) fn analyze_meshlet_writes_func(
        &mut self,
        func_id: u32,
        id_per_vertex: u32,
        id_per_primitive: u32,
        processed_func_ids: &mut HashSet<u32>,
    ) -> Result<()> {
        // Avoid processing a function more than once
        if processed_func_ids.contains(&func_id) {
            return Ok(());
        }
        processed_func_ids.insert(func_id);

        let blocks = self.get::<SPIRFunction>(func_id)?.blocks.clone();
        // Recursively establish global args added to functions on which we depend.
        for block in blocks {
            let b = self.get_rc::<SPIRBlock>(block)?;
            for i in &b.ops {
                let ops = self.stream(i)?;
                let op = i.op as Op;

                match op {
                    OpFunctionCall => {
                        // Then recurse into the function itself to extract globals used internally in the function
                        let inner_func_id = ops[2];
                        self.analyze_meshlet_writes_func(
                            inner_func_id,
                            id_per_vertex,
                            id_per_primitive,
                            processed_func_ids,
                        )?;
                        let inner_args = self.get::<SPIRFunction>(inner_func_id)?.arguments.clone();
                        for iarg in &inner_args {
                            if !iarg.alias_global_variable {
                                continue;
                            }

                            let already_declared = self
                                .get::<SPIRFunction>(func_id)?
                                .arguments
                                .iter()
                                .any(|arg| arg.id == iarg.id);

                            if !already_declared {
                                // basetype is effectively ignored here since we declare the argument
                                // with explicit types. Just pass down a valid type.
                                let type_ = self.expression_type_id(iarg.id)?;
                                self.get_mut::<SPIRFunction>(func_id)?
                                    .arguments
                                    .push(Parameter {
                                        type_,
                                        id: iarg.id,
                                        read_count: iarg.read_count,
                                        write_count: iarg.write_count,
                                        alias_global_variable: true,
                                    });
                            }
                        }
                    }

                    OpStore
                    | OpLoad
                    | OpInBoundsAccessChain
                    | OpAccessChain
                    | OpPtrAccessChain
                    | OpInBoundsPtrAccessChain
                    | OpArrayLength => {
                        let var = self
                            .maybe_get_rc::<SPIRVariable>(ops[if op == OpStore { 0 } else { 2 }]);
                        if let Some(var) = var.filter(|v| {
                            v.storage == StorageClassOutput
                                || v.storage == StorageClassTaskPayloadWorkgroupEXT
                        }) {
                            let builtin_type =
                                self.get_decoration(var.self_, DecorationBuiltIn) as BuiltIn;

                            let mut var_id = var.self_;
                            if var.storage != StorageClassTaskPayloadWorkgroupEXT
                                && builtin_type != BuiltInPrimitivePointIndicesEXT
                                && builtin_type != BuiltInPrimitiveLineIndicesEXT
                                && builtin_type != BuiltInPrimitiveTriangleIndicesEXT
                            {
                                var_id = if self.is_per_primitive_variable(&var)? {
                                    id_per_primitive
                                } else {
                                    id_per_vertex
                                };
                            }

                            let already_declared = self
                                .get::<SPIRFunction>(func_id)?
                                .arguments
                                .iter()
                                .any(|arg| arg.id == var_id);

                            if !already_declared {
                                // basetype is effectively ignored here since we declare the argument
                                // with explicit types. Just pass down a valid type.
                                let type_id = self.expression_type_id(var_id)?;
                                let write_count =
                                    if var.storage == StorageClassTaskPayloadWorkgroupEXT {
                                        0
                                    } else {
                                        1
                                    };
                                self.get_mut::<SPIRFunction>(func_id)?
                                    .arguments
                                    .push(Parameter {
                                        type_: type_id,
                                        id: var_id,
                                        read_count: 1,
                                        write_count,
                                        alias_global_variable: true,
                                    });
                            }
                        }
                    }

                    _ => {}
                }
            }
        }
        Ok(())
    }

    pub(crate) fn hlsl_layout_for_member(
        &mut self,
        type_: &SPIRType,
        index: u32,
    ) -> Result<String> {
        let flags = self.get_member_decoration_bitset(type_.self_, index);

        // HLSL can emit row_major or column_major decoration in any struct.
        // Do not try to merge combined decorations for children like in GLSL.

        // Flip the convention. HLSL is a bit odd in that the memory layout is column major ... but the language API is "row-major".
        // The way to deal with this is to multiply everything in inverse order, and reverse the memory layout.
        if flags.get(DecorationColMajor) {
            return Ok("row_major ".into());
        } else if flags.get(DecorationRowMajor) {
            return Ok("column_major ".into());
        }

        Ok(String::new())
    }

    pub(crate) fn hlsl_emit_struct_member(
        &mut self,
        type_: &SPIRType,
        member_type_id: u32,
        index: u32,
        qualifier: &str,
        base_offset: u32,
    ) -> Result<()> {
        let membertype = self.get_rc::<SPIRType>(member_type_id)?;

        let memb = self
            .meta_mut(type_.self_)
            .members
            .get(index as usize)
            .cloned();

        let mut packing_offset = String::new();
        let is_push_constant = type_.storage == StorageClassPushConstant;

        if (self.has_extended_decoration(type_.self_, SPIRVCrossDecorationExplicitOffset)
            || is_push_constant)
            && self.has_member_decoration(type_.self_, index, DecorationOffset)
        {
            let memb_offset = memb.as_ref().map_or(0, |m| m.offset);
            let offset = memb_offset.wrapping_sub(base_offset);
            if offset & 3 != 0 {
                spirv_cross_throw!("Cannot pack on tighter bounds than 4 bytes in HLSL.");
            }

            static PACKING_SWIZZLE: [&str; 4] = ["", ".y", ".z", ".w"];
            packing_offset = join!(
                " : packoffset(c",
                offset / 16,
                PACKING_SWIZZLE[((offset & 15) >> 2) as usize],
                ")"
            );
        }

        let member_name = self.to_member_name(type_, index)?;
        statement!(
            self,
            self.layout_for_member(type_, index)?,
            qualifier,
            self.variable_decl_type(&membertype, &member_name, 0)?,
            packing_offset,
            ";"
        );
        Ok(())
    }

    pub(crate) fn emit_rayquery_function(
        &mut self,
        commited: &str,
        candidate: &str,
        ops: &[u32],
    ) -> Result<()> {
        self.flush_variable_declaration(ops[0])?;
        let is_commited = self.evaluate_constant_u32(ops[3])?;
        let expr = join!(
            self.to_expression(ops[2], true)?,
            if is_commited != 0 {
                commited
            } else {
                candidate
            }
        );
        self.emit_op(ops[0], ops[1], &expr, false, false)
    }

    pub(crate) fn hlsl_emit_mesh_tasks(&mut self, block: &SPIRBlock) -> Result<()> {
        if block.mesh.payload != 0 {
            statement!(
                self,
                "DispatchMesh(",
                self.to_unpacked_expression(block.mesh.groups[0], true)?,
                ", ",
                self.to_unpacked_expression(block.mesh.groups[1], true)?,
                ", ",
                self.to_unpacked_expression(block.mesh.groups[2], true)?,
                ", ",
                self.to_unpacked_expression(block.mesh.payload, true)?,
                ");"
            );
        } else {
            spirv_cross_throw!("Amplification shader in HLSL must have payload");
        }
        Ok(())
    }

    pub(crate) fn emit_geometry_stream_append(&mut self) -> Result<()> {
        self.begin_scope();
        statement!(self, "SPIRV_Cross_Output stage_output;");

        for i in self.active_output_builtins.bits() {
            if i == BuiltInPointSize && self.hlsl.options.shader_model > 30 {
                continue;
            }
            match i as BuiltIn {
                BuiltInClipDistance => {
                    for clip in 0..self.clip_distance_count {
                        statement!(
                            self,
                            "stage_output.gl_ClipDistance",
                            clip / 4,
                            ".",
                            b"xyzw"[(clip & 3) as usize] as char,
                            " = gl_ClipDistance[",
                            clip,
                            "];"
                        );
                    }
                }
                BuiltInCullDistance => {
                    for cull in 0..self.cull_distance_count {
                        statement!(
                            self,
                            "stage_output.gl_CullDistance",
                            cull / 4,
                            ".",
                            b"xyzw"[(cull & 3) as usize] as char,
                            " = gl_CullDistance[",
                            cull,
                            "];"
                        );
                    }
                }
                BuiltInSampleMask => {
                    statement!(self, "stage_output.gl_SampleMask = gl_SampleMask[0];");
                }
                _ => {
                    let builtin_expr = self.builtin_to_glsl(i as BuiltIn, StorageClassOutput)?;
                    statement!(
                        self,
                        "stage_output.",
                        builtin_expr,
                        " = ",
                        builtin_expr,
                        ";"
                    );
                }
            }
        }

        for id in self.ir.typed_ids(TypeVariable) {
            let var = self.get_rc::<SPIRVariable>(id)?;
            let type_ = self.get_rc::<SPIRType>(var.basetype)?;
            let block = self.has_decoration(type_.self_, DecorationBlock);

            if var.storage != StorageClassOutput {
                continue;
            }

            if !var.remapped_variable
                && type_.pointer
                && !self.is_builtin_variable(&var)?
                && self.interface_variable_exists_in_entry_point(var.self_)?
            {
                if block {
                    let type_name = self.to_name(type_.self_, true)?;
                    let var_name = self.to_name(var.self_, true)?;
                    for mbr_idx in 0..type_.member_types.len() as u32 {
                        let mbr_name = self.to_member_name(&type_, mbr_idx)?;
                        let flat_name = join!(type_name, "_", mbr_name);
                        statement!(
                            self,
                            "stage_output.",
                            flat_name,
                            " = ",
                            var_name,
                            ".",
                            mbr_name,
                            ";"
                        );
                    }
                } else {
                    let name = self.to_name(var.self_, true)?;
                    if self.hlsl.options.shader_model <= 30
                        && self.get_entry_point().model == ExecutionModelFragment
                    {
                        let mut output_filler = String::new();
                        let mut size = type_.vecsize;
                        while size < 4 {
                            output_filler += ", 0.0";
                            size += 1;
                        }
                        statement!(
                            self,
                            "stage_output.",
                            name,
                            " = float4(",
                            name,
                            output_filler,
                            ");"
                        );
                    } else {
                        statement!(self, "stage_output.", name, " = ", name, ";");
                    }
                }
            }
        }

        statement!(self, "geometry_stream.Append(stage_output);");
        self.end_scope()
    }

    pub(crate) fn hlsl_emit_buffer_block(&mut self, var: &SPIRVariable) -> Result<()> {
        let type_ = self.get_rc::<SPIRType>(var.basetype)?;

        let is_uav = var.storage == StorageClassStorageBuffer
            || self.has_decoration(type_.self_, DecorationBufferBlock);

        if self.glsl.flattened_buffer_blocks.contains(&var.self_) {
            self.emit_buffer_block_flattened(var)?;
        } else if is_uav {
            let flags = self.ir.get_buffer_block_flags(var)?;
            let is_readonly = flags.get(DecorationNonWritable)
                && !self.is_hlsl_force_storage_buffer_as_uav(var.self_);
            let is_coherent = flags.get(DecorationCoherent) && !is_readonly;
            let is_interlocked = self.interlocked_resources.contains(&var.self_);

            let to_structuredbuffer_subtype_name = |this: &mut Compiler,
                                                    parent_type: &SPIRType|
             -> Result<String> {
                if parent_type.basetype == BaseType::Struct && parent_type.member_types.len() == 1 {
                    // Use type of first struct member as a StructuredBuffer will have only one '._m0' field in SPIR-V
                    let member0_type = this.get_rc::<SPIRType>(parent_type.member_types[0])?;
                    this.type_to_glsl(&member0_type, 0)
                } else {
                    // Otherwise, this StructuredBuffer only has a basic subtype, e.g. StructuredBuffer<int>
                    this.type_to_glsl(parent_type, 0)
                }
            };

            let type_name = if self.is_user_type_structured(var.self_)? {
                join!(
                    if is_readonly {
                        ""
                    } else if is_interlocked {
                        "RasterizerOrdered"
                    } else {
                        "RW"
                    },
                    "StructuredBuffer<",
                    to_structuredbuffer_subtype_name(self, &type_)?,
                    ">"
                )
            } else if is_readonly {
                "ByteAddressBuffer".to_string()
            } else if is_interlocked {
                "RasterizerOrderedByteAddressBuffer".to_string()
            } else {
                "RWByteAddressBuffer".to_string()
            };

            self.add_resource_name(var.self_)?;
            statement!(
                self,
                if is_coherent { "globallycoherent " } else { "" },
                type_name,
                " ",
                self.to_name(var.self_, true)?,
                self.type_to_array_glsl(&type_, var.self_)?,
                self.to_resource_binding(var)?,
                ";"
            );
        } else if type_.array.is_empty() {
            // Flatten the top-level struct so we can use packoffset,
            // this restriction is similar to GLSL where layout(offset) is not possible on sub-structs.
            self.glsl.flattened_structs.insert(var.self_, false);

            // Prefer the block name if possible.
            let mut buffer_name = self.to_name(type_.self_, false)?;
            if self.meta_mut(type_.self_).decoration.alias.is_empty()
                || self.glsl.resource_names.contains(&buffer_name)
                || self.glsl.block_names.contains(&buffer_name)
            {
                buffer_name = self.get_block_fallback_name(var.self_)?;
            }

            {
                let g = &mut *self.glsl;
                Self::add_variable(&mut g.block_names, &g.resource_names, &mut buffer_name);
            }

            // If for some reason buffer_name is an illegal name, make a final fallback to a workaround name.
            // This cannot conflict with anything else, so we're safe now.
            if buffer_name.is_empty() {
                buffer_name = join!(
                    "_",
                    self.get::<SPIRType>(var.basetype)?.self_,
                    "_",
                    var.self_
                );
            }

            let mut failed_index: u32 = 0;
            if self.buffer_is_packing_standard(
                &type_,
                BufferPackingHLSLCbufferPackOffset,
                Some(&mut failed_index),
                0,
                u32::MAX,
            )? {
                self.set_extended_decoration(type_.self_, SPIRVCrossDecorationExplicitOffset, 0);
            } else {
                spirv_cross_throw!(join!(
                    "cbuffer ID ",
                    var.self_,
                    " (name: ",
                    buffer_name,
                    "), member index ",
                    failed_index,
                    " (name: ",
                    self.to_member_name(&type_, failed_index)?,
                    ") cannot be expressed with either HLSL packing layout or packoffset."
                ));
            }

            self.glsl.block_names.insert(buffer_name.clone());

            // Save for post-reflection later.
            self.declared_block_names
                .insert(var.self_, buffer_name.clone());

            self.get_mut::<SPIRType>(var.basetype)?
                .member_name_cache
                .clear();
            // var.self can be used as a backup name for the block name,
            // so we need to make sure we don't disturb the name here on a recompile.
            // It will need to be reset if we have to recompile.
            self.preserve_alias_on_reset(var.self_);
            self.add_resource_name(var.self_)?;
            statement!(
                self,
                "cbuffer ",
                buffer_name,
                self.to_resource_binding(var)?
            );
            self.begin_scope();

            let type_id = var.basetype;
            for (i, &member) in type_.member_types.iter().enumerate() {
                let i = i as u32;
                self.add_member_name(type_id, i)?;
                let backup_name = self.get_member_name(type_.self_, i).clone();
                let t = self.get_rc::<SPIRType>(type_id)?;
                let mut member_name = self.to_member_name(&t, i)?;
                member_name = join!(self.to_name(var.self_, true)?, "_", member_name);
                ParsedIR::sanitize_underscores(&mut member_name);
                self.set_member_name(type_.self_, i, &member_name)?;
                let t = self.get_rc::<SPIRType>(type_id)?;
                self.emit_struct_member(&t, member, i, "", 0)?;
                self.set_member_name(type_.self_, i, &backup_name)?;
            }

            self.end_scope_decl()?;
            statement!(self, "");
        } else {
            if self.hlsl.options.shader_model < 51 {
                spirv_cross_throw!(
                    "Need ConstantBuffer<T> to use arrays of UBOs, but this is only supported in SM 5.1."
                );
            }

            self.add_resource_name(type_.self_)?;
            self.add_resource_name(var.self_)?;

            // ConstantBuffer<T> does not support packoffset, so it is unuseable unless everything aligns as we expect.
            let mut failed_index: u32 = 0;
            if !self.buffer_is_packing_standard(
                &type_,
                BufferPackingHLSLCbuffer,
                Some(&mut failed_index),
                0,
                u32::MAX,
            )? {
                spirv_cross_throw!(join!(
                    "HLSL ConstantBuffer<T> ID ",
                    var.self_,
                    " (name: ",
                    self.to_name(type_.self_, true)?,
                    "), member index ",
                    failed_index,
                    " (name: ",
                    self.to_member_name(&type_, failed_index)?,
                    ") cannot be expressed with normal HLSL packing rules."
                ));
            }

            self.emit_struct(type_.self_)?;
            statement!(
                self,
                "ConstantBuffer<",
                self.to_name(type_.self_, true)?,
                "> ",
                self.to_name(var.self_, true)?,
                self.type_to_array_glsl(&type_, var.self_)?,
                self.to_resource_binding(var)?,
                ";"
            );
        }
        Ok(())
    }

    pub(crate) fn hlsl_emit_push_constant_block(&mut self, var: &SPIRVariable) -> Result<()> {
        if self.glsl.flattened_buffer_blocks.contains(&var.self_) {
            self.emit_buffer_block_flattened(var)?;
        } else if self.hlsl.root_constants_layout.is_empty() {
            self.emit_buffer_block(var)?;
        } else {
            for layout in self.hlsl.root_constants_layout.clone() {
                let type_ = self.get_rc::<SPIRType>(var.basetype)?;

                let mut failed_index: u32 = 0;
                if self.buffer_is_packing_standard(
                    &type_,
                    BufferPackingHLSLCbufferPackOffset,
                    Some(&mut failed_index),
                    layout.start,
                    layout.end,
                )? {
                    self.set_extended_decoration(
                        type_.self_,
                        SPIRVCrossDecorationExplicitOffset,
                        0,
                    );
                } else {
                    spirv_cross_throw!(join!(
                        "Root constant cbuffer ID ",
                        var.self_,
                        " (name: ",
                        self.to_name(type_.self_, true)?,
                        ")",
                        ", member index ",
                        failed_index,
                        " (name: ",
                        self.to_member_name(&type_, failed_index)?,
                        ") cannot be expressed with either HLSL packing layout or packoffset."
                    ));
                }

                self.glsl.flattened_structs.insert(var.self_, false);
                self.get_mut::<SPIRType>(var.basetype)?
                    .member_name_cache
                    .clear();
                self.add_resource_name(var.self_)?;
                let memb = self.meta_mut(type_.self_).members.clone();

                statement!(
                    self,
                    "cbuffer SPIRV_CROSS_RootConstant_",
                    self.to_name(var.self_, true)?,
                    self.to_resource_register(
                        HLSL_BINDING_AUTO_PUSH_CONSTANT_BIT,
                        'b',
                        layout.binding,
                        layout.space
                    )?
                );
                self.begin_scope();

                // Index of the next field in the generated root constant constant buffer
                let mut constant_index = 0u32;

                // Iterate over all member of the push constant and check which of the fields
                // fit into the given root constant layout.
                let type_id = var.basetype;
                for (i, m) in memb.iter().enumerate() {
                    let i = i as u32;
                    let offset = m.offset;
                    if layout.start <= offset && offset < layout.end {
                        let Some(&member) = type_.member_types.get(i as usize) else {
                            spirv_cross_throw!("Member index is out of range.");
                        };

                        self.add_member_name(type_id, constant_index)?;
                        let backup_name = self.get_member_name(type_.self_, i).clone();
                        let t = self.get_rc::<SPIRType>(type_id)?;
                        let mut member_name = self.to_member_name(&t, i)?;
                        member_name = join!(self.to_name(var.self_, true)?, "_", member_name);
                        ParsedIR::sanitize_underscores(&mut member_name);
                        self.set_member_name(type_.self_, constant_index, &member_name)?;
                        let t = self.get_rc::<SPIRType>(type_id)?;
                        self.emit_struct_member(&t, member, i, "", layout.start)?;
                        self.set_member_name(type_.self_, constant_index, &backup_name)?;

                        constant_index += 1;
                    }
                }

                self.end_scope_decl()?;
            }
        }
        Ok(())
    }

    pub(crate) fn hlsl_to_sampler_expression(&mut self, id: u32) -> Result<String> {
        let mut expr = join!("_", self.to_non_uniform_aware_expression(id)?);
        match expr.find('[') {
            None => Ok(expr + "_sampler"),
            Some(index) => {
                // We have an expression like _ident[array], so we cannot tack on _sampler, insert it inside the string instead.
                expr.insert_str(index, "_sampler");
                Ok(expr)
            }
        }
    }

    pub(crate) fn hlsl_emit_sampled_image_op(
        &mut self,
        result_type: u32,
        result_id: u32,
        image_id: u32,
        samp_id: u32,
    ) -> Result<()> {
        if self.hlsl.options.shader_model >= 40 && self.combined_image_samplers.is_empty() {
            self.set::<SPIRCombinedImageSampler>(
                result_id,
                SPIRCombinedImageSampler::new(result_type, image_id, samp_id),
            )?;
        } else {
            // Make sure to suppress usage tracking. It is illegal to create temporaries of opaque types.
            let expr = self.to_combined_image_sampler(image_id, samp_id)?;
            self.emit_op(result_type, result_id, &expr, true, true)?;
        }
        Ok(())
    }

    pub(crate) fn hlsl_to_func_call_arg(&mut self, arg: &Parameter, id: u32) -> Result<String> {
        let mut arg_str = self.glsl_to_func_call_arg(arg, id)?;

        if self.hlsl.options.shader_model <= 30 {
            return Ok(arg_str);
        }

        // Manufacture automatic sampler arg if the arg is a SampledImage texture and we're in modern HLSL.
        let type_ = self.expression_type(id)?;

        // We don't have to consider combined image samplers here via OpSampledImage because
        // those variables cannot be passed as arguments to functions.
        // Only global SampledImage variables may be used as arguments.
        if type_.basetype == BaseType::SampledImage && type_.image.dim != DimBuffer {
            arg_str += &(", ".to_string() + &self.hlsl_to_sampler_expression(id)?);
        }

        Ok(arg_str)
    }

    pub(crate) fn get_inner_entry_point_name(&self) -> Result<String> {
        let execution = self.get_entry_point();

        if self.hlsl.options.use_entry_point_name {
            let mut name = join!(execution.name, "_inner");
            ParsedIR::sanitize_underscores(&mut name);
            return Ok(name);
        }

        Ok(if execution.model == ExecutionModelVertex {
            "vert_main"
        } else if execution.model == ExecutionModelFragment {
            "frag_main"
        } else if execution.model == ExecutionModelGLCompute {
            "comp_main"
        } else if execution.model == ExecutionModelGeometry {
            "geom_main"
        } else if execution.model == ExecutionModelMeshEXT {
            "mesh_main"
        } else if execution.model == ExecutionModelTaskEXT {
            "task_main"
        } else {
            spirv_cross_throw!("Unsupported execution model.");
        }
        .into())
    }

    pub(crate) fn input_vertices_from_execution_mode(
        &self,
        execution: &SPIREntryPoint,
    ) -> Result<u32> {
        let input_vertices = if execution.flags.get(ExecutionModeInputLines) {
            2
        } else if execution.flags.get(ExecutionModeInputLinesAdjacency) {
            4
        } else if execution.flags.get(ExecutionModeInputTrianglesAdjacency) {
            6
        } else if execution.flags.get(ExecutionModeTriangles) {
            3
        } else if execution.flags.get(ExecutionModeInputPoints) {
            1
        } else {
            spirv_cross_throw!("Unsupported execution model.");
        };
        Ok(input_vertices)
    }

    pub(crate) fn hlsl_emit_function_prototype(
        &mut self,
        func_id: u32,
        return_flags: &Bitset,
    ) -> Result<()> {
        let func = self.get_rc::<SPIRFunction>(func_id)?;
        if func.self_ != self.ir.default_entry_point {
            self.add_function_overload(&func)?;
        }

        // Avoid shadow declarations.
        self.glsl.local_variable_names = self.glsl.resource_names.clone();

        let mut decl = String::new();

        let type_ = self.get_rc::<SPIRType>(func.return_type)?;
        if type_.array.is_empty() {
            decl += &self.flags_to_qualifiers_glsl(&type_, 0, return_flags)?;
            decl += &self.type_to_glsl(&type_, 0)?;
            decl += " ";
        } else {
            // We cannot return arrays in HLSL, so "return" through an out variable.
            decl = "void ".into();
        }

        if func.self_ == self.ir.default_entry_point {
            decl += &self.get_inner_entry_point_name()?;
            self.glsl.processing_entry_point = true;
        } else {
            decl += &self.to_name(func.self_, true)?;
        }

        decl += "(";
        let mut arglist: Vec<String> = Vec::new();

        if !type_.array.is_empty() {
            // Fake array returns by writing to an out array instead.
            let mut out_argument = String::new();
            out_argument += "out ";
            out_argument += &self.type_to_glsl(&type_, 0)?;
            out_argument += " ";
            out_argument += "spvReturnValue";
            out_argument += &self.type_to_array_glsl(&type_, 0)?;
            arglist.push(out_argument);
        }

        for (index, arg) in func.arguments.iter().enumerate() {
            // Do not pass in separate images or samplers if we're remapping
            // to combined image samplers.
            if self.skip_argument(arg.id)? {
                continue;
            }

            // Might change the variable name if it already exists in this function.
            // SPIRV OpName doesn't have any semantic effect, so it's valid for an implementation
            // to use same name for variables.
            // Since we want to make the GLSL debuggable and somewhat sane, use fallback names for variables which are duplicates.
            self.add_local_variable_name(arg.id)?;

            arglist.push(self.argument_decl(arg)?);

            // Flatten a combined sampler to two separate arguments in modern HLSL.
            let arg_type = self.get_rc::<SPIRType>(arg.type_)?;
            if self.hlsl.options.shader_model > 30
                && arg_type.basetype == BaseType::SampledImage
                && arg_type.image.dim != DimBuffer
            {
                // Manufacture automatic sampler arg for SampledImage texture
                arglist.push(join!(
                    if self.is_depth_image(&arg_type, arg.id) {
                        "SamplerComparisonState "
                    } else {
                        "SamplerState "
                    },
                    self.hlsl_to_sampler_expression(arg.id)?,
                    self.type_to_array_glsl(&arg_type, arg.id)?
                ));
            }

            // Hold a pointer to the parameter so we can invalidate the readonly field if needed.
            if let Some(var) = self.maybe_get_mut::<SPIRVariable>(arg.id) {
                var.parameter = Some(ParameterRef {
                    function: func_id,
                    index,
                    shadow: false,
                });
            }
        }

        for (index, arg) in func.shadow_arguments.iter().enumerate() {
            // Might change the variable name if it already exists in this function.
            // SPIRV OpName doesn't have any semantic effect, so it's valid for an implementation
            // to use same name for variables.
            // Since we want to make the GLSL debuggable and somewhat sane, use fallback names for variables which are duplicates.
            self.add_local_variable_name(arg.id)?;

            arglist.push(self.argument_decl(arg)?);

            // Hold a pointer to the parameter so we can invalidate the readonly field if needed.
            if let Some(var) = self.maybe_get_mut::<SPIRVariable>(arg.id) {
                var.parameter = Some(ParameterRef {
                    function: func_id,
                    index,
                    shadow: true,
                });
            }
        }

        if (func.self_ == self.ir.default_entry_point || func.emits_geometry)
            && self.get_entry_point().model == ExecutionModelGeometry
        {
            let execution = self.get_entry_point();

            let input_vertices = self.input_vertices_from_execution_mode(execution)?;

            let prim = if execution.flags.get(ExecutionModeInputLinesAdjacency) {
                "lineadj"
            } else if execution.flags.get(ExecutionModeInputLines) {
                "line"
            } else if execution.flags.get(ExecutionModeInputTrianglesAdjacency) {
                "triangleadj"
            } else if execution.flags.get(ExecutionModeTriangles) {
                "triangle"
            } else {
                "point"
            };

            let stream_type = if execution.flags.get(ExecutionModeOutputPoints) {
                "PointStream"
            } else if execution.flags.get(ExecutionModeOutputLineStrip) {
                "LineStream"
            } else {
                "TriangleStream"
            };

            if func.self_ == self.ir.default_entry_point {
                arglist.push(join!(
                    prim,
                    " SPIRV_Cross_Input stage_input[",
                    input_vertices,
                    "]"
                ));
            }
            arglist.push(join!(
                "inout ",
                stream_type,
                "<SPIRV_Cross_Output> ",
                "geometry_stream"
            ));
        }

        decl += &merge_default(&arglist);
        decl += ")";
        statement!(self, decl);
        Ok(())
    }

    pub(crate) fn emit_hlsl_entry_point(&mut self) -> Result<()> {
        let mut arguments: Vec<String> = Vec::new();

        if self.hlsl.require_input && self.get_entry_point().model != ExecutionModelGeometry {
            arguments.push("SPIRV_Cross_Input stage_input".into());
        }

        let execution = self.get_entry_point().clone();

        let mut input_vertices: u32 = 1;

        match execution.model {
            ExecutionModelGeometry => {
                input_vertices = self.input_vertices_from_execution_mode(&execution)?;

                let prim = if execution.flags.get(ExecutionModeInputLinesAdjacency) {
                    "lineadj"
                } else if execution.flags.get(ExecutionModeInputLines) {
                    "line"
                } else if execution.flags.get(ExecutionModeInputTrianglesAdjacency) {
                    "triangleadj"
                } else if execution.flags.get(ExecutionModeTriangles) {
                    "triangle"
                } else {
                    "point"
                };

                let stream_type = if execution.flags.get(ExecutionModeOutputPoints) {
                    "PointStream"
                } else if execution.flags.get(ExecutionModeOutputLineStrip) {
                    "LineStream"
                } else {
                    "TriangleStream"
                };

                statement!(self, "[maxvertexcount(", execution.output_vertices, ")]");
                arguments.push(join!(
                    prim,
                    " SPIRV_Cross_Input stage_input[",
                    input_vertices,
                    "]"
                ));
                if self.active_input_builtins.get(BuiltInPrimitiveId) {
                    arguments.push("uint gl_PrimitiveID : SV_PrimitiveID".into());
                }
                arguments.push(join!(
                    "inout ",
                    stream_type,
                    "<SPIRV_Cross_Output> ",
                    "geometry_stream"
                ));
            }
            ExecutionModelTaskEXT | ExecutionModelMeshEXT | ExecutionModelGLCompute => {
                if execution.model == ExecutionModelMeshEXT {
                    if execution.flags.get(ExecutionModeOutputTrianglesEXT) {
                        statement!(self, "[outputtopology(\"triangle\")]");
                    } else if execution.flags.get(ExecutionModeOutputLinesEXT) {
                        statement!(self, "[outputtopology(\"line\")]");
                    } else if execution.flags.get(ExecutionModeOutputPoints) {
                        spirv_cross_throw!("Topology mode \"points\" is not supported in DirectX");
                    }

                    let func = self.get_rc::<SPIRFunction>(self.ir.default_entry_point)?;
                    for arg in &func.arguments {
                        let var = self.get_rc::<SPIRVariable>(arg.id)?;
                        let base_type_self = self.get::<SPIRType>(var.basetype)?.self_;
                        let block = self.has_decoration(base_type_self, DecorationBlock);
                        if var.storage == StorageClassTaskPayloadWorkgroupEXT {
                            arguments.push("in payload ".to_string() + &self.variable_decl(&var)?);
                        } else if block {
                            let flags = self.get_buffer_block_flags(var.self_)?;
                            if flags.get(DecorationPerPrimitiveEXT)
                                || self.has_decoration(arg.id, DecorationPerPrimitiveEXT)
                            {
                                arguments.push(join!(
                                    "out primitives gl_MeshPerPrimitiveEXT gl_MeshPrimitivesEXT[",
                                    execution.output_primitives,
                                    "]"
                                ));
                            } else {
                                arguments.push(join!(
                                    "out vertices gl_MeshPerVertexEXT gl_MeshVerticesEXT[",
                                    execution.output_vertices,
                                    "]"
                                ));
                            }
                        } else if execution.flags.get(ExecutionModeOutputTrianglesEXT) {
                            arguments.push(join!(
                                "out indices uint3 gl_PrimitiveTriangleIndicesEXT[",
                                execution.output_primitives,
                                "]"
                            ));
                        } else {
                            arguments.push(join!(
                                "out indices uint2 gl_PrimitiveLineIndicesEXT[",
                                execution.output_primitives,
                                "]"
                            ));
                        }
                    }
                }
                let (_, wg_x, wg_y, wg_z) = self.get_work_group_size_specialization_constants()?;

                let mut x = execution.workgroup_size.x;
                let mut y = execution.workgroup_size.y;
                let mut z = execution.workgroup_size.z;

                if execution.workgroup_size.constant == 0
                    && execution.flags.get(ExecutionModeLocalSizeId)
                {
                    if execution.workgroup_size.id_x != 0 {
                        x = self
                            .get::<SPIRConstant>(execution.workgroup_size.id_x)?
                            .scalar(0, 0);
                    }
                    if execution.workgroup_size.id_y != 0 {
                        y = self
                            .get::<SPIRConstant>(execution.workgroup_size.id_y)?
                            .scalar(0, 0);
                    }
                    if execution.workgroup_size.id_z != 0 {
                        z = self
                            .get::<SPIRConstant>(execution.workgroup_size.id_z)?
                            .scalar(0, 0);
                    }
                }

                let x_expr = if wg_x.id != 0 {
                    self.get::<SPIRConstant>(wg_x.id)?
                        .specialization_constant_macro_name
                        .clone()
                } else {
                    x.to_string()
                };
                let y_expr = if wg_y.id != 0 {
                    self.get::<SPIRConstant>(wg_y.id)?
                        .specialization_constant_macro_name
                        .clone()
                } else {
                    y.to_string()
                };
                let z_expr = if wg_z.id != 0 {
                    self.get::<SPIRConstant>(wg_z.id)?
                        .specialization_constant_macro_name
                        .clone()
                } else {
                    z.to_string()
                };

                statement!(
                    self,
                    "[numthreads(",
                    x_expr,
                    ", ",
                    y_expr,
                    ", ",
                    z_expr,
                    ")]"
                );
            }
            ExecutionModelFragment => {
                if execution.flags.get(ExecutionModeEarlyFragmentTests) {
                    statement!(self, "[earlydepthstencil]");
                }
            }
            _ => {}
        }

        let entry_point_name = if self.hlsl.options.use_entry_point_name {
            self.get_entry_point().name.clone()
        } else {
            "main".into()
        };

        statement!(
            self,
            if self.hlsl.require_output {
                "SPIRV_Cross_Output "
            } else {
                "void "
            },
            entry_point_name,
            "(",
            merge_default(&arguments),
            ")"
        );
        self.begin_scope();
        let legacy = self.hlsl.options.shader_model <= 30;

        // Copy builtins from entry point arguments to globals.
        for i in self.active_input_builtins.bits() {
            let builtin = self.builtin_to_glsl(i as BuiltIn, StorageClassInput)?;
            match i as BuiltIn {
                BuiltInPosition => {
                    if execution.model == ExecutionModelGeometry {
                        statement!(self, "for (int i = 0; i < ", input_vertices, "; i++)");
                        self.begin_scope();
                        statement!(self, builtin, "[i] = stage_input[i].", builtin, ";");
                        self.end_scope()?;
                    } else {
                        statement!(self, builtin, " = stage_input.", builtin, ";");
                    }
                }
                BuiltInFragCoord => {
                    // VPOS in D3D9 is sampled at integer locations, apply half-pixel offset to be consistent.
                    // TODO: Do we need an option here? Any reason why a D3D9 shader would be used
                    // on a D3D10+ system with a different rasterization config?
                    if legacy {
                        statement!(
                            self,
                            builtin,
                            " = stage_input.",
                            builtin,
                            " + float4(0.5f, 0.5f, 0.0f, 0.0f);"
                        );
                    } else {
                        statement!(self, builtin, " = stage_input.", builtin, ";");
                        // ZW are undefined in D3D9, only do this fixup here.
                        statement!(self, builtin, ".w = 1.0 / ", builtin, ".w;");
                    }
                }

                BuiltInVertexId | BuiltInVertexIndex | BuiltInInstanceIndex => {
                    // D3D semantics are uint, but shader wants int.
                    if self.hlsl.options.support_nonzero_base_vertex_base_instance
                        || self.hlsl.options.shader_model >= 68
                    {
                        if self.hlsl.options.shader_model >= 68 {
                            if i == BuiltInInstanceIndex {
                                statement!(
                                    self,
                                    builtin,
                                    " = int(stage_input.",
                                    builtin,
                                    " + stage_input.gl_BaseInstanceARB);"
                                );
                            } else {
                                statement!(
                                    self,
                                    builtin,
                                    " = int(stage_input.",
                                    builtin,
                                    " + stage_input.gl_BaseVertexARB);"
                                );
                            }
                        } else if i == BuiltInInstanceIndex {
                            statement!(
                                self,
                                builtin,
                                " = int(stage_input.",
                                builtin,
                                ") + SPIRV_Cross_BaseInstance;"
                            );
                        } else {
                            statement!(
                                self,
                                builtin,
                                " = int(stage_input.",
                                builtin,
                                ") + SPIRV_Cross_BaseVertex;"
                            );
                        }
                    } else {
                        statement!(self, builtin, " = int(stage_input.", builtin, ");");
                    }
                }

                BuiltInBaseVertex => {
                    if self.hlsl.options.shader_model >= 68 {
                        statement!(self, builtin, " = stage_input.gl_BaseVertexARB;");
                    } else {
                        statement!(self, builtin, " = SPIRV_Cross_BaseVertex;");
                    }
                }

                BuiltInBaseInstance => {
                    if self.hlsl.options.shader_model >= 68 {
                        statement!(self, builtin, " = stage_input.gl_BaseInstanceARB;");
                    } else {
                        statement!(self, builtin, " = SPIRV_Cross_BaseInstance;");
                    }
                }

                BuiltInInstanceId => {
                    // D3D semantics are uint, but shader wants int.
                    statement!(self, builtin, " = int(stage_input.", builtin, ");");
                }

                BuiltInSampleMask => {
                    statement!(self, builtin, "[0] = stage_input.", builtin, ";");
                }

                BuiltInNumWorkgroups
                | BuiltInPointCoord
                | BuiltInSubgroupSize
                | BuiltInSubgroupLocalInvocationId
                | BuiltInHelperInvocation => {}

                BuiltInPrimitiveId => {
                    if execution.model == ExecutionModelGeometry {
                        // PrimitiveId is a separate function parameter for GS.
                        // The global is named gl_PrimitiveIDIn (GLSL convention).
                        statement!(self, builtin, " = gl_PrimitiveID;");
                    } else {
                        statement!(self, builtin, " = stage_input.", builtin, ";");
                    }
                }

                BuiltInInvocationId => {
                    if execution.model == ExecutionModelTessellationControl {
                        // Copy from function parameter to global.
                        statement!(self, builtin, " = uCPID;");
                    } else {
                        // For geometry shaders, copy from struct as usual.
                        statement!(self, builtin, " = stage_input[0].", builtin, ";");
                    }
                }

                BuiltInSubgroupEqMask => {
                    // Emulate these ...
                    // No 64-bit in HLSL, so have to do it in 32-bit and unroll.
                    statement!(
                        self,
                        "gl_SubgroupEqMask = 1u << (WaveGetLaneIndex() - uint4(0, 32, 64, 96));"
                    );
                    statement!(
                        self,
                        "if (WaveGetLaneIndex() >= 32) gl_SubgroupEqMask.x = 0;"
                    );
                    statement!(self, "if (WaveGetLaneIndex() >= 64 || WaveGetLaneIndex() < 32) gl_SubgroupEqMask.y = 0;");
                    statement!(self, "if (WaveGetLaneIndex() >= 96 || WaveGetLaneIndex() < 64) gl_SubgroupEqMask.z = 0;");
                    statement!(
                        self,
                        "if (WaveGetLaneIndex() < 96) gl_SubgroupEqMask.w = 0;"
                    );
                }

                BuiltInSubgroupGeMask => {
                    // Emulate these ...
                    // No 64-bit in HLSL, so have to do it in 32-bit and unroll.
                    statement!(self, "gl_SubgroupGeMask = ~((1u << (WaveGetLaneIndex() - uint4(0, 32, 64, 96))) - 1u);");
                    statement!(
                        self,
                        "if (WaveGetLaneIndex() >= 32) gl_SubgroupGeMask.x = 0u;"
                    );
                    statement!(
                        self,
                        "if (WaveGetLaneIndex() >= 64) gl_SubgroupGeMask.y = 0u;"
                    );
                    statement!(
                        self,
                        "if (WaveGetLaneIndex() >= 96) gl_SubgroupGeMask.z = 0u;"
                    );
                    statement!(
                        self,
                        "if (WaveGetLaneIndex() < 32) gl_SubgroupGeMask.y = ~0u;"
                    );
                    statement!(
                        self,
                        "if (WaveGetLaneIndex() < 64) gl_SubgroupGeMask.z = ~0u;"
                    );
                    statement!(
                        self,
                        "if (WaveGetLaneIndex() < 96) gl_SubgroupGeMask.w = ~0u;"
                    );
                }

                BuiltInSubgroupGtMask => {
                    // Emulate these ...
                    // No 64-bit in HLSL, so have to do it in 32-bit and unroll.
                    statement!(self, "uint gt_lane_index = WaveGetLaneIndex() + 1;");
                    statement!(self, "gl_SubgroupGtMask = ~((1u << (gt_lane_index - uint4(0, 32, 64, 96))) - 1u);");
                    statement!(self, "if (gt_lane_index >= 32) gl_SubgroupGtMask.x = 0u;");
                    statement!(self, "if (gt_lane_index >= 64) gl_SubgroupGtMask.y = 0u;");
                    statement!(self, "if (gt_lane_index >= 96) gl_SubgroupGtMask.z = 0u;");
                    statement!(self, "if (gt_lane_index >= 128) gl_SubgroupGtMask.w = 0u;");
                    statement!(self, "if (gt_lane_index < 32) gl_SubgroupGtMask.y = ~0u;");
                    statement!(self, "if (gt_lane_index < 64) gl_SubgroupGtMask.z = ~0u;");
                    statement!(self, "if (gt_lane_index < 96) gl_SubgroupGtMask.w = ~0u;");
                }

                BuiltInSubgroupLeMask => {
                    // Emulate these ...
                    // No 64-bit in HLSL, so have to do it in 32-bit and unroll.
                    statement!(self, "uint le_lane_index = WaveGetLaneIndex() + 1;");
                    statement!(
                        self,
                        "gl_SubgroupLeMask = (1u << (le_lane_index - uint4(0, 32, 64, 96))) - 1u;"
                    );
                    statement!(self, "if (le_lane_index >= 32) gl_SubgroupLeMask.x = ~0u;");
                    statement!(self, "if (le_lane_index >= 64) gl_SubgroupLeMask.y = ~0u;");
                    statement!(self, "if (le_lane_index >= 96) gl_SubgroupLeMask.z = ~0u;");
                    statement!(self, "if (le_lane_index >= 128) gl_SubgroupLeMask.w = ~0u;");
                    statement!(self, "if (le_lane_index < 32) gl_SubgroupLeMask.y = 0u;");
                    statement!(self, "if (le_lane_index < 64) gl_SubgroupLeMask.z = 0u;");
                    statement!(self, "if (le_lane_index < 96) gl_SubgroupLeMask.w = 0u;");
                }

                BuiltInSubgroupLtMask => {
                    // Emulate these ...
                    // No 64-bit in HLSL, so have to do it in 32-bit and unroll.
                    statement!(self, "gl_SubgroupLtMask = (1u << (WaveGetLaneIndex() - uint4(0, 32, 64, 96))) - 1u;");
                    statement!(
                        self,
                        "if (WaveGetLaneIndex() >= 32) gl_SubgroupLtMask.x = ~0u;"
                    );
                    statement!(
                        self,
                        "if (WaveGetLaneIndex() >= 64) gl_SubgroupLtMask.y = ~0u;"
                    );
                    statement!(
                        self,
                        "if (WaveGetLaneIndex() >= 96) gl_SubgroupLtMask.z = ~0u;"
                    );
                    statement!(
                        self,
                        "if (WaveGetLaneIndex() < 32) gl_SubgroupLtMask.y = 0u;"
                    );
                    statement!(
                        self,
                        "if (WaveGetLaneIndex() < 64) gl_SubgroupLtMask.z = 0u;"
                    );
                    statement!(
                        self,
                        "if (WaveGetLaneIndex() < 96) gl_SubgroupLtMask.w = 0u;"
                    );
                }

                BuiltInClipDistance => {
                    for clip in 0..self.clip_distance_count {
                        statement!(
                            self,
                            "gl_ClipDistance[",
                            clip,
                            "] = stage_input.gl_ClipDistance",
                            clip / 4,
                            ".",
                            b"xyzw"[(clip & 3) as usize] as char,
                            ";"
                        );
                    }
                }

                BuiltInCullDistance => {
                    for cull in 0..self.cull_distance_count {
                        statement!(
                            self,
                            "gl_CullDistance[",
                            cull,
                            "] = stage_input.gl_CullDistance",
                            cull / 4,
                            ".",
                            b"xyzw"[(cull & 3) as usize] as char,
                            ";"
                        );
                    }
                }

                _ => {
                    statement!(self, builtin, " = stage_input.", builtin, ";");
                }
            }
        }

        // Copy from stage input struct to globals.
        for id in self.ir.typed_ids(TypeVariable) {
            let var = self.get_rc::<SPIRVariable>(id)?;
            let type_ = self.get_rc::<SPIRType>(var.basetype)?;
            let block = self.has_decoration(type_.self_, DecorationBlock);

            if var.storage != StorageClassInput {
                continue;
            }

            let is_hidden = self.is_hidden_io_variable(&var)?;

            let need_matrix_unroll =
                var.storage == StorageClassInput && execution.model == ExecutionModelVertex;

            if !var.remapped_variable
                && type_.pointer
                && !self.is_builtin_variable(&var)?
                && self.interface_variable_exists_in_entry_point(var.self_)?
                && !is_hidden
            {
                if block {
                    let type_name = self.to_name(type_.self_, true)?;
                    let var_name = self.to_name(var.self_, true)?;
                    let is_per_vertex = self.has_decoration(var.self_, DecorationPerVertexKHR);
                    let array_size = if is_per_vertex {
                        self.to_array_size_literal_last(&type_)?
                    } else {
                        0
                    };

                    for mbr_idx in 0..type_.member_types.len() as u32 {
                        let mbr_name = self.to_member_name(&type_, mbr_idx)?;
                        let flat_name = join!(type_name, "_", mbr_name);

                        if is_per_vertex {
                            for i in 0..array_size {
                                statement!(
                                    self,
                                    var_name,
                                    "[",
                                    i,
                                    "].",
                                    mbr_name,
                                    " = GetAttributeAtVertex(stage_input.",
                                    flat_name,
                                    ", ",
                                    i,
                                    ");"
                                );
                            }
                        } else {
                            statement!(
                                self,
                                var_name,
                                ".",
                                mbr_name,
                                " = stage_input.",
                                flat_name,
                                ";"
                            );
                        }
                    }
                } else {
                    let name = self.to_name(var.self_, true)?;
                    let mtype = self.get_rc::<SPIRType>(var.basetype)?;
                    if need_matrix_unroll && mtype.columns > 1 {
                        // Unroll matrices.
                        for col in 0..mtype.columns {
                            statement!(
                                self,
                                name,
                                "[",
                                col,
                                "] = stage_input.",
                                name,
                                "_",
                                col,
                                ";"
                            );
                        }
                    } else if self.has_decoration(var.self_, DecorationPerVertexKHR) {
                        let array_size = self.to_array_size_literal_last(&type_)?;
                        for i in 0..array_size {
                            statement!(
                                self,
                                name,
                                "[",
                                i,
                                "]",
                                " = GetAttributeAtVertex(stage_input.",
                                name,
                                ", ",
                                i,
                                ");"
                            );
                        }
                    } else if execution.model == ExecutionModelGeometry {
                        statement!(self, "for (int i = 0; i < ", input_vertices, "; i++)");
                        self.begin_scope();
                        statement!(self, name, "[i] = stage_input[i].", name, ";");
                        self.end_scope()?;
                    } else {
                        statement!(self, name, " = stage_input.", name, ";");
                    }
                }
            }
        }

        // Run the shader.
        if execution.model == ExecutionModelVertex
            || execution.model == ExecutionModelFragment
            || execution.model == ExecutionModelGLCompute
            || execution.model == ExecutionModelMeshEXT
            || execution.model == ExecutionModelGeometry
            || execution.model == ExecutionModelTaskEXT
        {
            // For mesh shaders, we receive special arguments that we must pass down as function arguments.
            // HLSL does not support proper reference types for passing these IO blocks,
            // but DXC post-inlining seems to magically fix it up anyways *shrug*.
            let mut arglist: Vec<String> = Vec::new();
            let func = self.get_rc::<SPIRFunction>(self.ir.default_entry_point)?;
            // The arguments are marked out, avoid detecting reads and emitting inout.

            for arg in &func.arguments {
                arglist.push(self.to_expression(arg.id, false)?);
            }

            if execution.model == ExecutionModelGeometry {
                arglist.push("stage_input".into());
                arglist.push("geometry_stream".into());
            }

            statement!(
                self,
                self.get_inner_entry_point_name()?,
                "(",
                merge_default(&arglist),
                ");"
            );
        } else {
            spirv_cross_throw!("Unsupported shader stage.");
        }

        // Copy stage outputs.
        if self.hlsl.require_output {
            statement!(self, "SPIRV_Cross_Output stage_output;");

            // Copy builtins from globals to return struct.
            for i in self.active_output_builtins.bits() {
                // PointSize doesn't exist in HLSL SM 4+.
                if i == BuiltInPointSize && !legacy {
                    continue;
                }

                match i as BuiltIn {
                    BuiltInClipDistance => {
                        for clip in 0..self.clip_distance_count {
                            statement!(
                                self,
                                "stage_output.gl_ClipDistance",
                                clip / 4,
                                ".",
                                b"xyzw"[(clip & 3) as usize] as char,
                                " = gl_ClipDistance[",
                                clip,
                                "];"
                            );
                        }
                    }

                    BuiltInCullDistance => {
                        for cull in 0..self.cull_distance_count {
                            statement!(
                                self,
                                "stage_output.gl_CullDistance",
                                cull / 4,
                                ".",
                                b"xyzw"[(cull & 3) as usize] as char,
                                " = gl_CullDistance[",
                                cull,
                                "];"
                            );
                        }
                    }

                    BuiltInSampleMask => {
                        statement!(self, "stage_output.gl_SampleMask = gl_SampleMask[0];");
                    }

                    _ => {
                        let builtin_expr =
                            self.builtin_to_glsl(i as BuiltIn, StorageClassOutput)?;
                        statement!(
                            self,
                            "stage_output.",
                            builtin_expr,
                            " = ",
                            builtin_expr,
                            ";"
                        );
                    }
                }
            }

            for id in self.ir.typed_ids(TypeVariable) {
                let var = self.get_rc::<SPIRVariable>(id)?;
                let type_ = self.get_rc::<SPIRType>(var.basetype)?;
                let block = self.has_decoration(type_.self_, DecorationBlock);

                if var.storage != StorageClassOutput {
                    continue;
                }

                if !var.remapped_variable
                    && type_.pointer
                    && !self.is_builtin_variable(&var)?
                    && self.interface_variable_exists_in_entry_point(var.self_)?
                {
                    if block {
                        // I/O blocks need to flatten output.
                        let type_name = self.to_name(type_.self_, true)?;
                        let var_name = self.to_name(var.self_, true)?;
                        for mbr_idx in 0..type_.member_types.len() as u32 {
                            let mbr_name = self.to_member_name(&type_, mbr_idx)?;
                            let flat_name = join!(type_name, "_", mbr_name);
                            statement!(
                                self,
                                "stage_output.",
                                flat_name,
                                " = ",
                                var_name,
                                ".",
                                mbr_name,
                                ";"
                            );
                        }
                    } else {
                        let name = self.to_name(var.self_, true)?;

                        if legacy && execution.model == ExecutionModelFragment {
                            let mut output_filler = String::new();
                            let mut size = type_.vecsize;
                            while size < 4 {
                                output_filler += ", 0.0";
                                size += 1;
                            }

                            statement!(
                                self,
                                "stage_output.",
                                name,
                                " = float4(",
                                name,
                                output_filler,
                                ");"
                            );
                        } else {
                            statement!(self, "stage_output.", name, " = ", name, ";");
                        }
                    }
                }
            }

            statement!(self, "return stage_output;");
        }

        self.end_scope()
    }

    pub(crate) fn hlsl_emit_fixup(&mut self) -> Result<()> {
        if self.is_vertex_like_shader() && self.active_output_builtins.get(BuiltInPosition) {
            // Do various mangling on the gl_Position.
            if self.hlsl.options.shader_model <= 30 {
                statement!(
                    self,
                    "gl_Position.x = gl_Position.x - gl_HalfPixel.x * gl_Position.w;"
                );
                statement!(
                    self,
                    "gl_Position.y = gl_Position.y + gl_HalfPixel.y * gl_Position.w;"
                );
            }

            if self.glsl.options.vertex.flip_vert_y {
                statement!(self, "gl_Position.y = -gl_Position.y;");
            }
            if self.glsl.options.vertex.fixup_clipspace {
                statement!(
                    self,
                    "gl_Position.z = (gl_Position.z + gl_Position.w) * 0.5;"
                );
            }
        }
        Ok(())
    }

    pub(crate) fn hlsl_emit_texture_op(&mut self, i: &Instruction, sparse: bool) -> Result<()> {
        if sparse {
            spirv_cross_throw!("Sparse feedback not yet supported in HLSL.");
        }

        let ops = self.stream(i)?;
        let op = i.op as Op;
        let mut length = i.length;

        let mut inherited_expressions: Vec<u32> = Vec::new();

        let result_type = ops[0];
        let id = ops[1];
        let img: VariableID = ops[2];
        let coord = ops[3];
        let mut dref = 0;
        let mut comp = 0;
        let mut gather = false;
        let mut proj = false;
        let mut opt: usize;
        let combined_image = self.maybe_get_rc::<SPIRCombinedImageSampler>(img);

        if let Some(ci) = &combined_image {
            if self.has_decoration(img, DecorationNonUniform) {
                self.set_decoration(ci.image, DecorationNonUniform, 0);
                self.set_decoration(ci.sampler, DecorationNonUniform, 0);
            }
        }

        let img_expr = self
            .to_non_uniform_aware_expression(combined_image.as_ref().map_or(img, |ci| ci.image))?;

        inherited_expressions.push(coord);

        match op {
            OpImageSampleDrefImplicitLod | OpImageSampleDrefExplicitLod => {
                dref = ops[4];
                opt = 5;
                length = length.wrapping_sub(5);
            }

            OpImageSampleProjDrefImplicitLod | OpImageSampleProjDrefExplicitLod => {
                dref = ops[4];
                proj = true;
                opt = 5;
                length = length.wrapping_sub(5);
            }

            OpImageDrefGather => {
                dref = ops[4];
                opt = 5;
                gather = true;
                length = length.wrapping_sub(5);
            }

            OpImageGather => {
                comp = ops[4];
                opt = 5;
                gather = true;
                length = length.wrapping_sub(5);
            }

            OpImageSampleProjImplicitLod | OpImageSampleProjExplicitLod => {
                opt = 4;
                length = length.wrapping_sub(4);
                proj = true;
            }

            OpImageQueryLod => {
                opt = 4;
                length = length.wrapping_sub(4);
            }

            _ => {
                opt = 4;
                length = length.wrapping_sub(4);
            }
        }

        let imgtype = self.expression_type_rc(img)?;
        let mut coord_components: u32 = match imgtype.image.dim {
            Dim1D => 1,
            Dim2D => 2,
            Dim3D => 3,
            DimCube => 3,
            DimBuffer => 1,
            _ => 2,
        };

        if dref != 0 {
            inherited_expressions.push(dref);
        }

        if imgtype.image.arrayed && op != OpImageQueryLod {
            coord_components += 1;
        }

        let mut bias = 0;
        let mut lod = 0;
        let mut grad_x = 0;
        let mut grad_y = 0;
        let mut coffset = 0;
        let mut offset = 0;
        let mut coffsets = 0;
        let mut sample = 0;
        let mut minlod = 0;
        let mut flags = 0;

        if length != 0 {
            flags = *ops.at(opt)?;
            opt += 1;
            length -= 1;
        }

        let mut test = |v: &mut u32, flag: u32| -> Result<()> {
            if length != 0 && (flags & flag) != 0 {
                *v = *ops.at(opt)?;
                opt += 1;
                inherited_expressions.push(*v);
                length -= 1;
            }
            Ok(())
        };

        test(&mut bias, ImageOperandsBiasMask)?;
        test(&mut lod, ImageOperandsLodMask)?;
        test(&mut grad_x, ImageOperandsGradMask)?;
        test(&mut grad_y, ImageOperandsGradMask)?;
        test(&mut coffset, ImageOperandsConstOffsetMask)?;
        test(&mut offset, ImageOperandsOffsetMask)?;
        test(&mut coffsets, ImageOperandsConstOffsetsMask)?;
        test(&mut sample, ImageOperandsSampleMask)?;
        test(&mut minlod, ImageOperandsMinLodMask)?;

        let mut expr = String::new();
        let mut texop = String::new();

        if minlod != 0 {
            spirv_cross_throw!("MinLod texture operand not supported in HLSL.");
        }

        if op == OpImageFetch {
            if self.hlsl.options.shader_model < 40 {
                spirv_cross_throw!("texelFetch is not supported in HLSL shader model 2/3.");
            }
            texop += &img_expr;
            texop += ".Load";
        } else if op == OpImageQueryLod {
            texop += &img_expr;
            texop += ".CalculateLevelOfDetail";
        } else {
            let imgformat_basetype = self.get::<SPIRType>(imgtype.image.type_)?.basetype;
            if self.hlsl.options.shader_model < 67
                && imgformat_basetype != BaseType::Float
                && !gather
            {
                spirv_cross_throw!(
                    "Sampling non-float textures is not supported in HLSL SM < 6.7."
                );
            }

            if self.hlsl.options.shader_model >= 40 {
                texop += &img_expr;

                if self.is_depth_image(&imgtype, img) {
                    if gather {
                        texop += ".GatherCmp";
                    } else if lod != 0 || grad_x != 0 || grad_y != 0 {
                        // Assume we want a fixed level, and the only thing we can get in HLSL is SampleCmpLevelZero.
                        texop += ".SampleCmpLevelZero";
                    } else {
                        texop += ".SampleCmp";
                    }
                } else if gather {
                    let comp_num = self.evaluate_constant_u32(comp)?;
                    if self.hlsl.options.shader_model >= 50 {
                        match comp_num {
                            0 => texop += ".GatherRed",
                            1 => texop += ".GatherGreen",
                            2 => texop += ".GatherBlue",
                            3 => texop += ".GatherAlpha",
                            _ => spirv_cross_throw!("Invalid component."),
                        }
                    } else if comp_num == 0 {
                        texop += ".Gather";
                    } else {
                        spirv_cross_throw!(
                            "HLSL shader model 4 can only gather from the red component."
                        );
                    }
                } else if bias != 0 {
                    texop += ".SampleBias";
                } else if grad_x != 0 || grad_y != 0 {
                    texop += ".SampleGrad";
                } else if lod != 0 {
                    texop += ".SampleLevel";
                } else {
                    texop += ".Sample";
                }
            } else {
                match imgtype.image.dim {
                    Dim1D => texop += "tex1D",
                    Dim2D => texop += "tex2D",
                    Dim3D => texop += "tex3D",
                    DimCube => texop += "texCUBE",
                    DimRect | DimBuffer | DimSubpassData => {
                        spirv_cross_throw!(
                            "Buffer texture support is not yet implemented for HLSL"
                        ); // TODO
                    }
                    _ => spirv_cross_throw!("Invalid dimension."),
                }

                if gather {
                    spirv_cross_throw!("textureGather is not supported in HLSL shader model 2/3.");
                }
                if offset != 0 || coffset != 0 {
                    spirv_cross_throw!("textureOffset is not supported in HLSL shader model 2/3.");
                }

                if grad_x != 0 || grad_y != 0 {
                    texop += "grad";
                } else if lod != 0 {
                    texop += "lod";
                } else if bias != 0 {
                    texop += "bias";
                } else if proj || dref != 0 {
                    texop += "proj";
                }
            }
        }

        expr += &texop;
        expr += "(";
        if self.hlsl.options.shader_model < 40 {
            if combined_image.is_some() {
                spirv_cross_throw!(
                    "Separate images/samplers are not supported in HLSL shader model 2/3."
                );
            }
            expr += &self.to_expression(img, true)?;
        } else if op != OpImageFetch {
            let sampler_expr = if let Some(ci) = &combined_image {
                self.to_non_uniform_aware_expression(ci.sampler)?
            } else {
                self.hlsl_to_sampler_expression(img)?
            };
            expr += &sampler_expr;
        }

        let swizzle = |comps: u32, in_comps: u32| -> &'static str {
            if comps == in_comps {
                return "";
            }

            match comps {
                1 => ".x",
                2 => ".xy",
                3 => ".xyz",
                _ => "",
            }
        };

        let mut forward = self.should_forward(coord)?;

        // The IR can give us more components than we need, so chop them off as needed.
        let mut coord_expr;
        let coord_type_vecsize = self.expression_type(coord)?.vecsize;
        if coord_components != coord_type_vecsize {
            coord_expr = self.to_enclosed_expression(coord, true)?
                + swizzle(coord_components, coord_type_vecsize);
        } else {
            coord_expr = self.to_expression(coord, true)?;
        }

        if proj && self.hlsl.options.shader_model >= 40 {
            // Legacy HLSL has "proj" operations which do this for us.
            coord_expr = coord_expr
                + " / "
                + &self.to_extract_component_expression(coord, coord_components)?;
        }

        if self.hlsl.options.shader_model < 40 {
            if dref != 0 {
                if imgtype.image.dim != Dim1D && imgtype.image.dim != Dim2D {
                    spirv_cross_throw!(
                        "Depth comparison is only supported for 1D and 2D textures in HLSL shader model 2/3."
                    );
                }

                if grad_x != 0 || grad_y != 0 {
                    spirv_cross_throw!("Depth comparison is not supported for grad sampling in HLSL shader model 2/3.");
                }

                let mut size = coord_components;
                while size < 2 {
                    coord_expr += ", 0.0";
                    size += 1;
                }

                forward = forward && self.should_forward(dref)?;
                coord_expr += &(", ".to_string() + &self.to_expression(dref, true)?);
            } else if lod != 0 || bias != 0 || proj {
                let mut size = coord_components;
                while size < 3 {
                    coord_expr += ", 0.0";
                    size += 1;
                }
            }

            if lod != 0 {
                coord_expr = "float4(".to_string()
                    + &coord_expr
                    + ", "
                    + &self.to_expression(lod, true)?
                    + ")";
            } else if bias != 0 {
                coord_expr = "float4(".to_string()
                    + &coord_expr
                    + ", "
                    + &self.to_expression(bias, true)?
                    + ")";
            } else if proj {
                coord_expr = "float4(".to_string()
                    + &coord_expr
                    + ", "
                    + &self.to_extract_component_expression(coord, coord_components)?
                    + ")";
            } else if dref != 0 {
                // A "normal" sample gets fed into tex2Dproj as well, because the
                // regular tex2D accepts only two coordinates.
                coord_expr = "float4(".to_string() + &coord_expr + ", 1.0)";
            }

            if (lod != 0) as u32 + (bias != 0) as u32 + proj as u32 > 1 {
                spirv_cross_throw!("Legacy HLSL can only use one of lod/bias/proj modifiers.");
            }
        }

        if op == OpImageFetch {
            if imgtype.image.dim != DimBuffer && !imgtype.image.ms {
                coord_expr = join!(
                    "int",
                    coord_components + 1,
                    "(",
                    coord_expr,
                    ", ",
                    if lod != 0 {
                        self.to_expression(lod, true)?
                    } else {
                        "0".to_string()
                    },
                    ")"
                );
            }
        } else {
            expr += ", ";
        }
        expr += &coord_expr;

        if dref != 0 && self.hlsl.options.shader_model >= 40 {
            forward = forward && self.should_forward(dref)?;
            expr += ", ";

            if proj {
                expr += &(self.to_enclosed_expression(dref, true)?
                    + " / "
                    + &self.to_extract_component_expression(coord, coord_components)?);
            } else {
                expr += &self.to_expression(dref, true)?;
            }
        }

        if dref == 0 && (grad_x != 0 || grad_y != 0) {
            forward = forward && self.should_forward(grad_x)?;
            forward = forward && self.should_forward(grad_y)?;
            expr += ", ";
            expr += &self.to_expression(grad_x, true)?;
            expr += ", ";
            expr += &self.to_expression(grad_y, true)?;
        }

        if dref == 0 && lod != 0 && self.hlsl.options.shader_model >= 40 && op != OpImageFetch {
            forward = forward && self.should_forward(lod)?;
            expr += ", ";
            expr += &self.to_expression(lod, true)?;
        }

        if dref == 0 && bias != 0 && self.hlsl.options.shader_model >= 40 {
            forward = forward && self.should_forward(bias)?;
            expr += ", ";
            expr += &self.to_expression(bias, true)?;
        }

        if coffset != 0 {
            forward = forward && self.should_forward(coffset)?;
            expr += ", ";
            expr += &self.to_expression(coffset, true)?;
        } else if offset != 0 {
            forward = forward && self.should_forward(offset)?;
            expr += ", ";
            expr += &self.to_expression(offset, true)?;
        }

        if sample != 0 {
            expr += ", ";
            expr += &self.to_expression(sample, true)?;
        }

        expr += ")";

        if dref != 0 && self.hlsl.options.shader_model < 40 {
            expr += ".x";
        }

        if op == OpImageQueryLod {
            // This is rather awkward.
            // textureQueryLod returns two values, the "accessed level",
            // as well as the actual LOD lambda.
            // As far as I can tell, there is no way to get the .x component
            // according to GLSL spec, and it depends on the sampler itself.
            // Just assume X == Y, so we will need to splat the result to a float2.
            statement!(self, "float _", id, "_tmp = ", expr, ";");
            statement!(self, "float2 _", id, " = _", id, "_tmp.xx;");
            self.set::<SPIRExpression>(id, SPIRExpression::new(join!("_", id), result_type, true))?;
        } else {
            self.emit_op(result_type, id, &expr, forward, false)?;
        }

        for inherit in inherited_expressions {
            self.inherit_expression_dependencies(id, inherit)?;
        }

        match op {
            OpImageSampleDrefImplicitLod
            | OpImageSampleImplicitLod
            | OpImageSampleProjImplicitLod
            | OpImageSampleProjDrefImplicitLod => {
                self.register_control_dependent_expression(id)?;
            }

            _ => {}
        }
        Ok(())
    }

    pub(crate) fn to_resource_binding(&mut self, var: &SPIRVariable) -> Result<String> {
        let type_ = self.get_rc::<SPIRType>(var.basetype)?;

        // We can remap push constant blocks, even if they don't have any binding decoration.
        if type_.storage != StorageClassPushConstant
            && !self.has_decoration(var.self_, DecorationBinding)
        {
            return Ok(String::new());
        }

        let mut space = '\0';

        let mut resource_flags: HLSLBindingFlagBits = HLSL_BINDING_AUTO_NONE_BIT;

        match type_.basetype {
            BaseType::SampledImage => {
                space = 't'; // SRV
                resource_flags = HLSL_BINDING_AUTO_SRV_BIT;
            }

            BaseType::Image => {
                if type_.image.sampled == 2 && type_.image.dim != DimSubpassData {
                    if self.has_decoration(var.self_, DecorationNonWritable)
                        && self.hlsl.options.nonwritable_uav_texture_as_srv
                    {
                        space = 't'; // SRV
                        resource_flags = HLSL_BINDING_AUTO_SRV_BIT;
                    } else {
                        space = 'u'; // UAV
                        resource_flags = HLSL_BINDING_AUTO_UAV_BIT;
                    }
                } else {
                    space = 't'; // SRV
                    resource_flags = HLSL_BINDING_AUTO_SRV_BIT;
                }
            }

            BaseType::Sampler => {
                space = 's';
                resource_flags = HLSL_BINDING_AUTO_SAMPLER_BIT;
            }

            BaseType::AccelerationStructure => {
                space = 't'; // SRV
                resource_flags = HLSL_BINDING_AUTO_SRV_BIT;
            }

            BaseType::Struct => {
                let storage = type_.storage;
                if storage == StorageClassUniform {
                    if self.has_decoration(type_.self_, DecorationBufferBlock) {
                        let flags = self.ir.get_buffer_block_flags(var)?;
                        let is_readonly = flags.get(DecorationNonWritable)
                            && !self.is_hlsl_force_storage_buffer_as_uav(var.self_);
                        space = if is_readonly { 't' } else { 'u' }; // UAV
                        resource_flags = if is_readonly {
                            HLSL_BINDING_AUTO_SRV_BIT
                        } else {
                            HLSL_BINDING_AUTO_UAV_BIT
                        };
                    } else if self.has_decoration(type_.self_, DecorationBlock) {
                        space = 'b'; // Constant buffers
                        resource_flags = HLSL_BINDING_AUTO_CBV_BIT;
                    }
                } else if storage == StorageClassPushConstant {
                    space = 'b'; // Constant buffers
                    resource_flags = HLSL_BINDING_AUTO_PUSH_CONSTANT_BIT;
                } else if storage == StorageClassStorageBuffer {
                    // UAV or SRV depending on readonly flag.
                    let flags = self.ir.get_buffer_block_flags(var)?;
                    let is_readonly = flags.get(DecorationNonWritable)
                        && !self.is_hlsl_force_storage_buffer_as_uav(var.self_);
                    space = if is_readonly { 't' } else { 'u' };
                    resource_flags = if is_readonly {
                        HLSL_BINDING_AUTO_SRV_BIT
                    } else {
                        HLSL_BINDING_AUTO_UAV_BIT
                    };
                }
            }
            _ => {}
        }

        if space == '\0' {
            return Ok(String::new());
        }

        let mut desc_set = if resource_flags == HLSL_BINDING_AUTO_PUSH_CONSTANT_BIT {
            ResourceBindingPushConstantDescriptorSet
        } else {
            0u32
        };
        let mut binding = if resource_flags == HLSL_BINDING_AUTO_PUSH_CONSTANT_BIT {
            ResourceBindingPushConstantBinding
        } else {
            0u32
        };

        if self.has_decoration(var.self_, DecorationBinding) {
            binding = self.get_decoration(var.self_, DecorationBinding);
        }
        if self.has_decoration(var.self_, DecorationDescriptorSet) {
            desc_set = self.get_decoration(var.self_, DecorationDescriptorSet);
        }

        self.to_resource_register(resource_flags, space, binding, desc_set)
    }

    pub(crate) fn to_resource_binding_sampler(&mut self, var: &SPIRVariable) -> Result<String> {
        // For combined image samplers.
        if !self.has_decoration(var.self_, DecorationBinding) {
            return Ok(String::new());
        }

        self.to_resource_register(
            HLSL_BINDING_AUTO_SAMPLER_BIT,
            's',
            self.get_decoration(var.self_, DecorationBinding),
            self.get_decoration(var.self_, DecorationDescriptorSet),
        )
    }

    pub(crate) fn remap_hlsl_resource_binding(
        &mut self,
        type_: HLSLBindingFlagBits,
        desc_set: &mut u32,
        binding: &mut u32,
    ) {
        let key = StageSetBinding {
            model: self.get_execution_model(),
            desc_set: *desc_set,
            binding: *binding,
        };
        if let Some(remap) = self.hlsl.resource_bindings.get_mut(&key) {
            remap.1 = true;

            match type_ {
                HLSL_BINDING_AUTO_PUSH_CONSTANT_BIT | HLSL_BINDING_AUTO_CBV_BIT => {
                    *desc_set = remap.0.cbv.register_space;
                    *binding = remap.0.cbv.register_binding;
                }

                HLSL_BINDING_AUTO_SRV_BIT => {
                    *desc_set = remap.0.srv.register_space;
                    *binding = remap.0.srv.register_binding;
                }

                HLSL_BINDING_AUTO_SAMPLER_BIT => {
                    *desc_set = remap.0.sampler.register_space;
                    *binding = remap.0.sampler.register_binding;
                }

                HLSL_BINDING_AUTO_UAV_BIT => {
                    *desc_set = remap.0.uav.register_space;
                    *binding = remap.0.uav.register_binding;
                }

                _ => {}
            }
        }
    }

    pub(crate) fn to_resource_register(
        &mut self,
        flag: HLSLBindingFlagBits,
        space: char,
        mut binding: u32,
        mut space_set: u32,
    ) -> Result<String> {
        if (flag & self.hlsl.resource_binding_flags) == 0 {
            self.remap_hlsl_resource_binding(flag, &mut space_set, &mut binding);

            // The push constant block did not have a binding, and there were no remap for it,
            // so, declare without register binding.
            if flag == HLSL_BINDING_AUTO_PUSH_CONSTANT_BIT
                && space_set == ResourceBindingPushConstantDescriptorSet
            {
                return Ok(String::new());
            }

            if self.hlsl.options.shader_model >= 51 {
                Ok(join!(
                    " : register(",
                    space,
                    binding,
                    ", space",
                    space_set,
                    ")"
                ))
            } else {
                Ok(join!(" : register(", space, binding, ")"))
            }
        } else {
            Ok(String::new())
        }
    }

    pub(crate) fn emit_modern_uniform(&mut self, var: &SPIRVariable) -> Result<()> {
        let type_ = self.get_rc::<SPIRType>(var.basetype)?;
        match type_.basetype {
            BaseType::SampledImage | BaseType::Image => {
                let mut is_coherent = false;
                if type_.basetype == BaseType::Image && type_.image.sampled == 2 {
                    is_coherent = self.has_decoration(var.self_, DecorationCoherent);
                }

                statement!(
                    self,
                    if is_coherent { "globallycoherent " } else { "" },
                    self.image_type_hlsl_modern(&type_, var.self_)?,
                    " ",
                    self.to_name(var.self_, true)?,
                    self.type_to_array_glsl(&type_, var.self_)?,
                    self.to_resource_binding(var)?,
                    ";"
                );

                if type_.basetype == BaseType::SampledImage && type_.image.dim != DimBuffer {
                    // For combined image samplers, also emit a combined image sampler.
                    if self.is_depth_image(&type_, var.self_) {
                        statement!(
                            self,
                            "SamplerComparisonState ",
                            self.hlsl_to_sampler_expression(var.self_)?,
                            self.type_to_array_glsl(&type_, var.self_)?,
                            self.to_resource_binding_sampler(var)?,
                            ";"
                        );
                    } else {
                        statement!(
                            self,
                            "SamplerState ",
                            self.hlsl_to_sampler_expression(var.self_)?,
                            self.type_to_array_glsl(&type_, var.self_)?,
                            self.to_resource_binding_sampler(var)?,
                            ";"
                        );
                    }
                }
            }

            BaseType::Sampler => {
                if self.comparison_ids.contains(&var.self_) {
                    statement!(
                        self,
                        "SamplerComparisonState ",
                        self.to_name(var.self_, true)?,
                        self.type_to_array_glsl(&type_, var.self_)?,
                        self.to_resource_binding(var)?,
                        ";"
                    );
                } else {
                    statement!(
                        self,
                        "SamplerState ",
                        self.to_name(var.self_, true)?,
                        self.type_to_array_glsl(&type_, var.self_)?,
                        self.to_resource_binding(var)?,
                        ";"
                    );
                }
            }

            _ => {
                statement!(
                    self,
                    self.variable_decl(var)?,
                    self.to_resource_binding(var)?,
                    ";"
                );
            }
        }
        Ok(())
    }

    pub(crate) fn emit_legacy_uniform(&mut self, var: &SPIRVariable) -> Result<()> {
        let basetype = self.get::<SPIRType>(var.basetype)?.basetype;
        match basetype {
            BaseType::Sampler | BaseType::Image => {
                spirv_cross_throw!("Separate image and samplers not supported in legacy HLSL.");
            }

            _ => {
                statement!(self, self.variable_decl(var)?, ";");
            }
        }
        Ok(())
    }

    pub(crate) fn hlsl_emit_uniform(&mut self, var: &SPIRVariable) -> Result<()> {
        self.add_resource_name(var.self_)?;
        if self.hlsl.options.shader_model >= 40 {
            self.emit_modern_uniform(var)
        } else {
            self.emit_legacy_uniform(var)
        }
    }

    pub(crate) fn hlsl_emit_complex_bitcast(&mut self, _: u32, _: u32, _: u32) -> Result<bool> {
        Ok(false)
    }

    pub(crate) fn hlsl_append_global_func_args(
        &mut self,
        func: &SPIRFunction,
        index: u32,
        arglist: &mut Vec<String>,
    ) -> Result<()> {
        self.glsl_append_global_func_args(func, index, arglist)?;

        if func.emits_geometry {
            arglist.push("geometry_stream".into());
        }
        Ok(())
    }

    pub(crate) fn hlsl_bitcast_glsl_op(
        &mut self,
        out_type: &SPIRType,
        in_type: &SPIRType,
    ) -> Result<String> {
        if out_type.basetype == BaseType::UInt && in_type.basetype == BaseType::Int {
            self.type_to_glsl(out_type, 0)
        } else if out_type.basetype == BaseType::UInt64 && in_type.basetype == BaseType::Int64 {
            self.type_to_glsl(out_type, 0)
        } else if out_type.basetype == BaseType::UInt && in_type.basetype == BaseType::Float {
            Ok("asuint".into())
        } else if out_type.basetype == BaseType::Int && in_type.basetype == BaseType::UInt {
            self.type_to_glsl(out_type, 0)
        } else if out_type.basetype == BaseType::Int64 && in_type.basetype == BaseType::UInt64 {
            self.type_to_glsl(out_type, 0)
        } else if out_type.basetype == BaseType::Int && in_type.basetype == BaseType::Float {
            Ok("asint".into())
        } else if out_type.basetype == BaseType::Float && in_type.basetype == BaseType::UInt {
            Ok("asfloat".into())
        } else if out_type.basetype == BaseType::Float && in_type.basetype == BaseType::Int {
            Ok("asfloat".into())
        } else if out_type.basetype == BaseType::Int64 && in_type.basetype == BaseType::Double {
            spirv_cross_throw!("Double to Int64 is not supported in HLSL.");
        } else if out_type.basetype == BaseType::UInt64 && in_type.basetype == BaseType::Double {
            spirv_cross_throw!("Double to UInt64 is not supported in HLSL.");
        } else if out_type.basetype == BaseType::Double && in_type.basetype == BaseType::Int64 {
            Ok("asdouble".into())
        } else if out_type.basetype == BaseType::Double && in_type.basetype == BaseType::UInt64 {
            Ok("asdouble".into())
        } else if out_type.basetype == BaseType::Half
            && in_type.basetype == BaseType::UInt
            && in_type.vecsize == 1
        {
            if !self.hlsl.requires_explicit_fp16_packing {
                self.hlsl.requires_explicit_fp16_packing = true;
                self.force_recompile();
            }
            Ok("spvUnpackFloat2x16".into())
        } else if out_type.basetype == BaseType::UInt
            && in_type.basetype == BaseType::Half
            && in_type.vecsize == 2
        {
            if !self.hlsl.requires_explicit_fp16_packing {
                self.hlsl.requires_explicit_fp16_packing = true;
                self.force_recompile();
            }
            Ok("spvPackFloat2x16".into())
        } else if out_type.basetype == BaseType::UShort && in_type.basetype == BaseType::Half {
            if self.hlsl.options.shader_model < 40 {
                spirv_cross_throw!("Half to UShort requires Shader Model 4.");
            }
            Ok("(".to_string() + &self.type_to_glsl(out_type, 0)? + ")f32tof16")
        } else if out_type.basetype == BaseType::Half && in_type.basetype == BaseType::UShort {
            if self.hlsl.options.shader_model < 40 {
                spirv_cross_throw!("UShort to Half requires Shader Model 4.");
            }
            Ok("(".to_string() + &self.type_to_glsl(out_type, 0)? + ")f16tof32")
        } else {
            Ok(String::new())
        }
    }

    pub(crate) fn hlsl_emit_glsl_op(
        &mut self,
        result_type: u32,
        id: u32,
        eop: u32,
        args: &[u32],
        count: u32,
    ) -> Result<()> {
        let mut op = eop;

        // If we need to do implicit bitcasts, make sure we do it with the correct type.
        let integer_width = self.get_integer_width_for_glsl_instruction(op, args, count)?;
        let int_type = to_signed_basetype(integer_width)?;
        let uint_type = to_unsigned_basetype(integer_width)?;

        op = self.get_remapped_glsl_op(op);
        let a = |i: usize| -> u32 { args.get(i).copied().unwrap_or(0) };

        macro_rules! require {
            ($flag:ident) => {
                if !self.hlsl.$flag {
                    self.hlsl.$flag = true;
                    self.force_recompile();
                }
            };
        }

        match op {
            GLSLstd450InverseSqrt => self.emit_unary_func_op(result_type, id, a(0), "rsqrt")?,

            GLSLstd450Fract => self.emit_unary_func_op(result_type, id, a(0), "frac")?,

            GLSLstd450RoundEven => {
                if self.hlsl.options.shader_model < 40 {
                    spirv_cross_throw!("roundEven is not supported in HLSL shader model 2/3.");
                }
                self.emit_unary_func_op(result_type, id, a(0), "round")?;
            }

            GLSLstd450Trunc => self.emit_unary_func_op(result_type, id, a(0), "trunc")?,

            GLSLstd450Acosh | GLSLstd450Asinh | GLSLstd450Atanh => {
                // These are not supported in HLSL, always emulate them.
                self.emit_emulated_ahyper_op(result_type, id, a(0), op)?;
            }

            GLSLstd450FMix | GLSLstd450IMix => {
                self.emit_trinary_func_op(result_type, id, a(0), a(1), a(2), "lerp")?
            }

            GLSLstd450Atan2 => self.emit_binary_func_op(result_type, id, a(0), a(1), "atan2")?,

            GLSLstd450Fma => self.emit_trinary_func_op(result_type, id, a(0), a(1), a(2), "mad")?,

            GLSLstd450InterpolateAtCentroid => {
                self.emit_unary_func_op(result_type, id, a(0), "EvaluateAttributeAtCentroid")?
            }
            GLSLstd450InterpolateAtSample => {
                self.emit_binary_func_op(result_type, id, a(0), a(1), "EvaluateAttributeAtSample")?
            }
            GLSLstd450InterpolateAtOffset => {
                self.emit_binary_func_op(result_type, id, a(0), a(1), "EvaluateAttributeSnapped")?
            }

            GLSLstd450PackHalf2x16 => {
                require!(requires_fp16_packing);
                self.emit_unary_func_op(result_type, id, a(0), "spvPackHalf2x16")?;
            }

            GLSLstd450UnpackHalf2x16 => {
                require!(requires_fp16_packing);
                self.emit_unary_func_op(result_type, id, a(0), "spvUnpackHalf2x16")?;
            }

            GLSLstd450PackSnorm4x8 => {
                require!(requires_snorm8_packing);
                self.emit_unary_func_op(result_type, id, a(0), "spvPackSnorm4x8")?;
            }

            GLSLstd450UnpackSnorm4x8 => {
                require!(requires_snorm8_packing);
                self.emit_unary_func_op(result_type, id, a(0), "spvUnpackSnorm4x8")?;
            }

            GLSLstd450PackUnorm4x8 => {
                require!(requires_unorm8_packing);
                self.emit_unary_func_op(result_type, id, a(0), "spvPackUnorm4x8")?;
            }

            GLSLstd450UnpackUnorm4x8 => {
                require!(requires_unorm8_packing);
                self.emit_unary_func_op(result_type, id, a(0), "spvUnpackUnorm4x8")?;
            }

            GLSLstd450PackSnorm2x16 => {
                require!(requires_snorm16_packing);
                self.emit_unary_func_op(result_type, id, a(0), "spvPackSnorm2x16")?;
            }

            GLSLstd450UnpackSnorm2x16 => {
                require!(requires_snorm16_packing);
                self.emit_unary_func_op(result_type, id, a(0), "spvUnpackSnorm2x16")?;
            }

            GLSLstd450PackUnorm2x16 => {
                require!(requires_unorm16_packing);
                self.emit_unary_func_op(result_type, id, a(0), "spvPackUnorm2x16")?;
            }

            GLSLstd450UnpackUnorm2x16 => {
                require!(requires_unorm16_packing);
                self.emit_unary_func_op(result_type, id, a(0), "spvUnpackUnorm2x16")?;
            }

            GLSLstd450PackDouble2x32 | GLSLstd450UnpackDouble2x32 => {
                spirv_cross_throw!("packDouble2x32/unpackDouble2x32 not supported in HLSL.");
            }

            GLSLstd450FindILsb => {
                let basetype = self.expression_type(a(0))?.basetype;
                self.emit_unary_func_op_cast(
                    result_type,
                    id,
                    a(0),
                    "firstbitlow",
                    basetype,
                    basetype,
                )?;
            }

            GLSLstd450FindSMsb => self.emit_unary_func_op_cast(
                result_type,
                id,
                a(0),
                "firstbithigh",
                int_type,
                int_type,
            )?,

            GLSLstd450FindUMsb => self.emit_unary_func_op_cast(
                result_type,
                id,
                a(0),
                "firstbithigh",
                uint_type,
                uint_type,
            )?,

            GLSLstd450MatrixInverse => {
                let (vecsize, columns) = {
                    let type_ = self.get::<SPIRType>(result_type)?;
                    (type_.vecsize, type_.columns)
                };
                if vecsize == 2 && columns == 2 {
                    require!(requires_inverse_2x2);
                } else if vecsize == 3 && columns == 3 {
                    require!(requires_inverse_3x3);
                } else if vecsize == 4 && columns == 4 {
                    require!(requires_inverse_4x4);
                }
                self.emit_unary_func_op(result_type, id, a(0), "spvInverse")?;
            }

            GLSLstd450Normalize => {
                // HLSL does not support scalar versions here.
                if self.expression_type(a(0))?.vecsize == 1 {
                    // Returns -1 or 1 for valid input, sign() does the job.
                    self.emit_unary_func_op(result_type, id, a(0), "sign")?;
                } else {
                    self.glsl_emit_glsl_op(result_type, id, eop, args, count)?;
                }
            }

            GLSLstd450Reflect => {
                if self.get::<SPIRType>(result_type)?.vecsize == 1 {
                    require!(requires_scalar_reflect);
                    self.emit_binary_func_op(result_type, id, a(0), a(1), "spvReflect")?;
                } else {
                    self.glsl_emit_glsl_op(result_type, id, eop, args, count)?;
                }
            }

            GLSLstd450Refract => {
                if self.get::<SPIRType>(result_type)?.vecsize == 1 {
                    require!(requires_scalar_refract);
                    self.emit_trinary_func_op(result_type, id, a(0), a(1), a(2), "spvRefract")?;
                } else {
                    self.glsl_emit_glsl_op(result_type, id, eop, args, count)?;
                }
            }

            GLSLstd450FaceForward => {
                if self.get::<SPIRType>(result_type)?.vecsize == 1 {
                    require!(requires_scalar_faceforward);
                    self.emit_trinary_func_op(result_type, id, a(0), a(1), a(2), "spvFaceForward")?;
                } else {
                    self.glsl_emit_glsl_op(result_type, id, eop, args, count)?;
                }
            }

            GLSLstd450NMin => {
                self.glsl_emit_glsl_op(result_type, id, GLSLstd450FMin, args, count)?
            }

            GLSLstd450NMax => {
                self.glsl_emit_glsl_op(result_type, id, GLSLstd450FMax, args, count)?
            }

            GLSLstd450NClamp => {
                self.glsl_emit_glsl_op(result_type, id, GLSLstd450FClamp, args, count)?
            }

            _ => self.glsl_emit_glsl_op(result_type, id, eop, args, count)?,
        }
        Ok(())
    }

    pub(crate) fn read_access_chain_array(
        &mut self,
        lhs: &str,
        chain: &SPIRAccessChain,
        depth: u32,
    ) -> Result<()> {
        let type_ = self.get_rc::<SPIRType>(chain.basetype)?;

        // Need to use a reserved identifier here since it might shadow an identifier in the access chain input or other loops.
        let ident = self.get_unique_identifier();

        statement!(self, "[unroll]");
        statement!(
            self,
            "for (int ",
            ident,
            " = 0; ",
            ident,
            " < ",
            self.to_array_size(&type_, (type_.array.len() as u32).wrapping_sub(1))?,
            "; ",
            ident,
            "++)"
        );
        self.begin_scope();
        let mut subchain = chain.clone();
        subchain.dynamic_index =
            join!(ident, " * ", chain.array_stride, " + ", chain.dynamic_index);
        subchain.basetype = type_.parent_type;
        if !self.get::<SPIRType>(subchain.basetype)?.array.is_empty() {
            subchain.array_stride = self.get_decoration(subchain.basetype, DecorationArrayStride);
        }
        self.read_access_chain_d(None, &join!(lhs, "[", ident, "]"), &subchain, depth + 1)?;
        self.end_scope()
    }

    pub(crate) fn read_access_chain_struct(
        &mut self,
        lhs: &str,
        chain: &SPIRAccessChain,
        depth: u32,
    ) -> Result<()> {
        let type_ = self.get_rc::<SPIRType>(chain.basetype)?;
        let mut subchain = chain.clone();
        let member_count = type_.member_types.len() as u32;

        for i in 0..member_count {
            let offset = self.type_struct_member_offset(&type_, i)?;
            subchain.static_index = (chain.static_index as u32).wrapping_add(offset) as i32;
            subchain.basetype = type_.member_types[i as usize];

            subchain.matrix_stride = 0;
            subchain.array_stride = 0;
            subchain.row_major_matrix = false;

            let member_type = self.get_rc::<SPIRType>(subchain.basetype)?;
            if member_type.columns > 1 {
                subchain.matrix_stride = self.type_struct_member_matrix_stride(&type_, i)?;
                subchain.row_major_matrix =
                    self.has_member_decoration(type_.self_, i, DecorationRowMajor);
            }

            if !member_type.array.is_empty() {
                subchain.array_stride = self.type_struct_member_array_stride(&type_, i)?;
            }

            let sub_lhs = join!(lhs, ".", self.to_member_name(&type_, i)?);
            self.read_access_chain_d(None, &sub_lhs, &subchain, depth + 1)?;
        }
        Ok(())
    }

    pub(crate) fn read_access_chain(
        &mut self,
        expr: Option<&mut String>,
        lhs: &str,
        chain: &SPIRAccessChain,
    ) -> Result<()> {
        self.read_access_chain_d(expr, lhs, chain, 0)
    }

    fn read_access_chain_d(
        &mut self,
        expr: Option<&mut String>,
        lhs: &str,
        chain: &SPIRAccessChain,
        depth: u32,
    ) -> Result<()> {
        if depth > 1024 {
            spirv_cross_throw!("Type recursion too deep.");
        }
        let type_ = self.get_rc::<SPIRType>(chain.basetype)?;

        let mut target_type = SPIRType::new(if self.is_scalar(&type_) {
            OpTypeInt
        } else {
            type_.op
        });
        target_type.basetype = BaseType::UInt;
        target_type.vecsize = type_.vecsize;
        target_type.columns = type_.columns;

        if !type_.array.is_empty() {
            return self.read_access_chain_array(lhs, chain, depth);
        } else if type_.basetype == BaseType::Struct {
            return self.read_access_chain_struct(lhs, chain, depth);
        } else if type_.width != 32 && !self.hlsl.options.enable_16bit_types {
            spirv_cross_throw!(
                "Reading types other than 32-bit from ByteAddressBuffer not yet supported, unless SM 6.2 and \
                 native 16-bit types are enabled."
            );
        }

        let mut base = chain.base.clone();
        if self.has_decoration(chain.self_, DecorationNonUniform) {
            self.convert_non_uniform_expression(&mut base, chain.self_)?;
        }

        let templated_load = self.hlsl.options.shader_model >= 62;
        let mut load_expr = String::new();

        let mut template_expr = String::new();
        if templated_load {
            template_expr = join!("<", self.type_to_glsl(&type_, 0)?, ">");
        }

        // Load a vector or scalar.
        if type_.columns == 1 && !chain.row_major_matrix {
            let mut load_op = match type_.vecsize {
                1 => "Load",
                2 => "Load2",
                3 => "Load3",
                4 => "Load4",
                _ => spirv_cross_throw!("Unknown vector size."),
            };

            if templated_load {
                load_op = "Load";
            }

            load_expr = join!(
                base,
                ".",
                load_op,
                template_expr,
                "(",
                chain.dynamic_index,
                chain.static_index,
                ")"
            );
        } else if type_.columns == 1 {
            // Strided load since we are loading a column from a row-major matrix.
            if templated_load {
                let mut scalar_type = (*type_).clone();
                scalar_type.vecsize = 1;
                scalar_type.columns = 1;
                template_expr = join!("<", self.type_to_glsl(&scalar_type, 0)?, ">");
                if type_.vecsize > 1 {
                    load_expr += &(self.type_to_glsl(&type_, 0)? + "(");
                }
            } else if type_.vecsize > 1 {
                load_expr = self.type_to_glsl(&target_type, 0)?;
                load_expr += "(";
            }

            for r in 0..type_.vecsize {
                load_expr += &join!(
                    base,
                    ".Load",
                    template_expr,
                    "(",
                    chain.dynamic_index,
                    (chain.static_index as u32).wrapping_add(r.wrapping_mul(chain.matrix_stride)),
                    ")"
                );
                if r + 1 < type_.vecsize {
                    load_expr += ", ";
                }
            }

            if type_.vecsize > 1 {
                load_expr += ")";
            }
        } else if !chain.row_major_matrix {
            // Load a matrix, column-major, the easy case.
            let mut load_op = match type_.vecsize {
                1 => "Load",
                2 => "Load2",
                3 => "Load3",
                4 => "Load4",
                _ => spirv_cross_throw!("Unknown vector size."),
            };

            if templated_load {
                let mut vector_type = (*type_).clone();
                vector_type.columns = 1;
                template_expr = join!("<", self.type_to_glsl(&vector_type, 0)?, ">");
                load_expr = self.type_to_glsl(&type_, 0)?;
                load_op = "Load";
            } else {
                // Note, this loading style in HLSL is *actually* row-major, but we always treat matrices as transposed in this backend,
                // so row-major is technically column-major ...
                load_expr = self.type_to_glsl(&target_type, 0)?;
            }
            load_expr += "(";

            for c in 0..type_.columns {
                load_expr += &join!(
                    base,
                    ".",
                    load_op,
                    template_expr,
                    "(",
                    chain.dynamic_index,
                    (chain.static_index as u32).wrapping_add(c.wrapping_mul(chain.matrix_stride)),
                    ")"
                );
                if c + 1 < type_.columns {
                    load_expr += ", ";
                }
            }
            load_expr += ")";
        } else {
            // Pick out elements one by one ... Hopefully compilers are smart enough to recognize this pattern
            // considering HLSL is "row-major decl", but "column-major" memory layout (basically implicit transpose model, ugh) ...

            if templated_load {
                load_expr = self.type_to_glsl(&type_, 0)?;
                let mut scalar_type = (*type_).clone();
                scalar_type.vecsize = 1;
                scalar_type.columns = 1;
                template_expr = join!("<", self.type_to_glsl(&scalar_type, 0)?, ">");
            } else {
                load_expr = self.type_to_glsl(&target_type, 0)?;
            }

            load_expr += "(";

            for c in 0..type_.columns {
                for r in 0..type_.vecsize {
                    load_expr += &join!(
                        base,
                        ".Load",
                        template_expr,
                        "(",
                        chain.dynamic_index,
                        (chain.static_index as u32)
                            .wrapping_add(c.wrapping_mul(type_.width / 8))
                            .wrapping_add(r.wrapping_mul(chain.matrix_stride)),
                        ")"
                    );

                    if (r + 1 < type_.vecsize) || (c + 1 < type_.columns) {
                        load_expr += ", ";
                    }
                }
            }
            load_expr += ")";
        }

        if !templated_load {
            let bitcast_op = self.bitcast_glsl_op(&type_, &target_type)?;
            if !bitcast_op.is_empty() {
                load_expr = join!(bitcast_op, "(", load_expr, ")");
            }
        }

        if lhs.is_empty() {
            let Some(expr) = expr else {
                spirv_cross_throw!("Access chain read without a target.");
            };
            *expr = load_expr;
        } else {
            statement!(self, lhs, " = ", load_expr, ";");
        }
        Ok(())
    }

    pub(crate) fn hlsl_emit_load(&mut self, instruction: &Instruction) -> Result<()> {
        let ops = self.stream(instruction)?;

        let result_type = ops[0];
        let id = ops[1];
        let ptr = ops[2];

        let chain = self.maybe_get_rc::<SPIRAccessChain>(ptr);
        if let Some(chain) = chain {
            let type_ = self.get_rc::<SPIRType>(result_type)?;
            let composite_load = !type_.array.is_empty() || type_.basetype == BaseType::Struct;

            if composite_load {
                // We cannot make this work in one single expression as we might have nested structures and arrays,
                // so unroll the load to an uninitialized temporary.
                self.emit_uninitialized_temporary_expression(result_type, id)?;
                let lhs = self.to_expression(id, true)?;
                self.read_access_chain(None, &lhs, &chain)?;
                self.track_expression_read(chain.self_)?;
            } else {
                let mut load_expr = String::new();
                self.read_access_chain(Some(&mut load_expr), "", &chain)?;

                let mut forward =
                    self.should_forward(ptr)? && !self.forced_temporaries.contains(&id);

                // If we are forwarding this load,
                // don't register the read to access chain here, defer that to when we actually use the expression,
                // using the add_implied_read_expression mechanism.
                if !forward {
                    self.track_expression_read(chain.self_)?;
                }

                // Do not forward complex load sequences like matrices, structs and arrays.
                if type_.columns > 1 {
                    forward = false;
                }

                self.emit_op(result_type, id, &load_expr, forward, true)?;
                self.get_mut::<SPIRExpression>(id)?.need_transpose = false;
                self.register_read(id, ptr, forward)?;
                self.inherit_expression_dependencies(id, ptr)?;
                if forward {
                    Self::add_implied_read_expression_expr(
                        self.get_mut::<SPIRExpression>(id)?,
                        chain.self_,
                    );
                }
            }
        } else {
            // Very special case where we cannot rely on IO lowering.
            // Mesh shader clip/cull arrays ... Cursed.
            let res_type = self.get_rc::<SPIRType>(result_type)?;
            if self.get_execution_model() == ExecutionModelMeshEXT
                && self.has_decoration(ptr, DecorationBuiltIn)
                && (self.get_decoration(ptr, DecorationBuiltIn) == BuiltInClipDistance
                    || self.get_decoration(ptr, DecorationBuiltIn) == BuiltInCullDistance)
                && self.is_array(&res_type)
                && !self.is_array(self.get::<SPIRType>(res_type.parent_type)?)
                && self.to_array_size_literal_last(&res_type)? > 1
            {
                self.track_expression_read(ptr)?;
                let mut load_expr = String::from("{ ");
                let num_elements = self.to_array_size_literal_last(&res_type)?;
                for i in 0..num_elements {
                    load_expr += &join!(
                        self.to_expression(ptr, true)?,
                        ".",
                        Self::index_to_swizzle(i)
                    );
                    if i + 1 < num_elements {
                        load_expr += ", ";
                    }
                }
                load_expr += " }";
                self.emit_op(result_type, id, &load_expr, false, false)?;
                self.register_read(id, ptr, false)?;
                self.inherit_expression_dependencies(id, ptr)?;
            } else {
                self.glsl_emit_instruction(instruction)?;
            }
        }
        Ok(())
    }

    pub(crate) fn write_access_chain_array(
        &mut self,
        chain: &SPIRAccessChain,
        value: u32,
        composite_chain: &[u32],
        depth: u32,
    ) -> Result<()> {
        let mut ptype = self.get_rc::<SPIRType>(chain.basetype)?;
        let mut steps = 0u32;
        while ptype.pointer {
            steps += 1;
            if steps > 1024 {
                spirv_cross_throw!("Type recursion too deep.");
            }
            // FIXME (upstream): this looks up the BaseType enum value as an ID
            // (parent_type was presumably meant).
            ptype = self.get_rc::<SPIRType>(ptype.basetype.to_u32())?;
        }
        let type_ = ptype;

        // Need to use a reserved identifier here since it might shadow an identifier in the access chain input or other loops.
        let ident = self.get_unique_identifier();

        let id = self.ir.increase_bound_by(2)?;
        let int_type_id = id + 1;
        let mut int_type = SPIRType::new(OpTypeInt);
        int_type.basetype = BaseType::Int;
        int_type.width = 32;
        self.set::<SPIRType>(int_type_id, int_type)?;
        self.set::<SPIRExpression>(id, SPIRExpression::new(ident.clone(), int_type_id, true))?;
        self.set_name(id, &ident);
        self.suppressed_usage_tracking.insert(id);

        statement!(self, "[unroll]");
        statement!(
            self,
            "for (int ",
            ident,
            " = 0; ",
            ident,
            " < ",
            self.to_array_size(&type_, (type_.array.len() as u32).wrapping_sub(1))?,
            "; ",
            ident,
            "++)"
        );
        self.begin_scope();
        let mut subchain = chain.clone();
        subchain.dynamic_index =
            join!(ident, " * ", chain.array_stride, " + ", chain.dynamic_index);
        subchain.basetype = type_.parent_type;

        // Forcefully allow us to use an ID here by setting MSB.
        let mut subcomposite_chain = composite_chain.to_vec();
        subcomposite_chain.push(0x80000000u32 | id);

        if !self.get::<SPIRType>(subchain.basetype)?.array.is_empty() {
            subchain.array_stride = self.get_decoration(subchain.basetype, DecorationArrayStride);
        }

        self.write_access_chain_d(&subchain, value, &subcomposite_chain, depth + 1)?;
        self.end_scope()
    }

    pub(crate) fn write_access_chain_struct(
        &mut self,
        chain: &SPIRAccessChain,
        value: u32,
        composite_chain: &[u32],
        depth: u32,
    ) -> Result<()> {
        let type_ = self.get_rc::<SPIRType>(chain.basetype)?;
        let member_count = type_.member_types.len() as u32;
        let mut subchain = chain.clone();

        let mut subcomposite_chain = composite_chain.to_vec();
        subcomposite_chain.push(0);

        for i in 0..member_count {
            let offset = self.type_struct_member_offset(&type_, i)?;
            subchain.static_index = (chain.static_index as u32).wrapping_add(offset) as i32;
            subchain.basetype = type_.member_types[i as usize];

            subchain.matrix_stride = 0;
            subchain.array_stride = 0;
            subchain.row_major_matrix = false;

            let member_type = self.get_rc::<SPIRType>(subchain.basetype)?;
            if member_type.columns > 1 {
                subchain.matrix_stride = self.type_struct_member_matrix_stride(&type_, i)?;
                subchain.row_major_matrix =
                    self.has_member_decoration(type_.self_, i, DecorationRowMajor);
            }

            if !member_type.array.is_empty() {
                subchain.array_stride = self.type_struct_member_array_stride(&type_, i)?;
            }

            *subcomposite_chain.last_mut().unwrap() = i;
            self.write_access_chain_d(&subchain, value, &subcomposite_chain, depth + 1)?;
        }
        Ok(())
    }

    pub(crate) fn write_access_chain_value(
        &mut self,
        value: u32,
        composite_chain: &[u32],
        enclose: bool,
    ) -> Result<String> {
        let mut ret = if composite_chain.is_empty() {
            self.to_expression(value, true)?
        } else {
            let mut meta = AccessChainMeta::default();
            self.access_chain_internal(
                value,
                composite_chain,
                ACCESS_CHAIN_INDEX_IS_LITERAL_BIT | ACCESS_CHAIN_LITERAL_MSB_FORCE_ID,
                Some(&mut meta),
            )?
        };

        if enclose {
            ret = self.enclose_expression(&ret);
        }
        Ok(ret)
    }

    pub(crate) fn write_access_chain(
        &mut self,
        chain: &SPIRAccessChain,
        value: u32,
        composite_chain: &[u32],
    ) -> Result<()> {
        self.write_access_chain_d(chain, value, composite_chain, 0)
    }

    fn write_access_chain_d(
        &mut self,
        chain: &SPIRAccessChain,
        value: u32,
        composite_chain: &[u32],
        depth: u32,
    ) -> Result<()> {
        if depth > 1024 {
            spirv_cross_throw!("Type recursion too deep.");
        }
        let type_ = self.get_rc::<SPIRType>(chain.basetype)?;

        // Make sure we trigger a read of the constituents in the access chain.
        self.track_expression_read(chain.self_)?;

        let mut target_type = SPIRType::new(if self.is_scalar(&type_) {
            OpTypeInt
        } else {
            type_.op
        });
        target_type.basetype = BaseType::UInt;
        target_type.vecsize = type_.vecsize;
        target_type.columns = type_.columns;

        if !type_.array.is_empty() {
            self.write_access_chain_array(chain, value, composite_chain, depth)?;
            self.register_write(chain.self_)?;
            return Ok(());
        } else if type_.basetype == BaseType::Struct {
            self.write_access_chain_struct(chain, value, composite_chain, depth)?;
            self.register_write(chain.self_)?;
            return Ok(());
        } else if type_.width != 32 && !self.hlsl.options.enable_16bit_types {
            spirv_cross_throw!(
                "Writing types other than 32-bit to RWByteAddressBuffer not yet supported, unless SM 6.2 and \
                 native 16-bit types are enabled."
            );
        }

        let templated_store = self.hlsl.options.shader_model >= 62;

        let mut base = chain.base.clone();
        if self.has_decoration(chain.self_, DecorationNonUniform) {
            self.convert_non_uniform_expression(&mut base, chain.self_)?;
        }

        let mut template_expr = String::new();
        if templated_store {
            template_expr = join!("<", self.type_to_glsl(&type_, 0)?, ">");
        }

        if type_.columns == 1 && !chain.row_major_matrix {
            let mut store_op = match type_.vecsize {
                1 => "Store",
                2 => "Store2",
                3 => "Store3",
                4 => "Store4",
                _ => spirv_cross_throw!("Unknown vector size."),
            };

            let mut store_expr = self.write_access_chain_value(value, composite_chain, false)?;

            if !templated_store {
                let bitcast_op = self.bitcast_glsl_op(&target_type, &type_)?;
                if !bitcast_op.is_empty() {
                    store_expr = join!(bitcast_op, "(", store_expr, ")");
                }
            } else {
                store_op = "Store";
            }
            statement!(
                self,
                base,
                ".",
                store_op,
                template_expr,
                "(",
                chain.dynamic_index,
                chain.static_index,
                ", ",
                store_expr,
                ");"
            );
        } else if type_.columns == 1 {
            if templated_store {
                let mut scalar_type = (*type_).clone();
                scalar_type.vecsize = 1;
                scalar_type.columns = 1;
                template_expr = join!("<", self.type_to_glsl(&scalar_type, 0)?, ">");
            }

            // Strided store.
            for r in 0..type_.vecsize {
                let mut store_expr = self.write_access_chain_value(value, composite_chain, true)?;
                if type_.vecsize > 1 {
                    store_expr += ".";
                    store_expr += Self::index_to_swizzle(r);
                }
                self.remove_duplicate_swizzle(&mut store_expr);

                if !templated_store {
                    let bitcast_op = self.bitcast_glsl_op(&target_type, &type_)?;
                    if !bitcast_op.is_empty() {
                        store_expr = join!(bitcast_op, "(", store_expr, ")");
                    }
                }

                statement!(
                    self,
                    base,
                    ".Store",
                    template_expr,
                    "(",
                    chain.dynamic_index,
                    (chain.static_index as u32).wrapping_add(chain.matrix_stride.wrapping_mul(r)),
                    ", ",
                    store_expr,
                    ");"
                );
            }
        } else if !chain.row_major_matrix {
            let mut store_op = match type_.vecsize {
                1 => "Store",
                2 => "Store2",
                3 => "Store3",
                4 => "Store4",
                _ => spirv_cross_throw!("Unknown vector size."),
            };

            if templated_store {
                store_op = "Store";
                let mut vector_type = (*type_).clone();
                vector_type.columns = 1;
                template_expr = join!("<", self.type_to_glsl(&vector_type, 0)?, ">");
            }

            for c in 0..type_.columns {
                let mut store_expr = join!(
                    self.write_access_chain_value(value, composite_chain, true)?,
                    "[",
                    c,
                    "]"
                );

                if !templated_store {
                    let bitcast_op = self.bitcast_glsl_op(&target_type, &type_)?;
                    if !bitcast_op.is_empty() {
                        store_expr = join!(bitcast_op, "(", store_expr, ")");
                    }
                }

                statement!(
                    self,
                    base,
                    ".",
                    store_op,
                    template_expr,
                    "(",
                    chain.dynamic_index,
                    (chain.static_index as u32).wrapping_add(c.wrapping_mul(chain.matrix_stride)),
                    ", ",
                    store_expr,
                    ");"
                );
            }
        } else {
            if templated_store {
                let mut scalar_type = (*type_).clone();
                scalar_type.vecsize = 1;
                scalar_type.columns = 1;
                template_expr = join!("<", self.type_to_glsl(&scalar_type, 0)?, ">");
            }

            for r in 0..type_.vecsize {
                for c in 0..type_.columns {
                    let mut store_expr = join!(
                        self.write_access_chain_value(value, composite_chain, true)?,
                        "[",
                        c,
                        "].",
                        Self::index_to_swizzle(r)
                    );
                    self.remove_duplicate_swizzle(&mut store_expr);
                    let bitcast_op = self.bitcast_glsl_op(&target_type, &type_)?;
                    if !bitcast_op.is_empty() {
                        store_expr = join!(bitcast_op, "(", store_expr, ")");
                    }
                    statement!(
                        self,
                        base,
                        ".Store",
                        template_expr,
                        "(",
                        chain.dynamic_index,
                        (chain.static_index as u32)
                            .wrapping_add(c.wrapping_mul(type_.width / 8))
                            .wrapping_add(r.wrapping_mul(chain.matrix_stride)),
                        ", ",
                        store_expr,
                        ");"
                    );
                }
            }
        }

        self.register_write(chain.self_)
    }

    pub(crate) fn hlsl_emit_store(&mut self, instruction: &Instruction) -> Result<()> {
        let ops = self.stream(instruction)?;
        if self.glsl.options.vertex.flip_vert_y {
            let flip = self
                .maybe_get::<SPIRExpression>(ops[0])
                .is_some_and(|e| e.access_meshlet_position_y);
            if flip {
                let lhs = self.to_dereferenced_expression(ops[0], true)?;
                let rhs = self.to_unpacked_expression(ops[1], true)?;
                statement!(self, lhs, " = spvFlipVertY(", rhs, ");");
                self.register_write(ops[0])?;
                return Ok(());
            }
        }

        let chain = self.maybe_get_rc::<SPIRAccessChain>(ops[0]);
        if let Some(chain) = chain {
            self.write_access_chain(&chain, ops[1], &[])
        } else {
            self.glsl_emit_instruction(instruction)
        }
    }

    pub(crate) fn hlsl_emit_access_chain(&mut self, instruction: &Instruction) -> Result<()> {
        let ops = self.stream(instruction)?;
        let length = instruction.length;

        let mut need_byte_access_chain = false;
        let type_ = self.expression_type_rc(ops[2])?;
        let chain = self.maybe_get_rc::<SPIRAccessChain>(ops[2]);

        if chain.is_some() {
            // Keep tacking on an existing access chain.
            need_byte_access_chain = true;
        } else if type_.storage == StorageClassStorageBuffer
            || self.has_decoration(type_.self_, DecorationBufferBlock)
        {
            // If we are starting to poke into an SSBO, we are dealing with ByteAddressBuffers, and we need
            // to emit SPIRAccessChain rather than a plain SPIRExpression.
            let chain_arguments = length.wrapping_sub(3);
            if chain_arguments as usize > type_.array.len() {
                need_byte_access_chain = true;
            }
        }

        if need_byte_access_chain {
            // If we have a chain variable, we are already inside the SSBO, and any array type will refer to arrays within a block,
            // and not array of SSBO.
            let to_plain_buffer_length: u32 = if chain.is_some() {
                0
            } else {
                type_.array.len() as u32
            };

            let backing_variable = self.maybe_get_backing_variable(ops[2]);

            if let Some(bv) = backing_variable {
                let bv_self = self.get::<SPIRVariable>(bv)?.self_;
                if self.is_user_type_structured(bv_self)? {
                    return self.glsl_emit_instruction(instruction);
                }
            }

            let base = if to_plain_buffer_length != 0 {
                let target = self.get_rc::<SPIRType>(ops[0])?;
                let indices = ops_sub(&ops, 3, to_plain_buffer_length)?;
                self.access_chain(ops[2], indices, &target, None, false, None)?
            } else if let Some(chain) = &chain {
                chain.base.clone()
            } else {
                self.to_expression(ops[2], true)?
            };

            // Start traversing type hierarchy at the proper non-pointer types.
            let mut basetype = self.get_pointee_type(&type_)?.clone();

            // Traverse the type hierarchy down to the actual buffer types.
            for _ in 0..to_plain_buffer_length {
                if basetype.parent_type == 0 {
                    spirv_cross_throw!("Access chain type has no parent type.");
                }
                basetype = self.get::<SPIRType>(basetype.parent_type)?.clone();
            }

            let mut matrix_stride: u32 = 0;
            let mut array_stride: u32 = 0;
            let mut row_major_matrix = false;

            // Inherit matrix information.
            if let Some(chain) = &chain {
                matrix_stride = chain.matrix_stride;
                row_major_matrix = chain.row_major_matrix;
                array_stride = chain.array_stride;
            }

            let indices = ops_sub(
                &ops,
                3 + to_plain_buffer_length,
                length.wrapping_sub(3).wrapping_sub(to_plain_buffer_length),
            )?;
            let offsets = self.flattened_access_chain_offset(
                &basetype,
                indices,
                0,
                1,
                Some(&mut row_major_matrix),
                Some(&mut matrix_stride),
                Some(&mut array_stride),
                false,
            )?;

            let immutable = self.should_forward(ops[2])?;
            let loaded_from = match backing_variable {
                Some(bv) => self.get::<SPIRVariable>(bv)?.self_,
                None => 0,
            };
            let e = self.set::<SPIRAccessChain>(
                ops[1],
                SPIRAccessChain::new(ops[0], type_.storage, base, offsets.0, offsets.1 as i32),
            )?;
            e.row_major_matrix = row_major_matrix;
            e.matrix_stride = matrix_stride;
            e.array_stride = array_stride;
            e.immutable = immutable;
            e.loaded_from = loaded_from;

            if let Some(chain) = &chain {
                e.dynamic_index += &chain.dynamic_index;
                e.static_index = e.static_index.wrapping_add(chain.static_index);
            }

            for i in 2..length {
                let opi = *ops.at(i)?;
                self.inherit_expression_dependencies(ops[1], opi)?;
                Self::add_implied_read_expression_chain(
                    self.get_mut::<SPIRAccessChain>(ops[1])?,
                    opi,
                );
            }
        } else {
            self.glsl_emit_instruction(instruction)?;
        }
        Ok(())
    }

    pub(crate) fn hlsl_emit_atomic(&mut self, ops: &[u32], length: u32, op: Op) -> Result<()> {
        let atomic_op;

        let mut value_expr = String::new();
        if op != OpAtomicIDecrement
            && op != OpAtomicIIncrement
            && op != OpAtomicLoad
            && op != OpAtomicStore
        {
            value_expr =
                self.to_expression(ops[if op == OpAtomicCompareExchange { 6 } else { 5 }], true)?;
        }

        let mut is_atomic_store = false;

        match op {
            OpAtomicIIncrement => {
                atomic_op = "InterlockedAdd";
                value_expr = "1".into();
            }

            OpAtomicIDecrement => {
                atomic_op = "InterlockedAdd";
                value_expr = "-1".into();
            }

            OpAtomicLoad => {
                atomic_op = "InterlockedAdd";
                value_expr = "0".into();
            }

            OpAtomicISub => {
                atomic_op = "InterlockedAdd";
                value_expr = join!("-", self.enclose_expression(&value_expr));
            }

            OpAtomicSMin | OpAtomicUMin => atomic_op = "InterlockedMin",

            OpAtomicSMax | OpAtomicUMax => atomic_op = "InterlockedMax",

            OpAtomicAnd => atomic_op = "InterlockedAnd",

            OpAtomicOr => atomic_op = "InterlockedOr",

            OpAtomicXor => atomic_op = "InterlockedXor",

            OpAtomicIAdd => atomic_op = "InterlockedAdd",

            OpAtomicExchange => atomic_op = "InterlockedExchange",

            OpAtomicStore => {
                atomic_op = "InterlockedExchange";
                is_atomic_store = true;
            }

            OpAtomicCompareExchange => {
                if length < 8 {
                    spirv_cross_throw!("Not enough data for opcode.");
                }
                atomic_op = "InterlockedCompareExchange";
                value_expr = join!(self.to_expression(ops[7], true)?, ", ", value_expr);
            }

            _ => spirv_cross_throw!("Unknown atomic opcode."),
        }

        if is_atomic_store {
            let data_type = self.expression_type_rc(ops[0])?;
            let chain = self.maybe_get_rc::<SPIRAccessChain>(ops[0]);

            let mut tmp_id = self
                .glsl
                .extra_sub_expressions
                .get(&ops[0])
                .copied()
                .unwrap_or(0);
            if tmp_id == 0 {
                tmp_id = self.ir.increase_bound_by(1)?;
                self.glsl.extra_sub_expressions.insert(ops[0], tmp_id);
                let pointee_self = self.get_pointee_type(&data_type)?.self_;
                self.emit_uninitialized_temporary_expression(pointee_self, tmp_id)?;
            }

            if data_type.storage == StorageClassImage || chain.is_none() {
                statement!(
                    self,
                    atomic_op,
                    "(",
                    self.to_non_uniform_aware_expression(ops[0])?,
                    ", ",
                    self.to_expression(ops[3], true)?,
                    ", ",
                    self.to_expression(tmp_id, true)?,
                    ");"
                );
            } else {
                let chain = chain.unwrap();
                let mut base = chain.base.clone();
                if self.has_decoration(chain.self_, DecorationNonUniform) {
                    self.convert_non_uniform_expression(&mut base, chain.self_)?;
                }
                // RWByteAddress buffer is always uint in its underlying type.
                statement!(
                    self,
                    base,
                    ".",
                    atomic_op,
                    "(",
                    chain.dynamic_index,
                    chain.static_index,
                    ", ",
                    self.to_expression(ops[3], true)?,
                    ", ",
                    self.to_expression(tmp_id, true)?,
                    ");"
                );
            }
        } else {
            let result_type = ops[0];
            let id = ops[1];
            self.forced_temporaries.insert(ops[1]);

            let type_ = self.get_rc::<SPIRType>(result_type)?;
            let name = self.to_name(id, true)?;
            statement!(self, self.variable_decl_type(&type_, &name, 0)?, ";");

            let data_type = self.expression_type_rc(ops[2])?;
            let chain = self.maybe_get_rc::<SPIRAccessChain>(ops[2]);
            let expr_type;
            if data_type.storage == StorageClassImage || chain.is_none() {
                statement!(
                    self,
                    atomic_op,
                    "(",
                    self.to_non_uniform_aware_expression(ops[2])?,
                    ", ",
                    value_expr,
                    ", ",
                    self.to_name(id, true)?,
                    ");"
                );
                expr_type = data_type.basetype;
            } else {
                let chain = chain.unwrap();
                // RWByteAddress buffer is always uint in its underlying type.
                let mut base = chain.base.clone();
                if self.has_decoration(chain.self_, DecorationNonUniform) {
                    self.convert_non_uniform_expression(&mut base, chain.self_)?;
                }
                expr_type = BaseType::UInt;
                statement!(
                    self,
                    base,
                    ".",
                    atomic_op,
                    "(",
                    chain.dynamic_index,
                    chain.static_index,
                    ", ",
                    value_expr,
                    ", ",
                    self.to_name(id, true)?,
                    ");"
                );
            }

            let name = self.to_name(id, true)?;
            let expr = self.bitcast_expression_type(&type_, expr_type, &name)?;
            self.set::<SPIRExpression>(id, SPIRExpression::new(expr, result_type, true))?;
        }
        self.flush_all_atomic_capable_variables()
    }

    pub(crate) fn hlsl_emit_subgroup_op(&mut self, i: &Instruction) -> Result<()> {
        if self.hlsl.options.shader_model < 60 {
            spirv_cross_throw!("Wave ops requires SM 6.0 or higher.");
        }

        let ops = self.stream(i)?;
        let op = i.op as Op;

        let result_type = ops[0];
        let id = ops[1];

        let scope = self.evaluate_constant_u32(ops[2])? as Scope;
        if scope != ScopeSubgroup {
            spirv_cross_throw!("Only subgroup scope is supported.");
        }

        // If we need to do implicit bitcasts, make sure we do it with the correct type.
        let integer_width = self.get_integer_width_for_instruction(i)?;
        let int_type = to_signed_basetype(integer_width)?;
        let uint_type = to_unsigned_basetype(integer_width)?;

        // HLSL_GROUP_OP(op, hlsl_op, supports_scan): `inclusive` is
        // make_inclusive_<hlsl_op>'s operator (None for the "" ones).
        macro_rules! hlsl_group_op {
            ($hlsl_op:literal, $supports_scan:expr, $inclusive:expr) => {{
                let operation = ops[3] as GroupOperation;
                if operation == GroupOperationReduce {
                    self.emit_unary_func_op(
                        result_type,
                        id,
                        ops[4],
                        concat!("WaveActive", $hlsl_op),
                    )?;
                } else if operation == GroupOperationInclusiveScan && $supports_scan {
                    let forward = self.should_forward(ops[4])?;
                    let prefix = join!(
                        concat!("WavePrefix", $hlsl_op),
                        "(",
                        self.to_expression(ops[4], true)?,
                        ")"
                    );
                    let inclusive: Option<&str> = $inclusive;
                    let expr = match inclusive {
                        Some(o) => join!(prefix, o, self.to_expression(ops[4], true)?),
                        None => String::new(),
                    };
                    self.emit_op(result_type, id, &expr, forward, false)?;
                    self.inherit_expression_dependencies(id, ops[4])?;
                } else if operation == GroupOperationExclusiveScan && $supports_scan {
                    self.emit_unary_func_op(
                        result_type,
                        id,
                        ops[4],
                        concat!("WavePrefix", $hlsl_op),
                    )?;
                } else if operation == GroupOperationClusteredReduce {
                    spirv_cross_throw!("Cannot trivially implement ClusteredReduce in HLSL.");
                } else {
                    spirv_cross_throw!("Invalid group operation.");
                }
            }};
        }

        macro_rules! hlsl_group_op_cast {
            ($hlsl_op:literal, $type:expr) => {{
                let operation = ops[3] as GroupOperation;
                if operation == GroupOperationReduce {
                    self.emit_unary_func_op_cast(
                        result_type,
                        id,
                        ops[4],
                        concat!("WaveActive", $hlsl_op),
                        $type,
                        $type,
                    )?;
                } else {
                    spirv_cross_throw!("Invalid group operation.");
                }
            }};
        }

        match op {
            OpGroupNonUniformElect => {
                self.emit_op(result_type, id, "WaveIsFirstLane()", true, false)?
            }

            OpGroupNonUniformBroadcast => {
                self.emit_binary_func_op(result_type, id, ops[3], ops[4], "WaveReadLaneAt")?
            }

            OpGroupNonUniformBroadcastFirst => {
                self.emit_unary_func_op(result_type, id, ops[3], "WaveReadLaneFirst")?
            }

            OpGroupNonUniformBallot => {
                self.emit_unary_func_op(result_type, id, ops[3], "WaveActiveBallot")?
            }

            OpGroupNonUniformInverseBallot => {
                spirv_cross_throw!("Cannot trivially implement InverseBallot in HLSL.");
            }

            OpGroupNonUniformBallotBitExtract => {
                spirv_cross_throw!("Cannot trivially implement BallotBitExtract in HLSL.");
            }

            OpGroupNonUniformBallotFindLSB => {
                spirv_cross_throw!("Cannot trivially implement BallotFindLSB in HLSL.");
            }

            OpGroupNonUniformBallotFindMSB => {
                spirv_cross_throw!("Cannot trivially implement BallotFindMSB in HLSL.");
            }

            OpGroupNonUniformBallotBitCount => {
                let operation = ops[3] as GroupOperation;
                let forward = self.should_forward(ops[4])?;
                if operation == GroupOperationReduce {
                    let left = join!(
                        "countbits(",
                        self.to_enclosed_expression(ops[4], true)?,
                        ".x) + countbits(",
                        self.to_enclosed_expression(ops[4], true)?,
                        ".y)"
                    );
                    let right = join!(
                        "countbits(",
                        self.to_enclosed_expression(ops[4], true)?,
                        ".z) + countbits(",
                        self.to_enclosed_expression(ops[4], true)?,
                        ".w)"
                    );
                    self.emit_op(result_type, id, &join!(left, " + ", right), forward, false)?;
                    self.inherit_expression_dependencies(id, ops[4])?;
                } else if operation == GroupOperationInclusiveScan {
                    let left = join!(
                        "countbits(",
                        self.to_enclosed_expression(ops[4], true)?,
                        ".x & gl_SubgroupLeMask.x) + countbits(",
                        self.to_enclosed_expression(ops[4], true)?,
                        ".y & gl_SubgroupLeMask.y)"
                    );
                    let right = join!(
                        "countbits(",
                        self.to_enclosed_expression(ops[4], true)?,
                        ".z & gl_SubgroupLeMask.z) + countbits(",
                        self.to_enclosed_expression(ops[4], true)?,
                        ".w & gl_SubgroupLeMask.w)"
                    );
                    self.emit_op(result_type, id, &join!(left, " + ", right), forward, false)?;
                    if !self.active_input_builtins.get(BuiltInSubgroupLeMask) {
                        self.active_input_builtins.set(BuiltInSubgroupLeMask);
                        self.force_recompile_guarantee_forward_progress();
                    }
                } else if operation == GroupOperationExclusiveScan {
                    let left = join!(
                        "countbits(",
                        self.to_enclosed_expression(ops[4], true)?,
                        ".x & gl_SubgroupLtMask.x) + countbits(",
                        self.to_enclosed_expression(ops[4], true)?,
                        ".y & gl_SubgroupLtMask.y)"
                    );
                    let right = join!(
                        "countbits(",
                        self.to_enclosed_expression(ops[4], true)?,
                        ".z & gl_SubgroupLtMask.z) + countbits(",
                        self.to_enclosed_expression(ops[4], true)?,
                        ".w & gl_SubgroupLtMask.w)"
                    );
                    self.emit_op(result_type, id, &join!(left, " + ", right), forward, false)?;
                    if !self.active_input_builtins.get(BuiltInSubgroupLtMask) {
                        self.active_input_builtins.set(BuiltInSubgroupLtMask);
                        self.force_recompile_guarantee_forward_progress();
                    }
                } else {
                    spirv_cross_throw!("Invalid BitCount operation.");
                }
            }

            OpGroupNonUniformShuffle => {
                self.emit_binary_func_op(result_type, id, ops[3], ops[4], "WaveReadLaneAt")?
            }
            OpGroupNonUniformShuffleXor => {
                let forward = self.should_forward(ops[3])?;
                let expr = join!(
                    "WaveReadLaneAt(",
                    self.to_unpacked_expression(ops[3], true)?,
                    ", ",
                    "WaveGetLaneIndex() ^ ",
                    self.to_enclosed_expression(ops[4], true)?,
                    ")"
                );
                self.emit_op(ops[0], ops[1], &expr, forward, false)?;
                self.inherit_expression_dependencies(ops[1], ops[3])?;
            }
            OpGroupNonUniformShuffleUp => {
                let forward = self.should_forward(ops[3])?;
                let expr = join!(
                    "WaveReadLaneAt(",
                    self.to_unpacked_expression(ops[3], true)?,
                    ", ",
                    "WaveGetLaneIndex() - ",
                    self.to_enclosed_expression(ops[4], true)?,
                    ")"
                );
                self.emit_op(ops[0], ops[1], &expr, forward, false)?;
                self.inherit_expression_dependencies(ops[1], ops[3])?;
            }
            OpGroupNonUniformShuffleDown => {
                let forward = self.should_forward(ops[3])?;
                let expr = join!(
                    "WaveReadLaneAt(",
                    self.to_unpacked_expression(ops[3], true)?,
                    ", ",
                    "WaveGetLaneIndex() + ",
                    self.to_enclosed_expression(ops[4], true)?,
                    ")"
                );
                self.emit_op(ops[0], ops[1], &expr, forward, false)?;
                self.inherit_expression_dependencies(ops[1], ops[3])?;
            }

            OpGroupNonUniformAll => {
                self.emit_unary_func_op(result_type, id, ops[3], "WaveActiveAllTrue")?
            }

            OpGroupNonUniformAny => {
                self.emit_unary_func_op(result_type, id, ops[3], "WaveActiveAnyTrue")?
            }

            OpGroupNonUniformAllEqual => {
                self.emit_unary_func_op(result_type, id, ops[3], "WaveActiveAllEqual")?
            }

            OpGroupNonUniformFAdd => hlsl_group_op!("Sum", true, Some(" + ")),
            OpGroupNonUniformFMul => hlsl_group_op!("Product", true, Some(" * ")),
            OpGroupNonUniformFMin => hlsl_group_op!("Min", false, None),
            OpGroupNonUniformFMax => hlsl_group_op!("Max", false, None),
            OpGroupNonUniformIAdd => hlsl_group_op!("Sum", true, Some(" + ")),
            OpGroupNonUniformIMul => hlsl_group_op!("Product", true, Some(" * ")),
            OpGroupNonUniformSMin => hlsl_group_op_cast!("Min", int_type),
            OpGroupNonUniformSMax => hlsl_group_op_cast!("Max", int_type),
            OpGroupNonUniformUMin => hlsl_group_op_cast!("Min", uint_type),
            OpGroupNonUniformUMax => hlsl_group_op_cast!("Max", uint_type),
            OpGroupNonUniformBitwiseAnd => hlsl_group_op!("BitAnd", false, None),
            OpGroupNonUniformBitwiseOr => hlsl_group_op!("BitOr", false, None),
            OpGroupNonUniformBitwiseXor => hlsl_group_op!("BitXor", false, None),
            OpGroupNonUniformLogicalAnd => hlsl_group_op_cast!("BitAnd", uint_type),
            OpGroupNonUniformLogicalOr => hlsl_group_op_cast!("BitOr", uint_type),
            OpGroupNonUniformLogicalXor => hlsl_group_op_cast!("BitXor", uint_type),

            OpGroupNonUniformQuadSwap => {
                let direction = self.evaluate_constant_u32(ops[4])?;
                if direction == 0 {
                    self.emit_unary_func_op(result_type, id, ops[3], "QuadReadAcrossX")?;
                } else if direction == 1 {
                    self.emit_unary_func_op(result_type, id, ops[3], "QuadReadAcrossY")?;
                } else if direction == 2 {
                    self.emit_unary_func_op(result_type, id, ops[3], "QuadReadAcrossDiagonal")?;
                } else {
                    spirv_cross_throw!("Invalid quad swap direction.");
                }
            }

            OpGroupNonUniformQuadBroadcast => {
                self.emit_binary_func_op(result_type, id, ops[3], ops[4], "QuadReadLaneAt")?;
            }

            _ => spirv_cross_throw!("Invalid opcode for subgroup."),
        }

        self.register_control_dependent_expression(id)
    }

    pub(crate) fn hlsl_emit_instruction(&mut self, instruction: &Instruction) -> Result<()> {
        let stream = self.stream(instruction)?;
        let ops: &[u32] = &stream;
        let mut opcode = instruction.op as Op;

        macro_rules! HLSL_BOP {
            ($op:expr) => {
                self.emit_binary_op(ops[0], ops[1], ops[2], ops[3], $op)?
            };
        }
        macro_rules! HLSL_BOP_CAST {
            ($op:expr, $type:expr) => {
                self.emit_binary_op_cast(
                    ops[0],
                    ops[1],
                    ops[2],
                    ops[3],
                    $op,
                    $type,
                    opcode_is_sign_invariant(opcode),
                    false,
                )?
            };
        }
        macro_rules! HLSL_UOP {
            ($op:expr) => {
                self.emit_unary_op(ops[0], ops[1], ops[2], $op)?
            };
        }
        macro_rules! HLSL_TFOP {
            ($op:expr) => {
                self.emit_trinary_func_op(ops[0], ops[1], ops[2], ops[3], ops[4], $op)?
            };
        }
        macro_rules! HLSL_UFOP {
            ($op:expr) => {
                self.emit_unary_func_op(ops[0], ops[1], ops[2], $op)?
            };
        }

        // If we need to do implicit bitcasts, make sure we do it with the correct type.
        let integer_width = self.get_integer_width_for_instruction(instruction)?;
        let int_type = to_signed_basetype(integer_width)?;
        let uint_type = to_unsigned_basetype(integer_width)?;

        opcode = self.get_remapped_spirv_op(opcode);

        match opcode {
            OpAccessChain | OpInBoundsAccessChain => {
                self.hlsl_emit_access_chain(instruction)?;
            }
            OpBitcast => {
                let bitcast_type = self.get_bitcast_type(ops[0], ops[2])?;
                if bitcast_type == BitcastType::TypeNormal {
                    self.glsl_emit_instruction(instruction)?;
                } else {
                    if !self.hlsl.requires_uint2_packing {
                        self.hlsl.requires_uint2_packing = true;
                        self.force_recompile();
                    }

                    if bitcast_type == BitcastType::TypePackUint2x32 {
                        self.emit_unary_func_op(ops[0], ops[1], ops[2], "spvPackUint2x32")?;
                    } else {
                        self.emit_unary_func_op(ops[0], ops[1], ops[2], "spvUnpackUint2x32")?;
                    }
                }
            }

            OpSelect => {
                let value_type = self.expression_type_rc(ops[3])?;
                if value_type.basetype == BaseType::Struct || self.is_array(&value_type) {
                    // HLSL does not support ternary expressions on composites.
                    // Cannot use branches, since we might be in a continue block
                    // where explicit control flow is prohibited.
                    // Emit a helper function where we can use control flow.
                    let value_type_id = self.expression_type_id(ops[3])?;
                    if !self
                        .hlsl
                        .composite_selection_workaround_types
                        .contains(&value_type_id)
                    {
                        self.hlsl
                            .composite_selection_workaround_types
                            .push(value_type_id);
                        self.force_recompile();
                    }
                    self.emit_uninitialized_temporary_expression(ops[0], ops[1])?;
                    statement!(
                        self,
                        "spvSelectComposite(",
                        self.to_expression(ops[1], true)?,
                        ", ",
                        self.to_expression(ops[2], true)?,
                        ", ",
                        self.to_expression(ops[3], true)?,
                        ", ",
                        self.to_expression(ops[4], true)?,
                        ");"
                    );
                } else {
                    self.glsl_emit_instruction(instruction)?;
                }
            }

            OpStore => {
                self.hlsl_emit_store(instruction)?;
            }

            OpLoad => {
                self.hlsl_emit_load(instruction)?;
            }

            OpMatrixTimesVector => {
                // Matrices are kept in a transposed state all the time, flip multiplication order always.
                self.emit_binary_func_op(ops[0], ops[1], ops[3], ops[2], "mul")?;
            }

            OpVectorTimesMatrix => {
                // Matrices are kept in a transposed state all the time, flip multiplication order always.
                self.emit_binary_func_op(ops[0], ops[1], ops[3], ops[2], "mul")?;
            }

            OpMatrixTimesMatrix => {
                // Matrices are kept in a transposed state all the time, flip multiplication order always.
                self.emit_binary_func_op(ops[0], ops[1], ops[3], ops[2], "mul")?;
            }

            OpOuterProduct => {
                let result_type = ops[0];
                let id = ops[1];
                let a = ops[2];
                let b = ops[3];

                let type_ = self.get_rc::<SPIRType>(result_type)?;
                let mut expr = self.type_to_glsl_constructor(&type_)?;
                expr += "(";
                for col in 0..type_.columns {
                    expr += &self.to_enclosed_expression(a, true)?;
                    expr += " * ";
                    expr += &self.to_extract_component_expression(b, col)?;
                    if col + 1 < type_.columns {
                        expr += ", ";
                    }
                }
                expr += ")";
                let forward = self.should_forward(a)? && self.should_forward(b)?;
                self.emit_op(result_type, id, &expr, forward, false)?;
                self.inherit_expression_dependencies(id, a)?;
                self.inherit_expression_dependencies(id, b)?;
            }

            OpFMod => {
                if !self.hlsl.requires_op_fmod {
                    self.hlsl.requires_op_fmod = true;
                    self.force_recompile();
                }
                self.glsl_emit_instruction(instruction)?;
            }

            OpFRem => self.emit_binary_func_op(ops[0], ops[1], ops[2], ops[3], "fmod")?,

            OpImage => {
                let result_type = ops[0];
                let id = ops[1];
                let combined = self.maybe_get_rc::<SPIRCombinedImageSampler>(ops[2]);

                if let Some(combined) = combined {
                    let expr = self.to_expression(combined.image, true)?;
                    self.emit_op(result_type, id, &expr, true, true)?;
                    if let Some(var) = self.maybe_get_backing_variable(combined.image) {
                        let var_self = self.get::<SPIRVariable>(var)?.self_;
                        self.get_mut::<SPIRExpression>(id)?.loaded_from = var_self;
                    }
                } else {
                    let expr = self.to_expression(ops[2], true)?;
                    self.emit_op(result_type, id, &expr, true, true)?;
                    if let Some(var) = self.maybe_get_backing_variable(ops[2]) {
                        let var_self = self.get::<SPIRVariable>(var)?.self_;
                        self.get_mut::<SPIRExpression>(id)?.loaded_from = var_self;
                    }
                }
            }

            OpDPdx => {
                HLSL_UFOP!("ddx");
                self.register_control_dependent_expression(ops[1])?;
            }

            OpDPdy => {
                HLSL_UFOP!("ddy");
                self.register_control_dependent_expression(ops[1])?;
            }

            OpDPdxFine => {
                HLSL_UFOP!("ddx_fine");
                self.register_control_dependent_expression(ops[1])?;
            }

            OpDPdyFine => {
                HLSL_UFOP!("ddy_fine");
                self.register_control_dependent_expression(ops[1])?;
            }

            OpDPdxCoarse => {
                HLSL_UFOP!("ddx_coarse");
                self.register_control_dependent_expression(ops[1])?;
            }

            OpDPdyCoarse => {
                HLSL_UFOP!("ddy_coarse");
                self.register_control_dependent_expression(ops[1])?;
            }

            OpFwidth | OpFwidthCoarse | OpFwidthFine => {
                HLSL_UFOP!("fwidth");
                self.register_control_dependent_expression(ops[1])?;
            }

            OpLogicalNot => {
                let result_type = ops[0];
                let id = ops[1];
                let vecsize = self.get::<SPIRType>(result_type)?.vecsize;

                if vecsize > 1 {
                    self.emit_unrolled_unary_op(result_type, id, ops[2], "!")?;
                } else {
                    HLSL_UOP!("!");
                }
            }

            OpIEqual => {
                let result_type = ops[0];
                let id = ops[1];

                if self.expression_type(ops[2])?.vecsize > 1 {
                    self.emit_unrolled_binary_op(
                        result_type,
                        id,
                        ops[2],
                        ops[3],
                        "==",
                        false,
                        BaseType::Unknown,
                    )?;
                } else {
                    HLSL_BOP_CAST!("==", int_type);
                }
            }

            OpLogicalEqual | OpFOrdEqual | OpFUnordEqual => {
                // HLSL != operator is unordered.
                // https://docs.microsoft.com/en-us/windows/win32/direct3d10/d3d10-graphics-programming-guide-resources-float-rules.
                // isnan() is apparently implemented as x != x as well.
                // We cannot implement UnordEqual as !(OrdNotEqual), as HLSL cannot express OrdNotEqual.
                // HACK: FUnordEqual will be implemented as FOrdEqual.

                let result_type = ops[0];
                let id = ops[1];

                if self.expression_type(ops[2])?.vecsize > 1 {
                    self.emit_unrolled_binary_op(
                        result_type,
                        id,
                        ops[2],
                        ops[3],
                        "==",
                        false,
                        BaseType::Unknown,
                    )?;
                } else {
                    HLSL_BOP!("==");
                }
            }

            OpINotEqual => {
                let result_type = ops[0];
                let id = ops[1];

                if self.expression_type(ops[2])?.vecsize > 1 {
                    self.emit_unrolled_binary_op(
                        result_type,
                        id,
                        ops[2],
                        ops[3],
                        "!=",
                        false,
                        BaseType::Unknown,
                    )?;
                } else {
                    HLSL_BOP_CAST!("!=", int_type);
                }
            }

            OpLogicalNotEqual | OpFOrdNotEqual | OpFUnordNotEqual => {
                // HLSL != operator is unordered.
                // https://docs.microsoft.com/en-us/windows/win32/direct3d10/d3d10-graphics-programming-guide-resources-float-rules.
                // isnan() is apparently implemented as x != x as well.

                // FIXME: FOrdNotEqual cannot be implemented in a crisp and simple way here.
                // We would need to do something like not(UnordEqual), but that cannot be expressed either.
                // Adding a lot of NaN checks would be a breaking change from perspective of performance.
                // SPIR-V will generally use isnan() checks when this even matters.
                // HACK: FOrdNotEqual will be implemented as FUnordEqual.

                let result_type = ops[0];
                let id = ops[1];

                if self.expression_type(ops[2])?.vecsize > 1 {
                    self.emit_unrolled_binary_op(
                        result_type,
                        id,
                        ops[2],
                        ops[3],
                        "!=",
                        false,
                        BaseType::Unknown,
                    )?;
                } else {
                    HLSL_BOP!("!=");
                }
            }

            OpUGreaterThan | OpSGreaterThan => {
                let result_type = ops[0];
                let id = ops[1];
                let type_ = if opcode == OpUGreaterThan {
                    uint_type
                } else {
                    int_type
                };

                if self.expression_type(ops[2])?.vecsize > 1 {
                    self.emit_unrolled_binary_op(
                        result_type,
                        id,
                        ops[2],
                        ops[3],
                        ">",
                        false,
                        type_,
                    )?;
                } else {
                    HLSL_BOP_CAST!(">", type_);
                }
            }

            OpFOrdGreaterThan => {
                let result_type = ops[0];
                let id = ops[1];

                if self.expression_type(ops[2])?.vecsize > 1 {
                    self.emit_unrolled_binary_op(
                        result_type,
                        id,
                        ops[2],
                        ops[3],
                        ">",
                        false,
                        BaseType::Unknown,
                    )?;
                } else {
                    HLSL_BOP!(">");
                }
            }

            OpFUnordGreaterThan => {
                let result_type = ops[0];
                let id = ops[1];

                if self.expression_type(ops[2])?.vecsize > 1 {
                    self.emit_unrolled_binary_op(
                        result_type,
                        id,
                        ops[2],
                        ops[3],
                        "<=",
                        true,
                        BaseType::Unknown,
                    )?;
                } else {
                    self.glsl_emit_instruction(instruction)?;
                }
            }

            OpUGreaterThanEqual | OpSGreaterThanEqual => {
                let result_type = ops[0];
                let id = ops[1];

                let type_ = if opcode == OpUGreaterThanEqual {
                    uint_type
                } else {
                    int_type
                };
                if self.expression_type(ops[2])?.vecsize > 1 {
                    self.emit_unrolled_binary_op(
                        result_type,
                        id,
                        ops[2],
                        ops[3],
                        ">=",
                        false,
                        type_,
                    )?;
                } else {
                    HLSL_BOP_CAST!(">=", type_);
                }
            }

            OpFOrdGreaterThanEqual => {
                let result_type = ops[0];
                let id = ops[1];

                if self.expression_type(ops[2])?.vecsize > 1 {
                    self.emit_unrolled_binary_op(
                        result_type,
                        id,
                        ops[2],
                        ops[3],
                        ">=",
                        false,
                        BaseType::Unknown,
                    )?;
                } else {
                    HLSL_BOP!(">=");
                }
            }

            OpFUnordGreaterThanEqual => {
                let result_type = ops[0];
                let id = ops[1];

                if self.expression_type(ops[2])?.vecsize > 1 {
                    self.emit_unrolled_binary_op(
                        result_type,
                        id,
                        ops[2],
                        ops[3],
                        "<",
                        true,
                        BaseType::Unknown,
                    )?;
                } else {
                    self.glsl_emit_instruction(instruction)?;
                }
            }

            OpULessThan | OpSLessThan => {
                let result_type = ops[0];
                let id = ops[1];

                let type_ = if opcode == OpULessThan {
                    uint_type
                } else {
                    int_type
                };
                if self.expression_type(ops[2])?.vecsize > 1 {
                    self.emit_unrolled_binary_op(
                        result_type,
                        id,
                        ops[2],
                        ops[3],
                        "<",
                        false,
                        type_,
                    )?;
                } else {
                    HLSL_BOP_CAST!("<", type_);
                }
            }

            OpFOrdLessThan => {
                let result_type = ops[0];
                let id = ops[1];

                if self.expression_type(ops[2])?.vecsize > 1 {
                    self.emit_unrolled_binary_op(
                        result_type,
                        id,
                        ops[2],
                        ops[3],
                        "<",
                        false,
                        BaseType::Unknown,
                    )?;
                } else {
                    HLSL_BOP!("<");
                }
            }

            OpFUnordLessThan => {
                let result_type = ops[0];
                let id = ops[1];

                if self.expression_type(ops[2])?.vecsize > 1 {
                    self.emit_unrolled_binary_op(
                        result_type,
                        id,
                        ops[2],
                        ops[3],
                        ">=",
                        true,
                        BaseType::Unknown,
                    )?;
                } else {
                    self.glsl_emit_instruction(instruction)?;
                }
            }

            OpULessThanEqual | OpSLessThanEqual => {
                let result_type = ops[0];
                let id = ops[1];

                let type_ = if opcode == OpULessThanEqual {
                    uint_type
                } else {
                    int_type
                };
                if self.expression_type(ops[2])?.vecsize > 1 {
                    self.emit_unrolled_binary_op(
                        result_type,
                        id,
                        ops[2],
                        ops[3],
                        "<=",
                        false,
                        type_,
                    )?;
                } else {
                    HLSL_BOP_CAST!("<=", type_);
                }
            }

            OpFOrdLessThanEqual => {
                let result_type = ops[0];
                let id = ops[1];

                if self.expression_type(ops[2])?.vecsize > 1 {
                    self.emit_unrolled_binary_op(
                        result_type,
                        id,
                        ops[2],
                        ops[3],
                        "<=",
                        false,
                        BaseType::Unknown,
                    )?;
                } else {
                    HLSL_BOP!("<=");
                }
            }

            OpFUnordLessThanEqual => {
                let result_type = ops[0];
                let id = ops[1];

                if self.expression_type(ops[2])?.vecsize > 1 {
                    self.emit_unrolled_binary_op(
                        result_type,
                        id,
                        ops[2],
                        ops[3],
                        ">",
                        true,
                        BaseType::Unknown,
                    )?;
                } else {
                    self.glsl_emit_instruction(instruction)?;
                }
            }

            OpImageQueryLod => self.emit_texture_op(instruction, false)?,

            OpImageQuerySizeLod => {
                let result_type = ops[0];
                let id = ops[1];

                self.require_texture_query_variant(ops[2])?;
                let dummy_samples_levels = join!(self.get_fallback_name(id), "_dummy_parameter");
                statement!(self, "uint ", dummy_samples_levels, ";");

                let mut expr = join!(
                    "spvTextureSize(",
                    self.to_non_uniform_aware_expression(ops[2])?,
                    ", ",
                    self.bitcast_expression(BaseType::UInt, ops[3])?,
                    ", ",
                    dummy_samples_levels,
                    ")"
                );

                let restype = self.get_rc::<SPIRType>(ops[0])?;
                expr = self.bitcast_expression_type(&restype, BaseType::UInt, &expr)?;
                self.emit_op(result_type, id, &expr, true, false)?;
            }

            OpImageQuerySize => {
                let result_type = ops[0];
                let id = ops[1];

                self.require_texture_query_variant(ops[2])?;
                let mut uav = self.expression_type(ops[2])?.image.sampled == 2;

                if let Some(var) = self.maybe_get_backing_variable(ops[2]) {
                    let var_self = self.get::<SPIRVariable>(var)?.self_;
                    if self.hlsl.options.nonwritable_uav_texture_as_srv
                        && self.has_decoration(var_self, DecorationNonWritable)
                    {
                        uav = false;
                    }
                }

                let dummy_samples_levels = join!(self.get_fallback_name(id), "_dummy_parameter");
                statement!(self, "uint ", dummy_samples_levels, ";");

                let mut expr = if uav {
                    join!(
                        "spvImageSize(",
                        self.to_non_uniform_aware_expression(ops[2])?,
                        ", ",
                        dummy_samples_levels,
                        ")"
                    )
                } else {
                    join!(
                        "spvTextureSize(",
                        self.to_non_uniform_aware_expression(ops[2])?,
                        ", 0u, ",
                        dummy_samples_levels,
                        ")"
                    )
                };

                let restype = self.get_rc::<SPIRType>(ops[0])?;
                expr = self.bitcast_expression_type(&restype, BaseType::UInt, &expr)?;
                self.emit_op(result_type, id, &expr, true, false)?;
            }

            OpImageQuerySamples | OpImageQueryLevels => {
                let result_type = ops[0];
                let id = ops[1];

                self.require_texture_query_variant(ops[2])?;
                let mut uav = self.expression_type(ops[2])?.image.sampled == 2;
                if opcode == OpImageQueryLevels && uav {
                    spirv_cross_throw!("Cannot query levels for UAV images.");
                }

                if let Some(var) = self.maybe_get_backing_variable(ops[2]) {
                    let var_self = self.get::<SPIRVariable>(var)?.self_;
                    if self.hlsl.options.nonwritable_uav_texture_as_srv
                        && self.has_decoration(var_self, DecorationNonWritable)
                    {
                        uav = false;
                    }
                }

                // Keep it simple and do not emit special variants to make this look nicer ...
                // This stuff is barely, if ever, used.
                self.forced_temporaries.insert(id);
                let type_ = self.get_rc::<SPIRType>(result_type)?;
                let name = self.to_name(id, true)?;
                statement!(self, self.variable_decl_type(&type_, &name, 0)?, ";");

                if uav {
                    statement!(
                        self,
                        "spvImageSize(",
                        self.to_non_uniform_aware_expression(ops[2])?,
                        ", ",
                        self.to_name(id, true)?,
                        ");"
                    );
                } else {
                    statement!(
                        self,
                        "spvTextureSize(",
                        self.to_non_uniform_aware_expression(ops[2])?,
                        ", 0u, ",
                        self.to_name(id, true)?,
                        ");"
                    );
                }

                let restype = self.get_rc::<SPIRType>(ops[0])?;
                let name = self.to_name(id, true)?;
                let expr = self.bitcast_expression_type(&restype, BaseType::UInt, &name)?;
                self.set::<SPIRExpression>(id, SPIRExpression::new(expr, result_type, true))?;
            }

            OpImageRead => {
                let result_type = ops[0];
                let id = ops[1];
                let var = self.maybe_get_backing_variable(ops[2]);
                let type_ = self.expression_type_rc(ops[2])?;
                let subpass_data = type_.image.dim == DimSubpassData;
                let pure;

                let mut imgexpr;

                if subpass_data {
                    if self.hlsl.options.shader_model < 40 {
                        spirv_cross_throw!(
                            "Subpass loads are not supported in HLSL shader model 2/3."
                        );
                    }

                    // Similar to GLSL, implement subpass loads using texelFetch.
                    if type_.image.ms {
                        let operands = ops[4];
                        if operands != ImageOperandsSampleMask || instruction.length != 6 {
                            spirv_cross_throw!(
                                "Multisampled image used in OpImageRead, but unexpected operand mask was used."
                            );
                        }
                        let sample = ops[5];
                        imgexpr = join!(
                            self.to_non_uniform_aware_expression(ops[2])?,
                            ".Load(int2(gl_FragCoord.xy), ",
                            self.to_expression(sample, true)?,
                            ")"
                        );
                    } else {
                        imgexpr = join!(
                            self.to_non_uniform_aware_expression(ops[2])?,
                            ".Load(int3(int2(gl_FragCoord.xy), 0))"
                        );
                    }

                    pure = true;
                } else {
                    imgexpr = join!(
                        self.to_non_uniform_aware_expression(ops[2])?,
                        "[",
                        self.to_expression(ops[3], true)?,
                        "]"
                    );
                    // The underlying image type in HLSL depends on the image format, unlike GLSL, where all images are "vec4",
                    // except that the underlying type changes how the data is interpreted.

                    let force_srv = match var {
                        Some(v) => {
                            self.hlsl.options.nonwritable_uav_texture_as_srv
                                && self.has_decoration(
                                    self.get::<SPIRVariable>(v)?.self_,
                                    DecorationNonWritable,
                                )
                        }
                        None => false,
                    };
                    pure = force_srv;

                    if let Some(v) = var {
                        if !subpass_data && !force_srv {
                            let var_basetype = self.get::<SPIRVariable>(v)?.basetype;
                            let format = self.get::<SPIRType>(var_basetype)?.image.format;
                            let result = self.get_rc::<SPIRType>(result_type)?;
                            imgexpr = self.remap_swizzle(
                                &result,
                                image_format_to_components(format)?,
                                &imgexpr,
                            )?;
                        }
                    }
                }

                if let Some(v) = var {
                    let forward = !self.forced_temporaries.contains(&id);
                    self.emit_op(result_type, id, &imgexpr, forward, false)?;

                    if !pure {
                        let var_self = self.get::<SPIRVariable>(v)?.self_;
                        self.get_mut::<SPIRExpression>(id)?.loaded_from = var_self;
                        if forward {
                            self.get_mut::<SPIRVariable>(v)?.dependees.push(id);
                        }
                    }
                } else {
                    self.emit_op(result_type, id, &imgexpr, false, false)?;
                }

                self.inherit_expression_dependencies(id, ops[2])?;
                if type_.image.ms {
                    self.inherit_expression_dependencies(id, ops[5])?;
                }
            }

            OpImageWrite => {
                let var = self.maybe_get_backing_variable(ops[0]);

                // The underlying image type in HLSL depends on the image format, unlike GLSL, where all images are "vec4",
                // except that the underlying type changes how the data is interpreted.
                let mut value_expr = self.to_expression(ops[2], true)?;
                if let Some(v) = var {
                    let var_basetype = self.get::<SPIRVariable>(v)?.basetype;
                    let type_ = self.get_rc::<SPIRType>(var_basetype)?;
                    let mut narrowed_type = self.get::<SPIRType>(type_.image.type_)?.clone();
                    narrowed_type.vecsize = image_format_to_components(type_.image.format)?;
                    let in_comps = self.expression_type(ops[2])?.vecsize;
                    value_expr = self.remap_swizzle(&narrowed_type, in_comps, &value_expr)?;
                }

                statement!(
                    self,
                    self.to_non_uniform_aware_expression(ops[0])?,
                    "[",
                    self.to_expression(ops[1], true)?,
                    "] = ",
                    value_expr,
                    ";"
                );
                if let Some(v) = var {
                    let v = self.get_rc::<SPIRVariable>(v)?;
                    if self.variable_storage_is_aliased(&v)? {
                        self.flush_all_aliased_variables()?;
                    }
                }
            }

            OpImageTexelPointer => {
                let result_type = ops[0];
                let id = ops[1];

                let mut expr = self.to_expression(ops[2], true)?;
                expr += &join!("[", self.to_expression(ops[3], true)?, "]");

                // When using the pointer, we need to know which variable it is actually loaded from.
                let loaded_from = match self.maybe_get_backing_variable(ops[2]) {
                    Some(v) => self.get::<SPIRVariable>(v)?.self_,
                    None => 0,
                };
                let e =
                    self.set::<SPIRExpression>(id, SPIRExpression::new(expr, result_type, true))?;
                e.loaded_from = loaded_from;
                self.inherit_expression_dependencies(id, ops[3])?;
            }

            OpAtomicFAddEXT | OpAtomicFMinEXT | OpAtomicFMaxEXT => {
                spirv_cross_throw!("Floating-point atomics are not supported in HLSL.");
            }

            OpAtomicCompareExchange
            | OpAtomicExchange
            | OpAtomicISub
            | OpAtomicSMin
            | OpAtomicUMin
            | OpAtomicSMax
            | OpAtomicUMax
            | OpAtomicAnd
            | OpAtomicOr
            | OpAtomicXor
            | OpAtomicIAdd
            | OpAtomicIIncrement
            | OpAtomicIDecrement
            | OpAtomicLoad
            | OpAtomicStore => {
                self.hlsl_emit_atomic(ops, instruction.length, opcode)?;
            }

            OpControlBarrier | OpMemoryBarrier => {
                let memory;
                let mut semantics;

                if opcode == OpMemoryBarrier {
                    memory = self.evaluate_constant_u32(ops[0])?;
                    semantics = self.evaluate_constant_u32(ops[1])?;
                } else {
                    memory = self.evaluate_constant_u32(ops[1])?;
                    semantics = self.evaluate_constant_u32(ops[2])?;
                }

                if memory == ScopeSubgroup {
                    // No Wave-barriers in HLSL.
                    return Ok(());
                }

                // We only care about these flags, acquire/release and friends are not relevant to GLSL.
                semantics = Self::mask_relevant_memory_semantics(semantics);

                if opcode == OpMemoryBarrier {
                    // If we are a memory barrier, and the next instruction is a control barrier, check if that memory barrier
                    // does what we need, so we avoid redundant barriers.
                    let next = self.get_next_instruction_in_block(instruction)?;
                    if let Some(next) = next.filter(|n| n.op as Op == OpControlBarrier) {
                        let next_ops = self.stream(&next)?;
                        let next_memory = self.evaluate_constant_u32(next_ops[1])?;
                        let mut next_semantics = self.evaluate_constant_u32(next_ops[2])?;
                        next_semantics = Self::mask_relevant_memory_semantics(next_semantics);

                        // There is no "just execution barrier" in HLSL.
                        // If there are no memory semantics for next instruction, we will imply group shared memory is synced.
                        if next_semantics == 0 {
                            next_semantics = MemorySemanticsWorkgroupMemoryMask;
                        }

                        let mut memory_scope_covered = false;
                        if next_memory == memory {
                            memory_scope_covered = true;
                        } else if next_semantics == MemorySemanticsWorkgroupMemoryMask {
                            // If we only care about workgroup memory, either Device or Workgroup scope is fine,
                            // scope does not have to match.
                            if (next_memory == ScopeDevice || next_memory == ScopeWorkgroup)
                                && (memory == ScopeDevice || memory == ScopeWorkgroup)
                            {
                                memory_scope_covered = true;
                            }
                        } else if memory == ScopeWorkgroup && next_memory == ScopeDevice {
                            // The control barrier has device scope, but the memory barrier just has workgroup scope.
                            memory_scope_covered = true;
                        }

                        // If we have the same memory scope, and all memory types are covered, we're good.
                        if memory_scope_covered && (semantics & next_semantics) == semantics {
                            return Ok(());
                        }
                    }
                }

                // We are synchronizing some memory or syncing execution,
                // so we cannot forward any loads beyond the memory barrier.
                if semantics != 0 || opcode == OpControlBarrier {
                    let Some(block) = self.glsl.current_emitting_block else {
                        spirv_cross_throw!("Barrier outside of a block.");
                    };
                    self.flush_control_dependent_expressions(block)?;
                    self.flush_all_active_variables()?;
                }

                if opcode == OpControlBarrier {
                    // We cannot emit just execution barrier, for no memory semantics pick the cheapest option.
                    if semantics == MemorySemanticsWorkgroupMemoryMask || semantics == 0 {
                        statement!(self, "GroupMemoryBarrierWithGroupSync();");
                    } else if semantics != 0
                        && (semantics & MemorySemanticsWorkgroupMemoryMask) == 0
                    {
                        statement!(self, "DeviceMemoryBarrierWithGroupSync();");
                    } else {
                        statement!(self, "AllMemoryBarrierWithGroupSync();");
                    }
                } else if semantics == MemorySemanticsWorkgroupMemoryMask {
                    statement!(self, "GroupMemoryBarrier();");
                } else if semantics != 0 && (semantics & MemorySemanticsWorkgroupMemoryMask) == 0 {
                    statement!(self, "DeviceMemoryBarrier();");
                } else {
                    statement!(self, "AllMemoryBarrier();");
                }
            }

            OpBitFieldInsert => {
                if !self.hlsl.requires_bitfield_insert {
                    self.hlsl.requires_bitfield_insert = true;
                    self.force_recompile();
                }

                let mut expr = join!(
                    "spvBitfieldInsert(",
                    self.to_expression(ops[2], true)?,
                    ", ",
                    self.to_expression(ops[3], true)?,
                    ", ",
                    self.to_expression(ops[4], true)?,
                    ", ",
                    self.to_expression(ops[5], true)?,
                    ")"
                );

                let forward = self.should_forward(ops[2])?
                    && self.should_forward(ops[3])?
                    && self.should_forward(ops[4])?
                    && self.should_forward(ops[5])?;

                let restype = self.get_rc::<SPIRType>(ops[0])?;
                expr = self.bitcast_expression_type(&restype, BaseType::UInt, &expr)?;
                self.emit_op(ops[0], ops[1], &expr, forward, false)?;
            }

            OpBitFieldSExtract | OpBitFieldUExtract => {
                if !self.hlsl.requires_bitfield_extract {
                    self.hlsl.requires_bitfield_extract = true;
                    self.force_recompile();
                }

                if opcode == OpBitFieldSExtract {
                    HLSL_TFOP!("spvBitfieldSExtract");
                } else {
                    HLSL_TFOP!("spvBitfieldUExtract");
                }
            }

            OpBitCount => {
                let basetype = self.expression_type(ops[2])?.basetype;
                self.emit_unary_func_op_cast(
                    ops[0],
                    ops[1],
                    ops[2],
                    "countbits",
                    basetype,
                    basetype,
                )?;
            }

            OpBitReverse => HLSL_UFOP!("reversebits"),

            OpArrayLength => {
                let Some(var) = self.maybe_get_backing_variable(ops[2]) else {
                    spirv_cross_throw!("Array length must point directly to an SSBO block.");
                };

                let var_basetype = self.get::<SPIRVariable>(var)?.basetype;
                let type_ = self.get_rc::<SPIRType>(var_basetype)?;
                if !self.has_decoration(type_.self_, DecorationBlock)
                    && !self.has_decoration(type_.self_, DecorationBufferBlock)
                {
                    spirv_cross_throw!("Array length expression must point to a block type.");
                }

                // This must be 32-bit uint, so we're good to go.
                self.emit_uninitialized_temporary_expression(ops[0], ops[1])?;
                statement!(
                    self,
                    self.to_non_uniform_aware_expression(ops[2])?,
                    ".GetDimensions(",
                    self.to_expression(ops[1], true)?,
                    ");"
                );
                let offset = self.type_struct_member_offset(&type_, ops[3])?;
                let stride = self.type_struct_member_array_stride(&type_, ops[3])?;
                statement!(
                    self,
                    self.to_expression(ops[1], true)?,
                    " = (",
                    self.to_expression(ops[1], true)?,
                    " - ",
                    offset,
                    ") / ",
                    stride,
                    ";"
                );
            }

            OpIsHelperInvocationEXT => {
                if self.hlsl.options.shader_model < 50
                    || self.get_entry_point().model != ExecutionModelFragment
                {
                    spirv_cross_throw!(
                        "Helper Invocation input is only supported in PS 5.0 or higher."
                    );
                }
                // Helper lane state with demote is volatile by nature.
                // Do not forward this.
                self.emit_op(ops[0], ops[1], "IsHelperLane()", false, false)?;
            }

            OpBeginInvocationInterlockEXT | OpEndInvocationInterlockEXT => {
                if self.hlsl.options.shader_model < 51 {
                    spirv_cross_throw!("Rasterizer order views require Shader Model 5.1.");
                }
                // Nothing to do in the body
            }

            OpRayQueryInitializeKHR => {
                self.flush_variable_declaration(ops[0])?;

                let ray_desc_name = self.get_unique_identifier();
                statement!(
                    self,
                    "RayDesc ",
                    ray_desc_name,
                    " = {",
                    self.to_expression(ops[4], true)?,
                    ", ",
                    self.to_expression(ops[5], true)?,
                    ", ",
                    self.to_expression(ops[6], true)?,
                    ", ",
                    self.to_expression(ops[7], true)?,
                    "};"
                );

                statement!(
                    self,
                    self.to_expression(ops[0], true)?,
                    ".TraceRayInline(",
                    self.to_expression(ops[1], true)?,
                    ", ", // acc structure
                    self.to_expression(ops[2], true)?,
                    ", ", // ray flags
                    self.to_expression(ops[3], true)?,
                    ", ", // mask
                    ray_desc_name,
                    ");" // ray
                );
            }
            OpRayQueryProceedKHR => {
                self.flush_variable_declaration(ops[0])?;
                let expr = join!(self.to_expression(ops[2], true)?, ".Proceed()");
                self.emit_op(ops[0], ops[1], &expr, false, false)?;
            }
            OpRayQueryTerminateKHR => {
                self.flush_variable_declaration(ops[0])?;
                statement!(self, self.to_expression(ops[0], true)?, ".Abort();");
            }
            OpRayQueryGenerateIntersectionKHR => {
                self.flush_variable_declaration(ops[0])?;
                statement!(
                    self,
                    self.to_expression(ops[0], true)?,
                    ".CommitProceduralPrimitiveHit(",
                    self.to_expression(ops[1], true)?,
                    ");"
                );
            }
            OpRayQueryConfirmIntersectionKHR => {
                self.flush_variable_declaration(ops[0])?;
                statement!(
                    self,
                    self.to_expression(ops[0], true)?,
                    ".CommitNonOpaqueTriangleHit();"
                );
            }
            OpRayQueryGetIntersectionTypeKHR => {
                self.emit_rayquery_function(".CommittedStatus()", ".CandidateType()", ops)?;
            }
            OpRayQueryGetIntersectionTKHR => {
                self.emit_rayquery_function(".CommittedRayT()", ".CandidateTriangleRayT()", ops)?;
            }
            OpRayQueryGetIntersectionInstanceCustomIndexKHR => {
                self.emit_rayquery_function(
                    ".CommittedInstanceID()",
                    ".CandidateInstanceID()",
                    ops,
                )?;
            }
            OpRayQueryGetIntersectionInstanceIdKHR => {
                self.emit_rayquery_function(
                    ".CommittedInstanceIndex()",
                    ".CandidateInstanceIndex()",
                    ops,
                )?;
            }
            OpRayQueryGetIntersectionInstanceShaderBindingTableRecordOffsetKHR => {
                self.emit_rayquery_function(
                    ".CommittedInstanceContributionToHitGroupIndex()",
                    ".CandidateInstanceContributionToHitGroupIndex()",
                    ops,
                )?;
            }
            OpRayQueryGetIntersectionGeometryIndexKHR => {
                self.emit_rayquery_function(
                    ".CommittedGeometryIndex()",
                    ".CandidateGeometryIndex()",
                    ops,
                )?;
            }
            OpRayQueryGetIntersectionPrimitiveIndexKHR => {
                self.emit_rayquery_function(
                    ".CommittedPrimitiveIndex()",
                    ".CandidatePrimitiveIndex()",
                    ops,
                )?;
            }
            OpRayQueryGetIntersectionBarycentricsKHR => {
                self.emit_rayquery_function(
                    ".CommittedTriangleBarycentrics()",
                    ".CandidateTriangleBarycentrics()",
                    ops,
                )?;
            }
            OpRayQueryGetIntersectionFrontFaceKHR => {
                self.emit_rayquery_function(
                    ".CommittedTriangleFrontFace()",
                    ".CandidateTriangleFrontFace()",
                    ops,
                )?;
            }
            OpRayQueryGetIntersectionCandidateAABBOpaqueKHR => {
                self.flush_variable_declaration(ops[0])?;
                let expr = join!(
                    self.to_expression(ops[2], true)?,
                    ".CandidateProceduralPrimitiveNonOpaque()"
                );
                self.emit_op(ops[0], ops[1], &expr, false, false)?;
            }
            OpRayQueryGetIntersectionObjectRayDirectionKHR => {
                self.emit_rayquery_function(
                    ".CommittedObjectRayDirection()",
                    ".CandidateObjectRayDirection()",
                    ops,
                )?;
            }
            OpRayQueryGetIntersectionObjectRayOriginKHR => {
                self.flush_variable_declaration(ops[0])?;
                self.emit_rayquery_function(
                    ".CommittedObjectRayOrigin()",
                    ".CandidateObjectRayOrigin()",
                    ops,
                )?;
            }
            OpRayQueryGetIntersectionObjectToWorldKHR => {
                self.emit_rayquery_function(
                    ".CommittedObjectToWorld4x3()",
                    ".CandidateObjectToWorld4x3()",
                    ops,
                )?;
            }
            OpRayQueryGetIntersectionWorldToObjectKHR => {
                self.emit_rayquery_function(
                    ".CommittedWorldToObject4x3()",
                    ".CandidateWorldToObject4x3()",
                    ops,
                )?;
            }
            OpRayQueryGetRayFlagsKHR => {
                self.flush_variable_declaration(ops[0])?;
                let expr = join!(self.to_expression(ops[2], true)?, ".RayFlags()");
                self.emit_op(ops[0], ops[1], &expr, false, false)?;
            }
            OpRayQueryGetRayTMinKHR => {
                self.flush_variable_declaration(ops[0])?;
                let expr = join!(self.to_expression(ops[2], true)?, ".RayTMin()");
                self.emit_op(ops[0], ops[1], &expr, false, false)?;
            }
            OpRayQueryGetWorldRayOriginKHR => {
                self.flush_variable_declaration(ops[0])?;
                let expr = join!(self.to_expression(ops[2], true)?, ".WorldRayOrigin()");
                self.emit_op(ops[0], ops[1], &expr, false, false)?;
            }
            OpRayQueryGetWorldRayDirectionKHR => {
                self.flush_variable_declaration(ops[0])?;
                let expr = join!(self.to_expression(ops[2], true)?, ".WorldRayDirection()");
                self.emit_op(ops[0], ops[1], &expr, false, false)?;
            }
            OpSetMeshOutputsEXT => {
                statement!(
                    self,
                    "SetMeshOutputCounts(",
                    self.to_unpacked_expression(ops[0], true)?,
                    ", ",
                    self.to_unpacked_expression(ops[1], true)?,
                    ");"
                );
            }
            OpEmitVertex => {
                self.emit_geometry_stream_append()?;
            }
            OpEndPrimitive => {
                statement!(self, "geometry_stream.RestartStrip();");
            }
            _ => self.glsl_emit_instruction(instruction)?,
        }
        Ok(())
    }

    pub(crate) fn require_texture_query_variant(&mut self, mut var_id: u32) -> Result<()> {
        if let Some(var) = self.maybe_get_backing_variable(var_id) {
            var_id = self.get::<SPIRVariable>(var)?.self_;
        }

        let type_ = self.expression_type_rc(var_id)?;
        let mut uav = type_.image.sampled == 2;
        if self.hlsl.options.nonwritable_uav_texture_as_srv
            && self.has_decoration(var_id, DecorationNonWritable)
        {
            uav = false;
        }

        let mut bit: u32 = match type_.image.dim {
            Dim1D => {
                if type_.image.arrayed {
                    Query1DArray
                } else {
                    Query1D
                }
            }

            Dim2D => {
                if type_.image.ms {
                    if type_.image.arrayed {
                        Query2DMSArray
                    } else {
                        Query2DMS
                    }
                } else if type_.image.arrayed {
                    Query2DArray
                } else {
                    Query2D
                }
            }

            Dim3D => Query3D,

            DimCube => {
                if type_.image.arrayed {
                    QueryCubeArray
                } else {
                    QueryCube
                }
            }

            DimBuffer => QueryBuffer,

            _ => spirv_cross_throw!("Unsupported query type."),
        };

        match self.get::<SPIRType>(type_.image.type_)?.basetype {
            BaseType::Float => bit += QueryTypeFloat,

            BaseType::Int => bit += QueryTypeInt,

            BaseType::UInt => bit += QueryTypeUInt,

            _ => spirv_cross_throw!("Unsupported query type."),
        }

        let norm_state = image_format_to_normalized_state(type_.image.format);
        let components = image_format_to_components(type_.image.format)?;
        let variant = if uav {
            &mut self.hlsl.required_texture_size_variants.uav[norm_state as usize]
                [(components - 1) as usize]
        } else {
            &mut self.hlsl.required_texture_size_variants.srv
        };

        let mask = 1u64 << bit;
        if (*variant & mask) == 0 {
            *variant |= mask;
            self.force_recompile();
        }
        Ok(())
    }

    /// `CompilerHLSL::set_root_constant_layouts()`.
    pub fn set_root_constant_layouts(&mut self, layout: Vec<RootConstants>) {
        self.hlsl.root_constants_layout = layout;
    }

    /// `CompilerHLSL::add_vertex_attribute_remap()`.
    pub fn add_vertex_attribute_remap(&mut self, vertex_attributes: &HLSLVertexAttributeRemap) {
        self.hlsl
            .remap_vertex_attributes
            .push(vertex_attributes.clone());
    }

    /// `CompilerHLSL::remap_num_workgroups_builtin()`.
    pub fn remap_num_workgroups_builtin(&mut self) -> Result<VariableID> {
        self.update_active_builtins()?;

        if !self.active_input_builtins.get(BuiltInNumWorkgroups) {
            return Ok(0);
        }

        // Create a new, fake UBO.
        let offset = self.ir.increase_bound_by(4)?;

        let uint_type_id = offset;
        let block_type_id = offset + 1;
        let block_pointer_type_id = offset + 2;
        let variable_id = offset + 3;

        let mut uint_type = SPIRType::new(OpTypeVector);
        uint_type.basetype = BaseType::UInt;
        uint_type.width = 32;
        uint_type.vecsize = 3;
        uint_type.columns = 1;
        self.set::<SPIRType>(uint_type_id, uint_type)?;

        let mut block_type = SPIRType::new(OpTypeStruct);
        block_type.basetype = BaseType::Struct;
        block_type.member_types.push(uint_type_id);
        let block_type = self.set::<SPIRType>(block_type_id, block_type)?.clone();
        self.set_decoration(block_type_id, DecorationBlock, 0);
        self.set_member_name(block_type_id, 0, "count")?;
        self.set_member_decoration(block_type_id, 0, DecorationOffset, 0)?;

        let mut block_pointer_type = block_type;
        block_pointer_type.pointer = true;
        block_pointer_type.storage = StorageClassUniform;
        block_pointer_type.parent_type = block_type_id;
        let ptr_type = self.set::<SPIRType>(block_pointer_type_id, block_pointer_type)?;

        // Preserve self.
        ptr_type.self_ = block_type_id;

        self.set::<SPIRVariable>(
            variable_id,
            SPIRVariable::new(block_pointer_type_id, StorageClassUniform, 0, 0),
        )?;
        self.meta_mut(variable_id).decoration.alias = "SPIRV_Cross_NumWorkgroups".into();

        self.hlsl.num_workgroups_builtin = variable_id;
        let nwb = self.hlsl.num_workgroups_builtin;
        self.get_entry_point_mut().interface_variables.push(nwb);
        Ok(variable_id)
    }

    /// `CompilerHLSL::set_resource_binding_flags()`.
    pub fn set_resource_binding_flags(&mut self, flags: HLSLBindingFlags) {
        self.hlsl.resource_binding_flags = flags;
    }

    pub(crate) fn validate_shader_model(&self) -> Result<()> {
        // Check for nonuniform qualifier.
        // Instead of looping over all decorations to find this, just look at capabilities.
        for &cap in &self.ir.declared_capabilities {
            match cap {
                CapabilityShaderNonUniformEXT | CapabilityRuntimeDescriptorArrayEXT => {
                    if self.hlsl.options.shader_model < 51 {
                        spirv_cross_throw!(
                            "Shader model 5.1 or higher is required to use bindless resources or NonUniformResourceIndex."
                        );
                    }
                }

                CapabilityVariablePointers | CapabilityVariablePointersStorageBuffer => {
                    spirv_cross_throw!("VariablePointers capability is not supported in HLSL.");
                }

                _ => {}
            }
        }

        if self.ir.addressing_model != AddressingModelLogical {
            spirv_cross_throw!("Only Logical addressing model can be used with HLSL.");
        }

        if self.hlsl.options.enable_16bit_types && self.hlsl.options.shader_model < 62 {
            spirv_cross_throw!(
                "Need at least shader model 6.2 when enabling native 16-bit type support."
            );
        }
        Ok(())
    }

    /// `CompilerHLSL::compile()`.
    pub(crate) fn hlsl_compile(&mut self) -> Result<String> {
        self.ir.fixup_reserved_names();

        // Do not deal with ES-isms like precision, older extensions and such.
        self.glsl.options.es = false;
        self.glsl.options.version = 450;
        self.glsl.options.vulkan_semantics = true;
        let force_merged_mesh_block = self.get_execution_model() == ExecutionModelMeshEXT;
        let shader_model = self.hlsl.options.shader_model;
        let backend = &mut self.glsl.backend;
        backend.float_literal_suffix = true;
        backend.double_literal_suffix = false;
        backend.long_long_literal_suffix = true;
        backend.uint32_t_literal_suffix = true;
        backend.int16_t_literal_suffix = "";
        backend.uint16_t_literal_suffix = "u";
        backend.basic_int_type = "int";
        backend.basic_uint_type = "uint";
        backend.demote_literal = "discard".into();
        backend.boolean_mix_function = "";
        backend.swizzle_is_function = false;
        backend.shared_is_implied = true;
        backend.unsized_array_supported = true;
        backend.explicit_struct_type = false;
        backend.use_initializer_list = true;
        backend.use_constructor_splatting = false;
        backend.can_swizzle_scalar = true;
        backend.can_declare_struct_inline = false;
        backend.can_declare_arrays_inline = false;
        backend.can_return_array = false;
        backend.nonuniform_qualifier = "NonUniformResourceIndex";
        backend.support_case_fallthrough = false;
        backend.requires_phi_undef_zero_init = true;
        backend.force_merged_mesh_block = force_merged_mesh_block;
        backend.force_gl_in_out_block = backend.force_merged_mesh_block;
        backend.supports_empty_struct = shader_model <= 30;

        // SM 4.1 does not support precise for some reason.
        backend.support_precise_qualifier = shader_model >= 50 || shader_model == 40;

        self.fixup_anonymous_struct_names()?;
        self.fixup_type_alias()?;
        self.reorder_type_alias()?;
        self.build_function_control_flow_graphs_and_analyze()?;
        self.validate_shader_model()?;
        self.update_active_builtins()?;
        self.analyze_image_and_sampler_usage()?;
        self.analyze_interlocked_resource_usage()?;
        if self.get_execution_model() == ExecutionModelMeshEXT {
            self.analyze_meshlet_writes()?;
        }

        if self.get_execution_model() == ExecutionModelGeometry {
            self.discover_geometry_emitters()?;
        }

        // Subpass input needs SV_Position.
        if self.need_subpass_input {
            self.active_input_builtins.set(BuiltInFragCoord);
        }

        // Need to offset by BaseVertex/BaseInstance in SM 6.8+.
        if self.hlsl.options.shader_model >= 68 {
            if self.active_input_builtins.get(BuiltInVertexIndex) {
                self.active_input_builtins.set(BuiltInBaseVertex);
            }
            if self.active_input_builtins.get(BuiltInInstanceIndex) {
                self.active_input_builtins.set(BuiltInBaseInstance);
            }
        }

        let mut pass_count: u32 = 0;
        loop {
            self.reset(pass_count)?;

            // Move constructor for this type is broken on GCC 4.9 ...
            self.glsl.buffer.clear();

            self.emit_header()?;
            self.hlsl_emit_resources()?;

            let entry = self.ir.default_entry_point;
            self.emit_function(entry, &Bitset::default())?;
            self.emit_hlsl_entry_point()?;

            pass_count += 1;
            if !self.is_forcing_recompilation() {
                break;
            }
        }

        // Entry point in HLSL is always main() for the time being.
        self.get_entry_point_mut().name = "main".into();

        Ok(self.glsl.buffer.clone())
    }

    pub(crate) fn hlsl_emit_block_hints(&mut self, block: &SPIRBlock) -> Result<()> {
        match block.hint {
            Hints::HintFlatten => statement!(self, "[flatten]"),
            Hints::HintDontFlatten => statement!(self, "[branch]"),
            Hints::HintUnroll => statement!(self, "[unroll]"),
            Hints::HintDontUnroll => statement!(self, "[loop]"),
            _ => {}
        }
        Ok(())
    }

    pub(crate) fn get_unique_identifier(&mut self) -> String {
        let n = self.hlsl.unique_identifier_count;
        self.hlsl.unique_identifier_count = n.wrapping_add(1);
        join!("_", n, "ident")
    }

    /// `CompilerHLSL::add_hlsl_resource_binding()`.
    pub fn add_hlsl_resource_binding(&mut self, binding: &HLSLResourceBinding) {
        let tuple = StageSetBinding {
            model: binding.stage,
            desc_set: binding.desc_set,
            binding: binding.binding,
        };
        self.hlsl.resource_bindings.insert(tuple, (*binding, false));
    }

    /// `CompilerHLSL::is_hlsl_resource_binding_used()`.
    pub fn is_hlsl_resource_binding_used(
        &self,
        model: ExecutionModel,
        desc_set: u32,
        binding: u32,
    ) -> bool {
        let tuple = StageSetBinding {
            model,
            desc_set,
            binding,
        };
        self.hlsl.resource_bindings.get(&tuple).is_some_and(|r| r.1)
    }

    pub(crate) fn get_bitcast_type(&self, result_type: u32, op0: u32) -> Result<BitcastType> {
        let rslt_type = self.get::<SPIRType>(result_type)?;
        let expr_type = self.expression_type(op0)?;

        if rslt_type.basetype == BaseType::UInt64
            && expr_type.basetype == BaseType::UInt
            && expr_type.vecsize == 2
        {
            return Ok(BitcastType::TypePackUint2x32);
        } else if rslt_type.basetype == BaseType::UInt
            && rslt_type.vecsize == 2
            && expr_type.basetype == BaseType::UInt64
        {
            return Ok(BitcastType::TypeUnpackUint64);
        }

        Ok(BitcastType::TypeNormal)
    }

    pub(crate) fn is_hlsl_force_storage_buffer_as_uav(&self, id: ID) -> bool {
        if self.hlsl.options.force_storage_buffer_as_uav {
            return true;
        }

        let desc_set = self.get_decoration(id, DecorationDescriptorSet);
        let binding = self.get_decoration(id, DecorationBinding);

        self.hlsl
            .force_uav_buffer_bindings
            .contains(&SetBindingPair { desc_set, binding })
    }

    pub(crate) fn is_hidden_io_variable(&self, var: &SPIRVariable) -> Result<bool> {
        if !self.is_hidden_variable(var, false)? {
            return Ok(false);
        }

        // It is too risky to remove stage IO variables that are linkable since it affects link compatibility.
        // For vertex inputs and fragment outputs, it's less of a concern and we want reflection data
        // to match reality.

        let is_external_linkage = (self.get_execution_model() == ExecutionModelVertex
            && var.storage == StorageClassInput)
            || (self.get_execution_model() == ExecutionModelFragment
                && var.storage == StorageClassOutput);

        if !is_external_linkage {
            return Ok(false);
        }

        // Unused output I/O variables might still be required to implement framebuffer fetch.
        if var.storage == StorageClassOutput
            && !self.is_legacy()
            && self
                .location_is_framebuffer_fetch(self.get_decoration(var.self_, DecorationLocation))
        {
            return Ok(false);
        }

        Ok(true)
    }

    /// `CompilerHLSL::set_hlsl_force_storage_buffer_as_uav()`.
    pub fn set_hlsl_force_storage_buffer_as_uav(&mut self, desc_set: u32, binding: u32) {
        let pair = SetBindingPair { desc_set, binding };
        self.hlsl.force_uav_buffer_bindings.insert(pair);
    }

    pub(crate) fn hlsl_is_user_type_structured(&self, id: u32) -> Result<bool> {
        if self.hlsl.options.preserve_structured_buffers {
            // Compare left hand side of string only as these user types can contain more meta data such as their subtypes,
            // e.g. "structuredbuffer:int"
            let user_type = self
                .get_decoration_string(id, DecorationUserTypeGOOGLE)
                .as_bytes();
            let starts = |n: usize, s: &str| -> bool {
                user_type[..user_type.len().min(n)] == s.as_bytes()[..]
            };
            return Ok(starts(16, "structuredbuffer")
                || starts(18, "rwstructuredbuffer")
                || starts(35, "globallycoherent rwstructuredbuffer")
                || starts(33, "rasterizerorderedstructuredbuffer"));
        }
        Ok(false)
    }

    pub(crate) fn hlsl_cast_to_variable_store(
        &mut self,
        target_id: u32,
        expr: &mut String,
        expr_type: &SPIRType,
    ) -> Result<()> {
        // Loading a full array of ClipDistance needs special consideration in mesh shaders
        // since we cannot lower them by wrapping the variables in global statics.
        // Fortunately, clip/cull is a proper vector in HLSL so we can lower with simple rvalue casts.
        if self.get_execution_model() != ExecutionModelMeshEXT
            || !self.has_decoration(target_id, DecorationBuiltIn)
            || !self.is_array(expr_type)
        {
            return self.glsl_cast_to_variable_store(target_id, expr, expr_type);
        }

        let builtin = self.get_decoration(target_id, DecorationBuiltIn) as BuiltIn;
        if builtin != BuiltInClipDistance && builtin != BuiltInCullDistance {
            return self.glsl_cast_to_variable_store(target_id, expr, expr_type);
        }

        // Array of array means one thread is storing clip distance for all vertices. Nonsensical?
        if self.is_array(self.get::<SPIRType>(expr_type.parent_type)?) {
            spirv_cross_throw!(
                "Attempting to store all mesh vertices in one go. This is not supported."
            );
        }

        let num_clip = self.to_array_size_literal_last(expr_type)?;
        if num_clip > 4 {
            spirv_cross_throw!(
                "Number of clip or cull distances exceeds 4, this will not work with mesh shaders."
            );
        }

        if num_clip == 1 {
            // We already emit array here.
            return self.glsl_cast_to_variable_store(target_id, expr, expr_type);
        }

        let mut unrolled_expr = join!("float", num_clip, "(");
        for i in 0..num_clip {
            unrolled_expr += &join!(expr, "[", i, "]");
            if i + 1 < num_clip {
                unrolled_expr += ", ";
            }
        }

        unrolled_expr += ")";
        *expr = unrolled_expr;
        Ok(())
    }
}

// CompilerHLSL doesn't override image_type_glsl() (its type_to_glsl() calls
// image_type_hlsl() instead): the dispatcher's HLSL arm is CompilerGLSL's.
impl Compiler {
    pub(crate) fn hlsl_image_type_glsl(
        &mut self,
        type_: &SPIRType,
        id: u32,
        member: bool,
    ) -> Result<String> {
        self.glsl_image_type_glsl(type_, id, member)
    }
}
