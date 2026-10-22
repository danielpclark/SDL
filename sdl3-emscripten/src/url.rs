// Rust translation of the JavaScript in src/misc/emscripten/SDL_sysurl.c from
// Simple DirectMedia Layer.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! The JavaScript of `misc/emscripten/SDL_sysurl.c` (its `EM_JS_DEPS`,
//! `$UTF8ToString`, is in the dependency table).

use core::ffi::c_char;

// SDL_SYS_OpenURL()'s EM_ASM, verbatim (in the braces emscripten puts
// around an EM_ASM's code).
crate::em_js_dollar!(
    /// Opens the NUL-terminated UTF-8 `url` in a new browsing context:
    /// `window.open(url, "_blank")`. A blocked popup isn't an error. Under
    /// node there is no `window`: it throws a ReferenceError, which ends
    /// the program, as upstream's does.
    pub fn sdl3_open_url(url: *const c_char),
    r#"{window.open(UTF8ToString($0), "_blank")}"#
);
