// Tests of the Emscripten build linked with JSPI.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! The program of `emscripten_asyncify.rs` linked with JSPI, the
//! WebAssembly JavaScript Promise Integration (`-sJSPI`, also
//! `-sASYNCIFY=2`), with which `emscripten_has_asyncify()` is 2 and SDL
//! sleeps the same way, suspending the WebAssembly instead of unwinding
//! it. JSPI has no trouble with the WebAssembly exceptions Rust unwinds
//! with. A sleep can suspend only what JavaScript called through
//! `WebAssembly.promising`, which emscripten wraps `main` in, but not the
//! function the main loop calls: in the main callbacks a sleep throws a
//! `SuspendError`, as upstream's does, so there the iterations turn the hint
//! off.
//!
//! JSPI is in Chrome and Chromium 137 and later; node 22 hasn't the
//! current API (`WebAssembly.Suspending`), and under such a node the runner
//! skips the program as `jspi` (the program asks for it, `jspi` in its
//! `sdl3_runner` section). `SDL3_EMSCRIPTEN_BROWSER=1
//! SDL3_TEST_REQUIRE=browser,webgl cargo test -p sdl3 --target
//! wasm32-unknown-emscripten --test emscripten_jspi` runs it as CI does.

#[cfg(target_os = "emscripten")]
mod asyncify_program;

#[cfg(not(target_os = "emscripten"))]
fn main() {}

#[cfg(target_os = "emscripten")]
fn main() -> std::process::ExitCode {
    asyncify_program::main(asyncify_program::Mode { jspi: true })
}

/// The program's request for the browser, and for JSPI (see
/// `emscripten_browser.rs`).
#[cfg(target_os = "emscripten")]
#[used]
#[link_section = "sdl3_runner"]
static RUNNER: [u8; 12] = *b"browser jspi";

/// The program's link options (see `tools/emscripten_link.py`).
#[cfg(target_os = "emscripten")]
#[used]
#[link_section = "sdl3_link"]
static LINK: [u8; 6] = *b"-sJSPI";
