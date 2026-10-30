// The Android emulator runner's spawned programs: android_runner.rs's twin.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! android_runner.rs's checks for a program `tools/android_run.py` spawns
//! (a process of the shell user's, with no `.sdl3_runner` section): its
//! arguments are `main`'s, its variables the command line's, its exit
//! status its own (`SDL3_ANDROID_RUNNER_EXIT`'s, when the checks pass), and
//! it has no Java VM: the JNI layer's checks say so, and SDL's environment
//! works without the manifest's. With `SDL3_ANDROID_RUNNER_CHECK` set, the
//! arguments must be that value, `second` and an empty one, as for
//! android_runner.rs. Elsewhere it does nothing.

#[cfg(not(target_os = "android"))]
fn main() {}

#[cfg(target_os = "android")]
mod android_support;

#[cfg(target_os = "android")]
fn main() -> std::process::ExitCode {
    android_support::start();
    println!("android_spawned: started");
    let status = android_support::checked(|| {
        assert!(!android_support::is_app());
        let argv0 = std::env::args().next().unwrap();
        assert!(argv0.contains("android_spawned"), "{argv0}");
        assert_ne!(android_support::os_thread_name(), "SDLThread");

        let assert = std::env::var("SDL_ASSERT").expect("SDL_ASSERT from the command line");
        assert!(!assert.is_empty());
        assert_eq!(
            sdl3::stdlib::getenv("SDL_ASSERT").as_deref(),
            Some(assert.as_str())
        );
        println!("SDL_ASSERT={assert}");

        let args = android_support::args();
        println!("arguments: {args:?}");
        if let Ok(check) = std::env::var("SDL3_ANDROID_RUNNER_CHECK") {
            assert_eq!(args, [check.as_str(), "second", ""]);
        }

        // No Java VM in a spawned program.
        let checks = sdl3::__android_jni_checks();
        assert_eq!(checks.len(), 1, "{checks:?}");
        assert_eq!(checks[0].0, "java_vm");
        assert!(checks[0].1.is_err(), "{checks:?}");
        println!("JNI: {:?}", checks[0].1);

        eprintln!("android_spawned: stderr");
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
