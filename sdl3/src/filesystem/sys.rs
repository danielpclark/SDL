// Rust translation of src/filesystem/unix/SDL_sysfilesystem.c and
// src/filesystem/windows/SDL_sysfilesystem.c from Simple DirectMedia Layer.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.
//
// The two functions prefixed with `xdg_` are adapted from xdg-user-dirs,
// Copyright (c) 2007 Red Hat, Inc., under the MIT license reproduced in
// LICENSE.txt.

//! The `SDL_SYS_*` path lookups.

use super::Folder;
use crate::error::{Error, Result};
#[cfg(unix)]
use crate::stdlib::getenv;

/// The separator SDL puts at the end of returned paths.
const SEP: char = if cfg!(windows) { '\\' } else { '/' };

/// Translation of `GetExePath()` (Unix) / `WIN_GetModulePath()` (Windows).
fn get_exe_path() -> Option<String> {
    // is a Linux-style /proc filesystem available?
    #[cfg(any(target_os = "linux", target_os = "android"))]
    if std::path::Path::new("/proc").exists() {
        /* !!! FIXME: after 2.0.6 ships, let's delete this code and just
        use the /proc/%llu version. There's no reason to have
        two copies of this plus all the #ifdefs. --ryan. */
        let result = std::fs::read_link("/proc/self/exe") // linux.
            .or_else(|_| {
                // older kernels don't have /proc/self ... try PID version...
                std::fs::read_link(format!("/proc/{}/exe", std::process::id()))
            });
        if let Ok(path) = result {
            return Some(path.to_string_lossy().into_owned());
        }
    }

    // Other systems: the platform query std performs (sysctl
    // KERN_PROC_PATHNAME, _NSGetExecutablePath, GetModuleFileNameW, ...).
    std::env::current_exe()
        .ok()
        .map(|p| p.to_string_lossy().into_owned())
}

/// The position of the last path separator in an executable path.
fn last_separator(path: &str) -> Option<usize> {
    if cfg!(windows) {
        path.rfind(['\\', '/'])
    } else {
        path.rfind('/')
    }
}

/// Translation of `SDL_SYS_GetBasePath()`.
pub(super) fn get_base_path() -> Result<String> {
    let path = get_exe_path().ok_or_else(|| Error::new("Couldn't locate our executable"))?;
    // Should have been an absolute path.
    let ptr = last_separator(&path).ok_or_else(|| Error::new("Couldn't locate our executable"))?;
    Ok(path[..=ptr].to_owned()) // chop off filename, leave '/'.
}

/// Translation of `SDL_SYS_GetExeName()`.
pub(super) fn get_exe_name() -> Result<String> {
    let path = get_exe_path().ok_or_else(|| Error::new("Couldn't locate our executable"))?;
    match last_separator(&path) {
        Some(ptr) => Ok(path[ptr + 1..].to_owned()),
        None => Ok(path),
    }
}

/// Translation of `SDL_IsUbuntuTouch()`.
#[cfg(target_os = "linux")]
fn is_ubuntu_touch() -> bool {
    static IS_UBUNTU_TOUCH: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *IS_UBUNTU_TOUCH
        .get_or_init(|| getenv("XDG_SESSION_DESKTOP").is_some_and(|v| v == "ubuntu-touch"))
}

/// `SDL_PROP_GLOBAL_SYSTEM_UBUNTU_TOUCH_APPID_STRING`.
#[cfg(target_os = "linux")]
pub const PROP_GLOBAL_SYSTEM_UBUNTU_TOUCH_APPID_STRING: &str = "SDL.system.ubuntu_touch.appid";

/// `mkdir(path, 0700)`, where an existing entry counts as success.
#[cfg(unix)]
fn mkdir_0700(path: &str) -> std::io::Result<()> {
    use std::os::unix::fs::DirBuilderExt;
    match std::fs::DirBuilder::new().mode(0o700).create(path) {
        Err(e) if e.kind() != std::io::ErrorKind::AlreadyExists => Err(e),
        _ => Ok(()),
    }
}

/// Translation of `SDL_SYS_GetPrefPath()` (Unix).
#[cfg(unix)]
pub(super) fn get_pref_path(org: &str, app: &str) -> Result<String> {
    /*
     * We use XDG's base directory spec, even if you're not on Linux.
     *  This isn't strictly correct, but the results are relatively sane
     *  in any case.
     *
     * http://standards.freedesktop.org/basedir-spec/basedir-spec-latest.html
     */
    let envr = getenv("XDG_DATA_HOME");
    let (mut org, mut app) = (org.to_owned(), app.to_owned());

    #[cfg(target_os = "linux")]
    if is_ubuntu_touch() {
        // On Ubuntu Touch, the only allowed data folder is:
        //     ~/.local/share/<app id>/

        let props = crate::properties::Properties::global();
        let Some(appid) = props.get_string(PROP_GLOBAL_SYSTEM_UBUNTU_TOUCH_APPID_STRING) else {
            return Err(Error::new(
                "Ubuntu Touch App ID missing from global properties",
            ));
        };

        app = appid;
        org = String::new();
    }

    let (envr, mut prepend) = match envr {
        Some(envr) => (envr, "/"),
        None => {
            // You end up with "$HOME/.local/share/Game Name 2"
            let Some(home) = getenv("HOME") else {
                // we could take heroic measures with /etc/passwd, but oh well.
                return Err(Error::new(
                    "neither XDG_DATA_HOME nor HOME environment is set",
                ));
            };
            (home, "/.local/share/")
        }
    };

    if envr.ends_with('/') {
        prepend = &prepend[1..];
    }

    let result = if !org.is_empty() {
        format!("{envr}{prepend}{org}/{app}/")
    } else {
        format!("{envr}{prepend}{app}/")
    };

    let fail = |e: std::io::Error| {
        Error::new(format!(
            "Couldn't create directory '{result}': '{}'",
            crate::io::strerror(&e)
        ))
    };
    for (i, _) in result.match_indices('/').filter(|&(i, _)| i >= 1) {
        mkdir_0700(&result[..i]).map_err(fail)?;
    }
    // (the loop's last stop already made the full path, which ends in '/')
    mkdir_0700(&result).map_err(fail)?;

    Ok(result)
}

/// Translation of `SDL_SYS_GetPrefPath()` (Windows).
///
/// Upstream asks `SHGetFolderPathW(CSIDL_APPDATA | CSIDL_FLAG_CREATE)`;
/// without the Win32 bindings this reads `%APPDATA%`, which Windows sets to
/// that same Roaming AppData folder.
#[cfg(windows)]
pub(super) fn get_pref_path(org: &str, app: &str) -> Result<String> {
    let Some(appdata) = std::env::var_os("APPDATA") else {
        return Err(Error::new("Couldn't locate our prefpath"));
    };
    let mut path = appdata.to_string_lossy().into_owned();

    // (MAX_PATH)
    if org.encode_utf16().count() + app.encode_utf16().count() + path.encode_utf16().count() + 3 + 1
        > 260
    {
        return Err(Error::new("Path too long."));
    }

    let create = |path: &str| match std::fs::create_dir(path) {
        Err(e) if e.kind() != std::io::ErrorKind::AlreadyExists => {
            Err(Error::new("Couldn't create a prefpath."))
        }
        _ => Ok(()),
    };

    if !org.is_empty() {
        path.push('\\');
        path.push_str(org);
    }
    create(&path)?;

    path.push('\\');
    path.push_str(app);
    create(&path)?;

    path.push('\\');
    Ok(path)
}

#[cfg(not(any(unix, windows)))]
pub(super) fn get_pref_path(_org: &str, _app: &str) -> Result<String> {
    Err(Error::unsupported())
}

/// Translation of `xdg_user_dir_lookup_with_fallback()`.
#[cfg(unix)]
fn xdg_user_dir_lookup_with_fallback(type_: &str, fallback: Option<&str>) -> Option<String> {
    let lookup = || -> Option<String> {
        let home_dir = getenv("HOME")?;

        let config_file = match getenv("XDG_CONFIG_HOME") {
            Some(config_home) if !config_home.is_empty() => format!("{config_home}/user-dirs.dirs"),
            _ => format!("{home_dir}/.config/user-dirs.dirs"),
        };

        let file = std::fs::read(config_file).ok()?;

        let mut user_dir = None;
        // (fgets() with a 512-byte buffer splits longer lines.)
        for line in file.split(|&b| b == b'\n').flat_map(|l| l.chunks(511)) {
            let mut p = line;
            let skip_blanks = |p: &mut &[u8]| {
                while let [b' ' | b'\t', rest @ ..] = *p {
                    *p = rest;
                }
            };
            skip_blanks(&mut p);

            let Some(rest) = p.strip_prefix(b"XDG_") else {
                continue;
            };
            let Some(rest) = rest.strip_prefix(type_.as_bytes()) else {
                continue;
            };
            let Some(rest) = rest.strip_prefix(b"_DIR") else {
                continue;
            };
            p = rest;

            skip_blanks(&mut p);

            let Some(rest) = p.strip_prefix(b"=") else {
                continue;
            };
            p = rest;

            skip_blanks(&mut p);

            let Some(rest) = p.strip_prefix(b"\"") else {
                continue;
            };
            p = rest;

            let mut dir: Vec<u8>;
            if let Some(rest) = p.strip_prefix(b"$HOME/") {
                p = rest;
                dir = format!("{home_dir}/").into_bytes();
            } else if p.first() != Some(&b'/') {
                continue;
            } else {
                dir = Vec::new();
            }

            let mut i = 0;
            while i < p.len() && p[i] != b'"' {
                if p[i] == b'\\' && i + 1 < p.len() {
                    i += 1;
                }
                dir.push(p[i]);
                i += 1;
            }
            user_dir = Some(String::from_utf8_lossy(&dir).into_owned());
        }
        user_dir
    };

    lookup().or_else(|| fallback.map(str::to_owned))
}

/// Translation of `xdg_user_dir_lookup()`.
#[cfg(unix)]
fn xdg_user_dir_lookup(type_: &str) -> Option<String> {
    if let Some(dir) = xdg_user_dir_lookup_with_fallback(type_, None) {
        return Some(dir);
    }

    let home_dir = getenv("HOME")?;

    // Special case desktop for historical compatibility
    if type_ == "DESKTOP" {
        return Some(format!("{home_dir}/Desktop"));
    }

    None
}

/// Translation of `SDL_SYS_GetUserFolder()` (Unix).
#[cfg(unix)]
pub(super) fn get_user_folder(folder: Folder) -> Result<String> {
    /* According to `man xdg-user-dir`, the possible values are:
        DESKTOP
        DOWNLOAD
        TEMPLATES
        PUBLICSHARE
        DOCUMENTS
        MUSIC
        PICTURES
        VIDEOS
    */
    let param = match folder {
        Folder::Home => {
            let Some(home) = getenv("HOME") else {
                return Err(Error::new("No $HOME environment variable available"));
            };
            return Ok(append_slash(home));
        }
        Folder::Desktop => "DESKTOP",
        Folder::Documents => "DOCUMENTS",
        Folder::Downloads => "DOWNLOAD",
        Folder::Music => "MUSIC",
        Folder::Pictures => "PICTURES",
        Folder::PublicShare => "PUBLICSHARE",
        Folder::SavedGames => return Err(Error::new("Saved Games folder unavailable on XDG")),
        Folder::Screenshots => return Err(Error::new("Screenshots folder unavailable on XDG")),
        Folder::Templates => "TEMPLATES",
        Folder::Videos => "VIDEOS",
    };

    match xdg_user_dir_lookup(param) {
        Some(result) => Ok(append_slash(result)),
        None => Err(Error::new("XDG directory not available")),
    }
}

/// Translation of `SDL_SYS_GetUserFolder()` (Windows).
///
/// Upstream asks `SHGetKnownFolderPath`/`SHGetFolderPathW`; without the
/// Win32 bindings only the home folder (`%USERPROFILE%`) is available.
#[cfg(windows)]
pub(super) fn get_user_folder(folder: Folder) -> Result<String> {
    match folder {
        Folder::Home => match std::env::var_os("USERPROFILE") {
            Some(home) => Ok(append_slash(home.to_string_lossy().into_owned())),
            None => Err(Error::new("Couldn't get folder")),
        },
        _ => Err(Error::unsupported()),
    }
}

#[cfg(not(any(unix, windows)))]
pub(super) fn get_user_folder(_folder: Folder) -> Result<String> {
    Err(Error::unsupported())
}

/// The `append_slash:` label.
fn append_slash(mut result: String) -> String {
    result.push(SEP);
    result
}
