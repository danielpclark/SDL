// Part of the Rust translation of SPIRV-Cross; see LICENSE.txt.

//! CompilerHLSL (spirv_hlsl.cpp). The translation lands in a later commit.

use super::common::*;
use super::cross::*;
#[allow(unused_imports)]
use super::glsl::*;
use super::spirv::*;

#[derive(Default)]
pub struct HlslState {}

impl Compiler {
    pub(crate) fn hlsl_init(&mut self) -> Result<()> {
        Ok(())
    }
    pub(crate) fn hlsl_compile(&mut self) -> Result<String> {
        Ok(String::new())
    }
}

// Overrides not translated yet: CompilerGLSL's behaviour.
#[allow(unused_variables)]
impl Compiler {
    pub(crate) fn hlsl_emit_header(&mut self) -> Result<()> {
        self.glsl_emit_header()
    }

    pub(crate) fn hlsl_to_interpolation_qualifiers(&mut self, flags: &Bitset) -> Result<String> {
        self.glsl_to_interpolation_qualifiers(flags)
    }

    pub(crate) fn hlsl_layout_for_member(
        &mut self,
        type_: &SPIRType,
        index: u32,
    ) -> Result<String> {
        self.glsl_layout_for_member(type_, index)
    }

    pub(crate) fn hlsl_emit_push_constant_block(&mut self, var: &SPIRVariable) -> Result<()> {
        self.glsl_emit_push_constant_block(var)
    }

    pub(crate) fn hlsl_emit_buffer_block(&mut self, var: &SPIRVariable) -> Result<()> {
        self.glsl_emit_buffer_block(var)
    }

    pub(crate) fn hlsl_to_storage_qualifiers_glsl(
        &mut self,
        var: &SPIRVariable,
    ) -> Result<&'static str> {
        self.glsl_to_storage_qualifiers_glsl(var)
    }

    pub(crate) fn hlsl_emit_uniform(&mut self, var: &SPIRVariable) -> Result<()> {
        self.glsl_emit_uniform(var)
    }

    pub(crate) fn hlsl_replace_illegal_names(&mut self) -> Result<()> {
        self.glsl_replace_illegal_names()
    }

    pub(crate) fn hlsl_to_func_call_arg(&mut self, arg: &Parameter, id: u32) -> Result<String> {
        self.glsl_to_func_call_arg(arg, id)
    }

    pub(crate) fn hlsl_emit_mesh_tasks(&mut self, block: &SPIRBlock) -> Result<()> {
        self.glsl_emit_mesh_tasks(block)
    }

    pub(crate) fn hlsl_emit_complex_bitcast(
        &mut self,
        result_type: u32,
        id: u32,
        op0: u32,
    ) -> Result<bool> {
        self.glsl_emit_complex_bitcast(result_type, id, op0)
    }

    pub(crate) fn hlsl_emit_sampled_image_op(
        &mut self,
        result_type: u32,
        result_id: u32,
        image_id: u32,
        samp_id: u32,
    ) -> Result<()> {
        self.glsl_emit_sampled_image_op(result_type, result_id, image_id, samp_id)
    }

    pub(crate) fn hlsl_emit_texture_op(&mut self, i: &Instruction, sparse: bool) -> Result<()> {
        self.glsl_emit_texture_op(i, sparse)
    }

    pub(crate) fn hlsl_emit_glsl_op(
        &mut self,
        result_type: u32,
        id: u32,
        eop: u32,
        args: &[u32],
        length: u32,
    ) -> Result<()> {
        self.glsl_emit_glsl_op(result_type, id, eop, args, length)
    }

    pub(crate) fn hlsl_emit_subgroup_op(&mut self, i: &Instruction) -> Result<()> {
        self.glsl_emit_subgroup_op(i)
    }

    pub(crate) fn hlsl_bitcast_glsl_op(
        &mut self,
        out_type: &SPIRType,
        in_type: &SPIRType,
    ) -> Result<String> {
        self.glsl_bitcast_glsl_op(out_type, in_type)
    }

    pub(crate) fn hlsl_builtin_to_glsl(
        &mut self,
        builtin: BuiltIn,
        storage: StorageClass,
    ) -> Result<String> {
        self.glsl_builtin_to_glsl(builtin, storage)
    }

    pub(crate) fn hlsl_emit_instruction(&mut self, instruction: &Instruction) -> Result<()> {
        self.glsl_emit_instruction(instruction)
    }

    pub(crate) fn hlsl_append_global_func_args(
        &mut self,
        func: &SPIRFunction,
        index: u32,
        arglist: &mut Vec<String>,
    ) -> Result<()> {
        self.glsl_append_global_func_args(func, index, arglist)
    }

    pub(crate) fn hlsl_emit_struct_member(
        &mut self,
        type_: &SPIRType,
        member_type_id: u32,
        index: u32,
        qualifier: &str,
        base_offset: u32,
    ) -> Result<()> {
        self.glsl_emit_struct_member(type_, member_type_id, index, qualifier, base_offset)
    }

    pub(crate) fn hlsl_to_initializer_expression(&mut self, var: &SPIRVariable) -> Result<String> {
        self.to_unpacked_expression(var.initializer, true)
    }

    pub(crate) fn hlsl_image_type_glsl(
        &mut self,
        type_: &SPIRType,
        id: u32,
        member: bool,
    ) -> Result<String> {
        self.glsl_image_type_glsl(type_, id, member)
    }

    pub(crate) fn hlsl_type_to_glsl(&mut self, type_: &SPIRType, id: u32) -> Result<String> {
        self.glsl_type_to_glsl(type_, id)
    }

    pub(crate) fn hlsl_is_user_type_structured(&self, id: u32) -> Result<bool> {
        Ok(false) // GLSL itself does not have structured user type, but HLSL does with StructuredBuffer and RWStructuredBuffer resources.
    }

    pub(crate) fn hlsl_emit_function_prototype(
        &mut self,
        func_id: u32,
        return_flags: &Bitset,
    ) -> Result<()> {
        self.glsl_emit_function_prototype(func_id, return_flags)
    }

    pub(crate) fn hlsl_emit_fixup(&mut self) -> Result<()> {
        self.glsl_emit_fixup()
    }

    pub(crate) fn hlsl_get_builtin_basetype(
        &mut self,
        builtin: BuiltIn,
        default_type: BaseType,
    ) -> BaseType {
        self.glsl_get_builtin_basetype(builtin, default_type)
    }

    pub(crate) fn hlsl_cast_to_variable_store(
        &mut self,
        target_id: u32,
        expr: &mut String,
        expr_type: &SPIRType,
    ) -> Result<()> {
        self.glsl_cast_to_variable_store(target_id, expr, expr_type)
    }

    pub(crate) fn hlsl_emit_block_hints(&mut self, block: &SPIRBlock) -> Result<()> {
        self.glsl_emit_block_hints(block)
    }
}
