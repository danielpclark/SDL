// Rust translation of lib/jxl/field_encodings.h, lib/jxl/fields.h and
// lib/jxl/fields.cc from libjxl (https://github.com/libjxl/libjxl, at the
// revision SDL_image's external/libjxl pins: libjxl 0.7.3 with SDL's
// patches).
// Copyright (c) the JPEG XL Project Authors. All rights reserved.
// SPDX-License-Identifier: BSD-3-Clause (see LICENSE.txt)
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! Forward/backward-compatible 'bundles' with auto-serialized 'fields'.
//!
//! The decoder's visitors are translated (init, set-default, all-default
//! and read); the encoder's (max bits, can-encode, write) are not.
//! `Visitor::AllDefault()` doesn't take the bundle, which only the
//! encoder's CanEncodeVisitor reads.

use super::base::{
    jxl_failure, jxl_status, num0_bits_below_ls1_bit_nonzero_u64, safe_add, Status, StatusCode,
};
use super::dec_bit_reader::BitReader;

// --- field_encodings.h ---

/// Translation of `Fields`.
pub(crate) trait Fields {
    fn visit_fields(&mut self, visitor: &mut dyn Visitor) -> Status;
}

/// Distribution of U32 values for one particular selector. Represents either
/// a power of two-sized range, or a single value. A separate type ensures
/// this is only passed to the U32Enc ctor. Translation of `U32Distr`.
#[derive(Clone, Copy)]
pub(crate) struct U32Distr {
    d: u32,
}

impl U32Distr {
    const K_DIRECT: u32 = 0x80000000;

    const fn is_direct(self) -> bool {
        (self.d & Self::K_DIRECT) != 0
    }

    // Only call if IsDirect().
    const fn direct(self) -> u32 {
        self.d & (Self::K_DIRECT - 1)
    }

    // Only call if !IsDirect().
    const fn extra_bits(self) -> usize {
        ((self.d & 0x1F) + 1) as usize
    }
    const fn offset(self) -> u32 {
        (self.d >> 5) & 0x3FFFFFF
    }
}

/// A direct-coded 31-bit value occupying 2 bits in the bitstream.
/// Translation of `Val()`.
pub(crate) const fn val(value: u32) -> U32Distr {
    U32Distr {
        d: value | U32Distr::K_DIRECT,
    }
}

/// Value - `offset` will be signaled in `bits` extra bits. Translation of
/// `BitsOffset()`.
pub(crate) const fn bits_offset(bits: u32, offset: u32) -> U32Distr {
    U32Distr {
        d: ((bits - 1) & 0x1F) + ((offset & 0x3FFFFFF) << 5),
    }
}

/// Value will be signaled in `bits` extra bits. Translation of `Bits()`.
pub(crate) const fn bits(bits: u32) -> U32Distr {
    bits_offset(bits, 0)
}

/// See U32Coder documentation in fields.h. Translation of `U32Enc`.
#[derive(Clone, Copy)]
pub(crate) struct U32Enc {
    d: [U32Distr; 4],
}

impl U32Enc {
    pub(crate) const fn new(d0: U32Distr, d1: U32Distr, d2: U32Distr, d3: U32Distr) -> Self {
        U32Enc {
            d: [d0, d1, d2, d3],
        }
    }

    // Returns the U32Distr at `selector` = 0..3, least-significant first.
    fn get_distr(&self, selector: u32) -> U32Distr {
        self.d[selector as usize]
    }
}

/// Returns bit with the given `index` (0 = least significant). Translation
/// of `MakeBit()`.
pub(crate) const fn make_bit(index: u32) -> u64 {
    1u64 << index
}

/// The enums read through `Visitor::Enum()`: their name, the bit array of
/// their values (`EnumBits()`) and the conversions. (Translation of the
/// `EnumName()`/`EnumBits()` overloads.)
pub(crate) trait JxlEnum: Copy {
    const NAME: &'static str;
    fn enum_bits() -> u64;
    fn to_u32(self) -> u32;
    /// Only called with values `EnumValid()` accepts.
    fn from_u32(v: u32) -> Self;
}

/// Returns true if value is one of Values<Enum>(). Translation of
/// `EnumValid()`.
pub(crate) fn enum_valid<E: JxlEnum>(value: u32) -> Status {
    if value >= 64 {
        return jxl_failure!("Value {} too large for {}\n", value, E::NAME);
    }
    let bit = make_bit(value);
    if (E::enum_bits() & bit) == 0 {
        return jxl_failure!("Invalid value {} for {}\n", value, E::NAME);
    }
    Ok(())
}

// --- fields.h ---

// Integer coders: BitsCoder (raw), U32Coder (table), U64Coder (varint).

/// Reads/writes a given (fixed) number of bits <= 32. Translation of
/// `BitsCoder::Read()`.
fn bits_coder_read(bits: usize, reader: &mut BitReader<'_>) -> u32 {
    reader.read_bits(bits) as u32
}

/// Encodes u32 using a lookup table and/or extra bits, governed by a
/// per-field encoding `enc` which consists of four distributions `d` chosen
/// via a 2-bit selector (least significant = 0). Each d may have two modes:
/// - direct: if d.IsDirect(), the value is d.Direct();
/// - offset: the value is derived from d.ExtraBits() extra bits plus
///   d.Offset();
///
/// This encoding is denser than Exp-Golomb or Gamma codes when both small and
/// large values occur. Translation of `U32Coder::Read()`.
pub(crate) fn u32_coder_read(enc: U32Enc, reader: &mut BitReader<'_>) -> u32 {
    let selector = reader.read_fixed_bits::<2>() as u32;
    let d = enc.get_distr(selector);
    if d.is_direct() {
        d.direct()
    } else {
        (reader.read_bits(d.extra_bits()) as u32).wrapping_add(d.offset())
    }
}

/// Encodes 64-bit unsigned integers with a fixed distribution, taking 2 bits
/// to encode 0, 6 bits to encode 1 to 16, 10 bits to encode 17 to 272, 15
/// bits to encode up to 4095, and on the order of log2(value) * 1.125 bits
/// for larger values. Translation of `U64Coder::Read()`.
pub(crate) fn u64_coder_read(reader: &mut BitReader<'_>) -> u64 {
    let selector = reader.read_fixed_bits::<2>();
    if selector == 0 {
        return 0;
    }
    if selector == 1 {
        return 1 + reader.read_fixed_bits::<4>();
    }
    if selector == 2 {
        return 17 + reader.read_fixed_bits::<8>();
    }

    // selector 3, varint, groups have first 12, then 8, and last 4 bits.
    let mut result = reader.read_fixed_bits::<12>();

    let mut shift: u64 = 12;
    while reader.read_fixed_bits::<1>() != 0 {
        if shift == 60 {
            result |= reader.read_fixed_bits::<4>() << shift;
            break;
        }
        result |= reader.read_fixed_bits::<8>() << shift;
        shift += 8;
    }

    result
}

/// IEEE 754 half-precision (binary16). Refuses to read/write NaN/Inf.
/// Translation of `F16Coder::Read()`.
pub(crate) fn f16_coder_read(reader: &mut BitReader<'_>, value: &mut f32) -> Status {
    let bits16 = reader.read_fixed_bits::<16>() as u32;
    let sign = bits16 >> 15;
    let biased_exp = (bits16 >> 10) & 0x1F;
    let mantissa = bits16 & 0x3FF;

    if biased_exp == 31 {
        return jxl_failure!("F16 infinity or NaN are not supported");
    }

    // Subnormal or zero
    if biased_exp == 0 {
        *value = (1.0f32 / 16384.0) * (mantissa as f32 * (1.0f32 / 1024.0));
        if sign != 0 {
            *value = -*value;
        }
        return Ok(());
    }

    // Normalized: convert the representation directly (faster than ldexp/tables).
    let biased_exp32 = biased_exp + (127 - 15);
    let mantissa32 = mantissa << (23 - 10);
    let bits32 = (sign << 31) | (biased_exp32 << 23) | mantissa32;
    *value = f32::from_bits(bits32);
    Ok(())
}

// A "bundle" is a forward- and backward compatible collection of fields.
// They are used for SizeHeader/FrameHeader/GroupHeader. Bundles can be
// extended by appending(!) fields. Optional fields may be omitted from the
// bitstream by conditionally visiting them. When reading new bitstreams with
// old code, we skip unknown fields at the end of the bundle. This requires
// storing the amount of extra appended bits, and that fields are visited in
// chronological order of being added to the format, because old decoders
// cannot skip some future fields and resume reading old fields. Similarly,
// new readers query bits in an "extensions" field to skip (groups of) fields
// not present in old bitstreams. Note that each bundle must include an
// "extensions" field prior to freezing the format, otherwise it cannot be
// extended.
//
// To ensure interoperability, there will be no opaque fields.
//
// (The HOWTO for writing bundles is in upstream's fields.h.)

/// Translation of `Bundle::kMaxExtensions`.
pub(crate) const K_MAX_EXTENSIONS: usize = 64; // bits in u64

/// Different subclasses of Visitor are passed to implementations of Fields
/// throughout their lifetime. Translation of `Visitor` (with `VisitorBase`'s
/// state reached through `base()`).
pub(crate) trait Visitor {
    fn base(&mut self) -> &mut VisitorBase;

    fn visit(&mut self, fields: &mut dyn Fields) -> Status;

    fn bool_(&mut self, default_value: bool, value: &mut bool) -> Status {
        // (VisitorBase::Bool(), overridden by InitVisitor)
        let mut bits = if *value { 1 } else { 0 };
        self.bits(1, default_value as u32, &mut bits)?;
        *value = bits == 1;
        Ok(())
    }
    fn u32_(&mut self, enc: U32Enc, default_value: u32, value: &mut u32) -> Status;

    // Helper to construct U32Enc from U32Distr.
    fn u32d(
        &mut self,
        d0: U32Distr,
        d1: U32Distr,
        d2: U32Distr,
        d3: U32Distr,
        default_value: u32,
        value: &mut u32,
    ) -> Status {
        self.u32_(U32Enc::new(d0, d1, d2, d3), default_value, value)
    }

    fn bits(&mut self, bits: usize, default_value: u32, value: &mut u32) -> Status;
    fn u64_(&mut self, default_value: u64, value: &mut u64) -> Status;
    fn f16(&mut self, default_value: f32, value: &mut f32) -> Status;

    // Returns whether VisitFields should visit some subsequent fields.
    // "condition" is typically from prior fields, e.g. flags.
    // Overridden by InitVisitor and MaxBitsVisitor.
    fn conditional(&mut self, condition: bool) -> bool {
        condition
    }

    // Overridden by InitVisitor, AllDefaultVisitor and CanEncodeVisitor.
    // (Returns whether all fields are default: the Status as a bool.)
    fn all_default(&mut self, all_default: &mut bool) -> bool {
        if self.bool_(true, all_default).is_err() {
            return false;
        }
        *all_default
    }

    fn set_default(&mut self, _fields: &mut dyn Fields) {
        // Do nothing by default, this is overridden by ReadVisitor.
    }

    // Returns the result of visiting a nested Bundle.
    // Overridden by InitVisitor.
    fn visit_nested(&mut self, fields: &mut dyn Fields) -> Status {
        self.visit(fields)
    }

    // Overridden by ReadVisitor. Enables dynamically-sized fields.
    fn is_reading(&self) -> bool {
        false
    }

    fn begin_extensions(&mut self, extensions: &mut u64) -> Status {
        // (VisitorBase::BeginExtensions())
        self.u64_(0, extensions)?;

        self.base().extension_states.begin();
        Ok(())
    }
    fn end_extensions(&mut self) -> Status {
        // (VisitorBase::EndExtensions())
        self.base().extension_states.end();
        Ok(())
    }
}

/// Translation of `Visitor::Enum()`.
pub(crate) fn visit_enum<E: JxlEnum>(
    visitor: &mut dyn Visitor,
    default_value: E,
    value: &mut E,
) -> Status {
    let mut u32 = value.to_u32();
    // 00 -> 0
    // 01 -> 1
    // 10xxxx -> 2..17
    // 11yyyyyy -> 18..81
    visitor.u32d(
        val(0),
        val(1),
        bits_offset(4, 2),
        bits_offset(6, 18),
        default_value.to_u32(),
        &mut u32,
    )?;
    enum_valid::<E>(u32)?;
    *value = E::from_u32(u32);
    Ok(())
}

// --- fields.cc ---

/// A bundle can be in one of three states concerning extensions: not-begun,
/// active, ended. Bundles may be nested, so we need a stack of states.
/// Translation of `ExtensionStates`.
#[derive(Default)]
pub(crate) struct ExtensionStates {
    // Current state := least-significant bit of begun_ and ended_.
    begun: u64,
    ended: u64,
}

impl ExtensionStates {
    fn push(&mut self) {
        // Initial state = not-begun.
        self.begun <<= 1;
        self.ended <<= 1;
    }

    // Clears current state; caller must check IsEnded beforehand.
    fn pop(&mut self) {
        self.begun >>= 1;
        self.ended >>= 1;
    }

    // Returns true if state == active || state == ended.
    fn is_begun(&self) -> bool {
        (self.begun & 1) != 0
    }
    // Returns true if state != not-begun && state != active.
    fn is_ended(&self) -> bool {
        (self.ended & 1) != 0
    }

    fn begin(&mut self) {
        debug_assert!(!self.is_begun());
        debug_assert!(!self.is_ended());
        self.begun += 1;
    }

    fn end(&mut self) {
        debug_assert!(self.is_begun());
        debug_assert!(!self.is_ended());
        self.ended += 1;
    }
}

/// Visitors generate Init/AllDefault/Read/Write logic for all fields. Each
/// bundle's VisitFields member function calls visitor->U32 etc. We do not
/// overload operator() because a function name is easier to search for.
/// Translation of `VisitorBase`'s state.
#[derive(Default)]
pub(crate) struct VisitorBase {
    depth: usize, // to check nesting
    extension_states: ExtensionStates,
}

/// This is the only call site of Fields::VisitFields.
/// Ensures EndExtensions was called. Translation of `VisitorBase::Visit()`.
fn visitor_base_visit<V: Visitor>(visitor: &mut V, fields: &mut dyn Fields) -> Status {
    visitor.base().depth += 1;
    if visitor.base().depth > K_MAX_EXTENSIONS {
        // (JXL_ASSERT)
        return jxl_failure!("depth_ <= Bundle::kMaxExtensions");
    }
    visitor.base().extension_states.push();

    let ok = fields.visit_fields(visitor);

    if ok.is_ok() {
        // If VisitFields called BeginExtensions, must also call
        // EndExtensions.
        debug_assert!(
            !visitor.base().extension_states.is_begun()
                || visitor.base().extension_states.is_ended()
        );
    } else {
        // Failed, undefined state: don't care whether EndExtensions was
        // called.
    }

    visitor.base().extension_states.pop();
    visitor.base().depth -= 1;

    ok
}

/// Translation of `InitVisitor`.
#[derive(Default)]
struct InitVisitor {
    base: VisitorBase,
}

impl Visitor for InitVisitor {
    fn base(&mut self) -> &mut VisitorBase {
        &mut self.base
    }
    fn visit(&mut self, fields: &mut dyn Fields) -> Status {
        visitor_base_visit(self, fields)
    }

    fn bits(&mut self, _bits: usize, default_value: u32, value: &mut u32) -> Status {
        *value = default_value;
        Ok(())
    }

    fn u32_(&mut self, _enc: U32Enc, default_value: u32, value: &mut u32) -> Status {
        *value = default_value;
        Ok(())
    }

    fn u64_(&mut self, default_value: u64, value: &mut u64) -> Status {
        *value = default_value;
        Ok(())
    }

    fn bool_(&mut self, default_value: bool, value: &mut bool) -> Status {
        *value = default_value;
        Ok(())
    }

    fn f16(&mut self, default_value: f32, value: &mut f32) -> Status {
        *value = default_value;
        Ok(())
    }

    // Always visit conditional fields to ensure they are initialized.
    fn conditional(&mut self, _condition: bool) -> bool {
        true
    }

    fn all_default(&mut self, all_default: &mut bool) -> bool {
        // Just initialize this field and don't skip initializing others.
        let _ = self.bool_(true, all_default);
        false
    }

    fn visit_nested(&mut self, _fields: &mut dyn Fields) -> Status {
        // Avoid re-initializing nested bundles (their ctors already called
        // Bundle::Init for their fields).
        Ok(())
    }
}

/// Similar to InitVisitor, but also initializes nested fields. Translation
/// of `SetDefaultVisitor`.
#[derive(Default)]
struct SetDefaultVisitor {
    base: VisitorBase,
}

impl Visitor for SetDefaultVisitor {
    fn base(&mut self) -> &mut VisitorBase {
        &mut self.base
    }
    fn visit(&mut self, fields: &mut dyn Fields) -> Status {
        visitor_base_visit(self, fields)
    }

    fn bits(&mut self, _bits: usize, default_value: u32, value: &mut u32) -> Status {
        *value = default_value;
        Ok(())
    }

    fn u32_(&mut self, _enc: U32Enc, default_value: u32, value: &mut u32) -> Status {
        *value = default_value;
        Ok(())
    }

    fn u64_(&mut self, default_value: u64, value: &mut u64) -> Status {
        *value = default_value;
        Ok(())
    }

    fn bool_(&mut self, default_value: bool, value: &mut bool) -> Status {
        *value = default_value;
        Ok(())
    }

    fn f16(&mut self, default_value: f32, value: &mut f32) -> Status {
        *value = default_value;
        Ok(())
    }

    // Always visit conditional fields to ensure they are initialized.
    fn conditional(&mut self, _condition: bool) -> bool {
        true
    }

    fn all_default(&mut self, all_default: &mut bool) -> bool {
        // Just initialize this field and don't skip initializing others.
        let _ = self.bool_(true, all_default);
        false
    }
}

/// Translation of `AllDefaultVisitor`.
#[derive(Default)]
struct AllDefaultVisitor {
    base: VisitorBase,
    all_default: bool,
}

impl Visitor for AllDefaultVisitor {
    fn base(&mut self) -> &mut VisitorBase {
        &mut self.base
    }
    fn visit(&mut self, fields: &mut dyn Fields) -> Status {
        visitor_base_visit(self, fields)
    }

    fn bits(&mut self, _bits: usize, default_value: u32, value: &mut u32) -> Status {
        self.all_default &= *value == default_value;
        Ok(())
    }

    fn u32_(&mut self, _enc: U32Enc, default_value: u32, value: &mut u32) -> Status {
        self.all_default &= *value == default_value;
        Ok(())
    }

    fn u64_(&mut self, default_value: u64, value: &mut u64) -> Status {
        self.all_default &= *value == default_value;
        Ok(())
    }

    fn f16(&mut self, default_value: f32, value: &mut f32) -> Status {
        self.all_default &= (*value - default_value).abs() < 1E-6f32;
        Ok(())
    }

    fn all_default(&mut self, _all_default: &mut bool) -> bool {
        // Visit all fields so we can compute the actual all_default_ value.
        false
    }
}

/// Translation of `ReadVisitor`.
struct ReadVisitor<'r, 'a> {
    base: VisitorBase,
    // Whether any error other than not enough bytes occurred.
    ok: bool,

    // Whether there are enough input bytes to read from.
    enough_bytes: bool,
    reader: &'r mut BitReader<'a>,
    // May be 0 even if the corresponding extension is present.
    extension_bits: [u64; K_MAX_EXTENSIONS],
    total_extension_bits: u64,
    pos_after_ext_size: usize, // 0 iff extensions == 0.
}

impl<'r, 'a> ReadVisitor<'r, 'a> {
    fn new(reader: &'r mut BitReader<'a>) -> Self {
        ReadVisitor {
            base: VisitorBase::default(),
            ok: true,
            enough_bytes: true,
            reader,
            extension_bits: [0; K_MAX_EXTENSIONS],
            total_extension_bits: 0,
            pos_after_ext_size: 0,
        }
    }

    fn ok(&self) -> Status {
        if self.ok {
            Ok(())
        } else {
            Err(StatusCode::GenericError)
        }
    }
}

impl Visitor for ReadVisitor<'_, '_> {
    fn base(&mut self) -> &mut VisitorBase {
        &mut self.base
    }
    fn visit(&mut self, fields: &mut dyn Fields) -> Status {
        visitor_base_visit(self, fields)
    }

    fn bits(&mut self, bits: usize, _default_value: u32, value: &mut u32) -> Status {
        *value = bits_coder_read(bits, self.reader);
        if !self.reader.all_reads_within_bounds() {
            return jxl_status!(StatusCode::NotEnoughBytes, "Not enough bytes for header");
        }
        Ok(())
    }

    fn u32_(&mut self, dist: U32Enc, _default_value: u32, value: &mut u32) -> Status {
        *value = u32_coder_read(dist, self.reader);
        if !self.reader.all_reads_within_bounds() {
            return jxl_status!(StatusCode::NotEnoughBytes, "Not enough bytes for header");
        }
        Ok(())
    }

    fn u64_(&mut self, _default_value: u64, value: &mut u64) -> Status {
        *value = u64_coder_read(self.reader);
        if !self.reader.all_reads_within_bounds() {
            return jxl_status!(StatusCode::NotEnoughBytes, "Not enough bytes for header");
        }
        Ok(())
    }

    fn f16(&mut self, _default_value: f32, value: &mut f32) -> Status {
        self.ok &= f16_coder_read(self.reader, value).is_ok();
        if !self.reader.all_reads_within_bounds() {
            return jxl_status!(StatusCode::NotEnoughBytes, "Not enough bytes for header");
        }
        Ok(())
    }

    fn set_default(&mut self, fields: &mut dyn Fields) {
        bundle_set_default(fields);
    }

    fn is_reading(&self) -> bool {
        true
    }

    // This never fails because visitors are expected to keep reading until
    // EndExtensions, see comment there.
    fn begin_extensions(&mut self, extensions: &mut u64) -> Status {
        // (VisitorBase::BeginExtensions())
        self.u64_(0, extensions)?;
        self.base.extension_states.begin();
        if *extensions == 0 {
            return Ok(());
        }

        // For each nonzero bit, i.e. extension that is present:
        let mut remaining_extensions = *extensions;
        while remaining_extensions != 0 {
            let idx_extension = num0_bits_below_ls1_bit_nonzero_u64(remaining_extensions);
            // Read additional U64 (one per extension) indicating the number of bits
            // (allows skipping individual extensions).
            let mut bits = 0u64;
            self.u64_(0, &mut bits)?;
            self.extension_bits[idx_extension] = bits;
            let mut total = 0u64;
            if !safe_add(
                self.total_extension_bits,
                self.extension_bits[idx_extension],
                &mut total,
            ) {
                self.total_extension_bits = total;
                return jxl_failure!("Extension bits overflowed, invalid codestream");
            }
            self.total_extension_bits = total;
            remaining_extensions &= remaining_extensions - 1;
        }
        // Used by EndExtensions to skip past any _remaining_ extensions.
        self.pos_after_ext_size = self.reader.total_bits_consumed();
        debug_assert!(self.pos_after_ext_size != 0);
        Ok(())
    }

    fn end_extensions(&mut self) -> Status {
        // (VisitorBase::EndExtensions())
        self.base.extension_states.end();
        // Happens if extensions == 0: don't read size, done.
        if self.pos_after_ext_size == 0 {
            return Ok(());
        }

        // Not enough bytes as set by BeginExtensions or earlier. Do not return
        // this as an JXL_FAILURE or false (which can also propagate to error
        // through e.g. JXL_RETURN_IF_ERROR), since this may be used while
        // silently checking whether there are enough bytes. If this case must be
        // treated as an error, reader_>Close() will do this, just like is already
        // done for non-extension fields.
        if !self.enough_bytes {
            return Ok(());
        }

        // Skip new fields this (old?) decoder didn't know about, if any.
        let bits_read = self.reader.total_bits_consumed();
        let mut end = 0u64;
        if !safe_add(
            self.pos_after_ext_size as u64,
            self.total_extension_bits,
            &mut end,
        ) {
            return jxl_failure!("Invalid extension size, caused overflow");
        }
        if bits_read as u64 > end {
            return jxl_failure!("Read more extension bits than budgeted");
        }
        let remaining_bits = (end - bits_read as u64) as usize;
        if remaining_bits != 0 {
            // JXL_WARNING("Skipping %" PRIuS "-bit extension(s)", remaining_bits);
            self.reader.skip_bits(remaining_bits);
            if !self.reader.all_reads_within_bounds() {
                return jxl_status!(StatusCode::NotEnoughBytes, "Not enough bytes for header");
            }
        }
        Ok(())
    }
}

/// Initializes fields to the default values. It is not recursive to nested
/// fields, this function is intended to be called in the constructors so
/// each nested field will already Init itself. Translation of
/// `Bundle::Init()`.
pub(crate) fn bundle_init(fields: &mut dyn Fields) {
    let mut visitor = InitVisitor::default();
    if visitor.visit(fields).is_err() {
        // JXL_ABORT("Init should never fail");
        debug_assert!(false, "Init should never fail");
    }
}

/// Similar to Init, but recursive to nested fields. Translation of
/// `Bundle::SetDefault()`.
pub(crate) fn bundle_set_default(fields: &mut dyn Fields) {
    let mut visitor = SetDefaultVisitor::default();
    if visitor.visit(fields).is_err() {
        // JXL_ABORT("SetDefault should never fail");
        debug_assert!(false, "SetDefault should never fail");
    }
}

/// Returns whether ALL fields (including `extensions`, if present) are equal
/// to their default value. Translation of `Bundle::AllDefault()` (on a copy:
/// the visitor takes the bundle mutably).
#[allow(dead_code)]
pub(crate) fn bundle_all_default(fields: &mut dyn Fields) -> bool {
    let mut visitor = AllDefaultVisitor {
        base: VisitorBase::default(),
        all_default: true,
    };
    if visitor.visit(fields).is_err() {
        // JXL_ABORT("AllDefault should never fail");
        debug_assert!(false, "AllDefault should never fail");
    }
    visitor.all_default
}

/// Translation of `Bundle::Read()`.
pub(crate) fn bundle_read(reader: &mut BitReader<'_>, fields: &mut dyn Fields) -> Status {
    let mut visitor = ReadVisitor::new(reader);
    visitor.visit(fields)?;
    visitor.ok()
}

/// Returns whether enough bits are available to fully read this bundle using
/// Read. Also returns true in case of a codestream error (other than not
/// being large enough): that means enough bits are available to determine
/// there's an error, use Read to get such error status.
/// NOTE: this advances the BitReader, a different one pointing back at the
/// original bit position in the codestream must be created to use Read after
/// this. Translation of `Bundle::CanRead()`.
pub(crate) fn bundle_can_read(reader: &mut BitReader<'_>, fields: &mut dyn Fields) -> bool {
    let mut visitor = ReadVisitor::new(reader);
    let status = visitor.visit(fields);
    // We are only checking here whether there are enough bytes. We still return
    // true for other errors because it means there are enough bytes to determine
    // there's an error. Use Read() to determine which error it is.
    status != Err(StatusCode::NotEnoughBytes)
}
