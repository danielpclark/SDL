// Tests of the Emscripten build linked with Asyncify.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! What SDL does differently in a program linked with Asyncify
//! (`-sASYNCIFY=1`): it sleeps with `emscripten_sleep()`, back in the
//! browser's event loop, where upstream's does (unless
//! `SDL_HINT_EMSCRIPTEN_ASYNCIFY` is false, which each step checks too):
//!
//! * `asyncify`: `emscripten_has_asyncify()`;
//! * `delay`: `delay()` and `delay_precise()`, during which a `setTimeout()`
//!   set before them runs (without the hint, after);
//! * `timers`: a timer that fires during a `delay()`;
//! * `wait_event`: `wait_timeout()`, which gets the event a timer pushes
//!   meanwhile (without the hint it can't);
//! * `message_box`: the message boxes as a modal `<dialog>` that
//!   `show_message_box()` waits on, answered by the page (Enter, Escape, a
//!   click, the hover colors, a dialog without a default button, the
//!   simple message box), and `alert()` without the hint;
//! * `framebuffer`, `gl_swap`: a window surface's update and a WebGL
//!   buffer swap, which give the browser its turn (`emscripten_sleep(0)`);
//! * `main_callbacks`: a `delay()` in the main callbacks' iterations.
//!
//! The program is linked with `-sASYNCIFY=1` by its own request to
//! `tools/emscripten_link.py` (the target's linker in .cargo/config.toml):
//! a custom section `sdl3_link`. Its steps are in `asyncify_program/`,
//! which `emscripten_jspi.rs` shares. The trace is compared step by step
//! with upstream's C SDL built the same way and run on the same page (and
//! under node, without the browser's steps).
//!
//! Rust's standard library unwinds with WebAssembly exceptions
//! (`-fwasm-exceptions`), which Asyncify doesn't fully support (emcc
//! warns): a sleep while a panic unwinds (in a `Drop`) breaks the program.
//! Nothing here sleeps then.
//!
//! Each step prints a libtest-like `test NAME ... ok` line (with both
//! traces when they differ); under node the browser's steps skip as
//! `browser`. `SDL3_EMSCRIPTEN_BROWSER=1 SDL3_TEST_REQUIRE=browser,webgl
//! cargo test -p sdl3 --target wasm32-unknown-emscripten --test
//! emscripten_asyncify` runs it as CI does.

#[cfg(target_os = "emscripten")]
mod asyncify_program;

#[cfg(not(target_os = "emscripten"))]
fn main() {}

#[cfg(target_os = "emscripten")]
fn main() -> std::process::ExitCode {
    asyncify_program::main(asyncify_program::Mode { jspi: false })
}

/// The program's request for the browser (see `emscripten_browser.rs`).
#[cfg(target_os = "emscripten")]
#[used]
#[link_section = "sdl3_runner"]
static RUNNER: [u8; 7] = *b"browser";

/// The program's link options (see `tools/emscripten_link.py`).
#[cfg(target_os = "emscripten")]
#[used]
#[link_section = "sdl3_link"]
static LINK: [u8; 40] = *b"-sASYNCIFY=1\0-sASYNCIFY_STACK_SIZE=65536";
