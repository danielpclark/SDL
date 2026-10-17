// Rust translation of src/filetype.c from libxmp 4.7.3 (as vendored by SDL_mixer).
// Extended Module Player
// Copyright (C) 1996-2026 Claudio Matsuoka and Hipolito Carraro Jr
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! Whether a path names a file or a directory (`stat()`, or
//! `GetFileAttributesA()` on Windows), for the loaders that look for
//! instruments in files of their own.

use std::path::PathBuf;

use super::common::*;

/// A path's bytes as a path of the platform's (upstream hands the bytes to
/// the C library as they are).
pub(crate) fn os_path(path: &[u8]) -> PathBuf {
    let path = &path[..strlen(path)];
    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStrExt;
        PathBuf::from(std::ffi::OsStr::from_bytes(path))
    }
    #[cfg(not(unix))]
    {
        PathBuf::from(String::from_utf8_lossy(path).into_owned())
    }
}

/// Translation of `libxmp_get_filetype()`.
pub(crate) fn libxmp_get_filetype(path: &[u8]) -> i32 {
    let Ok(st) = std::fs::metadata(os_path(path)) else {
        return XMP_FILETYPE_NONE;
    };
    if st.is_dir() {
        return XMP_FILETYPE_DIR;
    }
    #[cfg(windows)]
    {
        XMP_FILETYPE_FILE
    }
    #[cfg(not(windows))]
    {
        if st.is_file() {
            return XMP_FILETYPE_FILE;
        }
        XMP_FILETYPE_NONE
    }
}
