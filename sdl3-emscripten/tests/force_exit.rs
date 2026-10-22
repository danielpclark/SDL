// A test of emscripten_force_exit(), SDL_ExitProcess()'s way out on Emscripten.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! A program of its own (`harness = false`): it ends in the middle of
//! `main`. emscripten_force_exit(0) must end the program with status 0
//! from there. (It is declared `-> !`: if it returned, the program would
//! reach the `unreachable` after the call and trap, a failure.)

#[cfg(target_os = "emscripten")]
fn main() {
    use sdl3_emscripten::sys::{emscripten_cancel_main_loop, emscripten_force_exit};
    println!("force_exit: exiting with status 0");
    // SAFETY: plain calls (there is no main loop to cancel; that's harmless).
    unsafe {
        emscripten_cancel_main_loop();
        emscripten_force_exit(0);
    }
}

#[cfg(not(target_os = "emscripten"))]
fn main() {}
