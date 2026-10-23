// Rust translation of the JavaScript in src/video/emscripten/SDL_emscriptenframebuffer.c
// from Simple DirectMedia Layer.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! The JavaScript of `SDL_emscriptenframebuffer.c`.

use core::ffi::{c_char, c_void};

// Emscripten_UpdateWindowFramebuffer()'s MAIN_THREAD_EM_ASM_INT, verbatim.
// (`Browser.createContext` is the dependency table's `$Browser`.)
crate::em_js_dollar!(
    /// Shows the `w`x`h` RGBA32 pixels at `pixels` (packed rows, 4-byte
    /// aligned) on the canvas `canvas_id` (a CSS selector), with
    /// `putImageData()` on its 2D context. The context, the `ImageData` and
    /// its `Int32Array` view are kept in `Module['SDL3']` (`ctx`,
    /// `ctxCanvas`, `image`, `w`, `h`, `imageCtx`, `data32`, `data32Data`)
    /// for the next update. False if the canvas has no 2D context.
    pub fn sdl3_update_window_framebuffer(
        w: i32,
        h: i32,
        pixels: *const c_void,
        canvas_id: *const c_char
    ) -> i32,
    r#"{
        var w = $0;
        var h = $1;
        var pixels = $2;
        var canvasId = UTF8ToString($3);
        var canvas = document.querySelector(canvasId);

        //TODO: this should store a context per canvas
        var SDL3 = Module['SDL3'];
        if (SDL3.ctxCanvas !== canvas) {
            SDL3.ctx = Browser.createContext(canvas, false, true);
            if (!SDL3.ctx) {
                return false;
            }
            SDL3.ctxCanvas = canvas;
        }
        if (SDL3.w !== w || SDL3.h !== h || SDL3.imageCtx !== SDL3.ctx) {
            SDL3.image = SDL3.ctx.createImageData(w, h);
            SDL3.w = w;
            SDL3.h = h;
            SDL3.imageCtx = SDL3.ctx;
        }
        var data = SDL3.image.data;
        var src = pixels / 4;

        if (SDL3.data32Data !== data) {
            SDL3.data32 = new Int32Array(data.buffer);
            SDL3.data32Data = data;
        }
        var data32 = SDL3.data32;
        data32.set(HEAP32.subarray(src, src + data32.length));

        SDL3.ctx.putImageData(SDL3.image, 0, 0);
        return true;
    }"#
);
