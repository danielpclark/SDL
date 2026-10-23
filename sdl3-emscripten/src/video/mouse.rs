// Rust translation of the JavaScript in src/video/emscripten/SDL_emscriptenmouse.c
// from Simple DirectMedia Layer.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! The JavaScript of `SDL_emscriptenmouse.c` (its `EM_JS_DEPS`,
//! `$stringToUTF8,$UTF8ToString`, are in the dependency table).

use core::ffi::{c_char, c_void};

// Emscripten_CreateCursor()'s MAIN_THREAD_EM_ASM_PTR, verbatim.
crate::em_js_dollar!(
    /// The CSS `cursor` value of a custom cursor: the `w`x`h` RGBA32 pixels
    /// at `pixels` (packed rows, 4-byte aligned) drawn on a canvas of its
    /// own, as a PNG data URL, with the hot spot unless it is (0, 0):
    /// `url(data:…) hot_x hot_y, auto`. The string is NUL-terminated
    /// UTF-8 in memory from `_SDL_malloc` (which must be registered), the
    /// caller's to `SDL_free()`.
    pub fn sdl3_create_cursor(
        w: i32,
        h: i32,
        hot_x: i32,
        hot_y: i32,
        pixels: *const c_void
    ) -> *mut c_char,
    r#"{
        var w = $0;
        var h = $1;
        var hot_x = $2;
        var hot_y = $3;
        var pixels = $4;

        var canvas = document.createElement("canvas");
        canvas.width = w;
        canvas.height = h;

        var ctx = canvas.getContext("2d");

        var image = ctx.createImageData(w, h);
        var data = image.data;
        var src = pixels / 4;

        var data32 = new Int32Array(data.buffer);
        data32.set(HEAP32.subarray(src, src + data32.length));

        ctx.putImageData(image, 0, 0);
        var url = hot_x === 0 && hot_y === 0
            ? "url(" + canvas.toDataURL() + "), auto"
            : "url(" + canvas.toDataURL() + ") " + hot_x + " " + hot_y + ", auto";

        var urlBuf = _SDL_malloc(url.length + 1);
        stringToUTF8(url, urlBuf, url.length + 1);

        return urlBuf;
    }"#
);

// Emscripten_ShowCursor()'s first MAIN_THREAD_EM_ASM, verbatim.
crate::em_js_dollar!(
    /// Sets the CSS `cursor` of `Module['canvas']` (if there is one) to
    /// the NUL-terminated UTF-8 `cursor`.
    pub fn sdl3_show_cursor(cursor: *const c_char),
    r#"{
                    if (Module['canvas']) {
                        Module['canvas'].style['cursor'] = UTF8ToString($0);
                    }
                }"#
);

// Emscripten_ShowCursor()'s second MAIN_THREAD_EM_ASM, verbatim.
crate::em_js_dollar!(
    /// Hides the cursor over `Module['canvas']` (if there is one): CSS
    /// `cursor: none`.
    pub fn sdl3_hide_cursor(),
    r#"{
                if (Module['canvas']) {
                    Module['canvas'].style['cursor'] = 'none';
                }
            }"#
);

// Emscripten_GetGlobalMouseState()'s MAIN_THREAD_EM_ASM_DOUBLE and _INT,
// verbatim.
crate::em_js_dollar!(
    /// The mouse's last `clientX` on the document ([`sdl3_init_mouse`]).
    pub fn sdl3_global_mouse_x() -> f64,
    r#"{
        return Module['SDL3']['mouse_x'];
    }"#
);

crate::em_js_dollar!(
    /// The mouse's last `clientY` on the document ([`sdl3_init_mouse`]).
    pub fn sdl3_global_mouse_y() -> f64,
    r#"{
        return Module['SDL3']['mouse_y'];
    }"#
);

crate::em_js_dollar!(
    /// Whether the mouse button `button` (a `MouseEvent.button`, 0 to 4) is
    /// down on the document ([`sdl3_init_mouse`]).
    pub fn sdl3_global_mouse_button(button: i32) -> i32,
    r#"{
            return Module['SDL3']['mouse_buttons'][$0];
        }"#
);

// Emscripten_InitMouse()'s MAIN_THREAD_EM_ASM, verbatim.
crate::em_js_dollar!(
    /// Follows the mouse on the document, for the global mouse state:
    /// `mousemove`, `mousedown` and `mouseup` listeners that keep
    /// `Module['SDL3']`'s `mouse_x`, `mouse_y` and `mouse_buttons` (five
    /// booleans). Nothing removes them.
    pub fn sdl3_init_mouse(),
    r#"{
        var SDL3 = Module['SDL3'];
        SDL3['mouse_x'] = 0;
        SDL3['mouse_y'] = 0;
        /*
            Based on https://developer.mozilla.org/en-US/docs/Web/API/MouseEvent/button
            Possible value for button in the event object is [0, 5)
            NOTE: Some browsers do not allow handling the forwards and backwards buttons
        */
        SDL3['mouse_buttons'] = [];
        for (var i = 0; i < 5; ++i) {
            SDL3['mouse_buttons'][i] = false;
        }
        document.addEventListener('mousemove', function(e) {
            // Reacquire from object in case it changed for some reason
            var SDL3 = Module['SDL3'];
            SDL3['mouse_x'] = e.clientX;
            SDL3['mouse_y'] = e.clientY;
        });
        document.addEventListener('mousedown', function(e) {
            // Reacquire from object in case it changed for some reason
            var SDL3 = Module['SDL3'];
            if (0 <= e.button && e.button < SDL3['mouse_buttons'].length) {
                SDL3['mouse_buttons'][e.button] = true;
            }
        });
        document.addEventListener('mouseup', function(e) {
            // Reacquire from object in case it changed for some reason
            var SDL3 = Module['SDL3'];
            if (0 <= e.button && e.button < SDL3['mouse_buttons'].length) {
                SDL3['mouse_buttons'][e.button] = false;
            }
        });
    }"#
);
