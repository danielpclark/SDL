// Rust translation of the parts of the HarfBuzz text shaping library that
// SDL_ttf's bundled build compiles, from HarfBuzz (8.5.0, as SDL_ttf's
// external/harfbuzz pins it).
// Copyright © 2010-2022  Google, Inc. and the other HarfBuzz authors (see
// HARFBUZZ-COPYING).
// This is an altered (translated) version of the original software; it is
// used under HarfBuzz's "Old MIT" license (see HARFBUZZ-COPYING and
// LICENSE.txt).

//! HarfBuzz, translated from the copy SDL_ttf bundles (external/harfbuzz,
//! HarfBuzz 8.5.0), configured as SDL_ttf's CMake build configures it:
//! HarfBuzz's own Unicode functions (the UCD tables), the OpenType shaper
//! only, and the FreeType integration (`hb-ft`), with `HB_NO_BEYOND_64K`,
//! `HB_NO_CUBIC_GLYF` and `HB_NO_VAR_COMPOSITES`.
//!
//! The translation keeps HarfBuzz's file layout (one Rust module per C++
//! source file, `hb-ot-layout-gsub.hh` becoming `hb_ot_layout_gsub.rs`),
//! function order and comments where practical; the C++ identifiers are
//! kept as well, which is why non-snake-case names are allowed here. The
//! OpenType tables are read in place from the font's bytes, as in C++,
//! with the same sanitizer (including its edits of broken offsets).

#![allow(
    non_snake_case,
    non_upper_case_globals,
    non_camel_case_types,
    dead_code,
    unused_assignments
)]
// (the translation follows the C++ code's loops, branches, and arithmetic)
#![allow(
    clippy::collapsible_if,
    clippy::collapsible_else_if,
    clippy::collapsible_match,
    clippy::explicit_counter_loop,
    clippy::identity_op,
    clippy::if_same_then_else,
    clippy::implicit_saturating_sub,
    clippy::large_enum_variant,
    clippy::manual_clamp,
    clippy::manual_checked_ops,
    clippy::manual_is_multiple_of,
    clippy::manual_range_contains,
    clippy::manual_range_patterns,
    clippy::needless_bool,
    clippy::needless_late_init,
    clippy::needless_option_as_deref,
    clippy::needless_range_loop,
    clippy::too_many_arguments,
    clippy::type_complexity,
    clippy::unnecessary_unwrap,
    clippy::upper_case_acronyms
)]

pub mod hb_algs;
pub mod hb_buffer;
pub mod hb_common;
pub mod hb_face;
pub mod hb_font;
pub mod hb_ft;
pub mod hb_open_type;
pub mod hb_ot_kern_table;
pub mod hb_ot_layout;
pub mod hb_ot_layout_common;
pub mod hb_ot_layout_gdef;
pub mod hb_ot_layout_gpos;
pub mod hb_ot_layout_gsub;
pub mod hb_ot_layout_gsubgpos;
pub mod hb_ot_map;
pub mod hb_ot_shape;
pub mod hb_ot_shape_fallback;
pub mod hb_ot_shape_normalize;
pub mod hb_ot_shaper;
pub mod hb_ot_shaper_arabic;
pub mod hb_ot_shaper_arabic_pua;
pub mod hb_ot_shaper_arabic_table;
pub mod hb_ot_shaper_indic;
pub mod hb_ot_shaper_indic_machine;
pub mod hb_ot_shaper_indic_table;
pub mod hb_ot_shaper_khmer_machine;
pub mod hb_ot_shaper_myanmar_machine;
pub mod hb_ot_shaper_syllabic;
pub mod hb_ot_shaper_vowel_constraints;
pub mod hb_ot_shaper_vowel_constraints_table;
pub mod hb_ot_tag;
pub mod hb_ot_tag_table;
pub mod hb_ragel;
pub mod hb_sanitize;
pub mod hb_set;
pub mod hb_set_digest;
pub mod hb_shape;
pub mod hb_ucd;
pub mod hb_ucd_table;
pub mod hb_unicode;
pub mod hb_unicode_emoji_table;

#[cfg(test)]
mod tests;
