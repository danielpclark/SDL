// Rust translation of the Emscripten entry points of src/stdlib/SDL_malloc.c
// from Simple DirectMedia Layer.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! `SDL_malloc()`, `SDL_calloc()`, `SDL_realloc()` and `SDL_free()`, for
//! JavaScript.
//!
//! Upstream marks them `EMSCRIPTEN_KEEPALIVE` "so we can call these from
//! JavaScript": the Emscripten video backend's JavaScript allocates what it
//! hands to C (drop events, cursor URLs) with `_SDL_malloc` and frees it with
//! `_SDL_free`. Rust code has no use for them (its allocator replaces
//! SDL's), so these exist only for that JavaScript, which reaches them
//! through the callback registry (`core::emscripten::register_callbacks`).
//! SDL's default memory functions are the C runtime's, and so are these.
//!
//! Note (upstream): SDL counts the allocations (`SDL_GetNumAllocations()`)
//! and sets "Out of memory" as the error when one fails; neither exists in
//! this translation (no global allocation count, no global error string), so
//! a failure is only the null pointer, which the JavaScript checks.

use std::ffi::c_void;

/// Translation of `SDL_malloc()`.
pub(crate) extern "C" fn malloc(mut size: usize) -> *mut c_void {
    if size == 0 {
        size = 1;
    }
    // SAFETY: plain malloc().
    unsafe { libc::malloc(size) }
}

/// Translation of `SDL_calloc()`.
pub(crate) extern "C" fn calloc(mut nmemb: usize, mut size: usize) -> *mut c_void {
    if nmemb == 0 || size == 0 {
        nmemb = 1;
        size = 1;
    }
    // SAFETY: plain calloc().
    unsafe { libc::calloc(nmemb, size) }
}

/// Translation of `SDL_realloc()`.
///
/// # Safety
///
/// `ptr` is null or came from these functions and wasn't freed.
pub(crate) unsafe extern "C" fn realloc(ptr: *mut c_void, mut size: usize) -> *mut c_void {
    if size == 0 {
        size = 1;
    }
    // SAFETY: the caller's.
    unsafe { libc::realloc(ptr, size) }
}

/// Translation of `SDL_free()`.
///
/// # Safety
///
/// `ptr` is null or came from these functions and wasn't freed.
pub(crate) unsafe extern "C" fn free(ptr: *mut c_void) {
    if ptr.is_null() {
        return;
    }
    // SAFETY: the caller's.
    unsafe { libc::free(ptr) }
}
