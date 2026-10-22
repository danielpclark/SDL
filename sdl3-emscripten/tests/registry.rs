// Tests of the callback registry.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! Under node. This program is linked with `-sEXPORTED_FUNCTIONS=_main`
//! (build.rs), as an application with its own export list is: rustc's list
//! of `#[no_mangle]` functions is gone, and the registry still works.

#![cfg(target_os = "emscripten")]

use sdl3_emscripten::testing::*;
use sdl3_emscripten::{em_asm_double, em_asm_int};
use std::ffi::c_int;

extern "C" fn scaled(x: c_int, y: f64) -> f64 {
    x as f64 * y
}

extern "C" fn offset(x: c_int, y: f64) -> f64 {
    x as f64 + y
}

#[test]
fn registry() {
    assert_eq!(TEST_CALLBACK.name(), "_sdl3_test_callback");
    unsafe {
        // before registration: the forwarder is there, the function isn't
        assert_eq!(
            em_asm_int!(
                r#"
                if (typeof _sdl3_test_callback != 'function') return -1;
                if (typeof Module['_sdl3_test_callback'] != 'undefined') return -2;
                try { _sdl3_test_callback(1, 2); } catch (e) { return e instanceof TypeError ? 1 : -3; }
                return -4;
            "#
            ),
            1
        );

        TEST_CALLBACK.register(scaled as *const ());
        // the glue-scope name (from em_js! and from em_asm!), Module['_name']
        // and Module._name
        assert_eq!(sdl3_test_call_callback(4, 2.5), 10.0);
        assert_eq!(
            em_asm_double!("return _sdl3_test_callback($0, 0.5);", 3i32),
            1.5
        );
        assert_eq!(
            em_asm_double!("return Module['_sdl3_test_callback'](2, 2);"),
            4.0
        );
        assert_eq!(
            em_asm_double!("return Module._sdl3_test_callback(5, 3);"),
            15.0
        );

        // registering again replaces it
        TEST_CALLBACK.register(offset as *const ());
        assert_eq!(sdl3_test_call_callback(4, 2.5), 6.5);
    }
}

/// The export list really is the program's own: the `#[no_mangle]`
/// function rustc would export is linked, but JavaScript doesn't see it.
#[test]
fn exports_are_replaced() {
    assert_eq!(sdl3_emscripten_test_export(1), 2);
    unsafe {
        assert_eq!(
            em_asm_int!(
                "return typeof Module['_sdl3_emscripten_test_export'] == 'undefined' && typeof _main != 'undefined' ? 1 : 0;"
            ),
            1
        );
    }
}

/// The names SDL's JavaScript calls are declared (sdl3 registers them).
#[test]
fn sdl_names() {
    use sdl3_emscripten::stdlib::*;
    let names: Vec<_> = [
        &SDL_MALLOC,
        &SDL_CALLOC,
        &SDL_REALLOC,
        &SDL_FREE,
        &EMSCRIPTEN_FORCE_FREE,
    ]
    .iter()
    .map(|c| c.name())
    .collect();
    assert_eq!(
        names,
        [
            "_SDL_malloc",
            "_SDL_calloc",
            "_SDL_realloc",
            "_SDL_free",
            "_Emscripten_force_free"
        ]
    );
}
