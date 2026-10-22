// Rust translation of the JavaScript in src/SDL.c from Simple DirectMedia Layer.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! The JavaScript of `SDL.c`.

// SDL_InitSubSystem()'s MAIN_THREAD_EM_ASM, verbatim but for the
// preprocessor's choice between the wasm32 and wasm64 lines.
#[cfg(target_pointer_width = "32")]
crate::em_js!(
    /// Makes `Module['SDL3']`, the table SDL's JavaScript keeps its state
    /// in, with its pointer helpers `JSVarToCPtr` and `CPtrToHeap32Index`
    /// (unless the page made them). Upstream's `SDL_InitSubSystem()` does
    /// this first.
    pub fn sdl3_init_subsystem(),
    r#"{
        // make sure this generic table to hang SDL-specific Javascript stuff is available at init time.
        if (typeof(Module['SDL3']) === 'undefined') {
            Module['SDL3'] = {};
        }

        var SDL3 = Module['SDL3'];
        if (typeof(SDL3.JSVarToCPtr) === 'undefined') { SDL3.JSVarToCPtr = function(v) { return v; }; }
        if (typeof(SDL3.CPtrToHeap32Index) === 'undefined') { SDL3.CPtrToHeap32Index = function(ptr) { return ptr >>> 2; }; }
    }"#
);

#[cfg(target_pointer_width = "64")]
crate::em_js!(
    /// Makes `Module['SDL3']`, the table SDL's JavaScript keeps its state
    /// in, with its pointer helpers `JSVarToCPtr` and `CPtrToHeap32Index`
    /// (unless the page made them). Upstream's `SDL_InitSubSystem()` does
    /// this first.
    pub fn sdl3_init_subsystem(),
    r#"{
        // make sure this generic table to hang SDL-specific Javascript stuff is available at init time.
        if (typeof(Module['SDL3']) === 'undefined') {
            Module['SDL3'] = {};
        }

        var SDL3 = Module['SDL3'];
        if (typeof(SDL3.JSVarToCPtr) === 'undefined') { SDL3.JSVarToCPtr = function(v) { return BigInt(v); }; }
        if (typeof(SDL3.CPtrToHeap32Index) === 'undefined') { SDL3.CPtrToHeap32Index = function(ptr) { return Number(ptr / 4n); }; }
    }"#
);
