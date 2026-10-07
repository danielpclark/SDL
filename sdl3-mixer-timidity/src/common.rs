/*
    TiMidity -- Experimental MIDI to WAVE converter
    Copyright (C) 1995 Tuukka Toivonen <toivonen@clinet.fi>

    This program is free software; you can redistribute it and/or modify
    it under the terms of the Perl Artistic License, available in COPYING.

    common.c
*/
// Modified 2026-10-07: translated into Rust from common.c and common.h in SDL_mixer 3.3.0's TiMidity (see NOTICE).

//! Finding and opening files along the search path. Translation of
//! `common.c` (and `common.h`), with a few helpers the translation adds.

use std::sync::{Mutex, MutexGuard};

use sdl3::io::IoStream;

#[cfg(windows)]
const CHAR_DIRSEP: u8 = b'\\';
#[cfg(windows)]
fn is_dirsep(c: u8) -> bool {
    c == b'/' || c == b'\\'
}
#[cfg(windows)]
fn is_abspath(p: &[u8]) -> bool {
    p.first() == Some(&b'/')
        || p.first() == Some(&b'\\')
        || (!p.is_empty() && p.get(1) == Some(&b':'))
}

#[cfg(not(windows))] /* unix: */
const CHAR_DIRSEP: u8 = b'/';
#[cfg(not(windows))]
fn is_dirsep(c: u8) -> bool {
    c == b'/'
}
#[cfg(not(windows))]
fn is_abspath(p: &[u8]) -> bool {
    p.first() == Some(&b'/')
}

/* The paths in this list will be tried whenever we're reading a file */
// (`PathList`, a linked list with the newest path first: here a `Vec`
// with the newest path last.)
static PATHLIST: Mutex<Vec<Vec<u8>>> = Mutex::new(Vec::new());

fn pathlist() -> MutexGuard<'static, Vec<Vec<u8>>> {
    PATHLIST.lock().unwrap_or_else(|e| e.into_inner())
}

/// `SDL_IOFromFile(name, "rb")` on a C string's bytes.
fn io_from_file(name: &[u8]) -> Option<IoStream<'static>> {
    #[cfg(unix)]
    let path = {
        use std::os::unix::ffi::OsStrExt;
        std::path::PathBuf::from(std::ffi::OsStr::from_bytes(name))
    };
    #[cfg(not(unix))]
    let path = std::path::PathBuf::from(String::from_utf8_lossy(name).into_owned());
    IoStream::from_file(path, "rb").ok()
}

/* This is meant to find and open files for reading */
/// Translation of `timi_openfile()`.
pub(crate) fn timi_openfile(name: &[u8]) -> Option<IoStream<'static>> {
    if name.is_empty() {
        crate::snddbg!("Attempted to open nameless file.\n");
        return None;
    }

    /* First try the given name */

    crate::snddbg!("Trying to open {}\n", String::from_utf8_lossy(name));
    if let Some(io) = io_from_file(name) {
        return Some(io);
    }

    if !is_abspath(name) {
        const CURRENT_FILENAME_SIZE: usize = 1024;
        let plp = pathlist().clone();
        for path in plp.iter().rev() {
            /* Try along the path then */
            let mut current_filename = Vec::with_capacity(CURRENT_FILENAME_SIZE);
            let mut l = path.len();
            if l >= CURRENT_FILENAME_SIZE - 3 {
                l = 0;
            }
            if l != 0 {
                current_filename.extend_from_slice(&path[..l]);
                if !is_dirsep(path[l - 1]) {
                    current_filename.push(CHAR_DIRSEP);
                    l += 1;
                }
            }
            // SDL_strlcpy(p, name, sizeof(current_filename) - l)
            let room = CURRENT_FILENAME_SIZE - l - 1;
            current_filename.extend_from_slice(&name[..name.len().min(room)]);
            crate::snddbg!(
                "Trying to open {}\n",
                String::from_utf8_lossy(&current_filename)
            );
            if let Some(io) = io_from_file(&current_filename) {
                return Some(io);
            }
        }
    }

    /* Nothing could be opened. */
    crate::snddbg!("Could not open {}\n", String::from_utf8_lossy(name));
    None
}

/* This adds a directory to the path list */
/// Translation of `timi_add_pathlist()`.
pub(crate) fn timi_add_pathlist(s: &[u8]) -> i32 {
    let mut list = pathlist();
    if list.try_reserve(1).is_err() {
        return -2;
    }
    list.push(s.to_vec());
    0
}

/// Translation of `timi_free_pathlist()`.
pub(crate) fn timi_free_pathlist() {
    *pathlist() = Vec::new();
}

/* debug output */
/// `SNDDBG((...))`: logs only with `DEBUG_CHATTER` (which upstream
/// doesn't define, and neither does the translation).
#[macro_export]
#[doc(hidden)]
macro_rules! snddbg {
    ($($arg:tt)*) => {
        if false {
            let _ = format!($($arg)*);
        }
    };
}

// What the C's conversions of out-of-range floating point values to
// integers (undefined behavior) do on x86, where upstream is built and
// checked: `cvttsd2si`/`cvttss2si` give 0x80000000 for NaNs and values out
// of range (Rust's `as` saturates instead).

/// `(Sint32)x` for a `double` (or a `float` widened to one), as x86 does it.
#[inline]
pub(crate) fn c_f64_to_i32(x: f64) -> i32 {
    if x.is_nan() || x >= 2147483648.0 || x <= -2147483649.0 {
        i32::MIN
    } else {
        x as i32
    }
}

/// `(Sint32)x` for a `float`, as x86 does it.
#[inline]
pub(crate) fn c_f32_to_i32(x: f32) -> i32 {
    c_f64_to_i32(x as f64)
}

/// `SDL_atoi()`.
pub(crate) fn atoi(s: &[u8]) -> i32 {
    sdl3::stdlib::string::strtol(s, 10).0 as i32
}
