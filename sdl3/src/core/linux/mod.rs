// Rust translation of src/core/linux/ from Simple DirectMedia Layer.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! Linux (and other free Unix) glue shared by the backends: D-Bus, thread
//! priorities, the kernel's input interfaces, evdev device classification
//! and timestamps, and libudev.
//!
//! Not translated yet: `SDL_ibus.c`, `SDL_fcitx.c`, `SDL_ime.c`,
//! `SDL_system_theme.c`, `SDL_progressbar.c`, and the console
//! keyboard/mouse/touch reader of `SDL_evdev.c` and `SDL_evdev_kbd.c`, which
//! come with the KMS/DRM video driver.

pub(crate) mod dbus;
#[cfg(target_os = "linux")]
pub(crate) mod evdev;
#[cfg(target_os = "linux")]
pub(crate) mod evdev_capabilities;
#[cfg(all(test, target_os = "linux"))]
pub(crate) mod guess_tests;
#[cfg(target_os = "linux")]
pub(crate) mod input;
#[cfg(target_os = "linux")]
pub(crate) mod threadprio;
#[cfg(target_os = "linux")]
pub(crate) mod udev;
