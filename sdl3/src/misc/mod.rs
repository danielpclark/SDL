// Rust translation of src/misc/SDL_url.c, SDL_sysurl.h,
// src/misc/unix/SDL_sysurl.c, src/misc/windows/SDL_sysurl.c,
// src/misc/macos/SDL_sysurl.m, src/misc/emscripten/SDL_sysurl.c,
// src/misc/dummy/SDL_sysurl.c and include/SDL3/SDL_misc.h from Simple DirectMedia Layer.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! Opening URLs in the user's preferred application.
//!
//! The Unix backend prefers the D-Bus OpenURI portal and falls back to
//! `xdg-open` (on Unix systems other than Apple's, Android and Haiku); the
//! Windows backend hands the URL to `ShellExecuteW()`, and the macOS one to
//! LaunchServices (`LSOpenCFURLRef()`), and the Emscripten one opens it in a
//! new browser tab (`window.open()`); elsewhere the dummy backend reports
//! that the operation is unsupported until the platform layer arrives.
//! Under Wayland, an activation token is passed along so that the browser
//! may take focus.

use crate::error::Result;

/// Open a URL in a separate, system-provided application. A successful
/// result does not mean the URL loaded, just that a launch was attempted.
/// Translation of `SDL_OpenURL()`.
pub fn open_url(url: &str) -> Result<()> {
    sys_open_url(url)
}

#[cfg(all(
    unix,
    not(any(
        target_vendor = "apple",
        target_os = "android",
        target_os = "haiku",
        target_os = "emscripten"
    ))
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
    not(any(
        target_vendor = "apple",
        target_os = "android",
        target_os = "haiku",
        target_os = "emscripten"
    ))
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

/// Translation of `SDL_SYS_OpenURL()` (macOS): `LSOpenCFURLRef()`.
#[cfg(target_os = "macos")]
fn sys_open_url(url: &str) -> Result<()> {
    use crate::core::apple::foundation::{
        CFRelease, CFURLCreateWithBytes, CFURLRef, K_CF_STRING_ENCODING_UTF8,
    };
    use crate::core::apple::objc::autoreleasepool;

    /// `OSStatus`.
    type OSStatus = i32;
    /// `noErr`.
    const NO_ERR: OSStatus = 0;

    #[link(name = "CoreServices", kind = "framework")]
    unsafe extern "C" {
        fn LSOpenCFURLRef(in_url: CFURLRef, out_launched_url: *mut CFURLRef) -> OSStatus;
    }

    autoreleasepool(|| {
        // SAFETY: the URL's bytes and length; LaunchServices takes a valid
        // URL (or NULL, which it refuses with paramErr).
        let status = unsafe {
            let cfurl = CFURLCreateWithBytes(
                std::ptr::null(),
                url.as_ptr(),
                url.len() as isize,
                K_CF_STRING_ENCODING_UTF8,
                std::ptr::null(),
            );
            let status = LSOpenCFURLRef(cfurl, std::ptr::null_mut());
            // Note (upstream): CFRelease(NULL) crashes where the URL
            // couldn't be made (bytes that aren't UTF-8, which a &str can't
            // hold): released only when there is one.
            if !cfurl.is_null() {
                CFRelease(cfurl);
            }
            status
        };
        if status != NO_ERR {
            return Err(crate::error::Error::new(format!(
                "LSOpenCFURLRef() failed: {status}"
            )));
        }
        Ok(())
    })
}

/// Translation of `SDL_SYS_OpenURL()` (Emscripten): `window.open(url,
/// "_blank")`, which a browser may block as a popup without telling: it
/// always succeeds. Under node, which has no `window`, the JavaScript
/// throws a ReferenceError that ends the program, as upstream's does.
#[cfg(target_os = "emscripten")]
fn sys_open_url(url: &str) -> Result<()> {
    // (as a C string, up to its first NUL)
    let url = url.split('\0').next().unwrap_or_default();
    let url = std::ffi::CString::new(url).unwrap_or_default();
    // SAFETY: the URL is NUL-terminated.
    unsafe { sdl3_emscripten::url::sdl3_open_url(url.as_ptr()) };
    Ok(())
}

#[cfg(not(any(
    windows,
    target_os = "macos",
    target_os = "emscripten",
    all(
        unix,
        not(any(
            target_vendor = "apple",
            target_os = "android",
            target_os = "haiku",
            target_os = "emscripten"
        ))
    )
)))]
/// Translation of `SDL_SYS_OpenURL()` (dummy).
fn sys_open_url(_url: &str) -> Result<()> {
    Err(crate::error::Error::unsupported())
}

#[cfg(test)]
mod tests {
    /// A page's `window`, stubbed: `window.open()` records its arguments.
    /// (Without a `window` the JavaScript throws, which ends the program:
    /// `tests/emscripten_main.rs` checks that in a program of its own.)
    #[cfg(target_os = "emscripten")]
    #[test]
    fn emscripten_window_open() {
        use sdl3_emscripten::{em_asm, em_asm_int};
        let _l = crate::test_support::test_lock();
        // SAFETY: plain JavaScript.
        unsafe {
            em_asm!(
                r#"
                Module['sdl3_test_window'] = globalThis.window;
                Module['sdl3_test_opened'] = [];
                globalThis.window = { open: (url, target) => { Module['sdl3_test_opened'].push(JSON.stringify([url, target])); return null; } };
            "#
            )
        };
        super::open_url("https://libsdl.org/?q=\u{e9}").unwrap();
        super::open_url("").unwrap();
        super::open_url("a\0b").unwrap();
        let mut buf = [0u8; 256];
        // SAFETY: plain JavaScript, writing at most the buffer's length.
        let len = unsafe {
            let len = em_asm_int!(
                r#"
                var opened = Module['sdl3_test_opened'].join('\n');
                globalThis.window = Module['sdl3_test_window'];
                if (window === undefined) delete globalThis.window;
                delete Module['sdl3_test_window'];
                delete Module['sdl3_test_opened'];
                return stringToUTF8(opened, $0, $1);
            "#,
                buf.as_mut_ptr(),
                buf.len() as i32
            );
            len as usize
        };
        assert_eq!(
            std::str::from_utf8(&buf[..len]).unwrap(),
            "[\"https://libsdl.org/?q=\u{e9}\",\"_blank\"]\n[\"\",\"_blank\"]\n[\"a\",\"_blank\"]"
        );
    }

    /// LaunchServices' answers for URLs no application opens (nothing is
    /// launched): kLSApplicationNotFoundErr, and paramErr for an empty one.
    #[cfg(target_os = "macos")]
    #[test]
    fn macos_open_url_errors() {
        assert_eq!(
            super::open_url("sdl-no-such-scheme-for-tests://x")
                .unwrap_err()
                .message(),
            "LSOpenCFURLRef() failed: -10814"
        );
        assert_eq!(
            super::open_url("").unwrap_err().message(),
            "LSOpenCFURLRef() failed: -50"
        );
    }
}
