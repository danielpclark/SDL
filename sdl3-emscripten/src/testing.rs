// The em_js! functions this crate's tests call (feature `testing`).
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! What `tests/` needs defined in a library crate: [`em_js!`](crate::em_js)
//! functions (in the test programs themselves rustc wouldn't export their
//! text), a callback name, and a `#[no_mangle]` function. Built only with
//! the `testing` feature, which only this crate's tests turn on, so none of
//! it reaches a program that uses SDL.

use core::ffi::c_int;

crate::em_js!(
    /// `a * b - 1`, in JavaScript numbers.
    pub fn sdl3_test_i32(a: i32, b: i32) -> i32,
    r#"{ return a * b - 1; }"#
);

crate::em_js!(
    /// 64-bit integers are `BigInt`s: `a * 3 + 1`, or -1 when `a` isn't one.
    pub fn sdl3_test_i64(a: i64) -> i64,
    r#"{
    if (typeof a !== 'bigint') return -1n;
    return a * 3n + 1n;
}"#
);

crate::em_js!(
    /// `a / 4 + b` (`b` is an `f32`, a number like `a`).
    pub fn sdl3_test_f64(a: f64, b: f32) -> f64,
    r#"{ return a / 4 + b; }"#
);

crate::em_js!(
    /// Writes `0, 1, 2, …` into the `n` bytes at `p` (through `HEAPU8`) and
    /// returns the pointer past them.
    pub fn sdl3_test_ptr(p: *mut u8, n: usize) -> *mut u8,
    r#"{
    for (var i = 0; i < n; i++) {
        HEAPU8[p + i] = i;
    }
    return p + n;
}"#
);

/// What [`sdl3_test_text`] writes: what its JavaScript computes from string
/// literals, comments, backslashes, a regular expression and newlines.
pub const TEST_TEXT: &str =
    "back\\slash, \"quotes\" 'single' `tick`, regex 42.x, tab\tnewline\nend // not a comment";

crate::em_js!(
    /// Writes [`TEST_TEXT`] (made by JavaScript that has every kind of
    /// text that could be mangled on its way) into `buf` as UTF-8 and
    /// returns its length in bytes.
    pub fn sdl3_test_text(buf: *mut u8, len: i32) -> i32,
    r#"{
    // a line comment, with 'quotes', "double quotes", `back ticks` and a \ backslash
    /* a block comment
       over two lines */
    var s = "back\\slash, " + '"quotes" ' + "'single' " + `${'`'}tick${'`'}, `;
    s += "regex " + /(\d+)\.(\w)/.exec("ab42.xyz")[0] + ", ";
    s += 'tab\tnewline\nend' + " // not a comment";
    stringToUTF8(s, buf, len);
    return lengthBytesUTF8(s);
}"#
);

crate::em_js_dollar!(
    /// Upstream style: `$0 * $1 + HEAPU8[$2]`.
    pub fn sdl3_test_dollar(a: i32, b: f64, p: *const u8) -> f64,
    r#"{ return $0 * $1 + HEAPU8[$2]; }"#
);

crate::em_js!(
    /// Calls the callback `Module[name]`'s glue-scope forwarder,
    /// `_sdl3_test_callback(x, y)`.
    pub fn sdl3_test_call_callback(x: i32, y: f64) -> f64,
    r#"{ return _sdl3_test_callback(x, y); }"#
);

crate::em_js_callback!(
    /// A callback for the registry's tests.
    pub static TEST_CALLBACK = "_sdl3_test_callback";
);

/// A `#[no_mangle]` function, which rustc would list in
/// `-sEXPORTED_FUNCTIONS` (JavaScript's `_sdl3_emscripten_test_export`)
/// unless the program's own list replaces rustc's, as the tests' does.
#[no_mangle]
pub extern "C" fn sdl3_emscripten_test_export(x: c_int) -> c_int {
    x + 1
}
