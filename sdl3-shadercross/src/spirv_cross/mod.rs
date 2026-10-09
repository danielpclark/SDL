// Rust translation of SPIRV-Cross, as SDL_shadercross builds it
// (external/SPIRV-Cross).
// Copyright 2015-2021 Arm Limited and the other SPIRV-Cross authors
// SPDX-License-Identifier: Apache-2.0 OR MIT
// This is an altered (translated to Rust) version of the original
// software; see LICENSE.txt, SPIRV-CROSS-LICENSE and NOTICE.

//! SPIRV-Cross, translated from the copy SDL_shadercross pins
//! (external/SPIRV-Cross): the SPIR-V parser, the IR and its reflection
//! and analysis passes (`spirv_cross.cpp`, the CFG), the GLSL backend all
//! the others build on, and the HLSL and MSL backends, with the parts of
//! the C API (`spirv_cross_c.cpp`) SDL_shadercross calls.
//!
//! The translation keeps SPIRV-Cross's file layout (one Rust module per
//! C++ source file, `spirv_cross_parsed_ir.cpp` becoming `parsed_ir.rs`),
//! function order and comments; the C++ identifiers of the SPIR-V tokens
//! and IR fields are kept as well, which is why non-snake-case names are
//! allowed here. Every function that can throw (`SPIRV_CROSS_THROW`)
//! returns a [`common::Result`].

#![allow(
    non_snake_case,
    non_upper_case_globals,
    non_camel_case_types,
    dead_code,
    unused_assignments,
    missing_debug_implementations
)]
// (the translation follows the C++ code's loops, branches, and arithmetic)
#![allow(
    clippy::collapsible_if,
    clippy::collapsible_else_if,
    clippy::collapsible_match,
    clippy::comparison_chain,
    clippy::explicit_counter_loop,
    clippy::identity_op,
    clippy::if_same_then_else,
    clippy::implicit_saturating_sub,
    clippy::int_plus_one,
    clippy::len_zero,
    clippy::manual_is_multiple_of,
    clippy::manual_range_contains,
    clippy::manual_range_patterns,
    clippy::match_like_matches_macro,
    clippy::needless_bool,
    clippy::needless_late_init,
    clippy::needless_range_loop,
    clippy::neg_multiply,
    clippy::nonminimal_bool,
    clippy::too_many_arguments,
    clippy::type_complexity,
    clippy::unnecessary_sort_by,
    clippy::unnecessary_unwrap,
    clippy::wrong_self_convention
)]

pub mod cfg;
pub mod common;
pub mod cross;
pub mod glsl;
pub mod hlsl;
pub mod msl;
pub mod parsed_ir;
pub mod parser;
pub mod spirv;
pub mod std_hash;
