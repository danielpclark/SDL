// Tests of em_js!, em_js_dollar!, the em_asm macros and the dependency table.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! Under node. This program is linked with `-sEXPORTED_FUNCTIONS=_main`
//! (build.rs), as an application with its own export list is.

#![cfg(target_os = "emscripten")]

use sdl3_emscripten::testing::*;
use sdl3_emscripten::{
    em_asm, em_asm_double, em_asm_int, em_asm_ptr, main_thread_em_asm, main_thread_em_asm_double,
    main_thread_em_asm_int, main_thread_em_asm_ptr, DEPS,
};
use std::ffi::c_void;

#[test]
fn em_js_numbers() {
    unsafe {
        assert_eq!(sdl3_test_i32(6, 7), 41);
        assert_eq!(sdl3_test_i32(i32::MIN, 1), i32::MAX);
        assert_eq!(sdl3_test_i64(1 << 40), (1 << 40) * 3 + 1);
        assert_eq!(sdl3_test_i64(-5), -14);
        assert_eq!(sdl3_test_i64(i64::MAX / 3), (i64::MAX / 3) * 3 + 1);
        assert_eq!(sdl3_test_f64(3.0, 0.25), 1.0);
        assert_eq!(sdl3_test_f64(f64::INFINITY, 0.0), f64::INFINITY);
        assert!(sdl3_test_f64(f64::NAN, 1.0).is_nan());
    }
}

#[test]
fn em_js_pointers() {
    let mut buf = [0xffu8; 8];
    let end = unsafe { sdl3_test_ptr(buf.as_mut_ptr(), 5) };
    assert_eq!(end, buf.as_mut_ptr().wrapping_add(5));
    assert_eq!(buf, [0, 1, 2, 3, 4, 0xff, 0xff, 0xff]);
}

#[test]
fn em_js_text_reaches_javascript_verbatim() {
    let mut buf = [0u8; 256];
    let n = unsafe { sdl3_test_text(buf.as_mut_ptr(), buf.len() as i32) };
    assert_eq!(std::str::from_utf8(&buf[..n as usize]), Ok(TEST_TEXT));
}

#[test]
fn em_js_dollar_parameters() {
    let byte = 9u8;
    assert_eq!(unsafe { sdl3_test_dollar(3, 1.5, &byte) }, 13.5);
}

#[test]
fn em_asm_int_double_ptr() {
    unsafe {
        // no arguments, comments, a trailing // comment
        assert_eq!(em_asm_int!("return 42; // the answer"), 42);
        // i32 arguments and the smaller integers, sign-extended like C's
        assert_eq!(
            em_asm_int!(
                r#"
                // a comment with 'quotes' and a \ backslash
                return $0 * $1 + $2 + $3;
            "#,
                6i32,
                7u8,
                -2i8,
                true
            ),
            41
        );
        // u32 arrives signed, as C's 'i' does
        assert_eq!(em_asm_int!("return $0;", u32::MAX), -1);
        // i64: a BigInt; the next argument after it (8-aligned) is read right
        let big: i64 = 0x1234_5678_9abc_def0;
        assert_eq!(
            em_asm_int!(
                r#"if (typeof $0 !== 'bigint') return -1; return Number($0 & 0xffffn) + $1;"#,
                big,
                1i32
            ),
            0xdef1
        );
        // doubles, an f32 (promoted), an i32 after a double
        assert_eq!(
            em_asm_double!("return $0 * 2 + $1 + $2;", 1.25f64, 0.5f32, 3i32),
            6.0
        );
        // pointers: read through HEAPU8, returned as a pointer
        let data = [10u8, 20, 30, 40];
        let p = em_asm_ptr!(
            "return HEAPU8[$0] + HEAPU8[$0 + 3] == 50 ? $0 + 2 : 0;",
            data.as_ptr()
        );
        assert_eq!(p as *const u8, data.as_ptr().wrapping_add(2));
        // usize is a 32-bit 'i'
        assert_eq!(em_asm_int!("return $0 == -1 ? 1 : 0;", usize::MAX), 1);
        // a null pointer is 0
        assert_eq!(
            em_asm_int!("return $0 === 0 ? 7 : 0;", std::ptr::null::<c_void>()),
            7
        );
        // the text, unchanged: backslashes, regular expressions, newlines
        let mut buf = [0u8; 64];
        let n = em_asm_int!(
            r#"
            var s = "a\\b" + /\d+/.exec("x123y")[0] + 'tab\tend\n';
            stringToUTF8(s, $0, $1);
            return lengthBytesUTF8(s);
        "#,
            buf.as_mut_ptr(),
            buf.len()
        );
        assert_eq!(&buf[..n as usize], b"a\\b123tab\tend\n");
    }
}

#[test]
fn em_asm_runs_each_time() {
    // the compiled snippet is kept by address; its state is JavaScript's
    unsafe {
        em_asm!("Module['sdl3_em_asm_test'] = 0;");
        for _ in 0..3 {
            em_asm!("Module['sdl3_em_asm_test'] += $0;", 2i32);
        }
        assert_eq!(em_asm_int!("return Module['sdl3_em_asm_test'];"), 6);
    }
}

#[test]
fn main_thread_forms_are_the_same() {
    unsafe {
        main_thread_em_asm!("Module['sdl3_mt_test'] = $0;", 5i32);
        assert_eq!(main_thread_em_asm_int!("return Module['sdl3_mt_test'];"), 5);
        assert_eq!(main_thread_em_asm_double!("return $0 / 2;", 5i32), 2.5);
        let x = 1u8;
        let p = &x as *const u8;
        assert_eq!(main_thread_em_asm_ptr!("return $0;", p) as *const u8, p);
    }
}

#[test]
fn em_asm_sees_the_glue() {
    unsafe {
        assert_eq!(
            em_asm_int!(
                "return typeof Module == 'object' && typeof HEAPU8 == 'object' && typeof UTF8ToString == 'function' ? 1 : 0;"
            ),
            1
        );
    }
}

/// The table's library functions are in the glue. The em_asm text is
/// compiled at run time, where emscripten can't have seen what it uses: they
/// are there because of the table. (lengthBytesUTF8 comes with
/// stringToNewUTF8.)
#[test]
fn dependency_table() {
    assert!(DEPS.split(',').all(|d| d.starts_with('$')));
    for name in DEPS.split(',').map(|d| &d[1..]) {
        let js = std::ffi::CString::new(name).unwrap();
        let found = unsafe {
            em_asm_int!(
                "return eval('typeof ' + UTF8ToString($0)) != 'undefined' ? 1 : 0;",
                js.as_ptr()
            )
        };
        assert_eq!(found, 1, "{name} isn't in the program");
    }
    unsafe {
        // dynCall on a Rust function pointer
        extern "C" fn triple(x: i32) -> i32 {
            x * 3
        }
        let f = triple as extern "C" fn(i32) -> i32;
        assert_eq!(
            em_asm_int!("return dynCall('ii', $0, [$1]);", f as *const c_void, 14i32),
            42
        );
        assert_eq!(
            em_asm_int!(
                "return getWasmTableEntry($0)($1);",
                f as *const c_void,
                5i32
            ),
            15
        );
        // stringToUTF8, stringToNewUTF8 (which mallocs), writeArrayToMemory, setValue
        let mut buf = [0u8; 8];
        em_asm!("stringToUTF8('hi', $0, $1);", buf.as_mut_ptr(), buf.len());
        assert_eq!(&buf[..3], b"hi\0");
        em_asm!(
            "writeArrayToMemory([1, 2, 3], $0); setValue($0 + 3, 4, 'i8');",
            buf.as_mut_ptr()
        );
        assert_eq!(&buf[..4], &[1, 2, 3, 4]);
        let p = em_asm_ptr!("return stringToNewUTF8('new');") as *mut u8;
        assert!(!p.is_null());
        assert_eq!(std::ffi::CStr::from_ptr(p.cast()).to_bytes(), b"new");
        // that memory is malloc's (emscripten's _malloc); free() it as C does
        extern "C" {
            fn free(p: *mut c_void);
        }
        free(p.cast());
        // and a library function nobody asked for isn't there: the table,
        // not everything, is what's included
        assert_eq!(
            em_asm_int!("return typeof stringToUTF16 == 'undefined' ? 1 : 0;"),
            1
        );
    }
}

/// The two symbols of the table are where emscripten reads it from: the
/// list, then the byte after it.
#[test]
fn dependency_table_symbols() {
    let (start, stop) = sdl3_emscripten::__private::KEEP.table();
    assert_eq!(stop as usize - start as usize, DEPS.len() + 1);
    let text = unsafe { std::slice::from_raw_parts(start, DEPS.len() + 1) };
    assert_eq!(&text[..DEPS.len()], DEPS.as_bytes());
    assert_eq!(text[DEPS.len()], 0);
}
