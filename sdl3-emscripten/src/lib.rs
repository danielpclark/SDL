// Rust translation of the Emscripten parts of Simple DirectMedia Layer (SDL) 3.x.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! # sdl3-emscripten — the JavaScript side of the `sdl3` crate on Emscripten
//!
//! An internal crate: `sdl3` depends on it only when it's built for
//! `wasm32-unknown-emscripten`, and on every other target it is empty. It
//! holds what upstream SDL gets from `<emscripten.h>` and `<emscripten/html5.h>`,
//! in pure Rust, with no `.js` file:
//!
//! * the macros that put JavaScript in the program, as clang's `EM_JS`,
//!   `EM_ASM` and `EM_JS_DEPS` do for C:
//!   [`em_js!`](crate::em_js) (a named JavaScript function, the main form),
//!   [`em_js_dollar!`](crate::em_js_dollar) (the same with `$0`, `$1`, … parameters, for upstream
//!   `EM_ASM` bodies pasted verbatim), and the anonymous, run-time compiled
//!   [`em_asm!`](crate::em_asm), [`em_asm_int!`](crate::em_asm_int), [`em_asm_double!`](crate::em_asm_double) and [`em_asm_ptr!`](crate::em_asm_ptr)
//!   (with their `main_thread_` names: there are no threads);
//! * the program's one dependency table (`EM_JS_DEPS`): the JavaScript
//!   library functions SDL's Emscripten code uses, for all of it
//!   ([`DEPS`]);
//! * the callback registry ([`Callback`], [`em_js_callback!`](crate::em_js_callback)): Rust
//!   functions that JavaScript calls by name (upstream's
//!   `EMSCRIPTEN_KEEPALIVE` functions, `_SDL_malloc`, …);
//! * [`sys`]: the C functions, structures and constants of `emscripten.h`,
//!   `html5.h` and `html5_webgl.h` that SDL uses;
//! * the JavaScript of SDL's own sources, one module per upstream file
//!   ([`sdl`] for `SDL.c`, [`stdlib`] for `SDL_malloc.c` and `SDL_utils.c`).
//!
//! ## Why a crate of its own
//!
//! Emscripten finds an `EM_JS` function through an exported data symbol,
//! `__em_js__NAME`, that points at its text. Rust exports such a symbol from
//! a `#[used]` static with `#[export_name]`, but only when the static is in
//! a library: in an executable crate (a binary, or a crate's own
//! `#[cfg(test)]` harness) rustc gives it local linkage and emscripten never
//! sees the text. So every [`em_js!`](crate::em_js) of the `sdl3` crate is here, and
//! `sdl3` (whose unit tests are such an executable) only calls them. The
//! anonymous [`em_asm!`](crate::em_asm) forms work in any crate.
//!
//! ## Linking
//!
//! Nothing here needs a link option. The `__em_js__*` symbols are exported
//! because they are `#[used]`; the dependency table is found through its
//! `__start_em_lib_deps`/`__stop_em_lib_deps` symbols, which emscripten
//! exports when they are defined; Rust functions JavaScript calls are
//! function-table entries, not exports, so an application's own
//! `-sEXPORTED_FUNCTIONS` (which replaces rustc's list) can't hide them.
//! What applications pass (`-sALLOW_MEMORY_GROWTH`, a larger stack, …) is in
//! `docs/HARDWARE_TESTING.md`.

#![no_std]
// Everywhere but on Emscripten the crate is empty, and its documentation
// links to nothing.
#![cfg_attr(not(target_os = "emscripten"), allow(rustdoc::broken_intra_doc_links))]
#![forbid(unsafe_op_in_unsafe_fn)]
#![warn(missing_debug_implementations, missing_docs)]

#[cfg(target_os = "emscripten")]
mod deps;
#[cfg(target_os = "emscripten")]
pub mod js;
#[cfg(target_os = "emscripten")]
mod registry;
#[cfg(target_os = "emscripten")]
pub mod sdl;
#[cfg(target_os = "emscripten")]
pub mod stdlib;
#[cfg(target_os = "emscripten")]
pub mod sys;
#[cfg(all(target_os = "emscripten", feature = "testing"))]
#[doc(hidden)]
pub mod testing;

#[cfg(target_os = "emscripten")]
pub use deps::DEPS;
#[cfg(target_os = "emscripten")]
pub use js::{EmArg, EmRaw};
#[cfg(target_os = "emscripten")]
pub use registry::Callback;

/// What the macros' expansions use. Not part of the API.
#[cfg(target_os = "emscripten")]
#[doc(hidden)]
pub mod __private {
    pub use crate::deps::KEEP;
    pub use crate::js::{asm_code_check, body_check, js_bytes, js_len, sig_of};
    pub use crate::js::{sdl3_em_asm_double, sdl3_em_asm_int, sdl3_em_asm_ptr};
}
