// Rust translation of the JavaScript in src/main/emscripten/SDL_sysmain_runapp.c
// from Simple DirectMedia Layer.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! The JavaScript of `SDL_sysmain_runapp.c`: `SDL_RunApp()`'s URL
//! parameters and, with the `idbfs` feature, its persistent storage.

use core::ffi::c_void;
#[cfg(feature = "idbfs")]
use core::ffi::{c_char, c_int};

// SDL_RunApp()'s first MAIN_THREAD_EM_ASM, verbatim. The page's `window`
// is the browser's: under node there is none, and the first line throws a
// ReferenceError, as upstream's does.
crate::em_js_dollar!(
    /// Moves the page's URL parameters that start with `SDL_` into the
    /// environment, through `setenv` (a function pointer: `SDL_setenv_unsafe`,
    /// called as `setenv(name, value, 1)`). The strings it passes are
    /// emscripten's (`stringToNewUTF8`); it frees them with
    /// `_Emscripten_force_free`, which must be registered.
    pub fn sdl3_runapp_url_params(setenv: *const c_void),
    r#"{
        var parms = new URLSearchParams(window.location.search);
        for (const [key, value] of parms) {
            if (key.startsWith("SDL_")) {
                var ckey = stringToNewUTF8(key);
                var cvalue = stringToNewUTF8(value);
                if ((ckey != 0) && (cvalue != 0)) {
                    //console.log("Setting SDL env var '" + key + "' to '" + value + "' ...");
                    dynCall('iiii', $0, [ckey, cvalue, 1]);
                }
                _Emscripten_force_free(ckey);  // these must use free(), not SDL_free()!
                _Emscripten_force_free(cvalue);
            }
        }
    }"#
);

// Upstream's SDL_EMSCRIPTEN_PERSISTENT_PATH_STRING part: IDBFS is a
// JavaScript library of its own, which this links (-lidbfs.js), so the
// feature needs no link option.
#[cfg(feature = "idbfs")]
#[link(name = "idbfs.js")]
extern "C" {}

#[cfg(feature = "idbfs")]
crate::em_js_dollar!(
    /// Mounts IndexedDB storage (IDBFS, persisted automatically) at
    /// `persistent_path`, then loads what it holds and, when that's done
    /// (it's asynchronous in a browser), calls the main function through
    /// [`CALL_SDL_EMSCRIPTEN_MAIN_FUNCTION`] with `argc`, `argv` and
    /// `main_function`, failure or not. Upstream's second
    /// `MAIN_THREAD_EM_ASM` of `SDL_RunApp()`, verbatim.
    pub fn sdl3_runapp_persistent_path(
        persistent_path: *const c_char,
        argc: c_int,
        argv: *mut *mut c_char,
        main_function: *mut c_void,
    ),
    r#"{
        const persistent_path = UTF8ToString($0);
        const argc = $1;
        const argv = $2;
        const mainFunction = $3;
        //console.log("SDL is automounting persistent storage to '" + persistent_path + "' ...please wait.");
        FS.mkdirTree(persistent_path);
        FS.mount(IDBFS, { autoPersist: true }, persistent_path);
        FS.syncfs(true, function(err) {
            if (err) {
                console.error(`WARNING: Failed to populate persistent store at '${persistent_path}' (${err.name}: ${err.message}). Save games likely lost?`);
            }
            _CallSDLEmscriptenMainFunction(argc, argv, mainFunction);   // error or not, start the actual SDL_main().
        });
    }"#
);

#[cfg(feature = "idbfs")]
crate::em_js_callback!(
    /// `_CallSDLEmscriptenMainFunction(argc, argv, mainFunction)`, upstream's
    /// `EMSCRIPTEN_KEEPALIVE` function that the persistent storage's
    /// JavaScript calls once it's loaded.
    pub static CALL_SDL_EMSCRIPTEN_MAIN_FUNCTION = "_CallSDLEmscriptenMainFunction";
);
