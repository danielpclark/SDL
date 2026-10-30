// The Android emulator runner's app mode.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! A program (`harness = false`) that asks `tools/android_run.py` to run it
//! as an app (the `.sdl3_runner` section below): linked as an app's
//! libmain.so, packaged with sdl3's DEX file of upstream's classes,
//! launched as upstream's `SDLActivity`, whose SDL thread calls its `main`
//! through `SDL_main`. It checks what such a program sees (its process,
//! its thread, its arguments, the variables the runner passes on through
//! the manifest, both its output streams), asks the runner for something
//! (`foreground`), and ends as those programs do, with the line that gives
//! the runner its exit status: `SDL3_ANDROID_RUNNER_EXIT`'s, when the
//! checks pass. With `SDL3_ANDROID_RUNNER_CHECK` set, the runner must have
//! been given that value, `second` and an empty one as the arguments (CI
//! runs it so: the value comes through the manifest, the arguments their
//! own way). Its spawned twin is android_spawned.rs. Elsewhere it does
//! nothing.

#[cfg(not(target_os = "android"))]
fn main() {}

/// The runner's words: run as an app.
#[cfg(target_os = "android")]
#[used]
#[link_section = ".sdl3_runner"]
static RUNNER: [u8; 4] = *b"app\0";

#[cfg(target_os = "android")]
mod android_support;

#[cfg(target_os = "android")]
fn main() -> std::process::ExitCode {
    android_support::start();
    println!("android_runner: started");
    let status = android_support::checked(|| {
        assert!(
            android_support::is_app(),
            "no sockets of the runner's: not run as an app by tools/android_run.py"
        );

        // The app's process (named after its package), and main on the
        // activity's SDLThread, with upstream's arguments: "app_process",
        // then getArguments()'s, none.
        let process = android_support::process_name();
        let package = std::env::var("SDL3_ANDROID_PACKAGE")
            .unwrap_or_else(|_| "org.libsdl.sdl3_rust.android_runner".to_string());
        assert_eq!(process, package);
        assert_eq!(android_support::os_thread_name(), "SDLThread");
        let argv: Vec<String> = std::env::args().collect();
        assert_eq!(argv, ["app_process"]);
        println!("process: {process}, thread: SDLThread, argv: {argv:?}");

        // The runner's variables, set from the manifest: SDL_ASSERT
        // (`abort` unless it was set), in the process's environment and
        // SDL's.
        let assert = std::env::var("SDL_ASSERT").expect("SDL_ASSERT from the manifest");
        assert!(!assert.is_empty());
        assert_eq!(
            sdl3::stdlib::getenv("SDL_ASSERT").as_deref(),
            Some(assert.as_str())
        );
        println!("SDL_ASSERT={assert}");
        // (whether the app reads the test data the runner copied, which
        // a spawned program reads at CARGO_MANIFEST_DIR)
        let manifest_dir = std::env::var("CARGO_MANIFEST_DIR").unwrap_or_default();
        match std::fs::read_dir(&manifest_dir) {
            Ok(entries) => println!(
                "CARGO_MANIFEST_DIR {manifest_dir}: {} entries",
                entries.count()
            ),
            Err(e) => println!("CARGO_MANIFEST_DIR {manifest_dir}: {e}"),
        }

        // The runner's arguments.
        let args = android_support::args();
        println!("arguments: {args:?}");
        if let Ok(check) = std::env::var("SDL3_ANDROID_RUNNER_CHECK") {
            assert_eq!(args, [check.as_str(), "second", ""]);
        }

        // A request (the app is in the foreground already: the same
        // process, the activity's singleTop)
        android_support::request("foreground");

        // stderr is copied too
        eprintln!("android_runner: stderr");
    });
    let status = match status {
        0 => std::env::var("SDL3_ANDROID_RUNNER_EXIT")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(0),
        failed => failed,
    };
    android_support::finish(status)
}
