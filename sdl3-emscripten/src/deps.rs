// The EM_JS_DEPS of Simple DirectMedia Layer's Emscripten sources, in one table.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! The program's dependency table: the translation of every
//! `EM_JS_DEPS(tag, "…")` in upstream SDL.
//!
//! emscripten writes into the glue only the JavaScript library functions
//! something asks for. clang's `EM_JS_DEPS` puts its list in an
//! `em_lib_deps` data section; emscripten reads the strings between the
//! `__start_em_lib_deps` and `__stop_em_lib_deps` symbols, which it exports
//! whenever they are defined, and adds them to the program. Rust can't make
//! that section (see [`js`](crate::js)), so this module defines the two
//! symbols itself: the list, and a byte right after it. There can be only
//! one such table in a program, so it lists everything SDL's Emscripten code
//! uses, for every backend; later code needs no table of its own (it adds to
//! [`DEPS`]).

/// The JavaScript library functions SDL's Emscripten code needs.
///
/// Upstream's `EM_JS_DEPS`, file by file:
///
/// | upstream | deps |
/// |---|---|
/// | `audio/emscripten/SDL_emscriptenaudio.c` | `$autoResumeAudioContext,$dynCall` |
/// | `joystick/emscripten/SDL_sysjoystick.c` | `$stringToUTF8` |
/// | `main/emscripten/SDL_sysmain_runapp.c` | `$dynCall,$stringToNewUTF8` |
/// | `misc/emscripten/SDL_sysurl.c` | `$UTF8ToString` |
/// | `video/emscripten/SDL_emscriptenmouse.c` | `$stringToUTF8,$UTF8ToString` |
/// | `video/emscripten/SDL_emscriptenevents.c` | `$writeArrayToMemory` |
///
/// and what else the JavaScript uses that emscripten finds by itself in
/// C's `EM_ASM` text but not in the text the [`em_asm!`](crate::em_asm)
/// macros compile at run time, so it's listed here:
///
/// * `$setValue`: `SDL_emscriptenaudio.c` and `locale/emscripten`;
/// * `$getWasmTableEntry`: the callback registry ([`Callback`](crate::Callback));
/// * `$Browser`: `SDL_emscriptenframebuffer.c` (`Browser.createContext`),
///   which upstream gets from whatever else pulls `$Browser` in.
///
/// `FS` (drag and drop, `SDL_sysmain_runapp.c`) is in every program that
/// uses the file system, as Rust's standard library does. `IDBFS` is a
/// separate JavaScript library an application links (`-lidbfs.js`).
pub const DEPS: &str = "$autoResumeAudioContext,$dynCall,$stringToUTF8,$stringToNewUTF8,\
$UTF8ToString,$writeArrayToMemory,$setValue,$getWasmTableEntry,$Browser";

const DEPS_LEN: usize = DEPS.len() + 1;

const fn nul_terminated(s: &str) -> [u8; DEPS_LEN] {
    let b = s.as_bytes();
    let mut out = [0u8; DEPS_LEN];
    let mut i = 0;
    while i < b.len() {
        assert!(b[i] != 0 && b[i] != b'\\');
        out[i] = b[i];
        i += 1;
    }
    out
}

// The two symbols, in one codegen unit, one after the other: wasm-ld keeps
// their order in `.rodata` (both are byte arrays, with no padding between),
// so the strings emscripten reads are exactly DEPS. tests/js.rs checks the
// addresses in a linked program.
#[allow(non_upper_case_globals)]
#[used]
#[no_mangle]
static __start_em_lib_deps: [u8; DEPS_LEN] = nul_terminated(DEPS);
#[allow(non_upper_case_globals)]
#[used]
#[no_mangle]
static __stop_em_lib_deps: [u8; 1] = [0];

/// What every [`em_js!`](crate::em_js) function and every `em_asm` call
/// refers to, so that a program that uses any of them links the dependency
/// table and the `em_asm` dispatchers: a `#[used]` static is only kept from
/// an archive member that the linker loads for some other reason.
#[doc(hidden)]
#[derive(Debug)]
pub struct Keep {
    deps: [&'static [u8]; 2],
    // Only the linker reads it (through the references it holds).
    #[allow(dead_code)]
    asm: &'static [&'static [u8]; 5],
}

#[doc(hidden)]
pub static KEEP: Keep = Keep {
    deps: [&__start_em_lib_deps, &__stop_em_lib_deps],
    asm: &crate::js::ASM,
};

impl Keep {
    /// The table's two symbols' addresses (for the tests).
    #[doc(hidden)]
    pub fn table(&self) -> (*const u8, *const u8) {
        (self.deps[0].as_ptr(), self.deps[1].as_ptr())
    }
}
