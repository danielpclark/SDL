// Rust translation of spirv_cross.hpp and spirv_cross.cpp from
// SPIRV-Cross.
// Copyright 2015-2021 Arm Limited
// SPDX-License-Identifier: Apache-2.0 OR MIT
// This is an altered (translated to Rust) version of the original
// software; see LICENSE.txt.

//! `Compiler`: the reflection API and the analysis passes the backends
//! share.
//!
//! C++'s class hierarchy (`Compiler`, `CompilerGLSL` and its subclasses
//! `CompilerHLSL` and `CompilerMSL`) becomes one struct holding the state
//! of every level, with the backend recorded in [`Backend`]: a virtual
//! method is a method that dispatches on it, and the overrides are the
//! backend modules' `hlsl_*`/`msl_*` methods (`glsl_*` for CompilerGLSL's
//! version a subclass may call). References C++ keeps to IR objects are
//! ids here, looked up again where the C++ code reads through the
//! reference.

use std::collections::{HashMap, HashSet};
use std::ops::Deref;
use std::rc::Rc;

use super::cfg::{DominatorBuilder, CFG};
use super::common::*;
use super::parsed_ir::*;
use super::parser::Parser;
use super::spirv::*;
use super::std_hash::{StdHashMap, StdHashSet};

/// `Resource`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Resource {
    // Resources are identified with their SPIR-V ID.
    // This is the ID of the OpVariable.
    pub id: ID,

    // The type ID of the variable which includes arrays and all type modifications.
    // This type ID is not suitable for parsing OpMemberDecoration of a struct and other decorations in general
    // since these modifications typically happen on the base_type_id.
    pub type_id: TypeID,

    // The base type of the declared resource.
    // This type is the base type which ignores pointers and arrays of the type_id.
    // This is mostly useful to parse decorations of the underlying type.
    // base_type_id can also be obtained with get_type(get_type(type_id).self).
    pub base_type_id: TypeID,

    // The declared name (OpName) of the resource.
    // For Buffer blocks, the name actually reflects the externally
    // visible Block name.
    //
    // This name can be retrieved again by using either
    // get_name(id) or get_name(base_type_id) depending if it's a buffer block or not.
    //
    // This name can be an empty string in which case get_fallback_name(id) can be
    // used which obtains a suitable fallback identifier for an ID.
    pub name: String,
}

/// `BuiltInResource`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct BuiltInResource {
    // This is mostly here to support reflection of builtins such as Position/PointSize/CullDistance/ClipDistance.
    // This needs to be different from Resource since we can collect builtins from blocks.
    // A builtin present here does not necessarily mean it's considered an active builtin,
    // since variable ID "activeness" is only tracked on OpVariable level, not Block members.
    // For that, update_active_builtins() -> has_active_builtin() can be used to further refine the reflection.
    pub builtin: BuiltIn,

    // This is the actual value type of the builtin.
    // Typically float4, float, array<float, N> for the gl_PerVertex builtins.
    // If the builtin is a control point, the control point array type will be stripped away here as appropriate.
    pub value_type_id: TypeID,

    // This refers to the base resource which contains the builtin.
    // If resource is a Block, it can hold multiple builtins, or it might not be a block.
    // For advanced reflection scenarios, all information in builtin/value_type_id can be deduced,
    // it's just more convenient this way.
    pub resource: Resource,
}

/// `enum ResourceType`: needs to stay in sync 1:1 with C API.
pub type ResourceType = u32;
pub const ResourceTypeUnknown: ResourceType = 0;
pub const ResourceTypeUniformBuffer: ResourceType = 1;
pub const ResourceTypeStorageBuffer: ResourceType = 2;
pub const ResourceTypeStageInput: ResourceType = 3;
pub const ResourceTypeStageOutput: ResourceType = 4;
pub const ResourceTypeSubpassInput: ResourceType = 5;
pub const ResourceTypeStorageImage: ResourceType = 6;
pub const ResourceTypeSampledImage: ResourceType = 7;
pub const ResourceTypeAtomicCounter: ResourceType = 8;
pub const ResourceTypePushConstant: ResourceType = 9;
pub const ResourceTypeSeparateImage: ResourceType = 10;
pub const ResourceTypeSeparateSamplers: ResourceType = 11;
pub const ResourceTypeAccelerationStructure: ResourceType = 12;
pub const ResourceTypeRayQuery: ResourceType = 13;
pub const ResourceTypeShaderRecordBuffer: ResourceType = 14;
pub const ResourceTypeGLPlainUniform: ResourceType = 15;
pub const ResourceTypeTensor: ResourceType = 16;

/// `ShaderResources`.
#[derive(Clone, Debug, Default)]
pub struct ShaderResources {
    pub uniform_buffers: Vec<Resource>,
    pub storage_buffers: Vec<Resource>,
    pub stage_inputs: Vec<Resource>,
    pub stage_outputs: Vec<Resource>,
    pub subpass_inputs: Vec<Resource>,
    pub storage_images: Vec<Resource>,
    pub sampled_images: Vec<Resource>,
    pub atomic_counters: Vec<Resource>,
    pub acceleration_structures: Vec<Resource>,
    pub gl_plain_uniforms: Vec<Resource>,
    pub tensors: Vec<Resource>,

    // There can only be one push constant block,
    // but keep the vector in case this restriction is lifted in the future.
    pub push_constant_buffers: Vec<Resource>,

    pub shader_record_buffers: Vec<Resource>,

    // For Vulkan GLSL and HLSL source,
    // these correspond to separate texture2D and samplers respectively.
    pub separate_images: Vec<Resource>,
    pub separate_samplers: Vec<Resource>,

    pub builtin_inputs: Vec<BuiltInResource>,
    pub builtin_outputs: Vec<BuiltInResource>,
}

/// `CombinedImageSampler`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CombinedImageSampler {
    // The ID of the sampler2D variable.
    pub combined_id: VariableID,
    // The ID of the texture2D variable.
    pub image_id: VariableID,
    // The ID of the sampler variable.
    pub sampler_id: VariableID,
}

/// `SpecializationConstant`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SpecializationConstant {
    // The ID of the specialization constant.
    pub id: ConstantID,
    // The constant ID of the constant, used in Vulkan during pipeline creation.
    pub constant_id: u32,
}

/// `BufferRange`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct BufferRange {
    pub index: u32,
    pub offset: usize,
    pub range: usize,
}

/// `enum BufferPackingStandard`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BufferPackingStandard {
    BufferPackingStd140,
    BufferPackingStd430,
    BufferPackingStd140EnhancedLayout,
    BufferPackingStd430EnhancedLayout,
    BufferPackingHLSLCbuffer,
    BufferPackingHLSLCbufferPackOffset,
    BufferPackingScalar,
    BufferPackingScalarEnhancedLayout,
}

/// `EntryPoint`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct EntryPoint {
    pub name: String,
    pub execution_model: ExecutionModel,
}

/// `PhysicalBlockMeta`.
#[derive(Clone, Copy, Debug, Default)]
pub struct PhysicalBlockMeta {
    pub alignment: u32,
}

/// `DescriptorHeapMeta`.
#[derive(Clone, Copy, Debug, Default)]
pub struct DescriptorHeapMeta {
    pub type_: TypeID,
    pub hlsl_style_stride: bool,

    // For buffers
    pub buffer_pointer_id: ID,
    pub storage: StorageClass,
    pub nonwritable: bool,
    pub nonreadable: bool,
    pub coherent: bool,
    pub is_volatile: bool,
    pub is_restrict: bool,
}

/// Which class of the C++ hierarchy the compiler is.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Backend {
    /// `Compiler` itself: reflection only.
    #[default]
    None,
    /// `CompilerGLSL`.
    Glsl,
    /// `CompilerHLSL`.
    Hlsl,
    /// `CompilerMSL`.
    Msl,
}

/// How many words of zeros follow the SPIR-V for [`Stream`]: C++ reads
/// operands past an instruction's length into the words that follow it;
/// this keeps such reads (up to the longest instruction) in range at the
/// end of the module, where C++ would read past its buffer.
pub(crate) const STREAM_PADDING: usize = 0x10000;

/// `stream(instr)`: an instruction's operands, from the SPIR-V buffer to
/// its end (and its padding).
#[derive(Clone)]
pub struct Stream {
    data: Rc<Vec<u32>>,
    start: usize,
}

impl Deref for Stream {
    type Target = [u32];

    #[inline]
    fn deref(&self) -> &[u32] {
        &self.data[self.start..]
    }
}

/// `OpcodeHandler`: used internally to implement various traversals for
/// queries. The compiler is passed to each callback (C++ keeps a
/// reference to it in the handler).
pub(crate) trait OpcodeHandler {
    // Return true if traversal should continue.
    // If false, traversal will end immediately.
    fn handle(
        &mut self,
        compiler: &mut Compiler,
        opcode: Op,
        args: &[u32],
        length: u32,
    ) -> Result<bool>;

    fn handle_terminator(&mut self, _compiler: &mut Compiler, _block: &SPIRBlock) -> Result<bool> {
        Ok(true)
    }

    fn follow_function_call(
        &mut self,
        _compiler: &mut Compiler,
        _func: &SPIRFunction,
    ) -> Result<bool> {
        Ok(true)
    }

    fn set_current_block(&mut self, _compiler: &mut Compiler, _block: &SPIRBlock) -> Result<()> {
        Ok(())
    }

    // Called after returning from a function or when entering a block,
    // can be called multiple times per block,
    // while set_current_block is only called on block entry.
    fn rearm_current_block(&mut self, _compiler: &mut Compiler, _block: &SPIRBlock) -> Result<()> {
        Ok(())
    }

    fn begin_function_scope(
        &mut self,
        _compiler: &mut Compiler,
        _args: &[u32],
        _length: u32,
    ) -> Result<bool> {
        Ok(true)
    }

    fn end_function_scope(
        &mut self,
        _compiler: &mut Compiler,
        _args: &[u32],
        _length: u32,
    ) -> Result<bool> {
        Ok(true)
    }

    /// `result_types`, when the handler keeps them (`enable_result_types`).
    fn result_types(&mut self) -> Option<&mut HashMap<u32, u32>> {
        None
    }
}

/// `OpcodeHandler::get_expression_result_type()`.
pub(crate) fn get_expression_result_type<'a>(
    compiler: &'a Compiler,
    result_types: &HashMap<u32, u32>,
    id: u32,
) -> Result<Option<&'a SPIRType>> {
    match result_types.get(&id) {
        None => Ok(None),
        Some(&t) => Ok(Some(compiler.get::<SPIRType>(t)?)),
    }
}

// The deepest call graph the traversals follow; a deeper one is treated
// as the function recursion SPIR-V forbids (which overflows the stack in
// C++).
pub(crate) const MAX_CALL_DEPTH: u32 = 256;

/// `Compiler`, with the state of `CompilerGLSL`, `CompilerHLSL` and
/// `CompilerMSL`.
pub struct Compiler {
    pub(crate) backend: Backend,

    pub(crate) ir: ParsedIR,
    // The SPIR-V words with STREAM_PADDING zeros after them, for stream().
    pub(crate) spirv_stream: Rc<Vec<u32>>,
    pub(crate) zero_stream: Rc<Vec<u32>>,
    pub(crate) traversal_depth: u32,
    // The nesting depth of type_to_glsl() (the translation's guard).
    pub(crate) type_name_depth: u32,

    // Marks variables which have global scope and variables which can alias with other variables
    // (SSBO, image load store, etc)
    pub(crate) global_variables: Vec<u32>,
    pub(crate) aliased_variables: Vec<u32>,
    pub(crate) buffer_pointer_variables: Vec<u32>,

    // C++ keeps pointers to these; here, their ids.
    pub(crate) current_function: Option<u32>,
    pub(crate) current_block: Option<u32>,
    pub(crate) current_loop_level: u32,
    pub(crate) active_interface_variables: HashSet<VariableID>,
    pub(crate) check_active_interface_variables: bool,

    pub(crate) invalid_expressions: HashSet<u32>,

    pub(crate) is_force_recompile: bool,
    pub(crate) is_force_recompile_forward_progress: bool,

    pub(crate) combined_image_samplers: Vec<CombinedImageSampler>,

    // This must be an ordered data structure so we always pick the same type aliases.
    pub(crate) global_struct_cache: Vec<u32>,

    pub(crate) variable_remap_callback: Option<VariableTypeRemapCallback>,

    pub(crate) forced_temporaries: HashSet<u32>,
    pub(crate) forwarded_temporaries: HashSet<u32>,
    pub(crate) suppressed_usage_tracking: HashSet<u32>,
    pub(crate) hoisted_temporaries: HashSet<u32>,
    pub(crate) forced_invariant_temporaries: HashSet<u32>,

    pub(crate) active_input_builtins: Bitset,
    pub(crate) active_output_builtins: Bitset,
    pub(crate) clip_distance_count: u32,
    pub(crate) cull_distance_count: u32,
    pub(crate) position_invariant: bool,

    // If a variable ID or parameter ID is found in this set, a sampler is actually a shadow/comparison sampler.
    // SPIR-V does not support this distinction, so we must keep track of this information outside the type system.
    // There might be unrelated IDs found in this set which do not correspond to actual variables.
    // This set should only be queried for the existence of samplers which are already known to be variables or parameter IDs.
    // Similar is implemented for images, as well as if subpass inputs are needed.
    pub(crate) comparison_ids: HashSet<u32>,
    pub(crate) need_subpass_input: bool,
    pub(crate) need_subpass_input_ms: bool,

    // In certain backends, we will need to use a dummy sampler to be able to emit code.
    // GLSL does not support texelFetch on texture2D objects, but SPIR-V does,
    // so we need to workaround by having the application inject a dummy sampler.
    pub(crate) dummy_sampler_id: u32,

    pub(crate) function_cfgs: StdHashMap<Rc<CFG>>,

    pub(crate) physical_storage_non_block_pointer_types: Vec<u32>,
    pub(crate) physical_storage_type_to_alignment: HashMap<u32, PhysicalBlockMeta>,

    pub(crate) descriptor_heap_types: Vec<DescriptorHeapMeta>,

    // The set of all resources written while inside the critical section, if present.
    pub(crate) interlocked_resources: HashSet<u32>,
    pub(crate) interlocked_is_complex: bool,

    pub(crate) declared_block_names: HashMap<u32, String>,

    // An empty entry point, for get_entry_point() when the default entry
    // point is missing (which C++ dereferences).
    empty_entry_point: SPIREntryPoint,

    // CompilerGLSL's state.
    pub(crate) glsl: Box<super::glsl::GlslState>,
    // CompilerHLSL's state.
    pub(crate) hlsl: Box<super::hlsl::HlslState>,
    // CompilerMSL's state.
    pub(crate) msl: Box<super::msl::MslState>,
}

impl std::fmt::Debug for Compiler {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Compiler")
            .field("backend", &self.backend)
            .finish_non_exhaustive()
    }
}

#[inline]
fn storage_class_is_interface(storage: StorageClass) -> bool {
    matches!(
        storage,
        StorageClassInput
            | StorageClassOutput
            | StorageClassUniform
            | StorageClassUniformConstant
            | StorageClassAtomicCounter
            | StorageClassPushConstant
            | StorageClassStorageBuffer
    )
}

fn get_default_extended_decoration(decoration: ExtendedDecorations) -> u32 {
    match decoration {
        SPIRVCrossDecorationResourceIndexPrimary
        | SPIRVCrossDecorationResourceIndexSecondary
        | SPIRVCrossDecorationResourceIndexTertiary
        | SPIRVCrossDecorationResourceIndexQuaternary
        | SPIRVCrossDecorationInterfaceMemberIndex => !0u32,

        _ => 0,
    }
}

impl Compiler {
    fn blank(backend: Backend) -> Compiler {
        Compiler {
            backend,
            ir: ParsedIR::new(),
            spirv_stream: Rc::new(Vec::new()),
            zero_stream: Rc::new(Vec::new()),
            traversal_depth: 0,
            type_name_depth: 0,
            global_variables: Vec::new(),
            aliased_variables: Vec::new(),
            buffer_pointer_variables: Vec::new(),
            current_function: None,
            current_block: None,
            current_loop_level: 0,
            active_interface_variables: HashSet::new(),
            check_active_interface_variables: false,
            invalid_expressions: HashSet::new(),
            is_force_recompile: false,
            is_force_recompile_forward_progress: false,
            combined_image_samplers: Vec::new(),
            global_struct_cache: Vec::new(),
            variable_remap_callback: None,
            forced_temporaries: HashSet::new(),
            forwarded_temporaries: HashSet::new(),
            suppressed_usage_tracking: HashSet::new(),
            hoisted_temporaries: HashSet::new(),
            forced_invariant_temporaries: HashSet::new(),
            active_input_builtins: Bitset::default(),
            active_output_builtins: Bitset::default(),
            clip_distance_count: 0,
            cull_distance_count: 0,
            position_invariant: false,
            comparison_ids: HashSet::new(),
            need_subpass_input: false,
            need_subpass_input_ms: false,
            dummy_sampler_id: 0,
            function_cfgs: StdHashMap::new(),
            physical_storage_non_block_pointer_types: Vec::new(),
            physical_storage_type_to_alignment: HashMap::new(),
            descriptor_heap_types: Vec::new(),
            interlocked_resources: HashSet::new(),
            interlocked_is_complex: false,
            declared_block_names: HashMap::new(),
            empty_entry_point: SPIREntryPoint::default(),
            glsl: Box::default(),
            hlsl: Box::default(),
            msl: Box::default(),
        }
    }

    /// `Compiler(std::vector<uint32_t> ir)`: parses the SPIR-V words.
    pub fn from_spirv(spirv: Vec<u32>) -> Result<Compiler> {
        let mut parser = Parser::new(spirv);
        parser.parse()?;
        Self::from_ir(parser.into_parsed_ir())
    }

    /// `Compiler(ParsedIR &&ir)`: a reflection-only compiler.
    pub fn from_ir(ir: ParsedIR) -> Result<Compiler> {
        Self::with_backend(ir, Backend::None)
    }

    /// The constructor of the given class: `Compiler(ParsedIR &&)`, then
    /// the subclass constructor (`CompilerGLSL::init()` and so on).
    pub(crate) fn with_backend(ir: ParsedIR, backend: Backend) -> Result<Compiler> {
        let mut c = Self::blank(backend);
        c.set_ir(ir)?;
        match backend {
            Backend::None => {}
            Backend::Glsl => c.glsl_init()?,
            Backend::Hlsl => c.hlsl_init()?,
            Backend::Msl => c.msl_init()?,
        }
        Ok(c)
    }

    pub(crate) fn set_ir(&mut self, ir: ParsedIR) -> Result<()> {
        self.ir = ir;
        let mut padded = Vec::new();
        padded
            .try_reserve(self.ir.spirv.len() + STREAM_PADDING)
            .map_err(|_| CompilerError::new("std::bad_alloc"))?;
        padded.extend_from_slice(&self.ir.spirv);
        padded.resize(self.ir.spirv.len() + STREAM_PADDING, 0);
        self.spirv_stream = Rc::new(padded);
        self.zero_stream = Rc::new(vec![0u32; STREAM_PADDING]);
        self.parse_fixup()
    }

    /// `compile()` (virtual): the base class emits nothing.
    pub fn compile(&mut self) -> Result<String> {
        match self.backend {
            Backend::None => Ok(String::new()),
            Backend::Glsl => self.glsl_compile(),
            Backend::Hlsl => self.hlsl_compile(),
            Backend::Msl => self.msl_compile(),
        }
    }

    // Accessors for the IR objects (`get<T>()`, `maybe_get<T>()`, `set<T>()`).

    /// `get<T>(id)`.
    #[inline]
    pub(crate) fn get<T: IVariant>(&self, id: u32) -> Result<&T> {
        self.ir.get::<T>(id)
    }

    /// `get<T>(id)`, to modify the object.
    #[inline]
    pub(crate) fn get_mut<T: IVariant>(&mut self, id: u32) -> Result<&mut T> {
        self.ir.get_mut::<T>(id)
    }

    /// `get<T>(id)` as a snapshot that doesn't borrow the compiler.
    #[inline]
    pub(crate) fn get_rc<T: IVariant>(&self, id: u32) -> Result<Rc<T>> {
        self.ir.get_rc::<T>(id)
    }

    /// `maybe_get<T>(id)`.
    #[inline]
    pub(crate) fn maybe_get<T: IVariant>(&self, id: u32) -> Option<&T> {
        match self.ir.ids.get(id as usize) {
            Some(v) if v.get_type() == T::TYPE => v.get::<T>().ok(),
            _ => None,
        }
    }

    /// `maybe_get<T>(id)`, to modify the object.
    #[inline]
    pub(crate) fn maybe_get_mut<T: IVariant>(&mut self, id: u32) -> Option<&mut T> {
        match self.ir.ids.get_mut(id as usize) {
            Some(v) if v.get_type() == T::TYPE => v.get_mut::<T>().ok(),
            _ => None,
        }
    }

    /// `maybe_get<T>(id)` as a snapshot.
    #[inline]
    pub(crate) fn maybe_get_rc<T: IVariant>(&self, id: u32) -> Option<Rc<T>> {
        match self.ir.ids.get(id as usize) {
            Some(v) if v.get_type() == T::TYPE => v.get_rc::<T>().ok(),
            _ => None,
        }
    }

    /// `set<T>(id, args...)`.
    pub(crate) fn set<T: IVariant>(&mut self, id: u32, val: T) -> Result<&mut T> {
        // If our IDs are out of range here as part of opcodes, throw instead of
        // undefined behavior.
        self.ir.add_typed_id(T::TYPE, id)?;
        let level = self.current_loop_level;
        let var = variant_set(self.ir.ids.at_mut(id)?, val)?;
        var.set_self(id);
        var.set_initializers(level);
        Ok(var)
    }

    /// `ids[id].get_type()` (TypeNone when out of range, where C++ reads
    /// past the array).
    #[inline]
    pub(crate) fn id_type(&self, id: u32) -> Types {
        self.ir.id_type(id)
    }

    pub(crate) fn stream(&self, instr: &Instruction) -> Result<Stream> {
        // If we're not going to use any arguments, just return nullptr.
        // We want to avoid case where we return an out of range pointer
        // that trips debug assertions on some platforms.
        // (Here, a stream of zeros.)
        if instr.length == 0 {
            return Ok(Stream {
                data: self.zero_stream.clone(),
                start: 0,
            });
        }

        if let Some(embedded) = &instr.embedded {
            // assert(embedded.ops.size() == instr.length);
            let mut v = Vec::with_capacity(embedded.len() + STREAM_PADDING);
            v.extend_from_slice(embedded);
            v.resize(embedded.len() + STREAM_PADDING, 0);
            return Ok(Stream {
                data: Rc::new(v),
                start: 0,
            });
        }

        if instr.offset as usize + instr.length as usize > self.ir.spirv.len() {
            spirv_cross_throw!("Compiler::stream() out of range.");
        }
        Ok(Stream {
            data: self.spirv_stream.clone(),
            start: instr.offset as usize,
        })
    }

    /// `stream_mutable(instr)[i] = value`.
    pub(crate) fn stream_write(&mut self, instr: &Instruction, i: usize, value: u32) -> Result<()> {
        if instr.length == 0 || instr.embedded.is_some() {
            // (C++ writes through a null pointer or into the temporary
            // embedded instruction.)
            return Ok(());
        }
        let pos = instr.offset as usize + i;
        if pos >= self.ir.spirv.len() {
            spirv_cross_throw!("Compiler::stream() out of range.");
        }
        self.ir.spirv[pos] = value;
        Rc::make_mut(&mut self.spirv_stream)[pos] = value;
        Ok(())
    }
}

impl Compiler {
    pub(crate) fn variable_storage_is_aliased(&mut self, v: &SPIRVariable) -> Result<bool> {
        let type_ = self.get::<SPIRType>(v.basetype)?;

        // Untyped pointer, assume full aliasing.
        if type_.basetype == BaseType::Void {
            return Ok(true);
        }

        let type_self = type_.self_;
        let ssbo = v.storage == StorageClassStorageBuffer
            || self
                .ir
                .meta
                .entry(type_self)
                .or_default()
                .decoration
                .decoration_flags
                .get(DecorationBufferBlock);
        let type_ = self.get::<SPIRType>(v.basetype)?;
        let image = type_.basetype == BaseType::Image;
        let counter = type_.basetype == BaseType::AtomicCounter;
        let buffer_reference = type_.storage == StorageClassPhysicalStorageBuffer;

        let is_restrict = if ssbo {
            self.ir.get_buffer_block_flags(v)?.get(DecorationRestrict)
        } else {
            self.has_decoration(v.self_, DecorationRestrict)
        };

        Ok(!is_restrict && (ssbo || image || counter || buffer_reference))
    }

    pub(crate) fn block_is_control_dependent(&self, block: &SPIRBlock) -> Result<bool> {
        self.block_is_control_dependent_inner(block, 0)
    }

    fn block_is_control_dependent_inner(&self, block: &SPIRBlock, depth: u32) -> Result<bool> {
        for i in &block.ops {
            let ops = self.stream(i)?;
            let op = i.op as Op;

            match op {
                OpFunctionCall => {
                    let func = ops[2];
                    if self.function_is_control_dependent_inner(self.get::<SPIRFunction>(func)?, depth + 1)? {
                        return Ok(true);
                    }
                }

                // Derivatives
                OpDPdx | OpDPdxCoarse | OpDPdxFine | OpDPdy | OpDPdyCoarse | OpDPdyFine | OpFwidth | OpFwidthCoarse
                | OpFwidthFine
                // Anything implicit LOD
                | OpImageSampleImplicitLod
                | OpImageSampleDrefImplicitLod
                | OpImageSampleProjImplicitLod
                | OpImageSampleProjDrefImplicitLod
                | OpImageSparseSampleImplicitLod
                | OpImageSparseSampleDrefImplicitLod
                | OpImageSparseSampleProjImplicitLod
                | OpImageSparseSampleProjDrefImplicitLod
                | OpImageQueryLod
                | OpImageDrefGather
                | OpImageGather
                | OpImageSparseDrefGather
                | OpImageSparseGather
                // Anything subgroups
                | OpGroupNonUniformElect
                | OpGroupNonUniformAll
                | OpGroupNonUniformAny
                | OpGroupNonUniformAllEqual
                | OpGroupNonUniformBroadcast
                | OpGroupNonUniformBroadcastFirst
                | OpGroupNonUniformBallot
                | OpGroupNonUniformInverseBallot
                | OpGroupNonUniformBallotBitExtract
                | OpGroupNonUniformBallotBitCount
                | OpGroupNonUniformBallotFindLSB
                | OpGroupNonUniformBallotFindMSB
                | OpGroupNonUniformShuffle
                | OpGroupNonUniformShuffleXor
                | OpGroupNonUniformShuffleUp
                | OpGroupNonUniformShuffleDown
                | OpGroupNonUniformIAdd
                | OpGroupNonUniformFAdd
                | OpGroupNonUniformIMul
                | OpGroupNonUniformFMul
                | OpGroupNonUniformSMin
                | OpGroupNonUniformUMin
                | OpGroupNonUniformFMin
                | OpGroupNonUniformSMax
                | OpGroupNonUniformUMax
                | OpGroupNonUniformFMax
                | OpGroupNonUniformBitwiseAnd
                | OpGroupNonUniformBitwiseOr
                | OpGroupNonUniformBitwiseXor
                | OpGroupNonUniformLogicalAnd
                | OpGroupNonUniformLogicalOr
                | OpGroupNonUniformLogicalXor
                | OpGroupNonUniformQuadBroadcast
                | OpGroupNonUniformQuadSwap
                | OpGroupNonUniformRotateKHR
                // Control barriers
                | OpControlBarrier => return Ok(true),

                _ => {}
            }
        }

        Ok(false)
    }

    pub(crate) fn block_is_pure(&self, block: &SPIRBlock) -> Result<bool> {
        self.block_is_pure_inner(block, 0)
    }

    fn block_is_pure_inner(&self, block: &SPIRBlock, depth: u32) -> Result<bool> {
        // This is a global side effect of the function.
        if block.terminator == Terminator::Kill
            || block.terminator == Terminator::TerminateRay
            || block.terminator == Terminator::IgnoreIntersection
            || block.terminator == Terminator::EmitMeshTasks
        {
            return Ok(false);
        }

        for i in &block.ops {
            let ops = self.stream(i)?;
            let op = i.op as Op;

            match op {
                OpFunctionCall => {
                    let func = ops[2];
                    if !self.function_is_pure_inner(self.get::<SPIRFunction>(func)?, depth + 1)? {
                        return Ok(false);
                    }
                }

                OpCopyMemory | OpStore | OpCooperativeMatrixStoreKHR => {
                    let type_ = self.expression_type(ops[0])?;
                    if type_.storage != StorageClassFunction {
                        return Ok(false);
                    }
                }

                OpImageWrite => return Ok(false),

                // Atomics are impure.
                OpAtomicLoad
                | OpAtomicStore
                | OpAtomicExchange
                | OpAtomicCompareExchange
                | OpAtomicCompareExchangeWeak
                | OpAtomicIIncrement
                | OpAtomicIDecrement
                | OpAtomicIAdd
                | OpAtomicISub
                | OpAtomicSMin
                | OpAtomicUMin
                | OpAtomicSMax
                | OpAtomicUMax
                | OpAtomicAnd
                | OpAtomicOr
                | OpAtomicXor => return Ok(false),

                // Geometry shader builtins modify global state.
                OpEndPrimitive | OpEmitStreamVertex | OpEndStreamPrimitive | OpEmitVertex => {
                    return Ok(false)
                }

                // Mesh shader functions modify global state.
                // (EmitMeshTasks is a terminator).
                OpSetMeshOutputsEXT => return Ok(false),

                // Barriers disallow any reordering, so we should treat blocks with barrier as writing.
                OpControlBarrier | OpMemoryBarrier => return Ok(false),

                // Ray tracing builtins are impure.
                OpReportIntersectionKHR
                | OpIgnoreIntersectionNV
                | OpTerminateRayNV
                | OpTraceNV
                | OpTraceRayKHR
                | OpExecuteCallableNV
                | OpExecuteCallableKHR
                | OpRayQueryInitializeKHR
                | OpRayQueryTerminateKHR
                | OpRayQueryGenerateIntersectionKHR
                | OpRayQueryConfirmIntersectionKHR
                | OpRayQueryProceedKHR => {
                    // There are various getters in ray query, but they are considered pure.
                    return Ok(false);
                }

                // OpExtInst is potentially impure depending on extension, but GLSL builtins are at least pure.
                OpDemoteToHelperInvocationEXT => {
                    // This is a global side effect of the function.
                    return Ok(false);
                }

                OpTensorReadARM => return Ok(false),

                OpExtInst => {
                    let extension_set = ops[2];
                    if self.get::<SPIRExtension>(extension_set)?.ext == Extension::GLSL {
                        let op_450 = ops[3];
                        if op_450 == GLSLstd450Modf || op_450 == GLSLstd450Frexp {
                            let type_ = self.expression_type(ops[5])?;
                            if type_.storage != StorageClassFunction {
                                return Ok(false);
                            }
                        }
                    }
                }

                _ => {}
            }
        }

        Ok(true)
    }

    /// `to_name()` (virtual).
    pub(crate) fn to_name(&self, id: u32, allow_alias: bool) -> Result<String> {
        match self.backend {
            Backend::Msl => self.msl_to_name(id, allow_alias),
            _ => self.base_to_name(id, allow_alias),
        }
    }

    /// `Compiler::to_name()`.
    pub(crate) fn base_to_name(&self, id: u32, allow_alias: bool) -> Result<String> {
        self.base_to_name_inner(id, allow_alias, 0)
    }

    fn base_to_name_inner(&self, id: u32, allow_alias: bool, depth: u32) -> Result<String> {
        if allow_alias && self.id_type(id) == TypeType {
            // If this type is a simple alias, emit the
            // name of the original type instead.
            // We don't want to override the meta alias
            // as that can be overridden by the reflection APIs after parse.
            let type_ = self.get::<SPIRType>(id)?;
            if type_.type_alias != 0 {
                // If the alias master has been specially packed, we will have emitted a clean variant as well,
                // so skip the name aliasing here.
                if !self.has_extended_decoration(
                    type_.type_alias,
                    SPIRVCrossDecorationBufferBlockRepacked,
                ) {
                    // (A chain of aliases ends in valid SPIR-V; C++ recurses
                    // forever on a cycle.)
                    if depth > 4096 {
                        spirv_cross_throw!("Type alias recursion.");
                    }
                    return if self.backend == Backend::Msl {
                        self.to_name(type_.type_alias, true)
                    } else {
                        self.base_to_name_inner(type_.type_alias, true, depth + 1)
                    };
                }
            }
        }

        let alias = self.ir.get_name(id);
        if alias.is_empty() {
            Ok(join!("_", id))
        } else {
            Ok(alias.clone())
        }
    }

    pub(crate) fn function_is_pure(&self, func: &SPIRFunction) -> Result<bool> {
        self.function_is_pure_inner(func, 0)
    }

    fn function_is_pure_inner(&self, func: &SPIRFunction, depth: u32) -> Result<bool> {
        if depth > MAX_CALL_DEPTH {
            spirv_cross_throw!("Function call recursion is not supported.");
        }
        for &block in &func.blocks {
            if !self.block_is_pure_inner(self.get::<SPIRBlock>(block)?, depth)? {
                return Ok(false);
            }
        }

        Ok(true)
    }

    pub(crate) fn function_is_control_dependent(&self, func: &SPIRFunction) -> Result<bool> {
        self.function_is_control_dependent_inner(func, 0)
    }

    fn function_is_control_dependent_inner(&self, func: &SPIRFunction, depth: u32) -> Result<bool> {
        if depth > MAX_CALL_DEPTH {
            spirv_cross_throw!("Function call recursion is not supported.");
        }
        for &block in &func.blocks {
            if self.block_is_control_dependent_inner(self.get::<SPIRBlock>(block)?, depth)? {
                return Ok(true);
            }
        }

        Ok(false)
    }

    /// `register_global_read_dependencies(const SPIRBlock &, id)`.
    pub(crate) fn register_global_read_dependencies_block(
        &mut self,
        block: &SPIRBlock,
        id: u32,
    ) -> Result<()> {
        self.register_global_read_dependencies_block_inner(block, id, 0)
    }

    fn register_global_read_dependencies_block_inner(
        &mut self,
        block: &SPIRBlock,
        id: u32,
        depth: u32,
    ) -> Result<()> {
        for i in &block.ops {
            let ops = self.stream(i)?;
            let op = i.op as Op;

            match op {
                OpFunctionCall => {
                    let func = ops[2];
                    let f = self.get_rc::<SPIRFunction>(func)?;
                    self.register_global_read_dependencies_func_inner(&f, id, depth + 1)?;
                }

                OpLoad | OpCooperativeMatrixLoadKHR | OpCooperativeVectorLoadNV | OpImageRead => {
                    // If we're in a storage class which does not get invalidated, adding dependencies here is no big deal.
                    if let Some(var_id) = self.maybe_get_backing_variable(ops[2]) {
                        let var = self.get::<SPIRVariable>(var_id)?;
                        if var.storage != StorageClassFunction {
                            let type_ = self.get::<SPIRType>(var.basetype)?;

                            // InputTargets are immutable.
                            if type_.basetype != BaseType::Image
                                && type_.image.dim != DimSubpassData
                            {
                                self.get_mut::<SPIRVariable>(var_id)?.dependees.push(id);
                            }
                        }
                    }
                }

                _ => {}
            }
        }
        Ok(())
    }

    /// `register_global_read_dependencies(const SPIRFunction &, id)`.
    pub(crate) fn register_global_read_dependencies_func(
        &mut self,
        func: &SPIRFunction,
        id: u32,
    ) -> Result<()> {
        self.register_global_read_dependencies_func_inner(func, id, 0)
    }

    fn register_global_read_dependencies_func_inner(
        &mut self,
        func: &SPIRFunction,
        id: u32,
        depth: u32,
    ) -> Result<()> {
        if depth > MAX_CALL_DEPTH {
            spirv_cross_throw!("Function call recursion is not supported.");
        }
        for &block in &func.blocks {
            let b = self.get_rc::<SPIRBlock>(block)?;
            self.register_global_read_dependencies_block_inner(&b, id, depth)?;
        }
        Ok(())
    }

    /// `maybe_get_backing_variable()`: the id of the variable (C++
    /// returns a pointer to it).
    pub(crate) fn maybe_get_backing_variable(&self, chain: u32) -> Option<u32> {
        self.maybe_get_backing_variable_inner(chain, 0)
    }

    fn maybe_get_backing_variable_inner(&self, chain: u32, depth: u32) -> Option<u32> {
        // (C++ recurses forever on a cycle of expressions, which valid
        // SPIR-V can't make.)
        if depth > 4096 {
            return None;
        }
        let mut var = self.maybe_get::<SPIRVariable>(chain).map(|_| chain);
        if var.is_none() {
            if let Some(cexpr) = self.maybe_get::<SPIRExpression>(chain) {
                var = self
                    .maybe_get::<SPIRVariable>(cexpr.loaded_from)
                    .map(|_| cexpr.loaded_from);
                if var.is_none() && cexpr.loaded_from != chain {
                    var = self.maybe_get_backing_variable_inner(cexpr.loaded_from, depth + 1);
                }
            }

            if let Some(access_chain) = self.maybe_get::<SPIRAccessChain>(chain) {
                var = self
                    .maybe_get::<SPIRVariable>(access_chain.loaded_from)
                    .map(|_| access_chain.loaded_from);
                if var.is_none() && access_chain.loaded_from != chain {
                    var =
                        self.maybe_get_backing_variable_inner(access_chain.loaded_from, depth + 1);
                }
            }
        }

        var
    }

    /// `maybe_get_backing_buffer_pointer()`: the id of the expression.
    pub(crate) fn maybe_get_backing_buffer_pointer(&self, chain: u32) -> Option<u32> {
        let mut expr_id = chain;
        let mut expr = self.maybe_get::<SPIRExpression>(chain);
        let mut steps = 0;
        while let Some(e) = expr {
            if e.buffer_pointer || e.loaded_from == 0 {
                break;
            }
            steps += 1;
            if steps > 4096 {
                return None;
            }
            expr_id = e.loaded_from;
            expr = self.maybe_get::<SPIRExpression>(e.loaded_from);
        }
        match expr {
            Some(e) if e.buffer_pointer => Some(expr_id),
            _ => None,
        }
    }

    /// `var->parameter`: the argument of a function the variable is.
    pub(crate) fn variable_parameter_mut(&mut self, var: u32) -> Result<Option<&mut Parameter>> {
        let Some(p) = self.get::<SPIRVariable>(var)?.parameter else {
            return Ok(None);
        };
        let f = self.get_mut::<SPIRFunction>(p.function)?;
        Ok(if p.shadow {
            f.shadow_arguments.get_mut(p.index)
        } else {
            f.arguments.get_mut(p.index)
        })
    }

    pub(crate) fn variable_parameter(&self, var: u32) -> Result<Option<&Parameter>> {
        let Some(p) = self.get::<SPIRVariable>(var)?.parameter else {
            return Ok(None);
        };
        let f = self.get::<SPIRFunction>(p.function)?;
        Ok(if p.shadow {
            f.shadow_arguments.get(p.index)
        } else {
            f.arguments.get(p.index)
        })
    }

    pub(crate) fn register_read(&mut self, expr: u32, chain: u32, forwarded: bool) -> Result<()> {
        let e_self = self.get::<SPIRExpression>(expr)?.self_;
        let var = self.maybe_get_backing_variable(chain);
        let buffer_pointer = self.maybe_get_backing_buffer_pointer(chain);

        if let Some(var) = var {
            let var_self = self.get::<SPIRVariable>(var)?.self_;
            self.get_mut::<SPIRExpression>(expr)?.loaded_from = var_self;

            // If the backing variable is immutable, we do not need to depend on the variable.
            if forwarded && !self.is_immutable(var_self)? {
                self.get_mut::<SPIRVariable>(var)?.dependees.push(e_self);
            }

            // If we load from a parameter, make sure we create "inout" if we also write to the parameter.
            // The default is "in" however, so we never invalidate our compilation by reading.
            if let Some(p) = self.variable_parameter_mut(var)? {
                p.read_count = p.read_count.wrapping_add(1);
            }
        } else if let Some(bp) = buffer_pointer {
            let bp_self = self.get::<SPIRExpression>(bp)?.self_;
            self.get_mut::<SPIRExpression>(expr)?.loaded_from = bp_self;
            // If the backing variable is immutable, we do not need to depend on the variable.
            if forwarded && !self.is_immutable(bp_self)? {
                self.get_mut::<SPIRExpression>(bp)?
                    .buffer_pointer_dependees
                    .push(e_self);
            }
        }
        Ok(())
    }

    pub(crate) fn register_write(&mut self, chain: u32) -> Result<()> {
        let mut var = self.maybe_get::<SPIRVariable>(chain).map(|_| chain);
        if var.is_none() {
            // If we're storing through an access chain, invalidate the backing variable instead.
            if let Some(expr) = self.maybe_get::<SPIRExpression>(chain) {
                if expr.loaded_from != 0 {
                    var = self
                        .maybe_get::<SPIRVariable>(expr.loaded_from)
                        .map(|_| expr.loaded_from);
                }
            }

            if let Some(access_chain) = self.maybe_get::<SPIRAccessChain>(chain) {
                if access_chain.loaded_from != 0 {
                    var = self
                        .maybe_get::<SPIRVariable>(access_chain.loaded_from)
                        .map(|_| access_chain.loaded_from);
                }
            }
        }

        let buffer_pointer = self.maybe_get_backing_buffer_pointer(chain);

        let chain_type_pointer = self.expression_type(chain)?.pointer;

        if let Some(var) = var {
            let mut check_argument_storage_qualifier = true;
            let type_pointer_depth = self.expression_type(chain)?.pointer_depth;
            let type_storage = self.expression_type(chain)?.storage;

            // If our variable is in a storage class which can alias with other buffers,
            // invalidate all variables which depend on aliased variables. And if this is a
            // variable pointer, then invalidate all variables regardless.
            let var_obj = self.get_rc::<SPIRVariable>(var)?;
            if self.get_variable_data_type(&var_obj)?.pointer {
                self.flush_all_active_variables()?;

                if type_pointer_depth == 1 {
                    // We have a backing variable which is a pointer-to-pointer type.
                    // We are storing some data through a pointer acquired through that variable,
                    // but we are not writing to the value of the variable itself,
                    // i.e., we are not modifying the pointer directly.
                    // If we are storing a non-pointer type (pointer_depth == 1),
                    // we know that we are storing some unrelated data.
                    // A case here would be
                    // void foo(Foo * const *arg) {
                    //   Foo *bar = *arg;
                    //   bar->unrelated = 42;
                    // }
                    // arg, the argument is constant.
                    check_argument_storage_qualifier = false;
                }
            }

            let var_obj = self.get_rc::<SPIRVariable>(var)?;
            if type_storage == StorageClassPhysicalStorageBuffer
                || self.variable_storage_is_aliased(&var_obj)?
            {
                self.flush_all_aliased_variables()?;
            } else {
                self.flush_dependees_var(var)?;
            }

            // We tried to write to a parameter which is not marked with out qualifier, force a recompile.
            if check_argument_storage_qualifier {
                let mut recompile = false;
                if let Some(p) = self.variable_parameter_mut(var)? {
                    if p.write_count == 0 {
                        p.write_count += 1;
                        recompile = true;
                    }
                }
                if recompile {
                    self.force_recompile();
                }
            }
        } else if let Some(bp) = buffer_pointer {
            self.flush_dependees_expr(bp)?;
        } else if chain_type_pointer {
            // If we stored through a variable pointer, then we don't know which
            // variable we stored to. So *all* expressions after this point need to
            // be invalidated.
            // FIXME: If we can prove that the variable pointer will point to
            // only certain variables, we can invalidate only those.
            self.flush_all_active_variables()?;
        }

        // If chain_type.pointer is false, we're not writing to memory backed variables, but temporaries instead.
        // This can happen in copy_logical_type where we unroll complex reads and writes to temporaries.
        Ok(())
    }

    /// `flush_dependees(SPIRVariable &)`.
    pub(crate) fn flush_dependees_var(&mut self, var: u32) -> Result<()> {
        let deps = std::mem::take(&mut self.get_mut::<SPIRVariable>(var)?.dependees);
        for expr in deps {
            self.invalid_expressions.insert(expr);
        }
        Ok(())
    }

    /// `flush_dependees(SPIRExpression &)`.
    pub(crate) fn flush_dependees_expr(&mut self, expr: u32) -> Result<()> {
        // A little ugly to split things up like this since BufferPointerEXT is a weird case
        // where it's both an expression (chain into global heap) and a memory declaration at the same time ...
        // assert(expr.buffer_pointer);
        let deps = std::mem::take(
            &mut self
                .get_mut::<SPIRExpression>(expr)?
                .buffer_pointer_dependees,
        );
        for dep in deps {
            self.invalid_expressions.insert(dep);
        }
        Ok(())
    }

    pub(crate) fn flush_all_aliased_variables(&mut self) -> Result<()> {
        for aliased in self.aliased_variables.clone() {
            self.flush_dependees_var(aliased)?;
        }
        Ok(())
    }

    pub(crate) fn flush_all_atomic_capable_variables(&mut self) -> Result<()> {
        for global in self.global_variables.clone() {
            self.flush_dependees_var(global)?;
        }
        for global in self.buffer_pointer_variables.clone() {
            self.flush_dependees_expr(global)?;
        }
        self.flush_all_aliased_variables()
    }

    pub(crate) fn flush_control_dependent_expressions(&mut self, block_id: u32) -> Result<()> {
        let exprs =
            std::mem::take(&mut self.get_mut::<SPIRBlock>(block_id)?.invalidate_expressions);
        for expr in exprs {
            self.invalid_expressions.insert(expr);
        }
        Ok(())
    }

    pub(crate) fn flush_all_active_variables(&mut self) -> Result<()> {
        // Invalidate all temporaries we read from variables in this block since they were forwarded.
        // Invalidate all temporaries we read from globals.
        let func = self.get_rc::<SPIRFunction>(self.current_function.unwrap_or(0))?;
        for &v in &func.local_variables {
            self.flush_dependees_var(v)?;
        }
        for arg in &func.arguments {
            self.flush_dependees_var(arg.id)?;
        }
        for global in self.global_variables.clone() {
            self.flush_dependees_var(global)?;
        }
        for global in self.buffer_pointer_variables.clone() {
            self.flush_dependees_expr(global)?;
        }

        self.flush_all_aliased_variables()
    }

    pub(crate) fn expression_type_id(&self, id: u32) -> Result<u32> {
        match self.id_type(id) {
            TypeVariable => Ok(self.get::<SPIRVariable>(id)?.basetype),
            TypeExpression => Ok(self.get::<SPIRExpression>(id)?.expression_type),
            TypeConstant => Ok(self.get::<SPIRConstant>(id)?.constant_type),
            TypeConstantOp => Ok(self.get::<SPIRConstantOp>(id)?.basetype),
            TypeUndef => Ok(self.get::<SPIRUndef>(id)?.basetype),
            TypeCombinedImageSampler => Ok(self.get::<SPIRCombinedImageSampler>(id)?.combined_type),
            TypeAccessChain => Ok(self.get::<SPIRAccessChain>(id)?.basetype),
            _ => spirv_cross_throw!("Cannot resolve expression type."),
        }
    }

    pub(crate) fn expression_type(&self, id: u32) -> Result<&SPIRType> {
        self.get::<SPIRType>(self.expression_type_id(id)?)
    }

    /// `expression_type()` as a snapshot.
    pub(crate) fn expression_type_rc(&self, id: u32) -> Result<Rc<SPIRType>> {
        self.get_rc::<SPIRType>(self.expression_type_id(id)?)
    }

    pub(crate) fn expression_is_lvalue(&self, id: u32) -> Result<bool> {
        let type_ = self.expression_type(id)?;
        Ok(!matches!(
            type_.basetype,
            BaseType::SampledImage | BaseType::Image | BaseType::Sampler
        ))
    }

    pub(crate) fn is_immutable(&self, id: u32) -> Result<bool> {
        match self.id_type(id) {
            TypeVariable => {
                let var = self.get::<SPIRVariable>(id)?;

                // Anything we load from the UniformConstant address space is guaranteed to be immutable.
                let pointer_to_const = var.storage == StorageClassUniformConstant;
                Ok(pointer_to_const || var.phi_variable || !self.expression_is_lvalue(id)?)
            }
            TypeAccessChain => Ok(self.get::<SPIRAccessChain>(id)?.immutable),
            TypeExpression => Ok(self.get::<SPIRExpression>(id)?.immutable),
            TypeConstant | TypeConstantOp | TypeUndef => Ok(true),
            _ => Ok(false),
        }
    }

    pub(crate) fn is_hidden_variable(
        &self,
        var: &SPIRVariable,
        include_builtins: bool,
    ) -> Result<bool> {
        if (self.is_builtin_variable(var)? && !include_builtins) || var.remapped_variable {
            return Ok(true);
        }

        // Combined image samplers are always considered active as they are "magic" variables.
        if self
            .combined_image_samplers
            .iter()
            .any(|samp| samp.combined_id == var.self_)
        {
            return Ok(false);
        }

        // In SPIR-V 1.4 and up we must also use the active variable interface to disable global variables
        // which are not part of the entry point.
        if self.ir.get_spirv_version() >= 0x10400
            && var.storage != StorageClassGeneric
            && var.storage != StorageClassFunction
            && !self.interface_variable_exists_in_entry_point(var.self_)?
        {
            return Ok(true);
        }

        Ok(self.check_active_interface_variables
            && storage_class_is_interface(var.storage)
            && !self.active_interface_variables.contains(&var.self_))
    }

    pub(crate) fn is_builtin_type(&self, type_: &SPIRType) -> bool {
        // We can have builtin structs as well. If one member of a struct is builtin, the struct must also be builtin.
        if let Some(type_meta) = self.ir.find_meta(type_.self_) {
            for m in &type_meta.members {
                if m.builtin {
                    return true;
                }
            }
        }

        false
    }

    pub(crate) fn is_builtin_variable(&self, var: &SPIRVariable) -> Result<bool> {
        let m = self.ir.find_meta(var.self_);

        if var.compat_builtin || m.map(|m| m.decoration.builtin).unwrap_or(false) {
            Ok(true)
        } else {
            Ok(self.is_builtin_type(self.get::<SPIRType>(var.basetype)?))
        }
    }

    pub(crate) fn is_member_builtin(
        &self,
        type_: &SPIRType,
        index: u32,
        builtin: Option<&mut BuiltIn>,
    ) -> bool {
        if let Some(type_meta) = self.ir.find_meta(type_.self_) {
            let memb = &type_meta.members;
            if (index as usize) < memb.len() && memb[index as usize].builtin {
                if let Some(b) = builtin {
                    *b = memb[index as usize].builtin_type;
                }
                return true;
            }
        }

        false
    }

    pub(crate) fn is_scalar(&self, type_: &SPIRType) -> bool {
        type_.basetype != BaseType::Struct && type_.vecsize == 1 && type_.columns == 1
    }

    pub(crate) fn is_vector(&self, type_: &SPIRType) -> bool {
        type_.vecsize > 1 && type_.columns == 1
    }

    pub(crate) fn is_matrix(&self, type_: &SPIRType) -> bool {
        type_.vecsize > 1 && type_.columns > 1
    }

    pub(crate) fn is_array(&self, type_: &SPIRType) -> bool {
        type_.op == OpTypeArray || type_.op == OpTypeRuntimeArray
    }

    pub(crate) fn is_pointer(&self, type_: &SPIRType) -> bool {
        // Ignore function pointers.
        (type_.op == OpTypePointer || type_.op == OpTypeUntypedPointerKHR)
            && type_.basetype != BaseType::Unknown
    }

    pub(crate) fn is_physical_pointer(&self, type_: &SPIRType) -> bool {
        (type_.op == OpTypePointer || type_.op == OpTypeUntypedPointerKHR)
            && type_.storage == StorageClassPhysicalStorageBuffer
    }

    pub(crate) fn is_physical_or_buffer_pointer(&self, type_: &SPIRType) -> bool {
        (type_.op == OpTypePointer || type_.op == OpTypeUntypedPointerKHR)
            && (type_.storage == StorageClassPhysicalStorageBuffer
                || type_.storage == StorageClassUniform
                || type_.storage == StorageClassStorageBuffer
                || type_.storage == StorageClassWorkgroup
                || type_.storage == StorageClassPushConstant)
    }

    pub(crate) fn is_physical_pointer_to_buffer_block(&self, type_: &SPIRType) -> Result<bool> {
        Ok(self.is_physical_pointer(type_)
            && self.get_pointee_type(type_)?.self_ == type_.parent_type
            && (self.has_decoration(type_.self_, DecorationBlock)
                || self.has_decoration(type_.self_, DecorationBufferBlock)))
    }

    pub(crate) fn is_runtime_size_array(type_: &SPIRType) -> bool {
        type_.op == OpTypeRuntimeArray
    }

    /// `get_shader_resources()`.
    pub fn get_shader_resources(&mut self) -> Result<ShaderResources> {
        self.get_shader_resources_inner(None)
    }

    /// `get_shader_resources(active_variables)`.
    pub fn get_shader_resources_for(
        &mut self,
        active_variables: &HashSet<VariableID>,
    ) -> Result<ShaderResources> {
        self.get_shader_resources_inner(Some(active_variables))
    }
}

/// `InterfaceVariableAccessHandler`.
struct InterfaceVariableAccessHandler<'a> {
    variables: &'a mut HashSet<VariableID>,
}

fn is_interface_variable(compiler: &Compiler, id: u32) -> bool {
    match compiler.maybe_get::<SPIRVariable>(id) {
        Some(var) => storage_class_is_interface(var.storage),
        None => false,
    }
}

impl OpcodeHandler for InterfaceVariableAccessHandler<'_> {
    fn handle(
        &mut self,
        compiler: &mut Compiler,
        opcode: Op,
        args: &[u32],
        length: u32,
    ) -> Result<bool> {
        let mut variable: u32 = 0;
        match opcode {
            OpFunctionCall => {
                // Invalid SPIR-V.
                if length < 3 {
                    return Ok(false);
                }

                let count = length - 3;
                let args = &args[3..];
                for &arg in args.iter().take(count as usize) {
                    if is_interface_variable(compiler, arg) {
                        self.variables.insert(arg);
                    }
                }
            }

            OpSelect => {
                // Invalid SPIR-V.
                if length < 5 {
                    return Ok(false);
                }

                let count = length - 3;
                let args = &args[3..];
                for &arg in args.iter().take(count as usize) {
                    if is_interface_variable(compiler, arg) {
                        self.variables.insert(arg);
                    }
                }
            }

            OpPhi => {
                // Invalid SPIR-V.
                if length < 2 {
                    return Ok(false);
                }

                let count = length - 2;
                let args = &args[2..];
                let mut i = 0;
                while i < count as usize {
                    if is_interface_variable(compiler, args[i]) {
                        self.variables.insert(args[i]);
                    }
                    i += 2;
                }
            }

            OpAtomicStore | OpStore | OpCooperativeMatrixStoreKHR => {
                // Invalid SPIR-V.
                if length < 1 {
                    return Ok(false);
                }
                variable = args[0];
            }

            OpCopyMemory => {
                if length < 2 {
                    return Ok(false);
                }

                if is_interface_variable(compiler, args[0]) {
                    self.variables.insert(args[0]);
                }

                if is_interface_variable(compiler, args[1]) {
                    self.variables.insert(args[1]);
                }
            }

            OpExtInst => {
                if length < 3 {
                    return Ok(false);
                }
                let extension_set = compiler.get::<SPIRExtension>(args[2])?;
                match extension_set.ext {
                    Extension::GLSL => {
                        let op = args[3];

                        match op {
                            GLSLstd450InterpolateAtCentroid
                            | GLSLstd450InterpolateAtSample
                            | GLSLstd450InterpolateAtOffset => {
                                if is_interface_variable(compiler, args[4]) {
                                    self.variables.insert(args[4]);
                                }
                            }

                            GLSLstd450Modf | GLSLstd450Fract => {
                                if is_interface_variable(compiler, args[5]) {
                                    self.variables.insert(args[5]);
                                }
                            }

                            _ => {}
                        }
                    }
                    Extension::SPV_AMD_shader_explicit_vertex_parameter => {
                        // enum AMDShaderExplicitVertexParameter
                        const INTERPOLATE_AT_VERTEX_AMD: u32 = 1;

                        let op = args[3];

                        if op == INTERPOLATE_AT_VERTEX_AMD
                            && is_interface_variable(compiler, args[4])
                        {
                            self.variables.insert(args[4]);
                        }
                    }
                    _ => {}
                }
            }

            OpAccessChain
            | OpInBoundsAccessChain
            | OpPtrAccessChain
            | OpLoad
            | OpCooperativeMatrixLoadKHR
            | OpCopyObject
            | OpImageTexelPointer
            | OpAtomicLoad
            | OpAtomicExchange
            | OpAtomicCompareExchange
            | OpAtomicCompareExchangeWeak
            | OpAtomicIIncrement
            | OpAtomicIDecrement
            | OpAtomicIAdd
            | OpAtomicISub
            | OpAtomicSMin
            | OpAtomicUMin
            | OpAtomicSMax
            | OpAtomicUMax
            | OpAtomicAnd
            | OpAtomicOr
            | OpAtomicXor
            | OpArrayLength => {
                // Invalid SPIR-V.
                if length < 3 {
                    return Ok(false);
                }
                variable = args[2];
            }

            _ => {}
        }

        if variable != 0 && is_interface_variable(compiler, variable) {
            self.variables.insert(variable);
        }
        Ok(true)
    }
}

impl Compiler {
    /// `get_active_interface_variables()`: returns a set of all global
    /// variables which are statically accessed by the control flow graph
    /// from the current entry point.
    pub fn get_active_interface_variables(&mut self) -> Result<HashSet<VariableID>> {
        // Traverse the call graph and find all interface variables which are in use.
        let mut variables: HashSet<VariableID> = HashSet::new();
        {
            let mut handler = InterfaceVariableAccessHandler {
                variables: &mut variables,
            };
            let entry = self.ir.default_entry_point;
            self.traverse_all_reachable_opcodes_func(entry, &mut handler)?;
        }

        for id in self.ir.typed_ids(TypeVariable) {
            let var = self.get::<SPIRVariable>(id)?;
            if var.storage != StorageClassOutput {
                continue;
            }
            if !self.interface_variable_exists_in_entry_point(var.self_)? {
                continue;
            }

            // An output variable which is just declared (but uninitialized) might be read by subsequent stages
            // so we should force-enable these outputs,
            // since compilation will fail if a subsequent stage attempts to read from the variable in question.
            // Also, make sure we preserve output variables which are only initialized, but never accessed by any code.
            if var.initializer != 0 || self.get_execution_model() != ExecutionModelFragment {
                variables.insert(var.self_);
            }
        }

        // If we needed to create one, we'll need it.
        if self.dummy_sampler_id != 0 {
            variables.insert(self.dummy_sampler_id);
        }

        Ok(variables)
    }

    /// `set_enabled_interface_variables()`.
    pub fn set_enabled_interface_variables(&mut self, active_variables: HashSet<VariableID>) {
        self.active_interface_variables = active_variables;
        self.check_active_interface_variables = true;
    }

    fn get_shader_resources_inner(
        &mut self,
        active_variables: Option<&HashSet<VariableID>>,
    ) -> Result<ShaderResources> {
        let mut res = ShaderResources::default();

        let ssbo_instance_name = self.reflection_ssbo_instance_name_is_significant()?;

        for id in self.ir.typed_ids(TypeVariable) {
            let var = self.get_rc::<SPIRVariable>(id)?;
            let type_ = self.get_rc::<SPIRType>(var.basetype)?;

            // It is possible for uniform storage classes to be passed as function parameters, so detect
            // that. To detect function parameters, check of StorageClass of variable is function scope.
            if var.storage == StorageClassFunction || !type_.pointer {
                continue;
            }

            if let Some(active) = active_variables {
                if !active.contains(&var.self_) {
                    continue;
                }
            }

            // In SPIR-V 1.4 and up, every global must be present in the entry point interface list,
            // not just IO variables.
            let mut active_in_entry_point = true;
            if self.ir.get_spirv_version() < 0x10400 {
                if var.storage == StorageClassInput || var.storage == StorageClassOutput {
                    active_in_entry_point =
                        self.interface_variable_exists_in_entry_point(var.self_)?;
                }
            } else {
                active_in_entry_point = self.interface_variable_exists_in_entry_point(var.self_)?;
            }

            if !active_in_entry_point {
                continue;
            }

            let is_builtin = self.is_builtin_variable(&var)?;

            let resource = |c: &Compiler, name: String| Resource {
                id: var.self_,
                type_id: var.basetype,
                base_type_id: type_.self_,
                name: {
                    let _ = c;
                    name
                },
            };

            if is_builtin {
                if var.storage != StorageClassInput && var.storage != StorageClassOutput {
                    continue;
                }

                let mut list = Vec::new();
                let mut resource_b = BuiltInResource::default();

                if self.has_decoration(type_.self_, DecorationBlock) {
                    let name = self.get_remapped_declared_block_name_fallback(var.self_, false)?;
                    resource_b.resource = resource(self, name);

                    for i in 0..type_.member_types.len() as u32 {
                        resource_b.value_type_id = type_.member_types[i as usize];
                        resource_b.builtin =
                            self.get_member_decoration(type_.self_, i, DecorationBuiltIn);
                        list.push(resource_b.clone());
                    }
                } else {
                    let model = self.get_execution_model();
                    let strip_array = !self.has_decoration(var.self_, DecorationPatch)
                        && (model == ExecutionModelTessellationControl
                            || (model == ExecutionModelTessellationEvaluation
                                && var.storage == StorageClassInput));

                    let name = self.get_name(var.self_).clone();
                    resource_b.resource = resource(self, name);

                    if strip_array && !type_.array.is_empty() {
                        resource_b.value_type_id = self.get_variable_data_type(&var)?.parent_type;
                    } else {
                        resource_b.value_type_id = self.get_variable_data_type_id(&var)?;
                    }

                    // assert(resource.value_type_id);

                    resource_b.builtin = self.get_decoration(var.self_, DecorationBuiltIn);
                    list.push(resource_b);
                }
                if var.storage == StorageClassInput {
                    res.builtin_inputs.extend(list);
                } else {
                    res.builtin_outputs.extend(list);
                }
                continue;
            }

            // Input
            if var.storage == StorageClassInput {
                if self.has_decoration(type_.self_, DecorationBlock) {
                    let name = self.get_remapped_declared_block_name_fallback(var.self_, false)?;
                    res.stage_inputs.push(resource(self, name));
                } else {
                    let name = self.get_name(var.self_).clone();
                    res.stage_inputs.push(resource(self, name));
                }
            }
            // Subpass inputs
            else if var.storage == StorageClassUniformConstant
                && type_.image.dim == DimSubpassData
            {
                let name = self.get_name(var.self_).clone();
                res.subpass_inputs.push(resource(self, name));
            }
            // Outputs
            else if var.storage == StorageClassOutput {
                if self.has_decoration(type_.self_, DecorationBlock) {
                    let name = self.get_remapped_declared_block_name_fallback(var.self_, false)?;
                    res.stage_outputs.push(resource(self, name));
                } else {
                    let name = self.get_name(var.self_).clone();
                    res.stage_outputs.push(resource(self, name));
                }
            }
            // UBOs
            else if type_.storage == StorageClassUniform
                && self.has_decoration(type_.self_, DecorationBlock)
            {
                let name = self.get_remapped_declared_block_name_fallback(var.self_, false)?;
                res.uniform_buffers.push(resource(self, name));
            }
            // Old way to declare SSBOs.
            else if type_.storage == StorageClassUniform
                && self.has_decoration(type_.self_, DecorationBufferBlock)
            {
                let name =
                    self.get_remapped_declared_block_name_fallback(var.self_, ssbo_instance_name)?;
                res.storage_buffers.push(resource(self, name));
            }
            // Modern way to declare SSBOs.
            else if type_.storage == StorageClassStorageBuffer {
                let name =
                    self.get_remapped_declared_block_name_fallback(var.self_, ssbo_instance_name)?;
                res.storage_buffers.push(resource(self, name));
            }
            // Push constant blocks
            else if type_.storage == StorageClassPushConstant {
                // There can only be one push constant block, but keep the vector in case this restriction is lifted
                // in the future.
                let name = self.get_name(var.self_).clone();
                res.push_constant_buffers.push(resource(self, name));
            } else if type_.storage == StorageClassShaderRecordBufferKHR {
                let name =
                    self.get_remapped_declared_block_name_fallback(var.self_, ssbo_instance_name)?;
                res.shader_record_buffers.push(resource(self, name));
            }
            // Atomic counters
            else if type_.storage == StorageClassAtomicCounter {
                let name = self.get_name(var.self_).clone();
                res.atomic_counters.push(resource(self, name));
            } else if type_.storage == StorageClassUniformConstant {
                let name = self.get_name(var.self_).clone();
                if type_.basetype == BaseType::Image {
                    // Images
                    if type_.image.sampled == 2 {
                        res.storage_images.push(resource(self, name));
                    }
                    // Separate images
                    else if type_.image.sampled == 1 {
                        res.separate_images.push(resource(self, name));
                    }
                }
                // Separate samplers
                else if type_.basetype == BaseType::Sampler {
                    res.separate_samplers.push(resource(self, name));
                }
                // Textures
                else if type_.basetype == BaseType::SampledImage {
                    res.sampled_images.push(resource(self, name));
                }
                // Acceleration structures
                else if type_.basetype == BaseType::AccelerationStructure {
                    res.acceleration_structures.push(resource(self, name));
                }
                // Tensors
                else if type_.basetype == BaseType::Tensor {
                    res.tensors.push(resource(self, name));
                } else {
                    res.gl_plain_uniforms.push(resource(self, name));
                }
            }
        }

        Ok(res)
    }

    pub(crate) fn type_is_top_level_block(&self, type_: &SPIRType) -> bool {
        if type_.basetype != BaseType::Struct {
            return false;
        }
        self.has_decoration(type_.self_, DecorationBlock)
            || self.has_decoration(type_.self_, DecorationBufferBlock)
    }

    pub(crate) fn type_is_explicit_layout(&self, type_: &SPIRType) -> bool {
        if type_.basetype == BaseType::Struct {
            // Block-like types may have Offset decorations.
            for i in 0..type_.member_types.len() as u32 {
                if self.has_member_decoration(type_.self_, i, DecorationOffset) {
                    return true;
                }
            }
        }

        false
    }

    pub(crate) fn type_is_block_like(&self, type_: &SPIRType) -> bool {
        if self.type_is_top_level_block(type_) {
            true
        } else {
            self.type_is_explicit_layout(type_)
        }
    }

    pub(crate) fn parse_fixup(&mut self) -> Result<()> {
        // Figure out specialization constants for work group sizes.
        for id_ in self.ir.ids_for_constant_or_variable.clone() {
            let ty = self.id_type(id_);

            if ty == TypeConstant {
                let c = self.get_rc::<SPIRConstant>(id_)?;
                if self.has_decoration(c.self_, DecorationBuiltIn)
                    && self.get_decoration(c.self_, DecorationBuiltIn) == BuiltInWorkgroupSize
                {
                    // In current SPIR-V, there can be just one constant like this.
                    // All entry points will receive the constant value.
                    // WorkgroupSize take precedence over LocalSizeId.
                    for key in self.ir.entry_points.keys() {
                        let e = self.ir.entry_points.entry(key);
                        e.workgroup_size.constant = c.self_;
                        e.workgroup_size.x = c.scalar(0, 0);
                        e.workgroup_size.y = c.scalar(0, 1);
                        e.workgroup_size.z = c.scalar(0, 2);
                    }
                }
            } else if ty == TypeVariable {
                let var = self.get_rc::<SPIRVariable>(id_)?;
                if var.storage == StorageClassPrivate
                    || var.storage == StorageClassWorkgroup
                    || var.storage == StorageClassTaskPayloadWorkgroupEXT
                    || var.storage == StorageClassOutput
                {
                    self.global_variables.push(var.self_);
                }
                if self.variable_storage_is_aliased(&var)? {
                    self.aliased_variables.push(var.self_);
                }
            }
        }
        Ok(())
    }

    /// `update_name_cache(cache_primary, cache_secondary, name)`.
    pub(crate) fn update_name_cache2(
        cache_primary: &mut HashSet<String>,
        cache_secondary: Option<&HashSet<String>>,
        name: &mut String,
    ) {
        if name.is_empty() {
            return;
        }

        let find_name = |cache_primary: &HashSet<String>, n: &str| -> bool {
            if cache_primary.contains(n) {
                return true;
            }

            if let Some(secondary) = cache_secondary {
                if secondary.contains(n) {
                    return true;
                }
            }

            false
        };

        if !find_name(cache_primary, name) {
            cache_primary.insert(name.clone());
            return;
        }

        let mut counter: u32 = 0;
        let mut tmpname = name.clone();

        let mut use_linked_underscore = true;

        if tmpname == "_" {
            // We cannot just append numbers, as we will end up creating internally reserved names.
            // Make it like _0_<counter> instead.
            tmpname += "0";
        } else if tmpname.ends_with('_') {
            // The last_character is an underscore, so we don't need to link in underscore.
            // This would violate double underscore rules.
            use_linked_underscore = false;
        }

        // If there is a collision (very rare),
        // keep tacking on extra identifier until it's unique.
        loop {
            counter += 1;
            *name = tmpname.clone()
                + if use_linked_underscore { "_" } else { "" }
                + &convert_to_string_u(counter);
            if !find_name(cache_primary, name) {
                break;
            }
        }
        cache_primary.insert(name.clone());
    }

    /// `update_name_cache(cache, name)`.
    pub(crate) fn update_name_cache(cache: &mut HashSet<String>, name: &mut String) {
        Self::update_name_cache2(cache, None, name)
    }

    /// `set_name()`.
    pub fn set_name(&mut self, id: ID, name: &str) {
        self.ir.set_name(id, name);
    }

    /// `get_type()`.
    pub fn get_type(&self, id: TypeID) -> Result<&SPIRType> {
        self.get::<SPIRType>(id)
    }

    /// `get_type_from_variable()`.
    pub fn get_type_from_variable(&self, id: VariableID) -> Result<&SPIRType> {
        self.get::<SPIRType>(self.get::<SPIRVariable>(id)?.basetype)
    }

    pub(crate) fn get_pointee_type_id(&self, mut type_id: u32) -> Result<u32> {
        let p_type = self.get::<SPIRType>(type_id)?;
        if p_type.pointer {
            // assert(p_type->parent_type);
            type_id = p_type.parent_type;
        }
        Ok(type_id)
    }

    pub(crate) fn get_pointee_type<'a>(&'a self, type_: &'a SPIRType) -> Result<&'a SPIRType> {
        if type_.pointer {
            // assert(p_type->parent_type);
            self.get::<SPIRType>(type_.parent_type)
        } else {
            Ok(type_)
        }
    }

    pub(crate) fn get_pointee_type_by_id(&self, type_id: u32) -> Result<&SPIRType> {
        self.get_pointee_type(self.get::<SPIRType>(type_id)?)
    }

    pub(crate) fn get_variable_data_type_id(&self, var: &SPIRVariable) -> Result<u32> {
        if var.phi_variable || var.storage == StorageClassAtomicCounter {
            return Ok(var.basetype);
        }
        self.get_pointee_type_id(var.basetype)
    }

    pub(crate) fn get_variable_data_type(&self, var: &SPIRVariable) -> Result<&SPIRType> {
        self.get::<SPIRType>(self.get_variable_data_type_id(var)?)
    }

    pub(crate) fn get_variable_element_type(&self, var: &SPIRVariable) -> Result<&SPIRType> {
        let mut type_ = self.get_variable_data_type(var)?;
        if self.is_array(type_) {
            type_ = self.get::<SPIRType>(type_.parent_type)?;
        }
        Ok(type_)
    }

    pub(crate) fn is_sampled_image_type(&self, type_: &SPIRType) -> bool {
        (type_.basetype == BaseType::Image || type_.basetype == BaseType::SampledImage)
            && type_.image.sampled == 1
            && type_.image.dim != DimBuffer
    }

    /// `set_member_decoration_string()`.
    pub fn set_member_decoration_string(
        &mut self,
        id: TypeID,
        index: u32,
        decoration: Decoration,
        argument: &str,
    ) -> Result<()> {
        self.ir
            .set_member_decoration_string(id, index, decoration, argument)
    }

    /// `set_member_decoration()`.
    pub fn set_member_decoration(
        &mut self,
        id: TypeID,
        index: u32,
        decoration: Decoration,
        argument: u32,
    ) -> Result<()> {
        self.ir
            .set_member_decoration(id, index, decoration, argument)
    }

    /// `set_member_name()`.
    pub fn set_member_name(&mut self, id: TypeID, index: u32, name: &str) -> Result<()> {
        self.ir.set_member_name(id, index, name)
    }

    /// `get_member_name()`.
    pub fn get_member_name(&self, id: TypeID, index: u32) -> &String {
        self.ir.get_member_name(id, index)
    }

    pub(crate) fn set_qualified_name(&mut self, id: u32, name: &str) {
        self.ir
            .meta
            .entry(id)
            .or_default()
            .decoration
            .qualified_alias = name.to_string();
    }

    pub(crate) fn set_member_qualified_name(
        &mut self,
        type_id: u32,
        index: u32,
        name: &str,
    ) -> Result<()> {
        let words = self.ir.spirv.len();
        let m = self.ir.meta.entry(type_id).or_default();
        resize_members(&mut m.members, index, words)?;
        m.members[index as usize].qualified_alias = name.to_string();
        Ok(())
    }

    /// `get_member_qualified_name()`.
    pub fn get_member_qualified_name(&self, type_id: TypeID, index: u32) -> &String {
        match self.ir.find_meta(type_id) {
            Some(m) if (index as usize) < m.members.len() => {
                &m.members[index as usize].qualified_alias
            }
            _ => self.ir.get_empty_string(),
        }
    }

    /// `get_member_decoration()`.
    pub fn get_member_decoration(&self, id: TypeID, index: u32, decoration: Decoration) -> u32 {
        self.ir.get_member_decoration(id, index, decoration)
    }

    /// `get_member_decoration_bitset()`.
    pub fn get_member_decoration_bitset(&self, id: TypeID, index: u32) -> &Bitset {
        self.ir.get_member_decoration_bitset(id, index)
    }

    /// `has_member_decoration()`.
    pub fn has_member_decoration(&self, id: TypeID, index: u32, decoration: Decoration) -> bool {
        self.ir.has_member_decoration(id, index, decoration)
    }

    /// `unset_member_decoration()`.
    pub fn unset_member_decoration(&mut self, id: TypeID, index: u32, decoration: Decoration) {
        self.ir.unset_member_decoration(id, index, decoration)
    }

    /// `set_decoration_string()`.
    pub fn set_decoration_string(&mut self, id: ID, decoration: Decoration, argument: &str) {
        self.ir.set_decoration_string(id, decoration, argument)
    }

    /// `set_decoration()`.
    pub fn set_decoration(&mut self, id: ID, decoration: Decoration, argument: u32) {
        self.ir.set_decoration(id, decoration, argument)
    }

    pub(crate) fn set_extended_decoration(
        &mut self,
        id: u32,
        decoration: ExtendedDecorations,
        value: u32,
    ) {
        let dec = &mut self.ir.meta.entry(id).or_default().decoration;
        dec.extended.flags.set(decoration);
        dec.extended.values[decoration as usize] = value;
    }

    pub(crate) fn set_extended_member_decoration(
        &mut self,
        type_: u32,
        index: u32,
        decoration: ExtendedDecorations,
        value: u32,
    ) -> Result<()> {
        let words = self.ir.spirv.len();
        let m = self.ir.meta.entry(type_).or_default();
        resize_members(&mut m.members, index, words)?;
        let dec = &mut m.members[index as usize];
        dec.extended.flags.set(decoration);
        dec.extended.values[decoration as usize] = value;
        Ok(())
    }

    pub(crate) fn get_extended_decoration(&self, id: u32, decoration: ExtendedDecorations) -> u32 {
        let Some(m) = self.ir.find_meta(id) else {
            return 0;
        };

        let dec = &m.decoration;

        if !dec.extended.flags.get(decoration) {
            return get_default_extended_decoration(decoration);
        }

        dec.extended.values[decoration as usize]
    }

    pub(crate) fn get_extended_member_decoration(
        &self,
        type_: u32,
        index: u32,
        decoration: ExtendedDecorations,
    ) -> u32 {
        let Some(m) = self.ir.find_meta(type_) else {
            return 0;
        };

        if index as usize >= m.members.len() {
            return 0;
        }

        let dec = &m.members[index as usize];
        if !dec.extended.flags.get(decoration) {
            return get_default_extended_decoration(decoration);
        }
        dec.extended.values[decoration as usize]
    }

    pub(crate) fn has_extended_decoration(&self, id: u32, decoration: ExtendedDecorations) -> bool {
        let Some(m) = self.ir.find_meta(id) else {
            return false;
        };

        m.decoration.extended.flags.get(decoration)
    }

    pub(crate) fn has_extended_member_decoration(
        &self,
        type_: u32,
        index: u32,
        decoration: ExtendedDecorations,
    ) -> bool {
        let Some(m) = self.ir.find_meta(type_) else {
            return false;
        };

        if index as usize >= m.members.len() {
            return false;
        }

        m.members[index as usize].extended.flags.get(decoration)
    }

    pub(crate) fn unset_extended_decoration(&mut self, id: u32, decoration: ExtendedDecorations) {
        let dec = &mut self.ir.meta.entry(id).or_default().decoration;
        dec.extended.flags.clear(decoration);
        dec.extended.values[decoration as usize] = 0;
    }

    pub(crate) fn unset_extended_member_decoration(
        &mut self,
        type_: u32,
        index: u32,
        decoration: ExtendedDecorations,
    ) -> Result<()> {
        let words = self.ir.spirv.len();
        let m = self.ir.meta.entry(type_).or_default();
        resize_members(&mut m.members, index, words)?;
        let dec = &mut m.members[index as usize];
        dec.extended.flags.clear(decoration);
        dec.extended.values[decoration as usize] = 0;
        Ok(())
    }

    /// `get_storage_class()`.
    pub fn get_storage_class(&self, id: VariableID) -> Result<StorageClass> {
        Ok(self.get::<SPIRVariable>(id)?.storage)
    }

    /// `get_name()`.
    pub fn get_name(&self, id: ID) -> &String {
        self.ir.get_name(id)
    }

    /// `get_fallback_name()` (virtual; no backend overrides it).
    pub fn get_fallback_name(&self, id: ID) -> String {
        join!("_", id)
    }

    /// `get_block_fallback_name()` (virtual; no backend overrides it).
    pub fn get_block_fallback_name(&self, id: VariableID) -> Result<String> {
        let var = self.get::<SPIRVariable>(id)?;
        if self.get_name(id).is_empty() {
            Ok(join!(
                "_",
                self.get::<SPIRType>(var.basetype)?.self_,
                "_",
                id
            ))
        } else {
            Ok(self.get_name(id).clone())
        }
    }

    /// `get_fallback_member_name()` (virtual; no backend overrides it).
    pub fn get_fallback_member_name(&self, index: u32) -> String {
        join!("_", index)
    }

    /// `get_decoration_bitset()`.
    pub fn get_decoration_bitset(&self, id: ID) -> &Bitset {
        self.ir.get_decoration_bitset(id)
    }

    /// `has_decoration()`.
    pub fn has_decoration(&self, id: ID, decoration: Decoration) -> bool {
        self.ir.has_decoration(id, decoration)
    }

    /// `get_decoration_string()`.
    pub fn get_decoration_string(&self, id: ID, decoration: Decoration) -> &String {
        self.ir.get_decoration_string(id, decoration)
    }

    /// `get_member_decoration_string()`.
    pub fn get_member_decoration_string(
        &self,
        id: TypeID,
        index: u32,
        decoration: Decoration,
    ) -> &String {
        self.ir.get_member_decoration_string(id, index, decoration)
    }

    /// `get_decoration()`.
    pub fn get_decoration(&self, id: ID, decoration: Decoration) -> u32 {
        self.ir.get_decoration(id, decoration)
    }

    /// `unset_decoration()`.
    pub fn unset_decoration(&mut self, id: ID, decoration: Decoration) {
        self.ir.unset_decoration(id, decoration)
    }

    /// `get_binary_offset_for_decoration()`.
    pub fn get_binary_offset_for_decoration(
        &self,
        id: VariableID,
        decoration: Decoration,
    ) -> Option<u32> {
        let m = self.ir.find_meta(id)?;
        m.decoration_word_offset.get(&decoration).copied()
    }

    pub(crate) fn block_is_noop(&self, block: &SPIRBlock) -> Result<bool> {
        if block.terminator != Terminator::Direct {
            return Ok(false);
        }

        let child = self.get::<SPIRBlock>(block.next_block)?;

        // If this block participates in PHI, the block isn't really noop.
        for phi in &block.phi_variables {
            if phi.parent == block.self_ || phi.parent == child.self_ {
                return Ok(false);
            }
        }

        for phi in &child.phi_variables {
            if phi.parent == block.self_ {
                return Ok(false);
            }
        }

        // Verify all instructions have no semantic impact.
        for i in &block.ops {
            let op = i.op as Op;

            match op {
                // Non-Semantic instructions.
                OpLine | OpNoLine => {}

                OpExtInst => {
                    let ops = self.stream(i)?;
                    let ext = self.get::<SPIRExtension>(ops[2])?.ext;

                    let ext_is_nonsemantic_only = ext == Extension::NonSemanticShaderDebugInfo
                        || ext == Extension::SPV_debug_info
                        || ext == Extension::NonSemanticGeneric;

                    if !ext_is_nonsemantic_only {
                        return Ok(false);
                    }
                }

                _ => return Ok(false),
            }
        }

        Ok(true)
    }

    pub(crate) fn block_is_loop_candidate(
        &self,
        block: &SPIRBlock,
        method: Method,
    ) -> Result<bool> {
        // Tried and failed.
        if block.disable_block_optimization || block.complex_continue {
            return Ok(false);
        }

        if method == Method::MergeToSelectForLoop || method == Method::MergeToSelectContinueForLoop
        {
            // Try to detect common for loop pattern
            // which the code backend can use to create cleaner code.
            // for(;;) { if (cond) { some_body; } else { break; } }
            // is the pattern we're looking for.
            let false_block = self.maybe_get::<SPIRBlock>(block.false_block);
            let true_block = self.maybe_get::<SPIRBlock>(block.true_block);
            let merge_block = self.maybe_get::<SPIRBlock>(block.merge_block);

            let false_block_is_merge = block.false_block == block.merge_block
                || match (false_block, merge_block) {
                    (Some(f), Some(m)) => self.execution_is_noop(f, m)?,
                    _ => false,
                };

            let true_block_is_merge = block.true_block == block.merge_block
                || match (true_block, merge_block) {
                    (Some(t), Some(m)) => self.execution_is_noop(t, m)?,
                    _ => false,
                };

            let positive_candidate = block.true_block != block.merge_block
                && block.true_block != block.self_
                && false_block_is_merge;

            let negative_candidate = block.false_block != block.merge_block
                && block.false_block != block.self_
                && true_block_is_merge;

            let mut ret = block.terminator == Terminator::Select
                && block.merge == Merge::MergeLoop
                && (positive_candidate || negative_candidate);

            if ret && positive_candidate && method == Method::MergeToSelectContinueForLoop {
                ret = block.true_block == block.continue_block;
            } else if ret && negative_candidate && method == Method::MergeToSelectContinueForLoop {
                ret = block.false_block == block.continue_block;
            }

            // If we have OpPhi which depends on branches which came from our own block,
            // we need to flush phi variables in else block instead of a trivial break,
            // so we cannot assume this is a for loop candidate.
            if ret {
                for phi in &block.phi_variables {
                    if phi.parent == block.self_ {
                        return Ok(false);
                    }
                }

                if let Some(merge) = self.maybe_get::<SPIRBlock>(block.merge_block) {
                    for phi in &merge.phi_variables {
                        if phi.parent == block.self_ {
                            return Ok(false);
                        }
                    }
                }
            }
            Ok(ret)
        } else if method == Method::MergeToDirectForLoop {
            // Empty loop header that just sets up merge target
            // and branches to loop body.
            let mut ret = block.terminator == Terminator::Direct
                && block.merge == Merge::MergeLoop
                && self.block_is_noop(block)?;

            if !ret {
                return Ok(false);
            }

            let child = self.get::<SPIRBlock>(block.next_block)?;

            let false_block = self.maybe_get::<SPIRBlock>(child.false_block);
            let true_block = self.maybe_get::<SPIRBlock>(child.true_block);
            let merge_block = self.maybe_get::<SPIRBlock>(block.merge_block);

            let false_block_is_merge = child.false_block == block.merge_block
                || match (false_block, merge_block) {
                    (Some(f), Some(m)) => self.execution_is_noop(f, m)?,
                    _ => false,
                };

            let true_block_is_merge = child.true_block == block.merge_block
                || match (true_block, merge_block) {
                    (Some(t), Some(m)) => self.execution_is_noop(t, m)?,
                    _ => false,
                };

            let positive_candidate = child.true_block != block.merge_block
                && child.true_block != block.self_
                && false_block_is_merge;

            let negative_candidate = child.false_block != block.merge_block
                && child.false_block != block.self_
                && true_block_is_merge;

            ret = child.terminator == Terminator::Select
                && child.merge == Merge::MergeNone
                && (positive_candidate || negative_candidate);

            if ret {
                if let Some(merge) = self.maybe_get::<SPIRBlock>(block.merge_block) {
                    for phi in &merge.phi_variables {
                        if phi.parent == block.self_ || phi.parent == child.false_block {
                            return Ok(false);
                        }
                    }
                }
            }

            Ok(ret)
        } else {
            Ok(false)
        }
    }

    pub(crate) fn execution_is_noop(&self, from: &SPIRBlock, to: &SPIRBlock) -> Result<bool> {
        if !self.execution_is_branchless(from, to)? {
            return Ok(false);
        }

        let mut start = from;
        loop {
            if start.self_ == to.self_ {
                return Ok(true);
            }

            if !self.block_is_noop(start)? {
                return Ok(false);
            }

            start = self.get::<SPIRBlock>(start.next_block)?;
        }
    }

    pub(crate) fn execution_is_branchless(&self, from: &SPIRBlock, to: &SPIRBlock) -> Result<bool> {
        let mut start = from;
        // (A cycle of direct branches loops forever in C++; it can't be
        // branchless anyway.)
        let mut steps: usize = 0;
        loop {
            if start.self_ == to.self_ {
                return Ok(true);
            }

            if start.terminator == Terminator::Direct && start.merge == Merge::MergeNone {
                start = self.get::<SPIRBlock>(start.next_block)?;
                steps += 1;
                if steps > self.ir.ids.len() {
                    return Ok(false);
                }
            } else {
                return Ok(false);
            }
        }
    }

    pub(crate) fn execution_is_direct_branch(&self, from: &SPIRBlock, to: &SPIRBlock) -> bool {
        from.terminator == Terminator::Direct
            && from.merge == Merge::MergeNone
            && from.next_block == to.self_
    }

    pub(crate) fn continue_block_type(&self, block: &SPIRBlock) -> Result<ContinueBlockType> {
        // The block was deemed too complex during code emit, pick conservative fallback paths.
        if block.complex_continue {
            return Ok(ContinueBlockType::ComplexLoop);
        }

        // In older glslang output continue block can be equal to the loop header.
        // In this case, execution is clearly branchless, so just assume a while loop header here.
        if block.merge == Merge::MergeLoop {
            return Ok(ContinueBlockType::WhileLoop);
        }

        if block.loop_dominator == SPIRBlock::NO_DOMINATOR {
            // Continue block is never reached from CFG.
            return Ok(ContinueBlockType::ComplexLoop);
        }

        let dominator = self.get::<SPIRBlock>(block.loop_dominator)?;

        if self.execution_is_noop(block, dominator)? {
            Ok(ContinueBlockType::WhileLoop)
        } else if self.execution_is_branchless(block, dominator)? {
            Ok(ContinueBlockType::ForLoop)
        } else {
            let false_block = self.maybe_get::<SPIRBlock>(block.false_block);
            let true_block = self.maybe_get::<SPIRBlock>(block.true_block);
            let merge_block = self.maybe_get::<SPIRBlock>(dominator.merge_block);

            // If we need to flush Phi in this block, we cannot have a DoWhile loop.
            let flush_phi_to_false =
                false_block.is_some() && self.flush_phi_required(block.self_, block.false_block)?;
            let flush_phi_to_true =
                true_block.is_some() && self.flush_phi_required(block.self_, block.true_block)?;
            if flush_phi_to_false || flush_phi_to_true {
                return Ok(ContinueBlockType::ComplexLoop);
            }

            let positive_do_while = block.true_block == dominator.self_
                && (block.false_block == dominator.merge_block
                    || match (false_block, merge_block) {
                        (Some(f), Some(m)) => self.execution_is_noop(f, m)?,
                        _ => false,
                    });

            let negative_do_while = block.false_block == dominator.self_
                && (block.true_block == dominator.merge_block
                    || match (true_block, merge_block) {
                        (Some(t), Some(m)) => self.execution_is_noop(t, m)?,
                        _ => false,
                    });

            if block.merge == Merge::MergeNone
                && block.terminator == Terminator::Select
                && (positive_do_while || negative_do_while)
            {
                Ok(ContinueBlockType::DoWhileLoop)
            } else {
                Ok(ContinueBlockType::ComplexLoop)
            }
        }
    }

    /// `get_case_list()`: get the correct case list for the OpSwitch,
    /// since it can be either a 32 bit wide condition or a 64 bit, but the
    /// type is not embedded in the instruction itself.
    pub(crate) fn get_case_list<'a>(&self, block: &'a SPIRBlock) -> Result<&'a Vec<Case>> {
        let width;

        // First we check if we can get the type directly from the block.condition
        // since it can be a SPIRConstant or a SPIRVariable.
        if let Some(constant) = self.maybe_get::<SPIRConstant>(block.condition) {
            width = self.get::<SPIRType>(constant.constant_type)?.width;
        } else if let Some(op) = self.maybe_get::<SPIRConstantOp>(block.condition) {
            width = self.get::<SPIRType>(op.basetype)?.width;
        } else if let Some(var) = self.maybe_get::<SPIRVariable>(block.condition) {
            width = self.get::<SPIRType>(var.basetype)?.width;
        } else if let Some(undef) = self.maybe_get::<SPIRUndef>(block.condition) {
            width = self.get::<SPIRType>(undef.basetype)?.width;
        } else {
            match self.ir.load_type_width.get(&block.condition) {
                None => spirv_cross_throw!("Use of undeclared variable on a switch statement."),
                Some(&w) => width = w,
            }
        }

        if width > 32 {
            return Ok(&block.cases_64bit);
        }

        Ok(&block.cases_32bit)
    }

    /// `traverse_all_reachable_opcodes(const SPIRBlock &, handler)`.
    pub(crate) fn traverse_all_reachable_opcodes_block(
        &mut self,
        block_id: u32,
        handler: &mut dyn OpcodeHandler,
    ) -> Result<bool> {
        let block = self.get_rc::<SPIRBlock>(block_id)?;
        handler.set_current_block(self, &block)?;
        handler.rearm_current_block(self, &block)?;

        if handler.result_types().is_some() {
            for phi in &block.phi_variables {
                let bt = self.get::<SPIRVariable>(phi.function_variable)?.basetype;
                if let Some(rt) = handler.result_types() {
                    rt.insert(phi.function_variable, bt);
                }
            }
        }

        // Ideally, perhaps traverse the CFG instead of all blocks in order to eliminate dead blocks,
        // but this shouldn't be a problem in practice unless the SPIR-V is doing insane things like recursing
        // inside dead blocks ...
        for i in &block.ops {
            let ops = self.stream(i)?;
            let op = i.op as Op;

            if !handler.handle(self, op, &ops, i.length)? {
                return Ok(false);
            }

            if let Some(rt) = handler.result_types() {
                // If it has one, keep track of the instruction's result type, mapped by ID
                if let Some((result_type, result_id)) =
                    Self::instruction_to_result_type(op, &ops, i.length)
                {
                    rt.insert(result_id, result_type);
                }
            }

            if op == OpFunctionCall {
                let func = self.get_rc::<SPIRFunction>(ops[2])?;
                if handler.follow_function_call(self, &func)? {
                    if let Some(rt) = handler.result_types() {
                        for arg in &func.arguments {
                            if !arg.alias_global_variable {
                                rt.insert(arg.id, arg.type_);
                            }
                        }
                    }

                    if !handler.begin_function_scope(self, &ops, i.length)? {
                        return Ok(false);
                    }
                    if !self.traverse_all_reachable_opcodes_func(ops[2], handler)? {
                        return Ok(false);
                    }
                    if !handler.end_function_scope(self, &ops, i.length)? {
                        return Ok(false);
                    }

                    handler.rearm_current_block(self, &block)?;
                }
            }
        }

        if !handler.handle_terminator(self, &block)? {
            return Ok(false);
        }

        Ok(true)
    }

    /// `traverse_all_reachable_opcodes(const SPIRFunction &, handler)`.
    pub(crate) fn traverse_all_reachable_opcodes_func(
        &mut self,
        func_id: u32,
        handler: &mut dyn OpcodeHandler,
    ) -> Result<bool> {
        // (The SPIR-V call graph can't recurse; C++ overflows the stack on
        // one that does.)
        if self.traversal_depth > MAX_CALL_DEPTH {
            spirv_cross_throw!("Function call recursion is not supported.");
        }
        self.traversal_depth += 1;
        let func = self.get_rc::<SPIRFunction>(func_id)?;
        let mut ret = Ok(true);
        for &block in &func.blocks {
            match self.traverse_all_reachable_opcodes_block(block, handler) {
                Ok(true) => {}
                other => {
                    ret = other;
                    break;
                }
            }
        }
        self.traversal_depth -= 1;
        ret
    }

    /// `type_struct_member_offset()`.
    pub fn type_struct_member_offset(&self, type_: &SPIRType, index: u32) -> Result<u32> {
        if let Some(type_meta) = self.ir.find_meta(type_.self_) {
            // Decoration must be set in valid SPIR-V, otherwise throw.
            // (C++ reads past the members for an index out of range.)
            let Some(dec) = type_meta.members.get(index as usize) else {
                spirv_cross_throw!("Struct member does not have Offset set.");
            };
            if dec.decoration_flags.get(DecorationOffset) {
                Ok(dec.offset)
            } else {
                spirv_cross_throw!("Struct member does not have Offset set.");
            }
        } else {
            spirv_cross_throw!("Struct member does not have Offset set.");
        }
    }

    /// `type_struct_member_array_stride()`.
    pub fn type_struct_member_array_stride(&self, type_: &SPIRType, index: u32) -> Result<u32> {
        if let Some(type_meta) = self.ir.find_meta(*type_.member_types.at(index)?) {
            // Decoration must be set in valid SPIR-V, otherwise throw.
            // ArrayStride is part of the array type not OpMemberDecorate.
            let dec = &type_meta.decoration;
            if dec.decoration_flags.get(DecorationArrayStride) {
                Ok(dec.array_stride)
            } else {
                spirv_cross_throw!("Struct member does not have ArrayStride set.");
            }
        } else {
            spirv_cross_throw!("Struct member does not have ArrayStride set.");
        }
    }

    /// `type_struct_member_matrix_stride()`.
    pub fn type_struct_member_matrix_stride(&self, type_: &SPIRType, index: u32) -> Result<u32> {
        if let Some(type_meta) = self.ir.find_meta(type_.self_) {
            // Decoration must be set in valid SPIR-V, otherwise throw.
            // MatrixStride is part of OpMemberDecorate.
            let Some(dec) = type_meta.members.get(index as usize) else {
                spirv_cross_throw!("Struct member does not have MatrixStride set.");
            };
            if dec.decoration_flags.get(DecorationMatrixStride) {
                Ok(dec.matrix_stride)
            } else {
                spirv_cross_throw!("Struct member does not have MatrixStride set.");
            }
        } else {
            spirv_cross_throw!("Struct member does not have MatrixStride set.");
        }
    }

    /// `get_declared_struct_size()`.
    pub fn get_declared_struct_size(&self, type_: &SPIRType) -> Result<usize> {
        self.get_declared_struct_size_inner(type_, 0)
    }

    fn get_declared_struct_size_inner(&self, type_: &SPIRType, depth: u32) -> Result<usize> {
        if type_.member_types.is_empty() {
            spirv_cross_throw!("Declared struct in block cannot be empty.");
        }

        // Offsets can be declared out of order, so we need to deduce the actual size
        // based on last member instead.
        let mut member_index: u32 = 0;
        let mut highest_offset: usize = 0;
        for i in 0..type_.member_types.len() as u32 {
            let offset = self.type_struct_member_offset(type_, i)? as usize;
            if offset > highest_offset {
                highest_offset = offset;
                member_index = i;
            }
        }

        let size = self.get_declared_struct_member_size_inner(type_, member_index, depth)?;
        Ok(highest_offset.wrapping_add(size))
    }

    /// `get_declared_struct_size_runtime_array()`.
    pub fn get_declared_struct_size_runtime_array(
        &self,
        type_: &SPIRType,
        array_size: usize,
    ) -> Result<usize> {
        if type_.member_types.is_empty() {
            spirv_cross_throw!("Declared struct in block cannot be empty.");
        }

        let mut size = self.get_declared_struct_size(type_)?;
        let last_type = self.get::<SPIRType>(*type_.member_types.back()?)?;
        if !last_type.array.is_empty()
            && *last_type.array_size_literal.back()?
            && *last_type.array.back()? == 0
        {
            // Runtime array
            size = size.wrapping_add(array_size.wrapping_mul(
                self.type_struct_member_array_stride(type_, (type_.member_types.len() - 1) as u32)?
                    as usize,
            ));
        }

        Ok(size)
    }

    pub(crate) fn evaluate_spec_constant_u32(&self, spec: &SPIRConstantOp) -> Result<u32> {
        self.evaluate_spec_constant_u32_inner(spec, 0)
    }

    fn evaluate_spec_constant_u32_inner(&self, spec: &SPIRConstantOp, depth: u32) -> Result<u32> {
        // (A cycle of spec constant ops recurses forever in C++.)
        if depth > 4096 {
            spirv_cross_throw!("Spec constant recursion too deep.");
        }
        let result_type = self.get::<SPIRType>(spec.basetype)?;
        if result_type.basetype != BaseType::UInt
            && result_type.basetype != BaseType::Int
            && result_type.basetype != BaseType::Boolean
        {
            spirv_cross_throw!(
                "Only 32-bit integers and booleans are currently supported when evaluating specialization constants.\n"
            );
        }

        if !self.is_scalar(result_type) {
            spirv_cross_throw!("Spec constant evaluation must be a scalar.\n");
        }

        let eval_u32 = |id: u32| -> Result<u32> {
            let type_ = self.expression_type(id)?;
            if type_.basetype != BaseType::UInt
                && type_.basetype != BaseType::Int
                && type_.basetype != BaseType::Boolean
            {
                spirv_cross_throw!(
                    "Only 32-bit integers and booleans are currently supported when evaluating \
                     specialization constants.\n"
                );
            }

            if !self.is_scalar(type_) {
                spirv_cross_throw!("Spec constant evaluation must be a scalar.\n");
            }
            if let Some(c) = self.maybe_get::<SPIRConstant>(id) {
                Ok(c.scalar(0, 0))
            } else {
                self.evaluate_spec_constant_u32_inner(self.get::<SPIRConstantOp>(id)?, depth + 1)
            }
        };
        let arg = |i: usize| -> Result<u32> { Ok(*spec.arguments.at(i)?) };

        // (C++'s unsigned arithmetic wraps, and its shifts by 32 or more
        // and INT_MIN / -1 are undefined; the translation wraps and masks
        // the shift count as x86 does.)
        let value: u32 = match spec.opcode {
            // Support the basic opcodes which are typically used when computing array sizes.
            OpIAdd => eval_u32(arg(0)?)?.wrapping_add(eval_u32(arg(1)?)?),
            OpISub => eval_u32(arg(0)?)?.wrapping_sub(eval_u32(arg(1)?)?),
            OpIMul => eval_u32(arg(0)?)?.wrapping_mul(eval_u32(arg(1)?)?),
            OpBitwiseAnd => eval_u32(arg(0)?)? & eval_u32(arg(1)?)?,
            OpBitwiseOr => eval_u32(arg(0)?)? | eval_u32(arg(1)?)?,
            OpBitwiseXor => eval_u32(arg(0)?)? ^ eval_u32(arg(1)?)?,
            OpLogicalAnd => eval_u32(arg(0)?)? & eval_u32(arg(1)?)?,
            OpLogicalOr => eval_u32(arg(0)?)? | eval_u32(arg(1)?)?,
            OpShiftLeftLogical => eval_u32(arg(0)?)?.wrapping_shl(eval_u32(arg(1)?)?),
            OpShiftRightLogical => eval_u32(arg(0)?)?.wrapping_shr(eval_u32(arg(1)?)?),
            OpShiftRightArithmetic => {
                (eval_u32(arg(0)?)? as i32).wrapping_shr(eval_u32(arg(1)?)?) as u32
            }
            OpLogicalEqual => (eval_u32(arg(0)?)? == eval_u32(arg(1)?)?) as u32,
            OpLogicalNotEqual => (eval_u32(arg(0)?)? != eval_u32(arg(1)?)?) as u32,
            OpIEqual => (eval_u32(arg(0)?)? == eval_u32(arg(1)?)?) as u32,
            OpINotEqual => (eval_u32(arg(0)?)? != eval_u32(arg(1)?)?) as u32,
            OpULessThan => (eval_u32(arg(0)?)? < eval_u32(arg(1)?)?) as u32,
            OpULessThanEqual => (eval_u32(arg(0)?)? <= eval_u32(arg(1)?)?) as u32,
            OpUGreaterThan => (eval_u32(arg(0)?)? > eval_u32(arg(1)?)?) as u32,
            OpUGreaterThanEqual => (eval_u32(arg(0)?)? >= eval_u32(arg(1)?)?) as u32,
            OpSLessThan => ((eval_u32(arg(0)?)? as i32) < (eval_u32(arg(1)?)? as i32)) as u32,
            OpSLessThanEqual => ((eval_u32(arg(0)?)? as i32) <= (eval_u32(arg(1)?)? as i32)) as u32,
            OpSGreaterThan => ((eval_u32(arg(0)?)? as i32) > (eval_u32(arg(1)?)? as i32)) as u32,
            OpSGreaterThanEqual => {
                ((eval_u32(arg(0)?)? as i32) >= (eval_u32(arg(1)?)? as i32)) as u32
            }

            OpLogicalNot => (eval_u32(arg(0)?)? == 0) as u32,

            OpNot => !eval_u32(arg(0)?)?,

            OpSNegate => (eval_u32(arg(0)?)? as i32).wrapping_neg() as u32,

            OpSelect => {
                if eval_u32(arg(0)?)? != 0 {
                    eval_u32(arg(1)?)?
                } else {
                    eval_u32(arg(2)?)?
                }
            }

            OpUMod => {
                let a = eval_u32(arg(0)?)?;
                let b = eval_u32(arg(1)?)?;
                if b == 0 {
                    spirv_cross_throw!("Undefined behavior in UMod, b == 0.\n");
                }
                a % b
            }

            OpSRem => {
                let a = eval_u32(arg(0)?)? as i32;
                let b = eval_u32(arg(1)?)? as i32;
                if b == 0 {
                    spirv_cross_throw!("Undefined behavior in SRem, b == 0.\n");
                }
                a.wrapping_rem(b) as u32
            }

            OpSMod => {
                let a = eval_u32(arg(0)?)? as i32;
                let b = eval_u32(arg(1)?)? as i32;
                if b == 0 {
                    spirv_cross_throw!("Undefined behavior in SMod, b == 0.\n");
                }
                let mut v = a.wrapping_rem(b);

                // Makes sure we match the sign of b, not a.
                if (b < 0 && v > 0) || (b > 0 && v < 0) {
                    v = v.wrapping_add(b);
                }
                v as u32
            }

            OpUDiv => {
                let a = eval_u32(arg(0)?)?;
                let b = eval_u32(arg(1)?)?;
                if b == 0 {
                    spirv_cross_throw!("Undefined behavior in UDiv, b == 0.\n");
                }
                a / b
            }

            OpSDiv => {
                let a = eval_u32(arg(0)?)? as i32;
                let b = eval_u32(arg(1)?)? as i32;
                if b == 0 {
                    spirv_cross_throw!("Undefined behavior in SDiv, b == 0.\n");
                }
                a.wrapping_div(b) as u32
            }

            _ => spirv_cross_throw!("Unsupported spec constant opcode for evaluation.\n"),
        };

        Ok(value)
    }

    /// `evaluate_constant_u32()`.
    pub fn evaluate_constant_u32(&self, id: u32) -> Result<u32> {
        if let Some(c) = self.maybe_get::<SPIRConstant>(id) {
            Ok(c.scalar(0, 0))
        } else {
            self.evaluate_spec_constant_u32(self.get::<SPIRConstantOp>(id)?)
        }
    }

    /// `get_declared_struct_member_size()`.
    pub fn get_declared_struct_member_size(
        &self,
        struct_type: &SPIRType,
        index: u32,
    ) -> Result<usize> {
        self.get_declared_struct_member_size_inner(struct_type, index, 0)
    }

    fn get_declared_struct_member_size_inner(
        &self,
        struct_type: &SPIRType,
        index: u32,
        depth: u32,
    ) -> Result<usize> {
        if depth > 1024 {
            spirv_cross_throw!("Type recursion too deep.");
        }
        if struct_type.member_types.is_empty() {
            spirv_cross_throw!("Declared struct in block cannot be empty.");
        }

        let flags = self.get_member_decoration_bitset(struct_type.self_, index);
        let type_ = self.get::<SPIRType>(*struct_type.member_types.at(index)?)?;

        match type_.basetype {
            BaseType::Unknown
            | BaseType::Void
            | BaseType::Boolean // Bools are purely logical, and cannot be used for externally visible types.
            | BaseType::AtomicCounter
            | BaseType::Image
            | BaseType::SampledImage
            | BaseType::Sampler => spirv_cross_throw!("Querying size for object with opaque size."),

            _ => {}
        }

        if type_.pointer && type_.storage == StorageClassPhysicalStorageBuffer {
            // Check if this is a top-level pointer type, and not an array of pointers.
            if type_.pointer_depth > self.get::<SPIRType>(type_.parent_type)?.pointer_depth {
                return Ok(8);
            }
        }

        if !type_.array.is_empty() {
            // For arrays, we can use ArrayStride to get an easy check.
            let array_size_literal = *type_.array_size_literal.back()?;
            let array_size = if array_size_literal {
                *type_.array.back()?
            } else {
                self.evaluate_constant_u32(*type_.array.back()?)?
            };
            Ok((self
                .type_struct_member_array_stride(struct_type, index)?
                .wrapping_mul(array_size)) as usize)
        } else if type_.basetype == BaseType::Struct {
            self.get_declared_struct_size_inner(type_, depth + 1)
        } else {
            let vecsize = type_.vecsize;
            let columns = type_.columns;

            // Vectors.
            if columns == 1 {
                let component_size = (type_.width / 8) as usize;
                Ok((vecsize as usize).wrapping_mul(component_size))
            } else {
                let matrix_stride = self.type_struct_member_matrix_stride(struct_type, index)?;

                // Per SPIR-V spec, matrices must be tightly packed and aligned up for vec3 accesses.
                if flags.get(DecorationRowMajor) {
                    Ok(matrix_stride.wrapping_mul(vecsize) as usize)
                } else if flags.get(DecorationColMajor) {
                    Ok(matrix_stride.wrapping_mul(columns) as usize)
                } else {
                    spirv_cross_throw!(
                        "Either row-major or column-major must be declared for matrices."
                    );
                }
            }
        }
    }
}

/// `BufferAccessHandler`.
struct BufferAccessHandler<'a> {
    ranges: &'a mut Vec<BufferRange>,
    id: u32,
    seen: HashSet<u32>,
}

impl OpcodeHandler for BufferAccessHandler<'_> {
    fn handle(
        &mut self,
        compiler: &mut Compiler,
        opcode: Op,
        args: &[u32],
        length: u32,
    ) -> Result<bool> {
        if opcode != OpAccessChain && opcode != OpInBoundsAccessChain && opcode != OpPtrAccessChain
        {
            return Ok(true);
        }

        let ptr_chain = opcode == OpPtrAccessChain;

        // Invalid SPIR-V.
        if length < if ptr_chain { 5 } else { 4 } {
            return Ok(false);
        }

        if args[2] != self.id {
            return Ok(true);
        }

        // Don't bother traversing the entire access chain tree yet.
        // If we access a struct member, assume we access the entire member.
        let index = compiler
            .get::<SPIRConstant>(args[if ptr_chain { 4 } else { 3 }])?
            .scalar(0, 0);

        // Seen this index already.
        if self.seen.contains(&index) {
            return Ok(true);
        }
        self.seen.insert(index);

        let type_ = compiler.expression_type(self.id)?;
        let offset = compiler.type_struct_member_offset(type_, index)?;

        // If we have another member in the struct, deduce the range by looking at the next member.
        // This is okay since structs in SPIR-V can have padding, but Offset decoration must be
        // monotonically increasing.
        // Of course, this doesn't take into account if the SPIR-V for some reason decided to add
        // very large amounts of padding, but that's not really a big deal.
        let range = if (index as u64 + 1) < type_.member_types.len() as u64 {
            compiler
                .type_struct_member_offset(type_, index + 1)?
                .wrapping_sub(offset) as usize
        } else {
            // No padding, so just deduce it from the size of the member directly.
            compiler.get_declared_struct_member_size(type_, index)?
        };

        self.ranges.push(BufferRange {
            index,
            offset: offset as usize,
            range,
        });
        Ok(true)
    }
}

impl Compiler {
    /// `get_active_buffer_ranges()`.
    pub fn get_active_buffer_ranges(&mut self, id: VariableID) -> Result<Vec<BufferRange>> {
        let mut ranges = Vec::new();
        let mut handler = BufferAccessHandler {
            ranges: &mut ranges,
            id,
            seen: HashSet::new(),
        };
        let entry = self.ir.default_entry_point;
        self.traverse_all_reachable_opcodes_func(entry, &mut handler)?;
        Ok(ranges)
    }

    pub(crate) fn types_are_logically_equivalent(
        &self,
        a: &SPIRType,
        b: &SPIRType,
    ) -> Result<bool> {
        self.types_are_logically_equivalent_inner(a, b, 0)
    }

    fn types_are_logically_equivalent_inner(
        &self,
        a: &SPIRType,
        b: &SPIRType,
        depth: u32,
    ) -> Result<bool> {
        if depth > 1024 {
            spirv_cross_throw!("Type recursion too deep.");
        }
        if a.basetype != b.basetype {
            return Ok(false);
        }
        if a.width != b.width {
            return Ok(false);
        }
        if a.vecsize != b.vecsize {
            return Ok(false);
        }
        if a.columns != b.columns {
            return Ok(false);
        }
        if a.array.len() != b.array.len() {
            return Ok(false);
        }

        if !a.array.is_empty() && a.array != b.array {
            return Ok(false);
        }

        if a.basetype == BaseType::Image || a.basetype == BaseType::SampledImage {
            // FIXME (upstream): memcmp(&a.image, &b.image, sizeof(SPIRType::Image))
            // takes the size of the enumerator SPIRType::Image (an int), not
            // of the ImageType struct, so only image.type is compared.
            if a.image.type_ != b.image.type_ {
                return Ok(false);
            }
        }

        if a.member_types.len() != b.member_types.len() {
            return Ok(false);
        }

        for i in 0..a.member_types.len() {
            if !self.types_are_logically_equivalent_inner(
                self.get::<SPIRType>(a.member_types[i])?,
                self.get::<SPIRType>(b.member_types[i])?,
                depth + 1,
            )? {
                return Ok(false);
            }
        }

        Ok(true)
    }

    /// `get_execution_mode_bitset()`.
    pub fn get_execution_mode_bitset(&self) -> &Bitset {
        &self.get_entry_point().flags
    }

    /// `set_execution_mode()`.
    pub fn set_execution_mode(&mut self, mode: ExecutionMode, arg0: u32, arg1: u32, arg2: u32) {
        let execution = self.get_entry_point_mut();

        execution.flags.set(mode);
        match mode {
            ExecutionModeLocalSize => {
                execution.workgroup_size.x = arg0;
                execution.workgroup_size.y = arg1;
                execution.workgroup_size.z = arg2;
            }

            ExecutionModeLocalSizeId => {
                execution.workgroup_size.id_x = arg0;
                execution.workgroup_size.id_y = arg1;
                execution.workgroup_size.id_z = arg2;
            }

            ExecutionModeInvocations => execution.invocations = arg0,

            ExecutionModeOutputVertices => execution.output_vertices = arg0,

            ExecutionModeOutputPrimitivesEXT => execution.output_primitives = arg0,

            ExecutionModeFPFastMathDefault => {
                execution.fp_fast_math_defaults.insert(arg0, arg1);
            }

            _ => {}
        }
    }

    /// `unset_execution_mode()`.
    pub fn unset_execution_mode(&mut self, mode: ExecutionMode) {
        self.get_entry_point_mut().flags.clear(mode);
    }

    /// `get_work_group_size_specialization_constants()`: returns the
    /// constant ID of the builtin WorkGroupSize, and the x, y and z
    /// specialization constants.
    pub fn get_work_group_size_specialization_constants(
        &self,
    ) -> Result<(
        u32,
        SpecializationConstant,
        SpecializationConstant,
        SpecializationConstant,
    )> {
        let execution = self.get_entry_point();
        let mut x = SpecializationConstant::default();
        let mut y = SpecializationConstant::default();
        let mut z = SpecializationConstant::default();

        // WorkgroupSize builtin takes precedence over LocalSize / LocalSizeId.
        if execution.workgroup_size.constant != 0 {
            let c = self.get::<SPIRConstant>(execution.workgroup_size.constant)?;

            if c.m.c[0].id[0] != 0 {
                x.id = c.m.c[0].id[0];
                x.constant_id = self.get_decoration(c.m.c[0].id[0], DecorationSpecId);
            }

            if c.m.c[0].id[1] != 0 {
                y.id = c.m.c[0].id[1];
                y.constant_id = self.get_decoration(c.m.c[0].id[1], DecorationSpecId);
            }

            if c.m.c[0].id[2] != 0 {
                z.id = c.m.c[0].id[2];
                z.constant_id = self.get_decoration(c.m.c[0].id[2], DecorationSpecId);
            }
        } else if execution.flags.get(ExecutionModeLocalSizeId) {
            let cx = self.get::<SPIRConstant>(execution.workgroup_size.id_x)?;
            if cx.specialization {
                x.id = execution.workgroup_size.id_x;
                x.constant_id =
                    self.get_decoration(execution.workgroup_size.id_x, DecorationSpecId);
            }

            let cy = self.get::<SPIRConstant>(execution.workgroup_size.id_y)?;
            if cy.specialization {
                y.id = execution.workgroup_size.id_y;
                y.constant_id =
                    self.get_decoration(execution.workgroup_size.id_y, DecorationSpecId);
            }

            let cz = self.get::<SPIRConstant>(execution.workgroup_size.id_z)?;
            if cz.specialization {
                z.id = execution.workgroup_size.id_z;
                z.constant_id =
                    self.get_decoration(execution.workgroup_size.id_z, DecorationSpecId);
            }
        }

        Ok((execution.workgroup_size.constant, x, y, z))
    }

    /// `get_execution_mode_argument()`.
    pub fn get_execution_mode_argument(&self, mode: ExecutionMode, index: u32) -> Result<u32> {
        let execution = self.get_entry_point();
        Ok(match mode {
            ExecutionModeLocalSizeId => {
                if execution.flags.get(ExecutionModeLocalSizeId) {
                    match index {
                        0 => execution.workgroup_size.id_x,
                        1 => execution.workgroup_size.id_y,
                        2 => execution.workgroup_size.id_z,
                        _ => 0,
                    }
                } else {
                    0
                }
            }

            ExecutionModeLocalSize => match index {
                0 => {
                    if execution.flags.get(ExecutionModeLocalSizeId)
                        && execution.workgroup_size.id_x != 0
                    {
                        self.get::<SPIRConstant>(execution.workgroup_size.id_x)?
                            .scalar(0, 0)
                    } else {
                        execution.workgroup_size.x
                    }
                }
                1 => {
                    if execution.flags.get(ExecutionModeLocalSizeId)
                        && execution.workgroup_size.id_y != 0
                    {
                        self.get::<SPIRConstant>(execution.workgroup_size.id_y)?
                            .scalar(0, 0)
                    } else {
                        execution.workgroup_size.y
                    }
                }
                2 => {
                    if execution.flags.get(ExecutionModeLocalSizeId)
                        && execution.workgroup_size.id_z != 0
                    {
                        self.get::<SPIRConstant>(execution.workgroup_size.id_z)?
                            .scalar(0, 0)
                    } else {
                        execution.workgroup_size.z
                    }
                }
                _ => 0,
            },

            ExecutionModeInvocations => execution.invocations,

            ExecutionModeOutputVertices => execution.output_vertices,

            ExecutionModeOutputPrimitivesEXT => execution.output_primitives,

            _ => 0,
        })
    }

    /// `get_execution_model()`.
    pub fn get_execution_model(&self) -> ExecutionModel {
        self.get_entry_point().model
    }

    pub(crate) fn is_tessellation_shader_model(model: ExecutionModel) -> bool {
        model == ExecutionModelTessellationControl || model == ExecutionModelTessellationEvaluation
    }

    pub(crate) fn is_vertex_like_shader(&self) -> bool {
        let model = self.get_execution_model();
        model == ExecutionModelVertex
            || model == ExecutionModelGeometry
            || model == ExecutionModelTessellationControl
            || model == ExecutionModelTessellationEvaluation
    }

    /// `is_tessellation_shader()`.
    pub fn is_tessellation_shader(&self) -> bool {
        Self::is_tessellation_shader_model(self.get_execution_model())
    }

    /// `is_tessellating_triangles()`.
    pub fn is_tessellating_triangles(&self) -> bool {
        self.get_execution_mode_bitset().get(ExecutionModeTriangles)
    }

    /// `set_remapped_variable_state()`.
    pub fn set_remapped_variable_state(
        &mut self,
        id: VariableID,
        remap_enable: bool,
    ) -> Result<()> {
        self.get_mut::<SPIRVariable>(id)?.remapped_variable = remap_enable;
        Ok(())
    }

    /// `get_remapped_variable_state()`.
    pub fn get_remapped_variable_state(&self, id: VariableID) -> Result<bool> {
        Ok(self.get::<SPIRVariable>(id)?.remapped_variable)
    }

    /// `set_subpass_input_remapped_components()`.
    pub fn set_subpass_input_remapped_components(
        &mut self,
        id: VariableID,
        components: u32,
    ) -> Result<()> {
        self.get_mut::<SPIRVariable>(id)?.remapped_components = components;
        Ok(())
    }

    /// `get_subpass_input_remapped_components()`.
    pub fn get_subpass_input_remapped_components(&self, id: VariableID) -> Result<u32> {
        Ok(self.get::<SPIRVariable>(id)?.remapped_components)
    }

    /// `add_implied_read_expression(SPIRExpression &, source)`.
    pub(crate) fn add_implied_read_expression_expr(e: &mut SPIRExpression, source: u32) {
        if !e.implied_read_expressions.contains(&source) {
            e.implied_read_expressions.push(source);
        }
    }

    /// `add_implied_read_expression(SPIRAccessChain &, source)`.
    pub(crate) fn add_implied_read_expression_chain(e: &mut SPIRAccessChain, source: u32) {
        if !e.implied_read_expressions.contains(&source) {
            e.implied_read_expressions.push(source);
        }
    }

    pub(crate) fn add_active_interface_variable(&mut self, var_id: u32) {
        self.active_interface_variables.insert(var_id);

        // In SPIR-V 1.4 and up we must also track the interface variable in the entry point.
        if self.ir.get_spirv_version() >= 0x10400 {
            let vars = &mut self.get_entry_point_mut().interface_variables;
            if !vars.contains(&var_id) {
                vars.push(var_id);
            }
        }
    }

    pub(crate) fn inherit_expression_dependencies(
        &mut self,
        dst: u32,
        source_expression: u32,
    ) -> Result<()> {
        let ptr_e = self.maybe_get::<SPIRExpression>(dst).is_some();

        if self.is_position_invariant()
            && ptr_e
            && self
                .maybe_get::<SPIRExpression>(source_expression)
                .is_some()
        {
            let deps = &mut self.get_mut::<SPIRExpression>(dst)?.invariance_dependencies;
            if !deps.contains(&source_expression) {
                deps.push(source_expression);
            }
        }

        // Don't inherit any expression dependencies if the expression in dst
        // is not a forwarded temporary.
        if !self.forwarded_temporaries.contains(&dst) || self.forced_temporaries.contains(&dst) {
            return Ok(());
        }

        if let Some(phi) = self.maybe_get_mut::<SPIRVariable>(source_expression) {
            if phi.phi_variable {
                // We have used a phi variable, which can change at the end of the block,
                // so make sure we take a dependency on this phi variable.
                phi.dependees.push(dst);
            }
        }

        let Some(s) = self.maybe_get::<SPIRExpression>(source_expression) else {
            return Ok(());
        };
        let s_deps = s.expression_dependencies.clone();

        // (C++ dereferences dst as an expression here.)
        let e = self.get_mut::<SPIRExpression>(dst)?;
        let e_deps = &mut e.expression_dependencies;

        // If we depend on a expression, we also depend on all sub-dependencies from source.
        e_deps.push(source_expression);
        e_deps.extend_from_slice(&s_deps);

        // Eliminate duplicated dependencies.
        e_deps.sort();
        e_deps.dedup();
        Ok(())
    }

    /// `get_entry_points_and_stages()`.
    pub fn get_entry_points_and_stages(&self) -> Vec<EntryPoint> {
        let mut entries = Vec::new();
        for (_, entry) in self.ir.entry_points.iter() {
            entries.push(EntryPoint {
                name: entry.orig_name.clone(),
                execution_model: entry.model,
            });
        }
        entries
    }

    /// `rename_entry_point()`.
    pub fn rename_entry_point(
        &mut self,
        old_name: &str,
        new_name: &str,
        model: ExecutionModel,
    ) -> Result<()> {
        let entry = self.get_entry_point_by_name_mut(old_name, model)?;
        entry.orig_name = new_name.to_string();
        entry.name = new_name.to_string();
        Ok(())
    }

    /// `set_entry_point()`.
    pub fn set_entry_point(&mut self, name: &str, model: ExecutionModel) -> Result<()> {
        let self_ = self.get_entry_point_by_name(name, model)?.self_;
        self.ir.default_entry_point = self_;
        Ok(())
    }

    fn find_entry_point(&self, pred: impl Fn(&SPIREntryPoint) -> bool) -> Option<FunctionID> {
        self.ir
            .entry_points
            .iter()
            .find(|(_, e)| pred(e))
            .map(|(k, _)| k)
    }

    pub(crate) fn get_first_entry_point(&self, name: &str) -> Result<&SPIREntryPoint> {
        match self.find_entry_point(|e| e.orig_name == name) {
            Some(k) => Ok(self
                .ir
                .entry_points
                .get(&k)
                .unwrap_or(&self.empty_entry_point)),
            None => spirv_cross_throw!("Entry point does not exist."),
        }
    }

    pub(crate) fn get_first_entry_point_mut(&mut self, name: &str) -> Result<&mut SPIREntryPoint> {
        match self.find_entry_point(|e| e.orig_name == name) {
            Some(k) => Ok(self.ir.entry_points.entry(k)),
            None => spirv_cross_throw!("Entry point does not exist."),
        }
    }

    /// `get_entry_point(name, model)`.
    pub fn get_entry_point_by_name(
        &self,
        name: &str,
        model: ExecutionModel,
    ) -> Result<&SPIREntryPoint> {
        match self.find_entry_point(|e| e.orig_name == name && e.model == model) {
            Some(k) => Ok(self
                .ir
                .entry_points
                .get(&k)
                .unwrap_or(&self.empty_entry_point)),
            None => spirv_cross_throw!("Entry point does not exist."),
        }
    }

    pub(crate) fn get_entry_point_by_name_mut(
        &mut self,
        name: &str,
        model: ExecutionModel,
    ) -> Result<&mut SPIREntryPoint> {
        match self.find_entry_point(|e| e.orig_name == name && e.model == model) {
            Some(k) => Ok(self.ir.entry_points.entry(k)),
            None => spirv_cross_throw!("Entry point does not exist."),
        }
    }

    /// `get_cleansed_entry_point_name()`.
    pub fn get_cleansed_entry_point_name(
        &self,
        name: &str,
        model: ExecutionModel,
    ) -> Result<&String> {
        Ok(&self.get_entry_point_by_name(name, model)?.name)
    }

    /// `get_entry_point()`: the current entry point.
    pub(crate) fn get_entry_point(&self) -> &SPIREntryPoint {
        // (C++ dereferences find() of the default entry point, which a
        // parsed module always has.)
        self.ir
            .entry_points
            .get(&self.ir.default_entry_point)
            .unwrap_or(&self.empty_entry_point)
    }

    pub(crate) fn get_entry_point_mut(&mut self) -> &mut SPIREntryPoint {
        let id = self.ir.default_entry_point;
        self.ir.entry_points.entry(id)
    }

    pub(crate) fn interface_variable_exists_in_entry_point(&self, id: u32) -> Result<bool> {
        let var = self.get::<SPIRVariable>(id)?;

        if self.ir.get_spirv_version() < 0x10400 {
            if var.storage != StorageClassInput
                && var.storage != StorageClassOutput
                && var.storage != StorageClassUniformConstant
            {
                spirv_cross_throw!("Only Input, Output variables and Uniform constants are part of a shader linking interface.");
            }

            // This is to avoid potential problems with very old glslang versions which did
            // not emit input/output interfaces properly.
            // We can assume they only had a single entry point, and single entry point
            // shaders could easily be assumed to use every interface variable anyways.
            if self.ir.entry_points.len() <= 1 {
                return Ok(true);
            }
        }

        // In SPIR-V 1.4 and later, all global resource variables must be present.

        let execution = self.get_entry_point();
        Ok(execution.interface_variables.contains(&id))
    }
}

/// `CombinedImageSamplerHandler`.
struct CombinedImageSamplerHandler {
    // Each function in the call stack needs its own remapping for parameters so we can deduce which global variable each texture/sampler the parameter is statically bound to.
    parameter_remapping: Vec<HashMap<u32, u32>>,
    functions: Vec<u32>,
}

impl CombinedImageSamplerHandler {
    fn push_remap_parameters(
        &mut self,
        compiler: &Compiler,
        func: &SPIRFunction,
        args: &[u32],
        length: u32,
    ) -> Result<()> {
        // If possible, pipe through a remapping table so that parameters know
        // which variables they actually bind to in this scope.
        let mut remapping: HashMap<u32, u32> = HashMap::new();
        for i in 0..length as usize {
            let v = self.remap_parameter(compiler, args[i]);
            remapping.insert(func.arguments.at(i)?.id, v);
        }
        self.parameter_remapping.push(remapping);
        Ok(())
    }

    fn pop_remap_parameters(&mut self) {
        self.parameter_remapping.pop();
    }

    fn remap_parameter(&self, compiler: &Compiler, mut id: u32) -> u32 {
        if let Some(var) = compiler.maybe_get_backing_variable(id) {
            id = var;
        }

        let Some(remapping) = self.parameter_remapping.last() else {
            return id;
        };

        match remapping.get(&id) {
            Some(&v) => v,
            None => id,
        }
    }

    fn register_combined_image_sampler(
        &mut self,
        compiler: &mut Compiler,
        caller: u32,
        combined_module_id: VariableID,
        image_id: VariableID,
        sampler_id: VariableID,
        depth: bool,
    ) -> Result<()> {
        // We now have a texture ID and a sampler ID which will either be found as a global
        // or a parameter in our own function. If both are global, they will not need a parameter,
        // otherwise, add it to our list.
        let mut param = CombinedImageSamplerParameter {
            id: 0,
            image_id,
            sampler_id,
            global_image: true,
            global_sampler: true,
            depth,
        };

        let caller_f = compiler.get::<SPIRFunction>(caller)?;
        let texture_itr = caller_f.arguments.iter().position(|p| p.id == image_id);
        let sampler_itr = caller_f.arguments.iter().position(|p| p.id == sampler_id);

        if let Some(pos) = texture_itr {
            param.global_image = false;
            param.image_id = pos as u32;
        }

        if let Some(pos) = sampler_itr {
            param.global_sampler = false;
            param.sampler_id = pos as u32;
        }

        if param.global_image && param.global_sampler {
            return Ok(());
        }

        let found = caller_f.combined_parameters.iter().any(|p| {
            param.image_id == p.image_id
                && param.sampler_id == p.sampler_id
                && param.global_image == p.global_image
                && param.global_sampler == p.global_sampler
        });

        if !found {
            let id = compiler.ir.increase_bound_by(3)?;
            let type_id = id;
            let ptr_type_id = id + 1;
            let combined_id = id + 2;
            let base = compiler.expression_type(image_id)?.clone();
            compiler.set(type_id, SPIRType::new(OpTypeSampledImage))?;
            compiler.set(ptr_type_id, SPIRType::new(OpTypePointer))?;

            let mut type_ = base;
            type_.self_ = type_id;
            type_.basetype = BaseType::SampledImage;
            type_.pointer = false;
            type_.storage = StorageClassGeneric;
            type_.image.depth = depth;

            let mut ptr_type = type_.clone();
            ptr_type.pointer = true;
            ptr_type.storage = StorageClassUniformConstant;
            ptr_type.parent_type = type_id;
            *compiler.get_mut::<SPIRType>(type_id)? = type_;
            *compiler.get_mut::<SPIRType>(ptr_type_id)? = ptr_type;

            // Build new variable.
            compiler.set(
                combined_id,
                SPIRVariable::new(ptr_type_id, StorageClassFunction, 0, 0),
            )?;

            // Inherit RelaxedPrecision.
            // If any of OpSampledImage, underlying image or sampler are marked, inherit the decoration.
            let relaxed_precision = compiler.has_decoration(sampler_id, DecorationRelaxedPrecision)
                || compiler.has_decoration(image_id, DecorationRelaxedPrecision)
                || (combined_module_id != 0
                    && compiler.has_decoration(combined_module_id, DecorationRelaxedPrecision));

            if relaxed_precision {
                compiler.set_decoration(combined_id, DecorationRelaxedPrecision, 0);
            }

            param.id = combined_id;

            let name = join!(
                "SPIRV_Cross_Combined",
                compiler.to_name(image_id, true)?,
                compiler.to_name(sampler_id, true)?
            );
            compiler.set_name(combined_id, &name);

            let caller_f = compiler.get_mut::<SPIRFunction>(caller)?;
            caller_f.combined_parameters.push(param);
            caller_f.shadow_arguments.push(Parameter {
                type_: ptr_type_id,
                id: combined_id,
                read_count: 0,
                write_count: 0,
                alias_global_variable: true,
            });
        }
        Ok(())
    }
}

impl OpcodeHandler for CombinedImageSamplerHandler {
    fn begin_function_scope(
        &mut self,
        compiler: &mut Compiler,
        args: &[u32],
        length: u32,
    ) -> Result<bool> {
        if length < 3 {
            return Ok(false);
        }

        let callee = compiler.get_rc::<SPIRFunction>(args[2])?;
        let args = &args[3..];
        let length = length - 3;
        self.push_remap_parameters(compiler, &callee, args, length)?;
        self.functions.push(callee.self_);
        Ok(true)
    }

    fn end_function_scope(
        &mut self,
        compiler: &mut Compiler,
        args: &[u32],
        length: u32,
    ) -> Result<bool> {
        if length < 3 {
            return Ok(false);
        }

        let callee = args[2];
        compiler.get::<SPIRFunction>(callee)?;
        let args = &args[3..];

        // There are two types of cases we have to handle,
        // a callee might call sampler2D(texture2D, sampler) directly where
        // one or more parameters originate from parameters.
        // Alternatively, we need to provide combined image samplers to our callees,
        // and in this case we need to add those as well.

        self.pop_remap_parameters();

        // Our callee has now been processed at least once.
        // No point in doing it again.
        compiler
            .get_mut::<SPIRFunction>(callee)?
            .do_combined_parameters = false;

        let Some(top) = self.functions.pop() else {
            return Ok(true);
        };
        let params = compiler
            .get::<SPIRFunction>(top)?
            .combined_parameters
            .clone();
        let Some(&caller) = self.functions.last() else {
            return Ok(true);
        };

        if compiler.get::<SPIRFunction>(caller)?.do_combined_parameters {
            for param in &params {
                let mut image_id = if param.global_image {
                    param.image_id
                } else {
                    *args.at(param.image_id)?
                };
                let mut sampler_id = if param.global_sampler {
                    param.sampler_id
                } else {
                    *args.at(param.sampler_id)?
                };

                if let Some(i) = compiler.maybe_get_backing_variable(image_id) {
                    image_id = i;
                }
                if let Some(s) = compiler.maybe_get_backing_variable(sampler_id) {
                    sampler_id = s;
                }

                self.register_combined_image_sampler(
                    compiler,
                    caller,
                    0,
                    image_id,
                    sampler_id,
                    param.depth,
                )?;
            }
        }

        Ok(true)
    }

    fn handle(
        &mut self,
        compiler: &mut Compiler,
        opcode: Op,
        args: &[u32],
        length: u32,
    ) -> Result<bool> {
        // We need to figure out where samplers and images are loaded from, so do only the bare bones compilation we need.
        let mut is_fetch = false;

        match opcode {
            OpLoad => {
                if length < 3 {
                    return Ok(false);
                }

                let result_type = args[0];

                let type_ = compiler.get::<SPIRType>(result_type)?;
                let separate_image = type_.basetype == BaseType::Image && type_.image.sampled == 1;
                let separate_sampler = type_.basetype == BaseType::Sampler;

                // If not separate image or sampler, don't bother.
                if !separate_image && !separate_sampler {
                    return Ok(true);
                }

                let id = args[1];
                let ptr = args[2];
                compiler.set(id, SPIRExpression::new(String::new(), result_type, true))?;
                compiler.register_read(id, ptr, true)?;
                return Ok(true);
            }

            OpInBoundsAccessChain | OpAccessChain | OpPtrAccessChain => {
                if length < 3 {
                    return Ok(false);
                }

                // Technically, it is possible to have arrays of textures and arrays of samplers and combine them, but this becomes essentially
                // impossible to implement, since we don't know which concrete sampler we are accessing.
                // One potential way is to create a combinatorial explosion where N textures and M samplers are combined into N * M sampler2Ds,
                // but this seems ridiculously complicated for a problem which is easy to work around.
                // Checking access chains like this assumes we don't have samplers or textures inside uniform structs, but this makes no sense.

                let result_type = args[0];

                let type_ = compiler.get::<SPIRType>(result_type)?;
                let separate_image = type_.basetype == BaseType::Image && type_.image.sampled == 1;
                let separate_sampler = type_.basetype == BaseType::Sampler;
                if separate_sampler {
                    spirv_cross_throw!(
                        "Attempting to use arrays or structs of separate samplers. This is not possible to statically \
                         remap to plain GLSL."
                    );
                }

                if separate_image {
                    let id = args[1];
                    let ptr = args[2];
                    compiler.set(id, SPIRExpression::new(String::new(), result_type, true))?;
                    compiler.register_read(id, ptr, true)?;
                }
                return Ok(true);
            }

            OpImageFetch | OpImageQuerySizeLod | OpImageQuerySize | OpImageQueryLevels
            | OpImageQuerySamples => {
                // If we are fetching from a plain OpTypeImage or querying LOD, we must pre-combine with our dummy sampler.
                let Some(var) = compiler.maybe_get_backing_variable(args[2]) else {
                    return Ok(true);
                };

                let type_ =
                    compiler.get::<SPIRType>(compiler.get::<SPIRVariable>(var)?.basetype)?;
                if type_.basetype == BaseType::Image
                    && type_.image.sampled == 1
                    && type_.image.dim != DimBuffer
                {
                    if compiler.dummy_sampler_id == 0 {
                        spirv_cross_throw!(
                            "texelFetch without sampler was found, but no dummy sampler has been created with \
                             build_dummy_sampler_for_combined_images()."
                        );
                    }

                    // Do it outside.
                    is_fetch = true;
                } else {
                    return Ok(true);
                }
            }

            OpSampledImage => {
                // Do it outside.
            }

            _ => return Ok(true),
        }

        // Registers sampler2D calls used in case they are parameters so
        // that their callees know which combined image samplers to propagate down the call stack.
        if let Some(&callee) = self.functions.last() {
            if compiler.get::<SPIRFunction>(callee)?.do_combined_parameters {
                let mut image_id = args[2];

                if let Some(image) = compiler.maybe_get_backing_variable(image_id) {
                    image_id = image;
                }

                let mut sampler_id = if is_fetch {
                    compiler.dummy_sampler_id
                } else {
                    args[3]
                };
                if let Some(sampler) = compiler.maybe_get_backing_variable(sampler_id) {
                    sampler_id = sampler;
                }

                let combined_id = args[1];

                let depth = compiler.get::<SPIRType>(args[0])?.image.depth;
                self.register_combined_image_sampler(
                    compiler,
                    callee,
                    combined_id,
                    image_id,
                    sampler_id,
                    depth,
                )?;
            }
        }

        // For function calls, we need to remap IDs which are function parameters into global variables.
        // This information is statically known from the current place in the call stack.
        // Function parameters are not necessarily pointers, so if we don't have a backing variable, remapping will know
        // which backing variable the image/sample came from.
        let image_id = self.remap_parameter(compiler, args[2]);
        let sampler_id = if is_fetch {
            compiler.dummy_sampler_id
        } else {
            self.remap_parameter(compiler, args[3])
        };

        let found = compiler
            .combined_image_samplers
            .iter()
            .any(|combined| combined.image_id == image_id && combined.sampler_id == sampler_id);

        if !found {
            let sampled_type;
            let combined_module_id;
            if is_fetch {
                // Have to invent the sampled image type.
                sampled_type = compiler.ir.increase_bound_by(1)?;
                compiler.set(sampled_type, SPIRType::new(OpTypeSampledImage))?;
                let mut type_ = compiler.expression_type(args[2])?.clone();
                type_.self_ = sampled_type;
                type_.basetype = BaseType::SampledImage;
                type_.image.depth = false;
                *compiler.get_mut::<SPIRType>(sampled_type)? = type_;
                combined_module_id = 0;
            } else {
                sampled_type = args[0];
                combined_module_id = args[1];
            }

            let id = compiler.ir.increase_bound_by(2)?;
            let type_id = id;
            let combined_id = id + 1;

            // Make a new type, pointer to OpTypeSampledImage, so we can make a variable of this type.
            // We will probably have this type lying around, but it doesn't hurt to make duplicates for internal purposes.
            compiler.set(type_id, SPIRType::new(OpTypePointer))?;
            let mut type_ = compiler.get::<SPIRType>(sampled_type)?.clone();
            type_.pointer = true;
            type_.storage = StorageClassUniformConstant;
            type_.parent_type = type_id;
            *compiler.get_mut::<SPIRType>(type_id)? = type_;

            // Build new variable.
            compiler.set(
                combined_id,
                SPIRVariable::new(type_id, StorageClassUniformConstant, 0, 0),
            )?;

            // Inherit RelaxedPrecision (and potentially other useful flags if deemed relevant).
            // If any of OpSampledImage, underlying image or sampler are marked, inherit the decoration.
            let relaxed_precision = (sampler_id != 0
                && compiler.has_decoration(sampler_id, DecorationRelaxedPrecision))
                || (image_id != 0 && compiler.has_decoration(image_id, DecorationRelaxedPrecision))
                || (combined_module_id != 0
                    && compiler.has_decoration(combined_module_id, DecorationRelaxedPrecision));

            if relaxed_precision {
                compiler.set_decoration(combined_id, DecorationRelaxedPrecision, 0);
            }

            // Propagate the array type for the original image as well.
            if let Some(var) = compiler.maybe_get_backing_variable(image_id) {
                let parent_type = compiler
                    .get::<SPIRType>(compiler.get::<SPIRVariable>(var)?.basetype)?
                    .clone();
                let type_ = compiler.get_mut::<SPIRType>(type_id)?;
                type_.array = parent_type.array;
                type_.array_size_literal = parent_type.array_size_literal;
            }

            compiler.combined_image_samplers.push(CombinedImageSampler {
                combined_id,
                image_id,
                sampler_id,
            });
        }

        Ok(true)
    }
}

/// `DummySamplerForCombinedImageHandler`.
struct DummySamplerForCombinedImageHandler {
    need_dummy_sampler: bool,
}

impl OpcodeHandler for DummySamplerForCombinedImageHandler {
    fn handle(
        &mut self,
        compiler: &mut Compiler,
        opcode: Op,
        args: &[u32],
        length: u32,
    ) -> Result<bool> {
        if self.need_dummy_sampler {
            // No need to traverse further, we know the result.
            return Ok(false);
        }

        match opcode {
            OpLoad => {
                if length < 3 {
                    return Ok(false);
                }

                let result_type = args[0];

                let type_ = compiler.get::<SPIRType>(result_type)?;
                let separate_image = type_.basetype == BaseType::Image
                    && type_.image.sampled == 1
                    && type_.image.dim != DimBuffer;

                // If not separate image, don't bother.
                if !separate_image {
                    return Ok(true);
                }

                let id = args[1];
                let ptr = args[2];
                compiler.set(id, SPIRExpression::new(String::new(), result_type, true))?;
                compiler.register_read(id, ptr, true)?;
            }

            OpImageFetch | OpImageQuerySizeLod | OpImageQuerySize | OpImageQueryLevels
            | OpImageQuerySamples => {
                // If we are fetching or querying LOD from a plain OpTypeImage, we must pre-combine with our dummy sampler.
                if let Some(var) = compiler.maybe_get_backing_variable(args[2]) {
                    let type_ =
                        compiler.get::<SPIRType>(compiler.get::<SPIRVariable>(var)?.basetype)?;
                    if type_.basetype == BaseType::Image
                        && type_.image.sampled == 1
                        && type_.image.dim != DimBuffer
                    {
                        self.need_dummy_sampler = true;
                    }
                }
            }

            OpInBoundsAccessChain | OpAccessChain | OpPtrAccessChain => {
                if length < 3 {
                    return Ok(false);
                }

                let result_type = args[0];
                let type_ = compiler.get::<SPIRType>(result_type)?;
                let separate_image = type_.basetype == BaseType::Image
                    && type_.image.sampled == 1
                    && type_.image.dim != DimBuffer;
                if !separate_image {
                    return Ok(true);
                }

                let id = args[1];
                let ptr = args[2];
                compiler.set(id, SPIRExpression::new(String::new(), result_type, true))?;
                compiler.register_read(id, ptr, true)?;

                // Other backends might use SPIRAccessChain for this later.
                compiler.ir.ids.at_mut(id)?.set_allow_type_rewrite();
            }

            _ => {}
        }

        Ok(true)
    }
}

impl Compiler {
    /// `build_dummy_sampler_for_combined_images()`.
    pub fn build_dummy_sampler_for_combined_images(&mut self) -> Result<VariableID> {
        let mut handler = DummySamplerForCombinedImageHandler {
            need_dummy_sampler: false,
        };
        let entry = self.ir.default_entry_point;
        self.traverse_all_reachable_opcodes_func(entry, &mut handler)?;
        if handler.need_dummy_sampler {
            let offset = self.ir.increase_bound_by(3)?;
            let type_id = offset;
            let ptr_type_id = offset + 1;
            let var_id = offset + 2;

            let sampler = self.set(type_id, SPIRType::new(OpTypeSampler))?;
            sampler.basetype = BaseType::Sampler;
            let sampler = sampler.clone();

            let ptr_sampler = self.set(ptr_type_id, SPIRType::new(OpTypePointer))?;
            *ptr_sampler = sampler;
            ptr_sampler.self_ = type_id;
            ptr_sampler.storage = StorageClassUniformConstant;
            ptr_sampler.pointer = true;
            ptr_sampler.parent_type = type_id;

            self.set(
                var_id,
                SPIRVariable::new(ptr_type_id, StorageClassUniformConstant, 0, 0),
            )?;
            self.set_name(var_id, "SPIRV_Cross_DummySampler");
            self.dummy_sampler_id = var_id;
            Ok(var_id)
        } else {
            Ok(0)
        }
    }

    /// `build_combined_image_samplers()`.
    pub fn build_combined_image_samplers(&mut self) -> Result<()> {
        for id in self.ir.typed_ids(TypeFunction) {
            let func = self.get_mut::<SPIRFunction>(id)?;
            func.combined_parameters.clear();
            func.shadow_arguments.clear();
            func.do_combined_parameters = true;
        }

        self.combined_image_samplers.clear();
        let mut handler = CombinedImageSamplerHandler {
            parameter_remapping: Vec::new(),
            functions: Vec::new(),
        };
        let entry = self.ir.default_entry_point;
        self.traverse_all_reachable_opcodes_func(entry, &mut handler)?;
        Ok(())
    }

    /// `get_combined_image_samplers()`.
    pub fn get_combined_image_samplers(&self) -> &Vec<CombinedImageSampler> {
        &self.combined_image_samplers
    }

    /// `set_variable_type_remap_callback()`.
    pub fn set_variable_type_remap_callback(&mut self, cb: VariableTypeRemapCallback) {
        self.variable_remap_callback = Some(cb);
    }

    pub(crate) fn remap_variable_type_name(
        &self,
        type_: &SPIRType,
        var_name: &str,
        type_name: &mut String,
    ) {
        if let Some(cb) = &self.variable_remap_callback {
            cb(type_, var_name, type_name);
        }
    }

    /// `get_specialization_constants()`.
    pub fn get_specialization_constants(&self) -> Result<Vec<SpecializationConstant>> {
        let mut spec_consts = Vec::new();
        for id in self.ir.typed_ids(TypeConstant) {
            let c = self.get::<SPIRConstant>(id)?;
            if c.specialization && self.has_decoration(c.self_, DecorationSpecId) {
                spec_consts.push(SpecializationConstant {
                    id: c.self_,
                    constant_id: self.get_decoration(c.self_, DecorationSpecId),
                });
            }
        }
        Ok(spec_consts)
    }

    /// `get_constant()`.
    pub fn get_constant(&self, id: ConstantID) -> Result<&SPIRConstant> {
        self.get::<SPIRConstant>(id)
    }

    /// `get_constant()`, to modify it.
    pub fn get_constant_mut(&mut self, id: ConstantID) -> Result<&mut SPIRConstant> {
        self.get_mut::<SPIRConstant>(id)
    }

    /// `get_current_id_bound()`.
    pub fn get_current_id_bound(&self) -> u32 {
        self.ir.ids.len() as u32
    }

    pub(crate) fn analyze_parameter_preservation(
        &mut self,
        entry: u32,
        cfg: &CFG,
        variable_to_blocks: &StdHashMap<StdHashSet>,
        complete_write_blocks: &StdHashMap<StdHashSet>,
    ) -> Result<()> {
        let args = self.get::<SPIRFunction>(entry)?.arguments.clone();
        let entry_block = self.get::<SPIRFunction>(entry)?.entry_block;
        for (i, arg) in args.iter().enumerate() {
            // Non-pointers are always inputs.
            let type_ = self.get::<SPIRType>(arg.type_)?;
            if !type_.pointer {
                continue;
            }

            // Opaque argument types are always in
            let potential_preserve = !matches!(
                type_.basetype,
                BaseType::Sampler
                    | BaseType::Image
                    | BaseType::SampledImage
                    | BaseType::AtomicCounter
            );

            if !potential_preserve {
                continue;
            }

            if !variable_to_blocks.contains_key(&arg.id) {
                // Variable is never accessed.
                continue;
            }

            // We have accessed a variable, but there was no complete writes to that variable.
            // We deduce that we must preserve the argument.
            let Some(blocks) = complete_write_blocks.get(&arg.id) else {
                let a = &mut self.get_mut::<SPIRFunction>(entry)?.arguments[i];
                a.read_count = a.read_count.wrapping_add(1);
                continue;
            };

            // If there is a path through the CFG where no block completely writes to the variable, the variable will be in an undefined state
            // when the function returns. We therefore need to implicitly preserve the variable in case there are writers in the function.
            // Major case here is if a function is
            // void foo(int &var) { if (cond) var = 10; }
            // Using read/write counts, we will think it's just an out variable, but it really needs to be inout,
            // because if we don't write anything whatever we put into the function must return back to the caller.
            let mut visit_cache = HashSet::new();
            if exists_unaccessed_path_to_return(cfg, entry_block, blocks, &mut visit_cache, 0)? {
                let a = &mut self.get_mut::<SPIRFunction>(entry)?.arguments[i];
                a.read_count = a.read_count.wrapping_add(1);
            }
        }
        Ok(())
    }
}

fn exists_unaccessed_path_to_return(
    cfg: &CFG,
    block: u32,
    blocks: &StdHashSet,
    visit_cache: &mut HashSet<u32>,
    depth: u32,
) -> Result<bool> {
    if depth > 100000 {
        spirv_cross_throw!("CFG too deep.");
    }
    // This block accesses the variable.
    if blocks.contains(&block) {
        return Ok(false);
    }

    // We are at the end of the CFG.
    if cfg.get_succeeding_edges(block).is_empty() {
        return Ok(true);
    }

    // If any of our successors have a path to the end, there exists a path from block.
    for &succ in cfg.get_succeeding_edges(block) {
        if !visit_cache.contains(&succ) {
            if exists_unaccessed_path_to_return(cfg, succ, blocks, visit_cache, depth + 1)? {
                return Ok(true);
            }
            visit_cache.insert(succ);
        }
    }

    Ok(false)
}

/// `AnalyzeVariableScopeAccessHandler`.
pub(crate) struct AnalyzeVariableScopeAccessHandler {
    pub entry: u32,
    pub accessed_variables_to_block: StdHashMap<StdHashSet>,
    pub accessed_temporaries_to_block: StdHashMap<StdHashSet>,
    pub result_id_to_type: HashMap<u32, u32>,
    pub complete_write_variables_to_block: StdHashMap<StdHashSet>,
    pub partial_write_variables_to_block: StdHashMap<StdHashSet>,
    pub access_chain_expressions: HashSet<u32>,
    // Access chains used in multiple blocks mean hoisting all the variables used to construct the access chain as not all backends can use pointers.
    // This is also relevant when forwarding opaque objects since we cannot lower these to temporaries.
    pub rvalue_forward_children: HashMap<u32, StdHashSet>,
    pub current_block: Option<u32>,
}

impl AnalyzeVariableScopeAccessHandler {
    pub fn new(entry: u32) -> Self {
        AnalyzeVariableScopeAccessHandler {
            entry,
            accessed_variables_to_block: StdHashMap::new(),
            accessed_temporaries_to_block: StdHashMap::new(),
            result_id_to_type: HashMap::new(),
            complete_write_variables_to_block: StdHashMap::new(),
            partial_write_variables_to_block: StdHashMap::new(),
            access_chain_expressions: HashSet::new(),
            rvalue_forward_children: HashMap::new(),
            current_block: None,
        }
    }

    fn notify_variable_access(&mut self, compiler: &Compiler, id: u32, block: u32) -> Result<()> {
        self.notify_variable_access_inner(compiler, id, block, 0)
    }

    fn notify_variable_access_inner(
        &mut self,
        compiler: &Compiler,
        id: u32,
        block: u32,
        depth: u32,
    ) -> Result<()> {
        if id == 0 {
            return Ok(());
        }
        // (An access chain can't contain itself in valid SPIR-V; C++
        // recurses forever.)
        if depth > 4096 {
            spirv_cross_throw!("Access chain recursion too deep.");
        }

        // Access chains used in multiple blocks mean hoisting all the variables used to construct the access chain as not all backends can use pointers.
        if let Some(children) = self.rvalue_forward_children.get(&id) {
            for child_id in children.to_vec() {
                self.notify_variable_access_inner(compiler, child_id, block, depth + 1)?;
            }
        }

        if self.id_is_phi_variable(compiler, id) {
            self.accessed_variables_to_block
                .entry_or_default(id)
                .insert(block);
        } else if self.id_is_potential_temporary(compiler, id) {
            self.accessed_temporaries_to_block
                .entry_or_default(id)
                .insert(block);
        }
        Ok(())
    }

    fn id_is_phi_variable(&self, compiler: &Compiler, id: u32) -> bool {
        if id >= compiler.get_current_id_bound() {
            return false;
        }
        match compiler.maybe_get::<SPIRVariable>(id) {
            Some(var) => var.phi_variable,
            None => false,
        }
    }

    fn id_is_potential_temporary(&self, compiler: &Compiler, id: u32) -> bool {
        if id >= compiler.get_current_id_bound() {
            return false;
        }

        // Temporaries are not created before we start emitting code.
        let v = &compiler.ir.ids[id as usize];
        v.empty() || v.get_type() == TypeExpression
    }

    fn cur(&self) -> u32 {
        self.current_block.unwrap_or(0)
    }
}

impl OpcodeHandler for AnalyzeVariableScopeAccessHandler {
    fn follow_function_call(
        &mut self,
        _compiler: &mut Compiler,
        _func: &SPIRFunction,
    ) -> Result<bool> {
        // Only analyze within this function.
        Ok(false)
    }

    fn set_current_block(&mut self, compiler: &mut Compiler, block: &SPIRBlock) -> Result<()> {
        self.current_block = Some(block.self_);

        // If we're branching to a block which uses OpPhi, in GLSL
        // this will be a variable write when we branch,
        // so we need to track access to these variables as well to
        // have a complete picture.
        let test_phi = |this: &mut Self, compiler: &Compiler, to: u32| -> Result<()> {
            let next = compiler.get::<SPIRBlock>(to)?;
            for phi in &next.phi_variables {
                if phi.parent == block.self_ {
                    this.accessed_variables_to_block
                        .entry_or_default(phi.function_variable)
                        .insert(block.self_);
                    // Phi variables are also accessed in our target branch block.
                    this.accessed_variables_to_block
                        .entry_or_default(phi.function_variable)
                        .insert(next.self_);

                    this.notify_variable_access(compiler, phi.local_variable, block.self_)?;
                }
            }
            Ok(())
        };

        match block.terminator {
            Terminator::Direct => {
                self.notify_variable_access(compiler, block.condition, block.self_)?;
                test_phi(self, compiler, block.next_block)?;
            }

            Terminator::Select => {
                self.notify_variable_access(compiler, block.condition, block.self_)?;
                test_phi(self, compiler, block.true_block)?;
                test_phi(self, compiler, block.false_block)?;
            }

            Terminator::MultiSelect => {
                self.notify_variable_access(compiler, block.condition, block.self_)?;
                let cases = compiler.get_case_list(block)?;
                for target in cases {
                    test_phi(self, compiler, target.block)?;
                }
                if block.default_block != 0 {
                    test_phi(self, compiler, block.default_block)?;
                }
            }

            _ => {}
        }
        Ok(())
    }

    fn handle_terminator(&mut self, compiler: &mut Compiler, block: &SPIRBlock) -> Result<bool> {
        match block.terminator {
            Terminator::Return => {
                if block.return_value != 0 {
                    self.notify_variable_access(compiler, block.return_value, block.self_)?;
                }
            }

            Terminator::Select | Terminator::MultiSelect => {
                self.notify_variable_access(compiler, block.condition, block.self_)?;
            }

            _ => {}
        }

        Ok(true)
    }

    fn handle(
        &mut self,
        compiler: &mut Compiler,
        op: Op,
        args: &[u32],
        length: u32,
    ) -> Result<bool> {
        let cur = self.cur();
        // Keep track of the types of temporaries, so we can hoist them out as necessary.
        let mut result_type = 0;
        let mut result_id = 0;
        if let Some((rt, rid)) = Compiler::instruction_to_result_type(op, args, length) {
            result_type = rt;
            result_id = rid;
            // For some opcodes, we will need to override the result id.
            // If we need to hoist the temporary, the temporary type is the input, not the result.
            if op == OpConvertUToAccelerationStructureKHR {
                if let Some(&t) = self.result_id_to_type.get(&args[2]) {
                    result_type = t;
                }
            }

            self.result_id_to_type.insert(result_id, result_type);
        }

        match op {
            OpStore | OpCooperativeMatrixStoreKHR => {
                if length < 2 {
                    return Ok(false);
                }

                let ptr = args[0];
                let var = compiler.maybe_get_backing_variable(ptr);

                // If we store through an access chain, we have a partial write.
                if let Some(var) = var {
                    self.accessed_variables_to_block
                        .entry_or_default(var)
                        .insert(cur);
                    if var == ptr {
                        self.complete_write_variables_to_block
                            .entry_or_default(var)
                            .insert(cur);
                    } else {
                        self.partial_write_variables_to_block
                            .entry_or_default(var)
                            .insert(cur);
                    }
                }

                // args[0] might be an access chain we have to track use of.
                self.notify_variable_access(compiler, args[0], cur)?;
                // Might try to store a Phi variable here.
                self.notify_variable_access(compiler, args[1], cur)?;
            }

            OpAccessChain | OpInBoundsAccessChain | OpPtrAccessChain => {
                if length < 3 {
                    return Ok(false);
                }

                // Access chains used in multiple blocks mean hoisting all the variables used to construct the access chain as not all backends can use pointers.
                let ptr = args[2];
                if compiler.maybe_get::<SPIRVariable>(ptr).is_some() {
                    self.accessed_variables_to_block
                        .entry_or_default(ptr)
                        .insert(cur);
                    self.rvalue_forward_children
                        .entry(args[1])
                        .or_default()
                        .insert(ptr);
                }

                // args[2] might be another access chain we have to track use of.
                for i in 2..length as usize {
                    self.notify_variable_access(compiler, args[i], cur)?;
                    self.rvalue_forward_children
                        .entry(args[1])
                        .or_default()
                        .insert(args[i]);
                }

                // Also keep track of the access chain pointer itself.
                // In exceptionally rare cases, we can end up with a case where
                // the access chain is generated in the loop body, but is consumed in continue block.
                // This means we need complex loop workarounds, and we must detect this via CFG analysis.
                self.notify_variable_access(compiler, args[1], cur)?;

                // The result of an access chain is a fixed expression and is not really considered a temporary.
                compiler.set(args[1], SPIRExpression::new(String::new(), args[0], true))?;
                let backing_variable = compiler.maybe_get_backing_variable(ptr);
                compiler.get_mut::<SPIRExpression>(args[1])?.loaded_from =
                    backing_variable.unwrap_or(0);

                // Other backends might use SPIRAccessChain for this later.
                compiler.ir.ids.at_mut(args[1])?.set_allow_type_rewrite();
                self.access_chain_expressions.insert(args[1]);
            }

            OpCopyMemory => {
                if length < 2 {
                    return Ok(false);
                }

                let lhs = args[0];
                let rhs = args[1];
                let var = compiler.maybe_get_backing_variable(lhs);

                // If we store through an access chain, we have a partial write.
                if let Some(var) = var {
                    self.accessed_variables_to_block
                        .entry_or_default(var)
                        .insert(cur);
                    if var == lhs {
                        self.complete_write_variables_to_block
                            .entry_or_default(var)
                            .insert(cur);
                    } else {
                        self.partial_write_variables_to_block
                            .entry_or_default(var)
                            .insert(cur);
                    }
                }

                // args[0:1] might be access chains we have to track use of.
                for &arg in args.iter().take(2) {
                    self.notify_variable_access(compiler, arg, cur)?;
                }

                if let Some(var) = compiler.maybe_get_backing_variable(rhs) {
                    self.accessed_variables_to_block
                        .entry_or_default(var)
                        .insert(cur);
                }
            }

            OpCopyObject => {
                // OpCopyObject copies the underlying non-pointer type,
                // so any temp variable should be declared using the underlying type.
                // If the type is a pointer, get its base type and overwrite the result type mapping.
                let type_ = compiler.get::<SPIRType>(result_type)?;
                if type_.pointer {
                    self.result_id_to_type.insert(result_id, type_.parent_type);
                }

                if length < 3 {
                    return Ok(false);
                }

                if let Some(var) = compiler.maybe_get_backing_variable(args[2]) {
                    self.accessed_variables_to_block
                        .entry_or_default(var)
                        .insert(cur);
                }

                // Might be an access chain which we have to keep track of.
                self.notify_variable_access(compiler, args[1], cur)?;
                if self.access_chain_expressions.contains(&args[2]) {
                    self.access_chain_expressions.insert(args[1]);
                }

                // Might try to copy a Phi variable here.
                self.notify_variable_access(compiler, args[2], cur)?;
            }

            OpLoad | OpCooperativeMatrixLoadKHR => {
                if length < 3 {
                    return Ok(false);
                }
                let ptr = args[2];
                if let Some(var) = compiler.maybe_get_backing_variable(ptr) {
                    self.accessed_variables_to_block
                        .entry_or_default(var)
                        .insert(cur);
                }

                // Loaded value is a temporary.
                self.notify_variable_access(compiler, args[1], cur)?;

                // Might be an access chain we have to track use of.
                self.notify_variable_access(compiler, args[2], cur)?;

                // If we're loading an opaque type we cannot lower it to a temporary,
                // we must defer access of args[2] until it's used.
                let type_ = compiler.get::<SPIRType>(args[0])?;
                if compiler.type_is_opaque_value(type_) {
                    self.rvalue_forward_children
                        .entry(args[1])
                        .or_default()
                        .insert(args[2]);
                }
            }

            OpFunctionCall => {
                if length < 3 {
                    return Ok(false);
                }

                // Return value may be a temporary.
                if compiler.get_type(args[0])?.basetype != BaseType::Void {
                    self.notify_variable_access(compiler, args[1], cur)?;
                }

                let length = length - 3;
                let args = &args[3..];

                for &arg in args.iter().take(length as usize) {
                    if let Some(var) = compiler.maybe_get_backing_variable(arg) {
                        self.accessed_variables_to_block
                            .entry_or_default(var)
                            .insert(cur);
                        // Assume we can get partial writes to this variable.
                        self.partial_write_variables_to_block
                            .entry_or_default(var)
                            .insert(cur);
                    }

                    // Cannot easily prove if argument we pass to a function is completely written.
                    // Usually, functions write to a dummy variable,
                    // which is then copied to in full to the real argument.

                    // Might try to copy a Phi variable here.
                    self.notify_variable_access(compiler, arg, cur)?;
                }
            }

            OpSelect => {
                // In case of variable pointers, we might access a variable here.
                // We cannot prove anything about these accesses however.
                for i in 1..length as usize {
                    if i >= 3 {
                        if let Some(var) = compiler.maybe_get_backing_variable(args[i]) {
                            self.accessed_variables_to_block
                                .entry_or_default(var)
                                .insert(cur);
                            // Assume we can get partial writes to this variable.
                            self.partial_write_variables_to_block
                                .entry_or_default(var)
                                .insert(cur);
                        }
                    }

                    // Might try to copy a Phi variable here.
                    self.notify_variable_access(compiler, args[i], cur)?;
                }
            }

            OpExtInst => {
                for i in 4..length as usize {
                    self.notify_variable_access(compiler, args[i], cur)?;
                }
                self.notify_variable_access(compiler, args[1], cur)?;

                let extension_set = args[2];
                if compiler.get::<SPIRExtension>(extension_set)?.ext == Extension::GLSL {
                    let op_450 = args[3];
                    if op_450 == GLSLstd450Modf || op_450 == GLSLstd450Frexp {
                        let ptr = args[5];
                        if let Some(var) = compiler.maybe_get_backing_variable(ptr) {
                            self.accessed_variables_to_block
                                .entry_or_default(var)
                                .insert(cur);
                            if var == ptr {
                                self.complete_write_variables_to_block
                                    .entry_or_default(var)
                                    .insert(cur);
                            } else {
                                self.partial_write_variables_to_block
                                    .entry_or_default(var)
                                    .insert(cur);
                            }
                        }
                    }
                }
            }

            OpArrayLength => {
                // Only result is a temporary.
                self.notify_variable_access(compiler, args[1], cur)?;
            }

            OpLine | OpNoLine => {
                // Uses literals, but cannot be a phi variable or temporary, so ignore.
            }

            // Atomics shouldn't be able to access function-local variables.
            // Some GLSL builtins access a pointer.
            OpCompositeInsert | OpVectorShuffle => {
                // Specialize for opcode which contains literals.
                for &arg in &args[1..4] {
                    self.notify_variable_access(compiler, arg, cur)?;
                }
            }

            OpCompositeExtract => {
                // Specialize for opcode which contains literals.
                for &arg in &args[1..3] {
                    self.notify_variable_access(compiler, arg, cur)?;
                }
            }

            OpImageWrite => {
                for i in 0..length as usize {
                    // Argument 3 is a literal.
                    if i != 3 {
                        self.notify_variable_access(compiler, args[i], cur)?;
                    }
                }
            }

            OpImageSampleImplicitLod
            | OpImageSampleExplicitLod
            | OpImageSparseSampleImplicitLod
            | OpImageSparseSampleExplicitLod
            | OpImageSampleProjImplicitLod
            | OpImageSampleProjExplicitLod
            | OpImageSparseSampleProjImplicitLod
            | OpImageSparseSampleProjExplicitLod
            | OpImageFetch
            | OpImageSparseFetch
            | OpImageRead
            | OpImageSparseRead => {
                for i in 1..length as usize {
                    // Argument 4 is a literal.
                    if i != 4 {
                        self.notify_variable_access(compiler, args[i], cur)?;
                    }
                }
            }

            OpImageSampleDrefImplicitLod
            | OpImageSampleDrefExplicitLod
            | OpImageSparseSampleDrefImplicitLod
            | OpImageSparseSampleDrefExplicitLod
            | OpImageSampleProjDrefImplicitLod
            | OpImageSampleProjDrefExplicitLod
            | OpImageSparseSampleProjDrefImplicitLod
            | OpImageSparseSampleProjDrefExplicitLod
            | OpImageGather
            | OpImageSparseGather
            | OpImageDrefGather
            | OpImageSparseDrefGather => {
                for i in 1..length as usize {
                    // Argument 5 is a literal.
                    if i != 5 {
                        self.notify_variable_access(compiler, args[i], cur)?;
                    }
                }
            }

            _ => {
                // Rather dirty way of figuring out where Phi variables are used.
                // As long as only IDs are used, we can scan through instructions and try to find any evidence that
                // the ID of a variable has been used.
                // There are potential false positives here where a literal is used in-place of an ID,
                // but worst case, it does not affect the correctness of the compile.
                // Exhaustive analysis would be better here, but it's not worth it for now.
                for i in 0..length as usize {
                    self.notify_variable_access(compiler, args[i], cur)?;
                }
            }
        }
        Ok(true)
    }
}

/// `StaticExpressionAccessHandler`.
struct StaticExpressionAccessHandler {
    variable_id: u32,
    static_expression: u32,
    write_count: u32,
}

impl OpcodeHandler for StaticExpressionAccessHandler {
    fn follow_function_call(
        &mut self,
        _compiler: &mut Compiler,
        _func: &SPIRFunction,
    ) -> Result<bool> {
        Ok(false)
    }

    fn handle(
        &mut self,
        _compiler: &mut Compiler,
        op: Op,
        args: &[u32],
        length: u32,
    ) -> Result<bool> {
        match op {
            OpStore | OpCooperativeMatrixStoreKHR => {
                if length < 2 {
                    return Ok(false);
                }
                if args[0] == self.variable_id {
                    self.static_expression = args[1];
                    self.write_count += 1;
                }
            }

            OpLoad | OpCooperativeMatrixLoadKHR => {
                if length < 3 {
                    return Ok(false);
                }
                // Tried to read from variable before it was initialized.
                if args[2] == self.variable_id && self.static_expression == 0 {
                    return Ok(false);
                }
            }

            OpAccessChain | OpInBoundsAccessChain | OpPtrAccessChain => {
                if length < 3 {
                    return Ok(false);
                }
                // If we try to access chain our candidate variable before we store to it, bail.
                if args[2] == self.variable_id {
                    return Ok(false);
                }
            }

            _ => {}
        }

        Ok(true)
    }
}

impl Compiler {
    pub(crate) fn find_function_local_luts(
        &mut self,
        entry: u32,
        handler: &AnalyzeVariableScopeAccessHandler,
        single_function: bool,
    ) -> Result<()> {
        let Some(cfg) = self.function_cfgs.get(&entry).cloned() else {
            spirv_cross_throw!("No CFG for function.");
        };

        // For each variable which is statically accessed.
        for (accessed_var, blocks) in handler.accessed_variables_to_block.iter() {
            let var_obj = self.get::<SPIRVariable>(accessed_var)?;
            let var_self = var_obj.self_;
            let type_ = self.expression_type(accessed_var)?;
            let type_array_empty = type_.array.is_empty();

            // First check if there are writes to the variable. Later, if there are none, we'll
            // reconsider it as globally accessed LUT.
            if !var_obj.is_written_to {
                let written = handler
                    .complete_write_variables_to_block
                    .contains_key(&var_self)
                    || handler
                        .partial_write_variables_to_block
                        .contains_key(&var_self);
                self.get_mut::<SPIRVariable>(accessed_var)?.is_written_to = written;
            }

            let var = self.get::<SPIRVariable>(accessed_var)?;

            // Only consider function local variables here.
            // If we only have a single function in our CFG, private storage is also fine,
            // since it behaves like a function local variable.
            let allow_lut = var.storage == StorageClassFunction
                || (single_function && var.storage == StorageClassPrivate);
            if !allow_lut {
                continue;
            }

            // We cannot be a phi variable.
            if var.phi_variable {
                continue;
            }

            // Only consider arrays here.
            if type_array_empty {
                continue;
            }

            // If the variable has an initializer, make sure it is a constant expression.
            let static_constant_expression;
            if var.initializer != 0 {
                if self.id_type(var.initializer) != TypeConstant {
                    continue;
                }
                static_constant_expression = var.initializer;

                // There can be no stores to this variable, we have now proved we have a LUT.
                if var.is_written_to {
                    continue;
                }
            } else {
                // We can have one, and only one write to the variable, and that write needs to be a constant.

                // No partial writes allowed.
                if handler
                    .partial_write_variables_to_block
                    .contains_key(&var_self)
                {
                    continue;
                }

                // No writes?
                let Some(write_blocks) = handler.complete_write_variables_to_block.get(&var_self)
                else {
                    continue;
                };

                // We write to the variable in more than one block.
                if write_blocks.len() != 1 {
                    continue;
                }

                // The write needs to happen in the dominating block.
                let mut builder = DominatorBuilder::new();
                for block in blocks.iter() {
                    builder.add_block(&cfg, block)?;
                }
                let dominator = builder.get_dominator();

                // The complete write happened in a branch or similar, cannot deduce static expression.
                if !write_blocks.contains(&dominator) {
                    continue;
                }

                // Find the static expression for this variable.
                let mut static_expression_handler = StaticExpressionAccessHandler {
                    variable_id: var_self,
                    static_expression: 0,
                    write_count: 0,
                };
                self.traverse_all_reachable_opcodes_block(
                    dominator,
                    &mut static_expression_handler,
                )?;

                // We want one, and exactly one write
                if static_expression_handler.write_count != 1
                    || static_expression_handler.static_expression == 0
                {
                    continue;
                }

                // Is it a constant expression?
                if self.id_type(static_expression_handler.static_expression) != TypeConstant {
                    continue;
                }

                // We found a LUT!
                static_constant_expression = static_expression_handler.static_expression;
            }

            self.get_mut::<SPIRConstant>(static_constant_expression)?
                .is_used_as_lut = true;
            let var = self.get_mut::<SPIRVariable>(accessed_var)?;
            var.static_expression = static_constant_expression;
            var.statically_assigned = true;
            var.remapped_variable = true;
        }
        Ok(())
    }

    pub(crate) fn analyze_variable_scope(
        &mut self,
        entry: u32,
        handler: &mut AnalyzeVariableScopeAccessHandler,
    ) -> Result<()> {
        // First, we map out all variable access within a function.
        // Essentially a map of block -> { variables accessed in the basic block }
        self.traverse_all_reachable_opcodes_func(entry, handler)?;

        let Some(cfg) = self.function_cfgs.get(&entry).cloned() else {
            spirv_cross_throw!("No CFG for function.");
        };

        // Analyze if there are parameters which need to be implicitly preserved with an "in" qualifier.
        self.analyze_parameter_preservation(
            entry,
            &cfg,
            &handler.accessed_variables_to_block,
            &handler.complete_write_variables_to_block,
        )?;

        let mut potential_loop_variables: StdHashMap<u32> = StdHashMap::new();

        // Find the loop dominator block for each block.
        let entry_blocks = self.get::<SPIRFunction>(entry)?.blocks.clone();
        for &block_id in &entry_blocks {
            let lh = self
                .ir
                .continue_block_to_loop_header
                .get(&block_id)
                .copied();
            if let Some(h) = lh.filter(|&h| h != block_id) {
                // Continue block might be unreachable in the CFG, but we still like to know the loop dominator.
                // Edge case is when continue block is also the loop header, don't set the dominator in this case.
                self.get_mut::<SPIRBlock>(block_id)?.loop_dominator = h;
            } else {
                let loop_dominator = cfg.find_loop_dominator(self, block_id)?;
                let block = self.get_mut::<SPIRBlock>(block_id)?;
                if loop_dominator != block_id {
                    block.loop_dominator = loop_dominator;
                } else {
                    block.loop_dominator = SPIRBlock::NO_DOMINATOR;
                }
            }
        }

        // For each variable which is statically accessed.
        let entry_locals = self.get::<SPIRFunction>(entry)?.local_variables.clone();
        for (var_first, blocks) in handler.accessed_variables_to_block.iter() {
            // Only deal with variables which are considered local variables in this function.
            if !entry_locals.contains(&var_first) {
                continue;
            }

            let mut builder = DominatorBuilder::new();
            let type_ = self.expression_type_rc(var_first)?;
            let mut potential_continue_block: BlockID = 0;

            // Figure out which block is dominating all accesses of those variables.
            for block in blocks.iter() {
                // If we're accessing a variable inside a continue block, this variable might be a loop variable.
                // We can only use loop variables with scalars, as we cannot track static expressions for vectors.
                if self.is_continue(block)? {
                    // Potentially awkward case to check for.
                    // We might have a variable inside a loop, which is touched by the continue block,
                    // but is not actually a loop variable.
                    // The continue block is dominated by the inner part of the loop, which does not make sense in high-level
                    // language output because it will be declared before the body,
                    // so we will have to lift the dominator up to the relevant loop header instead.
                    let header = *self
                        .ir
                        .continue_block_to_loop_header
                        .entry(block)
                        .or_default();
                    builder.add_block(&cfg, header)?;

                    // Arrays or structs cannot be loop variables.
                    if type_.vecsize == 1
                        && type_.columns == 1
                        && type_.basetype != BaseType::Struct
                        && type_.array.is_empty()
                    {
                        // The variable is used in multiple continue blocks, this is not a loop
                        // candidate, signal that by setting block to -1u.
                        if potential_continue_block == 0 {
                            potential_continue_block = block;
                        } else {
                            potential_continue_block = !0u32;
                        }
                    }
                }

                builder.add_block(&cfg, block)?;
            }

            builder.lift_continue_block_dominator(self, &cfg)?;

            // Add it to a per-block list of variables.
            let mut dominating_block: BlockID = builder.get_dominator();

            if dominating_block != 0
                && potential_continue_block != 0
                && potential_continue_block != !0u32
            {
                let inner_block = self.get::<SPIRBlock>(dominating_block)?;

                let mut merge_candidate: BlockID = 0;

                // Analyze the dominator. If it lives in a different loop scope than the candidate continue
                // block, reject the loop variable candidate.
                if inner_block.merge == Merge::MergeLoop {
                    merge_candidate = inner_block.merge_block;
                } else if inner_block.loop_dominator != SPIRBlock::NO_DOMINATOR {
                    merge_candidate = self
                        .get::<SPIRBlock>(inner_block.loop_dominator)?
                        .merge_block;
                }

                if merge_candidate != 0 && cfg.is_reachable(merge_candidate) {
                    // If the merge block has a higher post-visit order, we know that continue candidate
                    // cannot reach the merge block, and we have two separate scopes.
                    if !cfg.is_reachable(potential_continue_block)
                        || cfg.get_visit_order(merge_candidate)?
                            > cfg.get_visit_order(potential_continue_block)?
                    {
                        potential_continue_block = 0;
                    }
                }
            }

            if potential_continue_block != 0 && potential_continue_block != !0u32 {
                potential_loop_variables.insert(var_first, potential_continue_block);
            }

            // For variables whose dominating block is inside a loop, there is a risk that these variables
            // actually need to be preserved across loop iterations. We can express this by adding
            // a "read" access to the loop header.
            // In the dominating block, we must see an OpStore or equivalent as the first access of an OpVariable.
            // Should that fail, we look for the outermost loop header and tack on an access there.
            // Phi nodes cannot have this problem.
            if dominating_block != 0 {
                let variable = self.get::<SPIRVariable>(var_first)?;
                if !variable.phi_variable {
                    let mut block = self.get::<SPIRBlock>(dominating_block)?;
                    let preserve = self.may_read_undefined_variable_in_block(block, var_first)?;
                    if preserve {
                        // Find the outermost loop scope.
                        let mut steps: usize = 0;
                        while block.loop_dominator != SPIRBlock::NO_DOMINATOR {
                            block = self.get::<SPIRBlock>(block.loop_dominator)?;
                            // (A cycle of loop dominators loops forever in C++.)
                            steps += 1;
                            if steps > self.ir.ids.len() {
                                spirv_cross_throw!("Loop dominator cycle.");
                            }
                        }

                        if block.self_ != dominating_block {
                            builder.add_block(&cfg, block.self_)?;
                            dominating_block = builder.get_dominator();
                        }
                    }
                }
            }

            // If all blocks here are dead code, this will be 0, so the variable in question
            // will be completely eliminated.
            if dominating_block != 0 {
                self.get_mut::<SPIRBlock>(dominating_block)?
                    .dominated_variables
                    .push(var_first);
                self.get_mut::<SPIRVariable>(var_first)?.dominator = dominating_block;
            }
        }

        for (var_first, blocks) in handler.accessed_temporaries_to_block.iter() {
            let Some(&ty) = handler.result_id_to_type.get(&var_first) else {
                // We found a false positive ID being used, ignore.
                // This should probably be an assert.
                continue;
            };

            // There is no point in doing domination analysis for opaque types.
            let type_ = self.get::<SPIRType>(ty)?;
            if self.type_is_opaque_value(type_) {
                continue;
            }

            let mut builder = DominatorBuilder::new();
            let mut force_temporary = false;
            let mut used_in_header_hoisted_continue_block = false;

            // Figure out which block is dominating all accesses of those temporaries.
            for block in blocks.iter() {
                builder.add_block(&cfg, block)?;

                if blocks.len() != 1 && self.is_continue(block)? {
                    // The risk here is that inner loop can dominate the continue block.
                    // Any temporary we access in the continue block must be declared before the loop.
                    // This is moot for complex loops however.
                    let header = *self
                        .ir
                        .continue_block_to_loop_header
                        .entry(block)
                        .or_default();
                    let loop_header_block = self.get::<SPIRBlock>(header)?;
                    // assert(loop_header_block.merge == SPIRBlock::MergeLoop);
                    builder.add_block(&cfg, loop_header_block.self_)?;
                    used_in_header_hoisted_continue_block = true;
                }
            }

            let dominating_block = builder.get_dominator();

            if blocks.len() != 1 && self.is_single_block_loop(dominating_block)? {
                // Awkward case, because the loop header is also the continue block,
                // so hoisting to loop header does not help.
                force_temporary = true;
            }

            if dominating_block != 0 {
                // If we touch a variable in the dominating block, this is the expected setup.
                // SPIR-V normally mandates this, but we have extra cases for temporary use inside loops.
                let first_use_is_dominator = blocks.contains(&dominating_block);

                if !first_use_is_dominator || force_temporary {
                    if handler.access_chain_expressions.contains(&var_first) {
                        // Exceptionally rare case.
                        // We cannot declare temporaries of access chains (except on MSL perhaps with pointers).
                        // Rather than do that, we force the indexing expressions to be declared in the right scope by
                        // tracking their usage to that end. There is no temporary to hoist.
                        // However, we still need to observe declaration order of the access chain.

                        if used_in_header_hoisted_continue_block {
                            // For this scenario, we used an access chain inside a continue block where we also registered an access to header block.
                            // This is a problem as we need to declare an access chain properly first with full definition.
                            // We cannot use temporaries for these expressions,
                            // so we must make sure the access chain is declared ahead of time.
                            // Force a complex for loop to deal with this.
                            // TODO: Out-of-order declaring for loops where continue blocks are emitted last might be another option.
                            let loop_header_block = self.get_mut::<SPIRBlock>(dominating_block)?;
                            // assert(loop_header_block.merge == SPIRBlock::MergeLoop);
                            loop_header_block.complex_continue = true;
                        }
                    } else {
                        // This should be very rare, but if we try to declare a temporary inside a loop,
                        // and that temporary is used outside the loop as well (spirv-opt inliner likes this)
                        // we should actually emit the temporary outside the loop.
                        self.hoisted_temporaries.insert(var_first);
                        self.forced_temporaries.insert(var_first);

                        let t = handler
                            .result_id_to_type
                            .get(&var_first)
                            .copied()
                            .unwrap_or(0);
                        self.get_mut::<SPIRBlock>(dominating_block)?
                            .declare_temporary
                            .push((t, var_first));
                    }
                } else if blocks.len() > 1 {
                    // Keep track of the temporary as we might have to declare this temporary.
                    // This can happen if the loop header dominates a temporary, but we have a complex fallback loop.
                    // In this case, the header is actually inside the for (;;) {} block, and we have problems.
                    // What we need to do is hoist the temporaries outside the for (;;) {} block in case the header block
                    // declares the temporary.
                    let t = handler
                        .result_id_to_type
                        .get(&var_first)
                        .copied()
                        .unwrap_or(0);
                    self.get_mut::<SPIRBlock>(dominating_block)?
                        .potential_declare_temporary
                        .push((t, var_first));
                }
            }
        }

        let mut seen_blocks: HashSet<u32> = HashSet::new();

        // Now, try to analyze whether or not these variables are actually loop variables.
        for (loop_variable_first, loop_variable_second) in potential_loop_variables.iter() {
            let var = self.get::<SPIRVariable>(loop_variable_first)?;
            let mut dominator = var.dominator;
            let block: BlockID = *loop_variable_second;

            // The variable was accessed in multiple continue blocks, ignore.
            if block == !0u32 || block == 0 {
                continue;
            }

            // Dead code.
            if dominator == 0 {
                continue;
            }

            let mut header: BlockID = 0;

            // Find the loop header for this block if we are a continue block.
            {
                if let Some(&h) = self.ir.continue_block_to_loop_header.get(&block) {
                    header = h;
                } else if self.get::<SPIRBlock>(block)?.continue_block == block {
                    // Also check for self-referential continue block.
                    header = block;
                }
            }

            // assert(header);
            let header_block_merge = self.get::<SPIRBlock>(header)?.merge_block;
            let blocks = handler
                .accessed_variables_to_block
                .entry_or_default(loop_variable_first)
                .clone();

            // If a loop variable is not used before the loop, it's probably not a loop variable.
            let mut has_accessed_variable = blocks.contains(&header);

            // Now, there are two conditions we need to meet for the variable to be a loop variable.
            // 1. The dominating block must have a branch-free path to the loop header,
            // this way we statically know which expression should be part of the loop variable initializer.

            // Walk from the dominator, if there is one straight edge connecting
            // dominator and loop header, we statically know the loop initializer.
            let mut static_loop_init = true;
            let mut steps: usize = 0;
            while dominator != header {
                if blocks.contains(&dominator) {
                    has_accessed_variable = true;
                }

                let succ = cfg.get_succeeding_edges(dominator);
                if succ.len() != 1 {
                    static_loop_init = false;
                    break;
                }

                let pred = cfg.get_preceding_edges(succ[0]);
                if pred.len() != 1 || pred[0] != dominator {
                    static_loop_init = false;
                    break;
                }

                dominator = succ[0];
                // (A cycle loops forever in C++.)
                steps += 1;
                if steps > self.ir.ids.len() {
                    static_loop_init = false;
                    break;
                }
            }

            if !static_loop_init || !has_accessed_variable {
                continue;
            }

            // The second condition we need to meet is that no access after the loop
            // merge can occur. Walk the CFG to see if we find anything.

            seen_blocks.clear();
            cfg.walk_from(&mut seen_blocks, header_block_merge, &mut |walk_block| {
                // We found a block which accesses the variable outside the loop.
                if blocks.contains(&walk_block) {
                    static_loop_init = false;
                }
                Ok(true)
            })?;

            if !static_loop_init {
                continue;
            }

            // We have a loop variable.
            let header_block = self.get_mut::<SPIRBlock>(header)?;
            header_block.loop_variables.push(loop_variable_first);
            // Need to sort here as variables come from an unordered container, and pushing stuff in wrong order
            // will break reproducability in regression runs.
            header_block.loop_variables.sort();
            self.get_mut::<SPIRVariable>(loop_variable_first)?
                .loop_variable = true;
        }
        Ok(())
    }

    pub(crate) fn may_read_undefined_variable_in_block(
        &self,
        block: &SPIRBlock,
        var: u32,
    ) -> Result<bool> {
        for op in &block.ops {
            let ops = self.stream(op)?;
            match op.op as Op {
                OpStore | OpCooperativeMatrixStoreKHR | OpCopyMemory => {
                    if ops[0] == var {
                        return Ok(false);
                    }
                }

                OpAccessChain | OpInBoundsAccessChain | OpPtrAccessChain => {
                    // Access chains are generally used to partially read and write. It's too hard to analyze
                    // if all constituents are written fully before continuing, so just assume it's preserved.
                    // This is the same as the parameter preservation analysis.
                    if ops[2] == var {
                        return Ok(true);
                    }
                }

                OpSelect => {
                    // Variable pointers.
                    // We might read before writing.
                    if ops[3] == var || ops[4] == var {
                        return Ok(true);
                    }
                }

                OpPhi => {
                    // Variable pointers.
                    // We might read before writing.
                    if op.length < 2 {
                        continue;
                    }

                    let count = op.length - 2;
                    let mut i = 0;
                    while i < count as usize {
                        if ops[i + 2] == var {
                            return Ok(true);
                        }
                        i += 2;
                    }
                }

                OpCopyObject | OpLoad | OpCooperativeVectorLoadNV | OpCooperativeMatrixLoadKHR => {
                    if ops[2] == var {
                        return Ok(true);
                    }
                }

                OpFunctionCall => {
                    if op.length < 3 {
                        continue;
                    }

                    // May read before writing.
                    let count = op.length - 3;
                    for i in 0..count as usize {
                        if ops[i + 3] == var {
                            return Ok(true);
                        }
                    }
                }

                _ => {}
            }
        }

        // Not accessed somehow, at least not in a usual fashion.
        // It's likely accessed in a branch, so assume we must preserve.
        Ok(true)
    }
}

/// `GeometryEmitDisocveryHandler`.
struct GeometryEmitDisocveryHandler {
    function_stack: Vec<u32>,
}

impl OpcodeHandler for GeometryEmitDisocveryHandler {
    fn handle(
        &mut self,
        compiler: &mut Compiler,
        opcode: Op,
        _args: &[u32],
        _length: u32,
    ) -> Result<bool> {
        if opcode == OpEmitVertex || opcode == OpEndPrimitive {
            for &func in &self.function_stack {
                compiler.get_mut::<SPIRFunction>(func)?.emits_geometry = true;
            }
        }

        Ok(true)
    }

    fn begin_function_scope(
        &mut self,
        compiler: &mut Compiler,
        stream: &[u32],
        _length: u32,
    ) -> Result<bool> {
        let callee = compiler.get::<SPIRFunction>(stream[2])?.self_;
        let _ = callee;
        self.function_stack.push(stream[2]);
        Ok(true)
    }

    fn end_function_scope(
        &mut self,
        _compiler: &mut Compiler,
        _stream: &[u32],
        _length: u32,
    ) -> Result<bool> {
        // assert(function_stack.back() == &compiler.get<SPIRFunction>(stream[2]));
        self.function_stack.pop();

        Ok(true)
    }
}

impl Compiler {
    pub(crate) fn discover_geometry_emitters(&mut self) -> Result<()> {
        let mut handler = GeometryEmitDisocveryHandler {
            function_stack: Vec::new(),
        };

        let entry = self.ir.default_entry_point;
        self.traverse_all_reachable_opcodes_func(entry, &mut handler)?;
        Ok(())
    }

    /// `get_buffer_block_flags()`.
    pub fn get_buffer_block_flags(&self, id: VariableID) -> Result<Bitset> {
        self.ir
            .get_buffer_block_flags(self.get::<SPIRVariable>(id)?)
    }

    /// `get_common_basic_type()`.
    pub(crate) fn get_common_basic_type(
        &self,
        type_: &SPIRType,
        base_type: &mut BaseType,
    ) -> Result<bool> {
        self.get_common_basic_type_inner(type_, base_type, 0)
    }

    fn get_common_basic_type_inner(
        &self,
        type_: &SPIRType,
        base_type: &mut BaseType,
        depth: u32,
    ) -> Result<bool> {
        if depth > 1024 {
            spirv_cross_throw!("Type recursion too deep.");
        }
        if type_.basetype == BaseType::Struct {
            *base_type = BaseType::Unknown;
            for &member_type in &type_.member_types {
                let mut member_base = BaseType::Unknown;
                if !self.get_common_basic_type_inner(
                    self.get::<SPIRType>(member_type)?,
                    &mut member_base,
                    depth + 1,
                )? {
                    return Ok(false);
                }

                if *base_type == BaseType::Unknown {
                    *base_type = member_base;
                } else if *base_type != member_base {
                    return Ok(false);
                }
            }
            Ok(true)
        } else {
            *base_type = type_.basetype;
            Ok(true)
        }
    }
}

/// `ActiveBuiltinHandler`.
struct ActiveBuiltinHandler;

impl ActiveBuiltinHandler {
    fn handle_builtin(
        compiler: &mut Compiler,
        type_: &SPIRType,
        builtin: BuiltIn,
        decoration_flags: &Bitset,
    ) -> Result<()> {
        // If used, we will need to explicitly declare a new array size for these builtins.

        if builtin == BuiltInClipDistance {
            if !*type_.array_size_literal.at(0)? {
                spirv_cross_throw!("Array size for ClipDistance must be a literal.");
            }
            let array_size = *type_.array.at(0)?;
            if array_size == 0 {
                spirv_cross_throw!("Array size for ClipDistance must not be unsized.");
            }
            compiler.clip_distance_count = array_size;
        } else if builtin == BuiltInCullDistance {
            if !*type_.array_size_literal.at(0)? {
                spirv_cross_throw!("Array size for CullDistance must be a literal.");
            }
            let array_size = *type_.array.at(0)?;
            if array_size == 0 {
                spirv_cross_throw!("Array size for CullDistance must not be unsized.");
            }
            compiler.cull_distance_count = array_size;
        } else if builtin == BuiltInPosition && decoration_flags.get(DecorationInvariant) {
            compiler.position_invariant = true;
        }
        Ok(())
    }

    fn add_if_builtin_blocks(compiler: &mut Compiler, id: u32, allow_blocks: bool) -> Result<()> {
        // Only handle plain variables here.
        // Builtins which are part of a block are handled in AccessChain.
        // If allow_blocks is used however, this is to handle initializers of blocks,
        // which implies that all members are written to.

        let Some(var) = compiler.maybe_get_rc::<SPIRVariable>(id) else {
            return Ok(());
        };
        let Some(m) = compiler.ir.find_meta(id).cloned() else {
            return Ok(());
        };
        let type_ = compiler.get_rc::<SPIRType>(var.basetype)?;
        let decorations = &m.decoration;
        let input = type_.storage == StorageClassInput;
        if decorations.builtin {
            if input {
                compiler.active_input_builtins.set(decorations.builtin_type);
            } else {
                compiler
                    .active_output_builtins
                    .set(decorations.builtin_type);
            }
            Self::handle_builtin(
                compiler,
                &type_,
                decorations.builtin_type,
                &decorations.decoration_flags,
            )?;
        } else if allow_blocks && compiler.has_decoration(type_.self_, DecorationBlock) {
            let member_count = type_.member_types.len() as u32;
            for i in 0..member_count {
                if compiler.has_member_decoration(type_.self_, i, DecorationBuiltIn) {
                    let member_type =
                        compiler.get_rc::<SPIRType>(type_.member_types[i as usize])?;
                    let builtin = compiler.get_member_decoration(type_.self_, i, DecorationBuiltIn);
                    if input {
                        compiler.active_input_builtins.set(builtin);
                    } else {
                        compiler.active_output_builtins.set(builtin);
                    }
                    let flags = compiler
                        .get_member_decoration_bitset(type_.self_, i)
                        .clone();
                    Self::handle_builtin(compiler, &member_type, builtin, &flags)?;
                }
            }
        }
        Ok(())
    }

    fn add_if_builtin(compiler: &mut Compiler, id: u32) -> Result<()> {
        Self::add_if_builtin_blocks(compiler, id, false)
    }

    fn add_if_builtin_or_block(compiler: &mut Compiler, id: u32) -> Result<()> {
        Self::add_if_builtin_blocks(compiler, id, true)
    }
}

impl OpcodeHandler for ActiveBuiltinHandler {
    fn handle(
        &mut self,
        compiler: &mut Compiler,
        opcode: Op,
        args: &[u32],
        length: u32,
    ) -> Result<bool> {
        match opcode {
            OpStore | OpCooperativeMatrixStoreKHR => {
                if length < 1 {
                    return Ok(false);
                }

                Self::add_if_builtin(compiler, args[0])?;
            }

            OpCopyMemory => {
                if length < 2 {
                    return Ok(false);
                }

                Self::add_if_builtin(compiler, args[0])?;
                Self::add_if_builtin(compiler, args[1])?;
            }

            OpCopyObject | OpLoad | OpCooperativeMatrixLoadKHR => {
                if length < 3 {
                    return Ok(false);
                }

                Self::add_if_builtin(compiler, args[2])?;
            }

            OpSelect => {
                if length < 5 {
                    return Ok(false);
                }

                Self::add_if_builtin(compiler, args[3])?;
                Self::add_if_builtin(compiler, args[4])?;
            }

            OpPhi => {
                if length < 2 {
                    return Ok(false);
                }

                let count = length - 2;
                let args = &args[2..];
                let mut i = 0;
                while i < count as usize {
                    Self::add_if_builtin(compiler, args[i])?;
                    i += 2;
                }
            }

            OpFunctionCall => {
                if length < 3 {
                    return Ok(false);
                }

                let count = length - 3;
                let args = &args[3..];
                for &arg in args.iter().take(count as usize) {
                    Self::add_if_builtin(compiler, arg)?;
                }
            }

            OpAccessChain | OpInBoundsAccessChain | OpPtrAccessChain => {
                if length < 4 {
                    return Ok(false);
                }

                // Only consider global variables, cannot consider variables in functions yet, or other
                // access chains as they have not been created yet.
                let Some(var) = compiler.maybe_get_rc::<SPIRVariable>(args[2]) else {
                    return Ok(true);
                };

                // Required if we access chain into builtins like gl_GlobalInvocationID.
                Self::add_if_builtin(compiler, args[2])?;

                // Start traversing type hierarchy at the proper non-pointer types.
                let mut type_ = compiler.get_variable_data_type(&var)?.clone();

                let input = var.storage == StorageClassInput;

                let count = length - 3;
                let args = &args[3..];
                for i in 0..count as usize {
                    // Pointers
                    // PtrAccessChain functions more like a pointer offset. Type remains the same.
                    if opcode == OpPtrAccessChain && i == 0 {
                        continue;
                    }

                    // Arrays
                    if !type_.array.is_empty() {
                        type_ = compiler.get::<SPIRType>(type_.parent_type)?.clone();
                    }
                    // Structs
                    else if type_.basetype == BaseType::Struct {
                        let index = compiler.get::<SPIRConstant>(args[i])?.scalar(0, 0);

                        let member = compiler
                            .ir
                            .meta
                            .entry(type_.self_)
                            .or_default()
                            .members
                            .get(index as usize)
                            .cloned();
                        if let Some(decorations) = &member {
                            if decorations.builtin {
                                if input {
                                    compiler.active_input_builtins.set(decorations.builtin_type);
                                } else {
                                    compiler
                                        .active_output_builtins
                                        .set(decorations.builtin_type);
                                }
                                let mt =
                                    compiler.get_rc::<SPIRType>(*type_.member_types.at(index)?)?;
                                Self::handle_builtin(
                                    compiler,
                                    &mt,
                                    decorations.builtin_type,
                                    &decorations.decoration_flags,
                                )?;
                            }
                        }

                        type_ = compiler
                            .get::<SPIRType>(*type_.member_types.at(index)?)?
                            .clone();
                    } else {
                        // No point in traversing further. We won't find any extra builtins.
                        break;
                    }
                }
            }

            _ => {}
        }

        Ok(true)
    }
}

impl Compiler {
    /// `update_active_builtins()`.
    pub fn update_active_builtins(&mut self) -> Result<()> {
        self.active_input_builtins.reset();
        self.active_output_builtins.reset();
        self.cull_distance_count = 0;
        self.clip_distance_count = 0;
        let mut handler = ActiveBuiltinHandler;
        let entry = self.ir.default_entry_point;
        self.traverse_all_reachable_opcodes_func(entry, &mut handler)?;

        for id in self.ir.typed_ids(TypeVariable) {
            let var = self.get::<SPIRVariable>(id)?;
            if var.storage != StorageClassOutput {
                continue;
            }
            if !self.interface_variable_exists_in_entry_point(var.self_)? {
                continue;
            }

            // Also, make sure we preserve output variables which are only initialized, but never accessed by any code.
            if var.initializer != 0 {
                let s = var.self_;
                ActiveBuiltinHandler::add_if_builtin_or_block(self, s)?;
            }
        }
        Ok(())
    }

    /// `has_active_builtin()`: returns whether this shader uses a builtin of
    /// the storage class.
    pub fn has_active_builtin(&self, builtin: BuiltIn, storage: StorageClass) -> bool {
        let flags = match storage {
            StorageClassInput => &self.active_input_builtins,
            StorageClassOutput => &self.active_output_builtins,
            _ => return false,
        };
        flags.get(builtin)
    }

    pub(crate) fn analyze_image_and_sampler_usage(&mut self) -> Result<()> {
        let mut dref_handler = CombinedImageSamplerDrefHandler {
            dref_combined_samplers: HashSet::new(),
        };
        let entry = self.ir.default_entry_point;
        self.traverse_all_reachable_opcodes_func(entry, &mut dref_handler)?;

        let mut handler = CombinedImageSamplerUsageHandler {
            dref_combined_samplers: dref_handler.dref_combined_samplers,
            dependency_hierarchy: HashMap::new(),
            comparison_ids: HashSet::new(),
            need_subpass_input: false,
            need_subpass_input_ms: false,
        };
        self.traverse_all_reachable_opcodes_func(entry, &mut handler)?;

        // Need to run this traversal twice. First time, we propagate any comparison sampler usage from leaf functions
        // down to main().
        // In the second pass, we can propagate up forced depth state coming from main() up into leaf functions.
        handler.dependency_hierarchy.clear();
        self.traverse_all_reachable_opcodes_func(entry, &mut handler)?;

        self.comparison_ids = std::mem::take(&mut handler.comparison_ids);
        self.need_subpass_input = handler.need_subpass_input;
        self.need_subpass_input_ms = handler.need_subpass_input_ms;

        // Forward information from separate images and samplers into combined image samplers.
        for combined in self.combined_image_samplers.clone() {
            if self.comparison_ids.contains(&combined.sampler_id) {
                self.comparison_ids.insert(combined.combined_id);
            }
        }
        Ok(())
    }
}

/// `CombinedImageSamplerDrefHandler`.
struct CombinedImageSamplerDrefHandler {
    dref_combined_samplers: HashSet<u32>,
}

impl OpcodeHandler for CombinedImageSamplerDrefHandler {
    fn handle(
        &mut self,
        _compiler: &mut Compiler,
        opcode: Op,
        args: &[u32],
        _length: u32,
    ) -> Result<bool> {
        // Mark all sampled images which are used with Dref.
        match opcode {
            OpImageSampleDrefExplicitLod
            | OpImageSampleDrefImplicitLod
            | OpImageSampleProjDrefExplicitLod
            | OpImageSampleProjDrefImplicitLod
            | OpImageSparseSampleProjDrefImplicitLod
            | OpImageSparseSampleDrefImplicitLod
            | OpImageSparseSampleProjDrefExplicitLod
            | OpImageSparseSampleDrefExplicitLod
            | OpImageDrefGather
            | OpImageSparseDrefGather => {
                self.dref_combined_samplers.insert(args[2]);
                return Ok(true);
            }

            _ => {}
        }

        Ok(true)
    }
}

impl Compiler {
    pub(crate) fn get_cfg_for_current_function(&self) -> Result<Rc<CFG>> {
        // assert(current_function);
        let id = match self.current_function {
            Some(f) => self.get::<SPIRFunction>(f)?.self_,
            None => spirv_cross_throw!("nullptr"),
        };
        self.get_cfg_for_function(id)
    }

    pub(crate) fn get_cfg_for_function(&self, id: u32) -> Result<Rc<CFG>> {
        match self.function_cfgs.get(&id) {
            // assert(cfg_itr != end(function_cfgs));
            // assert(cfg_itr->second);
            None => spirv_cross_throw!("No CFG for function."),
            Some(cfg) => Ok(cfg.clone()),
        }
    }

    pub(crate) fn build_function_control_flow_graphs_and_analyze(&mut self) -> Result<()> {
        let mut handler = CFGBuilder {
            function_cfgs: StdHashMap::new(),
        };
        let entry = self.ir.default_entry_point;
        let cfg = CFG::new(self, entry)?;
        handler.function_cfgs.insert(entry, Rc::new(cfg));
        self.traverse_all_reachable_opcodes_func(entry, &mut handler)?;
        self.function_cfgs = std::mem::take(&mut handler.function_cfgs);
        let single_function = self.function_cfgs.len() <= 1;

        for f in self.function_cfgs.keys() {
            let func = self.get::<SPIRFunction>(f)?.self_;
            let mut scope_handler = AnalyzeVariableScopeAccessHandler::new(func);
            self.analyze_variable_scope(f, &mut scope_handler)?;
            self.find_function_local_luts(f, &scope_handler, single_function)?;

            // Check if we can actually use the loop variables we found in analyze_variable_scope.
            // To use multiple initializers, we need the same type and qualifiers.
            for block in self.get::<SPIRFunction>(f)?.blocks.clone() {
                let b = self.get::<SPIRBlock>(block)?;
                if b.loop_variables.len() < 2 {
                    continue;
                }

                let flags = self.get_decoration_bitset(b.loop_variables[0]).clone();
                let type_ = self.get::<SPIRVariable>(b.loop_variables[0])?.basetype;
                let mut invalid_initializers = false;
                for &loop_variable in &b.loop_variables {
                    if flags != *self.get_decoration_bitset(loop_variable)
                        || type_ != self.get::<SPIRVariable>(b.loop_variables[0])?.basetype
                    {
                        invalid_initializers = true;
                        break;
                    }
                }

                if invalid_initializers {
                    for loop_variable in b.loop_variables.clone() {
                        self.get_mut::<SPIRVariable>(loop_variable)?.loop_variable = false;
                    }
                    self.get_mut::<SPIRBlock>(block)?.loop_variables.clear();
                }
            }
        }

        // Find LUTs which are not function local. Only consider this case if the CFG is multi-function,
        // otherwise we treat Private as Function trivially.
        // Needs to be analyzed from the outside since we have to block the LUT optimization if at least
        // one function writes to it.
        if !single_function {
            for id in self.global_variables.clone() {
                let var = self.get::<SPIRVariable>(id)?;
                let type_ = self.get_variable_data_type(var)?;

                if self.is_array(type_)
                    && var.storage == StorageClassPrivate
                    && var.initializer != 0
                    && !var.is_written_to
                    && self.id_type(var.initializer) == TypeConstant
                {
                    let init = var.initializer;
                    self.get_mut::<SPIRConstant>(init)?.is_used_as_lut = true;
                    let var = self.get_mut::<SPIRVariable>(id)?;
                    var.static_expression = init;
                    var.statically_assigned = true;
                    var.remapped_variable = true;
                }
            }
        }
        Ok(())
    }
}

/// `CFGBuilder`.
struct CFGBuilder {
    function_cfgs: StdHashMap<Rc<CFG>>,
}

impl OpcodeHandler for CFGBuilder {
    fn handle(
        &mut self,
        _compiler: &mut Compiler,
        _op: Op,
        _args: &[u32],
        _length: u32,
    ) -> Result<bool> {
        Ok(true)
    }

    fn follow_function_call(
        &mut self,
        compiler: &mut Compiler,
        func: &SPIRFunction,
    ) -> Result<bool> {
        if !self.function_cfgs.contains_key(&func.self_) {
            let cfg = CFG::new(compiler, func.self_)?;
            self.function_cfgs.insert(func.self_, Rc::new(cfg));
            Ok(true)
        } else {
            Ok(false)
        }
    }
}

/// `CombinedImageSamplerUsageHandler`.
struct CombinedImageSamplerUsageHandler {
    dref_combined_samplers: HashSet<u32>,

    dependency_hierarchy: HashMap<u32, HashSet<u32>>,
    comparison_ids: HashSet<u32>,

    need_subpass_input: bool,
    need_subpass_input_ms: bool,
}

impl CombinedImageSamplerUsageHandler {
    fn add_dependency(&mut self, dst: u32, src: u32) {
        self.dependency_hierarchy
            .entry(dst)
            .or_default()
            .insert(src);
        // Propagate up any comparison state if we're loading from one such variable.
        if self.comparison_ids.contains(&src) {
            self.comparison_ids.insert(dst);
        }
    }

    fn add_hierarchy_to_comparison_ids(&mut self, id: u32, depth: u32) -> Result<()> {
        // (A cycle in the hierarchy recurses forever in C++.)
        if depth > 4096 {
            spirv_cross_throw!("Dependency recursion too deep.");
        }
        // Traverse the variable dependency hierarchy and tag everything in its path with comparison ids.
        self.comparison_ids.insert(id);

        let deps: Vec<u32> = self
            .dependency_hierarchy
            .entry(id)
            .or_default()
            .iter()
            .copied()
            .collect();
        for dep_id in deps {
            self.add_hierarchy_to_comparison_ids(dep_id, depth + 1)?;
        }
        Ok(())
    }
}

impl OpcodeHandler for CombinedImageSamplerUsageHandler {
    fn begin_function_scope(
        &mut self,
        compiler: &mut Compiler,
        args: &[u32],
        length: u32,
    ) -> Result<bool> {
        if length < 3 {
            return Ok(false);
        }

        let func = compiler.get_rc::<SPIRFunction>(args[2])?;
        let arg = &args[3..];
        let length = length - 3;

        for i in 0..length as usize {
            let argument = func.arguments.at(i)?;
            self.add_dependency(argument.id, arg[i]);
        }

        Ok(true)
    }

    fn handle(
        &mut self,
        compiler: &mut Compiler,
        opcode: Op,
        args: &[u32],
        length: u32,
    ) -> Result<bool> {
        match opcode {
            OpAccessChain | OpInBoundsAccessChain | OpPtrAccessChain | OpLoad => {
                if length < 3 {
                    return Ok(false);
                }

                self.add_dependency(args[1], args[2]);

                // Ideally defer this to OpImageRead, but then we'd need to track loaded IDs.
                // If we load an image, we're going to use it and there is little harm in declaring an unused gl_FragCoord.
                let type_ = compiler.get::<SPIRType>(args[0])?;
                if type_.image.dim == DimSubpassData {
                    self.need_subpass_input = true;
                    if type_.image.ms {
                        self.need_subpass_input_ms = true;
                    }
                }

                // If we load a SampledImage and it will be used with Dref, propagate the state up.
                if self.dref_combined_samplers.contains(&args[1]) {
                    self.add_hierarchy_to_comparison_ids(args[1], 0)?;
                }
            }

            OpSampledImage => {
                if length < 4 {
                    return Ok(false);
                }

                // If the underlying resource has been used for comparison then duplicate loads of that resource must be too.
                // This image must be a depth image.
                let result_id = args[1];
                let image = args[2];
                let sampler = args[3];

                if self.dref_combined_samplers.contains(&result_id) {
                    self.add_hierarchy_to_comparison_ids(image, 0)?;

                    // This sampler must be a SamplerComparisonState, and not a regular SamplerState.
                    self.add_hierarchy_to_comparison_ids(sampler, 0)?;

                    // Mark the OpSampledImage itself as being comparison state.
                    self.comparison_ids.insert(result_id);
                }
                return Ok(true);
            }

            _ => {}
        }

        Ok(true)
    }
}

impl Compiler {
    /// `buffer_is_hlsl_counter_buffer()`.
    pub fn buffer_is_hlsl_counter_buffer(&self, id: VariableID) -> bool {
        self.ir
            .find_meta(id)
            .map(|m| m.hlsl_is_magic_counter_buffer)
            .unwrap_or(false)
    }

    /// `buffer_get_hlsl_counter_buffer()`.
    pub fn buffer_get_hlsl_counter_buffer(&self, id: VariableID) -> Option<u32> {
        let m = self.ir.find_meta(id)?;

        // First, check for the proper decoration.
        if m.hlsl_magic_counter_buffer != 0 {
            Some(m.hlsl_magic_counter_buffer)
        } else {
            None
        }
    }

    pub(crate) fn make_constant_null(&mut self, id: u32, type_: u32) -> Result<()> {
        let constant_type = self.get::<SPIRType>(type_)?.clone();

        if constant_type.pointer {
            let constant = self.set(id, SPIRConstant::new_typed(type_))?;
            constant.make_null(&constant_type);
        } else if !constant_type.array.is_empty() {
            // assert(constant_type.parent_type);
            let parent_id = self.ir.increase_bound_by(1)?;
            self.make_constant_null(parent_id, constant_type.parent_type)?;

            // The array size of OpConstantNull can be either literal or specialization constant.
            // In the latter case, we cannot take the value as-is, as it can be changed to anything.
            // Rather, we assume it to be *one* for the sake of initializer.
            let is_literal_array_size = *constant_type.array_size_literal.back()?;
            let count = if is_literal_array_size {
                *constant_type.array.back()?
            } else {
                1
            };
            let mut elements: Vec<u32> = Vec::new();
            elements
                .try_reserve(count as usize)
                .map_err(|_| CompilerError::new("std::bad_alloc"))?;
            elements.resize(count as usize, parent_id);
            let constant = self.set(
                id,
                SPIRConstant::new_composite(type_, &elements, false, false),
            )?;
            constant.is_null_array_specialized_length = !is_literal_array_size;
        } else if !constant_type.member_types.is_empty() {
            let member_ids = self
                .ir
                .increase_bound_by(constant_type.member_types.len() as u32)?;
            let mut elements = vec![0u32; constant_type.member_types.len()];
            for i in 0..constant_type.member_types.len() {
                self.make_constant_null(member_ids + i as u32, constant_type.member_types[i])?;
                elements[i] = member_ids + i as u32;
            }
            self.set(
                id,
                SPIRConstant::new_composite(type_, &elements, false, false),
            )?;
        } else {
            let constant = self.set(id, SPIRConstant::new_typed(type_))?;
            constant.make_null(&constant_type);
        }
        Ok(())
    }

    /// `get_declared_capabilities()`.
    pub fn get_declared_capabilities(&self) -> &Vec<Capability> {
        &self.ir.declared_capabilities
    }

    /// `get_declared_extensions()`.
    pub fn get_declared_extensions(&self) -> &Vec<String> {
        &self.ir.declared_extensions
    }

    /// `get_remapped_declared_block_name()`.
    pub fn get_remapped_declared_block_name(&self, id: VariableID) -> Result<String> {
        self.get_remapped_declared_block_name_fallback(id, false)
    }

    pub(crate) fn get_remapped_declared_block_name_fallback(
        &self,
        id: u32,
        fallback_prefer_instance_name: bool,
    ) -> Result<String> {
        if let Some(name) = self.declared_block_names.get(&id) {
            Ok(name.clone())
        } else {
            let var = self.get::<SPIRVariable>(id)?;

            if fallback_prefer_instance_name {
                self.to_name(var.self_, true)
            } else {
                let type_ = self.get::<SPIRType>(var.basetype)?;
                let type_meta = self.ir.find_meta(type_.self_);
                let block_name = type_meta.map(|m| &m.decoration.alias);
                match block_name {
                    Some(b) if !b.is_empty() => Ok(b.clone()),
                    _ => self.get_block_fallback_name(id),
                }
            }
        }
    }

    pub(crate) fn reflection_ssbo_instance_name_is_significant(&self) -> Result<bool> {
        if let Some(s) = self.ir.sources.first() {
            if s.known {
                // UAVs from HLSL source tend to be declared in a way where the type is reused
                // but the instance name is significant, and that's the name we should report.
                // For GLSL, SSBOs each have their own block type as that's how GLSL is written.
                return Ok(s.hlsl);
            }
        }

        let mut ssbo_type_ids: HashSet<u32> = HashSet::new();
        let mut aliased_ssbo_types = false;

        // If we don't have any OpSource information, we need to perform some shaky heuristics.
        for id in self.ir.typed_ids(TypeVariable) {
            let var = self.get::<SPIRVariable>(id)?;
            let type_ = self.get::<SPIRType>(var.basetype)?;
            if !type_.pointer || var.storage == StorageClassFunction {
                continue;
            }

            let ssbo = var.storage == StorageClassStorageBuffer
                || (var.storage == StorageClassUniform
                    && self.has_decoration(type_.self_, DecorationBufferBlock));

            if ssbo {
                if ssbo_type_ids.contains(&type_.self_) {
                    aliased_ssbo_types = true;
                } else {
                    ssbo_type_ids.insert(type_.self_);
                }
            }
        }

        // If the block name is aliased, assume we have HLSL-style UAV declarations.
        Ok(aliased_ssbo_types)
    }

    /// `instruction_to_result_type()`: the result type and id.
    pub(crate) fn instruction_to_result_type(
        op: Op,
        args: &[u32],
        length: u32,
    ) -> Option<(u32, u32)> {
        if length < 2 {
            return None;
        }

        let (has_result_id, has_result_type) = has_result_and_type(op);
        if has_result_id && has_result_type {
            Some((args[0], args[1]))
        } else {
            None
        }
    }

    pub(crate) fn combined_decoration_for_member(
        &self,
        type_: &SPIRType,
        index: u32,
    ) -> Result<Bitset> {
        self.combined_decoration_for_member_inner(type_, index, 0)
    }

    fn combined_decoration_for_member_inner(
        &self,
        type_: &SPIRType,
        index: u32,
        depth: u32,
    ) -> Result<Bitset> {
        if depth > 1024 {
            spirv_cross_throw!("Type recursion too deep.");
        }
        let mut flags = Bitset::default();

        if let Some(type_meta) = self.ir.find_meta(type_.self_) {
            let members = &type_meta.members;
            if index as usize >= members.len() {
                return Ok(flags);
            }
            let dec = &members[index as usize];

            flags.merge_or(&dec.decoration_flags);

            let member_type = self.get::<SPIRType>(*type_.member_types.at(index)?)?;

            // If our member type is a struct, traverse all the child members as well recursively.
            let member_childs = &member_type.member_types;
            for i in 0..member_childs.len() {
                let child_member_type = self.get::<SPIRType>(member_childs[i])?;
                if !child_member_type.pointer {
                    flags.merge_or(&self.combined_decoration_for_member_inner(
                        member_type,
                        i as u32,
                        depth + 1,
                    )?);
                }
            }
        }

        Ok(flags)
    }

    pub(crate) fn is_desktop_only_format(format: ImageFormat) -> bool {
        matches!(
            format,
            // Desktop-only formats
            ImageFormatR11fG11fB10f
                | ImageFormatR16f
                | ImageFormatRgb10A2
                | ImageFormatR8
                | ImageFormatRg8
                | ImageFormatR16
                | ImageFormatRg16
                | ImageFormatRgba16
                | ImageFormatR16Snorm
                | ImageFormatRg16Snorm
                | ImageFormatRgba16Snorm
                | ImageFormatR8Snorm
                | ImageFormatRg8Snorm
                | ImageFormatR8ui
                | ImageFormatRg8ui
                | ImageFormatR16ui
                | ImageFormatRgb10a2ui
                | ImageFormatR8i
                | ImageFormatRg8i
                | ImageFormatR16i
        )
    }

    // An image is determined to be a depth image if it is marked as a depth image and is not also
    // explicitly marked with a color format, or if there are any sample/gather compare operations on it.
    pub(crate) fn is_depth_image(&self, type_: &SPIRType, id: u32) -> bool {
        (type_.image.depth && type_.image.format == ImageFormatUnknown)
            || self.comparison_ids.contains(&id)
    }

    pub(crate) fn type_is_opaque_value(&self, type_: &SPIRType) -> bool {
        !type_.pointer
            && (type_.basetype == BaseType::SampledImage
                || type_.basetype == BaseType::Image
                || type_.basetype == BaseType::Sampler
                || type_.basetype == BaseType::Tensor)
    }

    // Make these member functions so we can easily break on any force_recompile events.
    pub(crate) fn force_recompile(&mut self) {
        self.is_force_recompile = true;
    }

    pub(crate) fn force_recompile_guarantee_forward_progress(&mut self) {
        self.force_recompile();
        self.is_force_recompile_forward_progress = true;
    }

    pub(crate) fn is_forcing_recompilation(&self) -> bool {
        self.is_force_recompile
    }

    pub(crate) fn clear_force_recompile(&mut self) {
        self.is_force_recompile = false;
        self.is_force_recompile_forward_progress = false;
    }

    #[inline]
    pub(crate) fn is_continue(&self, next: u32) -> Result<bool> {
        Ok((*self.ir.block_meta.at(next)? & BLOCK_META_CONTINUE_BIT) != 0)
    }

    #[inline]
    pub(crate) fn is_single_block_loop(&self, next: u32) -> Result<bool> {
        let block = self.get::<SPIRBlock>(next)?;
        Ok(block.merge == Merge::MergeLoop && block.continue_block == next)
    }

    #[inline]
    pub(crate) fn is_break(&self, next: u32) -> Result<bool> {
        Ok((*self.ir.block_meta.at(next)?
            & (BLOCK_META_LOOP_MERGE_BIT | BLOCK_META_MULTISELECT_MERGE_BIT))
            != 0)
    }

    #[inline]
    pub(crate) fn is_loop_break(&self, next: u32) -> Result<bool> {
        Ok((*self.ir.block_meta.at(next)? & BLOCK_META_LOOP_MERGE_BIT) != 0)
    }

    #[inline]
    pub(crate) fn is_conditional(&self, next: u32) -> Result<bool> {
        Ok((*self.ir.block_meta.at(next)?
            & (BLOCK_META_SELECTION_MERGE_BIT | BLOCK_META_MULTISELECT_MERGE_BIT))
            != 0)
    }

    /// `is_position_invariant()`.
    pub fn is_position_invariant(&self) -> bool {
        self.position_invariant
    }

    /// `get_ir()`.
    pub fn get_ir(&self) -> &ParsedIR {
        &self.ir
    }
}

/// `PhysicalStorageBufferPointerHandler`.
struct PhysicalStorageBufferPointerHandler {
    non_block_types: HashSet<u32>,
    physical_block_type_meta: HashMap<u32, PhysicalBlockMeta>,
    // C++ maps to pointers into physical_block_type_meta; here, its keys.
    access_chain_to_physical_block: HashMap<u32, u32>,
    analyzed_type_ids: HashSet<u32>,
}

impl PhysicalStorageBufferPointerHandler {
    fn find_block_meta(&mut self, id: u32) -> Option<&mut PhysicalBlockMeta> {
        let key = *self.access_chain_to_physical_block.get(&id)?;
        self.physical_block_type_meta.get_mut(&key)
    }

    fn mark_aligned_access(&mut self, id: u32, args: &[u32], mut length: u32) {
        let mask = args[0];
        let mut args = &args[1..];
        length -= 1;
        if length != 0 && (mask & MemoryAccessVolatileMask) != 0 {
            args = &args[1..];
            length -= 1;
        }

        if length != 0 && (mask & MemoryAccessAlignedMask) != 0 {
            let alignment = args[0];
            let meta = self.find_block_meta(id);

            // This makes the assumption that the application does not rely on insane edge cases like:
            // Bind buffer with ADDR = 8, use block offset of 8 bytes, load/store with 16 byte alignment.
            // If we emit the buffer with alignment = 16 here, the first element at offset = 0 should
            // actually have alignment of 8 bytes, but this is too theoretical and awkward to support.
            // We could potentially keep track of any offset in the access chain, but it's
            // practically impossible for high level compilers to emit code like that,
            // so deducing overall alignment requirement based on maximum observed Alignment value is probably fine.
            if let Some(meta) = meta {
                if alignment > meta.alignment {
                    meta.alignment = alignment;
                }
            }
        }
    }

    fn type_is_bda_block_entry(&self, compiler: &Compiler, type_id: u32) -> Result<bool> {
        let type_ = compiler.get::<SPIRType>(type_id)?;
        Ok(compiler.is_physical_pointer(type_))
    }

    fn get_minimum_scalar_alignment(
        &self,
        compiler: &Compiler,
        type_: &SPIRType,
        depth: u32,
    ) -> Result<u32> {
        if depth > 1024 {
            spirv_cross_throw!("Type recursion too deep.");
        }
        if type_.storage == StorageClassPhysicalStorageBuffer {
            Ok(8)
        } else if type_.basetype == BaseType::Struct {
            let mut alignment = 0;
            for &member_type in &type_.member_types {
                let member_align = self.get_minimum_scalar_alignment(
                    compiler,
                    compiler.get::<SPIRType>(member_type)?,
                    depth + 1,
                )?;
                if member_align > alignment {
                    alignment = member_align;
                }
            }
            Ok(alignment)
        } else {
            Ok(type_.width / 8)
        }
    }

    fn setup_meta_chain(&mut self, compiler: &Compiler, type_id: u32, var_id: u32) -> Result<()> {
        if self.type_is_bda_block_entry(compiler, type_id)? {
            self.physical_block_type_meta.entry(type_id).or_default();
            self.access_chain_to_physical_block.insert(var_id, type_id);

            let type_ = compiler.get::<SPIRType>(type_id)?;

            if !compiler.is_physical_pointer_to_buffer_block(type_)? {
                self.non_block_types.insert(type_id);
            }

            if self
                .physical_block_type_meta
                .get(&type_id)
                .map(|m| m.alignment)
                .unwrap_or(0)
                == 0
            {
                let a = self.get_minimum_scalar_alignment(
                    compiler,
                    compiler.get_pointee_type(type_)?,
                    0,
                )?;
                if let Some(meta) = self.physical_block_type_meta.get_mut(&type_id) {
                    meta.alignment = a;
                }
            }
        }
        Ok(())
    }

    fn get_base_non_block_type_id(&self, compiler: &Compiler, mut type_id: u32) -> Result<u32> {
        let mut type_ = compiler.get::<SPIRType>(type_id)?;
        let mut steps: usize = 0;
        while compiler.is_physical_pointer(type_)
            && !self.type_is_bda_block_entry(compiler, type_id)?
        {
            type_id = type_.parent_type;
            type_ = compiler.get::<SPIRType>(type_id)?;
            steps += 1;
            if steps > compiler.ir.ids.len() {
                break;
            }
        }

        // assert(type_is_bda_block_entry(type_id));
        Ok(type_id)
    }

    fn analyze_non_block_types_from_block(
        &mut self,
        compiler: &Compiler,
        type_: &SPIRType,
    ) -> Result<()> {
        if self.analyzed_type_ids.contains(&type_.self_) {
            return Ok(());
        }
        self.analyzed_type_ids.insert(type_.self_);

        for &member in &type_.member_types {
            let subtype = compiler.get::<SPIRType>(member)?;

            if compiler.is_physical_pointer(subtype)
                && !compiler.is_physical_pointer_to_buffer_block(subtype)?
            {
                let t = self.get_base_non_block_type_id(compiler, member)?;
                self.non_block_types.insert(t);
            } else if subtype.basetype == BaseType::Struct && !compiler.is_pointer(subtype) {
                self.analyze_non_block_types_from_block(compiler, subtype)?;
            }
        }
        Ok(())
    }
}

impl OpcodeHandler for PhysicalStorageBufferPointerHandler {
    fn handle(
        &mut self,
        compiler: &mut Compiler,
        op: Op,
        args: &[u32],
        length: u32,
    ) -> Result<bool> {
        // When a BDA pointer comes to life, we need to keep a mapping of SSA ID -> type ID for the pointer type.
        // For every load and store, we'll need to be able to look up the type ID being accessed and mark any alignment
        // requirements.
        match op {
            OpConvertUToPtr | OpBitcast | OpCompositeExtract => {
                // Extract can begin a new chain if we had a struct or array of pointers as input.
                // We don't begin chains before we have a pure scalar pointer.
                self.setup_meta_chain(compiler, args[0], args[1])?;
            }

            OpAccessChain | OpInBoundsAccessChain | OpPtrAccessChain | OpCopyObject => {
                if let Some(&t) = self.access_chain_to_physical_block.get(&args[2]) {
                    self.access_chain_to_physical_block.insert(args[1], t);
                }
            }

            OpLoad => {
                self.setup_meta_chain(compiler, args[0], args[1])?;
                if length >= 4 {
                    self.mark_aligned_access(args[2], &args[3..], length - 3);
                }
            }

            OpStore => {
                if length >= 3 {
                    self.mark_aligned_access(args[0], &args[2..], length - 2);
                }
            }

            OpCooperativeMatrixLoadKHR | OpCooperativeMatrixStoreKHR => {
                // TODO: Can we meaningfully deal with this?
            }

            _ => {}
        }

        Ok(true)
    }
}

impl Compiler {
    pub(crate) fn analyze_non_block_pointer_types(&mut self) -> Result<()> {
        let mut handler = PhysicalStorageBufferPointerHandler {
            non_block_types: HashSet::new(),
            physical_block_type_meta: HashMap::new(),
            access_chain_to_physical_block: HashMap::new(),
            analyzed_type_ids: HashSet::new(),
        };
        let entry = self.ir.default_entry_point;
        self.traverse_all_reachable_opcodes_func(entry, &mut handler)?;

        // Analyze any block declaration we have to make. It might contain
        // physical pointers to POD types which we never used, and thus never added to the list.
        // We'll need to add those pointer types to the set of types we declare.
        for id in self.ir.typed_ids(TypeType) {
            let type_ = self.get::<SPIRType>(id)?;
            // Only analyze the raw block struct, not any pointer-to-struct, since that's just redundant.
            if type_.self_ == id
                && (self.has_decoration(type_.self_, DecorationBlock)
                    || self.has_decoration(type_.self_, DecorationBufferBlock))
            {
                handler.analyze_non_block_types_from_block(self, type_)?;
            }
        }

        self.physical_storage_non_block_pointer_types
            .reserve(handler.non_block_types.len());
        for &type_ in &handler.non_block_types {
            self.physical_storage_non_block_pointer_types.push(type_);
        }
        self.physical_storage_non_block_pointer_types.sort();
        self.physical_storage_type_to_alignment = handler.physical_block_type_meta;
        Ok(())
    }
}

/// The `HeapHandler` of `analyze_descriptor_heap_types()`.
struct HeapHandler {
    heap_types: Vec<DescriptorHeapMeta>,
    buffer_pointers: HashMap<u32, (TypeID, bool)>,
    hlsl_style_stride_access_chains: HashSet<u32>,
}

impl HeapHandler {
    fn add_unique_type(&mut self, meta: DescriptorHeapMeta) {
        // assert(meta.type != 0);

        for type_ in &self.heap_types {
            if type_.type_ == meta.type_
                && type_.storage == meta.storage
                && type_.buffer_pointer_id == meta.buffer_pointer_id
                && type_.nonreadable == meta.nonreadable
                && type_.nonwritable == meta.nonwritable
                && type_.coherent == meta.coherent
                && type_.is_restrict == meta.is_restrict
                && type_.hlsl_style_stride == meta.hlsl_style_stride
                && type_.is_volatile == meta.is_volatile
            {
                return;
            }
        }

        self.heap_types.push(meta);
    }
}

impl OpcodeHandler for HeapHandler {
    fn handle(
        &mut self,
        compiler: &mut Compiler,
        opcode: Op,
        args: &[u32],
        _length: u32,
    ) -> Result<bool> {
        match opcode {
            OpBufferPointerEXT => {
                let ptr_type = compiler.get::<SPIRType>(args[0])?;
                // BufferPointerEXT can return untyped or typed pointers.
                // If it's typed, we resolve it here.
                if ptr_type.basetype == BaseType::Struct {
                    let meta = DescriptorHeapMeta {
                        type_: ptr_type.self_,
                        hlsl_style_stride: self.hlsl_style_stride_access_chains.contains(&args[2]),
                        buffer_pointer_id: args[1],
                        storage: ptr_type.storage,
                        nonreadable: compiler.has_decoration(args[1], DecorationNonReadable),
                        nonwritable: compiler.has_decoration(args[1], DecorationNonWritable),
                        coherent: compiler.has_decoration(args[1], DecorationCoherent),
                        is_restrict: compiler.has_decoration(args[1], DecorationRestrict),
                        is_volatile: compiler.has_decoration(args[1], DecorationVolatile),
                    };
                    self.add_unique_type(meta);
                }
                let hlsl = self.hlsl_style_stride_access_chains.contains(&args[2]);
                self.buffer_pointers.insert(args[1], (args[0], hlsl));
            }

            OpUntypedAccessChainKHR | OpUntypedInBoundsAccessChainKHR | OpUntypedArrayLengthKHR => {
                let data_type = compiler.get::<SPIRType>(args[2])?;

                if compiler.is_pointer(data_type) {
                    spirv_cross_throw!("pointer type not allowed.");
                }

                let mut hlsl_style_stride = false;

                // Need to validate the array stride and types. HLLs are not flexible enough to support the full flexibility of SPIR-V.
                if compiler.get_decoration(args[3], DecorationBuiltIn) == BuiltInResourceHeapEXT {
                    if !Compiler::is_runtime_size_array(data_type) {
                        spirv_cross_throw!("Descriptor heap must be accessed as a runtime array.");
                    }

                    // The only meaningful use of this is ArrayStride equal to sizeof(type) right now.
                    let array_stride_id =
                        compiler.get_decoration(args[2], DecorationArrayStrideIdEXT);
                    if array_stride_id == 0 {
                        spirv_cross_throw!(
                            "Expected ArrayStrideIdEXT to be set for resource heap."
                        );
                    }

                    let spec_c = compiler.maybe_get::<SPIRConstantOp>(array_stride_id);
                    let c = compiler.maybe_get::<SPIRConstant>(array_stride_id);

                    if spec_c.is_none() && c.is_none() {
                        spirv_cross_throw!("Array stride must be some constant expression.");
                    }

                    if let Some(spec_c) = spec_c {
                        // This gets potentially infinitely weird, but if we get HLSL-style shaders
                        // we expect the array stride to be max(buffer, image) since all descriptors have equal size in D3D12.
                        // We just have to be a bit loose here since it's impossible to anticipate every theoretical formulation.
                        // Anything non-conforming to strict GLSL is flagged in the codegen output.
                        if spec_c.opcode == OpSelect {
                            let true_value =
                                compiler.maybe_get::<SPIRConstant>(*spec_c.arguments.at(1)?);
                            let false_value =
                                compiler.maybe_get::<SPIRConstant>(*spec_c.arguments.at(2)?);
                            hlsl_style_stride =
                                true_value.map(|v| v.size_of_type != 0).unwrap_or(false)
                                    && false_value.map(|v| v.size_of_type != 0).unwrap_or(false);
                        }

                        if !hlsl_style_stride {
                            spirv_cross_throw!(
                                "Unusual pattern of descriptor stride detected. This probably cannot be expressed in current GLSL."
                            );
                        }
                    }

                    if let Some(c) = c {
                        if c.size_of_type == 0 {
                            spirv_cross_throw!("Resource heap array stride must be ConstantSizeOfEXT for high level languages.");
                        }
                    }

                    let element_type = compiler.get::<SPIRType>(data_type.parent_type)?;

                    if element_type.basetype == BaseType::DescriptorHeapBuffer {
                        if let Some(c) = c {
                            if compiler.get::<SPIRType>(c.size_of_type)?.basetype
                                != BaseType::DescriptorHeapBuffer
                            {
                                spirv_cross_throw!(
                                    "Buffer descriptors in heap must be ConstantSizeOfEXT(OpTypeBufferEXT) for GLSL."
                                );
                            }
                        }
                    } else if data_type.basetype == BaseType::Image {
                        if let Some(c) = c {
                            if compiler.get::<SPIRType>(c.size_of_type)?.basetype != BaseType::Image
                            {
                                spirv_cross_throw!("Image descriptors in heap must be ConstantSizeOfEXT(OpTypeImage) for GLSL.");
                            }
                        }
                    } else if data_type.basetype == BaseType::AccelerationStructure {
                        if let Some(c) = c {
                            if compiler.get::<SPIRType>(c.size_of_type)?.basetype
                                != BaseType::AccelerationStructure
                            {
                                spirv_cross_throw!(
                                    "RTAS descriptors in heap must be ConstantSizeOfEXT(OpTypeAccelerationStructure) for GLSL."
                                );
                            }
                        }
                    }
                } else if compiler.get_decoration(args[3], DecorationBuiltIn)
                    == BuiltInSamplerHeapEXT
                {
                    if !Compiler::is_runtime_size_array(data_type) {
                        spirv_cross_throw!("Descriptor heap must be accessed as a runtime array.");
                    }

                    // The only meaningful use of this is ArrayStride equal to sizeof(sampler) right now.
                    let array_stride_id =
                        compiler.get_decoration(args[2], DecorationArrayStrideIdEXT);
                    if array_stride_id == 0 {
                        spirv_cross_throw!("Expected ArrayStrideIdEXT to be set for sampler heap.");
                    }

                    let c = compiler.maybe_get::<SPIRConstant>(array_stride_id);
                    let ok = match c {
                        Some(c) => {
                            c.size_of_type != 0
                                && compiler.get::<SPIRType>(c.size_of_type)?.basetype
                                    == BaseType::Sampler
                        }
                        None => false,
                    };
                    if !ok {
                        spirv_cross_throw!(
                            "Sampler heap array stride must be ConstantSizeOfEXT(OpTypeSampler) for high level languages."
                        );
                    }
                }

                // Remember this for OpBufferPointerEXT.
                if hlsl_style_stride {
                    self.hlsl_style_stride_access_chains.insert(args[1]);
                }

                if data_type.basetype == BaseType::SampledImage {
                    spirv_cross_throw!("Attempting to access heap as combined sampler image. This does not make sense.");
                } else if data_type.basetype == BaseType::Image
                    || data_type.basetype == BaseType::AccelerationStructure
                    || data_type.basetype == BaseType::Sampler
                {
                    let meta = DescriptorHeapMeta {
                        type_: data_type.self_,
                        hlsl_style_stride,
                        ..Default::default()
                    };
                    self.add_unique_type(meta);
                } else if self.buffer_pointers.contains_key(&args[3]) {
                    if !compiler.has_decoration(data_type.self_, DecorationBlock)
                        && !compiler.has_decoration(data_type.self_, DecorationBufferBlock)
                    {
                        spirv_cross_throw!("BufferPointerEXT must reference a block type.");
                    }

                    let pointer_meta = self.buffer_pointers[&args[3]];
                    let buffer_type = compiler.get::<SPIRType>(pointer_meta.0)?;
                    if buffer_type.basetype == BaseType::Void {
                        // This is where the pointer becomes typed, so register it here.
                        let meta = DescriptorHeapMeta {
                            type_: data_type.self_,
                            hlsl_style_stride: pointer_meta.1,
                            buffer_pointer_id: args[3],
                            storage: buffer_type.storage,
                            nonreadable: compiler.has_decoration(args[3], DecorationNonReadable),
                            nonwritable: compiler.has_decoration(args[3], DecorationNonWritable),
                            coherent: compiler.has_decoration(args[3], DecorationCoherent),
                            is_volatile: compiler.has_decoration(args[3], DecorationVolatile),
                            is_restrict: compiler.has_decoration(args[3], DecorationRestrict),
                        };
                        self.add_unique_type(meta);
                    }
                }
            }

            _ => {}
        }

        Ok(true)
    }
}

impl Compiler {
    pub(crate) fn analyze_descriptor_heap_types(&mut self) -> Result<()> {
        let mut handler = HeapHandler {
            heap_types: Vec::new(),
            buffer_pointers: HashMap::new(),
            hlsl_style_stride_access_chains: HashSet::new(),
        };
        let entry = self.ir.default_entry_point;
        self.traverse_all_reachable_opcodes_func(entry, &mut handler)?;
        self.descriptor_heap_types = handler.heap_types;
        Ok(())
    }
}

/// `InterlockedResourceAccessPrepassHandler`.
struct InterlockedResourceAccessPrepassHandler {
    interlock_function_id: u32,
    current_block_id: u32,
    split_function_case: bool,
    control_flow_interlock: bool,
    call_stack: Vec<u32>,
}

impl OpcodeHandler for InterlockedResourceAccessPrepassHandler {
    fn handle(
        &mut self,
        compiler: &mut Compiler,
        op: Op,
        _args: &[u32],
        _length: u32,
    ) -> Result<bool> {
        if op == OpBeginInvocationInterlockEXT || op == OpEndInvocationInterlockEXT {
            let back = *self.call_stack.back()?;
            if self.interlock_function_id != 0 && self.interlock_function_id != back {
                // Most complex case, we have no sensible way of dealing with this
                // other than taking the 100% conservative approach, exit early.
                self.split_function_case = true;
                return Ok(false);
            } else {
                self.interlock_function_id = back;
                // If this call is performed inside control flow we have a problem.
                let cfg = compiler.get_cfg_for_function(self.interlock_function_id)?;

                let from_block_id = compiler
                    .get::<SPIRFunction>(self.interlock_function_id)?
                    .entry_block;
                let outside_control_flow = cfg.node_terminates_control_flow_in_sub_graph(
                    compiler,
                    from_block_id,
                    self.current_block_id,
                )?;
                if !outside_control_flow {
                    self.control_flow_interlock = true;
                }
            }
        }
        Ok(true)
    }

    fn rearm_current_block(&mut self, _compiler: &mut Compiler, block: &SPIRBlock) -> Result<()> {
        self.current_block_id = block.self_;
        Ok(())
    }

    fn begin_function_scope(
        &mut self,
        _compiler: &mut Compiler,
        args: &[u32],
        length: u32,
    ) -> Result<bool> {
        if length < 3 {
            return Ok(false);
        }
        self.call_stack.push(args[2]);
        Ok(true)
    }

    fn end_function_scope(
        &mut self,
        _compiler: &mut Compiler,
        _args: &[u32],
        _length: u32,
    ) -> Result<bool> {
        self.call_stack.pop();
        Ok(true)
    }
}

/// `InterlockedResourceAccessHandler`.
struct InterlockedResourceAccessHandler {
    in_crit_sec: bool,

    interlock_function_id: u32,
    split_function_case: bool,
    control_flow_interlock: bool,
    use_critical_section: bool,
    call_stack_is_interlocked: bool,
    call_stack: Vec<u32>,
}

impl InterlockedResourceAccessHandler {
    fn access_potential_resource(&mut self, compiler: &mut Compiler, id: u32) {
        if (self.use_critical_section && self.in_crit_sec)
            || (self.control_flow_interlock && self.call_stack_is_interlocked)
            || self.split_function_case
        {
            compiler.interlocked_resources.insert(id);
        }
    }
}

impl OpcodeHandler for InterlockedResourceAccessHandler {
    fn begin_function_scope(
        &mut self,
        _compiler: &mut Compiler,
        args: &[u32],
        length: u32,
    ) -> Result<bool> {
        if length < 3 {
            return Ok(false);
        }

        if args[2] == self.interlock_function_id {
            self.call_stack_is_interlocked = true;
        }

        self.call_stack.push(args[2]);
        Ok(true)
    }

    fn end_function_scope(
        &mut self,
        _compiler: &mut Compiler,
        _args: &[u32],
        _length: u32,
    ) -> Result<bool> {
        if self.call_stack.last() == Some(&self.interlock_function_id) {
            self.call_stack_is_interlocked = false;
        }

        self.call_stack.pop();
        Ok(true)
    }

    fn handle(
        &mut self,
        compiler: &mut Compiler,
        opcode: Op,
        args: &[u32],
        length: u32,
    ) -> Result<bool> {
        // Only care about critical section analysis if we have simple case.
        if self.use_critical_section {
            if opcode == OpBeginInvocationInterlockEXT {
                self.in_crit_sec = true;
                return Ok(true);
            }

            if opcode == OpEndInvocationInterlockEXT {
                // End critical section--nothing more to do.
                return Ok(false);
            }
        }

        let storage_of = |compiler: &Compiler, var: u32| -> Result<StorageClass> {
            Ok(compiler.get::<SPIRVariable>(var)?.storage)
        };
        let is_buffer_block = |compiler: &Compiler, var: u32| -> Result<bool> {
            let bt = compiler.get::<SPIRVariable>(var)?.basetype;
            Ok(compiler.has_decoration(compiler.get::<SPIRType>(bt)?.self_, DecorationBufferBlock))
        };

        // We need to figure out where images and buffers are loaded from, so do only the bare bones compilation we need.
        match opcode {
            OpLoad | OpCooperativeMatrixLoadKHR | OpCooperativeVectorLoadNV => {
                if length < 3 {
                    return Ok(false);
                }

                let ptr = args[2];

                // We're only concerned with buffer and image memory here.
                if let Some(var) = compiler.maybe_get_backing_variable(ptr) {
                    match storage_of(compiler, var)? {
                        StorageClassUniformConstant => {
                            let result_type = args[0];
                            let id = args[1];
                            compiler
                                .set(id, SPIRExpression::new(String::new(), result_type, true))?;
                            compiler.register_read(id, ptr, true)?;
                        }

                        StorageClassUniform => {
                            // Must have BufferBlock; we only care about SSBOs.
                            if is_buffer_block(compiler, var)? {
                                // fallthrough
                                self.access_potential_resource(compiler, var);
                            }
                        }
                        StorageClassStorageBuffer => self.access_potential_resource(compiler, var),

                        _ => {}
                    }
                }
            }

            OpInBoundsAccessChain | OpAccessChain | OpPtrAccessChain => {
                if length < 3 {
                    return Ok(false);
                }

                let result_type = args[0];

                let type_ = compiler.get::<SPIRType>(result_type)?;
                if type_.storage == StorageClassUniform
                    || type_.storage == StorageClassUniformConstant
                    || type_.storage == StorageClassStorageBuffer
                {
                    let id = args[1];
                    let ptr = args[2];
                    compiler.set(id, SPIRExpression::new(String::new(), result_type, true))?;
                    compiler.register_read(id, ptr, true)?;
                    compiler.ir.ids.at_mut(id)?.set_allow_type_rewrite();
                }
            }

            OpImageTexelPointer => {
                if length < 3 {
                    return Ok(false);
                }

                let result_type = args[0];
                let id = args[1];
                let ptr = args[2];
                compiler.set(id, SPIRExpression::new(String::new(), result_type, true))?;
                if let Some(var) = compiler.maybe_get_backing_variable(ptr) {
                    compiler.get_mut::<SPIRExpression>(id)?.loaded_from = var;
                }
            }

            OpStore
            | OpImageWrite
            | OpAtomicStore
            | OpCooperativeMatrixStoreKHR
            | OpCooperativeVectorStoreNV => {
                if length < 1 {
                    return Ok(false);
                }

                let ptr = args[0];
                if let Some(var) = compiler.maybe_get_backing_variable(ptr) {
                    let s = storage_of(compiler, var)?;
                    if s == StorageClassUniform
                        || s == StorageClassUniformConstant
                        || s == StorageClassStorageBuffer
                    {
                        self.access_potential_resource(compiler, var);
                    }
                }
            }

            OpCopyMemory => {
                if length < 2 {
                    return Ok(false);
                }

                let dst = args[0];
                let src = args[1];
                let dst_var = compiler.maybe_get_backing_variable(dst);
                let src_var = compiler.maybe_get_backing_variable(src);

                if let Some(dst_var) = dst_var {
                    let s = storage_of(compiler, dst_var)?;
                    if s == StorageClassUniform || s == StorageClassStorageBuffer {
                        self.access_potential_resource(compiler, dst_var);
                    }
                }

                if let Some(src_var) = src_var {
                    let s = storage_of(compiler, src_var)?;
                    if s != StorageClassUniform && s != StorageClassStorageBuffer {
                        return Ok(true);
                    }

                    if s == StorageClassUniform && !is_buffer_block(compiler, src_var)? {
                        return Ok(true);
                    }

                    self.access_potential_resource(compiler, src_var);
                }
            }

            OpImageRead | OpAtomicLoad => {
                if length < 3 {
                    return Ok(false);
                }

                let ptr = args[2];

                // We're only concerned with buffer and image memory here.
                if let Some(var) = compiler.maybe_get_backing_variable(ptr) {
                    match storage_of(compiler, var)? {
                        StorageClassUniform => {
                            // Must have BufferBlock; we only care about SSBOs.
                            if is_buffer_block(compiler, var)? {
                                // fallthrough
                                self.access_potential_resource(compiler, var);
                            }
                        }
                        StorageClassUniformConstant | StorageClassStorageBuffer => {
                            self.access_potential_resource(compiler, var);
                        }

                        _ => {}
                    }
                }
            }

            OpAtomicExchange
            | OpAtomicCompareExchange
            | OpAtomicIIncrement
            | OpAtomicIDecrement
            | OpAtomicIAdd
            | OpAtomicISub
            | OpAtomicSMin
            | OpAtomicUMin
            | OpAtomicSMax
            | OpAtomicUMax
            | OpAtomicAnd
            | OpAtomicOr
            | OpAtomicXor => {
                if length < 3 {
                    return Ok(false);
                }

                let ptr = args[2];
                if let Some(var) = compiler.maybe_get_backing_variable(ptr) {
                    let s = storage_of(compiler, var)?;
                    if s == StorageClassUniform
                        || s == StorageClassUniformConstant
                        || s == StorageClassStorageBuffer
                    {
                        self.access_potential_resource(compiler, var);
                    }
                }
            }

            _ => {}
        }

        Ok(true)
    }
}

impl Compiler {
    pub(crate) fn analyze_interlocked_resource_usage(&mut self) -> Result<()> {
        let flags = self.get_entry_point().flags.clone();
        if self.get_execution_model() == ExecutionModelFragment
            && (flags.get(ExecutionModePixelInterlockOrderedEXT)
                || flags.get(ExecutionModePixelInterlockUnorderedEXT)
                || flags.get(ExecutionModeSampleInterlockOrderedEXT)
                || flags.get(ExecutionModeSampleInterlockUnorderedEXT))
        {
            let entry = self.ir.default_entry_point;
            let mut prepass_handler = InterlockedResourceAccessPrepassHandler {
                interlock_function_id: 0,
                current_block_id: 0,
                split_function_case: false,
                control_flow_interlock: false,
                call_stack: vec![entry],
            };
            self.traverse_all_reachable_opcodes_func(entry, &mut prepass_handler)?;

            let mut handler = InterlockedResourceAccessHandler {
                in_crit_sec: false,
                interlock_function_id: prepass_handler.interlock_function_id,
                split_function_case: prepass_handler.split_function_case,
                control_flow_interlock: prepass_handler.control_flow_interlock,
                use_critical_section: false,
                call_stack_is_interlocked: false,
                call_stack: vec![entry],
            };
            handler.use_critical_section =
                !handler.split_function_case && !handler.control_flow_interlock;

            self.traverse_all_reachable_opcodes_func(entry, &mut handler)?;

            // For GLSL. If we hit any of these cases, we have to fall back to conservative approach.
            self.interlocked_is_complex =
                !handler.use_critical_section || handler.interlock_function_id != entry;
        }
        Ok(())
    }

    // Helper function
    pub(crate) fn check_internal_recursion(
        &self,
        type_: &SPIRType,
        checked_ids: &mut HashSet<u32>,
    ) -> Result<bool> {
        if type_.basetype != BaseType::Struct {
            return Ok(false);
        }

        if checked_ids.contains(&type_.self_) {
            return Ok(true);
        }

        // Recurse into struct members
        let mut is_recursive = false;
        checked_ids.insert(type_.self_);
        let mbr_cnt = type_.member_types.len();
        let mut mbr_idx = 0;
        while !is_recursive && mbr_idx < mbr_cnt {
            let mbr_type_id = type_.member_types[mbr_idx];
            let mbr_type = self.get::<SPIRType>(mbr_type_id)?;
            is_recursive |= self.check_internal_recursion(mbr_type, checked_ids)?;
            mbr_idx += 1;
        }
        checked_ids.remove(&type_.self_);
        Ok(is_recursive)
    }

    // Return whether the struct type contains a structural recursion nested somewhere within its content.
    pub(crate) fn type_contains_recursion(&self, type_: &SPIRType) -> Result<bool> {
        let mut checked_ids = HashSet::new();
        self.check_internal_recursion(type_, &mut checked_ids)
    }

    pub(crate) fn type_is_array_of_pointers(&self, type_: &SPIRType) -> Result<bool> {
        if !self.is_array(type_) {
            return Ok(false);
        }

        // BDA types must have parent type hierarchy.
        if type_.parent_type == 0 {
            return Ok(false);
        }

        // Punch through all array layers.
        let mut parent = self.get::<SPIRType>(type_.parent_type)?;
        let mut steps: usize = 0;
        while self.is_array(parent) {
            parent = self.get::<SPIRType>(parent.parent_type)?;
            steps += 1;
            if steps > self.ir.ids.len() {
                spirv_cross_throw!("Type recursion too deep.");
            }
        }

        Ok(self.is_pointer(parent))
    }

    pub(crate) fn flush_phi_required(&self, from: BlockID, to: BlockID) -> Result<bool> {
        let child = self.get::<SPIRBlock>(to)?;
        Ok(child.phi_variables.iter().any(|phi| phi.parent == from))
    }

    pub(crate) fn add_loop_level(&mut self) {
        self.current_loop_level += 1;
    }
}
