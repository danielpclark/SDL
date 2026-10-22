// Rust translation of the Emscripten parts of src/SDL.c from Simple DirectMedia Layer.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! What SDL's core does on Emscripten. The JavaScript itself is in the
//! `sdl3-emscripten` crate (see its documentation for why).

use sdl3_emscripten::stdlib::{
    EMSCRIPTEN_FORCE_FREE, SDL_CALLOC, SDL_FREE, SDL_MALLOC, SDL_REALLOC,
};
use sdl3_emscripten::sys;
use std::sync::Once;

/// Makes the functions upstream marks `EMSCRIPTEN_KEEPALIVE` callable from
/// JavaScript by their names (`_SDL_malloc`, `_SDL_calloc`, `_SDL_realloc`,
/// `_SDL_free`, `_Emscripten_force_free`), through the callback registry
/// rather than exports, which an application's `-sEXPORTED_FUNCTIONS`
/// replaces. Upstream's are callable as soon as the program loads; these
/// once SDL initializes (any `SDL_InitSubSystem()`), before any of SDL's
/// JavaScript that calls them is set up.
pub(crate) fn register_callbacks() {
    static ONCE: Once = Once::new();
    ONCE.call_once(|| {
        // SAFETY: extern "C" functions with the parameters upstream's
        // JavaScript passes; they live as long as the program.
        unsafe {
            SDL_MALLOC.register(crate::stdlib::malloc::malloc as *const ());
            SDL_CALLOC.register(crate::stdlib::malloc::calloc as *const ());
            SDL_REALLOC.register(crate::stdlib::malloc::realloc as *const ());
            SDL_FREE.register(crate::stdlib::malloc::free as *const ());
            EMSCRIPTEN_FORCE_FREE.register(crate::utils::emscripten_force_free as *const ());
        }
    });
}

/// `SDL_InitSubSystem()`'s Emscripten part: the `Module['SDL3']` table SDL's
/// JavaScript keeps its state in, made before anything else.
pub(crate) fn init_subsystem() {
    register_callbacks();
    // SAFETY: takes nothing, returns nothing, touches only Module.
    unsafe { sdl3_emscripten::sdl::sdl3_init_subsystem() };
}

/// `SDL_ExitProcess()` on Emscripten.
pub(crate) fn exit_process(exitcode: i32) -> ! {
    // SAFETY: plain calls; emscripten_force_exit() doesn't return (so
    // upstream's exit(exitcode) after it is never reached).
    unsafe {
        sys::emscripten_cancel_main_loop(); // this should "kill" the app.
        sys::emscripten_force_exit(exitcode) // this should "kill" the app.
    }
}
