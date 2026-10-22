// Rust translation of src/filesystem/emscripten/SDL_sysfilesystem.c from
// Simple DirectMedia Layer.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! The `SDL_SYS_*` path lookups of Emscripten: the base path is the root of
//! emscripten's file system, the pref paths are under `/libsdl` (or under
//! `SDL_EMSCRIPTEN_PERSISTENT_PATH`, with the `emscripten-persistent-path`
//! feature, so that they're kept in IndexedDB), and the only user folder
//! is `$HOME`. The current directory is the POSIX file operations'
//! (`SDL_FSOPS_POSIX`), as upstream's.

use super::Folder;
use crate::error::{Error, Result};
use crate::stdlib::getenv;

/// Translation of `SDL_SYS_GetBasePath()`.
pub(super) fn get_base_path() -> Result<String> {
    Ok("/".to_owned())
}

/// Translation of `SDL_SYS_GetExeName()`.
pub(super) fn get_exe_name() -> Result<String> {
    // no EXE name on this system. (Upstream returns NULL without setting
    // an error; this one's message is empty.)
    Err(Error::new(""))
}

/// `mkdir(path, 0700) != 0 && errno != EEXIST`: an existing entry (a file
/// too, as POSIX has it) counts as success.
fn mkdir_0700(path: &str) -> std::io::Result<()> {
    use std::os::unix::fs::DirBuilderExt;
    match std::fs::DirBuilder::new().mode(0o700).create(path) {
        Err(e) if e.kind() != std::io::ErrorKind::AlreadyExists => Err(e),
        _ => Ok(()),
    }
}

/// Translation of `SDL_SYS_GetPrefPath()`.
pub(super) fn get_pref_path(org: &str, app: &str) -> Result<String> {
    #[cfg(feature = "emscripten-persistent-path")]
    let prepend = crate::app::PERSISTENT_PATH_STRING;
    #[cfg(not(feature = "emscripten-persistent-path"))]
    let prepend = "/libsdl";

    let result = if !org.is_empty() {
        format!("{prepend}/{org}/{app}/")
    } else {
        format!("{prepend}/{app}/")
    };

    // (the error names the path as far as it got: upstream's message shows
    // the string with a NUL at the slash it stopped at)
    let fail = |path: &str, e: std::io::Error| {
        Error::new(format!(
            "Couldn't create directory '{path}': '{}'",
            crate::io::strerror(&e)
        ))
    };
    for (ptr, _) in result.match_indices('/').filter(|&(i, _)| i >= 1) {
        mkdir_0700(&result[..ptr]).map_err(|e| fail(&result[..ptr], e))?;
    }

    // (the loop's last stop already made the full path, which ends in '/')
    mkdir_0700(&result).map_err(|e| fail(&result, e))?;

    Ok(result)
}

/// Translation of `SDL_SYS_GetUserFolder()`.
pub(super) fn get_user_folder(folder: Folder) -> Result<String> {
    if folder != Folder::Home {
        return Err(Error::new("Emscripten only supports the home folder"));
    }

    let Some(home) = getenv("HOME") else {
        return Err(Error::new("No $HOME environment variable available"));
    };

    // (upstream's snprintf() failure can't happen to a String)
    Ok(format!("{home}/"))
}
