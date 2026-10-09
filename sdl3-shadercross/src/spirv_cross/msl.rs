// Part of the Rust translation of SPIRV-Cross; see LICENSE.txt.

//! CompilerMSL (spirv_msl.cpp). The translation lands in a later commit.

use super::common::*;
use super::cross::*;
use super::glsl::*;
use super::spirv::*;

#[derive(Default)]
pub struct MslState {}

impl Compiler {
    pub(crate) fn msl_init(&mut self) -> Result<()> {
        Ok(())
    }
    pub(crate) fn msl_compile(&mut self) -> Result<String> {
        Ok(String::new())
    }
    pub(crate) fn msl_to_name(&self, id: u32, allow_alias: bool) -> Result<String> {
        self.base_to_name(id, allow_alias)
    }
}

// Overrides not translated yet: CompilerGLSL's behaviour.
#[allow(unused_variables)]
impl Compiler {
    pub(crate) fn msl_emit_header(&mut self) -> Result<()> {
        self.glsl_emit_header()
    }

    pub(crate) fn msl_emit_entry_point_declarations(&mut self) -> Result<()> {
        Ok(())
    }

    pub(crate) fn msl_replace_illegal_names(&mut self) -> Result<()> {
        self.glsl_replace_illegal_names()
    }

    pub(crate) fn msl_to_func_call_arg(&mut self, arg: &Parameter, id: u32) -> Result<String> {
        self.glsl_to_func_call_arg(arg, id)
    }

    pub(crate) fn msl_unpack_expression_type(
        &mut self,
        expr_str: String,
        type_: &SPIRType,
        physical_type_id: u32,
        packed_type: bool,
        row_major: bool,
    ) -> Result<String> {
        Ok(expr_str)
    }

    pub(crate) fn msl_constant_op_expression(&mut self, cop: &SPIRConstantOp) -> Result<String> {
        self.glsl_constant_op_expression(cop)
    }

    pub(crate) fn msl_emit_mesh_tasks(&mut self, block: &SPIRBlock) -> Result<()> {
        self.glsl_emit_mesh_tasks(block)
    }

    pub(crate) fn msl_emit_complex_bitcast(
        &mut self,
        result_type: u32,
        id: u32,
        op0: u32,
    ) -> Result<bool> {
        self.glsl_emit_complex_bitcast(result_type, id, op0)
    }

    pub(crate) fn msl_emit_sampled_image_op(
        &mut self,
        result_type: u32,
        result_id: u32,
        image_id: u32,
        samp_id: u32,
    ) -> Result<()> {
        self.glsl_emit_sampled_image_op(result_type, result_id, image_id, samp_id)
    }

    pub(crate) fn msl_emit_texture_op(&mut self, i: &Instruction, sparse: bool) -> Result<()> {
        self.glsl_emit_texture_op(i, sparse)
    }

    pub(crate) fn msl_to_texture_op(
        &mut self,
        i: &Instruction,
        sparse: bool,
        forward: &mut bool,
        inherited_expressions: &mut Vec<u32>,
    ) -> Result<String> {
        self.glsl_to_texture_op(i, sparse, forward, inherited_expressions)
    }

    pub(crate) fn msl_to_function_name(
        &mut self,
        args: &TextureFunctionNameArguments,
    ) -> Result<String> {
        self.glsl_to_function_name(args)
    }

    pub(crate) fn msl_to_function_args(
        &mut self,
        args: &TextureFunctionArguments,
        p_forward: &mut bool,
    ) -> Result<String> {
        self.glsl_to_function_args(args, p_forward)
    }

    pub(crate) fn msl_emit_glsl_op(
        &mut self,
        result_type: u32,
        id: u32,
        eop: u32,
        args: &[u32],
        length: u32,
    ) -> Result<()> {
        self.glsl_emit_glsl_op(result_type, id, eop, args, length)
    }

    pub(crate) fn msl_emit_spv_amd_shader_trinary_minmax_op(
        &mut self,
        result_type: u32,
        id: u32,
        eop: u32,
        args: &[u32],
        length: u32,
    ) -> Result<()> {
        self.glsl_emit_spv_amd_shader_trinary_minmax_op(result_type, id, eop, args, length)
    }

    pub(crate) fn msl_emit_subgroup_op(&mut self, i: &Instruction) -> Result<()> {
        self.glsl_emit_subgroup_op(i)
    }

    pub(crate) fn msl_bitcast_glsl_op(
        &mut self,
        out_type: &SPIRType,
        in_type: &SPIRType,
    ) -> Result<String> {
        self.glsl_bitcast_glsl_op(out_type, in_type)
    }

    pub(crate) fn msl_builtin_to_glsl(
        &mut self,
        builtin: BuiltIn,
        storage: StorageClass,
    ) -> Result<String> {
        self.glsl_builtin_to_glsl(builtin, storage)
    }

    pub(crate) fn msl_access_chain_needs_stage_io_builtin_translation(
        &mut self,
        base: u32,
    ) -> Result<bool> {
        Ok(true)
    }

    pub(crate) fn msl_check_physical_type_cast(
        &mut self,
        expr: &mut String,
        type_: Option<&SPIRType>,
        physical_type: u32,
    ) -> Result<bool> {
        Ok(false)
    }

    pub(crate) fn msl_prepare_access_chain_for_scalar_access(
        &mut self,
        expr: &mut String,
        type_: &SPIRType,
        storage: StorageClass,
        is_packed: &mut bool,
    ) -> Result<bool> {
        Ok(false)
    }

    pub(crate) fn msl_get_physical_type_id_stride(&mut self, type_id: TypeID) -> Result<u32> {
        spirv_cross_throw!("Invalid to call get_physical_type_id_stride on a backend without native pointer support.")
    }

    pub(crate) fn msl_skip_argument(&self, id: u32) -> Result<bool> {
        self.glsl_skip_argument(id)
    }

    pub(crate) fn msl_emit_store_statement(
        &mut self,
        lhs_expression: u32,
        rhs_expression: u32,
    ) -> Result<()> {
        self.glsl_emit_store_statement(lhs_expression, rhs_expression)
    }

    pub(crate) fn msl_emit_instruction(&mut self, instruction: &Instruction) -> Result<()> {
        self.glsl_emit_instruction(instruction)
    }

    pub(crate) fn msl_to_member_reference(
        &mut self,
        base: u32,
        type_: &SPIRType,
        index: u32,
        ptr_chain_is_resolved: bool,
    ) -> Result<String> {
        Ok(join!(".", self.to_member_name(type_, index)?))
    }

    pub(crate) fn msl_is_non_native_row_major_matrix(&mut self, id: u32) -> Result<bool> {
        Ok(self.glsl_is_non_native_row_major_matrix(id))
    }

    pub(crate) fn msl_member_is_non_native_row_major_matrix(
        &mut self,
        type_: &SPIRType,
        index: u32,
    ) -> Result<bool> {
        self.glsl_member_is_non_native_row_major_matrix(type_, index)
    }

    pub(crate) fn msl_convert_row_major_matrix(
        &mut self,
        exp_str: String,
        exp_type: &SPIRType,
        physical_type_id: u32,
        is_packed: bool,
        relaxed: bool,
    ) -> Result<String> {
        self.glsl_convert_row_major_matrix(exp_str, exp_type, physical_type_id, is_packed, relaxed)
    }

    pub(crate) fn msl_variable_decl_type(
        &mut self,
        type_: &SPIRType,
        name: &str,
        id: u32,
    ) -> Result<String> {
        self.glsl_variable_decl_type(type_, name, id)
    }

    pub(crate) fn msl_variable_decl_is_remapped_storage(
        &self,
        var: &SPIRVariable,
        storage: StorageClass,
    ) -> Result<bool> {
        Ok(var.storage == storage)
    }

    pub(crate) fn msl_emit_struct_member(
        &mut self,
        type_: &SPIRType,
        member_type_id: u32,
        index: u32,
        qualifier: &str,
        base_offset: u32,
    ) -> Result<()> {
        self.glsl_emit_struct_member(type_, member_type_id, index, qualifier, base_offset)
    }

    pub(crate) fn msl_to_qualifiers_glsl(&mut self, id: u32) -> Result<String> {
        self.glsl_to_qualifiers_glsl(id)
    }

    pub(crate) fn msl_to_initializer_expression(&mut self, var: &SPIRVariable) -> Result<String> {
        self.to_unpacked_expression(var.initializer, true)
    }

    pub(crate) fn msl_to_zero_initialized_expression(&mut self, type_id: u32) -> Result<String> {
        self.glsl_to_zero_initialized_expression(type_id)
    }

    pub(crate) fn msl_type_to_array_glsl(
        &mut self,
        type_: &SPIRType,
        variable_id: u32,
    ) -> Result<String> {
        self.glsl_type_to_array_glsl(type_, variable_id)
    }

    pub(crate) fn msl_image_type_glsl(
        &mut self,
        type_: &SPIRType,
        id: u32,
        member: bool,
    ) -> Result<String> {
        self.glsl_image_type_glsl(type_, id, member)
    }

    pub(crate) fn msl_type_to_glsl(&mut self, type_: &SPIRType, id: u32) -> Result<String> {
        self.glsl_type_to_glsl(type_, id)
    }

    pub(crate) fn msl_builtin_translates_to_nonarray(&self, builtin: BuiltIn) -> Result<bool> {
        Ok(false) // GLSL itself does not need to translate array builtin types to non-array builtin types
    }

    pub(crate) fn msl_emit_function_prototype(
        &mut self,
        func_id: u32,
        return_flags: &Bitset,
    ) -> Result<()> {
        self.glsl_emit_function_prototype(func_id, return_flags)
    }

    pub(crate) fn msl_emit_fixup(&mut self) -> Result<()> {
        self.glsl_emit_fixup()
    }

    pub(crate) fn msl_emit_workgroup_initialization(&mut self, var: &SPIRVariable) -> Result<()> {
        Ok(())
    }

    pub(crate) fn msl_emit_array_copy(
        &mut self,
        expr: Option<&str>,
        lhs_id: u32,
        rhs_id: u32,
        lhs_storage: StorageClass,
        rhs_storage: StorageClass,
    ) -> Result<bool> {
        self.glsl_emit_array_copy(expr, lhs_id, rhs_id, lhs_storage, rhs_storage)
    }

    pub(crate) fn msl_cast_from_variable_load(
        &mut self,
        source_id: u32,
        expr: &mut String,
        expr_type: &SPIRType,
    ) -> Result<()> {
        self.glsl_cast_from_variable_load(source_id, expr, expr_type)
    }

    pub(crate) fn msl_cast_to_variable_store(
        &mut self,
        target_id: u32,
        expr: &mut String,
        expr_type: &SPIRType,
    ) -> Result<()> {
        self.glsl_cast_to_variable_store(target_id, expr, expr_type)
    }

    pub(crate) fn msl_emit_block_hints(&mut self, block: &SPIRBlock) -> Result<()> {
        self.glsl_emit_block_hints(block)
    }
}
