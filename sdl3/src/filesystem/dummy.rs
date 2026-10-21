// Rust translation of src/filesystem/dummy/SDL_sysfilesystem.c from Simple
// DirectMedia Layer.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! The `SDL_SYS_*` path lookups of a platform without them. Emscripten
//! uses these until its own backend (`src/filesystem/emscripten`) is
//! translated, in the Emscripten port's PR 5. The current directory is the
//! POSIX file operations' (`SDL_FSOPS_POSIX`), as upstream's.

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
