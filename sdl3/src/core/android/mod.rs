// Rust translation of src/core/android/ from Simple DirectMedia Layer,
// with the Java of android-project/app/src/main/java/org/libsdl/app/.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! The Android platform layer: upstream's JNI hub (`SDL_android.c`,
//! [`sdl_android`]) and its Java (`SDLActivity.java` and the other classes
//! of android-project, [`java`]), both Rust, over the JNI ABI declared by
//! hand ([`jni`]) and wrapped in [`env`].
//!
//! Upstream's Java runs on the Java side of the app and calls the C side's
//! `native` methods, which call back its static methods through JNI. Here
//! the Java is translated too, so those calls are Rust calls; JNI is left
//! between the translation and the framework (`android.*`). The framework
//! still needs upstream's classes (the manifest's activity, its views and
//! listeners) in the app's DEX file: [`classes`] declares them, with their
//! `native` methods (the framework's callbacks and upstream's natives),
//! `dex` writes them as DEX, kept generated in `classes_dex` (in its own
//! section of the library, `.sdl3_android_dex`, from where
//! `tools/android_run.py` packages it), and `JNI_OnLoad()` registers the
//! Rust functions.
//!
//! An app's code is its `libmain.so` (with SDL in it), whose `SDL_main`
//! upstream's `nativeRunMain()` calls on the "SDLThread" thread, with the
//! arguments `app_process` and those of `getArguments()` (none).

#[cfg(any(target_os = "android", test))]
pub(crate) mod classes;
#[cfg(any(target_os = "android", test))]
mod classes_dex;
#[cfg(test)]
mod dex;
#[cfg(target_os = "android")]
pub(crate) mod env;
#[cfg(target_os = "android")]
pub(crate) mod java;
#[cfg(any(target_os = "android", test))]
pub(crate) mod jni;
#[cfg(target_os = "android")]
pub(crate) mod ndk;
#[cfg(target_os = "android")]
pub(crate) mod sdl_android;
#[cfg(target_os = "android")]
pub(crate) mod tests_in_app;

#[cfg(target_os = "android")]
pub(crate) use sdl_android::*;
