// Rust translation of src/filesystem/posix/SDL_sysfsops.c from Simple
// DirectMedia Layer.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! The `SDL_SYS_*` filesystem operations, written against `std::fs` (which
//! wraps the same POSIX calls: `opendir`/`readdir`, `remove`, `rename`,
//! `mkdir`, `stat` and `getcwd`), on every platform but Windows.
//!
//! On iOS (`SDL_PLATFORM_IOS`: every UIKit platform), whose apps can't
//! write to their current directory, relative paths are in the pref path
//! (`SDL_GetPrefPath("", "")`).

use std::ops::ControlFlow;

use super::{EnumerationResult, PathInfo, PathType};
use crate::error::{Error, Result};
use crate::io::{strerror, IoStatus, IoStream};
use crate::time::Time;

/// `path` in the pref path, `SDL_GetPrefPath("", "")`'s path followed by
/// it (UIKit).
#[cfg(all(target_vendor = "apple", not(target_os = "macos")))]
fn in_pref_path(path: &str) -> Result<String> {
    let base = super::get_pref_path(Some(""), "")?;
    Ok(format!("{base}{path}"))
}

/// Translation of `SDL_SYS_EnumerateDirectory()`.
pub(super) fn enumerate_directory(
    path: &str,
    cb: &mut dyn FnMut(&str, &str) -> EnumerationResult,
) -> Result<()> {
    #[allow(unused_mut)] // (only UIKit makes it absolute)
    let mut apath: Option<String> = None; // absolute path (for Android, iOS, etc). Overrides `path`.

    #[cfg(all(target_vendor = "apple", not(target_os = "macos")))]
    if path.is_empty() {
        return Err(Error::new("No such file or directory"));
    } else if !path.starts_with('/') {
        apath = Some(in_pref_path(path)?);
    }

    let full = apath.as_deref().unwrap_or(path);
    // (where the caller's path starts in it: the callback is given that part)
    let extralen = full.len() - path.len();
    // trim down to a single path separator at the end, in case the caller added one or more.
    let pathwithsep = if full.is_empty() {
        String::new()
    } else {
        format!("{}/", full.trim_end_matches('/'))
    };

    let dir = std::fs::read_dir(if pathwithsep.is_empty() {
        // opendir("") fails with ENOENT; std refuses the empty path too.
        std::path::Path::new("")
    } else {
        std::path::Path::new(&pathwithsep)
    })
    .map_err(|e| Error::new(format!("Can't open directory: {}", strerror(&e))))?;

    // (readdir()'s "." and ".." entries are never reported by std.)
    for ent in dir {
        let Ok(ent) = ent else {
            break; // readdir() returning NULL on error ends the loop too.
        };
        let name = ent.file_name();
        let name = name.to_string_lossy();
        if name == "." || name == ".." {
            continue;
        }
        match cb(&pathwithsep[extralen..], &name)? {
            ControlFlow::Continue(()) => {}
            ControlFlow::Break(()) => break,
        }
    }
    Ok(())
}

/// Translation of `SDL_SYS_RemovePath()`: `remove()` deletes a file or an
/// empty directory.
pub(super) fn remove_path(path: &str) -> Result<()> {
    #[cfg(all(target_vendor = "apple", not(target_os = "macos")))]
    let apath;
    #[cfg(all(target_vendor = "apple", not(target_os = "macos")))]
    let path = if path.starts_with('/') {
        path
    } else {
        apath = in_pref_path(path)?;
        &apath
    };
    let rc = match std::fs::symlink_metadata(path) {
        Ok(md) if md.is_dir() => std::fs::remove_dir(path),
        Ok(_) => std::fs::remove_file(path),
        Err(e) => Err(e),
    };
    match rc {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            // It's already gone, this is a success
            Ok(())
        }
        Err(e) => Err(Error::new(format!("Can't remove path: {}", strerror(&e)))),
    }
}

/// Translation of `SDL_SYS_RenamePath()`.
pub(super) fn rename_path(oldpath: &str, newpath: &str) -> Result<()> {
    #[cfg(all(target_vendor = "apple", not(target_os = "macos")))]
    let (aoldpath, anewpath);
    #[cfg(all(target_vendor = "apple", not(target_os = "macos")))]
    let (oldpath, newpath) = {
        let base = if !oldpath.starts_with('/') || !newpath.starts_with('/') {
            super::get_pref_path(Some(""), "")?
        } else {
            String::new()
        };
        let oldpath = if !oldpath.starts_with('/') {
            aoldpath = format!("{base}{oldpath}");
            &aoldpath
        } else {
            oldpath
        };
        let newpath = if !newpath.starts_with('/') {
            anewpath = format!("{base}{newpath}");
            &anewpath
        } else {
            newpath
        };
        (oldpath, newpath)
    };
    std::fs::rename(oldpath, newpath)
        .map_err(|e| Error::new(format!("Can't rename path: {}", strerror(&e))))
}

/// Translation of `SDL_SYS_CopyFile()`.
pub(super) fn copy_file(oldpath: &str, newpath: &str) -> Result<()> {
    const MAXLEN: usize = 4096;

    let mut input = IoStream::from_file(oldpath, "rb")?;
    let mut output = IoStream::from_file(newpath, "wb")?;

    let mut buffer = vec![0u8; MAXLEN];
    loop {
        let len = input.read(&mut buffer);
        if len == 0 {
            break;
        }
        if output.write(&buffer[..len]) < len {
            return Err(stream_error(&output, "Error writing to datastream"));
        }
    }
    if input.status() != IoStatus::Eof {
        return Err(stream_error(&input, "Error reading from datastream"));
    }

    // (SDL_CloseIO(input)'s result is ignored.)
    let _ = input.close();

    output.flush()?;

    output.close() // it's gone, even if it failed.
}

/// The error a failed stream operation left behind.
fn stream_error(stream: &IoStream<'_>, fallback: &'static str) -> Error {
    stream
        .last_error()
        .cloned()
        .unwrap_or_else(|| Error::new(fallback))
}

/// Translation of `SDL_SYS_CreateDirectory()`.
pub(super) fn create_directory(path: &str) -> Result<()> {
    #[cfg_attr(not(unix), allow(unused_mut))] // (only Unix sets a mode)
    let mut builder = std::fs::DirBuilder::new();
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        builder.mode(0o770);
    }
    #[cfg(not(all(target_vendor = "apple", not(target_os = "macos"))))]
    let rc = builder.create(path);
    #[cfg(all(target_vendor = "apple", not(target_os = "macos")))]
    let rc = if path.starts_with('/') {
        builder.create(path)
    } else {
        builder.create(in_pref_path(path)?)
    };
    match rc {
        Ok(()) => Ok(()),
        Err(e) => {
            // FIXME (upstream): on iOS a relative path is made in the pref
            // path, but whether it already is a directory is asked of the
            // path as given, in the current directory: a relative directory
            // that exists is a failure (EEXIST), unless the current
            // directory has one of that name too.
            if e.kind() == std::io::ErrorKind::AlreadyExists
                && std::fs::metadata(path).is_ok_and(|m| m.is_dir())
            {
                return Ok(()); // it already exists and it's a directory, consider it success.
            }
            Err(Error::new(format!(
                "Can't create directory: {}",
                strerror(&e)
            )))
        }
    }
}

/// Nanoseconds since the epoch, `SDL_SECONDS_TO_NS(sec) + nsec`.
#[cfg(unix)]
fn timespec(sec: i64, nsec: i64) -> Time {
    Time::from_nanos(sec.wrapping_mul(1_000_000_000).wrapping_add(nsec))
}

#[cfg(not(unix))]
fn system_time(t: std::io::Result<std::time::SystemTime>) -> Time {
    match t.map(|t| t.duration_since(std::time::UNIX_EPOCH)) {
        Ok(Ok(d)) => Time::from_nanos(d.as_nanos() as i64),
        _ => Time::UNIX_EPOCH,
    }
}

/// Translation of `SDL_SYS_GetPathInfo()`; like `stat()`, symlinks are followed.
pub(super) fn get_path_info(path: &str) -> Result<PathInfo> {
    #[cfg(not(all(target_vendor = "apple", not(target_os = "macos"))))]
    let rc = std::fs::metadata(path);
    // (on iOS a relative path in the pref path, else in the current directory)
    #[cfg(all(target_vendor = "apple", not(target_os = "macos")))]
    let rc = if path.starts_with('/') {
        std::fs::metadata(path)
    } else {
        std::fs::metadata(in_pref_path(path)?).or_else(|_| std::fs::metadata(path))
    };
    let statbuf = rc.map_err(|e| Error::new(format!("Can't stat: {}", strerror(&e))))?;
    let mut info = PathInfo::default();
    if statbuf.is_file() {
        info.kind = PathType::File;
        info.size = statbuf.len();
    } else if statbuf.is_dir() {
        info.kind = PathType::Directory;
        info.size = 0;
    } else {
        info.kind = PathType::Other;
        info.size = statbuf.len();
    }

    #[cfg(unix)]
    {
        // POSIX.1-2008 standard
        use std::os::unix::fs::MetadataExt;
        info.create_time = timespec(statbuf.ctime(), statbuf.ctime_nsec());
        info.modify_time = timespec(statbuf.mtime(), statbuf.mtime_nsec());
        info.access_time = timespec(statbuf.atime(), statbuf.atime_nsec());
    }
    #[cfg(not(unix))]
    {
        info.create_time = system_time(statbuf.created());
        info.modify_time = system_time(statbuf.modified());
        info.access_time = system_time(statbuf.accessed());
    }
    Ok(info)
}

/// Translation of `SDL_SYS_GetCurrentDirectory()`.
///
/// Note that this is actually part of filesystem, not fsops, but everything
/// that uses posix fsops uses this implementation, even with separate
/// filesystem code.
pub(super) fn get_current_directory() -> Result<String> {
    let cwd = std::env::current_dir()
        .map_err(|e| Error::new(format!("getcwd failed: {}", strerror(&e))))?;
    let mut buf = cwd.to_string_lossy().into_owned();

    // make sure there's a path separator at the end.
    if !buf.ends_with('/') {
        buf.push('/');
    }
    Ok(buf)
}
