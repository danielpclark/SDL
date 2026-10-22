// Rust translation of the JavaScript in src/SDL_assert.c from Simple DirectMedia Layer.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! The JavaScript of `SDL_assert.c`.

use core::ffi::{c_char, c_int};

// SDL_PromptAssertion()'s MAIN_THREAD_EM_ASM_INT, verbatim.
crate::em_js_dollar!(
    /// Asks with `window.prompt()` what to do about the failed assertion
    /// `message` describes; returns the character code of the one-letter
    /// reply (a cancelled prompt is `'i'`), or -1 for a longer or empty one.
    /// Under node there is no `window`: it throws a ReferenceError, which
    /// ends the program, as upstream's does.
    pub fn sdl3_prompt_assertion(message: *const c_char) -> c_int,
    r#"{
                var str =
                    UTF8ToString($0) + '\n\n' +
                    'Abort/Retry/Ignore/AlwaysIgnore? [ariA] :';
                var reply = window.prompt(str, "i");
                if (reply === null) {
                    reply = "i";
                }
                return reply.length === 1 ? reply.charCodeAt(0) : -1;
            }"#
);
