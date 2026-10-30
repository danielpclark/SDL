// Rust translation of src/filesystem/android/SDL_sysfilesystem.c from
// Simple DirectMedia Layer.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! The `SDL_SYS_*` path lookups of Android: the base path is the app
//! package's assets (`assets://`, which the file operations and
//! `IoStream::from_file()` read through the asset manager), the pref path
//! is the app's internal storage (whatever the organization and the app
//! names), the executable's name is the package's, and there are no user
//! folders. The current directory is the POSIX file operations'
//! (`SDL_FSOPS_POSIX`).
//!
//! The internal storage and the package's name come from the app's Java VM
//! (`core::android`): a program that isn't an app (one spawned from the
//! shell) has neither.

use super::Folder;
use crate::core::android as core_android;
use crate::error::{Error, Result};

/// Translation of `SDL_SYS_GetBasePath()`.
pub(super) fn get_base_path() -> Result<String> {
    Ok("assets://".to_owned())
}

/// Translation of `SDL_SYS_GetExeName()`.
pub(super) fn get_exe_name() -> Result<String> {
    core_android::get_android_package_name()
}

/// Translation of `SDL_SYS_GetPrefPath()`: the internal storage path, with
/// a separator at the end (`org` and `app` aren't used).
pub(super) fn get_pref_path(_org: &str, _app: &str) -> Result<String> {
    let path = core_android::get_android_internal_storage_path()?;
    Ok(format!("{path}/"))
}

/// The relative `path` in the app's internal storage: `"%s/%s"` of the
/// internal storage path and it, as upstream's file operations and
/// `SDL_IOFromFile()` make it. Note (upstream): SDL's printf writes a null
/// string as `(null)`, which the internal storage path is when the app's
/// can't be had. A program that isn't an app (no Java VM: a spawned one)
/// has no internal storage, where upstream dereferences a null
/// environment: there `path` stays as it is, relative to the current
/// directory, as on the other Unix platforms.
pub(crate) fn in_internal_storage(path: &str) -> String {
    if crate::core::android::env::java_vm().is_none() {
        return path.to_owned();
    }
    let base = core_android::get_android_internal_storage_path().ok();
    format!("{}/{}", base.as_deref().unwrap_or("(null)"), path)
}

/// Translation of `SDL_SYS_GetUserFolder()`.
pub(super) fn get_user_folder(_folder: Folder) -> Result<String> {
    /* TODO: see https://developer.android.com/reference/android/os/Environment#lfields
    and https://stackoverflow.com/questions/39332085/get-path-to-pictures-directory */
    Err(Error::unsupported())
}
