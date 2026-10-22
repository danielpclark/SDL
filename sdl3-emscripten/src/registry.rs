// The Rust functions JavaScript calls by name (upstream's EMSCRIPTEN_KEEPALIVE).
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! The callback registry: Rust functions JavaScript calls by name.
//!
//! Upstream's JavaScript calls C functions marked `EMSCRIPTEN_KEEPALIVE`
//! by their exported names: `_SDL_malloc(n)`, `_Emscripten_force_free(p)`,
//! `Module._SDLEmscriptenThreadIterate(…)`. Rust could export them the same
//! way (`#[no_mangle]`: rustc lists every such function in
//! `-sEXPORTED_FUNCTIONS`), but an application that passes its own
//! `-sEXPORTED_FUNCTIONS` replaces that list, and they're gone. So nothing
//! is exported: a function is *registered* at run time, by its address in
//! the function table, which is what a Rust function pointer is on wasm.
//!
//! [`em_js_callback!`](crate::em_js_callback) declares the name; it gives
//! a [`Callback`], and [`Callback::register`] (at SDL's initialization)
//! stores the function. JavaScript then finds it both ways upstream's code
//! calls such functions:
//!
//! * `Module['_name']` (and `Module._name`): set to
//!   `getWasmTableEntry(f)`, the function itself;
//! * `_name(…)`, a name in the glue's scope: a JavaScript function, written
//!   into the glue at link time, that forwards to `Module['_name']`.
//!
//! Calling one before it's registered throws a `TypeError` (as calling an
//! unexported function does in C).

use core::ffi::{c_char, c_void, CStr};

crate::em_js!(
    fn sdl3_register_callback(name: *const c_char, f: *const c_void),
    r#"{
    Module[UTF8ToString(name)] = getWasmTableEntry(f);
}"#
);

/// A Rust function JavaScript calls by name; see the
/// [`em_js_callback!`](crate::em_js_callback) macro, which makes one.
#[derive(Debug)]
pub struct Callback {
    name: &'static CStr,
    forwarder: &'static [u8],
}

impl Callback {
    #[doc(hidden)]
    pub const fn __new(name: &'static [u8], forwarder: &'static [u8]) -> Callback {
        let name = match CStr::from_bytes_with_nul(name) {
            Ok(name) => name,
            Err(_) => panic!("a callback name must be NUL-terminated, without other NULs"),
        };
        Callback { name, forwarder }
    }

    /// The JavaScript name: `_SDL_malloc`, …
    pub fn name(&self) -> &'static str {
        match self.name.to_str() {
            Ok(name) => name,
            Err(_) => unreachable!("the name is a Rust string literal"),
        }
    }

    /// Makes `f` the function JavaScript calls by this name (and replaces the
    /// one before, if any).
    ///
    /// # Safety
    ///
    /// `f` must be an `extern "C" fn` (cast with `as *const ()`), with the
    /// parameters and result the JavaScript calls it with, and must stay
    /// callable for as long as JavaScript may call it.
    pub unsafe fn register(&self, f: *const ()) {
        core::hint::black_box(self.forwarder);
        // SAFETY: the name is NUL-terminated, the function is the caller's.
        unsafe { sdl3_register_callback(self.name.as_ptr(), f.cast()) }
    }
}

/// Declares a [`Callback`]: a name JavaScript calls a registered Rust
/// function by, as `Module['_name']` and as `_name` in the glue's scope.
///
/// ```ignore
/// em_js_callback!(
///     /// `_SDL_malloc`, which upstream's JavaScript calls.
///     pub static SDL_MALLOC = "_SDL_malloc";
/// );
///
/// extern "C" fn malloc(size: usize) -> *mut c_void { /* ... */ }
/// unsafe { SDL_MALLOC.register(malloc as *const ()) };
/// // JavaScript: var p = _SDL_malloc(16); /* ... */ _SDL_free(p);
/// ```
///
/// Like [`em_js!`](crate::em_js), it must be used in a library crate (it
/// writes the forwarding function `_name` into the glue). The name must be a
/// JavaScript identifier that nothing else in the program uses.
#[macro_export]
macro_rules! em_js_callback {
    ($(#[$attr:meta])* $vis:vis static $id:ident = $name:literal $(;)?) => {
        $(#[$attr])*
        $vis static $id: $crate::Callback = {
            const __TEXT: &str = concat!(
                "(...args)<::>{ return Module['", $name, "'](...args); }"
            );
            #[used]
            #[export_name = concat!("__em_js__", $name)]
            static __EM_JS: [u8; $crate::__private::js_len(__TEXT)] =
                $crate::__private::js_bytes(__TEXT);
            $crate::Callback::__new(concat!($name, "\0").as_bytes(), &__EM_JS)
        };
    };
}
