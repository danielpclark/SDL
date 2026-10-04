// Rust translation of src/misc/SDL_url.c, SDL_sysurl.h,
// src/misc/unix/SDL_sysurl.c, src/misc/dummy/SDL_sysurl.c and
// include/SDL3/SDL_misc.h from Simple DirectMedia Layer.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! Opening URLs in the user's preferred application.
//!
//! The Unix backend prefers the D-Bus OpenURI portal and falls back to
//! `xdg-open` (on Unix systems other than Apple's, Android and Haiku);
//! elsewhere the dummy backend reports that the operation is unsupported
//! until the platform layer arrives. Under Wayland, an activation token is
//! passed along so that the browser may take focus.

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

    let (activation_token, window_id) = get_activation_token();

    // Prefer the D-Bus portal, if available.
    if crate::core::linux::dbus::open_uri(url, window_id.as_deref(), activation_token.as_deref()) {
        return Ok(());
    }

    let env = Environment::new(true);

    // Clear LD_PRELOAD so Chrome opens correctly when this application is launched by Steam
    env.unset("LD_PRELOAD")?;
    if let Some(activation_token) = &activation_token {
        env.set("XDG_ACTIVATION_TOKEN", activation_token, false)?;
    }

    let _process = ProcessBuilder::new(["xdg-open", url])
        .environment(env)
        .background(true)
        .spawn()?;

    Ok(())
}

/// Wayland requires an activation token for the browser to take focus: the
/// token and the window ID. Translation of `GetActivationToken()`.
#[cfg(all(
    unix,
    not(any(target_vendor = "apple", target_os = "android", target_os = "haiku"))
))]
fn get_activation_token() -> (Option<String>, Option<String>) {
    crate::video::drivers::wayland::util::get_activation_token()
}

#[cfg(not(all(
    unix,
    not(any(target_vendor = "apple", target_os = "android", target_os = "haiku"))
)))]
/// Translation of `SDL_SYS_OpenURL()` (dummy).
fn sys_open_url(_url: &str) -> Result<()> {
    Err(crate::error::Error::unsupported())
}
