// Rust translation of src/core/android/ from Simple DirectMedia Layer,
// with the Java of android-project/app/src/main/java/org/libsdl/app/.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! The Android platform layer: upstream's JNI hub (`SDL_android.c`) and
//! its Java (`SDLActivity.java` and the other classes of android-project),
//! both Rust, over the JNI ABI declared by hand ([`jni`]) and wrapped in
//! [`env`]. So far the JNI layer.

#[cfg(target_os = "android")]
pub(crate) mod env;
#[cfg(any(target_os = "android", test))]
pub(crate) mod jni;
#[cfg(target_os = "android")]
pub(crate) mod ndk;
