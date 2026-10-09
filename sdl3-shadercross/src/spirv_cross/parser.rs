// Rust translation of spirv_parser.hpp and spirv_parser.cpp from
// SPIRV-Cross.
// Copyright 2018-2021 Arm Limited
// SPDX-License-Identifier: Apache-2.0 OR MIT
// This is an altered (translated to Rust) version of the original
// software; see LICENSE.txt.

//! `Parser`: SPIR-V words to [`ParsedIR`].

use super::common::*;
use super::parsed_ir::*;
use super::spirv::*;

/// `Parser`.
pub struct Parser {
    ir: ParsedIR,
    // C++ keeps pointers to the function and block being parsed; here,
    // their ids.
    current_function: Option<u32>,
    current_block: Option<u32>,
    // For workarounds.
    ignore_trailing_block_opcodes: bool,

    // This must be an ordered data structure so we always pick the same type aliases.
    global_struct_cache: Vec<u32>,
    forward_pointer_fixups: Vec<(u32, u32)>,
}

fn decoration_is_string(decoration: Decoration) -> bool {
    matches!(decoration, DecorationUserSemantic)
}

#[inline]
fn swap_endian(v: u32) -> u32 {
    v.swap_bytes()
}

fn is_valid_spirv_version(version: u32) -> bool {
    matches!(
        version,
        // Allow v99 since it tends to just work.
        99 | 0x10000 // SPIR-V 1.0
            | 0x10100 // SPIR-V 1.1
            | 0x10200 // SPIR-V 1.2
            | 0x10300 // SPIR-V 1.3
            | 0x10400 // SPIR-V 1.4
            | 0x10500 // SPIR-V 1.5
            | 0x10600 // SPIR-V 1.6
    )
}

fn extract_string(spirv: &[u32], offset: u32) -> Result<String> {
    let mut ret: Vec<u8> = Vec::new();
    for &w in spirv.iter().skip(offset as usize) {
        let mut w = w;
        for _ in 0..4 {
            let c = (w & 0xff) as u8;
            if c == 0 {
                // Note: strings are UTF-8 here; invalid UTF-8 (which C++
                // keeps as raw bytes) is replaced by U+FFFD.
                return Ok(String::from_utf8_lossy(&ret).into_owned());
            }
            ret.push(c);
            w >>= 8;
        }
    }

    spirv_cross_throw!("String was not terminated before EOF");
}

/// The words of an instruction: C++ reads `ops[i]` straight from the
/// SPIR-V buffer, also past the instruction's length (into the next
/// instructions); past the end of the buffer (or through the null stream
/// of an instruction without operands) the translation throws instead.
#[derive(Clone, Copy)]
struct Ops {
    offset: usize,
    length: u32,
}

impl Ops {
    #[inline]
    fn get(&self, spirv: &[u32], i: u32) -> Result<u32> {
        if self.length == 0 {
            spirv_cross_throw!("nullptr");
        }
        match spirv.get(self.offset + i as usize) {
            Some(&w) => Ok(w),
            None => spirv_cross_throw!("Compiler::stream() out of range."),
        }
    }

    fn slice<'a>(&self, spirv: &'a [u32], start: u32, count: u32) -> Result<&'a [u32]> {
        if count == 0 {
            return Ok(&[]);
        }
        if self.length == 0 {
            spirv_cross_throw!("nullptr");
        }
        let begin = self.offset + start as usize;
        match spirv.get(begin..begin + count as usize) {
            Some(s) => Ok(s),
            None => spirv_cross_throw!("Compiler::stream() out of range."),
        }
    }
}

impl Parser {
    pub fn new(spirv: Vec<u32>) -> Self {
        let mut ir = ParsedIR::new();
        ir.spirv = spirv;
        Parser {
            ir,
            current_function: None,
            current_block: None,
            ignore_trailing_block_opcodes: false,
            global_struct_cache: Vec::new(),
            forward_pointer_fixups: Vec::new(),
        }
    }

    pub fn from_words(spirv_data: &[u32]) -> Result<Self> {
        let mut v = Vec::new();
        v.try_reserve(spirv_data.len())
            .map_err(|_| CompilerError::new("std::bad_alloc"))?;
        v.extend_from_slice(spirv_data);
        Ok(Self::new(v))
    }

    pub fn get_parsed_ir(&mut self) -> &mut ParsedIR {
        &mut self.ir
    }

    pub fn into_parsed_ir(self) -> ParsedIR {
        self.ir
    }

    pub fn parse(&mut self) -> Result<()> {
        let len = self.ir.spirv.len();
        if len < 5 {
            spirv_cross_throw!("SPIRV file too small.");
        }

        // Endian-swap if we need to.
        if self.ir.spirv[0] == swap_endian(MagicNumber) {
            for c in self.ir.spirv.iter_mut() {
                *c = swap_endian(*c);
            }
        }

        let s = &self.ir.spirv;
        if s[0] != MagicNumber || !is_valid_spirv_version(s[1]) {
            spirv_cross_throw!("Invalid SPIRV format.");
        }

        let bound = s[3];

        const MAXIMUM_NUMBER_OF_IDS: u32 = 0x3fffff;
        if bound > MAXIMUM_NUMBER_OF_IDS {
            spirv_cross_throw!("ID bound exceeds limit of 0x3fffff.\n");
        }

        self.ir.set_id_bounds(bound)?;

        let mut offset: usize = 5;

        let mut instructions: Vec<Instruction> = Vec::new();
        while offset < len {
            let w = self.ir.spirv[offset];
            let mut instr = Instruction {
                op: (w & 0xffff) as u16,
                count: ((w >> 16) & 0xffff) as u16,
                ..Default::default()
            };

            if instr.count == 0 {
                spirv_cross_throw!(
                    "SPIR-V instructions cannot consume 0 words. Invalid SPIR-V file."
                );
            }

            instr.offset = offset as u32 + 1;
            instr.length = instr.count as u32 - 1;

            offset += instr.count as usize;

            if offset > self.ir.spirv.len() {
                spirv_cross_throw!("SPIR-V instruction goes out of bounds.");
            }

            instructions.push(instr);
        }

        for i in &instructions {
            self.parse_instruction(i)?;
        }

        for &(first, second) in &self.forward_pointer_fixups {
            let source = self.ir.get::<SPIRType>(second)?.clone();
            let target = self.ir.get_mut::<SPIRType>(first)?;
            target.member_types = source.member_types;
            target.basetype = source.basetype;
            target.self_ = source.self_;
        }
        self.forward_pointer_fixups.clear();

        for source in self.ir.sources.iter_mut() {
            // (std::sort is not stable; the markers are compared by line only.)
            source.line_markers.sort_by(|a, b| a.line.cmp(&b.line));
        }

        if self.current_function.is_some() {
            spirv_cross_throw!("Function was not terminated.");
        }
        if self.current_block.is_some() {
            spirv_cross_throw!("Block was not terminated.");
        }
        if self.ir.default_entry_point == 0 {
            spirv_cross_throw!("There is no entry point in the SPIR-V module.");
        }
        Ok(())
    }

    fn set<T: IVariant>(&mut self, id: u32, val: T) -> Result<&mut T> {
        self.ir.add_typed_id(T::TYPE, id)?;
        let var = variant_set(self.ir.ids.at_mut(id)?, val)?;
        var.set_self(id);
        Ok(var)
    }

    fn get<T: IVariant>(&self, id: u32) -> Result<&T> {
        self.ir.get::<T>(id)
    }

    fn maybe_get<T: IVariant>(&self, id: u32) -> Result<Option<&T>> {
        // FIXME (upstream): C++ indexes ids[] unchecked here; past the end
        // it reads memory that (in practice) isn't of the type asked for,
        // so an out-of-range ID is "not a T" here.
        let Some(v) = self.ir.ids.get(id as usize) else {
            return Ok(None);
        };
        if v.get_type() == T::TYPE {
            Ok(Some(self.get::<T>(id)?))
        } else {
            Ok(None)
        }
    }

    fn current_function_mut(&mut self) -> Result<&mut SPIRFunction> {
        let id = self.current_function.unwrap_or(0);
        self.ir.get_mut::<SPIRFunction>(id)
    }

    fn current_block_mut(&mut self) -> Result<&mut SPIRBlock> {
        let id = self.current_block.unwrap_or(0);
        self.ir.get_mut::<SPIRBlock>(id)
    }

    fn block_meta(&mut self, id: u32) -> Result<&mut BlockMetaFlags> {
        self.ir.block_meta.at_mut(id)
    }

    fn parse_instruction(&mut self, instruction: &Instruction) -> Result<()> {
        let ops = Ops {
            offset: instruction.offset as usize,
            length: instruction.length,
        };
        // (stream(): the instruction was bounds-checked when it was split off.)
        let op = instruction.op as Op;
        let length = instruction.length;
        macro_rules! o {
            ($i:expr) => {
                ops.get(&self.ir.spirv, $i)?
            };
        }

        // HACK for glslang that might emit OpEmitMeshTasksEXT followed by return / branch.
        // Instead of failing hard, just ignore it.
        if self.ignore_trailing_block_opcodes {
            self.ignore_trailing_block_opcodes = false;
            if op == OpReturn || op == OpBranch || op == OpUnreachable {
                return Ok(());
            }
        }

        match op {
            OpSourceExtension | OpNop | OpModuleProcessed => {}

            OpString => {
                let s = extract_string(&self.ir.spirv, instruction.offset + 1)?;
                self.set(o!(0), SPIRString::new(s))?;
            }

            OpMemoryModel => {
                self.ir.addressing_model = o!(0);
                self.ir.memory_model = o!(1);
            }

            OpSource => {
                let mut source = Source {
                    lang: o!(0),
                    ..Default::default()
                };

                match source.lang {
                    SourceLanguageESSL => {
                        source.es = true;
                        source.version = o!(1);
                        source.known = true;
                        source.hlsl = false;
                    }

                    SourceLanguageGLSL => {
                        source.es = false;
                        source.version = o!(1);
                        source.known = true;
                        source.hlsl = false;
                    }

                    SourceLanguageHLSL => {
                        // For purposes of cross-compiling, this is GLSL 450.
                        source.es = false;
                        source.version = 450;
                        source.known = true;
                        source.hlsl = true;
                    }

                    _ => {
                        source.known = false;
                    }
                }

                if length >= 3 {
                    source.file_id = o!(2);
                }

                if length >= 4 {
                    source.source = extract_string(&self.ir.spirv, instruction.offset + 3)?;
                }

                self.ir.sources.push(source);
            }

            OpSourceContinued => {
                if !self.ir.sources.is_empty() {
                    let s = extract_string(&self.ir.spirv, instruction.offset)?;
                    self.ir.sources.back_mut()?.source += &s;
                }
            }

            OpUndef => {
                let result_type = o!(0);
                let id = o!(1);
                self.set(id, SPIRUndef::new(result_type))?;
                if self.current_block.is_some() {
                    self.current_block_mut()?.ops.push(instruction.clone());
                }
            }

            OpCapability => {
                let cap = o!(0);
                if cap == CapabilityKernel {
                    spirv_cross_throw!("Kernel capability not supported.");
                }

                self.ir.declared_capabilities.push(cap);
            }

            OpExtension => {
                let ext = extract_string(&self.ir.spirv, instruction.offset)?;
                self.ir.declared_extensions.push(ext);
            }

            OpExtInstImport => {
                let id = o!(0);

                let ext = extract_string(&self.ir.spirv, instruction.offset + 1)?;
                let spirv_ext = match ext.as_str() {
                    "GLSL.std.450" => Extension::GLSL,
                    "DebugInfo" => Extension::SPV_debug_info,
                    "SPV_AMD_shader_ballot" => Extension::SPV_AMD_shader_ballot,
                    "SPV_AMD_shader_explicit_vertex_parameter" => {
                        Extension::SPV_AMD_shader_explicit_vertex_parameter
                    }
                    "SPV_AMD_shader_trinary_minmax" => Extension::SPV_AMD_shader_trinary_minmax,
                    "SPV_AMD_gcn_shader" => Extension::SPV_AMD_gcn_shader,
                    "NonSemantic.DebugPrintf" => Extension::NonSemanticDebugPrintf,
                    "NonSemantic.Shader.DebugInfo.100" => Extension::NonSemanticShaderDebugInfo,
                    _ if ext.starts_with("NonSemantic.") => Extension::NonSemanticGeneric,
                    _ => Extension::Unsupported,
                };

                self.set(id, SPIRExtension::new(spirv_ext))?;
                // Other SPIR-V extensions which have ExtInstrs are currently not supported.
            }

            OpExtInst | OpExtInstWithForwardRefsKHR => {
                // The SPIR-V debug information extended instructions might come at global scope.
                if self.current_block.is_some() {
                    self.current_block_mut()?.ops.push(instruction.clone());
                    if length >= 2 {
                        if let Some(type_) = self.maybe_get::<SPIRType>(o!(0))? {
                            let width = type_.width;
                            self.ir.load_type_width.entry(o!(1)).or_insert(width);
                        }
                    }
                }

                if op == OpExtInst && length > 4 {
                    // Don't want to deal with ForwardRefs here.
                    let ext = self.get::<SPIRExtension>(o!(2))?;
                    if ext.ext == Extension::NonSemanticShaderDebugInfo {
                        let instr = o!(3);
                        if instr == NonSemanticShaderDebugInfo100DebugSource {
                            let s = self.get::<SPIRString>(o!(4))?.str.clone();
                            self.set(o!(1), SPIRString::new(s))?;

                            let mut source = Source {
                                file_id: o!(4),
                                define_id: o!(1),
                                ..Default::default()
                            };
                            if length >= 6 {
                                source.source = self.ir.get::<SPIRString>(o!(5))?.str.clone();
                            }
                            self.ir.sources.push(source);
                        } else if instr == NonSemanticShaderDebugInfo100DebugSourceContinued {
                            if length < 5 {
                                spirv_cross_throw!(
                                    "Invalid arguments for ShaderDebugInfo100DebugSourceContinued"
                                );
                            }
                            if !self.ir.sources.is_empty() {
                                let s = self.ir.get::<SPIRString>(o!(4))?.str.clone();
                                self.ir.sources.back_mut()?.source += &s;
                            }
                        } else if instr == NonSemanticShaderDebugInfo100DebugLine {
                            if length < 9 {
                                spirv_cross_throw!(
                                    "Invalid arguments for ShaderDebugInfo100DebugLine"
                                );
                            }
                            let source_id = o!(4);
                            let line_start = self.ir.get::<SPIRConstant>(o!(5))?.scalar_i32(0, 0);
                            let col_start = self.ir.get::<SPIRConstant>(o!(7))?.scalar_i32(0, 0);

                            let function_id = self.current_function.unwrap_or(0);
                            let block_id = self.current_block.unwrap_or(0);
                            for source in self.ir.sources.iter_mut() {
                                if source.define_id != source_id {
                                    continue;
                                }

                                source.line_markers.push(Marker {
                                    line: line_start as u32,
                                    col: col_start as u32,
                                    offset: instruction.offset - 1,
                                    function_id,
                                    block_id,
                                });
                                break;
                            }
                        } else if instr == NonSemanticShaderDebugInfo100DebugLocalVariable {
                            if length < 11 {
                                spirv_cross_throw!(
                                    "Invalid arguments for ShaderDebugInfo100DebugLocalVariable"
                                );
                            }
                            let name_id = o!(4);
                            let lvar = self.set(o!(1), SPIRDebugLocalVariable::default())?;
                            lvar.name_id = name_id;
                        } else if instr == NonSemanticShaderDebugInfo100DebugDeclare {
                            if length < 7 {
                                spirv_cross_throw!(
                                    "Invalid arguments for ShaderDebugInfo100DebugDeclare"
                                );
                            }
                            let lvar_self = self.get::<SPIRDebugLocalVariable>(o!(4))?.self_;
                            let var = self.ir.get_mut::<SPIRVariable>(o!(5))?;
                            var.debug_local_variables.push(lvar_self);
                        }
                    }
                }
            }

            OpEntryPoint => {
                let name = extract_string(&self.ir.spirv, instruction.offset + 2)?;
                let id = o!(1);
                let model = o!(0);
                // (std::unordered_map::insert(): an existing entry is kept.)
                if !self.ir.entry_points.contains_key(&id) {
                    self.ir
                        .entry_points
                        .insert(id, SPIREntryPoint::new(id, model, &name));
                }

                // Strings need nul-terminator and consume the whole word.
                let e_name_len = self
                    .ir
                    .entry_points
                    .get(&id)
                    .map(|e| e.name.len())
                    .unwrap_or(0);
                let strlen_words = ((e_name_len + 1 + 3) >> 2) as u32;

                let mut ivars = Vec::new();
                let mut i = strlen_words + 2;
                while i < instruction.length {
                    ivars.push(o!(i));
                    i += 1;
                }
                if let Some(e) = self.ir.entry_points.get_mut(&id) {
                    e.interface_variables.extend(ivars);
                }

                // Set the name of the entry point in case OpName is not provided later.
                let e_name = self
                    .ir
                    .entry_points
                    .get(&id)
                    .map(|e| e.name.clone())
                    .unwrap_or_default();
                self.ir.set_name(id, &e_name);

                // If we don't have an entry, make the first one our "default".
                if self.ir.default_entry_point == 0 {
                    self.ir.default_entry_point = id;
                }
            }

            OpExecutionMode => {
                let ep = o!(0);
                let mode = o!(1);
                // Read the operands first: C++ reads them as it goes, after
                // creating the entry.
                self.ir.entry_points.entry(ep).flags.set(mode);

                match mode {
                    ExecutionModeInvocations => {
                        let v = o!(2);
                        self.ir.entry_points.entry(ep).invocations = v;
                    }

                    ExecutionModeLocalSize => {
                        let (x, y, z) = (o!(2), o!(3), o!(4));
                        let execution = self.ir.entry_points.entry(ep);
                        execution.workgroup_size.x = x;
                        execution.workgroup_size.y = y;
                        execution.workgroup_size.z = z;
                    }

                    ExecutionModeOutputVertices => {
                        let v = o!(2);
                        self.ir.entry_points.entry(ep).output_vertices = v;
                    }

                    ExecutionModeOutputPrimitivesEXT => {
                        let v = o!(2);
                        self.ir.entry_points.entry(ep).output_primitives = v;
                    }

                    ExecutionModeSignedZeroInfNanPreserve => {
                        let v = o!(2);
                        let execution = self.ir.entry_points.entry(ep);
                        match v {
                            8 => execution.signed_zero_inf_nan_preserve_8 = true,
                            16 => execution.signed_zero_inf_nan_preserve_16 = true,
                            32 => execution.signed_zero_inf_nan_preserve_32 = true,
                            64 => execution.signed_zero_inf_nan_preserve_64 = true,
                            _ => spirv_cross_throw!(
                                "Invalid bit-width for SignedZeroInfNanPreserve."
                            ),
                        }
                    }

                    _ => {}
                }
            }

            OpExecutionModeId => {
                let ep = o!(0);
                let mode = o!(1);
                self.ir.entry_points.entry(ep).flags.set(mode);

                match mode {
                    ExecutionModeLocalSizeId => {
                        let (x, y, z) = (o!(2), o!(3), o!(4));
                        let execution = self.ir.entry_points.entry(ep);
                        execution.workgroup_size.id_x = x;
                        execution.workgroup_size.id_y = y;
                        execution.workgroup_size.id_z = z;
                    }

                    ExecutionModeFPFastMathDefault => {
                        let (k, v) = (o!(2), o!(3));
                        self.ir
                            .entry_points
                            .entry(ep)
                            .fp_fast_math_defaults
                            .insert(k, v);
                    }

                    _ => {}
                }
            }

            OpName => {
                let id = o!(0);
                let s = extract_string(&self.ir.spirv, instruction.offset + 1)?;
                self.ir.set_name(id, &s);
            }

            OpMemberName => {
                let id = o!(0);
                let member = o!(1);
                let s = extract_string(&self.ir.spirv, instruction.offset + 2)?;
                self.ir.set_member_name(id, member, &s)?;
            }

            OpDecorationGroup => {
                // Noop, this simply means an ID should be a collector of decorations.
                // The meta array is already a flat array of decorations which will contain the relevant decorations.
            }

            OpGroupDecorate => {
                let group_id = o!(0);
                let flags = self
                    .ir
                    .meta
                    .entry(group_id)
                    .or_default()
                    .decoration
                    .decoration_flags
                    .clone();

                // Copies decorations from one ID to another. Only copy decorations which are set in the group,
                // i.e., we cannot just copy the meta structure directly.
                for i in 1..length {
                    let target = o!(i);
                    for bit in flags.bits() {
                        let decoration = bit;

                        if decoration_is_string(decoration) {
                            let s = self.ir.get_decoration_string(group_id, decoration).clone();
                            self.ir.set_decoration_string(target, decoration, &s);
                        } else {
                            let w = *self
                                .ir
                                .meta
                                .entry(group_id)
                                .or_default()
                                .decoration_word_offset
                                .entry(decoration)
                                .or_default();
                            self.ir
                                .meta
                                .entry(target)
                                .or_default()
                                .decoration_word_offset
                                .insert(decoration, w);
                            let arg = self.ir.get_decoration(group_id, decoration);
                            self.ir.set_decoration(target, decoration, arg);
                        }
                    }
                }
            }

            OpGroupMemberDecorate => {
                let group_id = o!(0);
                let flags = self
                    .ir
                    .meta
                    .entry(group_id)
                    .or_default()
                    .decoration
                    .decoration_flags
                    .clone();

                // Copies decorations from one ID to another. Only copy decorations which are set in the group,
                // i.e., we cannot just copy the meta structure directly.
                let mut i: u32 = 1;
                while i + 1 < length {
                    let target = o!(i);
                    let index = o!(i + 1);
                    for bit in flags.bits() {
                        let decoration = bit;

                        if decoration_is_string(decoration) {
                            let s = self.ir.get_decoration_string(group_id, decoration).clone();
                            self.ir
                                .set_member_decoration_string(target, index, decoration, &s)?;
                        } else {
                            let arg = self.ir.get_decoration(group_id, decoration);
                            self.ir
                                .set_member_decoration(target, index, decoration, arg)?;
                        }
                    }
                    i += 2;
                }
            }

            OpDecorate | OpDecorateId => {
                // OpDecorateId technically supports an array of arguments, but our only supported decorations are single uint,
                // so merge decorate and decorate-id here.
                let id = o!(0);

                let decoration = o!(1);
                if length >= 3 {
                    let word_offset = instruction.offset + 2;
                    self.ir
                        .meta
                        .entry(id)
                        .or_default()
                        .decoration_word_offset
                        .insert(decoration, word_offset);
                    let arg = o!(2);
                    self.ir.set_decoration(id, decoration, arg);
                } else {
                    self.ir.set_decoration(id, decoration, 0);
                }
            }

            OpDecorateStringGOOGLE => {
                let id = o!(0);
                let decoration = o!(1);
                let s = extract_string(&self.ir.spirv, instruction.offset + 2)?;
                self.ir.set_decoration_string(id, decoration, &s);
            }

            OpMemberDecorate => {
                let id = o!(0);
                let member = o!(1);
                let decoration = o!(2);
                if length >= 4 {
                    let arg = o!(3);
                    self.ir.set_member_decoration(id, member, decoration, arg)?;
                } else {
                    self.ir.set_member_decoration(id, member, decoration, 0)?;
                }
            }

            // MemberDecorateIdEXT only applies to OffsetIdEXT when descriptors are packed in structs.
            // This is currently unsupported and will fail in compilation.
            // Pass it through in case someone just needs reflection.
            OpMemberDecorateIdEXT => {}

            OpMemberDecorateStringGOOGLE => {
                let id = o!(0);
                let member = o!(1);
                let decoration = o!(2);
                let s = extract_string(&self.ir.spirv, instruction.offset + 3)?;
                self.ir
                    .set_member_decoration_string(id, member, decoration, &s)?;
            }

            // Build up basic types.
            OpTypeVoid => {
                let id = o!(0);
                let type_ = self.set(id, SPIRType::new(op))?;
                type_.basetype = BaseType::Void;
            }

            OpTypeBool => {
                let id = o!(0);
                let type_ = self.set(id, SPIRType::new(op))?;
                type_.basetype = BaseType::Boolean;
                type_.width = 1;
            }

            OpTypeFloat => {
                let id = o!(0);
                let width = o!(1);
                self.set(id, SPIRType::new(op))?;

                if width != 16 && width != 8 && length > 2 {
                    spirv_cross_throw!("Unrecognized FP encoding mode for OpTypeFloat.");
                }

                let basetype = if width == 64 {
                    BaseType::Double
                } else if width == 32 {
                    BaseType::Float
                } else if width == 16 {
                    if length > 2 {
                        if o!(2) == FPEncodingBFloat16KHR {
                            BaseType::BFloat16
                        } else {
                            spirv_cross_throw!("Unrecognized encoding for OpTypeFloat 16.");
                        }
                    } else {
                        BaseType::Half
                    }
                } else if width == 8 {
                    if length < 2 {
                        spirv_cross_throw!("Missing encoding for OpTypeFloat 8.");
                    } else if o!(2) == FPEncodingFloat8E4M3EXT {
                        BaseType::FloatE4M3
                    } else if o!(2) == FPEncodingFloat8E5M2EXT {
                        BaseType::FloatE5M2
                    } else {
                        spirv_cross_throw!("Invalid encoding for OpTypeFloat 8.");
                    }
                } else {
                    spirv_cross_throw!("Unrecognized bit-width of floating point type.");
                };
                let type_ = self.ir.get_mut::<SPIRType>(id)?;
                type_.basetype = basetype;
                type_.width = width;
            }

            OpTypeInt => {
                let id = o!(0);
                let width = o!(1);
                let signedness = o!(2) != 0;
                self.set(id, SPIRType::new(op))?;
                let basetype = if signedness {
                    to_signed_basetype(width)?
                } else {
                    to_unsigned_basetype(width)?
                };
                let type_ = self.ir.get_mut::<SPIRType>(id)?;
                type_.basetype = basetype;
                type_.width = width;
            }

            // Build composite types by "inheriting".
            // NOTE: The self member is also copied! For pointers and array modifiers this is a good thing
            // since we can refer to decorations on pointee classes which is needed for UBO/SSBO, I/O blocks in geometry/tess etc.
            OpTypeVector => {
                let id = o!(0);
                let vecsize = o!(2);
                // The translation's guard: SPIR-V vectors have at most 16
                // components (with Vector16); the backends emit code per
                // component, so a huge count from malformed SPIR-V would
                // run them out of memory.
                if vecsize > 16 {
                    spirv_cross_throw!("Vector component count is out of range.");
                }

                let base = self.get::<SPIRType>(o!(1))?.clone();
                let parent = o!(1);
                let vecbase = self.set(id, base)?;

                vecbase.op = op;
                vecbase.vecsize = vecsize;
                vecbase.self_ = id;
                vecbase.parent_type = parent;
            }

            OpTypeMatrix => {
                let id = o!(0);
                let colcount = o!(2);
                // As for vectors: matrices have at most 4 columns.
                if colcount > 16 {
                    spirv_cross_throw!("Matrix column count is out of range.");
                }

                let base = self.get::<SPIRType>(o!(1))?.clone();
                let parent = o!(1);
                let matrixbase = self.set(id, base)?;

                matrixbase.op = op;
                matrixbase.columns = colcount;
                matrixbase.self_ = id;
                matrixbase.parent_type = parent;
            }

            OpTypeCooperativeMatrixKHR => {
                let id = o!(0);
                let base = self.get::<SPIRType>(o!(1))?.clone();
                let (w1, w2, w3, w4, w5) = (o!(1), o!(2), o!(3), o!(4), o!(5));
                let matrixbase = self.set(id, base)?;

                matrixbase.op = op;
                matrixbase.ext.w[3] = w2; // cooperative.scope_id
                matrixbase.ext.w[1] = w3; // cooperative.rows_id
                matrixbase.ext.w[2] = w4; // cooperative.columns_id
                matrixbase.ext.w[0] = w5; // cooperative.use_id
                matrixbase.self_ = id;
                matrixbase.parent_type = w1;
            }

            OpTypeCooperativeVectorNV => {
                let id = o!(0);
                let (w1, w2) = (o!(1), o!(2));
                let type_ = self.set(id, SPIRType::new(op))?;

                type_.basetype = BaseType::CoopVecNV;
                type_.op = op;
                type_.ext.w[0] = w1; // coopVecNV.component_type_id
                type_.ext.w[1] = w2; // coopVecNV.component_count_id
                type_.parent_type = w1;

                // CoopVec-Nv can be used with integer operations like SMax where
                // where spirv-opt does explicit checks on integer bitwidth
                let width = self.get::<SPIRType>(w1)?.width;
                self.ir.get_mut::<SPIRType>(id)?.width = width;
            }

            OpTypeArray => {
                let id = o!(0);
                let tid = o!(1);
                let base = self.get::<SPIRType>(tid)?.clone();
                let base_forward_pointer = base.forward_pointer;
                let base_self = base.self_;
                self.set(id, base)?;
                {
                    let arraybase = self.ir.get_mut::<SPIRType>(id)?;
                    arraybase.op = op;
                    arraybase.parent_type = tid;
                }

                let cid = o!(2);
                self.ir.mark_used_as_array_length(cid)?;
                let (literal, scalar) = match self.maybe_get::<SPIRConstant>(cid)? {
                    Some(c) => (!c.specialization, c.scalar(0, 0)),
                    None => (false, 0),
                };

                // We're copying type information into Array types, so we'll need a fixup for any physical pointer
                // references.
                if base_forward_pointer {
                    self.forward_pointer_fixups.push((id, tid));
                }

                let arraybase = self.ir.get_mut::<SPIRType>(id)?;
                arraybase.array_size_literal.push(literal);
                arraybase.array.push(if literal { scalar } else { cid });

                // .self resolves down to non-array/non-pointer type.
                arraybase.self_ = base_self;
            }

            OpTypeRuntimeArray => {
                let id = o!(0);

                let base = self.get::<SPIRType>(o!(1))?.clone();
                let base_forward_pointer = base.forward_pointer;
                let base_self = base.self_;
                let parent = o!(1);
                self.set(id, base)?;

                // We're copying type information into Array types, so we'll need a fixup for any physical pointer
                // references.
                if base_forward_pointer {
                    self.forward_pointer_fixups.push((id, parent));
                }

                let arraybase = self.ir.get_mut::<SPIRType>(id)?;
                arraybase.op = op;
                arraybase.array.push(0);
                arraybase.array_size_literal.push(true);
                arraybase.parent_type = parent;

                // .self resolves down to non-array/non-pointer type.
                arraybase.self_ = base_self;
            }

            OpTypeImage => {
                let id = o!(0);
                let (w1, w2, w3, w4, w5, w6, w7) =
                    (o!(1), o!(2), o!(3), o!(4), o!(5), o!(6), o!(7));
                let access = if length >= 9 {
                    o!(8)
                } else {
                    AccessQualifierMax
                };
                let type_ = self.set(id, SPIRType::new(op))?;
                type_.basetype = BaseType::Image;
                type_.image.type_ = w1;
                type_.image.dim = w2;
                type_.image.depth = w3 == 1;
                type_.image.arrayed = w4 != 0;
                type_.image.ms = w5 != 0;
                type_.image.sampled = w6;
                type_.image.format = w7;
                type_.image.access = access;
            }

            OpTypeSampledImage => {
                let id = o!(0);
                let imagetype = o!(1);
                self.set(id, SPIRType::new(op))?;
                let mut t = self.get::<SPIRType>(imagetype)?.clone();
                t.basetype = BaseType::SampledImage;
                t.self_ = id;
                *self.ir.get_mut::<SPIRType>(id)? = t;
            }

            OpTypeSampler => {
                let id = o!(0);
                let type_ = self.set(id, SPIRType::new(op))?;
                type_.basetype = BaseType::Sampler;
            }

            OpTypeUntypedPointerKHR | OpTypePointer => {
                let id = o!(0);

                // Very rarely, we might receive a FunctionPrototype here.
                // We won't be able to compile it, but we shouldn't crash when parsing.
                // We should be able to reflect.
                let base_id = if op == OpTypePointer { o!(2) } else { 0 };
                let storage = o!(1);
                // (The base is looked up before set<>() replaces the id, as
                // C++ does; it is copied after.)
                let had_base =
                    op == OpTypePointer && self.maybe_get::<SPIRType>(base_id)?.is_some();
                self.set(id, SPIRType::new(op))?;
                let base = if had_base {
                    self.maybe_get::<SPIRType>(base_id)?.cloned()
                } else {
                    None
                };
                let base_forward_pointer =
                    base.as_ref().map(|b| b.forward_pointer).unwrap_or(false);

                let ptrbase = self.ir.get_mut::<SPIRType>(id)?;
                if let Some(base) = base {
                    *ptrbase = base;
                    ptrbase.op = op;
                }

                ptrbase.pointer = true;
                ptrbase.pointer_depth = ptrbase.pointer_depth.wrapping_add(1);
                ptrbase.storage = storage;

                if ptrbase.storage == StorageClassAtomicCounter {
                    ptrbase.basetype = BaseType::AtomicCounter;
                }

                if op == OpTypePointer {
                    ptrbase.parent_type = base_id;
                } else {
                    ptrbase.basetype = BaseType::Void;
                }

                if had_base && base_forward_pointer {
                    self.forward_pointer_fixups.push((id, base_id));
                }

                // Do NOT set ptrbase.self!
            }

            OpTypeForwardPointer => {
                let id = o!(0);
                let storage = o!(1);
                let ptrbase = self.set(id, SPIRType::new(op))?;
                ptrbase.pointer = true;
                ptrbase.pointer_depth += 1;
                ptrbase.storage = storage;
                ptrbase.forward_pointer = true;

                if ptrbase.storage == StorageClassAtomicCounter {
                    ptrbase.basetype = BaseType::AtomicCounter;
                }
            }

            OpTypeStruct => {
                let id = o!(0);
                let members = ops
                    .slice(&self.ir.spirv, 1, length.saturating_sub(1))?
                    .to_vec();
                let type_ = self.set(id, SPIRType::new(op))?;
                type_.basetype = BaseType::Struct;
                type_.member_types.extend(members);

                // Check if we have seen this struct type before, with just different
                // decorations.
                //
                // Add workaround for issue #17 as well by looking at OpName for the struct
                // types, which we shouldn't normally do.
                // We should not normally have to consider type aliases like this to begin with
                // however ... glslang issues #304, #307 cover this.

                // For stripped names, never consider struct type aliasing.
                // We risk declaring the same struct multiple times, but type-punning is not allowed
                // so this is safe.
                let consider_aliasing = !self.ir.get_name(id).is_empty();
                if consider_aliasing {
                    let mut alias = 0;
                    for &other in &self.global_struct_cache {
                        if self.ir.get_name(id) == self.ir.get_name(other)
                            && self.types_are_logically_equivalent(
                                self.get::<SPIRType>(id)?,
                                self.get::<SPIRType>(other)?,
                                0,
                            )?
                        {
                            alias = other;
                            break;
                        }
                    }
                    self.ir.get_mut::<SPIRType>(id)?.type_alias = alias;

                    if alias == 0 {
                        self.global_struct_cache.push(id);
                    }
                }
            }

            OpTypeFunction => {
                let id = o!(0);
                let ret = o!(1);

                let params = ops
                    .slice(&self.ir.spirv, 2, length.saturating_sub(2))?
                    .to_vec();
                let func = self.set(id, SPIRFunctionPrototype::new(ret))?;
                func.parameter_types.extend(params);
            }

            OpTypeAccelerationStructureKHR => {
                let id = o!(0);
                let type_ = self.set(id, SPIRType::new(op))?;
                type_.basetype = BaseType::AccelerationStructure;
            }

            OpTypeRayQueryKHR => {
                let id = o!(0);
                let type_ = self.set(id, SPIRType::new(op))?;
                type_.basetype = BaseType::RayQuery;
            }

            OpTypeTensorARM => {
                let id = o!(0);
                let t = o!(1);
                let rank = if length >= 3 { o!(2) } else { 0 };
                let shape = if length >= 4 { o!(3) } else { 0 };
                let type_ = self.set(id, SPIRType::new(op))?;
                type_.basetype = BaseType::Tensor;
                type_.ext = TypeExt::default();
                type_.ext.w[0] = t;
                type_.ext.w[1] = rank;
                type_.ext.w[2] = shape;
            }

            // Variable declaration
            // All variables are essentially pointers with a storage qualifier.
            OpVariable => {
                let type_ = o!(0);
                let id = o!(1);
                let storage = o!(2);
                let initializer = if length == 4 { o!(3) } else { 0 };

                if storage == StorageClassFunction {
                    if self.current_function.is_none() {
                        spirv_cross_throw!("No function currently in scope");
                    }
                    self.current_function_mut()?.add_local_variable(id);
                }

                self.set(id, SPIRVariable::new(type_, storage, initializer, 0))?;
            }

            OpUntypedVariableKHR => {
                let type_ = o!(0);
                let id = o!(1);
                let storage = o!(2);
                let data_type = if length >= 4 { o!(3) } else { 0 };
                let initializer = if length >= 5 { o!(4) } else { 0 };

                if storage == StorageClassFunction {
                    if self.current_function.is_none() {
                        spirv_cross_throw!("No function currently in scope");
                    }
                    self.current_function_mut()?.add_local_variable(id);
                }

                let v = self.set(id, SPIRVariable::new(type_, storage, initializer, 0))?;
                v.untyped = true;
                v.untyped_alloca_type = data_type;
            }

            // OpPhi
            // OpPhi is a fairly magical opcode.
            // It selects temporary variables based on which parent block we *came from*.
            // In high-level languages we can "de-SSA" by creating a function local, and flush out temporaries to this function-local
            // variable to emulate SSA Phi.
            OpPhi => {
                if self.current_function.is_none() {
                    spirv_cross_throw!("No function currently in scope");
                }
                if self.current_block.is_none() {
                    spirv_cross_throw!("No block currently in scope");
                }

                let result_type = o!(0);
                let id = o!(1);

                // Instead of a temporary, create a new function-wide temporary with this ID instead.
                let var = self.set(
                    id,
                    SPIRVariable::new(result_type, StorageClassFunction, 0, 0),
                )?;
                var.phi_variable = true;

                self.current_function_mut()?.add_local_variable(id);

                let mut phis = Vec::new();
                let mut i: u32 = 2;
                while i + 2 <= length {
                    phis.push(Phi {
                        local_variable: o!(i),
                        parent: o!(i + 1),
                        function_variable: id,
                    });
                    i += 2;
                }
                self.current_block_mut()?.phi_variables.extend(phis);
            }

            // Constants
            OpSpecConstant
            | OpConstant
            | OpConstantCompositeReplicateEXT
            | OpSpecConstantCompositeReplicateEXT => {
                let id = o!(1);
                let width = self.get::<SPIRType>(o!(0))?.width;
                let ty = o!(0);
                if op == OpConstantCompositeReplicateEXT
                    || op == OpSpecConstantCompositeReplicateEXT
                {
                    let subconstant = o!(2);
                    self.set(
                        id,
                        SPIRConstant::new_composite(
                            ty,
                            &[subconstant],
                            op == OpSpecConstantCompositeReplicateEXT,
                            true,
                        ),
                    )?;
                } else if width > 32 {
                    let v = o!(2) as u64 | ((o!(3) as u64) << 32);
                    self.set(id, SPIRConstant::new_scalar64(ty, v, op == OpSpecConstant))?;
                } else {
                    let v = o!(2);
                    self.set(id, SPIRConstant::new_scalar32(ty, v, op == OpSpecConstant))?;
                }
            }

            OpSpecConstantFalse | OpConstantFalse => {
                let id = o!(1);
                let ty = o!(0);
                self.set(
                    id,
                    SPIRConstant::new_scalar32(ty, 0, op == OpSpecConstantFalse),
                )?;
            }

            OpSpecConstantTrue | OpConstantTrue => {
                let id = o!(1);
                let ty = o!(0);
                self.set(
                    id,
                    SPIRConstant::new_scalar32(ty, 1, op == OpSpecConstantTrue),
                )?;
            }

            OpConstantNull => {
                let id = o!(1);
                let type_ = o!(0);
                self.ir.make_constant_null(id, type_, true)?;
            }

            OpSpecConstantComposite | OpConstantComposite => {
                let id = o!(1);
                let type_ = o!(0);

                let ctype = self.get::<SPIRType>(type_)?;

                // We can have constants which are structs and arrays.
                // In this case, our SPIRConstant will be a list of other SPIRConstant ids which we
                // can refer to.
                if ctype.basetype == BaseType::Struct || !ctype.array.is_empty() {
                    let elems = ops
                        .slice(&self.ir.spirv, 2, length.wrapping_sub(2))?
                        .to_vec();
                    self.set(
                        id,
                        SPIRConstant::new_composite(
                            type_,
                            &elems,
                            op == OpSpecConstantComposite,
                            false,
                        ),
                    )?;
                } else {
                    let elements = length.wrapping_sub(2);
                    if elements > 4 {
                        spirv_cross_throw!(
                            "OpConstantComposite only supports 1, 2, 3 and 4 elements."
                        );
                    }

                    let mut c: Vec<SPIRConstant> = Vec::with_capacity(4);
                    for i in 0..elements {
                        // Specialization constants operations can also be part of this.
                        // We do not know their value, so any attempt to query SPIRConstant later
                        // will fail. We can only propagate the ID of the expression and use to_expression on it.
                        let eid = o!(2 + i);
                        let constant_op = self.maybe_get::<SPIRConstantOp>(eid)?.cloned();
                        let undef_op = self.maybe_get::<SPIRUndef>(eid)?.cloned();
                        if let Some(constant_op) = constant_op {
                            if op == OpConstantComposite {
                                spirv_cross_throw!("Specialization constant operation used in OpConstantComposite.");
                            }

                            let mut r = SPIRConstant::default();
                            r.make_null(self.get::<SPIRType>(constant_op.basetype)?);
                            r.self_ = constant_op.self_;
                            r.constant_type = constant_op.basetype;
                            r.specialization = true;
                            c.push(r);
                        } else if let Some(undef_op) = undef_op {
                            // Undefined, just pick 0.
                            let mut r = SPIRConstant::default();
                            r.make_null(self.get::<SPIRType>(undef_op.basetype)?);
                            r.constant_type = undef_op.basetype;
                            c.push(r);
                        } else {
                            c.push(self.get::<SPIRConstant>(eid)?.clone());
                        }
                    }
                    let refs: Vec<&SPIRConstant> = c.iter().collect();
                    let constant = SPIRConstant::new_vector_matrix(
                        type_,
                        &refs,
                        op == OpSpecConstantComposite,
                    )?;
                    self.set(id, constant)?;
                }
            }

            OpConstantSizeOfEXT => {
                let id = o!(1);
                let type_ = o!(0);
                let size_of = o!(2);
                let c = self.set(id, SPIRConstant::new_typed(type_))?;
                c.size_of_type = size_of;
            }

            OpTypeBufferEXT => {
                let type_ = o!(0);
                let storage = o!(1);
                let t = self.set(type_, SPIRType::new(OpTypeBufferEXT))?;
                t.basetype = BaseType::DescriptorHeapBuffer;
                t.ext.w[0] = storage; // descriptor_heap_buffer.storage
            }

            // Functions
            OpFunction => {
                let res = o!(0);
                let id = o!(1);
                // Control
                let type_ = o!(3);

                if self.current_function.is_some() {
                    spirv_cross_throw!("Must end a function before starting a new one!");
                }

                self.set(id, SPIRFunction::new(res, type_))?;
                self.current_function = Some(id);
            }

            OpFunctionParameter => {
                let type_ = o!(0);
                let id = o!(1);

                if self.current_function.is_none() {
                    spirv_cross_throw!("Must be in a function!");
                }

                self.current_function_mut()?.add_parameter(type_, id, false);
                self.set(id, SPIRVariable::new(type_, StorageClassFunction, 0, 0))?;
            }

            OpFunctionEnd => {
                if self.current_block.is_some() {
                    // Very specific error message, but seems to come up quite often.
                    spirv_cross_throw!(
                        "Cannot end a function before ending the current block.\n\
                         Likely cause: If this SPIR-V was created from glslang HLSL, make sure the entry point is valid."
                    );
                }
                self.current_function = None;
            }

            // Blocks
            OpLabel => {
                // OpLabel always starts a block.
                if self.current_function.is_none() {
                    spirv_cross_throw!("Blocks cannot exist outside functions!");
                }

                let id = o!(0);

                {
                    let f = self.current_function_mut()?;
                    f.blocks.push(id);
                    if f.entry_block == 0 {
                        f.entry_block = id;
                    }
                }

                if self.current_block.is_some() {
                    spirv_cross_throw!("Cannot start a block before ending the current block.");
                }

                self.set(id, SPIRBlock::default())?;
                self.current_block = Some(id);
            }

            // Branch instructions end blocks.
            OpBranch => {
                if self.current_block.is_none() {
                    spirv_cross_throw!("Trying to end a non-existing block.");
                }

                let target = o!(0);
                let b = self.current_block_mut()?;
                b.terminator = Terminator::Direct;
                b.next_block = target;
                self.current_block = None;
            }

            OpBranchConditional => {
                if self.current_block.is_none() {
                    spirv_cross_throw!("Trying to end a non-existing block.");
                }

                let (w0, w1, w2) = (o!(0), o!(1), o!(2));
                let b = self.current_block_mut()?;
                b.condition = w0;
                b.true_block = w1;
                b.false_block = w2;

                b.terminator = Terminator::Select;

                if b.true_block == b.false_block {
                    // Bogus conditional, translate to a direct branch.
                    // Avoids some ugly edge cases later when analyzing CFGs.

                    // There are some super jank cases where the merge block is different from the true/false,
                    // and later branches can "break" out of the selection construct this way.
                    // This is complete nonsense, but CTS hits this case.
                    // In this scenario, we should see the selection construct as more of a Switch with one default case.
                    // The problem here is that this breaks any attempt to break out of outer switch statements,
                    // but it's theoretically solvable if this ever comes up using the ladder breaking system ...

                    if b.true_block != b.next_block && b.merge == Merge::MergeSelection {
                        let ids = self.ir.increase_bound_by(2)?;

                        let type_ = self.set(ids, SPIRType::new(OpTypeInt))?;
                        type_.basetype = BaseType::Int;
                        type_.width = 32;
                        let c_self = self.set(ids + 1, SPIRConstant::new_typed(ids))?.self_;

                        let b = self.current_block_mut()?;
                        b.condition = c_self;
                        b.default_block = b.true_block;
                        b.terminator = Terminator::MultiSelect;
                        let next = b.next_block;
                        *self.block_meta(next)? &= !BLOCK_META_SELECTION_MERGE_BIT;
                        *self.block_meta(next)? |= BLOCK_META_MULTISELECT_MERGE_BIT;
                    } else {
                        // Collapse loops if we have to.
                        let collapsed_loop =
                            b.true_block == b.merge_block && b.merge == Merge::MergeLoop;

                        let (merge_block, continue_block) = (b.merge_block, b.continue_block);
                        b.next_block = b.true_block;
                        b.condition = 0;
                        b.true_block = 0;
                        b.false_block = 0;
                        b.merge_block = 0;
                        b.merge = Merge::MergeNone;
                        b.terminator = Terminator::Direct;

                        if collapsed_loop {
                            *self.block_meta(merge_block)? &= !BLOCK_META_LOOP_MERGE_BIT;
                            *self.block_meta(continue_block)? &= !BLOCK_META_CONTINUE_BIT;
                        }
                    }
                }

                self.current_block = None;
            }

            OpSwitch => {
                if self.current_block.is_none() {
                    spirv_cross_throw!("Trying to end a non-existing block.");
                }

                let (w0, w1) = (o!(0), o!(1));
                let remaining_ops = length.wrapping_sub(2);
                let mut cases_32bit = Vec::new();
                let mut cases_64bit = Vec::new();
                if (remaining_ops % 2) == 0 {
                    let mut i: u32 = 2;
                    while i + 2 <= length {
                        cases_32bit.push(Case {
                            value: o!(i) as u64,
                            block: o!(i + 1),
                        });
                        i += 2;
                    }
                }

                if (remaining_ops % 3) == 0 {
                    let mut i: u32 = 2;
                    while i + 3 <= length {
                        let value = ((o!(i + 1) as u64) << 32) | o!(i) as u64;
                        cases_64bit.push(Case {
                            value,
                            block: o!(i + 2),
                        });
                        i += 3;
                    }
                }

                let b = self.current_block_mut()?;
                b.terminator = Terminator::MultiSelect;

                b.condition = w0;
                b.default_block = w1;
                b.cases_32bit.extend(cases_32bit);
                b.cases_64bit.extend(cases_64bit);

                // If we jump to next block, make it break instead since we're inside a switch case block at that point.
                let next = b.next_block;
                *self.block_meta(next)? |= BLOCK_META_MULTISELECT_MERGE_BIT;

                self.current_block = None;
            }

            OpKill | OpTerminateInvocation => {
                if self.current_block.is_none() {
                    spirv_cross_throw!("Trying to end a non-existing block.");
                }
                self.current_block_mut()?.terminator = Terminator::Kill;
                self.current_block = None;
            }

            OpTerminateRayKHR => {
                // NV variant is not a terminator.
                if self.current_block.is_none() {
                    spirv_cross_throw!("Trying to end a non-existing block.");
                }
                self.current_block_mut()?.terminator = Terminator::TerminateRay;
                self.current_block = None;
            }

            OpIgnoreIntersectionKHR => {
                // NV variant is not a terminator.
                if self.current_block.is_none() {
                    spirv_cross_throw!("Trying to end a non-existing block.");
                }
                self.current_block_mut()?.terminator = Terminator::IgnoreIntersection;
                self.current_block = None;
            }

            OpEmitMeshTasksEXT => {
                if self.current_block.is_none() {
                    spirv_cross_throw!("Trying to end a non-existing block.");
                }
                let groups = [o!(0), o!(1), o!(2)];
                let payload = if length >= 4 { o!(3) } else { 0 };
                let b = self.current_block_mut()?;
                b.terminator = Terminator::EmitMeshTasks;
                b.mesh.groups = groups;
                b.mesh.payload = payload;
                self.current_block = None;
                // Currently glslang is bugged and does not treat EmitMeshTasksEXT as a terminator.
                self.ignore_trailing_block_opcodes = true;
            }

            OpReturn => {
                if self.current_block.is_none() {
                    spirv_cross_throw!("Trying to end a non-existing block.");
                }
                self.current_block_mut()?.terminator = Terminator::Return;
                self.current_block = None;
            }

            OpReturnValue => {
                if self.current_block.is_none() {
                    spirv_cross_throw!("Trying to end a non-existing block.");
                }
                let v = o!(0);
                let b = self.current_block_mut()?;
                b.terminator = Terminator::Return;
                b.return_value = v;
                self.current_block = None;
            }

            OpUnreachable => {
                if self.current_block.is_none() {
                    spirv_cross_throw!("Trying to end a non-existing block.");
                }
                self.current_block_mut()?.terminator = Terminator::Unreachable;
                self.current_block = None;
            }

            OpSelectionMerge => {
                if self.current_block.is_none() {
                    spirv_cross_throw!("Trying to modify a non-existing block.");
                }

                let w0 = o!(0);
                let w1 = if length >= 2 { Some(o!(1)) } else { None };
                let b = self.current_block_mut()?;
                b.next_block = w0;
                b.merge = Merge::MergeSelection;

                if let Some(w1) = w1 {
                    if w1 & SelectionControlFlattenMask != 0 {
                        b.hint = Hints::HintFlatten;
                    } else if w1 & SelectionControlDontFlattenMask != 0 {
                        b.hint = Hints::HintDontFlatten;
                    }
                }
                *self.block_meta(w0)? |= BLOCK_META_SELECTION_MERGE_BIT;
            }

            OpLoopMerge => {
                if self.current_block.is_none() {
                    spirv_cross_throw!("Trying to modify a non-existing block.");
                }

                let (w0, w1) = (o!(0), o!(1));
                let w2 = if length >= 3 { Some(o!(2)) } else { None };
                let b = self.current_block_mut()?;
                b.merge_block = w0;
                b.continue_block = w1;
                b.merge = Merge::MergeLoop;
                let self_ = b.self_;

                if let Some(w2) = w2 {
                    if w2 & LoopControlUnrollMask != 0 {
                        b.hint = Hints::HintUnroll;
                    } else if w2 & LoopControlDontUnrollMask != 0 {
                        b.hint = Hints::HintDontUnroll;
                    }
                }

                *self.block_meta(self_)? |= BLOCK_META_LOOP_HEADER_BIT;
                *self.block_meta(w0)? |= BLOCK_META_LOOP_MERGE_BIT;

                self.ir.continue_block_to_loop_header.insert(w1, self_);

                // Don't add loop headers to continue blocks,
                // which would make it impossible branch into the loop header since
                // they are treated as continues.
                if w1 != self_ {
                    *self.block_meta(w1)? |= BLOCK_META_CONTINUE_BIT;
                }
            }

            OpSpecConstantOp => {
                if length < 3 {
                    spirv_cross_throw!("OpSpecConstantOp not enough arguments.");
                }

                let result_type = o!(0);
                let id = o!(1);
                let spec_op = o!(2);

                let args = ops.slice(&self.ir.spirv, 3, length - 3)?.to_vec();
                self.set(id, SPIRConstantOp::new(result_type, spec_op, &args))?;
            }

            OpLine => {
                // OpLine might come at global scope, but we don't care about those since they will not be declared in any
                // meaningful correct order.
                // Ignore all OpLine directives which live outside a function.
                if self.current_block.is_some() {
                    self.current_block_mut()?.ops.push(instruction.clone());
                }

                // Line directives may arrive before first OpLabel.
                // Treat this as the line of the function declaration,
                // so warnings for arguments can propagate properly.
                if self.current_function.is_some() {
                    // Store the first one we find and emit it before creating the function prototype.
                    if self.current_function_mut()?.entry_line.file_id == 0 {
                        let (w0, w1) = (o!(0), o!(1));
                        let f = self.current_function_mut()?;
                        f.entry_line.file_id = w0;
                        f.entry_line.line_literal = w1;
                    }
                }

                let file = o!(0);
                let line = o!(1);

                let function_id = self.current_function.unwrap_or(0);
                let block_id = self.current_block.unwrap_or(0);
                for source in self.ir.sources.iter_mut() {
                    if source.file_id == file {
                        source.line_markers.push(Marker {
                            line,
                            col: 0,
                            offset: instruction.offset - 1,
                            function_id,
                            block_id,
                        });
                        break;
                    }
                }
            }

            OpNoLine => {
                // OpNoLine might come at global scope.
                if self.current_block.is_some() {
                    self.current_block_mut()?.ops.push(instruction.clone());
                }
            }

            // Actual opcodes.
            _ => {
                if length >= 2 {
                    if let Some(type_) = self.maybe_get::<SPIRType>(o!(0))? {
                        let width = type_.width;
                        self.ir.load_type_width.entry(o!(1)).or_insert(width);
                    }
                }

                if self.current_block.is_none() {
                    spirv_cross_throw!("Currently no block to insert opcode.");
                }

                self.current_block_mut()?.ops.push(instruction.clone());
            }
        }
        Ok(())
    }

    fn types_are_logically_equivalent(
        &self,
        a: &SPIRType,
        b: &SPIRType,
        depth: u32,
    ) -> Result<bool> {
        // (A struct can name itself as a member in broken SPIR-V, which
        // recurses forever in C++; the translation throws instead.)
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
            if !self.types_are_logically_equivalent(
                self.get::<SPIRType>(a.member_types[i])?,
                self.get::<SPIRType>(b.member_types[i])?,
                depth + 1,
            )? {
                return Ok(false);
            }
        }

        Ok(true)
    }

    pub(crate) fn variable_storage_is_aliased(&self, v: &SPIRVariable) -> Result<bool> {
        let type_ = self.get::<SPIRType>(v.basetype)?;

        let type_meta = self.ir.find_meta(type_.self_);

        let ssbo = v.storage == StorageClassStorageBuffer
            || type_meta
                .map(|m| m.decoration.decoration_flags.get(DecorationBufferBlock))
                .unwrap_or(false);
        let image = type_.basetype == BaseType::Image;
        let counter = type_.basetype == BaseType::AtomicCounter;

        let is_restrict = if ssbo {
            self.ir.get_buffer_block_flags(v)?.get(DecorationRestrict)
        } else {
            self.ir.has_decoration(v.self_, DecorationRestrict)
        };

        Ok(!is_restrict && (ssbo || image || counter))
    }
}
