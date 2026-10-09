// Rust translation of spirv_common.hpp (with spirv_cross_containers.hpp
// and spirv_cross_error_handling.hpp) from SPIRV-Cross.
// Copyright 2015-2021 Arm Limited
// SPDX-License-Identifier: Apache-2.0 OR MIT
// This is an altered (translated to Rust) version of the original
// software; see LICENSE.txt.

//! The IR data structures shared by the parser, the reflection and the
//! backends (`spirv_common.hpp`), the error type
//! (`spirv_cross_error_handling.hpp`) and the string helpers of
//! `spirv_cross_containers.hpp`.
//!
//! The containers (`SmallVector`, `StringStream`, the object pools) are
//! Rust's own. The `Variant` slots hold their object behind an `Rc`, so a
//! caller can keep a snapshot of an object (C++ keeps a reference) across
//! calls that need the whole compiler mutably; mutation goes through
//! `Rc::make_mut`.

use std::collections::{BTreeSet, HashMap, HashSet};
use std::fmt::Write as _;
use std::rc::Rc;

use super::cross::Compiler;
use super::spirv::*;

// spirv_cross_error_handling.hpp

/// `CompilerError`: what SPIRV-Cross throws (`SPIRV_CROSS_THROW`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CompilerError(pub String);

impl CompilerError {
    pub fn new(msg: impl Into<String>) -> Self {
        CompilerError(msg.into())
    }
}

impl std::fmt::Display for CompilerError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for CompilerError {}

pub type Result<T> = std::result::Result<T, CompilerError>;

/// `SPIRV_CROSS_THROW(x)`.
macro_rules! spirv_cross_throw {
    ($($arg:tt)*) => {
        return Err($crate::spirv_cross::common::CompilerError::new($($arg)*))
    };
}
pub(crate) use spirv_cross_throw;

/// Where C++ indexes a container with an index from the input (UB when out
/// of range), the translation checks and throws instead.
pub(crate) trait At<T> {
    fn at(&self, i: impl TryInto<usize>) -> Result<&T>;
    fn at_mut(&mut self, i: impl TryInto<usize>) -> Result<&mut T>;
}

impl<T> At<T> for [T] {
    #[inline]
    fn at(&self, i: impl TryInto<usize>) -> Result<&T> {
        match i.try_into() {
            Ok(i) if i < self.len() => Ok(&self[i]),
            _ => Err(CompilerError::new("Index out of range.")),
        }
    }

    #[inline]
    fn at_mut(&mut self, i: impl TryInto<usize>) -> Result<&mut T> {
        match i.try_into() {
            Ok(i) if i < self.len() => Ok(&mut self[i]),
            _ => Err(CompilerError::new("Index out of range.")),
        }
    }
}

/// `std::vector::back()` (UB on an empty vector in C++).
pub(crate) trait Back<T> {
    fn back(&self) -> Result<&T>;
    fn back_mut(&mut self) -> Result<&mut T>;
}

impl<T> Back<T> for Vec<T> {
    #[inline]
    fn back(&self) -> Result<&T> {
        self.last()
            .ok_or_else(|| CompilerError::new("Index out of range."))
    }

    #[inline]
    fn back_mut(&mut self) -> Result<&mut T> {
        self.last_mut()
            .ok_or_else(|| CompilerError::new("Index out of range."))
    }
}

/// Grows a vector to `len` default elements, failing instead of aborting
/// when the size (which comes from the input) can't be allocated.
pub(crate) fn try_resize<T: Default + Clone>(v: &mut Vec<T>, len: usize) -> Result<()> {
    if len > v.len() {
        v.try_reserve(len - v.len())
            .map_err(|_| CompilerError::new("Out of memory."))?;
    }
    v.resize(len, T::default());
    Ok(())
}

// Bitset

/// `Bitset`: decoration and flag bits, the low 64 inline.
#[derive(Clone, Debug, Default)]
pub struct Bitset {
    // The most common bits to set are all lower than 64,
    // so optimize for this case. Bits spilling outside 64 go into a slower data structure.
    // In almost all cases, higher data structure will not be used.
    lower: u64,
    higher: BTreeSet<u32>,
}

impl Bitset {
    pub fn new(lower: u64) -> Self {
        Bitset {
            lower,
            higher: BTreeSet::new(),
        }
    }

    #[inline]
    pub fn get(&self, bit: u32) -> bool {
        if bit < 64 {
            (self.lower & (1u64 << bit)) != 0
        } else {
            self.higher.contains(&bit)
        }
    }

    #[inline]
    pub fn set(&mut self, bit: u32) {
        if bit < 64 {
            self.lower |= 1u64 << bit;
        } else {
            self.higher.insert(bit);
        }
    }

    #[inline]
    pub fn clear(&mut self, bit: u32) {
        if bit < 64 {
            self.lower &= !(1u64 << bit);
        } else {
            self.higher.remove(&bit);
        }
    }

    pub fn get_lower(&self) -> u64 {
        self.lower
    }

    pub fn reset(&mut self) {
        self.lower = 0;
        self.higher.clear();
    }

    pub fn merge_and(&mut self, other: &Bitset) {
        self.lower &= other.lower;
        self.higher = self.higher.intersection(&other.higher).copied().collect();
    }

    pub fn merge_or(&mut self, other: &Bitset) {
        self.lower |= other.lower;
        for v in &other.higher {
            self.higher.insert(*v);
        }
    }

    /// `for_each_bit()`: the bits in increasing order.
    pub fn bits(&self) -> Vec<u32> {
        // TODO: Add ctz-based iteration.
        let mut out = Vec::new();
        for i in 0..64 {
            if self.lower & (1u64 << i) != 0 {
                out.push(i);
            }
        }
        // Need to enforce an order here for reproducible results,
        // but hitting this path should happen extremely rarely, so having this slow path is fine.
        out.extend(self.higher.iter().copied());
        out
    }

    pub fn empty(&self) -> bool {
        self.lower == 0 && self.higher.is_empty()
    }
}

impl PartialEq for Bitset {
    fn eq(&self, other: &Self) -> bool {
        self.lower == other.lower && self.higher == other.higher
    }
}

// join() and the StringStream formatting.

/// What `StringStream::operator<<` accepts: strings and characters as
/// they are, integers (and `bool`) through `std::to_string`.
pub trait JoinArg {
    fn append_to(&self, out: &mut String);
}

impl JoinArg for str {
    fn append_to(&self, out: &mut String) {
        out.push_str(self);
    }
}

impl JoinArg for String {
    fn append_to(&self, out: &mut String) {
        out.push_str(self);
    }
}

impl JoinArg for char {
    fn append_to(&self, out: &mut String) {
        out.push(*self);
    }
}

impl JoinArg for bool {
    fn append_to(&self, out: &mut String) {
        out.push(if *self { '1' } else { '0' });
    }
}

macro_rules! join_arg_int {
    ($($t:ty),*) => {
        $(impl JoinArg for $t {
            fn append_to(&self, out: &mut String) {
                let _ = write!(out, "{}", self);
            }
        })*
    };
}
join_arg_int!(u8, i8, u16, i16, u32, i32, u64, i64, usize, isize);

impl<T: JoinArg + ?Sized> JoinArg for &T {
    fn append_to(&self, out: &mut String) {
        (**self).append_to(out);
    }
}

impl<T: JoinArg + ?Sized> JoinArg for &mut T {
    fn append_to(&self, out: &mut String) {
        (**self).append_to(out);
    }
}

/// `join(ts...)`: concatenates the arguments as a `StringStream` would.
macro_rules! join {
    ($($x:expr),* $(,)?) => {{
        #[allow(unused_mut)]
        let mut s = String::new();
        $( $crate::spirv_cross::common::JoinArg::append_to(&$x, &mut s); )*
        s
    }};
}
pub(crate) use join;

/// `merge(list, between)`.
pub fn merge(list: &[String], between: &str) -> String {
    let mut s = String::new();
    for (i, elem) in list.iter().enumerate() {
        s.push_str(elem);
        if i + 1 != list.len() {
            s.push_str(between);
        }
    }
    s
}

/// `merge(list)`, with ", " between the elements.
pub fn merge_default(list: &[String]) -> String {
    merge(list, ", ")
}

/// `convert_to_string(const T &)` for unsigned and other non-floating
/// point types: `std::to_string`.
pub fn convert_to_string_u<T: std::fmt::Display>(t: T) -> String {
    t.to_string()
}

/// `convert_to_string(int32_t)`.
pub fn convert_to_string_i32(value: i32) -> String {
    // INT_MIN is ... special on some backends. If we use a decimal literal, and negate it, we
    // could accidentally promote the literal to long first, then negate.
    // To workaround it, emit int(0x80000000) instead.
    if value == i32::MIN {
        "int(0x80000000)".to_string()
    } else {
        value.to_string()
    }
}

/// `convert_to_string(int64_t, int64_type, long_long_literal_suffix)`.
pub fn convert_to_string_i64(
    value: i64,
    int64_type: &str,
    long_long_literal_suffix: bool,
) -> String {
    // INT64_MIN is ... special on some backends.
    // If we use a decimal literal, and negate it, we might overflow the representable numbers.
    // To workaround it, emit int(0x80000000) instead.
    if value == i64::MIN {
        join!(
            int64_type,
            "(0x8000000000000000u",
            if long_long_literal_suffix { "ll" } else { "l" },
            ")"
        )
    } else {
        value.to_string() + if long_long_literal_suffix { "ll" } else { "l" }
    }
}

/// printf's `%.<precision>g` (as glibc formats it), for SPIRV_CROSS_FLT_FMT.
pub fn format_g(value: f64, precision: usize) -> String {
    if value.is_nan() {
        return if value.is_sign_negative() {
            "-nan".into()
        } else {
            "nan".into()
        };
    }
    if value.is_infinite() {
        return if value < 0.0 {
            "-inf".into()
        } else {
            "inf".into()
        };
    }
    let p = if precision == 0 { 1 } else { precision };
    // The exponent X of the %e conversion with precision P - 1.
    let e_str = format!("{:.*e}", p - 1, value);
    let epos = e_str.find('e').unwrap_or(e_str.len());
    let x: i32 = e_str[epos + 1..].parse().unwrap_or(0);
    let mut s = if (x as i64) < p as i64 && x >= -4 {
        format!("{:.*}", (p as i64 - 1 - x as i64) as usize, value)
    } else {
        // %e: the mantissa, then the exponent with at least two digits.
        let mant = &e_str[..epos];
        let sign = if x < 0 { '-' } else { '+' };
        let mut mant = mant.to_string();
        if mant.contains('.') {
            while mant.ends_with('0') {
                mant.pop();
            }
            if mant.ends_with('.') {
                mant.pop();
            }
        }
        return format!("{}e{}{:02}", mant, sign, x.unsigned_abs());
    };
    // Without the '#' flag, trailing zeros are removed, then the point.
    if s.contains('.') {
        while s.ends_with('0') {
            s.pop();
        }
        if s.ends_with('.') {
            s.pop();
        }
    }
    s
}

// Allow implementations to set a convenient standard precision
// (SPIRV_CROSS_FLT_FMT "%.32g")
const SPIRV_CROSS_FLT_PRECISION: usize = 32;

/// `convert_to_string(float, char locale_radix_point)`.
pub fn convert_to_string_f32(t: f32, _locale_radix_point: char) -> String {
    // std::to_string for floating point values is broken.
    // Fallback to something more sane.
    // (The radix point fixup is a no-op: Rust's formatting is locale-free.)
    let mut buf = format_g(t as f64, SPIRV_CROSS_FLT_PRECISION);
    // Ensure that the literal is float.
    if !buf.contains('.') && !buf.contains('e') {
        buf.push_str(".0");
    }
    buf
}

/// `convert_to_string(double, char locale_radix_point)`.
pub fn convert_to_string_f64(t: f64, _locale_radix_point: char) -> String {
    // std::to_string for floating point values is broken.
    // Fallback to something more sane.
    let mut buf = format_g(t, SPIRV_CROSS_FLT_PRECISION);
    // Ensure that the literal is float.
    if !buf.contains('.') && !buf.contains('e') {
        buf.push_str(".0");
    }
    buf
}

/// `FloatFormatter`: lets an application format float literals.
pub trait FloatFormatter {
    fn format_float(&mut self, value: f32) -> String;
    fn format_double(&mut self, value: f64) -> String;
}

/// `Instruction`. An `EmbeddedInstruction` (an instruction stream which
/// is embedded in the object) is an instruction with `embedded` operands.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Instruction {
    pub op: u16,
    pub count: u16,
    // If offset is 0 (not a valid offset into the instruction stream),
    // we have an instruction stream which is embedded in the object.
    pub offset: u32,
    pub length: u32,
    pub embedded: Option<Rc<Vec<u32>>>,
}

impl Instruction {
    #[inline]
    pub fn is_embedded(&self) -> bool {
        self.offset == 0
    }

    /// An `EmbeddedInstruction` with these operands.
    pub fn embedded(op: Op, ops: Vec<u32>) -> Self {
        Instruction {
            op: op as u16,
            count: 0,
            offset: 0,
            length: ops.len() as u32,
            embedded: Some(Rc::new(ops)),
        }
    }
}

/// `enum Types`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[repr(u32)]
pub enum Types {
    #[default]
    TypeNone,
    TypeType,
    TypeVariable,
    TypeConstant,
    TypeFunction,
    TypeFunctionPrototype,
    TypeBlock,
    TypeExtension,
    TypeExpression,
    TypeConstantOp,
    TypeCombinedImageSampler,
    TypeAccessChain,
    TypeUndef,
    TypeString,
    TypeDebugLocalVariable,
    TypeCount,
}
pub use Types::*;

pub const TYPE_COUNT: usize = Types::TypeCount as usize;

// The TypedID<> wrappers are plain ids here.
pub type ID = u32;
pub type VariableID = u32;
pub type TypeID = u32;
pub type ConstantID = u32;
pub type FunctionID = u32;
pub type BlockID = u32;

/// `SPIRUndef`.
#[derive(Clone, Debug, Default)]
pub struct SPIRUndef {
    pub self_: ID,
    pub basetype: TypeID,
}

impl SPIRUndef {
    pub fn new(basetype: TypeID) -> Self {
        SPIRUndef { self_: 0, basetype }
    }
}

/// `SPIRString`.
#[derive(Clone, Debug, Default)]
pub struct SPIRString {
    pub self_: ID,
    pub str: String,
}

impl SPIRString {
    pub fn new(str: String) -> Self {
        SPIRString { self_: 0, str }
    }
}

/// `SPIRDebugLocalVariable`.
#[derive(Clone, Debug, Default)]
pub struct SPIRDebugLocalVariable {
    pub self_: ID,
    pub name_id: u32,
}

/// `SPIRCombinedImageSampler`: this type is only used by backends which
/// need to access the combined image and sampler IDs separately after the
/// OpSampledImage opcode.
#[derive(Clone, Debug, Default)]
pub struct SPIRCombinedImageSampler {
    pub self_: ID,
    pub combined_type: TypeID,
    pub image: VariableID,
    pub sampler: VariableID,
}

impl SPIRCombinedImageSampler {
    pub fn new(combined_type: TypeID, image: VariableID, sampler: VariableID) -> Self {
        SPIRCombinedImageSampler {
            self_: 0,
            combined_type,
            image,
            sampler,
        }
    }
}

/// `SPIRConstantOp`.
#[derive(Clone, Debug, Default)]
pub struct SPIRConstantOp {
    pub self_: ID,
    pub opcode: Op,
    pub arguments: Vec<u32>,
    pub basetype: TypeID,
}

impl SPIRConstantOp {
    pub fn new(result_type: TypeID, op: Op, args: &[u32]) -> Self {
        SPIRConstantOp {
            self_: 0,
            opcode: op,
            arguments: args.to_vec(),
            basetype: result_type,
        }
    }
}

/// `SPIRType::BaseType`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[repr(u32)]
pub enum BaseType {
    #[default]
    Unknown,
    Void,
    Boolean,
    SByte,
    UByte,
    Short,
    UShort,
    Int,
    UInt,
    Int64,
    UInt64,
    AtomicCounter,
    Half,
    Float,
    Double,
    Struct,
    Image,
    SampledImage,
    Sampler,
    AccelerationStructure,
    RayQuery,
    CoopVecNV,

    // Keep internal types at the end.
    ControlPointArray,
    Interpolant,
    Char,
    // MSL specific type, that is used by 'object'(analog of 'task' from glsl) shader.
    MeshGridProperties,
    BFloat16,
    FloatE4M3,
    FloatE5M2,

    Tensor,
    DescriptorHeapBuffer,
}

impl BaseType {
    /// The enumerator's value (C++ casts `BaseType` to `int`).
    pub fn to_u32(self) -> u32 {
        self as u32
    }

    /// `static_cast<SPIRType::BaseType>(v)`, for the values that exist.
    pub fn from_u32(v: u32) -> BaseType {
        use BaseType::*;
        const ALL: [BaseType; 31] = [
            Unknown,
            Void,
            Boolean,
            SByte,
            UByte,
            Short,
            UShort,
            Int,
            UInt,
            Int64,
            UInt64,
            AtomicCounter,
            Half,
            Float,
            Double,
            Struct,
            Image,
            SampledImage,
            Sampler,
            AccelerationStructure,
            RayQuery,
            CoopVecNV,
            ControlPointArray,
            Interpolant,
            Char,
            MeshGridProperties,
            BFloat16,
            FloatE4M3,
            FloatE5M2,
            Tensor,
            DescriptorHeapBuffer,
        ];
        ALL.get(v as usize).copied().unwrap_or(Unknown)
    }
}

/// `SPIRType::ImageType`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ImageType {
    pub type_: TypeID,
    pub dim: Dim,
    pub depth: bool,
    pub arrayed: bool,
    pub ms: bool,
    pub sampled: u32,
    pub format: ImageFormat,
    pub access: AccessQualifier,
}

/// The `SPIRType::ext` union: its words, whichever member is in use.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct TypeExt {
    pub w: [u32; 4],
}

impl TypeExt {
    // cooperative
    pub fn cooperative_use_id(&self) -> u32 {
        self.w[0]
    }
    pub fn cooperative_rows_id(&self) -> u32 {
        self.w[1]
    }
    pub fn cooperative_columns_id(&self) -> u32 {
        self.w[2]
    }
    pub fn cooperative_scope_id(&self) -> u32 {
        self.w[3]
    }
    // coopVecNV
    pub fn coop_vec_nv_component_type_id(&self) -> u32 {
        self.w[0]
    }
    pub fn coop_vec_nv_component_count_id(&self) -> u32 {
        self.w[1]
    }
    // tensor
    pub fn tensor_type(&self) -> u32 {
        self.w[0]
    }
    pub fn tensor_rank(&self) -> u32 {
        self.w[1]
    }
    pub fn tensor_shape(&self) -> u32 {
        self.w[2]
    }
    // descriptor_heap_buffer
    pub fn descriptor_heap_buffer_storage(&self) -> StorageClass {
        self.w[0]
    }
}

/// `SPIRType`.
#[derive(Clone, Debug)]
pub struct SPIRType {
    pub self_: ID,

    pub op: Op,

    // Scalar/vector/matrix support.
    pub basetype: BaseType,
    pub width: u32,
    pub vecsize: u32,
    pub columns: u32,

    // Arrays, support array of arrays by having a vector of array sizes.
    pub array: Vec<u32>,

    // Array elements can be either specialization constants or specialization ops.
    // This array determines how to interpret the array size.
    // If an element is true, the element is a literal,
    // otherwise, it's an expression, which must be resolved on demand.
    // The actual size is not really known until runtime.
    pub array_size_literal: Vec<bool>,

    // Pointers
    // Keep track of how many pointer layers we have.
    pub pointer_depth: u32,
    pub pointer: bool,
    pub forward_pointer: bool,

    pub ext: TypeExt,

    pub storage: StorageClass,

    pub member_types: Vec<TypeID>,

    // If member order has been rewritten to handle certain scenarios with Offset,
    // allow codegen to rewrite the index.
    pub member_type_index_redirection: Vec<u32>,

    pub image: ImageType,

    // Structs can be declared multiple times if they are used as part of interface blocks.
    // We want to detect this so that we only emit the struct definition once.
    // Since we cannot rely on OpName to be equal, we need to figure out aliases.
    pub type_alias: TypeID,

    // Denotes the type which this type is based on.
    // Allows the backend to traverse how a complex type is built up during access chains.
    pub parent_type: TypeID,

    // Used in backends to avoid emitting members with conflicting names.
    pub member_name_cache: HashSet<String>,
}

impl SPIRType {
    pub fn new(op: Op) -> Self {
        SPIRType {
            self_: 0,
            op,
            basetype: BaseType::Unknown,
            width: 0,
            vecsize: 1,
            columns: 1,
            array: Vec::new(),
            array_size_literal: Vec::new(),
            pointer_depth: 0,
            pointer: false,
            forward_pointer: false,
            ext: TypeExt::default(),
            storage: StorageClassGeneric,
            member_types: Vec::new(),
            member_type_index_redirection: Vec::new(),
            image: ImageType::default(),
            type_alias: 0,
            parent_type: 0,
            member_name_cache: HashSet::new(),
        }
    }
}

impl Default for SPIRType {
    fn default() -> Self {
        SPIRType::new(OpNop)
    }
}

/// `SPIRExtension::Extension`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Extension {
    #[default]
    Unsupported,
    GLSL,
    SPV_debug_info,
    SPV_AMD_shader_ballot,
    SPV_AMD_shader_explicit_vertex_parameter,
    SPV_AMD_shader_trinary_minmax,
    SPV_AMD_gcn_shader,
    NonSemanticDebugPrintf,
    NonSemanticShaderDebugInfo,
    NonSemanticGeneric,
}

/// `SPIRExtension`.
#[derive(Clone, Debug, Default)]
pub struct SPIRExtension {
    pub self_: ID,
    pub ext: Extension,
}

impl SPIRExtension {
    // enum ShaderDebugInfoOps
    pub const DEBUG_LINE: u32 = 103;
    pub const DEBUG_SOURCE: u32 = 35;

    pub fn new(ext: Extension) -> Self {
        SPIRExtension { self_: 0, ext }
    }
}

/// `SPIREntryPoint::WorkgroupSize`.
#[derive(Clone, Copy, Debug, Default)]
pub struct WorkgroupSize {
    pub x: u32,
    pub y: u32,
    pub z: u32,
    pub id_x: u32,
    pub id_y: u32,
    pub id_z: u32,
    pub constant: u32, // Workgroup size can be expressed as a constant/spec-constant instead.
}

/// `SPIREntryPoint`: not a variant since its IDs are used to decorate
/// OpFunction, so in order to avoid conflicts, we can't stick them in the
/// ids array.
#[derive(Clone, Debug)]
pub struct SPIREntryPoint {
    pub self_: FunctionID,
    pub name: String,
    pub orig_name: String,
    pub fp_fast_math_defaults: HashMap<u32, u32>,
    pub signed_zero_inf_nan_preserve_8: bool,
    pub signed_zero_inf_nan_preserve_16: bool,
    pub signed_zero_inf_nan_preserve_32: bool,
    pub signed_zero_inf_nan_preserve_64: bool,
    pub interface_variables: Vec<VariableID>,

    pub flags: Bitset,
    pub workgroup_size: WorkgroupSize,
    pub invocations: u32,
    pub output_vertices: u32,
    pub output_primitives: u32,
    pub model: ExecutionModel,
    pub geometry_passthrough: bool,
}

impl SPIREntryPoint {
    pub fn new(self_: FunctionID, execution_model: ExecutionModel, entry_name: &str) -> Self {
        SPIREntryPoint {
            self_,
            name: entry_name.to_string(),
            orig_name: entry_name.to_string(),
            model: execution_model,
            ..Default::default()
        }
    }
}

impl Default for SPIREntryPoint {
    fn default() -> Self {
        SPIREntryPoint {
            self_: 0,
            name: String::new(),
            orig_name: String::new(),
            fp_fast_math_defaults: HashMap::new(),
            signed_zero_inf_nan_preserve_8: false,
            signed_zero_inf_nan_preserve_16: false,
            signed_zero_inf_nan_preserve_32: false,
            signed_zero_inf_nan_preserve_64: false,
            interface_variables: Vec::new(),
            flags: Bitset::default(),
            workgroup_size: WorkgroupSize::default(),
            invocations: 0,
            output_vertices: 0,
            output_primitives: 0,
            model: ExecutionModelMax,
            geometry_passthrough: false,
        }
    }
}

/// `SPIRExpression`.
#[derive(Clone, Debug, Default)]
pub struct SPIRExpression {
    pub self_: ID,

    // If non-zero, prepend expression with to_expression(base_expression).
    // Used in amortizing multiple calls to to_expression()
    // where in certain cases that would quickly force a temporary when not needed.
    pub base_expression: ID,

    pub expression: String,
    pub expression_type: TypeID,

    // If this expression is a forwarded load,
    // allow us to reference the original variable.
    pub loaded_from: ID,

    // If this expression will never change, we can avoid lots of temporaries
    // in high level source.
    // An expression being immutable can be speculative,
    // it is assumed that this is true almost always.
    pub immutable: bool,

    // Before use, this expression must be transposed.
    // This is needed for targets which don't support row_major layouts.
    pub need_transpose: bool,

    // Whether or not this is an access chain expression.
    pub access_chain: bool,

    // Whether or not gl_MeshVerticesEXT[].gl_Position (as a whole or .y) is referenced
    pub access_meshlet_position_y: bool,

    // If this expression represents a OpBufferPointerEXT cast.
    pub buffer_pointer: bool,

    // Temporaries which can remain forwarded as long as this variable is not modified.
    // Only used for buffer pointers.
    pub buffer_pointer_dependees: Vec<ID>,

    // A list of expressions which this expression depends on.
    pub expression_dependencies: Vec<ID>,

    // Similar as expression dependencies, but does not stop the tracking for force-temporary variables.
    // We need to know the full chain from store back to any SSA variable.
    pub invariance_dependencies: Vec<ID>,

    // By reading this expression, we implicitly read these expressions as well.
    // Used by access chain Store and Load since we read multiple expressions in this case.
    pub implied_read_expressions: Vec<ID>,

    // The expression was emitted at a certain scope. Lets us track when an expression read means multiple reads.
    pub emitted_loop_level: u32,
}

impl SPIRExpression {
    // Only created by the backend target to avoid creating tons of temporaries.
    pub fn new(expr: String, expression_type: TypeID, immutable: bool) -> Self {
        SPIRExpression {
            expression: expr,
            expression_type,
            immutable,
            ..Default::default()
        }
    }
}

/// `SPIRFunctionPrototype`.
#[derive(Clone, Debug, Default)]
pub struct SPIRFunctionPrototype {
    pub self_: ID,
    pub return_type: TypeID,
    pub parameter_types: Vec<u32>,
}

impl SPIRFunctionPrototype {
    pub fn new(return_type: TypeID) -> Self {
        SPIRFunctionPrototype {
            self_: 0,
            return_type,
            parameter_types: Vec::new(),
        }
    }
}

/// `SPIRBlock::Terminator`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Terminator {
    #[default]
    Unknown,
    Direct, // Emit next block directly without a particular condition.

    Select,      // Block ends with an if/else block.
    MultiSelect, // Block ends with switch statement.

    Return,             // Block ends with return.
    Unreachable,        // Noop
    Kill,               // Discard
    IgnoreIntersection, // Ray Tracing
    TerminateRay,       // Ray Tracing
    EmitMeshTasks,      // Mesh shaders
}

/// `SPIRBlock::Merge`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Merge {
    #[default]
    MergeNone,
    MergeLoop,
    MergeSelection,
}

/// `SPIRBlock::Hints`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Hints {
    #[default]
    HintNone,
    HintUnroll,
    HintDontUnroll,
    HintFlatten,
    HintDontFlatten,
}

/// `SPIRBlock::Method`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Method {
    MergeToSelectForLoop,
    MergeToDirectForLoop,
    MergeToSelectContinueForLoop,
}

/// `SPIRBlock::ContinueBlockType`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ContinueBlockType {
    #[default]
    ContinueNone,

    // Continue block is branchless and has at least one instruction.
    ForLoop,

    // Noop continue block.
    WhileLoop,

    // Continue block is conditional.
    DoWhileLoop,

    // Highly unlikely that anything will use this,
    // since it is really awkward/impossible to express in GLSL.
    ComplexLoop,
}

/// `SPIRBlock::Phi`.
#[derive(Clone, Copy, Debug, Default)]
pub struct Phi {
    pub local_variable: ID,            // flush local variable ...
    pub parent: BlockID, // If we're in from_block and want to branch into this block ...
    pub function_variable: VariableID, // to this function-global "phi" variable first.
}

/// `SPIRBlock::Case`.
#[derive(Clone, Copy, Debug, Default)]
pub struct Case {
    pub value: u64,
    pub block: BlockID,
}

/// `SPIRBlock::mesh`.
#[derive(Clone, Copy, Debug, Default)]
pub struct MeshTasks {
    pub groups: [ID; 3],
    pub payload: ID,
}

/// `SPIRBlock`.
#[derive(Clone, Debug, Default)]
pub struct SPIRBlock {
    pub self_: ID,

    pub terminator: Terminator,
    pub merge: Merge,
    pub hint: Hints,
    pub next_block: BlockID,
    pub merge_block: BlockID,
    pub continue_block: BlockID,

    pub return_value: ID, // If 0, return nothing (void).
    pub condition: ID,
    pub true_block: BlockID,
    pub false_block: BlockID,
    pub default_block: BlockID,

    // If terminator is EmitMeshTasksEXT.
    pub mesh: MeshTasks,

    pub ops: Vec<Instruction>,

    // Before entering this block flush out local variables to magical "phi" variables.
    pub phi_variables: Vec<Phi>,

    // Declare these temporaries before beginning the block.
    // Used for handling complex continue blocks which have side effects.
    pub declare_temporary: Vec<(TypeID, ID)>,

    // Declare these temporaries, but only conditionally if this block turns out to be
    // a complex loop header.
    pub potential_declare_temporary: Vec<(TypeID, ID)>,

    pub cases_32bit: Vec<Case>,
    pub cases_64bit: Vec<Case>,

    // If we have tried to optimize code for this block but failed,
    // keep track of this.
    pub disable_block_optimization: bool,

    // If the continue block is complex, fallback to "dumb" for loops.
    pub complex_continue: bool,

    // Do we need a ladder variable to defer breaking out of a loop construct after a switch block?
    pub need_ladder_break: bool,

    // If marked, we have explicitly handled Phi from this block, so skip any flushes related to that on a branch.
    // Used to handle an edge case with switch and case-label fallthrough where fall-through writes to Phi.
    pub ignore_phi_from_block: BlockID,

    // The dominating block which this block might be within.
    // Used in continue; blocks to determine if we really need to write continue.
    pub loop_dominator: BlockID,

    // All access to these variables are dominated by this block,
    // so before branching anywhere we need to make sure that we declare these variables.
    pub dominated_variables: Vec<VariableID>,
    pub rearm_dominated_variables: Vec<bool>,

    // These are variables which should be declared in a for loop header, if we
    // fail to use a classic for-loop,
    // we remove these variables, and fall back to regular variables outside the loop.
    pub loop_variables: Vec<VariableID>,

    // Some expressions are control-flow dependent, i.e. any instruction which relies on derivatives or
    // sub-group-like operations.
    // Make sure that we only use these expressions in the original block.
    pub invalidate_expressions: Vec<ID>,
}

impl SPIRBlock {
    pub const NO_DOMINATOR: u32 = 0xffffffff;
}

/// `SPIRFunction::Parameter`.
#[derive(Clone, Copy, Debug, Default)]
pub struct Parameter {
    pub type_: TypeID,
    pub id: ID,
    pub read_count: u32,
    pub write_count: u32,

    // Set to true if this parameter aliases a global variable,
    // used mostly in Metal where global variables
    // have to be passed down to functions as regular arguments.
    // However, for this kind of variable, we should not care about
    // read and write counts as access to the function arguments
    // is not local to the function in question.
    pub alias_global_variable: bool,
}

/// `SPIRFunction::CombinedImageSamplerParameter`.
#[derive(Clone, Copy, Debug, Default)]
pub struct CombinedImageSamplerParameter {
    pub id: VariableID,
    pub image_id: VariableID,
    pub sampler_id: VariableID,
    pub global_image: bool,
    pub global_sampler: bool,
    pub depth: bool,
}

/// `SPIRFunction::EntryLine`.
#[derive(Clone, Copy, Debug, Default)]
pub struct EntryLine {
    pub file_id: u32,
    pub line_literal: u32,
}

/// The `std::function<void()>` fixup hooks of a function, which run on the
/// compiler.
pub type FixupHook = Rc<dyn Fn(&mut Compiler) -> Result<()>>;

/// `SPIRFunction`.
#[derive(Clone, Default)]
pub struct SPIRFunction {
    pub self_: ID,

    pub return_type: TypeID,
    pub function_type: TypeID,
    pub arguments: Vec<Parameter>,

    // Can be used by backends to add magic arguments.
    // Currently used by combined image/sampler implementation.
    pub shadow_arguments: Vec<Parameter>,
    pub local_variables: Vec<VariableID>,
    pub entry_block: BlockID,
    pub blocks: Vec<BlockID>,
    pub combined_parameters: Vec<CombinedImageSamplerParameter>,

    pub entry_line: EntryLine,

    // Hooks to be run when the function returns.
    // Mostly used for lowering internal data structures onto flattened structures.
    // Need to defer this, because they might rely on things which change during compilation.
    // Intentionally not a small vector, this one is rare, and std::function can be large.
    pub fixup_hooks_out: Vec<FixupHook>,

    // Hooks to be run when the function begins.
    // Mostly used for populating internal data structures from flattened structures.
    // Need to defer this, because they might rely on things which change during compilation.
    // Intentionally not a small vector, this one is rare, and std::function can be large.
    pub fixup_hooks_in: Vec<FixupHook>,

    // On function entry, make sure to copy a constant array into thread addr space to work around
    // the case where we are passing a constant array by value to a function on backends which do not
    // consider arrays value types.
    pub constant_arrays_needed_on_stack: Vec<ID>,

    // Does this function (or any function called by it), emit geometry?
    pub emits_geometry: bool,

    pub active: bool,
    pub flush_undeclared: bool,
    pub do_combined_parameters: bool,
}

impl std::fmt::Debug for SPIRFunction {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SPIRFunction")
            .field("self", &self.self_)
            .finish_non_exhaustive()
    }
}

impl SPIRFunction {
    pub fn new(return_type: TypeID, function_type: TypeID) -> Self {
        SPIRFunction {
            return_type,
            function_type,
            flush_undeclared: true,
            do_combined_parameters: true,
            ..Default::default()
        }
    }

    pub fn add_local_variable(&mut self, id: VariableID) {
        self.local_variables.push(id);
    }

    pub fn add_parameter(&mut self, parameter_type: TypeID, id: ID, alias_global_variable: bool) {
        // Arguments are read-only until proven otherwise.
        self.arguments.push(Parameter {
            type_: parameter_type,
            id,
            read_count: 0,
            write_count: 0,
            alias_global_variable,
        });
    }
}

/// `SPIRAccessChain`.
#[derive(Clone, Debug, Default)]
pub struct SPIRAccessChain {
    pub self_: ID,

    // The access chain represents an offset into a buffer.
    // Some backends need more complicated handling of access chains to be able to use buffers, like HLSL
    // which has no usable buffer type ala GLSL SSBOs.
    // StructuredBuffer is too limited, so our only option is to deal with ByteAddressBuffer which works with raw addresses.
    pub basetype: TypeID,
    pub storage: StorageClass,
    pub base: String,
    pub dynamic_index: String,
    pub static_index: i32,

    pub loaded_from: VariableID,
    pub matrix_stride: u32,
    pub array_stride: u32,
    pub row_major_matrix: bool,
    pub immutable: bool,

    // By reading this expression, we implicitly read these expressions as well.
    // Used by access chain Store and Load since we read multiple expressions in this case.
    pub implied_read_expressions: Vec<ID>,
}

impl SPIRAccessChain {
    pub fn new(
        basetype: TypeID,
        storage: StorageClass,
        base: String,
        dynamic_index: String,
        static_index: i32,
    ) -> Self {
        SPIRAccessChain {
            basetype,
            storage,
            base,
            dynamic_index,
            static_index,
            ..Default::default()
        }
    }
}

/// Where `SPIRVariable::parameter` points: an argument of a function, by
/// index (C++ keeps a pointer into the function's `arguments`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ParameterRef {
    pub function: FunctionID,
    pub index: usize,
}

/// `SPIRVariable`.
#[derive(Clone, Debug, Default)]
pub struct SPIRVariable {
    pub self_: ID,

    pub basetype: TypeID,
    pub storage: StorageClass,
    pub decoration: u32,
    pub initializer: ID,
    pub basevariable: VariableID,

    pub dereference_chain: Vec<u32>,
    pub compat_builtin: bool,

    // If a variable is shadowed, we only statically assign to it
    // and never actually emit a statement for it.
    // When we read the variable as an expression, just forward
    // shadowed_id as the expression.
    pub statically_assigned: bool,
    pub static_expression: ID,

    // Temporaries which can remain forwarded as long as this variable is not modified.
    pub dependees: Vec<ID>,

    // ShaderDebugInfo local variables attached to this variable via DebugDeclare
    pub debug_local_variables: Vec<ID>,

    pub deferred_declaration: bool,
    pub phi_variable: bool,

    // Used to deal with Phi variable flushes. See flush_phi().
    pub allocate_temporary_copy: bool,

    pub remapped_variable: bool,
    pub remapped_components: u32,

    // The block which dominates all access to this variable.
    pub dominator: BlockID,
    // If true, this variable is a loop variable, when accessing the variable
    // outside a loop,
    // we should statically forward it.
    pub loop_variable: bool,
    // Set to true while we're inside the for loop.
    pub loop_variable_enable: bool,

    // Used to find global LUTs
    pub is_written_to: bool,

    // Untyped pointer. The pointer of the variable is effectively void.
    // The underlying payload for allocation is in alloca_type, but may be 0 too.
    // This is mostly here to support descriptor heap proxy.
    pub untyped: bool,
    pub untyped_alloca_type: ID,

    pub parameter: Option<ParameterRef>,
}

impl SPIRVariable {
    pub fn new(
        basetype: TypeID,
        storage: StorageClass,
        initializer: ID,
        basevariable: VariableID,
    ) -> Self {
        SPIRVariable {
            basetype,
            storage,
            initializer,
            basevariable,
            ..Default::default()
        }
    }
}

/// `SPIRConstant::ConstantVector`. The `Constant` union is kept as its
/// raw 64 bits: `u32`/`i32`/`f32` are the low half (little-endian).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ConstantVector {
    pub r: [u64; 4],
    // If != 0, this element is a specialization constant, and we should keep track of it as such.
    pub id: [ID; 4],
    pub vecsize: u32,
}

impl Default for ConstantVector {
    fn default() -> Self {
        ConstantVector {
            r: [0; 4],
            id: [0; 4],
            vecsize: 1,
        }
    }
}

/// Sets the `u32` member of a `Constant` union (its low 32 bits).
#[inline]
pub fn constant_set_u32(r: &mut u64, v: u32) {
    *r = (*r & !0xffff_ffffu64) | v as u64;
}

/// `SPIRConstant::ConstantMatrix`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ConstantMatrix {
    pub c: [ConstantVector; 4],
    // If != 0, this column is a specialization constant, and we should keep track of it as such.
    pub id: [ID; 4],
    pub columns: u32,
}

impl Default for ConstantMatrix {
    fn default() -> Self {
        ConstantMatrix {
            c: [ConstantVector::default(); 4],
            id: [0; 4],
            columns: 1,
        }
    }
}

/// `SPIRConstant`.
#[derive(Clone, Debug, Default)]
pub struct SPIRConstant {
    pub self_: ID,

    pub constant_type: TypeID,
    pub m: ConstantMatrix,

    // If this constant is a specialization constant (i.e. created with OpSpecConstant*).
    pub specialization: bool,
    // If this constant is used as an array length which creates specialization restrictions on some backends.
    pub is_used_as_array_length: bool,

    // If true, this is a LUT, and should always be declared in the outer scope.
    pub is_used_as_lut: bool,

    // If this is a null constant of array type with specialized length.
    // May require special handling in initializer
    pub is_null_array_specialized_length: bool,

    // For composites which are constant arrays, etc.
    pub subconstants: Vec<ConstantID>,

    // Whether the subconstants are intended to be replicated (e.g. OpConstantCompositeReplicateEXT)
    pub replicated: bool,

    // Non-Vulkan GLSL, HLSL and sometimes MSL emits defines for each specialization constant,
    // and uses them to initialize the constant. This allows the user
    // to still be able to specialize the value by supplying corresponding
    // preprocessor directives before compiling the shader.
    pub specialization_constant_macro_name: String,

    // ConstantSizeOfEXT.
    pub size_of_type: ID,
}

impl SPIRConstant {
    pub fn f16_to_f32(u16_value: u16) -> f32 {
        // Based on the GLM implementation.
        let s = ((u16_value >> 15) & 0x1) as i32;
        let mut e = ((u16_value >> 10) & 0x1f) as i32;
        let mut m = (u16_value & 0x3ff) as i32;

        if e == 0 {
            if m == 0 {
                return f32::from_bits((s as u32) << 31);
            } else {
                while (m & 0x400) == 0 {
                    m <<= 1;
                    e -= 1;
                }

                e += 1;
                m &= !0x400;
            }
        } else if e == 31 {
            if m == 0 {
                return f32::from_bits(((s as u32) << 31) | 0x7f800000u32);
            } else {
                return f32::from_bits(((s as u32) << 31) | 0x7f800000u32 | ((m as u32) << 13));
            }
        }

        e += 127 - 15;
        m <<= 13;
        f32::from_bits(((s as u32) << 31) | ((e as u32) << 23) | m as u32)
    }

    pub fn fe4m3_to_f32(v: u8) -> f32 {
        if (v & 0x7f) == 0x7f {
            f32::from_bits(if v & 0x80 != 0 {
                0xffffffffu32
            } else {
                0x7fffffffu32
            })
        } else {
            // Reuse the FP16 to FP32 code. Cute bit-hackery.
            Self::f16_to_f32((((v as i8 as i16) << 7) as u16) & (0xffff ^ 0x4000)) * 256.0f32
        }
    }

    #[inline]
    pub fn specialization_constant_id(&self, col: u32, row: u32) -> u32 {
        self.m.c[col as usize].id[row as usize]
    }

    #[inline]
    pub fn specialization_constant_id_col(&self, col: u32) -> u32 {
        self.m.id[col as usize]
    }

    #[inline]
    pub fn scalar(&self, col: u32, row: u32) -> u32 {
        self.m.c[col as usize].r[row as usize] as u32
    }

    #[inline]
    pub fn scalar_i16(&self, col: u32, row: u32) -> i16 {
        (self.scalar(col, row) & 0xffff) as u16 as i16
    }

    #[inline]
    pub fn scalar_u16(&self, col: u32, row: u32) -> u16 {
        (self.scalar(col, row) & 0xffff) as u16
    }

    #[inline]
    pub fn scalar_i8(&self, col: u32, row: u32) -> i8 {
        (self.scalar(col, row) & 0xff) as u8 as i8
    }

    #[inline]
    pub fn scalar_u8(&self, col: u32, row: u32) -> u8 {
        (self.scalar(col, row) & 0xff) as u8
    }

    #[inline]
    pub fn scalar_f16(&self, col: u32, row: u32) -> f32 {
        Self::f16_to_f32(self.scalar_u16(col, row))
    }

    #[inline]
    pub fn scalar_bf16(&self, col: u32, row: u32) -> f32 {
        let v = (self.scalar_u16(col, row) as u32) << 16;
        f32::from_bits(v)
    }

    #[inline]
    pub fn scalar_floate4m3(&self, col: u32, row: u32) -> f32 {
        Self::fe4m3_to_f32(self.scalar_u8(col, row))
    }

    #[inline]
    pub fn scalar_bf8(&self, col: u32, row: u32) -> f32 {
        Self::f16_to_f32((self.scalar_u8(col, row) as u16) << 8)
    }

    #[inline]
    pub fn scalar_f32(&self, col: u32, row: u32) -> f32 {
        f32::from_bits(self.scalar(col, row))
    }

    #[inline]
    pub fn scalar_i32(&self, col: u32, row: u32) -> i32 {
        self.scalar(col, row) as i32
    }

    #[inline]
    pub fn scalar_f64(&self, col: u32, row: u32) -> f64 {
        f64::from_bits(self.scalar_u64(col, row))
    }

    #[inline]
    pub fn scalar_i64(&self, col: u32, row: u32) -> i64 {
        self.scalar_u64(col, row) as i64
    }

    #[inline]
    pub fn scalar_u64(&self, col: u32, row: u32) -> u64 {
        self.m.c[col as usize].r[row as usize]
    }

    #[inline]
    pub fn vector(&self) -> &ConstantVector {
        &self.m.c[0]
    }

    #[inline]
    pub fn vector_size(&self) -> u32 {
        self.m.c[0].vecsize
    }

    #[inline]
    pub fn columns(&self) -> u32 {
        self.m.columns
    }

    pub fn make_null(&mut self, constant_type: &SPIRType) {
        self.m = ConstantMatrix::default();
        self.m.columns = constant_type.columns;
        for c in self.m.c.iter_mut() {
            c.vecsize = constant_type.vecsize;
        }
    }

    pub fn constant_is_null(&self) -> bool {
        if self.specialization {
            return false;
        }
        if !self.subconstants.is_empty() {
            return false;
        }

        // Note (upstream): C++ reads past the four columns and rows of the
        // union for larger sizes; the translation stops at four.
        for col in 0..self.columns().min(4) {
            for row in 0..self.vector_size().min(4) {
                if self.scalar_u64(col, row) != 0 {
                    return false;
                }
            }
        }

        true
    }

    pub fn new_typed(constant_type: u32) -> Self {
        SPIRConstant {
            constant_type,
            ..Default::default()
        }
    }

    /// The constructor of composites.
    pub fn new_composite(
        constant_type: TypeID,
        elements: &[u32],
        specialized: bool,
        replicated: bool,
    ) -> Self {
        SPIRConstant {
            constant_type,
            specialization: specialized,
            replicated,
            subconstants: elements.to_vec(),
            ..Default::default()
        }
    }

    /// Construct scalar (32-bit).
    pub fn new_scalar32(constant_type: TypeID, v0: u32, specialized: bool) -> Self {
        let mut c = SPIRConstant {
            constant_type,
            specialization: specialized,
            ..Default::default()
        };
        constant_set_u32(&mut c.m.c[0].r[0], v0);
        c.m.c[0].vecsize = 1;
        c.m.columns = 1;
        c
    }

    /// Construct scalar (64-bit).
    pub fn new_scalar64(constant_type: TypeID, v0: u64, specialized: bool) -> Self {
        let mut c = SPIRConstant {
            constant_type,
            specialization: specialized,
            ..Default::default()
        };
        c.m.c[0].r[0] = v0;
        c.m.c[0].vecsize = 1;
        c.m.columns = 1;
        c
    }

    /// Construct vectors and matrices.
    pub fn new_vector_matrix(
        constant_type: TypeID,
        vector_elements: &[&SPIRConstant],
        specialized: bool,
    ) -> Result<Self> {
        let mut c = SPIRConstant {
            constant_type,
            specialization: specialized,
            ..Default::default()
        };
        let num_elements = vector_elements.len();
        let first = vector_elements
            .first()
            .ok_or_else(|| CompilerError::new("Index out of range."))?;
        let matrix = first.m.c[0].vecsize > 1;
        // Note (upstream): C++ writes past the four columns (or rows) of
        // the union for more elements; the translation throws instead.
        if num_elements > 4 {
            spirv_cross_throw!("Index out of range.");
        }

        if matrix {
            c.m.columns = num_elements as u32;

            for (i, e) in vector_elements.iter().enumerate() {
                c.m.c[i] = e.m.c[0];
                if e.specialization {
                    c.m.id[i] = e.self_;
                }
            }
        } else {
            c.m.c[0].vecsize = num_elements as u32;
            c.m.columns = 1;

            for (i, e) in vector_elements.iter().enumerate() {
                c.m.c[0].r[i] = e.m.c[0].r[0];
                if e.specialization {
                    c.m.c[0].id[i] = e.self_;
                }
            }
        }
        Ok(c)
    }
}

// Variants. C++ allocates the objects from per-type pools; here each
// object is an `Rc`.

/// The object a `Variant` holds.
#[derive(Clone)]
pub enum Holder {
    Type(Rc<SPIRType>),
    Variable(Rc<SPIRVariable>),
    Constant(Rc<SPIRConstant>),
    Function(Rc<SPIRFunction>),
    FunctionPrototype(Rc<SPIRFunctionPrototype>),
    Block(Rc<SPIRBlock>),
    Extension(Rc<SPIRExtension>),
    Expression(Rc<SPIRExpression>),
    ConstantOp(Rc<SPIRConstantOp>),
    CombinedImageSampler(Rc<SPIRCombinedImageSampler>),
    AccessChain(Rc<SPIRAccessChain>),
    Undef(Rc<SPIRUndef>),
    String(Rc<SPIRString>),
    DebugLocalVariable(Rc<SPIRDebugLocalVariable>),
}

/// The `IVariant` types: what can live in a `Variant`.
pub trait IVariant: Clone + 'static {
    const TYPE: Types;
    fn from_holder(h: &Holder) -> Option<&Rc<Self>>;
    fn from_holder_mut(h: &mut Holder) -> Option<&mut Rc<Self>>;
    fn into_holder(v: Rc<Self>) -> Holder;
    fn self_id(&self) -> ID;
    fn set_self(&mut self, id: ID);
    /// `set_initializers()`: only expressions record their loop level.
    fn set_initializers(&mut self, _current_loop_level: u32) {}
}

macro_rules! ivariant {
    ($t:ident, $variant:ident, $types:ident) => {
        impl IVariant for $t {
            const TYPE: Types = Types::$types;
            #[inline]
            fn from_holder(h: &Holder) -> Option<&Rc<Self>> {
                match h {
                    Holder::$variant(v) => Some(v),
                    _ => None,
                }
            }
            #[inline]
            fn from_holder_mut(h: &mut Holder) -> Option<&mut Rc<Self>> {
                match h {
                    Holder::$variant(v) => Some(v),
                    _ => None,
                }
            }
            #[inline]
            fn into_holder(v: Rc<Self>) -> Holder {
                Holder::$variant(v)
            }
            #[inline]
            fn self_id(&self) -> ID {
                self.self_
            }
            #[inline]
            fn set_self(&mut self, id: ID) {
                self.self_ = id;
            }
        }
    };
}

ivariant!(SPIRType, Type, TypeType);
ivariant!(SPIRVariable, Variable, TypeVariable);
ivariant!(SPIRConstant, Constant, TypeConstant);
ivariant!(SPIRFunction, Function, TypeFunction);
ivariant!(
    SPIRFunctionPrototype,
    FunctionPrototype,
    TypeFunctionPrototype
);
ivariant!(SPIRBlock, Block, TypeBlock);
ivariant!(SPIRExtension, Extension, TypeExtension);
ivariant!(SPIRConstantOp, ConstantOp, TypeConstantOp);
ivariant!(
    SPIRCombinedImageSampler,
    CombinedImageSampler,
    TypeCombinedImageSampler
);
ivariant!(SPIRAccessChain, AccessChain, TypeAccessChain);
ivariant!(SPIRUndef, Undef, TypeUndef);
ivariant!(SPIRString, String, TypeString);
ivariant!(
    SPIRDebugLocalVariable,
    DebugLocalVariable,
    TypeDebugLocalVariable
);

impl IVariant for SPIRExpression {
    const TYPE: Types = Types::TypeExpression;
    #[inline]
    fn from_holder(h: &Holder) -> Option<&Rc<Self>> {
        match h {
            Holder::Expression(v) => Some(v),
            _ => None,
        }
    }
    #[inline]
    fn from_holder_mut(h: &mut Holder) -> Option<&mut Rc<Self>> {
        match h {
            Holder::Expression(v) => Some(v),
            _ => None,
        }
    }
    #[inline]
    fn into_holder(v: Rc<Self>) -> Holder {
        Holder::Expression(v)
    }
    #[inline]
    fn self_id(&self) -> ID {
        self.self_
    }
    #[inline]
    fn set_self(&mut self, id: ID) {
        self.self_ = id;
    }
    fn set_initializers(&mut self, current_loop_level: u32) {
        self.emitted_loop_level = current_loop_level;
    }
}

/// `Variant`.
#[derive(Clone, Default)]
pub struct Variant {
    holder: Option<Holder>,
    type_: Types,
    allow_type_rewrite: bool,
}

impl Variant {
    pub fn set(&mut self, val: Holder, new_type: Types) -> Result<()> {
        self.holder = None;

        if !self.allow_type_rewrite && self.type_ != TypeNone && self.type_ != new_type {
            spirv_cross_throw!("Overwriting a variant with new type.");
        }

        self.holder = Some(val);
        self.type_ = new_type;
        self.allow_type_rewrite = false;
        Ok(())
    }

    pub fn get<T: IVariant>(&self) -> Result<&T> {
        let Some(h) = &self.holder else {
            spirv_cross_throw!("nullptr");
        };
        if T::TYPE != self.type_ {
            spirv_cross_throw!("Bad cast");
        }
        match T::from_holder(h) {
            Some(v) => Ok(v),
            None => spirv_cross_throw!("Bad cast"),
        }
    }

    pub fn get_rc<T: IVariant>(&self) -> Result<Rc<T>> {
        let Some(h) = &self.holder else {
            spirv_cross_throw!("nullptr");
        };
        if T::TYPE != self.type_ {
            spirv_cross_throw!("Bad cast");
        }
        match T::from_holder(h) {
            Some(v) => Ok(v.clone()),
            None => spirv_cross_throw!("Bad cast"),
        }
    }

    pub fn get_mut<T: IVariant>(&mut self) -> Result<&mut T> {
        let ty = self.type_;
        let Some(h) = &mut self.holder else {
            spirv_cross_throw!("nullptr");
        };
        if T::TYPE != ty {
            spirv_cross_throw!("Bad cast");
        }
        match T::from_holder_mut(h) {
            Some(v) => Ok(Rc::make_mut(v)),
            None => spirv_cross_throw!("Bad cast"),
        }
    }

    pub fn get_type(&self) -> Types {
        self.type_
    }

    pub fn get_id(&self) -> ID {
        match &self.holder {
            None => 0,
            Some(h) => match h {
                Holder::Type(v) => v.self_,
                Holder::Variable(v) => v.self_,
                Holder::Constant(v) => v.self_,
                Holder::Function(v) => v.self_,
                Holder::FunctionPrototype(v) => v.self_,
                Holder::Block(v) => v.self_,
                Holder::Extension(v) => v.self_,
                Holder::Expression(v) => v.self_,
                Holder::ConstantOp(v) => v.self_,
                Holder::CombinedImageSampler(v) => v.self_,
                Holder::AccessChain(v) => v.self_,
                Holder::Undef(v) => v.self_,
                Holder::String(v) => v.self_,
                Holder::DebugLocalVariable(v) => v.self_,
            },
        }
    }

    pub fn empty(&self) -> bool {
        self.holder.is_none()
    }

    pub fn reset(&mut self) {
        self.holder = None;
        self.type_ = TypeNone;
    }

    pub fn set_allow_type_rewrite(&mut self) {
        self.allow_type_rewrite = true;
    }
}

/// `variant_set<T>(var, args...)`.
pub fn variant_set<T: IVariant>(var: &mut Variant, val: T) -> Result<&mut T> {
    var.set(T::into_holder(Rc::new(val)), T::TYPE)?;
    var.get_mut::<T>()
}

/// `AccessChainMeta`.
#[derive(Clone, Copy, Debug, Default)]
pub struct AccessChainMeta {
    pub storage_physical_type: u32,
    pub need_transpose: bool,
    pub storage_is_packed: bool,
    pub storage_is_invariant: bool,
    pub flattened_struct: bool,
    pub relaxed_precision: bool,
    pub access_meshlet_position_y: bool,
    pub chain_is_builtin: bool,
    pub builtin: BuiltIn,
}

/// `enum ExtendedDecorations`.
pub type ExtendedDecorations = u32;
// Marks if a buffer block is re-packed, i.e. member declaration might be subject to PhysicalTypeID remapping and padding.
pub const SPIRVCrossDecorationBufferBlockRepacked: ExtendedDecorations = 0;

// A type in a buffer block might be declared with a different physical type than the logical type.
// If this is not set, PhysicalTypeID == the SPIR-V type as declared.
pub const SPIRVCrossDecorationPhysicalTypeID: ExtendedDecorations = 1;

// Marks if the physical type is to be declared with tight packing rules, i.e. packed_floatN on MSL and friends.
// If this is set, PhysicalTypeID might also be set. It can be set to same as logical type if all we're doing
// is converting float3 to packed_float3 for example.
// If this is marked on a struct, it means the struct itself must use only Packed types for all its members.
pub const SPIRVCrossDecorationPhysicalTypePacked: ExtendedDecorations = 2;

// The padding in bytes before declaring this struct member.
// If used on a struct type, marks the target size of a struct.
pub const SPIRVCrossDecorationPaddingTarget: ExtendedDecorations = 3;

pub const SPIRVCrossDecorationInterfaceMemberIndex: ExtendedDecorations = 4;
pub const SPIRVCrossDecorationInterfaceOrigID: ExtendedDecorations = 5;
pub const SPIRVCrossDecorationResourceIndexPrimary: ExtendedDecorations = 6;
// Used for decorations like resource indices for samplers when part of combined image samplers.
// A variable might need to hold two resource indices in this case.
pub const SPIRVCrossDecorationResourceIndexSecondary: ExtendedDecorations = 7;
// Used for resource indices for multiplanar images when part of combined image samplers.
pub const SPIRVCrossDecorationResourceIndexTertiary: ExtendedDecorations = 8;
pub const SPIRVCrossDecorationResourceIndexQuaternary: ExtendedDecorations = 9;

// Marks a buffer block for using explicit offsets (GLSL/HLSL).
pub const SPIRVCrossDecorationExplicitOffset: ExtendedDecorations = 10;

// Apply to a variable in the Input storage class; marks it as holding the base group passed to vkCmdDispatchBase(),
// or the base vertex and instance indices passed to vkCmdDrawIndexed().
// In MSL, this is used to adjust the WorkgroupId and GlobalInvocationId variables in compute shaders,
// and to hold the BaseVertex and BaseInstance variables in vertex shaders.
pub const SPIRVCrossDecorationBuiltInDispatchBase: ExtendedDecorations = 11;

// Apply to a variable that is a function parameter; marks it as being a "dynamic"
// combined image-sampler. In MSL, this is used when a function parameter might hold
// either a regular combined image-sampler or one that has an attached sampler
// Y'CbCr conversion.
pub const SPIRVCrossDecorationDynamicImageSampler: ExtendedDecorations = 12;

// Apply to a variable in the Input storage class; marks it as holding the size of the stage
// input grid.
// In MSL, this is used to hold the vertex and instance counts in a tessellation pipeline
// vertex shader.
pub const SPIRVCrossDecorationBuiltInStageInputSize: ExtendedDecorations = 13;

// Apply to any access chain of a tessellation I/O variable; stores the type of the sub-object
// that was chained to, as recorded in the input variable itself. This is used in case the pointer
// is itself used as the base of an access chain, to calculate the original type of the sub-object
// chained to, in case a swizzle needs to be applied. This should not happen normally with valid
// SPIR-V, but the MSL backend can change the type of input variables, necessitating the
// addition of swizzles to keep the generated code compiling.
pub const SPIRVCrossDecorationTessIOOriginalInputTypeID: ExtendedDecorations = 14;

// Apply to any access chain of an interface variable used with pull-model interpolation, where the variable is a
// vector but the resulting pointer is a scalar; stores the component index that is to be accessed by the chain.
// This is used when emitting calls to interpolation functions on the chain in MSL: in this case, the component
// must be applied to the result, since pull-model interpolants in MSL cannot be swizzled directly, but the
// results of interpolation can.
pub const SPIRVCrossDecorationInterpolantComponentExpr: ExtendedDecorations = 15;

// Apply to any struct type that is used in the Workgroup storage class.
// This causes matrices in MSL prior to Metal 3.0 to be emitted using a special
// class that is convertible to the standard matrix type, to work around the
// lack of constructors in the 'threadgroup' address space.
pub const SPIRVCrossDecorationWorkgroupStruct: ExtendedDecorations = 16;

pub const SPIRVCrossDecorationOverlappingBinding: ExtendedDecorations = 17;

pub const SPIRVCrossDecorationCount: ExtendedDecorations = 18;

/// `Meta::Decoration::Extended`.
#[derive(Clone, Debug, Default)]
pub struct DecorationExtended {
    pub flags: Bitset,
    pub values: [u32; SPIRVCrossDecorationCount as usize],
}

/// `Meta::Decoration`.
#[derive(Clone, Debug)]
pub struct MetaDecoration {
    pub alias: String,
    pub qualified_alias: String,
    pub user_semantic: String,
    pub user_type: String,
    pub decoration_flags: Bitset,
    pub builtin_type: BuiltIn,
    pub location: u32,
    pub component: u32,
    pub set: u32,
    pub binding: u32,
    pub offset: u32,
    pub offset_id: u32,
    pub xfb_buffer: u32,
    pub xfb_stride: u32,
    pub stream: u32,
    pub array_stride: u32,
    pub array_stride_id: u32,
    pub matrix_stride: u32,
    pub input_attachment: u32,
    pub spec_id: u32,
    pub index: u32,
    pub fp_rounding_mode: FPRoundingMode,
    pub fp_fast_math_mode: FPFastMathModeMask,
    pub builtin: bool,
    pub qualified_alias_explicit_override: bool,

    pub extended: DecorationExtended,
}

impl Default for MetaDecoration {
    fn default() -> Self {
        MetaDecoration {
            alias: String::new(),
            qualified_alias: String::new(),
            user_semantic: String::new(),
            user_type: String::new(),
            decoration_flags: Bitset::default(),
            builtin_type: BuiltInMax,
            location: 0,
            component: 0,
            set: 0,
            binding: 0,
            offset: 0,
            offset_id: 0,
            xfb_buffer: 0,
            xfb_stride: 0,
            stream: 0,
            array_stride: 0,
            array_stride_id: 0,
            matrix_stride: 0,
            input_attachment: 0,
            spec_id: 0,
            index: 0,
            fp_rounding_mode: FPRoundingModeMax,
            fp_fast_math_mode: FPFastMathModeMaskNone,
            builtin: false,
            qualified_alias_explicit_override: false,
            extended: DecorationExtended::default(),
        }
    }
}

/// `Meta`.
#[derive(Clone, Debug, Default)]
pub struct Meta {
    pub decoration: MetaDecoration,

    // Intentionally not a SmallVector. Decoration is large and somewhat rare.
    pub members: Vec<MetaDecoration>,

    pub decoration_word_offset: HashMap<u32, u32>,

    // For SPV_GOOGLE_hlsl_functionality1.
    pub hlsl_is_magic_counter_buffer: bool,
    // ID for the sibling counter buffer.
    pub hlsl_magic_counter_buffer: u32,
}

/// `VariableTypeRemapCallback`: a user callback that remaps the type of
/// any variable. var_name is the declared name of the variable.
/// name_of_type is the textual name of the type which will be used in the
/// code unless written to by the callback.
pub type VariableTypeRemapCallback = Rc<dyn Fn(&SPIRType, &str, &mut String)>;

/// `Hasher`.
#[derive(Clone, Copy, Debug)]
pub struct Hasher {
    h: u64,
}

impl Default for Hasher {
    fn default() -> Self {
        Hasher {
            h: 0xcbf29ce484222325,
        }
    }
}

impl Hasher {
    #[inline]
    pub fn u32(&mut self, value: u32) {
        self.h = self.h.wrapping_mul(0x100000001b3) ^ value as u64;
    }

    #[inline]
    pub fn get(&self) -> u64 {
        self.h
    }
}

pub fn type_is_floating_point(type_: &SPIRType) -> bool {
    use BaseType::*;
    matches!(
        type_.basetype,
        Half | Float | Double | BFloat16 | FloatE5M2 | FloatE4M3
    )
}

pub fn type_is_integral(type_: &SPIRType) -> bool {
    use BaseType::*;
    matches!(
        type_.basetype,
        SByte | UByte | Short | UShort | Int | UInt | Int64 | UInt64
    )
}

pub fn to_signed_basetype(width: u32) -> Result<BaseType> {
    Ok(match width {
        8 => BaseType::SByte,
        16 => BaseType::Short,
        32 => BaseType::Int,
        64 => BaseType::Int64,
        _ => spirv_cross_throw!("Invalid bit width."),
    })
}

pub fn to_unsigned_basetype(width: u32) -> Result<BaseType> {
    Ok(match width {
        8 => BaseType::UByte,
        16 => BaseType::UShort,
        32 => BaseType::UInt,
        64 => BaseType::UInt64,
        _ => spirv_cross_throw!("Invalid bit width."),
    })
}

// Returns true if an arithmetic operation does not change behavior depending on signedness.
pub fn opcode_is_sign_invariant(opcode: Op) -> bool {
    matches!(
        opcode,
        OpIEqual
            | OpINotEqual
            | OpISub
            | OpIAdd
            | OpIMul
            | OpShiftLeftLogical
            | OpBitwiseOr
            | OpBitwiseXor
            | OpBitwiseAnd
    )
}

pub fn opcode_can_promote_integer_implicitly(opcode: Op) -> bool {
    matches!(
        opcode,
        OpSNegate
            | OpNot
            | OpBitwiseAnd
            | OpBitwiseOr
            | OpBitwiseXor
            | OpShiftLeftLogical
            | OpShiftRightLogical
            | OpShiftRightArithmetic
            | OpIAdd
            | OpISub
            | OpIMul
            | OpSDiv
            | OpUDiv
            | OpSRem
            | OpUMod
            | OpSMod
    )
}

/// `SetBindingPair`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct SetBindingPair {
    pub desc_set: u32,
    pub binding: u32,
}

/// `LocationComponentPair`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct LocationComponentPair {
    pub location: u32,
    pub component: u32,
}

/// `StageSetBinding`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct StageSetBinding {
    pub model: ExecutionModel,
    pub desc_set: u32,
    pub binding: u32,
}

// Special constant used in a {MSL,HLSL}ResourceBinding desc_set
// element to indicate the bindings for the push constants.
pub const ResourceBindingPushConstantDescriptorSet: u32 = !0u32;

// Special constant used in a {MSL,HLSL}ResourceBinding binding
// element to indicate the bindings for the push constants.
pub const ResourceBindingPushConstantBinding: u32 = 0;

/// `ValueSaver`: C++ restores a value when the saver goes out of scope;
/// here the caller saves the value and restores it on every path.
pub(crate) struct ValueSaver;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn format_g_matches_printf() {
        // Values checked against glibc's printf("%.32g").
        assert_eq!(format_g(1.0, 32), "1");
        assert_eq!(format_g(0.5, 32), "0.5");
        assert_eq!(format_g(0.1f32 as f64, 32), "0.100000001490116119384765625");
        assert_eq!(format_g(1e-5, 32), "1.0000000000000000818030539140313e-05");
        assert_eq!(format_g(1e32, 32), "1.0000000000000000536616220439347e+32");
        assert_eq!(
            format_g(3.0e38f32 as f64, 32),
            "3.0000000054977557577780399428115e+38"
        );
        assert_eq!(format_g(-2.5, 32), "-2.5");
        assert_eq!(
            format_g(0.0001, 32),
            "0.00010000000000000000479217360238593"
        );
        assert_eq!(format_g(123456.0, 6), "123456");
        assert_eq!(format_g(1234567.0, 6), "1.23457e+06");
    }

    #[test]
    fn float_literals() {
        assert_eq!(convert_to_string_f32(1.0, '.'), "1.0");
        assert_eq!(
            convert_to_string_f32(2.0e20, '.'),
            "200000004008175468544.0"
        );
        assert_eq!(
            convert_to_string_f32(1.0e33, '.'),
            "9.9999999449572728642799288503501e+32"
        );
    }

    #[test]
    fn f16() {
        assert_eq!(SPIRConstant::f16_to_f32(0x3c00), 1.0);
        assert_eq!(SPIRConstant::f16_to_f32(0xc000), -2.0);
        assert_eq!(SPIRConstant::f16_to_f32(0x0001), 5.9604645e-8);
        assert!(SPIRConstant::f16_to_f32(0x7c00).is_infinite());
    }
}
