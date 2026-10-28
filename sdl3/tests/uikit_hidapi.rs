// Tests of HIDAPI's iOS backend (CoreBluetooth's Steam Controllers) without
// Bluetooth: SDL_JOYSTICK_HIDAPI_STEAM unset.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! A program of its own (`harness = false`), run as an app on the iOS
//! Simulator by `tools/ios_run.py` (the `__DATA,__sdl3_runner` section
//! below), as `uikit_hidapi_ble` is with Bluetooth: the backend's manager
//! reads the hint once, when it is made. Its main function, shared with
//! `uikit_hidapi_ble`, is in `uikit_support/hidapi.rs`, with what it traces
//! and upstream's traces. Elsewhere this program does nothing.

#[cfg(not(target_os = "ios"))]
fn main() {}

/// The runner's word: run as an app.
#[cfg(target_os = "ios")]
#[used]
#[link_section = "__DATA,__sdl3_runner"]
static RUNNER: [u8; 4] = *b"app\0";

#[cfg(target_os = "ios")]
mod uikit_support;

#[cfg(target_os = "ios")]
#[path = "uikit_support/hidapi.rs"]
mod hidapi;

#[cfg(target_os = "ios")]
fn main() {
    hidapi::main(false)
}
