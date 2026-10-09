// Rust translation of src/hb-buffer.h, src/hb-buffer.hh, src/hb-buffer.cc
// and the UTF-8, UTF-32 and Latin-1 decoders of src/hb-utf.hh from
// HarfBuzz (8.5.0, as SDL_ttf's external/harfbuzz pins it).
// Copyright © 1998-2004  David Turner and Werner Lemberg
// Copyright © 2004,2007,2009,2010  Red Hat, Inc.
// Copyright © 2011,2012  Google, Inc.
// This is an altered (translated) version of the original software; see
// LICENSE.txt (HarfBuzz's "Old MIT" license).
//
// Red Hat Author(s): Owen Taylor, Behdad Esfahbod
// Google Author(s): Behdad Esfahbod

//! Input and output buffers.
//!
//! Translation notes: the buffer's arrays are `Vec`s of the allocated
//! size (C's `allocated`, which grows as in C, so that the length limits
//! behave the same). C lets `out_info` alias `info` (in-place operation)
//! or move to the memory of `pos`; here it moves to a separate array of
//! the same size (`out_info_sep`), with `separate_out` telling which is in
//! use. The message callback, user data and the immutable state are not
//! translated.

use super::hb_common::*;
use super::hb_unicode::*;

/* hb_glyph_info_t */

/// `hb_glyph_info_t`: holds information about the glyphs and their
/// relation to input text.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct HbGlyphInfo {
    /// either a Unicode code point (before shaping) or a glyph index
    /// (after shaping).
    pub codepoint: HbCodepoint,
    /*< private >*/
    pub(crate) mask: HbMask,
    /// the index of the character in the original text that corresponds
    /// to this `hb_glyph_info_t`, or whatever the client passes to
    /// `hb_buffer_add()`.
    pub cluster: u32,

    /*< private >*/
    pub(crate) var1: u32,
    pub(crate) var2: u32,
}

/// `hb_glyph_flags_t`
pub type HbGlyphFlags = u32;
/// `HB_GLYPH_FLAG_UNSAFE_TO_BREAK`
pub const HB_GLYPH_FLAG_UNSAFE_TO_BREAK: u32 = 0x00000001;
/// `HB_GLYPH_FLAG_UNSAFE_TO_CONCAT`
pub const HB_GLYPH_FLAG_UNSAFE_TO_CONCAT: u32 = 0x00000002;
/// `HB_GLYPH_FLAG_SAFE_TO_INSERT_TATWEEL`
pub const HB_GLYPH_FLAG_SAFE_TO_INSERT_TATWEEL: u32 = 0x00000004;
/// `HB_GLYPH_FLAG_DEFINED`: OR of all defined flags
pub const HB_GLYPH_FLAG_DEFINED: u32 = 0x00000007;

/// `hb_glyph_info_get_glyph_flags`
#[inline]
pub fn hb_glyph_info_get_glyph_flags(info: &HbGlyphInfo) -> HbGlyphFlags {
    info.mask & HB_GLYPH_FLAG_DEFINED
}

/// The `hb_var_int_t` views of `var1` and `var2` (`u8[4]`, `u16[2]`,
/// `i16[2]`; little-endian, as on the platforms SDL_ttf is tested on: the
/// byte order only matters for code that reads a view other than the one
/// written, which HarfBuzz does not do).
macro_rules! var_views {
    ($var:ident, $get_u8:ident, $set_u8:ident, $get_u16:ident, $set_u16:ident) => {
        #[inline]
        pub(crate) fn $get_u8(&self, i: usize) -> u8 {
            (self.$var >> (8 * i)) as u8
        }
        #[inline]
        pub(crate) fn $set_u8(&mut self, i: usize, v: u8) {
            self.$var = (self.$var & !(0xFF << (8 * i))) | ((v as u32) << (8 * i));
        }
        #[inline]
        pub(crate) fn $get_u16(&self, i: usize) -> u16 {
            (self.$var >> (16 * i)) as u16
        }
        #[inline]
        pub(crate) fn $set_u16(&mut self, i: usize, v: u16) {
            self.$var = (self.$var & !(0xFFFF << (16 * i))) | ((v as u32) << (16 * i));
        }
    };
}

impl HbGlyphInfo {
    var_views!(var1, var1_u8, set_var1_u8, var1_u16, set_var1_u16);
    var_views!(var2, var2_u8, set_var2_u8, var2_u16, set_var2_u16);

    /* buffer var allocations, used during the entire shaping process */
    /// `unicode_props()`: `var2.u16[0]`
    #[inline]
    pub(crate) fn unicode_props(&self) -> u32 {
        self.var2_u16(0) as u32
    }
    #[inline]
    pub(crate) fn set_unicode_props(&mut self, v: u32) {
        self.set_var2_u16(0, v as u16)
    }

    /* buffer var allocations, used during the GSUB/GPOS processing */
    /// `glyph_props()`: `var1.u16[0]` (GDEF glyph properties)
    #[inline]
    pub(crate) fn glyph_props(&self) -> u32 {
        self.var1_u16(0) as u32
    }
    #[inline]
    pub(crate) fn set_glyph_props(&mut self, v: u32) {
        self.set_var1_u16(0, v as u16)
    }
    /// `lig_props()`: `var1.u8[2]` (GSUB/GPOS ligature tracking)
    #[inline]
    pub(crate) fn lig_props(&self) -> u32 {
        self.var1_u8(2) as u32
    }
    #[inline]
    pub(crate) fn set_lig_props(&mut self, v: u32) {
        self.set_var1_u8(2, v as u8)
    }
    /// `syllable()`: `var1.u8[3]` (GSUB/GPOS shaping boundaries)
    #[inline]
    pub(crate) fn syllable(&self) -> u8 {
        self.var1_u8(3)
    }
    #[inline]
    pub(crate) fn set_syllable(&mut self, v: u8) {
        self.set_var1_u8(3, v)
    }

    /* (the shapers' variables) */
    /// `complex_var_u8_category()`: `var2.u8[2]`
    #[inline]
    pub(crate) fn complex_var_u8_category(&self) -> u8 {
        self.var2_u8(2)
    }
    #[inline]
    pub(crate) fn set_complex_var_u8_category(&mut self, v: u8) {
        self.set_var2_u8(2, v)
    }
    /// `complex_var_u8_auxiliary()`: `var2.u8[3]`
    #[inline]
    pub(crate) fn complex_var_u8_auxiliary(&self) -> u8 {
        self.var2_u8(3)
    }
    #[inline]
    pub(crate) fn set_complex_var_u8_auxiliary(&mut self, v: u8) {
        self.set_var2_u8(3, v)
    }
}

/* hb_glyph_position_t */

/// `hb_glyph_position_t`: holds the positions of the glyph in both
/// horizontal and vertical directions.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct HbGlyphPosition {
    /// how much the line advances after drawing this glyph when setting
    /// text in horizontal direction.
    pub x_advance: HbPosition,
    /// how much the line advances after drawing this glyph when setting
    /// text in vertical direction.
    pub y_advance: HbPosition,
    /// how much the glyph moves on the X-axis before drawing it.
    pub x_offset: HbPosition,
    /// how much the glyph moves on the Y-axis before drawing it.
    pub y_offset: HbPosition,

    /*< private >*/
    pub(crate) var: u32,
}

impl HbGlyphPosition {
    /// `attach_chain()`: `var.i16[0]` (the index of the glyph this one
    /// attaches to, relative to it)
    #[inline]
    pub(crate) fn attach_chain(&self) -> i16 {
        self.var as u16 as i16
    }
    #[inline]
    pub(crate) fn set_attach_chain(&mut self, v: i16) {
        self.var = (self.var & 0xFFFF0000) | (v as u16 as u32);
    }
    /// `attach_type()`: `var.u8[2]`
    #[inline]
    pub(crate) fn attach_type(&self) -> u8 {
        (self.var >> 16) as u8
    }
    #[inline]
    pub(crate) fn set_attach_type(&mut self, v: u8) {
        self.var = (self.var & 0xFF00FFFF) | ((v as u32) << 16);
    }
}

/// `hb_buffer_content_type_t`
pub type HbBufferContentType = u32;
/// Initial value for new buffer.
pub const HB_BUFFER_CONTENT_TYPE_INVALID: u32 = 0;
/// The buffer contains input characters (before shaping).
pub const HB_BUFFER_CONTENT_TYPE_UNICODE: u32 = 1;
/// The buffer contains output glyphs (after shaping).
pub const HB_BUFFER_CONTENT_TYPE_GLYPHS: u32 = 2;

/// `hb_buffer_flags_t`
pub type HbBufferFlags = u32;
/// the default buffer flag.
pub const HB_BUFFER_FLAG_DEFAULT: u32 = 0x00000000;
/// Beginning-of-text
pub const HB_BUFFER_FLAG_BOT: u32 = 0x00000001;
/// End-of-text
pub const HB_BUFFER_FLAG_EOT: u32 = 0x00000002;
/// `HB_BUFFER_FLAG_PRESERVE_DEFAULT_IGNORABLES`
pub const HB_BUFFER_FLAG_PRESERVE_DEFAULT_IGNORABLES: u32 = 0x00000004;
/// `HB_BUFFER_FLAG_REMOVE_DEFAULT_IGNORABLES`
pub const HB_BUFFER_FLAG_REMOVE_DEFAULT_IGNORABLES: u32 = 0x00000008;
/// `HB_BUFFER_FLAG_DO_NOT_INSERT_DOTTED_CIRCLE`
pub const HB_BUFFER_FLAG_DO_NOT_INSERT_DOTTED_CIRCLE: u32 = 0x00000010;
/// `HB_BUFFER_FLAG_VERIFY`
pub const HB_BUFFER_FLAG_VERIFY: u32 = 0x00000020;
/// `HB_BUFFER_FLAG_PRODUCE_UNSAFE_TO_CONCAT`
pub const HB_BUFFER_FLAG_PRODUCE_UNSAFE_TO_CONCAT: u32 = 0x00000040;
/// `HB_BUFFER_FLAG_PRODUCE_SAFE_TO_INSERT_TATWEEL`
pub const HB_BUFFER_FLAG_PRODUCE_SAFE_TO_INSERT_TATWEEL: u32 = 0x00000080;
/// All currently defined flags.
pub const HB_BUFFER_FLAG_DEFINED: u32 = 0x000000FF;

/// `hb_buffer_cluster_level_t`
pub type HbBufferClusterLevel = u32;
/// `HB_BUFFER_CLUSTER_LEVEL_MONOTONE_GRAPHEMES`
pub const HB_BUFFER_CLUSTER_LEVEL_MONOTONE_GRAPHEMES: u32 = 0;
/// `HB_BUFFER_CLUSTER_LEVEL_MONOTONE_CHARACTERS`
pub const HB_BUFFER_CLUSTER_LEVEL_MONOTONE_CHARACTERS: u32 = 1;
/// `HB_BUFFER_CLUSTER_LEVEL_CHARACTERS`
pub const HB_BUFFER_CLUSTER_LEVEL_CHARACTERS: u32 = 2;
/// `HB_BUFFER_CLUSTER_LEVEL_DEFAULT`
pub const HB_BUFFER_CLUSTER_LEVEL_DEFAULT: u32 = HB_BUFFER_CLUSTER_LEVEL_MONOTONE_GRAPHEMES;

/// `HB_BUFFER_REPLACEMENT_CODEPOINT_DEFAULT`
pub const HB_BUFFER_REPLACEMENT_CODEPOINT_DEFAULT: HbCodepoint = 0xFFFD;

/* hb-limits.hh */
pub(crate) const HB_BUFFER_MAX_LEN_FACTOR: u32 = 64;
pub(crate) const HB_BUFFER_MAX_LEN_MIN: u32 = 16384;
pub(crate) const HB_BUFFER_MAX_LEN_DEFAULT: u32 = 0x3FFFFFFF; /* Shaping more than a billion chars? Let us know! */
pub(crate) const HB_BUFFER_MAX_OPS_FACTOR: u32 = 1024;
pub(crate) const HB_BUFFER_MAX_OPS_MIN: u32 = 16384;
pub(crate) const HB_BUFFER_MAX_OPS_DEFAULT: i32 = 0x1FFFFFFF; /* Shaping more than a billion operations? Let us know! */

/// `hb_buffer_scratch_flags_t`
pub(crate) type HbBufferScratchFlags = u32;
pub(crate) const HB_BUFFER_SCRATCH_FLAG_DEFAULT: u32 = 0x00000000;
pub(crate) const HB_BUFFER_SCRATCH_FLAG_HAS_NON_ASCII: u32 = 0x00000001;
pub(crate) const HB_BUFFER_SCRATCH_FLAG_HAS_DEFAULT_IGNORABLES: u32 = 0x00000002;
pub(crate) const HB_BUFFER_SCRATCH_FLAG_HAS_SPACE_FALLBACK: u32 = 0x00000004;
pub(crate) const HB_BUFFER_SCRATCH_FLAG_HAS_GPOS_ATTACHMENT: u32 = 0x00000008;
pub(crate) const HB_BUFFER_SCRATCH_FLAG_HAS_CGJ: u32 = 0x00000010;
pub(crate) const HB_BUFFER_SCRATCH_FLAG_HAS_GLYPH_FLAGS: u32 = 0x00000020;
pub(crate) const HB_BUFFER_SCRATCH_FLAG_HAS_BROKEN_SYLLABLE: u32 = 0x00000040;

/* Reserved for shapers' internal use. */
pub(crate) const HB_BUFFER_SCRATCH_FLAG_SHAPER0: u32 = 0x01000000;
pub(crate) const HB_BUFFER_SCRATCH_FLAG_SHAPER1: u32 = 0x02000000;
pub(crate) const HB_BUFFER_SCRATCH_FLAG_SHAPER2: u32 = 0x04000000;
pub(crate) const HB_BUFFER_SCRATCH_FLAG_SHAPER3: u32 = 0x08000000;

/// `hb_buffer_t`: the main structure holding the input text and its
/// properties before shaping, and output glyphs and their information
/// after shaping.
#[derive(Debug, Clone)]
pub struct HbBuffer {
    /*
     * Information about how the text in the buffer should be treated.
     */
    pub(crate) unicode: HbUnicodeFuncs, /* Unicode functions */
    pub(crate) flags: HbBufferFlags,    /* BOT / EOT / etc. */
    pub(crate) cluster_level: HbBufferClusterLevel,
    pub(crate) replacement: HbCodepoint, /* U+FFFD or something else. */
    pub(crate) invisible: HbCodepoint,   /* 0 or something else. */
    pub(crate) not_found: HbCodepoint,   /* 0 or something else. */

    /*
     * Buffer contents
     */
    pub(crate) content_type: HbBufferContentType,
    pub(crate) props: HbSegmentProperties, /* Script, language, direction */

    pub(crate) successful: bool,     /* Allocations successful */
    pub(crate) shaping_failed: bool, /* Shaping failure */
    pub(crate) have_output: bool,    /* Whether we have an output buffer going on */
    pub(crate) have_positions: bool, /* Whether we have positions */

    pub(crate) idx: u32,     /* Cursor into ->info and ->pos arrays */
    pub(crate) len: u32,     /* Length of ->info and ->pos arrays */
    pub(crate) out_len: u32, /* Length of ->out_info array if have_output */

    pub(crate) allocated: u32, /* Length of allocated arrays */
    pub(crate) info: Vec<HbGlyphInfo>,
    /// the separate output array (C's `out_info` once it no longer
    /// aliases `info`)
    pub(crate) out_info_sep: Vec<HbGlyphInfo>,
    /// whether `out_info` is `out_info_sep` (C's `out_info != info`)
    pub(crate) separate_out: bool,
    pub(crate) pos: Vec<HbGlyphPosition>,

    /* Text before / after the main buffer contents.
     * Always in Unicode, and ordered outward.
     * Index 0 is for "pre-context", 1 for "post-context". */
    pub(crate) context: [[HbCodepoint; HbBuffer::CONTEXT_LENGTH]; 2],
    pub(crate) context_len: [u32; 2],

    /*
     * Managed by enter / leave
     */
    pub(crate) allocated_var_bits: u8,
    pub(crate) serial: u8,
    pub(crate) random_state: u32,
    pub(crate) scratch_flags: HbBufferScratchFlags, /* Have space-fallback, etc. */
    pub(crate) max_len: u32,                        /* Maximum allowed len. */
    pub(crate) max_ops: i32,                        /* Maximum allowed operations. */
}

impl Default for HbBuffer {
    fn default() -> Self {
        HbBuffer::new()
    }
}

/// The UTF decoders of hb-utf.hh used by `hb_buffer_add_utf`.
pub(crate) trait HbUtf {
    /// `codepoint_t`
    type CodepointT: Copy;
    /// `next`
    fn next(
        text: &[Self::CodepointT],
        i: usize,
        end: usize,
        unicode: &mut HbCodepoint,
        replacement: HbCodepoint,
    ) -> usize;
    /// `prev`
    fn prev(
        text: &[Self::CodepointT],
        i: usize,
        start: usize,
        unicode: &mut HbCodepoint,
        replacement: HbCodepoint,
    ) -> usize;
}

/// `hb_utf8_t`
pub(crate) struct HbUtf8;

impl HbUtf for HbUtf8 {
    type CodepointT = u8;

    fn next(
        text: &[u8],
        mut i: usize,
        end: usize,
        unicode: &mut HbCodepoint,
        replacement: HbCodepoint,
    ) -> usize {
        /* Written to only accept well-formed sequences.
         * Based on ideas from ICU's U8_NEXT.
         * Generates one "replacement" for each ill-formed byte. */

        let mut c = text[i] as u32;
        i += 1;

        'error: {
            if c > 0x7F {
                if (0xC2..=0xDF).contains(&c) {
                    /* Two-byte */
                    let t1;
                    if i < end && {
                        t1 = (text[i] as u32).wrapping_sub(0x80);
                        t1 <= 0x3F
                    } {
                        c = ((c & 0x1F) << 6) | t1;
                        i += 1;
                    } else {
                        break 'error;
                    }
                } else if (0xE0..=0xEF).contains(&c) {
                    /* Three-byte */
                    let (t1, t2);
                    if 1 < end - i && {
                        t1 = (text[i] as u32).wrapping_sub(0x80);
                        t2 = (text[i + 1] as u32).wrapping_sub(0x80);
                        t1 <= 0x3F && t2 <= 0x3F
                    } {
                        c = ((c & 0xF) << 12) | (t1 << 6) | t2;
                        if c < 0x0800 || (0xD800..=0xDFFF).contains(&c) {
                            break 'error;
                        }
                        i += 2;
                    } else {
                        break 'error;
                    }
                } else if (0xF0..=0xF4).contains(&c) {
                    /* Four-byte */
                    let (t1, t2, t3);
                    if 2 < end - i && {
                        t1 = (text[i] as u32).wrapping_sub(0x80);
                        t2 = (text[i + 1] as u32).wrapping_sub(0x80);
                        t3 = (text[i + 2] as u32).wrapping_sub(0x80);
                        t1 <= 0x3F && t2 <= 0x3F && t3 <= 0x3F
                    } {
                        c = ((c & 0x7) << 18) | (t1 << 12) | (t2 << 6) | t3;
                        if !(0x10000..=0x10FFFF).contains(&c) {
                            break 'error;
                        }
                        i += 3;
                    } else {
                        break 'error;
                    }
                } else {
                    break 'error;
                }
            }

            *unicode = c;
            return i;
        }

        *unicode = replacement;
        i
    }

    fn prev(
        text: &[u8],
        i: usize,
        start: usize,
        unicode: &mut HbCodepoint,
        replacement: HbCodepoint,
    ) -> usize {
        let end = i;
        let mut i = i - 1;
        while start < i && (text[i] & 0xc0) == 0x80 && end - i < 4 {
            i -= 1;
        }

        if Self::next(text, i, end, unicode, replacement) == end {
            return i;
        }

        *unicode = replacement;
        end - 1
    }
}

/// `hb_utf32_novalidate_t` (`hb_buffer_add_codepoints`)
pub(crate) struct HbUtf32Novalidate;

impl HbUtf for HbUtf32Novalidate {
    type CodepointT = u32;

    fn next(
        text: &[u32],
        i: usize,
        _end: usize,
        unicode: &mut HbCodepoint,
        _replacement: HbCodepoint,
    ) -> usize {
        *unicode = text[i];
        i + 1
    }

    fn prev(
        text: &[u32],
        i: usize,
        _start: usize,
        unicode: &mut HbCodepoint,
        _replacement: HbCodepoint,
    ) -> usize {
        *unicode = text[i - 1];
        i - 1
    }
}

/// `hb_utf32_t` (`hb_buffer_add_utf32`: validating)
pub(crate) struct HbUtf32;

impl HbUtf for HbUtf32 {
    type CodepointT = u32;

    fn next(
        text: &[u32],
        i: usize,
        _end: usize,
        unicode: &mut HbCodepoint,
        replacement: HbCodepoint,
    ) -> usize {
        let c = text[i];
        *unicode = c;
        if c >= 0xD800 && (c <= 0xDFFF || c > 0x10FFFF) {
            *unicode = replacement;
        }
        i + 1
    }

    fn prev(
        text: &[u32],
        i: usize,
        _start: usize,
        unicode: &mut HbCodepoint,
        replacement: HbCodepoint,
    ) -> usize {
        let c = text[i - 1];
        *unicode = c;
        if c >= 0xD800 && (c <= 0xDFFF || c > 0x10FFFF) {
            *unicode = replacement;
        }
        i - 1
    }
}

/// `hb_latin1_t`
pub(crate) struct HbLatin1;

impl HbUtf for HbLatin1 {
    type CodepointT = u8;

    fn next(
        text: &[u8],
        i: usize,
        _end: usize,
        unicode: &mut HbCodepoint,
        _replacement: HbCodepoint,
    ) -> usize {
        *unicode = text[i] as u32;
        i + 1
    }

    fn prev(
        text: &[u8],
        i: usize,
        _start: usize,
        unicode: &mut HbCodepoint,
        _replacement: HbCodepoint,
    ) -> usize {
        *unicode = text[i - 1] as u32;
        i - 1
    }
}

impl HbBuffer {
    pub(crate) const CONTEXT_LENGTH: usize = 5;

    /// Creates a new `hb_buffer_t` with all properties to defaults
    /// (`hb_buffer_create`).
    pub fn new() -> HbBuffer {
        let mut buffer = HbBuffer {
            unicode: HbUnicodeFuncs,
            flags: HB_BUFFER_FLAG_DEFAULT,
            cluster_level: HB_BUFFER_CLUSTER_LEVEL_DEFAULT,
            replacement: HB_BUFFER_REPLACEMENT_CODEPOINT_DEFAULT,
            invisible: 0,
            not_found: 0,
            content_type: HB_BUFFER_CONTENT_TYPE_INVALID,
            props: HB_SEGMENT_PROPERTIES_DEFAULT,
            successful: true,
            shaping_failed: false,
            have_output: false,
            have_positions: false,
            idx: 0,
            len: 0,
            out_len: 0,
            allocated: 0,
            info: Vec::new(),
            out_info_sep: Vec::new(),
            separate_out: false,
            pos: Vec::new(),
            context: [[0; HbBuffer::CONTEXT_LENGTH]; 2],
            context_len: [0; 2],
            allocated_var_bits: 0,
            serial: 0,
            random_state: 1,
            scratch_flags: HB_BUFFER_SCRATCH_FLAG_DEFAULT,
            max_len: HB_BUFFER_MAX_LEN_DEFAULT,
            max_ops: HB_BUFFER_MAX_OPS_DEFAULT,
        };

        buffer.reset();

        buffer
    }

    /* Methods */

    #[inline]
    pub(crate) fn in_error(&self) -> bool {
        !self.successful
    }

    pub(crate) fn allocate_var(&mut self, start: u32, count: u32) {
        let end = start + count;
        let bits = ((1u32 << end) - (1u32 << start)) as u8;
        self.allocated_var_bits |= bits;
    }
    pub(crate) fn try_allocate_var(&mut self, start: u32, count: u32) -> bool {
        let end = start + count;
        let bits = ((1u32 << end) - (1u32 << start)) as u8;
        if self.allocated_var_bits & bits != 0 {
            return false;
        }
        self.allocated_var_bits |= bits;
        true
    }
    pub(crate) fn deallocate_var(&mut self, start: u32, count: u32) {
        let end = start + count;
        let bits = ((1u32 << end) - (1u32 << start)) as u8;
        self.allocated_var_bits &= !bits;
    }
    #[inline]
    pub(crate) fn assert_var(&self, _start: u32, _count: u32) {}
    pub(crate) fn deallocate_var_all(&mut self) {
        self.allocated_var_bits = 0;
    }

    /// `cur (i)`
    #[inline]
    pub(crate) fn cur(&self, i: u32) -> &HbGlyphInfo {
        &self.info[(self.idx + i) as usize]
    }
    #[inline]
    pub(crate) fn cur_mut(&mut self, i: u32) -> &mut HbGlyphInfo {
        let idx = (self.idx + i) as usize;
        &mut self.info[idx]
    }

    /// `cur_pos (i)`
    #[inline]
    pub(crate) fn cur_pos(&self, i: u32) -> &HbGlyphPosition {
        &self.pos[(self.idx + i) as usize]
    }
    #[inline]
    pub(crate) fn cur_pos_mut(&mut self, i: u32) -> &mut HbGlyphPosition {
        let idx = (self.idx + i) as usize;
        &mut self.pos[idx]
    }

    /// `out_info[i]`
    #[inline]
    pub(crate) fn out_info(&self, i: u32) -> &HbGlyphInfo {
        if self.separate_out {
            &self.out_info_sep[i as usize]
        } else {
            &self.info[i as usize]
        }
    }
    #[inline]
    pub(crate) fn out_info_mut(&mut self, i: u32) -> &mut HbGlyphInfo {
        if self.separate_out {
            &mut self.out_info_sep[i as usize]
        } else {
            &mut self.info[i as usize]
        }
    }
    /// The output array (C's `out_info` pointer) and its length
    /// allocation.
    #[inline]
    pub(crate) fn out_infos_mut(&mut self) -> &mut [HbGlyphInfo] {
        if self.separate_out {
            &mut self.out_info_sep
        } else {
            &mut self.info
        }
    }

    /// `prev ()`
    #[inline]
    pub(crate) fn prev(&self) -> &HbGlyphInfo {
        self.out_info(if self.out_len != 0 {
            self.out_len - 1
        } else {
            0
        })
    }
    #[inline]
    pub(crate) fn prev_mut(&mut self) -> &mut HbGlyphInfo {
        let i = if self.out_len != 0 {
            self.out_len - 1
        } else {
            0
        };
        self.out_info_mut(i)
    }

    /// `backtrack_len`
    #[inline]
    pub(crate) fn backtrack_len(&self) -> u32 {
        if self.have_output {
            self.out_len
        } else {
            self.idx
        }
    }
    /// `lookahead_len`
    #[inline]
    pub(crate) fn lookahead_len(&self) -> u32 {
        self.len - self.idx
    }
    /// `next_serial`
    #[inline]
    pub(crate) fn next_serial(&mut self) -> u8 {
        self.serial = self.serial.wrapping_add(1);
        if self.serial != 0 {
            self.serial
        } else {
            self.serial = self.serial.wrapping_add(1);
            self.serial
        }
    }

    /// `reverse_range`
    pub(crate) fn reverse_range(&mut self, start: u32, end: u32) {
        if end <= start {
            return;
        }
        self.info[start as usize..end as usize].reverse();
        if self.have_positions {
            self.pos[start as usize..end as usize].reverse();
        }
    }
    /// `reverse`
    pub(crate) fn reverse(&mut self) {
        self.reverse_range(0, self.len)
    }

    /// `reverse_groups`
    pub(crate) fn reverse_groups(
        &mut self,
        group: impl Fn(&HbGlyphInfo, &HbGlyphInfo) -> bool,
        merge_clusters: bool,
    ) {
        if self.len == 0 {
            return;
        }

        let mut start = 0;
        let mut i = 1;
        while i < self.len {
            if !group(&self.info[(i - 1) as usize], &self.info[i as usize]) {
                if merge_clusters {
                    self.merge_clusters(start, i);
                }
                self.reverse_range(start, i);
                start = i;
            }
            i += 1;
        }
        if merge_clusters {
            self.merge_clusters(start, i);
        }
        self.reverse_range(start, i);

        self.reverse();
    }

    /// `group_end`
    pub(crate) fn group_end(
        &self,
        mut start: u32,
        group: impl Fn(&HbGlyphInfo, &HbGlyphInfo) -> bool,
    ) -> u32 {
        loop {
            start += 1;
            if !(start < self.len
                && group(&self.info[(start - 1) as usize], &self.info[start as usize]))
            {
                break;
            }
        }

        start
    }

    /// `_cluster_group_func`
    #[inline]
    pub(crate) fn _cluster_group_func(a: &HbGlyphInfo, b: &HbGlyphInfo) -> bool {
        a.cluster == b.cluster
    }

    /// `reverse_clusters`
    pub(crate) fn reverse_clusters(&mut self) {
        self.reverse_groups(HbBuffer::_cluster_group_func, false)
    }

    /// `replace_glyphs`
    pub(crate) fn replace_glyphs(
        &mut self,
        num_in: u32,
        num_out: u32,
        glyph_data: &[HbCodepoint],
    ) -> bool {
        if !self.make_room_for(num_in, num_out) {
            return false;
        }

        self.merge_clusters(self.idx, self.idx + num_in);

        let orig_info = if self.idx < self.len {
            *self.cur(0)
        } else {
            *self.prev()
        };

        let out_len = self.out_len;
        let out = self.out_infos_mut();
        for i in 0..num_out {
            let pinfo = &mut out[(out_len + i) as usize];
            *pinfo = orig_info;
            pinfo.codepoint = glyph_data[i as usize];
        }

        self.idx += num_in;
        self.out_len += num_out;
        true
    }

    /// `replace_glyph`
    pub(crate) fn replace_glyph(&mut self, glyph_index: HbCodepoint) -> bool {
        self.replace_glyphs(1, 1, &[glyph_index])
    }

    /// Makes a copy of the glyph at idx to output and replace glyph_index
    /// (`output_glyph`)
    pub(crate) fn output_glyph(&mut self, glyph_index: HbCodepoint) -> bool {
        self.replace_glyphs(0, 1, &[glyph_index])
    }

    /// `output_info`
    pub(crate) fn output_info(&mut self, glyph_info: HbGlyphInfo) -> bool {
        if !self.make_room_for(0, 1) {
            return false;
        }

        let out_len = self.out_len;
        *self.out_info_mut(out_len) = glyph_info;

        self.out_len += 1;
        true
    }
    /// Copies glyph at idx to output but doesn't advance idx
    /// (`copy_glyph`)
    pub(crate) fn copy_glyph(&mut self) -> bool {
        /* Extra copy because cur()'s return can be freed within
         * output_info() call if buffer reallocates. */
        self.output_info(*self.cur(0))
    }

    /// Copies glyph at idx to output and advance idx.
    /// If there's no output, just advance idx. (`next_glyph`)
    pub(crate) fn next_glyph(&mut self) -> bool {
        if self.have_output {
            if self.separate_out || self.out_len != self.idx {
                if !self.make_room_for(1, 1) {
                    return false;
                }
                let v = self.info[self.idx as usize];
                let out_len = self.out_len;
                *self.out_info_mut(out_len) = v;
            }
            self.out_len += 1;
        }

        self.idx += 1;
        true
    }
    /// Copies n glyphs at idx to output and advance idx.
    /// If there's no output, just advance idx. (`next_glyphs`)
    pub(crate) fn next_glyphs(&mut self, n: u32) -> bool {
        if self.have_output {
            if self.separate_out || self.out_len != self.idx {
                if !self.make_room_for(n, n) {
                    return false;
                }
                let (idx, out_len) = (self.idx as usize, self.out_len as usize);
                if self.separate_out {
                    self.out_info_sep[out_len..out_len + n as usize]
                        .copy_from_slice(&self.info[idx..idx + n as usize]);
                } else {
                    self.info.copy_within(idx..idx + n as usize, out_len);
                }
            }
            self.out_len += n;
        }

        self.idx += n;
        true
    }
    /// Advance idx without copying to output. (`skip_glyph`)
    #[inline]
    pub(crate) fn skip_glyph(&mut self) {
        self.idx += 1;
    }
    /// `reset_masks`
    pub(crate) fn reset_masks(&mut self, mask: HbMask) {
        for j in 0..self.len as usize {
            self.info[j].mask = mask;
        }
    }
    /// `add_masks`
    pub(crate) fn add_masks(&mut self, mask: HbMask) {
        for j in 0..self.len as usize {
            self.info[j].mask |= mask;
        }
    }

    /// `merge_clusters`
    #[inline]
    pub(crate) fn merge_clusters(&mut self, start: u32, end: u32) {
        if end.wrapping_sub(start) < 2 || end < start {
            return;
        }
        self.merge_clusters_impl(start, end);
    }

    /// Adds glyph flags in mask to infos with clusters between start and
    /// end. The start index will be from out-buffer if from_out_buffer is
    /// true. If interior is true, then the cluster having the minimum value
    /// is skipped. (`_set_glyph_flags`)
    pub(crate) fn _set_glyph_flags(
        &mut self,
        mask: HbMask,
        start: u32,
        end: u32,
        interior: bool,
        from_out_buffer: bool,
    ) {
        let end = end.min(self.len);

        if interior && !from_out_buffer && end.wrapping_sub(start) < 2 {
            return;
        }

        self.scratch_flags |= HB_BUFFER_SCRATCH_FLAG_HAS_GLYPH_FLAGS;

        if !from_out_buffer || !self.have_output {
            if !interior {
                for i in start..end {
                    self.info[i as usize].mask |= mask;
                }
            } else {
                let cluster = Self::_infos_find_min_cluster(
                    self.cluster_level,
                    &self.info,
                    start,
                    end,
                    u32::MAX,
                );
                let level = self.cluster_level;
                let mut scratch = self.scratch_flags;
                Self::_infos_set_glyph_flags(
                    level,
                    &mut scratch,
                    &mut self.info,
                    start,
                    end,
                    cluster,
                    mask,
                );
                self.scratch_flags = scratch;
            }
        } else {
            if !interior {
                for i in start..self.out_len {
                    self.out_info_mut(i).mask |= mask;
                }
                for i in self.idx..end {
                    self.info[i as usize].mask |= mask;
                }
            } else {
                let level = self.cluster_level;
                let mut cluster =
                    Self::_infos_find_min_cluster(level, &self.info, self.idx, end, u32::MAX);
                let out_len = self.out_len;
                cluster = if self.separate_out {
                    Self::_infos_find_min_cluster(
                        level,
                        &self.out_info_sep,
                        start,
                        out_len,
                        cluster,
                    )
                } else {
                    Self::_infos_find_min_cluster(level, &self.info, start, out_len, cluster)
                };

                let mut scratch = self.scratch_flags;
                if self.separate_out {
                    Self::_infos_set_glyph_flags(
                        level,
                        &mut scratch,
                        &mut self.out_info_sep,
                        start,
                        out_len,
                        cluster,
                        mask,
                    );
                } else {
                    Self::_infos_set_glyph_flags(
                        level,
                        &mut scratch,
                        &mut self.info,
                        start,
                        out_len,
                        cluster,
                        mask,
                    );
                }
                let idx = self.idx;
                Self::_infos_set_glyph_flags(
                    level,
                    &mut scratch,
                    &mut self.info,
                    idx,
                    end,
                    cluster,
                    mask,
                );
                self.scratch_flags = scratch;
            }
        }
    }

    /// `unsafe_to_break`
    pub(crate) fn unsafe_to_break(&mut self, start: u32, end: u32) {
        self._set_glyph_flags(
            HB_GLYPH_FLAG_UNSAFE_TO_BREAK | HB_GLYPH_FLAG_UNSAFE_TO_CONCAT,
            start,
            end,
            true,
            false,
        );
    }
    /// `unsafe_to_break ()` over the whole buffer
    pub(crate) fn unsafe_to_break_all(&mut self) {
        self.unsafe_to_break(0, u32::MAX);
    }
    /// `safe_to_insert_tatweel`
    pub(crate) fn safe_to_insert_tatweel(&mut self, start: u32, end: u32) {
        if (self.flags & HB_BUFFER_FLAG_PRODUCE_SAFE_TO_INSERT_TATWEEL) == 0 {
            self.unsafe_to_break(start, end);
            return;
        }
        self._set_glyph_flags(
            HB_GLYPH_FLAG_SAFE_TO_INSERT_TATWEEL,
            start,
            end,
            true,
            false,
        );
    }
    /// `unsafe_to_concat`
    #[inline]
    pub(crate) fn unsafe_to_concat(&mut self, start: u32, end: u32) {
        if (self.flags & HB_BUFFER_FLAG_PRODUCE_UNSAFE_TO_CONCAT) == 0 {
            return;
        }
        self._set_glyph_flags(HB_GLYPH_FLAG_UNSAFE_TO_CONCAT, start, end, false, false);
    }
    /// `unsafe_to_break_from_outbuffer`
    pub(crate) fn unsafe_to_break_from_outbuffer(&mut self, start: u32, end: u32) {
        self._set_glyph_flags(
            HB_GLYPH_FLAG_UNSAFE_TO_BREAK | HB_GLYPH_FLAG_UNSAFE_TO_CONCAT,
            start,
            end,
            true,
            true,
        );
    }
    /// `unsafe_to_concat_from_outbuffer`
    #[inline]
    pub(crate) fn unsafe_to_concat_from_outbuffer(&mut self, start: u32, end: u32) {
        if (self.flags & HB_BUFFER_FLAG_PRODUCE_UNSAFE_TO_CONCAT) == 0 {
            return;
        }
        self._set_glyph_flags(HB_GLYPH_FLAG_UNSAFE_TO_CONCAT, start, end, false, true);
    }

    /// `resize`
    pub(crate) fn resize(&mut self, length: u32) -> bool {
        if !self.ensure(length) {
            return false;
        }
        self.len = length;
        true
    }
    /// `ensure`
    #[inline]
    pub(crate) fn ensure(&mut self, size: u32) -> bool {
        if size == 0 || size < self.allocated {
            true
        } else {
            self.enlarge(size)
        }
    }

    /// `ensure_inplace`
    #[inline]
    pub(crate) fn ensure_inplace(&self, size: u32) -> bool {
        size == 0 || size < self.allocated
    }

    /// `ensure_glyphs`
    pub(crate) fn ensure_glyphs(&mut self) -> bool {
        if self.content_type != HB_BUFFER_CONTENT_TYPE_GLYPHS {
            if self.content_type != HB_BUFFER_CONTENT_TYPE_INVALID {
                return false;
            }
            self.content_type = HB_BUFFER_CONTENT_TYPE_GLYPHS;
        }
        true
    }
    /// `ensure_unicode`
    pub(crate) fn ensure_unicode(&mut self) -> bool {
        if self.content_type != HB_BUFFER_CONTENT_TYPE_UNICODE {
            if self.content_type != HB_BUFFER_CONTENT_TYPE_INVALID {
                return false;
            }
            self.content_type = HB_BUFFER_CONTENT_TYPE_UNICODE;
        }
        true
    }

    /// `clear_context`
    #[inline]
    pub(crate) fn clear_context(&mut self, side: usize) {
        self.context_len[side] = 0;
    }

    /// `set_cluster`
    #[inline]
    pub(crate) fn set_cluster(inf: &mut HbGlyphInfo, cluster: u32, mask: u32) {
        if inf.cluster != cluster {
            inf.mask = (inf.mask & !HB_GLYPH_FLAG_DEFINED) | (mask & HB_GLYPH_FLAG_DEFINED);
        }
        inf.cluster = cluster;
    }

    /// `_infos_set_glyph_flags`
    fn _infos_set_glyph_flags(
        cluster_level: u32,
        scratch_flags: &mut u32,
        infos: &mut [HbGlyphInfo],
        start: u32,
        end: u32,
        cluster: u32,
        mask: HbMask,
    ) {
        if start == end {
            return;
        }

        let cluster_first = infos[start as usize].cluster;
        let cluster_last = infos[(end - 1) as usize].cluster;

        if cluster_level == HB_BUFFER_CLUSTER_LEVEL_CHARACTERS
            || (cluster != cluster_first && cluster != cluster_last)
        {
            for i in start..end {
                if cluster != infos[i as usize].cluster {
                    *scratch_flags |= HB_BUFFER_SCRATCH_FLAG_HAS_GLYPH_FLAGS;
                    infos[i as usize].mask |= mask;
                }
            }
            return;
        }

        /* Monotone clusters */

        if cluster == cluster_first {
            let mut i = end;
            while start < i && infos[(i - 1) as usize].cluster != cluster_first {
                *scratch_flags |= HB_BUFFER_SCRATCH_FLAG_HAS_GLYPH_FLAGS;
                infos[(i - 1) as usize].mask |= mask;
                i -= 1;
            }
        } else
        /* cluster == cluster_last */
        {
            let mut i = start;
            while i < end && infos[i as usize].cluster != cluster_last {
                *scratch_flags |= HB_BUFFER_SCRATCH_FLAG_HAS_GLYPH_FLAGS;
                infos[i as usize].mask |= mask;
                i += 1;
            }
        }
    }

    /// `_infos_find_min_cluster`
    fn _infos_find_min_cluster(
        cluster_level: u32,
        infos: &[HbGlyphInfo],
        start: u32,
        end: u32,
        mut cluster: u32,
    ) -> u32 {
        if start == end {
            return cluster;
        }

        if cluster_level == HB_BUFFER_CLUSTER_LEVEL_CHARACTERS {
            for i in start..end {
                cluster = cluster.min(infos[i as usize].cluster);
            }
            return cluster;
        }

        cluster.min(
            infos[start as usize]
                .cluster
                .min(infos[(end - 1) as usize].cluster),
        )
    }

    /// `clear_glyph_flags`
    pub(crate) fn clear_glyph_flags(&mut self, mask: HbMask) {
        for i in 0..self.len as usize {
            self.info[i].mask =
                (self.info[i].mask & !HB_GLYPH_FLAG_DEFINED) | (mask & HB_GLYPH_FLAG_DEFINED);
        }
    }

    /* Here is how the buffer works internally:
     *
     * There are two info pointers: info and out_info.  They always have
     * the same allocated size, but different lengths.
     *
     * As an optimization, both info and out_info may point to the
     * same piece of memory, which is owned by info.  This remains the
     * case as long as out_len doesn't exceed i at any time.
     * In that case, sync() is mostly no-op and the glyph operations
     * operate mostly in-place.
     *
     * As soon as out_info gets longer than info, out_info is moved over
     * to an alternate buffer (which we reuse the pos buffer for), and its
     * current contents (out_len entries) are copied to the new place.
     *
     * This should all remain transparent to the user.  sync() then
     * switches info over to out_info and does housekeeping.
     */

    /* Internal API */

    /// `enlarge`
    pub(crate) fn enlarge(&mut self, size: u32) -> bool {
        if !self.successful {
            return false;
        }
        if size > self.max_len {
            self.successful = false;
            return false;
        }

        let mut new_allocated = self.allocated;

        /* (C fails when the byte size overflows) */
        if (size as u64) * 20 > u32::MAX as u64 {
            self.successful = false;
            return false;
        }

        while size >= new_allocated {
            new_allocated += (new_allocated >> 1) + 32;
        }

        if (new_allocated as u64) * 20 > u32::MAX as u64 {
            self.successful = false;
            return false;
        }

        self.info
            .resize(new_allocated as usize, HbGlyphInfo::default());
        self.out_info_sep
            .resize(new_allocated as usize, HbGlyphInfo::default());
        self.pos
            .resize(new_allocated as usize, HbGlyphPosition::default());
        self.allocated = new_allocated;

        true
    }

    /// `make_room_for`
    pub(crate) fn make_room_for(&mut self, num_in: u32, num_out: u32) -> bool {
        if !self.ensure(self.out_len + num_out) {
            return false;
        }

        if !self.separate_out && self.out_len + num_out > self.idx + num_in {
            self.separate_out = true;
            let n = self.out_len as usize;
            self.out_info_sep[..n].copy_from_slice(&self.info[..n]);
        }

        true
    }

    /// `shift_forward`
    pub(crate) fn shift_forward(&mut self, count: u32) -> bool {
        if !self.ensure(self.len + count) {
            return false;
        }

        let (idx, len) = (self.idx as usize, self.len as usize);
        self.info.copy_within(idx..len, idx + count as usize);
        if self.idx + count > self.len {
            /* Under memory failure we might expose this area.  At least
             * clean it up.  Oh well...
             *
             * Ideally, we should at least set Default_Ignorable bits on
             * these, as well as consistent cluster values.  But the former
             * is layering violation... */
            for i in len..idx + count as usize {
                self.info[i] = HbGlyphInfo::default();
            }
        }
        self.len += count;
        self.idx += count;

        true
    }

    /* HarfBuzz-Internal API */

    /// `similar`
    pub(crate) fn similar(&mut self, src: &HbBuffer) {
        self.unicode = src.unicode;
        self.flags = src.flags;
        self.cluster_level = src.cluster_level;
        self.replacement = src.replacement;
        self.invisible = src.invisible;
        self.not_found = src.not_found;
    }

    /// `reset`
    pub(crate) fn reset(&mut self) {
        self.unicode = hb_unicode_funcs_get_default();
        self.flags = HB_BUFFER_FLAG_DEFAULT;
        self.cluster_level = HB_BUFFER_CLUSTER_LEVEL_DEFAULT;
        self.replacement = HB_BUFFER_REPLACEMENT_CODEPOINT_DEFAULT;
        self.invisible = 0;
        self.not_found = 0;

        self.clear();
    }

    /// `clear`
    pub(crate) fn clear(&mut self) {
        self.content_type = HB_BUFFER_CONTENT_TYPE_INVALID;
        self.props = HB_SEGMENT_PROPERTIES_DEFAULT;

        self.successful = true;
        self.shaping_failed = false;
        self.have_output = false;
        self.have_positions = false;

        self.idx = 0;
        self.len = 0;
        self.out_len = 0;
        self.separate_out = false;

        self.context = [[0; HbBuffer::CONTEXT_LENGTH]; 2];
        self.context_len = [0; 2];

        self.deallocate_var_all();
        self.serial = 0;
        self.random_state = 1;
        self.scratch_flags = HB_BUFFER_SCRATCH_FLAG_DEFAULT;
    }

    /// Called around shape() (`enter`)
    pub(crate) fn enter(&mut self) {
        self.deallocate_var_all();
        self.serial = 0;
        self.shaping_failed = false;
        self.scratch_flags = HB_BUFFER_SCRATCH_FLAG_DEFAULT;
        if let Some(mul) = self.len.checked_mul(HB_BUFFER_MAX_LEN_FACTOR) {
            self.max_len = mul.max(HB_BUFFER_MAX_LEN_MIN);
        }
        if let Some(mul) = self.len.checked_mul(HB_BUFFER_MAX_OPS_FACTOR) {
            self.max_ops = mul.max(HB_BUFFER_MAX_OPS_MIN) as i32;
        }
    }
    /// `leave`
    pub(crate) fn leave(&mut self) {
        self.max_len = HB_BUFFER_MAX_LEN_DEFAULT;
        self.max_ops = HB_BUFFER_MAX_OPS_DEFAULT;
        self.deallocate_var_all();
        self.serial = 0;
        // Intentionally not reseting shaping_failed, such that it can be inspected.
    }

    /// `add`
    pub(crate) fn add(&mut self, codepoint: HbCodepoint, cluster: u32) {
        if !self.ensure(self.len + 1) {
            return;
        }

        let glyph = &mut self.info[self.len as usize];

        *glyph = HbGlyphInfo::default();
        glyph.codepoint = codepoint;
        glyph.mask = 0;
        glyph.cluster = cluster;

        self.len += 1;
    }

    /// `add_info`
    pub(crate) fn add_info(&mut self, glyph_info: HbGlyphInfo) {
        if !self.ensure(self.len + 1) {
            return;
        }

        self.info[self.len as usize] = glyph_info;

        self.len += 1;
    }

    /// `clear_output`
    pub(crate) fn clear_output(&mut self) {
        self.have_output = true;
        self.have_positions = false;

        self.idx = 0;
        self.out_len = 0;
        self.separate_out = false;
    }

    /// `clear_positions`
    pub(crate) fn clear_positions(&mut self) {
        self.have_output = false;
        self.have_positions = true;

        self.out_len = 0;
        self.separate_out = false;

        for p in &mut self.pos[..self.len as usize] {
            *p = HbGlyphPosition::default();
        }
    }

    /// `sync`
    pub(crate) fn sync(&mut self) -> bool {
        let mut ret = false;

        'reset: {
            if !self.successful || !self.next_glyphs(self.len - self.idx) {
                break 'reset;
            }

            if self.separate_out {
                std::mem::swap(&mut self.info, &mut self.out_info_sep);
            }
            self.len = self.out_len;
            ret = true;
        }

        self.have_output = false;
        self.out_len = 0;
        self.separate_out = false;
        self.idx = 0;

        ret
    }

    /// `sync_so_far`
    pub(crate) fn sync_so_far(&mut self) -> i32 {
        let had_output = self.have_output;
        let out_i = self.out_len;
        let i = self.idx;
        let old_idx = self.idx;

        if self.sync() {
            self.idx = out_i;
        } else {
            self.idx = i;
        }

        if had_output {
            self.have_output = true;
            self.out_len = self.idx;
        }

        self.idx.wrapping_sub(old_idx) as i32
    }

    /// i is output-buffer index. (`move_to`)
    pub(crate) fn move_to(&mut self, i: u32) -> bool {
        if !self.have_output {
            self.idx = i;
            return true;
        }
        if !self.successful {
            return false;
        }

        if self.out_len < i {
            let count = i - self.out_len;
            if !self.make_room_for(count, count) {
                return false;
            }

            let (idx, out_len, n) = (self.idx as usize, self.out_len as usize, count as usize);
            if self.separate_out {
                self.out_info_sep[out_len..out_len + n].copy_from_slice(&self.info[idx..idx + n]);
            } else {
                self.info.copy_within(idx..idx + n, out_len);
            }
            self.idx += count;
            self.out_len += count;
        } else if self.out_len > i {
            /* Tricky part: rewinding... */
            let count = self.out_len - i;

            /* This will blow in our face if memory allocation fails later
             * in this same lookup...
             *
             * We used to shift with extra 32 items.
             * But that would leave empty slots in the buffer in case of allocation
             * failures.  See comments in shift_forward().  This can cause O(N^2)
             * behavior more severely than adding 32 empty slots can... */
            if self.idx < count && !self.shift_forward(count - self.idx) {
                return false;
            }

            self.idx -= count;
            self.out_len -= count;
            let (idx, out_len, n) = (self.idx as usize, self.out_len as usize, count as usize);
            if self.separate_out {
                self.info[idx..idx + n].copy_from_slice(&self.out_info_sep[out_len..out_len + n]);
            } else {
                self.info.copy_within(out_len..out_len + n, idx);
            }
        }

        true
    }

    /// `set_masks`
    pub(crate) fn set_masks(
        &mut self,
        mut value: HbMask,
        mask: HbMask,
        cluster_start: u32,
        cluster_end: u32,
    ) {
        if mask == 0 {
            return;
        }

        let not_mask = !mask;
        value &= mask;

        for i in 0..self.len as usize {
            if cluster_start <= self.info[i].cluster && self.info[i].cluster < cluster_end {
                self.info[i].mask = (self.info[i].mask & not_mask) | value;
            }
        }
    }

    /// `merge_clusters_impl`
    pub(crate) fn merge_clusters_impl(&mut self, mut start: u32, mut end: u32) {
        if self.cluster_level == HB_BUFFER_CLUSTER_LEVEL_CHARACTERS {
            self.unsafe_to_break(start, end);
            return;
        }

        let mut cluster = self.info[start as usize].cluster;

        for i in start + 1..end {
            cluster = cluster.min(self.info[i as usize].cluster);
        }

        /* Extend end */
        if cluster != self.info[(end - 1) as usize].cluster {
            while end < self.len
                && self.info[(end - 1) as usize].cluster == self.info[end as usize].cluster
            {
                end += 1;
            }
        }

        /* Extend start */
        if cluster != self.info[start as usize].cluster {
            while self.idx < start
                && self.info[(start - 1) as usize].cluster == self.info[start as usize].cluster
            {
                start -= 1;
            }
        }

        /* If we hit the start of buffer, continue in out-buffer. */
        if self.idx == start && self.info[start as usize].cluster != cluster {
            let start_cluster = self.info[start as usize].cluster;
            let mut i = self.out_len;
            while i != 0 && self.out_info(i - 1).cluster == start_cluster {
                HbBuffer::set_cluster(self.out_info_mut(i - 1), cluster, 0);
                i -= 1;
            }
        }

        for i in start..end {
            HbBuffer::set_cluster(&mut self.info[i as usize], cluster, 0);
        }
    }

    /// `merge_out_clusters`
    pub(crate) fn merge_out_clusters(&mut self, mut start: u32, mut end: u32) {
        if self.cluster_level == HB_BUFFER_CLUSTER_LEVEL_CHARACTERS {
            return;
        }

        if end.wrapping_sub(start) < 2 || end < start {
            return;
        }

        let mut cluster = self.out_info(start).cluster;

        for i in start + 1..end {
            cluster = cluster.min(self.out_info(i).cluster);
        }

        /* Extend start */
        while start != 0 && self.out_info(start - 1).cluster == self.out_info(start).cluster {
            start -= 1;
        }

        /* Extend end */
        while end < self.out_len && self.out_info(end - 1).cluster == self.out_info(end).cluster {
            end += 1;
        }

        /* If we hit the end of out-buffer, continue in buffer. */
        if end == self.out_len {
            let last = self.out_info(end - 1).cluster;
            let mut i = self.idx;
            while i < self.len && self.info[i as usize].cluster == last {
                HbBuffer::set_cluster(&mut self.info[i as usize], cluster, 0);
                i += 1;
            }
        }

        for i in start..end {
            HbBuffer::set_cluster(self.out_info_mut(i), cluster, 0);
        }
    }

    /// Merge clusters for deleting current glyph, and skip it.
    /// (`delete_glyph`)
    pub(crate) fn delete_glyph(&mut self) {
        /* The logic here is duplicated in hb_ot_hide_default_ignorables(). */

        let cluster = self.info[self.idx as usize].cluster;
        'done: {
            if (self.idx + 1 < self.len && cluster == self.info[(self.idx + 1) as usize].cluster)
                || (self.out_len != 0 && cluster == self.out_info(self.out_len - 1).cluster)
            {
                /* Cluster survives; do nothing. */
                break 'done;
            }

            if self.out_len != 0 {
                /* Merge cluster backward. */
                if cluster < self.out_info(self.out_len - 1).cluster {
                    let mask = self.info[self.idx as usize].mask;
                    let old_cluster = self.out_info(self.out_len - 1).cluster;
                    let mut i = self.out_len;
                    while i != 0 && self.out_info(i - 1).cluster == old_cluster {
                        HbBuffer::set_cluster(self.out_info_mut(i - 1), cluster, mask);
                        i -= 1;
                    }
                }
                break 'done;
            }

            if self.idx + 1 < self.len {
                /* Merge cluster forward. */
                self.merge_clusters(self.idx, self.idx + 2);
                break 'done;
            }
        }

        self.skip_glyph();
    }

    /// `delete_glyphs_inplace`
    pub(crate) fn delete_glyphs_inplace(&mut self, filter: impl Fn(&HbGlyphInfo) -> bool) {
        /* Merge clusters and delete filtered glyphs.
         * NOTE! We can't use out-buffer as we have positioning data. */
        let mut j: u32 = 0;
        let count = self.len;
        for i in 0..count {
            if filter(&self.info[i as usize]) {
                /* Merge clusters.
                 * Same logic as delete_glyph(), but for in-place removal. */

                let cluster = self.info[i as usize].cluster;
                if i + 1 < count && cluster == self.info[(i + 1) as usize].cluster {
                    continue; /* Cluster survives; do nothing. */
                }

                if j != 0 {
                    /* Merge cluster backward. */
                    if cluster < self.info[(j - 1) as usize].cluster {
                        let mask = self.info[i as usize].mask;
                        let old_cluster = self.info[(j - 1) as usize].cluster;
                        let mut k = j;
                        while k != 0 && self.info[(k - 1) as usize].cluster == old_cluster {
                            HbBuffer::set_cluster(&mut self.info[(k - 1) as usize], cluster, mask);
                            k -= 1;
                        }
                    }
                    continue;
                }

                if i + 1 < count {
                    self.merge_clusters(i, i + 2); /* Merge cluster forward. */
                }

                continue;
            }

            if j != i {
                self.info[j as usize] = self.info[i as usize];
                self.pos[j as usize] = self.pos[i as usize];
            }
            j += 1;
        }
        self.len = j;
    }

    /// `guess_segment_properties`
    pub(crate) fn guess_segment_properties_internal(&mut self) {
        /* If script is set to INVALID, guess from buffer contents */
        if self.props.script == HB_SCRIPT_INVALID {
            for i in 0..self.len as usize {
                let script = self.unicode.script(self.info[i].codepoint);
                if script != HB_SCRIPT_COMMON
                    && script != HB_SCRIPT_INHERITED
                    && script != HB_SCRIPT_UNKNOWN
                {
                    self.props.script = script;
                    break;
                }
            }
        }

        /* If direction is set to INVALID, guess from script */
        if self.props.direction == HB_DIRECTION_INVALID {
            self.props.direction = hb_script_get_horizontal_direction(self.props.script);
            if self.props.direction == HB_DIRECTION_INVALID {
                self.props.direction = HB_DIRECTION_LTR;
            }
        }

        /* If language is not set, use default language from locale */
        if self.props.language == HB_LANGUAGE_INVALID {
            /* TODO get_default_for_script? using $LANGUAGE */
            self.props.language = hb_language_get_default();
        }
    }

    /// `sort`
    pub(crate) fn sort(
        &mut self,
        start: u32,
        end: u32,
        compar: impl Fn(&HbGlyphInfo, &HbGlyphInfo) -> i32,
    ) {
        for i in start + 1..end {
            let mut j = i;
            while j > start && compar(&self.info[(j - 1) as usize], &self.info[i as usize]) > 0 {
                j -= 1;
            }
            if i == j {
                continue;
            }
            /* Move item i to occupy place for item j, shift what's in between. */
            self.merge_clusters(j, i + 1);
            {
                let t = self.info[i as usize];
                self.info
                    .copy_within(j as usize..i as usize, (j + 1) as usize);
                self.info[j as usize] = t;
            }
        }
    }

    /* Public API */

    /// Resets the buffer to its initial status, as if it was just newly
    /// created with `hb_buffer_create()` (`hb_buffer_reset`).
    pub fn hb_reset(&mut self) {
        self.reset();
    }

    /// Sets the type of buffer contents (`hb_buffer_set_content_type`).
    pub fn set_content_type(&mut self, content_type: HbBufferContentType) {
        self.content_type = content_type;
    }
    /// `hb_buffer_get_content_type`
    pub fn get_content_type(&self) -> HbBufferContentType {
        self.content_type
    }

    /// Fetches the Unicode-functions structure of a buffer
    /// (`hb_buffer_get_unicode_funcs`).
    pub fn get_unicode_funcs(&self) -> HbUnicodeFuncs {
        self.unicode
    }

    /// `hb_buffer_set_direction`
    pub fn set_direction(&mut self, direction: HbDirection) {
        self.props.direction = direction;
    }
    /// `hb_buffer_get_direction`
    pub fn get_direction(&self) -> HbDirection {
        self.props.direction
    }
    /// `hb_buffer_set_script`
    pub fn set_script(&mut self, script: HbScript) {
        self.props.script = script;
    }
    /// `hb_buffer_get_script`
    pub fn get_script(&self) -> HbScript {
        self.props.script
    }
    /// `hb_buffer_set_language`
    pub fn set_language(&mut self, language: HbLanguage) {
        self.props.language = language;
    }
    /// `hb_buffer_get_language`
    pub fn get_language(&self) -> HbLanguage {
        self.props.language
    }
    /// `hb_buffer_set_segment_properties`
    pub fn set_segment_properties(&mut self, props: &HbSegmentProperties) {
        self.props = *props;
    }
    /// `hb_buffer_get_segment_properties`
    pub fn get_segment_properties(&self) -> HbSegmentProperties {
        self.props
    }
    /// `hb_buffer_set_flags`
    pub fn set_flags(&mut self, flags: HbBufferFlags) {
        self.flags = flags;
    }
    /// `hb_buffer_get_flags`
    pub fn get_flags(&self) -> HbBufferFlags {
        self.flags
    }
    /// `hb_buffer_set_cluster_level`
    pub fn set_cluster_level(&mut self, cluster_level: HbBufferClusterLevel) {
        self.cluster_level = cluster_level;
    }
    /// `hb_buffer_get_cluster_level`
    pub fn get_cluster_level(&self) -> HbBufferClusterLevel {
        self.cluster_level
    }
    /// `hb_buffer_set_replacement_codepoint`
    pub fn set_replacement_codepoint(&mut self, replacement: HbCodepoint) {
        self.replacement = replacement;
    }
    /// `hb_buffer_set_invisible_glyph`
    pub fn set_invisible_glyph(&mut self, invisible: HbCodepoint) {
        self.invisible = invisible;
    }
    /// `hb_buffer_set_not_found_glyph`
    pub fn set_not_found_glyph(&mut self, not_found: HbCodepoint) {
        self.not_found = not_found;
    }

    /// Similar to `hb_buffer_reset()`, but does not clear the Unicode
    /// functions and the replacement code point (`hb_buffer_clear_contents`).
    pub fn clear_contents(&mut self) {
        self.clear();
    }

    /// `hb_buffer_allocation_successful`
    pub fn allocation_successful(&self) -> bool {
        self.successful
    }

    /// Appends a character with the Unicode value of `codepoint` to the
    /// buffer, and gives it the initial cluster value of `cluster`
    /// (`hb_buffer_add`).
    pub fn hb_add(&mut self, codepoint: HbCodepoint, cluster: u32) {
        self.add(codepoint, cluster);
        self.clear_context(1);
    }

    /// Returns the number of items in the buffer (`hb_buffer_get_length`).
    pub fn get_length(&self) -> u32 {
        self.len
    }

    /// Returns the buffer glyph information array
    /// (`hb_buffer_get_glyph_infos`).
    pub fn get_glyph_infos(&self) -> &[HbGlyphInfo] {
        &self.info[..self.len as usize]
    }

    /// Returns the buffer glyph positions array
    /// (`hb_buffer_get_glyph_positions`; positions are cleared first if
    /// the buffer has none).
    pub fn get_glyph_positions(&mut self) -> &[HbGlyphPosition] {
        if !self.have_positions {
            self.clear_positions();
        }
        &self.pos[..self.len as usize]
    }

    /// `hb_buffer_has_positions`
    pub fn has_positions(&self) -> bool {
        self.have_positions
    }

    /// `hb_buffer_reverse`
    pub fn hb_reverse(&mut self) {
        self.reverse();
    }

    /// `hb_buffer_reverse_clusters`
    pub fn hb_reverse_clusters(&mut self) {
        self.reverse_clusters();
    }

    /// Sets unset buffer segment properties based on buffer Unicode
    /// contents (`hb_buffer_guess_segment_properties`).
    pub fn guess_segment_properties(&mut self) {
        self.guess_segment_properties_internal();
    }

    /// `hb_buffer_add_utf`
    pub(crate) fn add_utf<U: HbUtf>(
        &mut self,
        text: &[U::CodepointT],
        item_offset: u32,
        item_length: i32,
    ) {
        let replacement = self.replacement;

        let text_length = text.len() as i64;
        let item_length = if item_length == -1 {
            text_length - item_offset as i64
        } else {
            item_length as i64
        };

        if item_length < 0
            || item_length > (i32::MAX / 8) as i64
            || item_offset as i64 + item_length > text_length
            || !self.ensure(
                self.len + (item_length as u32) * std::mem::size_of::<U::CodepointT>() as u32 / 4,
            )
        {
            return;
        }

        /* If buffer is empty and pre-context provided, install it.
         * This check is written this way, to make sure people can
         * provide pre-context in one add_utf() call, then provide
         * text in a follow-up call.  See:
         *
         * https://bugzilla.mozilla.org/show_bug.cgi?id=801410#c13
         */
        if self.len == 0 && item_offset > 0 {
            /* Add pre-context */
            self.clear_context(0);
            let mut prev = item_offset as usize;
            let start = 0;
            while start < prev && (self.context_len[0] as usize) < HbBuffer::CONTEXT_LENGTH {
                let mut u = 0;
                prev = U::prev(text, prev, start, &mut u, replacement);
                self.context[0][self.context_len[0] as usize] = u;
                self.context_len[0] += 1;
            }
        }

        let mut next = item_offset as usize;
        let end = next + item_length as usize;
        while next < end {
            let mut u = 0;
            let old_next = next;
            next = U::next(text, next, end, &mut u, replacement);
            self.add(u, old_next as u32);
        }

        /* Add post-context */
        self.clear_context(1);
        let end = text.len();
        while next < end && (self.context_len[1] as usize) < HbBuffer::CONTEXT_LENGTH {
            let mut u = 0;
            next = U::next(text, next, end, &mut u, replacement);
            self.context[1][self.context_len[1] as usize] = u;
            self.context_len[1] += 1;
        }

        self.content_type = HB_BUFFER_CONTENT_TYPE_UNICODE;
    }

    /// Appends the characters of the UTF-8 `text`, from `item_offset`, of
    /// `item_length` bytes (-1: to the end), with the rest as context
    /// (`hb_buffer_add_utf8`; `text` is the whole string, C's
    /// `text_length` bytes).
    pub fn add_utf8(&mut self, text: &[u8], item_offset: u32, item_length: i32) {
        self.add_utf::<HbUtf8>(text, item_offset, item_length);
    }

    /// `hb_buffer_add_utf32`
    pub fn add_utf32(&mut self, text: &[u32], item_offset: u32, item_length: i32) {
        self.add_utf::<HbUtf32>(text, item_offset, item_length);
    }

    /// `hb_buffer_add_latin1`
    pub fn add_latin1(&mut self, text: &[u8], item_offset: u32, item_length: i32) {
        self.add_utf::<HbLatin1>(text, item_offset, item_length);
    }

    /// `hb_buffer_add_codepoints`
    pub fn add_codepoints(&mut self, text: &[HbCodepoint], item_offset: u32, item_length: i32) {
        self.add_utf::<HbUtf32Novalidate>(text, item_offset, item_length);
    }
}

/// `foreach_group`: the `(start, end)` ranges of the groups of the
/// buffer.
pub(crate) fn foreach_group(
    buffer: &HbBuffer,
    group: impl Fn(&HbGlyphInfo, &HbGlyphInfo) -> bool + Copy,
) -> Vec<(u32, u32)> {
    let count = buffer.len;
    let mut v = Vec::new();
    let mut start = 0;
    let mut end = if count != 0 {
        buffer.group_end(0, group)
    } else {
        0
    };
    while start < count {
        v.push((start, end));
        start = end;
        end = buffer.group_end(start, group);
    }
    v
}

/// `foreach_cluster`
pub(crate) fn foreach_cluster(buffer: &HbBuffer) -> Vec<(u32, u32)> {
    foreach_group(buffer, HbBuffer::_cluster_group_func)
}
