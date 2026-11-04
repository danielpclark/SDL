// Tests of the OpenXR loader's loading and reference count.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! The loader's reference count, its errors and the hint, as a trace of
//! `xrtrace:` lines (printed, and compared with the expected ones here),
//! which a C twin on upstream's SDL prints the same way on the CI's OSes
//! (in the diag branches' reference workflow). Without an OpenXR loader on
//! the machine (the CI's runners) the loads fail with upstream's message;
//! with one the trace is the other expected one. The hint's library that
//! isn't a loader (the C library) is loaded all the same, as upstream's
//! (see `openxrdyn::get_sym`), and has no `xrGetInstanceProcAddr`.

use super::openxrdyn::{self, OPENXR_LIBRARY_NAMES};
use crate::hints;
use crate::loadso::SharedObject;

/// A library that every process of the target can load and that isn't an
/// OpenXR loader.
#[cfg(target_os = "linux")]
const NOT_A_LOADER: Option<&str> = Some("libm.so.6");
#[cfg(target_os = "android")]
const NOT_A_LOADER: Option<&str> = Some("libm.so");
#[cfg(windows)]
const NOT_A_LOADER: Option<&str> = Some("kernel32.dll");
#[cfg(any(target_os = "macos", target_os = "ios", target_os = "tvos"))]
const NOT_A_LOADER: Option<&str> = Some("/usr/lib/libSystem.B.dylib");
// (Emscripten loads no libraries; on visionOS every load is refused: see
// SharedObject::load)
#[cfg(not(any(
    target_os = "linux",
    target_os = "android",
    windows,
    target_os = "macos",
    target_os = "ios",
    target_os = "tvos"
)))]
const NOT_A_LOADER: Option<&str> = None;

/// A path that doesn't exist.
const BAD_PATH: &str = "/nonexistent/sdl3-test/libopenxr_loader.so";

const LOAD_FAILED: &str = "Failed to load OpenXR loader library. On Windows, ensure openxr_loader.dll is in your application directory or PATH. On Linux, install the OpenXR loader package (libopenxr-loader) or set LD_LIBRARY_PATH. You can also use the SDL_HINT_OPENXR_LIBRARY hint to specify the loader path.";
const NOT_LOADED: &str = "The OpenXR loader has not been loaded";
#[cfg_attr(not(target_os = "android"), allow(dead_code))]
const NOT_THE_KHRONOS_LOADER: &str = "Failed to get xrGetInstanceProcAddr from OpenXR loader. Make sure you're using the Khronos loader (libopenxr_loader.so), not Meta's forwardloader.";

/// The trace's steps (the C twin makes the same calls).
fn trace() -> Vec<String> {
    let mut lines = Vec::new();
    let mut line = |s: String| {
        println!("xrtrace: {s}");
        lines.push(s);
    };
    let load = |line: &mut dyn FnMut(String)| {
        line(match super::openxr_load_library() {
            Ok(()) => "load: ok".to_owned(),
            Err(e) => format!("load: failed: {}", e.message()),
        })
    };
    let get = |line: &mut dyn FnMut(String)| {
        line(match super::openxr_get_xr_get_instance_proc_addr() {
            Ok(_) => "get: ok".to_owned(),
            Err(e) => format!("get: {}", e.message()),
        })
    };
    let unload = |line: &mut dyn FnMut(String)| {
        super::openxr_unload_library();
        line("unload".to_owned());
    };
    let hint = |line: &mut dyn FnMut(String), value: Option<&str>| {
        match value {
            Some(v) => assert!(hints::set(hints::OPENXR_LIBRARY, v).unwrap()),
            None => {
                hints::reset(hints::OPENXR_LIBRARY);
            }
        }
        line(format!("hint {}", value.unwrap_or("(none)")));
    };

    hint(&mut line, None);
    get(&mut line);
    // (nothing loaded: nothing to release)
    unload(&mut line);
    get(&mut line);
    load(&mut line);
    get(&mut line);
    // (loaded: the hint isn't read again; not loaded: it's tried first,
    // then the default name)
    hint(&mut line, Some(BAD_PATH));
    load(&mut line);
    get(&mut line);
    unload(&mut line);
    get(&mut line);
    unload(&mut line);
    get(&mut line);
    unload(&mut line);
    if let Some(lib) = NOT_A_LOADER {
        hint(&mut line, Some(lib));
        load(&mut line);
        get(&mut line);
        hint(&mut line, Some(BAD_PATH));
        // (the reference count: the hint isn't read again)
        load(&mut line);
        get(&mut line);
        unload(&mut line);
        unload(&mut line);
        // (unloaded: the bad hint, then the default name)
        load(&mut line);
        get(&mut line);
        unload(&mut line);
    }
    hint(&mut line, None);
    lines
}

/// What the trace is without a loader on the machine (`loader` false) or
/// with one.
fn expected(loader: bool) -> Vec<String> {
    let (load, get) = if loader {
        ("load: ok".to_owned(), "get: ok".to_owned())
    } else {
        (
            format!("load: failed: {LOAD_FAILED}"),
            format!("get: {NOT_LOADED}"),
        )
    };
    let not_loaded = format!("get: {NOT_LOADED}");
    let mut lines = vec![
        "hint (none)".to_owned(),
        not_loaded.clone(),
        "unload".to_owned(),
        not_loaded.clone(),
        load.clone(),
        get.clone(),
        format!("hint {BAD_PATH}"),
        load.clone(),
        get.clone(),
        "unload".to_owned(),
        // (two loads with a loader: still loaded)
        if loader {
            get.clone()
        } else {
            not_loaded.clone()
        },
        "unload".to_owned(),
        not_loaded.clone(),
        "unload".to_owned(),
    ];
    if let Some(lib) = NOT_A_LOADER {
        // (Android's loader initialization fails without the function, and
        // the load with it: the second load is a first one)
        let (lib_load, second_load) = if cfg!(target_os = "android") {
            (
                format!("load: failed: {NOT_THE_KHRONOS_LOADER}"),
                load.clone(),
            )
        } else {
            ("load: ok".to_owned(), "load: ok".to_owned())
        };
        lines.extend([
            format!("hint {lib}"),
            lib_load,
            not_loaded.clone(),
            format!("hint {BAD_PATH}"),
            second_load,
            not_loaded.clone(),
            "unload".to_owned(),
            "unload".to_owned(),
            load.clone(),
            get.clone(),
            "unload".to_owned(),
        ]);
    }
    lines.push("hint (none)".to_owned());
    lines
}

#[test]
fn loader_refcount_errors_and_hint() {
    let _l = crate::test_support::test_lock();
    let loader = OPENXR_LIBRARY_NAMES
        .iter()
        .any(|name| SharedObject::load(name).is_ok());
    println!("xrtrace: (an OpenXR loader on the machine: {loader})");
    assert_eq!(openxrdyn::refcount(), 0);
    assert_eq!(trace(), expected(loader));
    assert_eq!(openxrdyn::refcount(), 0);
}
