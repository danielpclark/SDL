// The Android JNI layer's checks, in an app.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! An app (as android_runner.rs is) that runs the checks of sdl3's JNI
//! layer, which need its Java VM (a spawned program has none): the
//! environment of each thread, local and global references, local frames,
//! exceptions, strings, fields, arrays, nonvirtual calls, class lookup on
//! native threads, the registered natives, the DEX file through an
//! `InMemoryDexClassLoader`, the manifest's environment, the shims of
//! upstream's anonymous classes, the activity's command handler and the
//! lifecycle event queue. Elsewhere it does nothing.

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
    let status = android_support::checked(|| {
        let checks = sdl3::__android_jni_checks();
        let mut failed = Vec::new();
        for (name, result) in &checks {
            match result {
                Ok(()) => println!("android_jni: {name} ... ok"),
                Err(e) => {
                    println!("android_jni: {name} ... FAILED: {e}");
                    failed.push(*name);
                }
            }
        }
        println!(
            "android_jni: {} passed, {} failed",
            checks.len() - failed.len(),
            failed.len()
        );
        assert!(failed.is_empty(), "failed: {failed:?}");
    });
    android_support::finish(status)
}
