// The iOS Simulator runner's app mode.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! A program (`harness = false`) that asks `tools/ios_run.py` to run it as
//! an app (the `__DATA,__sdl3_runner` section below), as the UIKit test
//! programs will be: in an app bundle, launched with `simctl launch`. It
//! checks what such a program sees (its bundle, the variables the runner
//! passes on, both its output streams) and ends as those programs do, with
//! the line that gives the runner its exit status. Elsewhere it does
//! nothing.

#[cfg(not(target_os = "ios"))]
fn main() {}

/// The runner's words: run as an app.
#[cfg(target_os = "ios")]
#[used]
#[link_section = "__DATA,__sdl3_runner"]
static RUNNER: [u8; 4] = *b"app\0";

#[cfg(target_os = "ios")]
mod uikit_support;

#[cfg(target_os = "ios")]
fn main() {
    println!("ios_runner: started");
    let checks = std::panic::catch_unwind(|| {
        // The executable is in an app bundle, NAME.app, which the base path
        // is (the bundle's resource directory, as upstream's on iOS).
        let exe = std::env::current_exe().unwrap();
        let bundle = exe.parent().unwrap();
        assert_eq!(
            bundle.extension().and_then(|e| e.to_str()),
            Some("app"),
            "{}",
            exe.display()
        );
        let base = sdl3::filesystem::get_base_path().unwrap();
        assert!(base.ends_with(".app/"), "{base}");
        println!("bundle: {}", bundle.file_name().unwrap().to_string_lossy());

        // The runner's variables: SDL_ASSERT (`abort` unless it was set).
        let assert = std::env::var("SDL_ASSERT").unwrap();
        assert!(!assert.is_empty());
        println!("SDL_ASSERT={assert}");
        println!(
            "arguments: {:?}",
            std::env::args().skip(1).collect::<Vec<_>>()
        );

        // stderr is copied too
        eprintln!("ios_runner: stderr");
    });
    let status = if checks.is_ok() { 0 } else { 101 };
    // The runner's exit status (simctl doesn't give an app's).
    uikit_support::exit(status);
}
