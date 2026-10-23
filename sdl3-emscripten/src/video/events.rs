// Rust translation of the JavaScript in src/video/emscripten/SDL_emscriptenevents.c
// from Simple DirectMedia Layer.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! The JavaScript of `SDL_emscriptenevents.c`: the focus check, the
//! orientation handler's page sizes, the pointer listeners (Pointer Events,
//! for which emscripten has no `emscripten_set_*_callback`), the global
//! `pointerup` listener, the drag-and-drop listeners and the lock-key check
//! (its `EM_JS_DEPS`, `$writeArrayToMemory`, is in the dependency table).
//! The rest of the event handling is html5.h's callbacks ([`sys`](crate::sys)).
//!
//! Each function is upstream's `MAIN_THREAD_EM_ASM` body, verbatim. Where
//! upstream passes an `SDL_WindowData *` to JavaScript, which hands it back
//! to C, these pass the window's ID; the `SDL_VideoDevice *` of the global
//! `pointerup` listener is 0 (the Rust walks the video core's windows). The
//! Rust functions the JavaScript calls by name are the callbacks here
//! ([`EMSCRIPTEN_HANDLE_POINTER_ENTER`] and the rest), which the driver
//! registers, with `_SDL_malloc`, `_SDL_free` and `_Emscripten_force_free`
//! ([`stdlib`](crate::stdlib)).

use core::ffi::c_char;

// Emscripten_GetFocusedWindow()'s first MAIN_THREAD_EM_ASM_INT, verbatim.
crate::em_js_dollar!(
    /// Whether the canvas `canvas_id` (a CSS selector) is
    /// `document.activeElement` (false if there's no such element, or the
    /// selector isn't valid).
    pub fn sdl3_canvas_is_focused(canvas_id: *const c_char) -> i32,
    r#"{
            var id = UTF8ToString($0);
            try
            {
                var canvas = document.querySelector(id);
                if (canvas) {
                    return canvas === document.activeElement;
                }
            }
            catch (e)
            {
                // querySelector throws if not a valid selector
            }
            return false;
        }"#
);

// Emscripten_GetFocusedWindow()'s second MAIN_THREAD_EM_ASM_INT, verbatim.
crate::em_js_dollar!(
    /// `document.hasFocus()`.
    pub fn sdl3_document_has_focus() -> i32,
    r#"{
            return document.hasFocus();
        }"#
);

// Emscripten_HandleOrientationChange()'s MAIN_THREAD_EM_ASM_INTs, verbatim
// (window.innerWidth and window.innerHeight are the video driver's,
// super::sdl3_window_inner_width() and super::sdl3_window_inner_height()).
crate::em_js_dollar!(
    /// `document.body.clientWidth`.
    pub fn sdl3_document_body_client_width() -> i32,
    r#"{ return document.body.clientWidth; }"#
);

crate::em_js_dollar!(
    /// `document.body.clientHeight`.
    pub fn sdl3_document_body_client_height() -> i32,
    r#"{ return document.body.clientHeight; }"#
);

crate::em_js_dollar!(
    /// `window.outerWidth`.
    pub fn sdl3_window_outer_width() -> i32,
    r#"{ return window.outerWidth; }"#
);

crate::em_js_dollar!(
    /// `window.outerHeight`.
    pub fn sdl3_window_outer_height() -> i32,
    r#"{ return window.outerHeight; }"#
);

crate::em_js_dollar!(
    /// `window.pageXOffset`.
    pub fn sdl3_window_page_x_offset() -> i32,
    r#"{ return window.pageXOffset; }"#
);

crate::em_js_dollar!(
    /// `window.pageYOffset`.
    pub fn sdl3_window_page_y_offset() -> i32,
    r#"{ return window.pageYOffset; }"#
);

crate::em_js_callback!(
    /// `_Emscripten_HandlePointerEnter(window, event)`, which the
    /// `pointerenter` listener calls ([`sdl3_set_pointer_event_callbacks`]).
    pub static EMSCRIPTEN_HANDLE_POINTER_ENTER = "_Emscripten_HandlePointerEnter";
);

crate::em_js_callback!(
    /// `_Emscripten_HandlePointerLeave(window, event)`, which the
    /// `pointerleave` and `pointercancel` listeners call.
    pub static EMSCRIPTEN_HANDLE_POINTER_LEAVE = "_Emscripten_HandlePointerLeave";
);

crate::em_js_callback!(
    /// `_Emscripten_HandlePointerGeneric(window, event)`, which the
    /// `pointerdown`, `pointermove` and `pointerup` listeners call.
    pub static EMSCRIPTEN_HANDLE_POINTER_GENERIC = "_Emscripten_HandlePointerGeneric";
);

crate::em_js_callback!(
    /// `_Emscripten_HandleMouseButtonUpGlobal(device, event)`, which the
    /// document's `pointerup` listener calls
    /// ([`sdl3_set_global_mouseup_callback`]).
    pub static EMSCRIPTEN_HANDLE_MOUSE_BUTTON_UP_GLOBAL = "_Emscripten_HandleMouseButtonUpGlobal";
);

// Emscripten_prep_pointer_event_callbacks()'s MAIN_THREAD_EM_ASM, verbatim.
crate::em_js_dollar!(
    /// Makes `Module['SDL3'].makePointerEventCStruct(left, top, event)`
    /// (unless it's there): a pointer event as the driver's
    /// `Emscripten_PointerEvent` (`size` bytes, from `_SDL_malloc`, which
    /// must be registered; the caller's to `_SDL_free`), with the position
    /// relative to (`left`, `top`); 0 for a pointer that isn't a mouse, a
    /// touch or a pen. Only a pen's event has the pressure, tilt and twist.
    pub fn sdl3_prep_pointer_event_callbacks(size: usize),
    r#"{
        var SDL3 = Module['SDL3'];

        if (SDL3.makePointerEventCStruct === undefined) {
            SDL3.makePointerEventCStruct = function(left, top, event) {
                var ptrtype = 0;
                if (event.pointerType == "mouse") {
                    ptrtype = 1;
                } else if (event.pointerType == "touch") {
                    ptrtype = 2;
                } else if (event.pointerType == "pen") {
                    ptrtype = 3;
                } else {
                    return 0;
                }

                var ptr = _SDL_malloc($0);
                if (ptr != 0) {
                    var idx = SDL3.CPtrToHeap32Index(ptr);
                    HEAP32[idx++] = ptrtype;
                    HEAP32[idx++] = event.pointerId;
                    HEAP32[idx++] = (typeof(event.button) !== "undefined") ? event.button : -1;
                    HEAP32[idx++] = event.buttons;
                    HEAP32[idx++] = (event.type == "pointerdown") ? 1 : 0;
                    HEAPF32[idx++] = event.movementX;
                    HEAPF32[idx++] = event.movementY;
                    HEAPF32[idx++] = event.clientX - left;
                    HEAPF32[idx++] = event.clientY - top;
                    if (ptrtype == 3) {
                        HEAPF32[idx++] = event.pressure;
                        HEAPF32[idx++] = event.tangentialPressure;
                        HEAPF32[idx++] = event.tiltX;
                        HEAPF32[idx++] = event.tiltY;
                        HEAPF32[idx++] = event.twist;
                    }
                }
                return ptr;
            };
        }
    }"#
);

// Emscripten_set_pointer_event_callbacks()'s MAIN_THREAD_EM_ASM, verbatim.
crate::em_js_dollar!(
    /// Listens to the pointer events of the canvas `canvas_id` (a CSS
    /// selector; nothing if there's no such element), which call
    /// `_Emscripten_HandlePointerEnter`, `_Emscripten_HandlePointerLeave`
    /// and `_Emscripten_HandlePointerGeneric` with `window` and the event
    /// ([`sdl3_prep_pointer_event_callbacks`] first), and sets its
    /// `touch-action` to `none`. The listeners are the canvas's
    /// `sdlEventHandlerPointer*`.
    pub fn sdl3_set_pointer_event_callbacks(window: u32, canvas_id: *const c_char),
    r#"{
        var target = document.querySelector(UTF8ToString($1));
        if (target) {
            var SDL3 = Module['SDL3'];
            var data = $0;
            target.sdlEventHandlerPointerEnter = function(event) {
                var rect = target.getBoundingClientRect();
                var d = SDL3.makePointerEventCStruct(rect.left, rect.top, event);
                if (d != 0)
                {
                    _Emscripten_HandlePointerEnter(SDL3.JSVarToCPtr(data), d);
                    _SDL_free(d);
                }
            };
            target.sdlEventHandlerPointerLeave = function(event) {
                var rect = target.getBoundingClientRect();
                var d = SDL3.makePointerEventCStruct(rect.left, rect.top, event);
                if (d != 0)
                {
                    _Emscripten_HandlePointerLeave(SDL3.JSVarToCPtr(data), d);
                    _SDL_free(d);
                }
            };
            target.sdlEventHandlerPointerGeneric = function(event) {
                var rect = target.getBoundingClientRect();
                var d = SDL3.makePointerEventCStruct(rect.left, rect.top, event);
                if (d != 0)
                {
                    _Emscripten_HandlePointerGeneric(SDL3.JSVarToCPtr(data), d);
                    _SDL_free(d);
                }
            };

            target.style.touchAction = "none";  // or mobile devices will scroll as your touch moves across the element.
            target.addEventListener("pointerenter", target.sdlEventHandlerPointerEnter);
            target.addEventListener("pointerleave", target.sdlEventHandlerPointerLeave);
            target.addEventListener("pointercancel", target.sdlEventHandlerPointerLeave);
            target.addEventListener("pointerdown", target.sdlEventHandlerPointerGeneric);
            target.addEventListener("pointermove", target.sdlEventHandlerPointerGeneric);
            target.addEventListener("pointerup", target.sdlEventHandlerPointerGeneric);
        }
    }"#
);

// Emscripten_unset_pointer_event_callbacks()'s MAIN_THREAD_EM_ASM, verbatim.
crate::em_js_dollar!(
    /// Removes the listeners of [`sdl3_set_pointer_event_callbacks`] from the
    /// canvas `canvas_id`, and its `touch-action`.
    pub fn sdl3_unset_pointer_event_callbacks(canvas_id: *const c_char),
    r#"{
        var target = document.querySelector(UTF8ToString($0));
        if (target) {
            target.removeEventListener("pointerenter", target.sdlEventHandlerPointerEnter);
            target.removeEventListener("pointerleave", target.sdlEventHandlerPointerLeave);
            target.removeEventListener("pointercancel", target.sdlEventHandlerPointerLeave);
            target.removeEventListener("pointerdown", target.sdlEventHandlerPointerGeneric);
            target.removeEventListener("pointermove", target.sdlEventHandlerPointerGeneric);
            target.removeEventListener("pointerup", target.sdlEventHandlerPointerGeneric);
            target.style.touchAction = "";  // let mobile devices scroll again as your touch moves across the element.
            target.sdlEventHandlerPointerEnter = undefined;
            target.sdlEventHandlerPointerLeave = undefined;
            target.sdlEventHandlerPointerGeneric = undefined;
        }
    }"#
);

// Emscripten_set_global_mouseup_callback()'s MAIN_THREAD_EM_ASM, verbatim.
crate::em_js_dollar!(
    /// Listens to the document's `pointerup`, which calls
    /// `_Emscripten_HandleMouseButtonUpGlobal(device, event)` (with the
    /// position relative to the page; [`sdl3_prep_pointer_event_callbacks`]
    /// first). The listener is `document.sdlEventHandlerMouseButtonUpGlobal`.
    pub fn sdl3_set_global_mouseup_callback(device: u32),
    r#"{
        var target = document;
        if (target) {
            target.sdlEventHandlerMouseButtonUpGlobal = function(event) {
                var SDL3 = Module['SDL3'];
                var d = SDL3.makePointerEventCStruct(0, 0, event);
                if (d != 0)
                {
                    _Emscripten_HandleMouseButtonUpGlobal(SDL3.JSVarToCPtr($0), d);
                    _SDL_free(d);
                }
            };
            target.addEventListener("pointerup", target.sdlEventHandlerMouseButtonUpGlobal);
        }
    }"#
);

// Emscripten_unset_global_mouseup_callback()'s MAIN_THREAD_EM_ASM, verbatim.
crate::em_js_dollar!(
    /// Removes the listener of [`sdl3_set_global_mouseup_callback`].
    pub fn sdl3_unset_global_mouseup_callback(),
    r#"{
        var target = document;
        if (target) {
            target.removeEventListener("pointerup", target.sdlEventHandlerMouseButtonUpGlobal);
            target.sdlEventHandlerMouseButtonUpGlobal = undefined;
        }
    }"#
);

crate::em_js_callback!(
    /// `_Emscripten_SendDragEvent(window, event)`, which the `dragover`
    /// listener calls ([`sdl3_set_drag_event_callbacks`]).
    pub static EMSCRIPTEN_SEND_DRAG_EVENT = "_Emscripten_SendDragEvent";
);

crate::em_js_callback!(
    /// `_Emscripten_SendDragCompleteEvent(window)`, which the `drop`,
    /// `dragend` and `dragleave` listeners call.
    pub static EMSCRIPTEN_SEND_DRAG_COMPLETE_EVENT = "_Emscripten_SendDragCompleteEvent";
);

crate::em_js_callback!(
    /// `_Emscripten_SendDragTextEvent(window, text)`, which the `drop`
    /// listener calls for text.
    pub static EMSCRIPTEN_SEND_DRAG_TEXT_EVENT = "_Emscripten_SendDragTextEvent";
);

crate::em_js_callback!(
    /// `_Emscripten_SendDragFileEvent(window, filename)`, which the `drop`
    /// listener calls for each file it wrote.
    pub static EMSCRIPTEN_SEND_DRAG_FILE_EVENT = "_Emscripten_SendDragFileEvent";
);

// Emscripten_set_drag_event_callbacks()'s MAIN_THREAD_EM_ASM, verbatim.
crate::em_js_dollar!(
    /// Listens to the drag-and-drop events of the canvas `canvas_id` (a CSS
    /// selector; nothing if there's no such element), with `window`:
    ///
    /// * `dragover`: `_Emscripten_SendDragEvent` with the position relative
    ///   to the canvas, as the driver's `Emscripten_DropEvent` (`size`
    ///   bytes, from `_SDL_malloc`);
    /// * `drop`: text (`text/plain`) goes to `_Emscripten_SendDragTextEvent`;
    ///   each file is read (`FileReader`) into `/tmp/filedrop/N/NAME` (`N`
    ///   counting the files from 0) and its path goes to
    ///   `_Emscripten_SendDragFileEvent` once it's written; then
    ///   `_Emscripten_SendDragCompleteEvent`, at once, and again once every
    ///   file is read;
    /// * `dragend` and `dragleave`: `_Emscripten_SendDragCompleteEvent`.
    ///
    /// It makes `/tmp/filedrop`. The listeners are
    /// `Module['SDL3'].eventHandlerDrop*` (the last window's).
    pub fn sdl3_set_drag_event_callbacks(window: u32, canvas_id: *const c_char, size: usize),
    r#"{
        var target = document.querySelector(UTF8ToString($1));
        if (target) {
            var data = $0;
            var SDL3 = Module['SDL3'];

            var makeDropEventCStruct = function(event) {
                var ptr = 0;
                ptr = _SDL_malloc($2);
                if (ptr != 0) {
                    var idx = ptr >> 2;
                    var rect = target.getBoundingClientRect();
                    HEAP32[idx++] = event.clientX - rect.left;
                    HEAP32[idx++] = event.clientY - rect.top;
                }
                return ptr;
            };

            SDL3.eventHandlerDropDragover = function(event) {
                event.preventDefault();
                var d = makeDropEventCStruct(event); if (d != 0) { _Emscripten_SendDragEvent(data, d); _SDL_free(d); }
            };
            target.addEventListener("dragover", SDL3.eventHandlerDropDragover);

            SDL3.drop_count = 0;

            // FS.* functions throw exceptions when there are errors (such as the temp dir already existing),
            //  but we ignore all of these in a catch handler; you just won't get the drop event if there's a problem.
            try { FS.mkdir("/tmp/filedrop"); } catch (e) {}

            SDL3.eventHandlerDropDrop = function(event) {
                event.preventDefault();
                if (event.dataTransfer.types.includes("text/plain")) {
                    let plain_text = stringToNewUTF8(event.dataTransfer.getData("text/plain"));
                    _Emscripten_SendDragTextEvent(data, plain_text);
                    _Emscripten_force_free(plain_text);
                } else if (event.dataTransfer.types.includes("Files")) {
                    let files_read = 0;
                    const files_to_read = event.dataTransfer.files.length;
                    for (let i = 0; i < files_to_read; i++) {
                        const file = event.dataTransfer.files.item(i);
                        const file_reader = new FileReader();
                        file_reader.readAsArrayBuffer(file);
                        file_reader.onload = function(event) {
                            const fs_dropdir = `/tmp/filedrop/${SDL3.drop_count}`;
                            SDL3.drop_count += 1;

                            const fs_filepath = `${fs_dropdir}/${file.name}`;
                            const c_fs_filepath = stringToNewUTF8(fs_filepath);
                            const contents_array8 = new Uint8Array(event.target.result);

                            try {
                                FS.mkdir(fs_dropdir);
                                var stream = FS.open(fs_filepath, "w");
                                FS.write(stream, contents_array8, 0, contents_array8.length, 0);
                                FS.close(stream);
                                _Emscripten_SendDragFileEvent(data, c_fs_filepath);
                            } catch (e) {
                                // if this threw an exception at any point, we skip this drop event. Sorry!
                            }
                            _Emscripten_force_free(c_fs_filepath);
                            onFileRead();
                        };
                        file_reader.onerror = function(event) {
                            // Handle when error occurs to ensure that the drag event can still complete
                            onFileRead();
                        };
                    }
                    function onFileRead() {
                        ++files_read;
                        if (files_read === files_to_read) {
                            _Emscripten_SendDragCompleteEvent(data);
                        }
                    }
                }
                _Emscripten_SendDragCompleteEvent(data);
            };
            target.addEventListener("drop", SDL3.eventHandlerDropDrop);

            SDL3.eventHandlerDropDragend = function(event) {
                event.preventDefault();
                _Emscripten_SendDragCompleteEvent(data);
            };
            target.addEventListener("dragend", SDL3.eventHandlerDropDragend);
            target.addEventListener("dragleave", SDL3.eventHandlerDropDragend);
        }
    }"#
);

// Emscripten_unset_drag_event_callbacks()'s MAIN_THREAD_EM_ASM, verbatim.
//
// FIXME (upstream): `recursive_remove` is declared and never called (the
// `("/tmp/filedrop")` after its body is an expression statement of its
// own), so only the `FS.rmdir()` after it runs, and it throws (an uncaught
// `ErrnoError`, which ends the program) where the directory isn't there or
// isn't empty: when a second window is destroyed (the first one removed
// it), and once anything was dropped (the files are in it).
// `recursive_remove` would stop at the first subdirectory too: `FS.isDir()`
// wants the mode, not the path.
crate::em_js_dollar!(
    /// Removes the listeners of [`sdl3_set_drag_event_callbacks`] from the
    /// canvas `canvas_id`, and `/tmp/filedrop` (which throws if it isn't
    /// there, or isn't empty).
    pub fn sdl3_unset_drag_event_callbacks(canvas_id: *const c_char),
    r#"{
        var target = document.querySelector(UTF8ToString($0));
        if (target) {
            var SDL3 = Module['SDL3'];
            target.removeEventListener("dragleave", SDL3.eventHandlerDropDragend);
            target.removeEventListener("dragend", SDL3.eventHandlerDropDragend);
            target.removeEventListener("drop", SDL3.eventHandlerDropDrop);
            SDL3.drop_count = undefined;

            function recursive_remove(dirpath) {
                FS.readdir(dirpath).forEach((filename) => {
                    const p = `${dirpath}/${filename}`;
                    const p_s = FS.stat(p);
                    if (FS.isFile(p_s.mode)) {
                        FS.unlink(p);
                    } else if (FS.isDir(p)) {
                        recursive_remove(p);
                    }
                });
                FS.rmdir(dirpath);
            }("/tmp/filedrop");

            FS.rmdir("/tmp/filedrop");
            target.removeEventListener("dragover", SDL3.eventHandlerDropDragover);
            SDL3.eventHandlerDropDragover = undefined;
            SDL3.eventHandlerDropDrop = undefined;
            SDL3.eventHandlerDropDragend = undefined;
        }
    }"#
);

crate::em_js_callback!(
    /// `_Emscripten_HandleLockKeysCheck(window, capslock, numlock,
    /// scrolllock)`, which the lock-key check calls
    /// ([`sdl3_set_lock_keys_check`]).
    pub static EMSCRIPTEN_HANDLE_LOCK_KEYS_CHECK = "_Emscripten_HandleLockKeysCheck";
);

// Emscripten_RegisterEventHandlers()'s MAIN_THREAD_EM_ASM_INT, verbatim (its
// result, undefined, isn't used).
crate::em_js_dollar!(
    /// Listens to the document's `keydown` (replacing the listener before,
    /// if any): but for the lock keys' own, each calls
    /// `_Emscripten_HandleLockKeysCheck(window, …)` with the Caps Lock,
    /// Num Lock and Scroll Lock state the event has. The listener is
    /// `document.sdlEventHandlerLockKeysCheck`.
    pub fn sdl3_set_lock_keys_check(window: u32),
    r#"{
            var data = $0;
            // our keymod state can get confused in various ways (changed capslock when browser didn't have focus, etc), and you can't query the current
            //  state from the DOM, outside of a keyboard event, so catch keypresses globally and reset mod state if it's unexpectedly wrong. Best we can do.
            //  Note that this thing _only_ adjusts the lock keys if necessary; the real SDL keypress handling happens elsewhere.

            // Remove any prior listener first -- `addEventListener` does not deduplicate either.
            if (document.sdlEventHandlerLockKeysCheck) {
                document.removeEventListener("keydown", document.sdlEventHandlerLockKeysCheck);
            }

            document.sdlEventHandlerLockKeysCheck = function(event) {
                // don't try to adjust the state on the actual lock key presses; the normal key handler will catch that and adjust.
                if ((event.key != "CapsLock") && (event.key != "NumLock") && (event.key != "ScrollLock"))
                {
                    _Emscripten_HandleLockKeysCheck(Module['SDL3'].JSVarToCPtr(data), event.getModifierState("CapsLock"), event.getModifierState("NumLock"), event.getModifierState("ScrollLock"));
                }
            };
            document.addEventListener("keydown", document.sdlEventHandlerLockKeysCheck);
        }"#
);

// Emscripten_UnregisterEventHandlers()'s MAIN_THREAD_EM_ASM_INT, verbatim (its
// result isn't used).
crate::em_js_dollar!(
    /// Removes the listener of [`sdl3_set_lock_keys_check`] (the function
    /// stays `document.sdlEventHandlerLockKeysCheck`).
    pub fn sdl3_unset_lock_keys_check(),
    r#"{
            document.removeEventListener("keydown", document.sdlEventHandlerLockKeysCheck);
        }"#
);
