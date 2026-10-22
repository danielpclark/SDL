// Rust translation of the JavaScript in src/locale/emscripten/SDL_syslocale.c
// from Simple DirectMedia Layer.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! The JavaScript of `locale/emscripten/SDL_syslocale.c`.

use core::ffi::c_char;

// SDL_SYS_GetPreferredLocales()'s MAIN_THREAD_EM_ASM, verbatim.
crate::em_js_dollar!(
    /// Writes the browser's preferred languages (`navigator.languages`, or
    /// the one `navigator.language` and its older names give), as
    /// comma-separated `xx_YY` entries that fit in `buflen` bytes, into the
    /// `buflen` bytes at `buf`, one byte per UTF-16 unit (cut to 8 bits).
    /// Nothing is written without languages. There's no NUL after a list
    /// as long as `buflen`: the caller's buffer must be zeroed.
    ///
    /// `buf` must be below 2 GB (on the stack, as upstream's is): the
    /// JavaScript adds to it as it is.
    pub fn sdl3_get_preferred_locales(buf: *mut c_char, buflen: usize),
    r#"{
        var buf = $0;
        var buflen = $1;
        var list = undefined;

        if (navigator.languages && navigator.languages.length) {
            list = navigator.languages;
        } else {
            var oneOfThese = navigator.userLanguage || navigator.language || navigator.browserLanguage || navigator.systemLanguage;
            if (oneOfThese !== undefined) {
                list = [ oneOfThese ];
            }
        }

        if (list === undefined) {
            return;  // we've got nothing.
        }

        var str = "";  // Can't do list.join() because we need to fit in buflen.
        for (var i = 0; i < list.length; i++) {
            var item = list[i];
            if ((str.length + item.length + 1) > buflen) {
                break;   // don't add, we're out of space.
            }
            if (str.length > 0) {
                str += ",";
            }
            str += item;
        }

        str = str.replace(/-/g, "_");
        if (buflen > str.length) {
            buflen = str.length;  // clamp to size of string.
        }

        for (var i = 0; i < buflen; i++) {
            setValue(buf + i, str.charCodeAt(i), "i8");  // fill in C array.
        }
    }"#
);
