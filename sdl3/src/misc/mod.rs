// Rust translation of src/misc/SDL_url.c, SDL_sysurl.h,
// src/misc/unix/SDL_sysurl.c, src/misc/dummy/SDL_sysurl.c and
// include/SDL3/SDL_misc.h from Simple DirectMedia Layer.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! Opening URLs in the user's preferred application.
//!
//! The Unix backend runs `xdg-open` (on Unix systems other than Apple's,
//! Android and Haiku); elsewhere the dummy backend reports that the
//! operation is unsupported until the platform layer arrives. The D-Bus
//! portal and Wayland activation tokens arrive with the platform layer.

use crate::error::Result;

/// Open a URL in a separate, system-provided application. A successful
/// result does not mean the URL loaded, just that a launch was attempted.
/// Translation of `SDL_OpenURL()`.
pub fn open_url(url: &str) -> Result<()> {
    sys_open_url(url)
}

#[cfg(all(
    unix,
    not(any(target_vendor = "apple", target_os = "android", target_os = "haiku"))
))]
/// Translation of `SDL_SYS_OpenURL()` (Unix).
fn sys_open_url(url: &str) -> Result<()> {
    use crate::process::ProcessBuilder;
    use crate::stdlib::Environment;

    // (The D-Bus portal is preferred, if available, and Wayland requires an
    // activation token for the browser to take focus: both arrive with the
    // platform layer.)

    let env = Environment::new(true);

    // Clear LD_PRELOAD so Chrome opens correctly when this application is launched by Steam
    env.unset("LD_PRELOAD")?;

    let _process = ProcessBuilder::new(["xdg-open", url])
        .environment(env)
        .background(true)
        .spawn()?;

    Ok(())
}

#[cfg(not(all(
    unix,
    not(any(target_vendor = "apple", target_os = "android", target_os = "haiku"))
)))]
/// Translation of `SDL_SYS_OpenURL()` (dummy).
fn sys_open_url(_url: &str) -> Result<()> {
    Err(crate::error::Error::unsupported())
}
