// Rust translation of src/misc/SDL_url.c, SDL_sysurl.h,
// src/misc/unix/SDL_sysurl.c, src/misc/windows/SDL_sysurl.c,
// src/misc/dummy/SDL_sysurl.c and include/SDL3/SDL_misc.h from Simple
// DirectMedia Layer.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! Opening URLs in the user's preferred application.
//!
//! The Unix backend prefers the D-Bus OpenURI portal and falls back to
//! `xdg-open` (on Unix systems other than Apple's, Android and Haiku); the
//! Windows backend hands the URL to `ShellExecuteW()`; elsewhere the dummy
//! backend reports that the operation is unsupported until the platform
//! layer arrives. Under Wayland, an activation token is
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

/// Translation of `SDL_SYS_OpenURL()` (Windows).
///
/// See <https://msdn.microsoft.com/en-us/library/windows/desktop/bb762153%28v=vs.85%29.aspx>
#[cfg(windows)]
fn sys_open_url(url: &str) -> Result<()> {
    use crate::core::windows::{
        co_initialize, co_uninitialize, error_from_hresult, set_error, utf8_to_wide,
    };
    use windows_sys::Win32::UI::Shell::ShellExecuteW;
    use windows_sys::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;

    // MSDN says for safety's sake, make sure COM is initialized.
    let hr = co_initialize();
    if hr < 0 {
        return Err(error_from_hresult(Some("CoInitialize failed"), hr));
    }

    let wurl = utf8_to_wide(url);
    let open = utf8_to_wide("open");

    // Success returns value greater than 32. Less is an error.
    // SAFETY: both strings are NUL-terminated; the other pointers may be NULL.
    let rc = unsafe {
        ShellExecuteW(
            std::ptr::null_mut(),
            open.as_ptr(),
            wurl.as_ptr(),
            std::ptr::null(),
            std::ptr::null(),
            SW_SHOWNORMAL,
        )
    };
    co_uninitialize();
    if rc as usize <= 32 {
        return Err(set_error("Couldn't open given URL."));
    }
    Ok(())
}

#[cfg(not(any(
    windows,
    all(
        unix,
        not(any(target_vendor = "apple", target_os = "android", target_os = "haiku"))
    )
)))]
/// Translation of `SDL_SYS_OpenURL()` (dummy).
fn sys_open_url(_url: &str) -> Result<()> {
    Err(crate::error::Error::unsupported())
}
