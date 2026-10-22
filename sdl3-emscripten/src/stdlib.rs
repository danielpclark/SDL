// Rust translation of the EMSCRIPTEN_KEEPALIVE functions of src/stdlib/SDL_malloc.c
// and src/SDL_utils.c from Simple DirectMedia Layer.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! The names upstream's JavaScript calls SDL's allocator by.
//!
//! `SDL_malloc.c` marks `SDL_malloc`, `SDL_calloc`, `SDL_realloc` and
//! `SDL_free` `EMSCRIPTEN_KEEPALIVE` "so we can call these from
//! JavaScript": the video backend's JavaScript allocates the strings and
//! drop events it hands to C with `_SDL_malloc` and frees them with
//! `_SDL_free`. `SDL_utils.c`'s `Emscripten_force_free` is C's `free()`,
//! for what JavaScript allocated with `stringToNewUTF8` (emscripten's
//! `_malloc`). The `sdl3` crate registers its functions under these names
//! when SDL initializes; see [`Callback`](crate::Callback).

crate::em_js_callback!(
    /// `_SDL_malloc(size)`.
    pub static SDL_MALLOC = "_SDL_malloc";
);
crate::em_js_callback!(
    /// `_SDL_calloc(nmemb, size)`.
    pub static SDL_CALLOC = "_SDL_calloc";
);
crate::em_js_callback!(
    /// `_SDL_realloc(mem, size)`.
    pub static SDL_REALLOC = "_SDL_realloc";
);
crate::em_js_callback!(
    /// `_SDL_free(mem)`.
    pub static SDL_FREE = "_SDL_free";
);
crate::em_js_callback!(
    /// `_Emscripten_force_free(ptr)`: `free()`, not `SDL_free()`.
    pub static EMSCRIPTEN_FORCE_FREE = "_Emscripten_force_free";
);
