// Rust translation of src/storage/generic/SDL_genericstorage.c from Simple
// DirectMedia Layer.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! The generic storage backend: a base directory on the local filesystem.

use super::{Storage, StorageInterface, TitleStorageBootStrap, UserStorageBootStrap};
use crate::error::{Error, Result};
use crate::filesystem::{self, EnumerationResult, PathInfo};
use crate::io::IoStream;
use crate::properties::Properties;

/// Which operations a generic storage offers (the three `SDL_StorageInterface`
/// tables of the C source).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Kind {
    /// `GENERIC_title_iface`: read-only.
    Title,
    /// `GENERIC_user_iface` and `GENERIC_file_iface`: read/write.
    ReadWrite,
}

/// A generic storage: the base path is the `userdata` of the C backend.
#[derive(Debug)]
struct GenericStorage {
    base: String,
    kind: Kind,
}

impl GenericStorage {
    /// Translation of `GENERIC_INTERNAL_CreateFullPath()`.
    fn full_path(&self, relative: &str) -> String {
        #[cfg(target_os = "android")]
        let relative = {
            // Removes any leading slash
            relative.strip_prefix(['/', '\\']).unwrap_or(relative)
        };
        format!("{}{}", self.base, relative)
    }

    fn writable(&self) -> Result<()> {
        match self.kind {
            Kind::ReadWrite => Ok(()),
            Kind::Title => Err(Error::unsupported()),
        }
    }
}

impl StorageInterface for GenericStorage {
    /// Translation of `GENERIC_EnumerateStorageDirectory()`.
    fn enumerate(
        &self,
        path: &str,
        callback: &mut dyn FnMut(&str, &str) -> EnumerationResult,
    ) -> Result<()> {
        let fullpath = self.full_path(path);
        let base_len = self.base.len();

        // Translation of `GENERIC_EnumerateDirectory()`:
        // SDL_EnumerateDirectory will return the full path, so for Storage we
        // can take the base directory and add its length to the dirname string,
        // effectively trimming the root without having to strdup anything.
        filesystem::enumerate_directory(&fullpath, |dirname, fname| {
            let dirname = dirname.get(base_len..).unwrap_or(""); // skip the base, just return the part inside of the Storage.
            if cfg!(windows) && dirname.ends_with('\\') {
                // storage layer always uses '/' path separators.
                let dirnamecpy = format!("{}/", &dirname[..dirname.len() - 1]);
                return callback(&dirnamecpy, fname);
            }
            callback(dirname, fname)
        })
    }

    /// Translation of `GENERIC_GetStoragePathInfo()`.
    fn info(&self, path: &str) -> Result<PathInfo> {
        filesystem::get_path_info(&self.full_path(path))
    }

    /// Translation of `GENERIC_ReadStorageFile()`.
    fn read_file(&self, path: &str, destination: &mut [u8]) -> Result<()> {
        let mut stream = IoStream::from_file(self.full_path(path), "rb")?;
        // FIXME: Should SDL_ReadIO use u64 now...?
        let result = if stream.read(destination) == destination.len() {
            Ok(())
        } else {
            Err(Error::new(
                "File length did not exactly match the destination length",
            ))
        };
        let _ = stream.close();
        result
    }

    /// Translation of `GENERIC_WriteStorageFile()`.
    fn write_file(&self, path: &str, source: &[u8]) -> Result<()> {
        self.writable()?;
        // TODO: Recursively create subdirectories with SDL_CreateDirectory
        let mut stream = IoStream::from_file(self.full_path(path), "wb")?;
        // FIXME: Should SDL_WriteIO use u64 now...?
        let result = if stream.write(source) == source.len() {
            Ok(())
        } else {
            Err(Error::new(
                "Resulting file length did not exactly match the source length",
            ))
        };
        let _ = stream.close();
        result
    }

    /// Translation of `GENERIC_CreateStorageDirectory()`.
    fn mkdir(&self, path: &str) -> Result<()> {
        self.writable()?;
        // TODO: Recursively create subdirectories with SDL_CreateDirectory
        filesystem::create_directory(&self.full_path(path))
    }

    /// Translation of `GENERIC_RemoveStoragePath()`.
    fn remove(&self, path: &str) -> Result<()> {
        self.writable()?;
        filesystem::remove_path(&self.full_path(path))
    }

    /// Translation of `GENERIC_RenameStoragePath()`.
    fn rename(&self, oldpath: &str, newpath: &str) -> Result<()> {
        self.writable()?;
        filesystem::rename_path(&self.full_path(oldpath), &self.full_path(newpath))
    }

    /// Translation of `GENERIC_CopyStorageFile()`.
    fn copy(&self, oldpath: &str, newpath: &str) -> Result<()> {
        self.writable()?;
        filesystem::copy_file(&self.full_path(oldpath), &self.full_path(newpath))
    }

    /// Translation of `GENERIC_GetStorageSpaceRemaining()`.
    fn space_remaining(&self) -> Result<u64> {
        self.writable()?;
        // TODO: There's totally a way to query a folder root's quota...
        Ok(u64::MAX)
    }
}

/// Translation of `GENERIC_Title_Create()`.
fn generic_title_create(override_: Option<&str>, _props: Option<&Properties>) -> Result<Storage> {
    let basepath = match override_ {
        Some(over) if !over.is_empty() => {
            // make sure override has a path separator at the end. If you're not on Windows and used '\\', that's on you.
            let need_sep = !over.ends_with('/') && !over.ends_with('\\');
            format!("{over}{}", if need_sep { "/" } else { "" })
        }
        // override == "" -> empty base (not "/")
        Some(_) => String::new(),
        None => filesystem::get_base_path()?,
    };

    Ok(Storage::new(GenericStorage {
        base: basepath,
        kind: Kind::Title,
    }))
}

/// Translation of `GENERIC_titlebootstrap`.
pub(super) static GENERIC_TITLEBOOTSTRAP: TitleStorageBootStrap = TitleStorageBootStrap {
    name: "generic",
    desc: "SDL generic title storage driver",
    create: generic_title_create,
};

/// Translation of `GENERIC_User_Create()`.
fn generic_user_create(
    org: Option<&str>,
    app: &str,
    _props: Option<&Properties>,
) -> Result<Storage> {
    let prefpath = filesystem::get_pref_path(org, app)?;
    Ok(Storage::new(GenericStorage {
        base: prefpath,
        kind: Kind::ReadWrite,
    }))
}

/// Translation of `GENERIC_userbootstrap`.
pub(super) static GENERIC_USERBOOTSTRAP: UserStorageBootStrap = UserStorageBootStrap {
    name: "generic",
    desc: "SDL generic user storage driver",
    create: generic_user_create,
};

/// Translation of `GENERIC_OpenFileStorage()`.
pub(super) fn open_file_storage(path: Option<&str>) -> Result<Storage> {
    #[allow(unused_mut)] // (Android never prepends)
    let mut prepend = None;

    #[cfg(target_os = "android")]
    let path = match path {
        // Use a base path of "." so the filesystem operations fall back to internal storage and the asset system
        Some(p) if !p.is_empty() => p,
        _ => "./",
    };

    #[cfg(not(target_os = "android"))]
    let path = {
        let path = match path {
            Some(p) if !p.is_empty() => p,
            _ if cfg!(windows) => "C:/",
            _ => "/",
        };

        let b = path.as_bytes();
        let is_absolute = if cfg!(windows) {
            let ch = b[0].to_ascii_uppercase();
            (ch == b'/') // some sort of absolute Unix-style path.
                || (ch == b'\\') // some sort of absolute Windows-style path.
                || (ch.is_ascii_uppercase()
                    && b.get(1) == Some(&b':')
                    && matches!(b.get(2), Some(b'\\' | b'/'))) // an absolute path with a drive letter.
        } else {
            b[0] == b'/' // some sort of absolute Unix-style path.
        };
        if !is_absolute {
            prepend = Some(filesystem::get_current_directory()?);
        }
        path
    };

    let appended_separator = if path.ends_with('/') || (cfg!(windows) && path.ends_with('\\')) {
        ""
    } else {
        "/"
    };
    let basepath = format!(
        "{}{}{}",
        prepend.as_deref().unwrap_or(""),
        path,
        appended_separator
    );

    Ok(Storage::new(GenericStorage {
        base: basepath,
        kind: Kind::ReadWrite,
    }))
}
