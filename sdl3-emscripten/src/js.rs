// Rust translation of the EM_JS / EM_ASM macros of <emscripten/em_js.h>
// and <emscripten/em_asm.h>, as SDL uses them.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! JavaScript in a Rust program: [`em_js!`](crate::em_js), [`em_js_dollar!`](crate::em_js_dollar) and the
//! `em_asm` family.
//!
//! # How emscripten finds the JavaScript
//!
//! clang's `EM_JS(ret, name, params, { body })` defines an exported data
//! symbol `__em_js__name` pointing at the NUL-terminated text
//! `"(params)<::>{ body }"`, and declares `name` as an import from `env`.
//! After the link, emscripten reads every `__em_js__*` export and writes
//! `function name(params) { body }` into the JavaScript glue, at its top
//! level (so the body sees `Module`, `HEAPU8`, the library functions, and the
//! other `EM_JS` functions), and the import resolves to it. [`em_js!`](crate::em_js) makes
//! the same symbol from a `#[used]` static with `#[export_name]`, and the
//! import from an `extern` block with `#[link(wasm_import_module = "env")]`.
//!
//! clang's `EM_ASM` instead puts its text in a data section, `em_asm`, read
//! through the `__start_em_asm`/`__stop_em_asm` symbols the linker makes.
//! `#[link_section]` on wasm makes a *custom* section, not a data segment, so
//! there are no such symbols for Rust; the `em_asm` macros here are built on
//! `EM_JS` instead: three C-variadic `EM_JS` dispatchers (one per return type,
//! called like `emscripten_asm_const_int(code, sig, ...)`) compile the
//! snippet's text, read from memory, with `eval` the first time it runs.
//!
//! # The text
//!
//! The JavaScript is the macro's string literal, byte for byte (use raw
//! strings, `r#"…"#`): comments, newlines, quotes, backslashes and regular
//! expressions all reach the program unchanged. (emscripten turns `\\` into
//! `\` when it reads `EM_JS` text, a leftover of the JSON it once went
//! through; the macros double every backslash so that it gets the text back.
//! C's `EM_ASM` text goes through the preprocessor's `#` first, so a C body
//! with a doubled backslash differs; SDL's have only single ones, so its
//! bodies paste verbatim.) The text can't contain a NUL.
//!
//! # Values
//!
//! The parameters and return values are wasm values: `i32`/`u32`/`bool` and
//! the smaller integers are numbers, `i64`/`u64` are `BigInt`s, `f32`/`f64`
//! are numbers, and pointers are numbers, addresses into `HEAPU8` & co. As in
//! C, a pointer at or above 2 GB (with `-sMAXIMUM_MEMORY` past 2 GB) reaches
//! an [`em_js!`](crate::em_js) function as a negative number; write `ptr >>> 0` where that
//! can happen. The `em_asm` macros read pointers unsigned.

use core::ffi::{c_char, c_int, c_void};

// ---------------------------------------------------------------------------
// Compile-time text helpers

/// The size of [`js_bytes`]'s array for `s`: every byte, with backslashes
/// doubled, plus the NUL.
#[doc(hidden)]
pub const fn js_len(s: &str) -> usize {
    let b = s.as_bytes();
    let mut i = 0;
    let mut n = 0;
    while i < b.len() {
        if b[i] == 0 {
            panic!("JavaScript text can't contain a NUL byte");
        }
        n += if b[i] == b'\\' { 2 } else { 1 };
        i += 1;
    }
    n + 1
}

/// `s` as a NUL-terminated array, each backslash doubled to undo
/// emscripten's `\\` → `\` when it reads the text back
/// (`tools/extract_metadata.py`, `data_to_string()`).
#[doc(hidden)]
pub const fn js_bytes<const N: usize>(s: &str) -> [u8; N] {
    let b = s.as_bytes();
    let mut out = [0u8; N];
    let mut i = 0;
    let mut o = 0;
    while i < b.len() {
        out[o] = b[i];
        o += 1;
        if b[i] == b'\\' {
            out[o] = b'\\';
            o += 1;
        }
        i += 1;
    }
    assert!(o + 1 == N, "js_bytes: wrong length");
    out
}

const fn is_space(c: u8) -> bool {
    matches!(c, b' ' | b'\t' | b'\n' | b'\r')
}

/// An [`em_js!`](crate::em_js) body must be a `{ … }` block, as emscripten writes
/// `function name(params) BODY`.
#[doc(hidden)]
pub const fn body_check(s: &str) {
    let b = s.as_bytes();
    let mut start = 0;
    while start < b.len() && is_space(b[start]) {
        start += 1;
    }
    let mut end = b.len();
    while end > start && is_space(b[end - 1]) {
        end -= 1;
    }
    if end - start < 2 || b[start] != b'{' || b[end - 1] != b'}' {
        panic!("an em_js! body must be a JavaScript block, in braces");
    }
}

/// An `em_asm` snippet is read up to its NUL at run time.
#[doc(hidden)]
pub const fn asm_code_check(s: &str) {
    let b = s.as_bytes();
    let mut i = 0;
    while i < b.len() {
        if b[i] == 0 {
            panic!("JavaScript text can't contain a NUL byte");
        }
        i += 1;
    }
}

// ---------------------------------------------------------------------------
// em_js!

/// A JavaScript function callable from Rust: the translation of clang's
/// `EM_JS(ret, name, params, { body })`, the main form.
///
/// ```ignore
/// em_js!(
///     /// Writes the page's title into `buf`.
///     pub fn sdl3_get_title(buf: *mut u8, len: i32) -> i32,
///     r#"{
///         // document.title, as UTF-8
///         stringToUTF8(document.title, buf, len);
///         return lengthBytesUTF8(document.title);
///     }"#
/// );
/// let n = unsafe { sdl3_get_title(buf.as_mut_ptr(), buf.len() as i32) };
/// ```
///
/// This defines `unsafe fn sdl3_get_title`, which calls the JavaScript
/// function `sdl3_get_title(buf, len) { … }` that emscripten writes into the
/// program's glue at link time (its parameters are the Rust parameters'
/// names). The text is checked by emscripten when the program is linked
/// (and by `node --check` for side modules), is seen by the Closure
/// compiler, and needs no `eval`: prefer this form to [`em_asm!`](crate::em_asm).
///
/// * The body must be a `{ … }` block; see the [module documentation](self)
///   for how the text and the values pass.
/// * The JavaScript name is global in the program: the functions of this
///   crate are named `sdl3_…` so that they can't meet emscripten's own
///   (`out`, `run`, `abort`, …) or an application's.
/// * The library functions the body uses must be in the program: those of
///   SDL's Emscripten code are in the one dependency table
///   ([`DEPS`](crate::DEPS)); `UTF8ToString`, `getValue`, `setValue`,
///   `getWasmTableEntry` and the `HEAP*` views emscripten adds by itself
///   when an `EM_JS` body mentions them.
/// * It must be used in a library crate (like this one): in an executable
///   crate rustc doesn't export the text (see the [crate
///   documentation](crate)).
/// * Calling it is `unsafe`: the JavaScript can read and write all of the
///   program's memory, and the types must match what it returns.
#[macro_export]
macro_rules! em_js {
    ($(#[$attr:meta])* $vis:vis fn $name:ident($($arg:ident : $ty:ty),* $(,)?) $(-> $ret:ty)?, $body:literal $(,)?) => {
        $crate::__em_js_fn!(
            [$(#[$attr])*] $vis $name ($($arg: $ty),*) ($($ret)?) $body,
            concat!("(", stringify!($($arg),*), ")<::>", $body)
        );
    };
}

/// [`em_js!`](crate::em_js) for upstream's `EM_ASM` bodies pasted verbatim: the
/// JavaScript sees the parameters as `$0`, `$1`, … (up to `$15`), as an
/// `EM_ASM` body does; the Rust names are only the wrapper's.
///
/// ```ignore
/// // upstream: MAIN_THREAD_EM_ASM_INT({ return $0 + $1; }, a, b);
/// em_js_dollar!(fn sdl3_add(a: i32, b: i32) -> i32, r#"{ return $0 + $1; }"#);
/// ```
#[macro_export]
macro_rules! em_js_dollar {
    ($(#[$attr:meta])* $vis:vis fn $name:ident($($arg:ident : $ty:ty),* $(,)?) $(-> $ret:ty)?, $body:literal $(,)?) => {
        $crate::__em_js_fn!(
            [$(#[$attr])*] $vis $name ($($arg: $ty),*) ($($ret)?) $body,
            concat!(
                "(",
                $crate::__dollar_params!([] ["$0" "$1" "$2" "$3" "$4" "$5" "$6" "$7" "$8" "$9" "$10" "$11" "$12" "$13" "$14" "$15"] $($ty),*),
                ")<::>",
                $body
            )
        );
    };
}

/// The function [`em_js!`](crate::em_js) and [`em_js_dollar!`](crate::em_js_dollar) define, and the
/// `__em_js__NAME` text. The `black_box` of the text's address is what makes
/// the linker load the archive member holding it when the wrapper is
/// inlined into another crate (a `#[used]` static in a member nothing
/// refers to is never linked); [`KEEP`](crate::__private::KEEP) brings the
/// dependency table and the `em_asm` dispatchers along.
#[doc(hidden)]
#[macro_export]
macro_rules! __em_js_fn {
    ([$($attr:tt)*] $vis:vis $name:ident ($($arg:ident: $ty:ty),*) ($($ret:ty)?) $body:literal, $text:expr) => {
        $($attr)*
        #[inline]
        #[allow(clippy::missing_safety_doc)]
        $vis unsafe fn $name($($arg: $ty),*) $(-> $ret)? {
            #[link(wasm_import_module = "env")]
            extern "C" {
                #[link_name = stringify!($name)]
                fn __import($($arg: $ty),*) $(-> $ret)?;
            }
            const _: () = $crate::__private::body_check($body);
            const __TEXT: &str = $text;
            #[used]
            #[export_name = concat!("__em_js__", stringify!($name))]
            static __EM_JS: [u8; $crate::__private::js_len(__TEXT)] = $crate::__private::js_bytes(__TEXT);
            ::core::hint::black_box((&__EM_JS, &$crate::__private::KEEP));
            // SAFETY: the caller's (this function is `unsafe`).
            unsafe { __import($($arg),*) }
        }
    };
}

/// `"$0, $1, …"`, one name per type.
#[doc(hidden)]
#[macro_export]
macro_rules! __dollar_params {
    ([] [$n:literal $($ns:literal)*] $t:ty $(, $rest:ty)*) => {
        $crate::__dollar_params!([$n] [$($ns)*] $($rest),*)
    };
    ([$($done:literal)+] [$n:literal $($ns:literal)*] $t:ty $(, $rest:ty)*) => {
        $crate::__dollar_params!([$($done)+ ", " $n] [$($ns)*] $($rest),*)
    };
    ([$($done:literal)*] [$($ns:literal)*]) => {
        concat!($($done),*)
    };
}

/// A JavaScript function in the glue that wasm doesn't import (emscripten
/// writes every `__em_js__*` text, imported or not), for the other functions
/// to call.
macro_rules! js_function {
    ($id:ident, $name:literal, $text:expr) => {
        #[used]
        #[export_name = concat!("__em_js__", $name)]
        pub(crate) static $id: [u8; js_len($text)] = js_bytes($text);
    };
}

// ---------------------------------------------------------------------------
// em_asm!: three C-variadic EM_JS dispatchers, like emscripten's
// emscripten_asm_const_int/_double/_ptr, called as (code, sig, argbuf): the
// variadic arguments are in memory at `argbuf`, laid out as C lays them out
// (each at its own alignment), and `sig` says what they are.

#[link(wasm_import_module = "env")]
extern "C" {
    /// The `em_asm!`/`em_asm_int!` dispatcher.
    #[doc(hidden)]
    pub fn sdl3_em_asm_int(code: *const c_char, sig: *const c_char, ...) -> c_int;
    /// The `em_asm_double!` dispatcher.
    #[doc(hidden)]
    pub fn sdl3_em_asm_double(code: *const c_char, sig: *const c_char, ...) -> f64;
    /// The `em_asm_ptr!` dispatcher.
    #[doc(hidden)]
    pub fn sdl3_em_asm_ptr(code: *const c_char, sig: *const c_char, ...) -> *mut c_void;
}

const DISPATCH: &str = "(code, sig, buf)<::>{ return sdl3_em_asm_run(code, sig, buf); }";
js_function!(ASM_INT, "sdl3_em_asm_int", DISPATCH);
js_function!(ASM_DOUBLE, "sdl3_em_asm_double", DISPATCH);
js_function!(ASM_PTR, "sdl3_em_asm_ptr", DISPATCH);

// The snippet at `code`, compiled on its first run and kept by address, called
// with the arguments read as emscripten's readEmAsmArgs() reads them: 'i' a
// signed 32-bit integer, 'p' an unsigned pointer, 'j' a 64-bit integer
// (BigInt) and 'd' a double, the last two 8-aligned.
js_function!(
    ASM_RUN,
    "sdl3_em_asm_run",
    r#"(code, sig, buf)<::>{
    code >>>= 0;
    sig >>>= 0;
    buf >>>= 0;
    var cache = sdl3_em_asm_run.cache || (sdl3_em_asm_run.cache = {});
    var f = cache[code];
    if (!f) {
        f = cache[code] = sdl3_em_asm_compile(UTF8ToString(code));
    }
    var args = [], ch;
    while ((ch = HEAPU8[sig++])) {
        if (ch == 105) {
            args.push(HEAP32[buf >>> 2]);
            buf += 4;
        } else if (ch == 112) {
            args.push(HEAPU32[buf >>> 2]);
            buf += 4;
        } else {
            buf += (8 - buf % 8) % 8;
            args.push(ch == 106 ? HEAP64[buf >>> 3] : HEAPF64[buf >>> 3]);
            buf += 8;
        }
    }
    return f.apply(null, args);
}"#
);

// The direct eval is what lets the snippet see the glue's scope (Module,
// HEAPU8, the library functions), as emscripten's ASM_CONSTS functions do;
// its only own name is the parameter. The '\n' ends a trailing // comment.
js_function!(
    ASM_COMPILE,
    "sdl3_em_asm_compile",
    r#"(sdl3_em_asm_text)<::>{
    return eval('(function($0, $1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15) {' + sdl3_em_asm_text + '\n})');
}"#
);

/// The texts of the dispatchers, for [`KEEP`](crate::__private::KEEP).
pub(crate) static ASM: [&[u8]; 5] = [&ASM_INT, &ASM_DOUBLE, &ASM_PTR, &ASM_RUN, &ASM_COMPILE];

/// A value an `em_asm` snippet can take: converted to the type C's variadic
/// call would pass ([`EmArg::Raw`]: `int` for the smaller integers and
/// `bool`, `double` for `f32`), as `EM_ASM`'s arguments are.
pub trait EmArg {
    /// The promoted type.
    type Raw: EmRaw;
    /// The promotion.
    fn em_raw(self) -> Self::Raw;
}

/// A promoted variadic argument type and its letter in the signature
/// string: `i` (32-bit integer), `j` (64-bit), `d` (double), `p` (pointer).
pub trait EmRaw {
    /// The signature letter.
    const SIG: u8;
}

/// The signature letter of an argument.
#[doc(hidden)]
pub const fn sig_of<T: EmRaw>(_: &T) -> u8 {
    T::SIG
}

macro_rules! raw {
    ($($t:ty => $c:literal),*) => {
        $(impl EmRaw for $t { const SIG: u8 = $c; })*
    };
}
raw!(i32 => b'i', u32 => b'i', i64 => b'j', u64 => b'j', f64 => b'd');
impl<T> EmRaw for *const T {
    const SIG: u8 = b'p';
}
impl<T> EmRaw for *mut T {
    const SIG: u8 = b'p';
}

macro_rules! arg {
    ($($t:ty => $r:ty),*) => {
        $(impl EmArg for $t {
            type Raw = $r;
            #[inline(always)]
            fn em_raw(self) -> $r {
                self as $r
            }
        })*
    };
}
arg!(i8 => i32, u8 => i32, i16 => i32, u16 => i32, i32 => i32, u32 => u32,
     i64 => i64, u64 => u64, f32 => f64, f64 => f64, isize => i32, usize => u32);
impl EmArg for bool {
    type Raw = i32;
    #[inline(always)]
    fn em_raw(self) -> i32 {
        self as i32
    }
}
impl<T> EmArg for *const T {
    type Raw = *const T;
    #[inline(always)]
    fn em_raw(self) -> Self {
        self
    }
}
impl<T> EmArg for *mut T {
    type Raw = *mut T;
    #[inline(always)]
    fn em_raw(self) -> Self {
        self
    }
}

/// The snippet's NUL-terminated text, read with `UTF8ToString` at run time
/// (not through emscripten, so the backslashes stay as they are).
#[doc(hidden)]
#[macro_export]
macro_rules! __em_asm_code {
    ($code:literal) => {{
        const _: () = $crate::__private::asm_code_check($code);
        static __EM_ASM_CODE: &str = concat!($code, "\0");
        __EM_ASM_CODE.as_ptr() as *const ::core::ffi::c_char
    }};
}

/// Converts each argument once (macro hygiene gives each `__a` its own
/// identity), then calls the dispatcher with the signature string.
#[doc(hidden)]
#[macro_export]
macro_rules! __em_asm_call {
    (@bind $f:path, $code:expr, [$($b:ident)*] $head:expr $(, $rest:expr)*) => {{
        let __a = $crate::EmArg::em_raw($head);
        $crate::__em_asm_call!(@bind $f, $code, [$($b)* __a] $($rest),*)
    }};
    (@bind $f:path, $code:expr, [$($b:ident)*]) => {{
        let __sig: [u8; 0 $(+ { let _ = stringify!($b); 1 })* + 1] =
            [$($crate::__private::sig_of(&$b),)* 0u8];
        ::core::hint::black_box(&$crate::__private::KEEP);
        $f($code, __sig.as_ptr() as *const ::core::ffi::c_char $(, $b)*)
    }};
}

/// An anonymous JavaScript snippet returning an `i32`: the translation of
/// `EM_ASM_INT({ … }, args…)`. The snippet sees its arguments as `$0`, `$1`,
/// … (at most 16); each must be an [`EmArg`] (integers, `bool`, floats, raw
/// pointers; give integer literals a suffix, `40i32`).
///
/// ```ignore
/// // upstream: MAIN_THREAD_EM_ASM_INT({ return window.innerWidth * $0; }, scale)
/// let w = unsafe { em_asm_int!(r#"return window.innerWidth * $0;"#, scale) };
/// ```
///
/// The snippet is compiled with a direct `eval` the first time it runs (and
/// kept): unlike [`em_js!`](crate::em_js)'s text it isn't checked at link time, so a
/// syntax error shows when it first runs, and
///
/// * it doesn't run under a Content Security Policy without
///   `'unsafe-eval'`;
/// * the Closure compiler (`--closure 1`) never sees it, so it can't use a
///   name of the glue that Closure renames (the `HEAP*` views, library
///   functions), and anything it reaches on `Module` must be quoted
///   (`Module['SDL3']`);
/// * the library functions it uses must be in the program through the
///   dependency table ([`DEPS`](crate::DEPS)): emscripten can't see them.
///
/// Prefer [`em_js!`](crate::em_js) or [`em_js_dollar!`](crate::em_js_dollar); this form is for one-off blocks.
/// It works in any crate, executables included. It must be used inside
/// `unsafe`: the snippet can read and write all of memory.
#[macro_export]
macro_rules! em_asm_int {
    ($code:literal $(, $arg:expr)* $(,)?) => {
        $crate::__em_asm_call!(@bind $crate::__private::sdl3_em_asm_int, $crate::__em_asm_code!($code), [] $($arg),*)
    };
}

/// [`em_asm_int!`](crate::em_asm_int) returning an `f64`: `EM_ASM_DOUBLE`.
#[macro_export]
macro_rules! em_asm_double {
    ($code:literal $(, $arg:expr)* $(,)?) => {
        $crate::__em_asm_call!(@bind $crate::__private::sdl3_em_asm_double, $crate::__em_asm_code!($code), [] $($arg),*)
    };
}

/// [`em_asm_int!`](crate::em_asm_int) returning a pointer (`*mut c_void`): `EM_ASM_PTR`.
#[macro_export]
macro_rules! em_asm_ptr {
    ($code:literal $(, $arg:expr)* $(,)?) => {
        $crate::__em_asm_call!(@bind $crate::__private::sdl3_em_asm_ptr, $crate::__em_asm_code!($code), [] $($arg),*)
    };
}

/// [`em_asm_int!`](crate::em_asm_int) without a result: `EM_ASM({ … }, args…)`.
///
/// ```ignore
/// unsafe { em_asm!(r#"console.log("x =", $0);"#, x) };
/// ```
#[macro_export]
macro_rules! em_asm {
    ($code:literal $(, $arg:expr)* $(,)?) => {{
        let _: ::core::ffi::c_int = $crate::em_asm_int!($code $(, $arg)*);
    }};
}

/// `MAIN_THREAD_EM_ASM`: [`em_asm!`](crate::em_asm). Without threads (upstream's default
/// Emscripten build, and this one's) every thread is the main one.
#[macro_export]
macro_rules! main_thread_em_asm {
    ($($t:tt)*) => {
        $crate::em_asm!($($t)*)
    };
}

/// `MAIN_THREAD_EM_ASM_INT`: [`em_asm_int!`](crate::em_asm_int) (no threads).
#[macro_export]
macro_rules! main_thread_em_asm_int {
    ($($t:tt)*) => {
        $crate::em_asm_int!($($t)*)
    };
}

/// `MAIN_THREAD_EM_ASM_DOUBLE`: [`em_asm_double!`](crate::em_asm_double) (no threads).
#[macro_export]
macro_rules! main_thread_em_asm_double {
    ($($t:tt)*) => {
        $crate::em_asm_double!($($t)*)
    };
}

/// `MAIN_THREAD_EM_ASM_PTR`: [`em_asm_ptr!`](crate::em_asm_ptr) (no threads).
#[macro_export]
macro_rules! main_thread_em_asm_ptr {
    ($($t:tt)*) => {
        $crate::em_asm_ptr!($($t)*)
    };
}
