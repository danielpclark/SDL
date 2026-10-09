// Rust translation of spirv_cross_parsed_ir.hpp and
// spirv_cross_parsed_ir.cpp from SPIRV-Cross.
// Copyright 2018-2021 Arm Limited
// SPDX-License-Identifier: Apache-2.0 OR MIT
// This is an altered (translated to Rust) version of the original
// software; see LICENSE.txt.

//! `ParsedIR`: this data structure holds all information needed to
//! perform cross-compilation and reflection. It is the output of the
//! Parser, but any implementation could create this structure. It is
//! intentionally very "open" and struct-like with some helper functions
//! to deal with decorations. Parser is the reference implementation of
//! how this data structure should be filled in.

use std::collections::{HashMap, HashSet};

use super::common::*;
use super::spirv::*;
use super::std_hash::StdHashMap;

/// `ParsedIR::Source::Marker`.
#[derive(Clone, Copy, Debug, Default)]
pub struct Marker {
    pub line: ID,        // in source
    pub col: ID,         // in source
    pub offset: ID,      // in spirv stream
    pub function_id: ID, //
    pub block_id: ID,
}

/// `ParsedIR::Source`.
#[derive(Clone, Debug)]
pub struct Source {
    pub lang: SourceLanguage,
    pub version: u32,
    pub es: bool,
    pub known: bool,
    pub hlsl: bool,

    pub file_id: ID,   // string
    pub define_id: ID, // only non-zero for DebugSource
    pub source: String,

    pub line_markers: Vec<Marker>, // sorted by line
}

impl Default for Source {
    fn default() -> Self {
        Source {
            lang: SourceLanguageUnknown,
            version: 0,
            es: false,
            known: false,
            hlsl: false,
            file_id: 0,
            define_id: 0,
            source: String::new(),
            line_markers: Vec::new(),
        }
    }
}

// Meta data about blocks. The cross-compiler needs to query if a block is either of these types.
// It is a bitset as there can be more than one tag per block.
pub const BLOCK_META_LOOP_HEADER_BIT: u8 = 1 << 0;
pub const BLOCK_META_CONTINUE_BIT: u8 = 1 << 1;
pub const BLOCK_META_LOOP_MERGE_BIT: u8 = 1 << 2;
pub const BLOCK_META_SELECTION_MERGE_BIT: u8 = 1 << 3;
pub const BLOCK_META_MULTISELECT_MERGE_BIT: u8 = 1 << 4;
pub type BlockMetaFlags = u8;

/// `ParsedIR`.
#[derive(Clone)]
pub struct ParsedIR {
    // The raw SPIR-V, instructions and opcodes refer to this by offset + count.
    pub spirv: Vec<u32>,

    // Holds various data structures which inherit from IVariant.
    pub ids: Vec<Variant>,

    // Various meta data for IDs, decorations, names, etc.
    pub meta: HashMap<ID, Meta>,

    // Holds all IDs which have a certain type.
    // This is needed so we can iterate through a specific kind of resource quickly,
    // and in-order of module declaration.
    pub ids_for_type: [Vec<ID>; TYPE_COUNT],

    // Special purpose lists which contain a union of types.
    // This is needed so we can declare specialization constants and structs in an interleaved fashion,
    // among other things.
    // Constants can be undef or of struct type, and struct array sizes can use specialization constants.
    pub ids_for_constant_undef_or_type: Vec<ID>,
    pub ids_for_constant_or_variable: Vec<ID>,

    // We need to keep track of the width the Ops that contains a type for the
    // OpSwitch instruction, since this one doesn't contains the type in the
    // instruction itself. And in some case we need to cast the condition to
    // wider types. We only need the width to do the branch fixup since the
    // type check itself can be done at runtime
    pub load_type_width: HashMap<ID, u32>,

    // Declared capabilities and extensions in the SPIR-V module.
    // Not really used except for reflection at the moment.
    pub declared_capabilities: Vec<Capability>,
    pub declared_extensions: Vec<String>,

    pub block_meta: Vec<BlockMetaFlags>,
    pub continue_block_to_loop_header: HashMap<BlockID, BlockID>,

    // Normally, we'd stick SPIREntryPoint in ids array, but it conflicts with SPIRFunction.
    // Entry points can therefore be seen as some sort of meta structure.
    pub entry_points: EntryPoints,
    pub default_entry_point: FunctionID,

    pub sources: Vec<Source>,

    pub addressing_model: AddressingModel,
    pub memory_model: MemoryModel,

    pub(crate) loop_iteration_depth_hard: u32,
    pub(crate) loop_iteration_depth_soft: u32,
    empty_string: String,
    cleared_bitset: Bitset,

    meta_needing_name_fixup: HashSet<u32>,
}

/// `std::unordered_map<FunctionID, SPIREntryPoint>`, iterated in the order
/// libstdc++ iterates it (SPIRV-Cross's output depends on it with several
/// entry points): its unique-key hash table with the identity hash of
/// integers, emulated.
#[derive(Clone, Debug, Default)]
pub struct EntryPoints {
    table: StdHashMap<SPIREntryPoint>,
}

impl EntryPoints {
    pub fn get(&self, id: &FunctionID) -> Option<&SPIREntryPoint> {
        self.table.get(id)
    }

    pub fn get_mut(&mut self, id: &FunctionID) -> Option<&mut SPIREntryPoint> {
        self.table.get_mut(id)
    }

    pub fn contains_key(&self, id: &FunctionID) -> bool {
        self.table.get(id).is_some()
    }

    pub fn len(&self) -> usize {
        self.table.len()
    }

    pub fn is_empty(&self) -> bool {
        self.table.len() == 0
    }

    /// `operator[]`: inserts a default entry point when missing.
    pub fn entry(&mut self, id: FunctionID) -> &mut SPIREntryPoint {
        self.table.entry_or_default(id)
    }

    pub fn insert(&mut self, id: FunctionID, ep: SPIREntryPoint) {
        self.table.insert(id, ep);
    }

    /// The keys in libstdc++'s iteration order.
    pub fn keys(&self) -> Vec<FunctionID> {
        self.table.keys()
    }

    pub fn iter(&self) -> impl Iterator<Item = (FunctionID, &SPIREntryPoint)> {
        self.table.iter()
    }
}

impl Default for ParsedIR {
    fn default() -> Self {
        Self::new()
    }
}

// Roll our own versions of these functions to avoid potential locale shenanigans.
fn is_alpha(c: char) -> bool {
    c.is_ascii_lowercase() || c.is_ascii_uppercase()
}

fn is_numeric(c: char) -> bool {
    c.is_ascii_digit()
}

fn is_alphanumeric(c: char) -> bool {
    is_alpha(c) || is_numeric(c)
}

fn is_valid_identifier(name: &str) -> bool {
    if name.is_empty() {
        return true;
    }

    if is_numeric(name.chars().next().unwrap_or('\0')) {
        return false;
    }

    for c in name.chars() {
        if !is_alphanumeric(c) && c != '_' {
            return false;
        }
    }

    let mut saw_underscore = false;
    // Two underscores in a row is not a valid identifier either.
    // Technically reserved, but it's easier to treat it as invalid.
    for c in name.chars() {
        let is_underscore = c == '_';
        if is_underscore && saw_underscore {
            return false;
        }
        saw_underscore = is_underscore;
    }

    true
}

fn is_reserved_prefix(name: &str) -> bool {
    // Generic reserved identifiers used by the implementation.
    name.starts_with("gl_")
        // Ignore this case for now, might rewrite internal code to always use spv prefix.
        //name.compare(0, 11, "SPIRV_Cross", 11) == 0 ||
        || name.starts_with("spv")
}

fn is_reserved_identifier(name: &str, member: bool, allow_reserved_prefixes: bool) -> bool {
    if !allow_reserved_prefixes && is_reserved_prefix(name) {
        return true;
    }

    let b = name.as_bytes();
    if member {
        // Reserved member identifiers come in one form:
        // _m[0-9]+$.
        if b.len() < 3 {
            return false;
        }

        if !name.starts_with("_m") {
            return false;
        }

        let mut index = 2;
        while index < b.len() && b[index].is_ascii_digit() {
            index += 1;
        }

        index == b.len()
    } else {
        // Reserved non-member identifiers come in two forms:
        // _[0-9]+$, used for temporaries which map directly to a SPIR-V ID.
        // _[0-9]+_, used for auxillary temporaries which derived from a SPIR-V ID.
        if b.len() < 2 {
            return false;
        }

        if b[0] != b'_' || !b[1].is_ascii_digit() {
            return false;
        }

        let mut index = 2;
        while index < b.len() && b[index].is_ascii_digit() {
            index += 1;
        }

        index == b.len() || (index < b.len() && b[index] == b'_')
    }
}

fn make_unreserved_identifier(name: &str) -> String {
    if is_reserved_prefix(name) {
        "_RESERVED_IDENTIFIER_FIXUP_".to_string() + name
    } else {
        "_RESERVED_IDENTIFIER_FIXUP".to_string() + name
    }
}

fn ensure_valid_identifier(name: &str) -> String {
    // Functions in glslangValidator are mangled with name(<mangled> stuff.
    // Normally, we would never see '(' in any legal identifiers, so just strip them out.
    let str = match name.find('(') {
        Some(p) => &name[..p],
        None => name,
    };

    if str.is_empty() {
        return String::new();
    }

    // (Each character that isn't valid becomes one '_'; C++ replaces each
    // byte of a multi-byte UTF-8 character, which sanitize_underscores()
    // then compacts to the same single '_'.)
    let mut out = String::with_capacity(str.len());
    for (i, c) in str.chars().enumerate() {
        if i == 0 && is_numeric(c) {
            out.push('_');
        } else if !is_alphanumeric(c) && c != '_' {
            out.push('_');
        } else {
            out.push(c);
        }
    }

    ParsedIR::sanitize_underscores(&mut out);
    out
}

impl ParsedIR {
    pub fn new() -> Self {
        ParsedIR {
            spirv: Vec::new(),
            ids: Vec::new(),
            meta: HashMap::new(),
            ids_for_type: Default::default(),
            ids_for_constant_undef_or_type: Vec::new(),
            ids_for_constant_or_variable: Vec::new(),
            load_type_width: HashMap::new(),
            declared_capabilities: Vec::new(),
            declared_extensions: Vec::new(),
            block_meta: Vec::new(),
            continue_block_to_loop_header: HashMap::new(),
            entry_points: EntryPoints::default(),
            default_entry_point: 0,
            sources: Vec::new(),
            addressing_model: AddressingModelMax,
            memory_model: MemoryModelMax,
            loop_iteration_depth_hard: 0,
            loop_iteration_depth_soft: 0,
            empty_string: String::new(),
            cleared_bitset: Bitset::default(),
            meta_needing_name_fixup: HashSet::new(),
        }
    }

    /// Resizes ids, meta and block_meta.
    pub fn set_id_bounds(&mut self, bounds: u32) -> Result<()> {
        try_resize(&mut self.ids, bounds as usize)?;
        try_resize(&mut self.block_meta, bounds as usize)
    }

    pub fn is_globally_reserved_identifier(str: &str, allow_reserved_prefixes: bool) -> bool {
        is_reserved_identifier(str, false, allow_reserved_prefixes)
    }

    pub fn get_spirv_version(&self) -> u32 {
        self.spirv.get(1).copied().unwrap_or(0)
    }

    pub fn sanitize_underscores(str: &mut String) {
        // Compact adjacent underscores to make it valid.
        let mut out = String::with_capacity(str.len());
        let mut saw_underscore = false;
        for c in str.chars() {
            let is_underscore = c == '_';
            if saw_underscore && is_underscore {
                continue;
            }
            out.push(c);
            saw_underscore = is_underscore;
        }
        *str = out;
    }

    pub fn get_name(&self, id: ID) -> &String {
        match self.find_meta(id) {
            Some(m) => &m.decoration.alias,
            None => &self.empty_string,
        }
    }

    pub fn get_member_name(&self, id: TypeID, index: u32) -> &String {
        match self.find_meta(id) {
            Some(m) => {
                if index as usize >= m.members.len() {
                    return &self.empty_string;
                }
                &m.members[index as usize].alias
            }
            None => &self.empty_string,
        }
    }

    pub fn sanitize_identifier(name: &mut String, member: bool, allow_reserved_prefixes: bool) {
        if !is_valid_identifier(name) {
            *name = ensure_valid_identifier(name);
        }
        if is_reserved_identifier(name, member, allow_reserved_prefixes) {
            *name = make_unreserved_identifier(name);
        }
    }

    pub fn fixup_reserved_names(&mut self) {
        let ids: Vec<u32> = self.meta_needing_name_fixup.iter().copied().collect();
        for id in ids {
            // Don't rename remapped variables like 'gl_LastFragDepthARM'.
            if let Some(v) = self.ids.get(id as usize) {
                if v.get_type() == TypeVariable {
                    if let Ok(var) = v.get::<SPIRVariable>() {
                        if var.remapped_variable {
                            continue;
                        }
                    }
                }
            }

            let m = self.meta.entry(id).or_default();
            Self::sanitize_identifier(&mut m.decoration.alias, false, false);
            for memb in m.members.iter_mut() {
                Self::sanitize_identifier(&mut memb.alias, true, false);
            }
        }
        self.meta_needing_name_fixup.clear();
    }

    pub fn set_name(&mut self, id: ID, name: &str) {
        let m = self.meta.entry(id).or_default();
        m.decoration.alias = name.to_string();
        if !is_valid_identifier(name) || is_reserved_identifier(name, false, false) {
            self.meta_needing_name_fixup.insert(id);
        }
    }

    pub fn set_member_name(&mut self, id: TypeID, index: u32, name: &str) -> Result<()> {
        let m = self.meta.entry(id).or_default();
        let len = m.members.len().max(index as usize + 1);
        try_resize(&mut m.members, len)?;
        m.members[index as usize].alias = name.to_string();
        if !is_valid_identifier(name) || is_reserved_identifier(name, true, false) {
            self.meta_needing_name_fixup.insert(id);
        }
        Ok(())
    }

    pub fn set_decoration_string(&mut self, id: ID, decoration: Decoration, argument: &str) {
        let dec = &mut self.meta.entry(id).or_default().decoration;
        dec.decoration_flags.set(decoration);

        match decoration {
            DecorationUserSemantic => dec.user_semantic = argument.to_string(),
            DecorationUserTypeGOOGLE => dec.user_type = argument.to_string(),
            _ => {}
        }
    }

    pub fn set_decoration(&mut self, id: ID, decoration: Decoration, argument: u32) {
        let dec = &mut self.meta.entry(id).or_default().decoration;
        dec.decoration_flags.set(decoration);

        match decoration {
            DecorationBuiltIn => {
                dec.builtin = true;
                dec.builtin_type = argument;
            }
            DecorationLocation => dec.location = argument,
            DecorationComponent => dec.component = argument,
            DecorationOffset => dec.offset = argument,
            DecorationOffsetIdEXT => dec.offset_id = argument,
            DecorationXfbBuffer => dec.xfb_buffer = argument,
            DecorationXfbStride => dec.xfb_stride = argument,
            DecorationStream => dec.stream = argument,
            DecorationArrayStride => dec.array_stride = argument,
            DecorationArrayStrideIdEXT => dec.array_stride_id = argument,
            DecorationMatrixStride => dec.matrix_stride = argument,
            DecorationBinding => dec.binding = argument,
            DecorationDescriptorSet => dec.set = argument,
            DecorationInputAttachmentIndex => dec.input_attachment = argument,
            DecorationSpecId => dec.spec_id = argument,
            DecorationIndex => dec.index = argument,
            DecorationHlslCounterBufferGOOGLE => {
                self.meta.entry(id).or_default().hlsl_magic_counter_buffer = argument;
                self.meta
                    .entry(argument)
                    .or_default()
                    .hlsl_is_magic_counter_buffer = true;
            }
            DecorationFPRoundingMode => dec.fp_rounding_mode = argument,
            DecorationFPFastMathMode => dec.fp_fast_math_mode = argument,
            _ => {}
        }
    }

    pub fn set_member_decoration(
        &mut self,
        id: TypeID,
        index: u32,
        decoration: Decoration,
        argument: u32,
    ) -> Result<()> {
        let m = self.meta.entry(id).or_default();
        let len = m.members.len().max(index as usize + 1);
        try_resize(&mut m.members, len)?;
        let dec = &mut m.members[index as usize];
        dec.decoration_flags.set(decoration);

        match decoration {
            DecorationBuiltIn => {
                dec.builtin = true;
                dec.builtin_type = argument;
            }
            DecorationLocation => dec.location = argument,
            DecorationComponent => dec.component = argument,
            DecorationBinding => dec.binding = argument,
            DecorationOffset => dec.offset = argument,
            DecorationOffsetIdEXT => dec.offset_id = argument,
            DecorationXfbBuffer => dec.xfb_buffer = argument,
            DecorationXfbStride => dec.xfb_stride = argument,
            DecorationStream => dec.stream = argument,
            DecorationSpecId => dec.spec_id = argument,
            DecorationMatrixStride => dec.matrix_stride = argument,
            DecorationIndex => dec.index = argument,
            _ => {}
        }
        Ok(())
    }

    // Recursively marks any constants referenced by the specified constant instruction as being used
    // as an array length. The id must be a constant instruction (SPIRConstant or SPIRConstantOp).
    pub fn mark_used_as_array_length(&mut self, id: ID) -> Result<()> {
        let ty = self.ids.at(id)?.get_type();
        match ty {
            TypeConstant => {
                let c = self.get_mut::<SPIRConstant>(id)?;
                c.is_used_as_array_length = true;
                let c = c.clone();

                // Mark composite dependencies as well.
                for &sub_id in c.m.id.iter() {
                    if sub_id != 0 {
                        self.mark_used_as_array_length(sub_id)?;
                    }
                }

                for col in 0..c.m.columns {
                    for &sub_id in c.m.c.at(col)?.id.iter() {
                        if sub_id != 0 {
                            self.mark_used_as_array_length(sub_id)?;
                        }
                    }
                }

                for &sub_id in c.subconstants.iter() {
                    if sub_id != 0 {
                        self.mark_used_as_array_length(sub_id)?;
                    }
                }
            }

            TypeConstantOp => {
                let cop = self.get::<SPIRConstantOp>(id)?.clone();
                if cop.opcode == OpCompositeExtract {
                    self.mark_used_as_array_length(*cop.arguments.at(0)?)?;
                } else if cop.opcode == OpCompositeInsert {
                    self.mark_used_as_array_length(*cop.arguments.at(0)?)?;
                    self.mark_used_as_array_length(*cop.arguments.at(1)?)?;
                } else {
                    for &arg_id in cop.arguments.iter() {
                        self.mark_used_as_array_length(arg_id)?;
                    }
                }
            }

            TypeUndef => {}

            _ => {
                // assert(0);
            }
        }
        Ok(())
    }

    pub fn get_buffer_block_type_flags(&self, type_: &SPIRType) -> Bitset {
        if type_.member_types.is_empty() {
            return Bitset::default();
        }

        let mut all_members_flags = self.get_member_decoration_bitset(type_.self_, 0).clone();
        for i in 1..type_.member_types.len() as u32 {
            all_members_flags.merge_and(self.get_member_decoration_bitset(type_.self_, i));
        }
        all_members_flags
    }

    pub fn get_buffer_block_flags(&self, var: &SPIRVariable) -> Result<Bitset> {
        let type_ = self.get::<SPIRType>(var.basetype)?;
        if type_.basetype != BaseType::Struct {
            spirv_cross_throw!("Cannot get buffer block flags for non-buffer variable.");
        }

        // Some flags like non-writable, non-readable are actually found
        // as member decorations. If all members have a decoration set, propagate
        // the decoration up as a regular variable decoration.
        let mut base_flags = Bitset::default();
        if let Some(m) = self.find_meta(var.self_) {
            base_flags = m.decoration.decoration_flags.clone();
        }

        if type_.member_types.is_empty() {
            return Ok(base_flags);
        }

        let all_members_flags = self.get_buffer_block_type_flags(type_);
        base_flags.merge_or(&all_members_flags);
        Ok(base_flags)
    }

    pub fn get_member_decoration_bitset(&self, id: TypeID, index: u32) -> &Bitset {
        match self.find_meta(id) {
            Some(m) => {
                if index as usize >= m.members.len() {
                    return &self.cleared_bitset;
                }
                &m.members[index as usize].decoration_flags
            }
            None => &self.cleared_bitset,
        }
    }

    pub fn has_decoration(&self, id: ID, decoration: Decoration) -> bool {
        self.get_decoration_bitset(id).get(decoration)
    }

    pub fn get_decoration(&self, id: ID, decoration: Decoration) -> u32 {
        let Some(m) = self.find_meta(id) else {
            return 0;
        };

        let dec = &m.decoration;
        if !dec.decoration_flags.get(decoration) {
            return 0;
        }

        match decoration {
            DecorationBuiltIn => dec.builtin_type,
            DecorationLocation => dec.location,
            DecorationComponent => dec.component,
            DecorationOffset => dec.offset,
            DecorationOffsetIdEXT => dec.offset_id,
            DecorationXfbBuffer => dec.xfb_buffer,
            DecorationXfbStride => dec.xfb_stride,
            DecorationStream => dec.stream,
            DecorationBinding => dec.binding,
            DecorationDescriptorSet => dec.set,
            DecorationInputAttachmentIndex => dec.input_attachment,
            DecorationSpecId => dec.spec_id,
            DecorationArrayStride => dec.array_stride,
            DecorationArrayStrideIdEXT => dec.array_stride_id,
            DecorationMatrixStride => dec.matrix_stride,
            DecorationIndex => dec.index,
            DecorationFPRoundingMode => dec.fp_rounding_mode,
            DecorationFPFastMathMode => dec.fp_fast_math_mode,
            _ => 1,
        }
    }

    pub fn get_decoration_string(&self, id: ID, decoration: Decoration) -> &String {
        let Some(m) = self.find_meta(id) else {
            return &self.empty_string;
        };

        let dec = &m.decoration;

        if !dec.decoration_flags.get(decoration) {
            return &self.empty_string;
        }

        match decoration {
            DecorationUserSemantic => &dec.user_semantic,
            DecorationUserTypeGOOGLE => &dec.user_type,
            _ => &self.empty_string,
        }
    }

    pub fn unset_decoration(&mut self, id: ID, decoration: Decoration) {
        let dec = &mut self.meta.entry(id).or_default().decoration;
        dec.decoration_flags.clear(decoration);
        match decoration {
            DecorationBuiltIn => dec.builtin = false,
            DecorationLocation => dec.location = 0,
            DecorationComponent => dec.component = 0,
            DecorationOffset => dec.offset = 0,
            DecorationOffsetIdEXT => dec.offset_id = 0,
            DecorationXfbBuffer => dec.xfb_buffer = 0,
            DecorationXfbStride => dec.xfb_stride = 0,
            DecorationStream => dec.stream = 0,
            DecorationBinding => dec.binding = 0,
            DecorationDescriptorSet => dec.set = 0,
            DecorationInputAttachmentIndex => dec.input_attachment = 0,
            DecorationSpecId => dec.spec_id = 0,
            DecorationUserSemantic => dec.user_semantic.clear(),
            DecorationFPRoundingMode => dec.fp_rounding_mode = FPRoundingModeMax,
            DecorationFPFastMathMode => dec.fp_fast_math_mode = FPFastMathModeMaskNone,
            DecorationHlslCounterBufferGOOGLE => {
                let counter = self.meta.entry(id).or_default().hlsl_magic_counter_buffer;
                if counter != 0 {
                    self.meta
                        .entry(counter)
                        .or_default()
                        .hlsl_is_magic_counter_buffer = false;
                    self.meta.entry(id).or_default().hlsl_magic_counter_buffer = 0;
                }
            }
            _ => {}
        }
    }

    pub fn has_member_decoration(&self, id: TypeID, index: u32, decoration: Decoration) -> bool {
        self.get_member_decoration_bitset(id, index).get(decoration)
    }

    pub fn get_member_decoration(&self, id: TypeID, index: u32, decoration: Decoration) -> u32 {
        let Some(m) = self.find_meta(id) else {
            return 0;
        };

        if index as usize >= m.members.len() {
            return 0;
        }

        let dec = &m.members[index as usize];
        if !dec.decoration_flags.get(decoration) {
            return 0;
        }

        match decoration {
            DecorationBuiltIn => dec.builtin_type,
            DecorationLocation => dec.location,
            DecorationComponent => dec.component,
            DecorationBinding => dec.binding,
            DecorationOffset => dec.offset,
            DecorationOffsetIdEXT => dec.offset_id,
            DecorationXfbBuffer => dec.xfb_buffer,
            DecorationXfbStride => dec.xfb_stride,
            DecorationStream => dec.stream,
            DecorationSpecId => dec.spec_id,
            DecorationMatrixStride => dec.matrix_stride,
            DecorationIndex => dec.index,
            _ => 1,
        }
    }

    pub fn get_decoration_bitset(&self, id: ID) -> &Bitset {
        match self.find_meta(id) {
            Some(m) => &m.decoration.decoration_flags,
            None => &self.cleared_bitset,
        }
    }

    pub fn set_member_decoration_string(
        &mut self,
        id: TypeID,
        index: u32,
        decoration: Decoration,
        argument: &str,
    ) -> Result<()> {
        let m = self.meta.entry(id).or_default();
        let len = m.members.len().max(index as usize + 1);
        try_resize(&mut m.members, len)?;
        let dec = &mut m.members[index as usize];
        dec.decoration_flags.set(decoration);

        if decoration == DecorationUserSemantic {
            dec.user_semantic = argument.to_string();
        }
        Ok(())
    }

    pub fn get_member_decoration_string(
        &self,
        id: TypeID,
        index: u32,
        decoration: Decoration,
    ) -> &String {
        match self.find_meta(id) {
            Some(m) => {
                if !self.has_member_decoration(id, index, decoration) {
                    return &self.empty_string;
                }

                let dec = &m.members[index as usize];

                match decoration {
                    DecorationUserSemantic => &dec.user_semantic,
                    _ => &self.empty_string,
                }
            }
            None => &self.empty_string,
        }
    }

    pub fn unset_member_decoration(&mut self, id: TypeID, index: u32, decoration: Decoration) {
        let m = self.meta.entry(id).or_default();
        if index as usize >= m.members.len() {
            return;
        }

        let dec = &mut m.members[index as usize];

        dec.decoration_flags.clear(decoration);
        match decoration {
            DecorationBuiltIn => dec.builtin = false,
            DecorationLocation => dec.location = 0,
            DecorationComponent => dec.component = 0,
            DecorationOffset => dec.offset = 0,
            DecorationOffsetIdEXT => dec.offset_id = 0,
            DecorationXfbBuffer => dec.xfb_buffer = 0,
            DecorationXfbStride => dec.xfb_stride = 0,
            DecorationStream => dec.stream = 0,
            DecorationSpecId => dec.spec_id = 0,
            DecorationUserSemantic => dec.user_semantic.clear(),
            _ => {}
        }
    }

    pub fn increase_bound_by(&mut self, incr_amount: u32) -> Result<u32> {
        let curr_bound = self.ids.len();
        let new_bound = curr_bound + incr_amount as usize;

        try_resize(&mut self.ids, new_bound)?;
        try_resize(&mut self.block_meta, new_bound)?;
        Ok(curr_bound as u32)
    }

    pub fn remove_typed_id(&mut self, type_: Types, id: ID) {
        self.ids_for_type[type_ as usize].retain(|&x| x != id);
    }

    pub fn reset_all_of_type(&mut self, type_: Types) {
        let list = std::mem::take(&mut self.ids_for_type[type_ as usize]);
        for id in list {
            if let Some(v) = self.ids.get_mut(id as usize) {
                if v.get_type() == type_ {
                    v.reset();
                }
            }
        }
    }

    pub fn add_typed_id(&mut self, type_: Types, id: ID) -> Result<()> {
        // assert(id < ids.size());
        if id as usize >= self.ids.len() {
            spirv_cross_throw!("ID out of range.");
        }

        if self.loop_iteration_depth_hard != 0 {
            spirv_cross_throw!("Cannot add typed ID while looping over it.");
        }

        if self.loop_iteration_depth_soft != 0 {
            if !self.ids[id as usize].empty() {
                spirv_cross_throw!("Cannot override IDs when loop is soft locked.");
            }
            return Ok(());
        }

        let cur = &self.ids[id as usize];
        if cur.empty() || cur.get_type() != type_ {
            match type_ {
                TypeConstant => {
                    self.ids_for_constant_or_variable.push(id);
                    self.ids_for_constant_undef_or_type.push(id);
                }

                TypeVariable => self.ids_for_constant_or_variable.push(id),

                TypeType | TypeConstantOp | TypeUndef => {
                    self.ids_for_constant_undef_or_type.push(id)
                }

                _ => {}
            }
        }

        let cur = &self.ids[id as usize];
        if cur.empty() {
            self.ids_for_type[type_ as usize].push(id);
        } else if cur.get_type() != type_ {
            let old = cur.get_type();
            self.remove_typed_id(old, id);
            self.ids_for_type[type_ as usize].push(id);
        }
        Ok(())
    }

    pub fn find_meta(&self, id: ID) -> Option<&Meta> {
        self.meta.get(&id)
    }

    pub fn find_meta_mut(&mut self, id: ID) -> Option<&mut Meta> {
        self.meta.get_mut(&id)
    }

    pub fn get_empty_string(&self) -> &String {
        &self.empty_string
    }

    pub fn make_constant_null(
        &mut self,
        id: u32,
        type_: u32,
        add_to_typed_id_set: bool,
    ) -> Result<()> {
        // assert(id < ids.size());
        if id as usize >= self.ids.len() {
            spirv_cross_throw!("ID out of range.");
        }

        let constant_type = self.get::<SPIRType>(type_)?.clone();

        if constant_type.pointer {
            if add_to_typed_id_set {
                self.add_typed_id(TypeConstant, id)?;
            }
            let constant = variant_set(&mut self.ids[id as usize], SPIRConstant::new_typed(type_))?;
            constant.self_ = id;
            constant.make_null(&constant_type);
        } else if !constant_type.array.is_empty() {
            // assert(constant_type.parent_type);
            let parent_id = self.increase_bound_by(1)?;
            self.make_constant_null(parent_id, constant_type.parent_type, add_to_typed_id_set)?;

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
                .map_err(|_| CompilerError::new("Out of memory."))?;
            elements.resize(count as usize, parent_id);

            if add_to_typed_id_set {
                self.add_typed_id(TypeConstant, id)?;
            }
            let constant = variant_set(
                &mut self.ids[id as usize],
                SPIRConstant::new_composite(type_, &elements, false, false),
            )?;
            constant.self_ = id;
            constant.is_null_array_specialized_length = !is_literal_array_size;
        } else if !constant_type.member_types.is_empty() {
            let member_ids = self.increase_bound_by(constant_type.member_types.len() as u32)?;
            let mut elements = vec![0u32; constant_type.member_types.len()];
            for i in 0..constant_type.member_types.len() {
                self.make_constant_null(
                    member_ids + i as u32,
                    constant_type.member_types[i],
                    add_to_typed_id_set,
                )?;
                elements[i] = member_ids + i as u32;
            }

            if add_to_typed_id_set {
                self.add_typed_id(TypeConstant, id)?;
            }
            variant_set(
                &mut self.ids[id as usize],
                SPIRConstant::new_composite(type_, &elements, false, false),
            )?
            .self_ = id;
        } else {
            if add_to_typed_id_set {
                self.add_typed_id(TypeConstant, id)?;
            }
            let constant = variant_set(&mut self.ids[id as usize], SPIRConstant::new_typed(type_))?;
            constant.self_ = id;
            constant.make_null(&constant_type);
        }
        Ok(())
    }

    /// `get<T>(id)`.
    #[inline]
    pub fn get<T: IVariant>(&self, id: u32) -> Result<&T> {
        match self.ids.get(id as usize) {
            Some(v) => v.get::<T>(),
            None => spirv_cross_throw!("nullptr"),
        }
    }

    /// `get<T>(id)` for writing.
    #[inline]
    pub fn get_mut<T: IVariant>(&mut self, id: u32) -> Result<&mut T> {
        match self.ids.get_mut(id as usize) {
            Some(v) => v.get_mut::<T>(),
            None => spirv_cross_throw!("nullptr"),
        }
    }

    /// `get<T>(id)`, as a snapshot that outlives the borrow of the IR.
    #[inline]
    pub fn get_rc<T: IVariant>(&self, id: u32) -> Result<std::rc::Rc<T>> {
        match self.ids.get(id as usize) {
            Some(v) => v.get_rc::<T>(),
            None => spirv_cross_throw!("nullptr"),
        }
    }

    /// The type of the variant at `id` (`ids[id].get_type()`), `TypeNone`
    /// when out of range.
    #[inline]
    pub fn id_type(&self, id: u32) -> Types {
        match self.ids.get(id as usize) {
            Some(v) => v.get_type(),
            None => TypeNone,
        }
    }

    /// `ids_for_type[T::type]` with the hard loop lock taken
    /// (`for_each_typed_id`): the ids of type `T` in declaration order.
    pub fn typed_ids(&self, type_: Types) -> Vec<ID> {
        self.ids_for_type[type_ as usize]
            .iter()
            .copied()
            .filter(|&id| self.id_type(id) == type_)
            .collect()
    }
}
