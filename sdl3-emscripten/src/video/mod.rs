// Rust translation of the JavaScript in src/video/emscripten/SDL_emscriptenvideo.c
// from Simple DirectMedia Layer.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! The JavaScript of the canvas video driver: `SDL_emscriptenvideo.c`
//! here, [`events`] for `SDL_emscriptenevents.c` (the pointer, drag and
//! lock-key listeners), [`framebuffer`] for `SDL_emscriptenframebuffer.c`
//! and [`mouse`] for `SDL_emscriptenmouse.c`.
//!
//! Each function is upstream's `MAIN_THREAD_EM_ASM` body, verbatim (in the
//! braces emscripten puts around an `EM_ASM`'s code): without threads the
//! main thread is the only one. Where upstream passes an `SDL_Window *` to
//! JavaScript, which hands it back to C, these pass the window's ID.

pub mod events;
pub mod framebuffer;
pub mod mouse;

use core::ffi::c_char;

crate::em_js!(
    /// Whether the program runs in a page, with a DOM (1): a `window` and
    /// a `document` (not under node, with or without a stand-in `window`).
    /// The video driver is unavailable without one.
    pub fn sdl3_video_has_dom() -> i32,
    r#"{
    var node = typeof process === 'object' && !!(process.versions && process.versions.node);
    return !node && typeof window === 'object' && typeof document === 'object' && document !== null ? 1 : 0;
}"#
);

// Emscripten_GetSystemTheme()'s MAIN_THREAD_EM_ASM_INT, verbatim.
crate::em_js_dollar!(
    /// The page's `prefers-color-scheme`: 0 light, 1 dark, -1 neither (or
    /// no `matchMedia`).
    pub fn sdl3_get_system_theme() -> i32,
    r#"{
        if (!window.matchMedia) {
            return -1;
        }

        if (window.matchMedia('(prefers-color-scheme: light)').matches) {
            return 0;
        }

        if (window.matchMedia('(prefers-color-scheme: dark)').matches) {
            return 1;
        }

        return -1;
    }"#
);

crate::em_js_callback!(
    /// `_Emscripten_SendSystemThemeChangedEvent()`, which the theme
    /// listener calls ([`sdl3_listen_system_theme`]).
    pub static EMSCRIPTEN_SEND_SYSTEM_THEME_CHANGED_EVENT =
        "_Emscripten_SendSystemThemeChangedEvent";
);

// Emscripten_ListenSystemTheme()'s MAIN_THREAD_EM_ASM, verbatim.
crate::em_js_dollar!(
    /// Listens to `prefers-color-scheme: dark` changes, which call
    /// `_Emscripten_SendSystemThemeChangedEvent()`
    /// ([`EMSCRIPTEN_SEND_SYSTEM_THEME_CHANGED_EVENT`], which must be
    /// registered). The listener is `Module['SDL3'].eventHandlerThemeChanged`.
    pub fn sdl3_listen_system_theme(),
    r#"{
        if (window.matchMedia) {
            var SDL3 = Module['SDL3'];

            SDL3.eventHandlerThemeChanged = function(event) {
                _Emscripten_SendSystemThemeChangedEvent();
            };

            SDL3.themeChangedMatchMedia = window.matchMedia('(prefers-color-scheme: dark)');
            SDL3.themeChangedMatchMedia.addEventListener('change', SDL3.eventHandlerThemeChanged);
        }
    }"#
);

// Emscripten_UnlistenSystemTheme()'s MAIN_THREAD_EM_ASM, verbatim.
crate::em_js_dollar!(
    /// Removes the listener of [`sdl3_listen_system_theme`].
    pub fn sdl3_unlisten_system_theme(),
    r#"{
        if (typeof(Module['SDL3']) !== 'undefined') {
            var SDL3 = Module['SDL3'];

            SDL3.themeChangedMatchMedia.removeEventListener('change', SDL3.eventHandlerThemeChanged);
            SDL3.themeChangedMatchMedia = undefined;
            SDL3.eventHandlerThemeChanged = undefined;
        }
    }"#
);

// Emscripten_ShowMessagebox()'s EM_ASM blocks, verbatim.
crate::em_js_dollar!(
    /// Adds a modal `<dialog class="SDL3_messagebox">` with the id `id`, the
    /// title (`<h1>`) and the message (`<p>`), in the CSS colors
    /// `background` and `color` (empty: the page's), and shows it.
    pub fn sdl3_messagebox_show(
        title: *const c_char,
        message: *const c_char,
        background: *const c_char,
        color: *const c_char,
        id: *const c_char
    ),
    r#"{
            var title = UTF8ToString($0);
            var message = UTF8ToString($1);
            var background = UTF8ToString($2);
            var color = UTF8ToString($3);
            var id = UTF8ToString($4);

            // Dialogs are always put in the front of the DOM
            var dialog = document.createElement("dialog");
            // Set class to allow for CSS selectors
            dialog.classList.add("SDL3_messagebox");
            dialog.id = id;
            dialog.style.color = color;
            dialog.style.backgroundColor = background;
            document.body.append(dialog);

            var h1 = document.createElement("h1");
            h1.innerText = title;
            dialog.append(h1);

            var p = document.createElement("p");
            p.innerText = message;
            dialog.append(p);

            dialog.showModal();
        }"#
);

crate::em_js_dollar!(
    /// Adds a button to the dialog `dialog_id`, which closes it with
    /// `response_id` as its return value; `click_on_return` and
    /// `click_on_escape` (non-zero) make Enter and Escape click it.
    /// False if there's no such dialog.
    #[allow(clippy::too_many_arguments)] // (upstream's EM_ASM_INT's)
    pub fn sdl3_messagebox_add_button(
        dialog_id: *const c_char,
        text: *const c_char,
        response_id: i32,
        click_on_return: i32,
        click_on_escape: i32,
        border: *const c_char,
        background: *const c_char,
        hovered: *const c_char
    ) -> i32,
    r#"{
                    var dialog_id = UTF8ToString($0);
                    var text = UTF8ToString($1);
                    var responseId = $2;
                    var clickOnReturn = $3;
                    var clickOnEscape = $4;
                    var border = UTF8ToString($5);
                    var background = UTF8ToString($6);
                    var hovered = UTF8ToString($7);

                    var dialog = document.getElementById(dialog_id);
                    if (!dialog) {
                        return false;
                    }

                    var button = document.createElement("button");
                    button.innerText = text;
                    button.style.borderColor = border;
                    button.style.backgroundColor = background;

                    dialog.addEventListener('keydown', function(e) {
                        if (clickOnReturn && e.key === "Enter") {
                            e.preventDefault();
                            button.click();
                        } else if (clickOnEscape && e.key === "Escape") {
                            e.preventDefault();
                            button.click();
                        }
                    });
                    dialog.addEventListener('cancel', function(e){
                        e.preventDefault();
                    });

                    button.onmouseenter = function(e){
                        button.style.backgroundColor = hovered;
                    };
                    button.onmouseleave = function(e){
                        button.style.backgroundColor = background;
                    };
                    button.onclick = function(e) {
                        dialog.close(responseId);
                    };

                    dialog.append(button);
                    return true;
                }"#
);

crate::em_js_dollar!(
    /// Whether the dialog `dialog_id` is still open (false if there's none).
    pub fn sdl3_messagebox_is_open(dialog_id: *const c_char) -> i32,
    r#"{
                var dialog_id = UTF8ToString($0);

                var dialog = document.getElementById(dialog_id);
                if (!dialog) {
                    return false;
                }
                return dialog.open;
            }"#
);

crate::em_js_dollar!(
    /// The button the dialog `dialog_id` was closed with (0 if there's no
    /// such dialog, `NaN` → 0 for a return value that isn't a number).
    pub fn sdl3_messagebox_button(dialog_id: *const c_char) -> i32,
    r#"{
                var dialog_id = UTF8ToString($0);
                var dialog = document.getElementById(dialog_id);
                if (!dialog) {
                    return 0;
                }
                try
                {
                    return parseInt(dialog.returnValue);
                }
                catch(e)
                {
                    return 0;
                }
            }"#
);

crate::em_js_dollar!(
    /// `alert(title + "\n\n" + message)`: the message box without Asyncify,
    /// which can't wait for a dialog.
    pub fn sdl3_messagebox_alert(title: *const c_char, message: *const c_char),
    r#"{
            alert(UTF8ToString($0) + "\n\n" + UTF8ToString($1));
        }"#
);

// Emscripten_GetDisplayUsableBounds() and Emscripten_SetWindowFillDocument()'s
// MAIN_THREAD_EM_ASM_INT, verbatim.
crate::em_js_dollar!(
    /// `window.innerWidth`.
    pub fn sdl3_window_inner_width() -> i32,
    r#"{
            return window.innerWidth;
        }"#
);

crate::em_js_dollar!(
    /// `window.innerHeight`.
    pub fn sdl3_window_inner_height() -> i32,
    r#"{
            return window.innerHeight;
        }"#
);

// Emscripten_SetWindowFillDocument()'s MAIN_THREAD_EM_ASM blocks, verbatim.
crate::em_js_dollar!(
    /// Makes the canvas `canvas_id` (a CSS selector) the page: everything
    /// else in `<body>` moves into a hidden `<div
    /// id="SDL3_fill_document_background_elements">`, and the canvas is put
    /// at the top left, `position: fixed`.
    pub fn sdl3_fill_document(canvas_id: *const c_char),
    r#"{
            var canvas = document.querySelector(UTF8ToString($0));
            canvas.SDL3_original_position = canvas.style.position;
            canvas.SDL3_original_top = canvas.style.top;
            canvas.SDL3_original_left = canvas.style.left;

            // hide everything on the page that isn't the canvas.
            var div = document.createElement('div');
            div.id = 'SDL3_fill_document_background_elements';

            div.SDL3_canvas = canvas;
            div.SDL3_canvas_parent = canvas.parentNode;
            div.SDL3_canvas_nextsib = canvas.nextSibling;

            var children = Array.from(document.body.children);
            for (var child of children) {
                div.appendChild(child);
            }
            document.body.appendChild(div);
            div.style.display = 'none';
            document.body.appendChild(canvas);
            canvas.style.position = 'fixed';
            canvas.style.top = '0';
            canvas.style.left = '0';
        }"#
);

crate::em_js_dollar!(
    /// Undoes [`sdl3_fill_document`]: the page and the canvas as they were.
    pub fn sdl3_unfill_document(),
    r#"{
                // if we had previously hidden everything behind a fill_document window, put it back.
                var div = document.getElementById('SDL3_fill_document_background_elements');
                if (div) {
                    if (div.SDL3_canvas_nextsib) {
                        div.SDL3_canvas_parent.insertBefore(div.SDL3_canvas, div.SDL3_canvas_nextsib);
                    } else {
                        div.SDL3_canvas_parent.appendChild(div.SDL3_canvas);
                    }
                    while (div.firstChild) {
                        document.body.insertBefore(div.firstChild, div);
                    }
                    div.SDL3_canvas.style.position = div.SDL3_canvas.SDL3_original_position;
                    div.SDL3_canvas.style.top = div.SDL3_canvas.SDL3_original_top;
                    div.SDL3_canvas.style.left = div.SDL3_canvas.SDL3_original_left;
                    div.remove();
                }
            }"#
);

crate::em_js_callback!(
    /// `_requestFullscreenThroughSDL(window)`, which
    /// `Module['requestFullscreen']` calls ([`sdl3_set_request_fullscreen`]).
    pub static REQUEST_FULLSCREEN_THROUGH_SDL = "_requestFullscreenThroughSDL";
);

// Emscripten_CreateWindow()'s MAIN_THREAD_EM_ASM, verbatim.
crate::em_js_dollar!(
    /// Makes emscripten's own "fullscreen" button (`Module.requestFullscreen()`,
    /// which a page's shell calls) go through SDL:
    /// `_requestFullscreenThroughSDL(window)`
    /// ([`REQUEST_FULLSCREEN_THROUGH_SDL`], which must be registered).
    pub fn sdl3_set_request_fullscreen(window: u32),
    r#"{
        Module['requestFullscreen'] = function(lockPointer, resizeCanvas) {
            _requestFullscreenThroughSDL($0);
        };
    }"#
);

// Emscripten_DestroyWindow()'s MAIN_THREAD_EM_ASM, verbatim.
crate::em_js_dollar!(
    /// `Module.requestFullscreen()` that does nothing, while there's no window.
    pub fn sdl3_unset_request_fullscreen(),
    r#"{
        // just ignore clicks on the fullscreen button while there's no SDL window.
        Module['requestFullscreen'] = function(lockPointer, resizeCanvas) {};
    }"#
);

// Emscripten_SetWindowIcon()'s MAIN_THREAD_EM_ASM, verbatim.
crate::em_js_dollar!(
    /// Makes the `size` bytes of PNG at `png` the page's icon: a `blob:` URL
    /// in the `href` of its `<link rel="icon">` (made if there's none),
    /// revoking the `blob:` URL it had.
    pub fn sdl3_set_window_icon(png: *const u8, size: usize),
    r#"{
        // Use `.slice` to make a copy of the data if we are dealing with a SharedArrayBuffer, or `.subarray` to create a
        // view into the existing buffer if its non-shared.
        var pngData = HEAPU8.buffer instanceof ArrayBuffer ? HEAPU8.subarray($0, $0 + $1) : HEAPU8.slice($0, $0 + $1);

        var blob = new Blob([pngData], {type: 'image/png'});
        var url = URL.createObjectURL(blob);

        // Set as favicon (create link element if it doesn't exist)
        var link = document.querySelector("link[rel~='icon']");
        if (!link) {
            link = document.createElement('link');
            link.rel = 'icon';
            link.type = 'image/png';
            document.head.appendChild(link);
        }

        // Revoke old URL if it exists to prevent memory leaks
        if (link.href && link.href.startsWith('blob:')) {
            URL.revokeObjectURL(link.href);
        }

        link.href = url;
    }"#
);
