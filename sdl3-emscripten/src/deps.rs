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

use crate::sys;
use core::ffi::c_void;

/// The table: `DEPS`, and the JavaScript function that names its functions.
macro_rules! deps {
    ($(#[$attr:meta])* $first:ident $(, $rest:ident)* $(,)?) => {
        $(#[$attr])*
        pub const DEPS: &str = concat!("$", stringify!($first) $(, ",$", stringify!($rest))*);

        /// `sdl3_em_asm_deps()`: the table's functions, named (see [`DEPS`]).
        pub(crate) const DEPS_JS: &str = concat!(
            "()<::>{ return [", stringify!($first) $(, ", ", stringify!($rest))*, "]; }"
        );
    };
}

deps!(
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
    ///
    /// Two things more make the table work in optimized builds:
    ///
    /// * At `-O2` and up emscripten drops the glue's functions that nothing
    ///   in the glue names. The text an `em_asm` macro compiles at run time
    ///   isn't in the glue, so a JavaScript function, `sdl3_em_asm_deps`,
    ///   names every function of the table, and the `em_asm` compiler names
    ///   it.
    /// * `$stringToNewUTF8` calls C's `malloc` (`_malloc`), which must then be
    ///   a wasm export. emscripten exports the C functions a JavaScript
    ///   library function uses when the program imports that function, but it
    ///   reads this table only after the link, too late for that. So, next to
    ///   the table, a static holds the address of an `html5.h` function whose
    ///   JavaScript uses `malloc`,
    ///   `emscripten_set_visibilitychange_callback_on_thread` (which SDL's
    ///   video backend calls anyway).
    autoResumeAudioContext,
    dynCall,
    stringToUTF8,
    stringToNewUTF8,
    UTF8ToString,
    writeArrayToMemory,
    setValue,
    getWasmTableEntry,
    Browser,
);

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
// addresses in a linked program. (In an optimized link emscripten then
// zeroes the bytes, which nothing reads at run time.)
#[allow(non_upper_case_globals)]
#[used]
#[no_mangle]
static __start_em_lib_deps: [u8; DEPS_LEN] = nul_terminated(DEPS);
#[allow(non_upper_case_globals)]
#[used]
#[no_mangle]
static __stop_em_lib_deps: [u8; 1] = [0];

/// A function whose JavaScript uses `malloc` (see [`DEPS`]).
type MallocUser = unsafe extern "C" fn(
    *mut c_void,
    bool,
    sys::em_visibilitychange_callback_func,
    sys::pthread_t,
) -> sys::EMSCRIPTEN_RESULT;

/// The address of `emscripten_set_visibilitychange_callback_on_thread`,
/// which makes `malloc` a wasm export. `#[used]` as the table is: with LTO,
/// every `#[used]` static of the program is kept (the table with it) even
/// when nothing refers to [`KEEP`].
#[used]
static MALLOC_USER: MallocUser = sys::emscripten_set_visibilitychange_callback_on_thread;

/// What every [`em_js!`](crate::em_js) function and every `em_asm` call
/// refers to, so that a program that uses any of them links the dependency
/// table and the `em_asm` dispatchers (a `#[used]` static is only kept from
/// an archive member that the linker loads for some other reason), and has
/// `malloc` exported.
#[doc(hidden)]
#[derive(Debug)]
pub struct Keep {
    deps: [&'static [u8]; 2],
    // Only the linker reads these (through the references they hold).
    #[allow(dead_code)]
    asm: &'static [&'static [u8]; 6],
    #[allow(dead_code)]
    malloc: &'static MallocUser,
}

#[doc(hidden)]
pub static KEEP: Keep = Keep {
    deps: [&__start_em_lib_deps, &__stop_em_lib_deps],
    asm: &crate::js::ASM,
    malloc: &MALLOC_USER,
};

impl Keep {
    /// The table's two symbols' addresses (for the tests).
    #[doc(hidden)]
    pub fn table(&self) -> (*const u8, *const u8) {
        (self.deps[0].as_ptr(), self.deps[1].as_ptr())
    }
}
