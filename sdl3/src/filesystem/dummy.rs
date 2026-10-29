// Rust translation of src/filesystem/dummy/SDL_sysfilesystem.c from Simple
// DirectMedia Layer.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! The `SDL_SYS_*` path lookups of a platform without a backend
//! (`SDL_FILESYSTEM_DUMMY`): every one is unsupported. The current
//! directory is the POSIX file operations' (`SDL_FSOPS_POSIX`).
//!
//! Android stands in here until its backend (`SDL_FILESYSTEM_ANDROID`: the
//! `assets://` base path, the package name and the internal storage path,
//! through JNI) is translated.

use super::Folder;
use crate::error::{Error, Result};

/// Translation of `SDL_SYS_GetBasePath()`.
pub(super) fn get_base_path() -> Result<String> {
    Err(Error::unsupported())
}

/// Translation of `SDL_SYS_GetExeName()`.
pub(super) fn get_exe_name() -> Result<String> {
    Err(Error::unsupported())
}

/// Translation of `SDL_SYS_GetPrefPath()`.
pub(super) fn get_pref_path(_org: &str, _app: &str) -> Result<String> {
    Err(Error::unsupported())
}

/// Translation of `SDL_SYS_GetUserFolder()`.
pub(super) fn get_user_folder(_folder: Folder) -> Result<String> {
    Err(Error::unsupported())
}
