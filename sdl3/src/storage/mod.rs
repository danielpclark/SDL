// Rust translation of src/storage/SDL_storage.c from Simple DirectMedia Layer.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! Abstract storage containers for read-only game data ("title storage")
//! and per-user save data ("user storage"). Translation of `SDL_storage.h`.
//!
//! Storage paths always use `/` separators and may not contain relative
//! components (`.` or `..`).

mod generic;

use crate::error::{Error, Result};
use crate::filesystem::{internal_glob_directory, EnumerationResult, GlobFlags, PathInfo};
use crate::hints;
use crate::properties::Properties;
use crate::stdlib::string::strncasecmp;

/// A storage backend. Translation of `SDL_StorageInterface`.
///
/// Every method has a default that behaves like a `NULL` function pointer
/// in the C interface: the operation reports
/// [`Unsupported`](crate::ErrorKind::Unsupported) (and [`ready`](Self::ready)
/// reports `true`). Paths have already been validated.
pub trait StorageInterface: Send {
    /// Called when the storage is closed.
    fn close(&mut self) -> Result<()> {
        Ok(())
    }

    /// Optional, returns whether the storage is currently ready for access.
    fn ready(&self) -> bool {
        true
    }

    /// Enumerate a directory, optional for write-only storage.
    fn enumerate(
        &self,
        _path: &str,
        _callback: &mut dyn FnMut(&str, &str) -> EnumerationResult,
    ) -> Result<()> {
        Err(Error::unsupported())
    }

    /// Get path information, optional for write-only storage.
    fn info(&self, _path: &str) -> Result<PathInfo> {
        Err(Error::unsupported())
    }

    /// Read a file from storage, optional for write-only storage.
    fn read_file(&self, _path: &str, _destination: &mut [u8]) -> Result<()> {
        Err(Error::unsupported())
    }

    /// Write a file to storage, optional for read-only storage.
    fn write_file(&self, _path: &str, _source: &[u8]) -> Result<()> {
        Err(Error::unsupported())
    }

    /// Create a directory, optional for read-only storage.
    fn mkdir(&self, _path: &str) -> Result<()> {
        Err(Error::unsupported())
    }

    /// Remove a file or empty directory, optional for read-only storage.
    fn remove(&self, _path: &str) -> Result<()> {
        Err(Error::unsupported())
    }

    /// Rename a path, optional for read-only storage.
    fn rename(&self, _oldpath: &str, _newpath: &str) -> Result<()> {
        Err(Error::unsupported())
    }

    /// Copy a file, optional for read-only storage.
    fn copy(&self, _oldpath: &str, _newpath: &str) -> Result<()> {
        Err(Error::unsupported())
    }

    /// Get the space remaining, optional for read-only storage.
    fn space_remaining(&self) -> Result<u64> {
        Err(Error::unsupported())
    }
}

/// An abstract interface for filesystem access. Translation of `SDL_Storage`.
///
/// Dropping a `Storage` closes it; call [`close`](Storage::close) to see
/// whether closing succeeded.
pub struct Storage {
    iface: Box<dyn StorageInterface>,
    closed: bool,
}

impl std::fmt::Debug for Storage {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Storage")
            .field("ready", &self.iface.ready())
            .finish_non_exhaustive()
    }
}

/// Translation of `TitleStorageBootStrap`.
struct TitleStorageBootStrap {
    name: &'static str,
    #[allow(dead_code)]
    desc: &'static str,
    create: fn(Option<&str>, Option<&Properties>) -> Result<Storage>,
}

/// Translation of `UserStorageBootStrap`.
struct UserStorageBootStrap {
    name: &'static str,
    #[allow(dead_code)]
    desc: &'static str,
    create: fn(Option<&str>, &str, Option<&Properties>) -> Result<Storage>,
}

/// Available title storage drivers. Translation of `titlebootstrap`.
static TITLEBOOTSTRAP: &[&TitleStorageBootStrap] = &[&generic::GENERIC_TITLEBOOTSTRAP];

/// Available user storage drivers. Translation of `userbootstrap`.
///
/// (The Steam and private backends need their platform SDKs.)
static USERBOOTSTRAP: &[&UserStorageBootStrap] = &[&generic::GENERIC_USERBOOTSTRAP];

// we don't make any effort to convert path separators here, because a)
// everything including Windows will accept a '/' separator and b) that
// conversion should probably happen in the storage backend anyhow.

/// Translation of `ValidateStoragePath()`.
fn validate_storage_path(path: &str) -> Result<()> {
    if path.contains('\\') {
        return Err(Error::new(
            "Windows-style path separators ('\\') not permitted, use '/' instead.",
        ));
    }

    let mut prev = path;
    while let Some(ptr) = prev.find('/') {
        if prev.starts_with("./") || prev.starts_with("../") {
            return Err(Error::new("Relative paths not permitted"));
        }
        prev = &prev[ptr + 1..];
    }

    // check the last path element (or the only path element).
    if prev == "." || prev == ".." {
        return Err(Error::new("Relative paths not permitted"));
    }

    Ok(())
}

/// The driver-list walk shared by `SDL_OpenTitleStorage()` and
/// `SDL_OpenUserStorage()`.
fn select_driver<D>(
    hint: &str,
    drivers: &[&D],
    name_of: impl Fn(&D) -> &'static str,
    create: impl Fn(&D) -> Result<Storage>,
    subsystem: &str,
    none_available: &'static str,
) -> Result<Storage> {
    let mut storage: Option<(&'static str, Storage)> = None;

    // Select the proper storage driver
    let driver_name = hints::get(hint);
    match driver_name.as_deref() {
        Some(driver_name) if !driver_name.is_empty() => {
            for driver_attempt in driver_name.split(',') {
                if storage.is_some() || driver_attempt.is_empty() {
                    break;
                }
                if let Some(d) = drivers.iter().find(|d| {
                    let name = name_of(d);
                    name.len() == driver_attempt.len()
                        && strncasecmp(name, driver_attempt, driver_attempt.len()).is_eq()
                }) {
                    storage = create(d).ok().map(|s| (name_of(d), s));
                }
            }
        }
        _ => {
            for d in drivers {
                storage = create(d).ok().map(|s| (name_of(d), s));
                if storage.is_some() {
                    break;
                }
            }
        }
    }

    match storage {
        Some((name, storage)) => {
            crate::debug!(
                crate::log::Category::System,
                "SDL chose {subsystem} backend '{name}'"
            );
            Ok(storage)
        }
        None => Err(match driver_name {
            Some(driver_name) => Error::new(format!("{driver_name} not available")),
            None => Error::new(none_available),
        }),
    }
}

impl Storage {
    /// Opens up a read-only container for the application's filesystem.
    /// Translation of `SDL_OpenTitleStorage()`.
    ///
    /// By default, [`Storage::open_title`] uses the generic storage
    /// implementation. When the path override is not provided, the generic
    /// implementation will use the output of
    /// [`get_base_path`](crate::filesystem::get_base_path) as the base path.
    pub fn open_title(override_: Option<&str>, props: Option<&Properties>) -> Result<Storage> {
        select_driver(
            hints::STORAGE_TITLE_DRIVER,
            TITLEBOOTSTRAP,
            |d| d.name,
            |d| (d.create)(override_, props),
            "title_storage",
            "No available title storage driver",
        )
    }

    /// Opens up a container for a user's unique read/write filesystem.
    /// Translation of `SDL_OpenUserStorage()`.
    ///
    /// While title storage can generally be kept open throughout runtime,
    /// user storage should only be opened when the client is ready to read
    /// or write files. This allows the backend to properly batch file
    /// operations and flush them when the container has been closed;
    /// ensuring safe and optimal save I/O.
    pub fn open_user(org: Option<&str>, app: &str, props: Option<&Properties>) -> Result<Storage> {
        select_driver(
            hints::STORAGE_USER_DRIVER,
            USERBOOTSTRAP,
            |d| d.name,
            |d| (d.create)(org, app, props),
            "user_storage",
            "No available user storage driver",
        )
    }

    /// Opens up a container for local filesystem storage.
    /// Translation of `SDL_OpenFileStorage()`.
    ///
    /// This is provided for development and tools. Portable applications
    /// should use [`Storage::open_title`] for access to game data and
    /// [`Storage::open_user`] for access to user data.
    ///
    /// `path` is the base path prepended to all storage paths; `None` (or
    /// an empty string) means the filesystem root.
    pub fn open_file(path: Option<&str>) -> Result<Storage> {
        generic::open_file_storage(path)
    }

    /// Opens up a container using a client-provided storage interface.
    /// Translation of `SDL_OpenStorage()`.
    ///
    /// Applications do not need to use this function unless they are
    /// providing their own Storage implementation. If you just need a
    /// Storage, you should use the built-in implementations in SDL, like
    /// [`Storage::open_title`] or [`Storage::open_user`].
    pub fn new(iface: impl StorageInterface + 'static) -> Storage {
        Storage {
            iface: Box::new(iface),
            closed: false,
        }
    }

    /// Closes and frees a storage container. Translation of `SDL_CloseStorage()`.
    ///
    /// Returns an error if the container had problems writing any pending
    /// data.
    pub fn close(mut self) -> Result<()> {
        self.closed = true;
        self.iface.close()
    }

    /// Checks if the storage container is ready to use.
    /// Translation of `SDL_StorageReady()`.
    ///
    /// This function should be called in regular intervals until it returns
    /// true - however, it is not recommended to spinwait on this call, as
    /// the backend may depend on a synchronous message loop. You might
    /// instead poll this in your game's main loop while processing events
    /// and drawing a loading screen.
    pub fn ready(&self) -> bool {
        self.iface.ready()
    }

    /// Query the size of a file within a storage container.
    /// Translation of `SDL_GetStorageFileSize()`.
    pub fn file_size(&self, path: &str) -> Result<u64> {
        self.path_info(path).map(|info| info.size)
    }

    /// Synchronously read a file from a storage container into a
    /// client-provided buffer. Translation of `SDL_ReadStorageFile()`.
    ///
    /// The value of `destination.len()` must match the length of the file
    /// exactly. Call [`Storage::file_size`] to get this value. This
    /// behavior may be relaxed in a future release. (The generic backend
    /// only fails when the file is shorter than the buffer.)
    pub fn read_file(&self, path: &str, destination: &mut [u8]) -> Result<()> {
        validate_storage_path(path)?;
        self.iface.read_file(path, destination)
    }

    /// Read a whole file: [`Storage::file_size`] followed by
    /// [`Storage::read_file`].
    pub fn load_file(&self, path: &str) -> Result<Vec<u8>> {
        let size = usize::try_from(self.file_size(path)?)
            .map_err(|_| Error::new("Read size exceeds SDL_SIZE_MAX"))?;
        let mut data = vec![0u8; size];
        self.read_file(path, &mut data)?;
        Ok(data)
    }

    /// Synchronously write a file from client memory into a storage
    /// container. Translation of `SDL_WriteStorageFile()`.
    pub fn write_file(&self, path: &str, source: &[u8]) -> Result<()> {
        validate_storage_path(path)?;
        self.iface.write_file(path, source)
    }

    /// Create a directory in a writable storage container.
    /// Translation of `SDL_CreateStorageDirectory()`.
    pub fn create_directory(&self, path: &str) -> Result<()> {
        validate_storage_path(path)?;
        self.iface.mkdir(path)
    }

    /// Enumerate a directory in a storage container through a callback
    /// function. Translation of `SDL_EnumerateStorageDirectory()`.
    ///
    /// This function provides every directory entry through an app-provided
    /// callback, called once for each directory entry, until all results
    /// have been provided or the callback stops (see
    /// [`enumerate_directory`](crate::filesystem::enumerate_directory)).
    ///
    /// `None` for `path` means the root of the storage tree.
    pub fn enumerate_directory(
        &self,
        path: Option<&str>,
        mut callback: impl FnMut(&str, &str) -> EnumerationResult,
    ) -> Result<()> {
        let path = path.unwrap_or(""); // we allow NULL to mean "root of the storage tree".
        validate_storage_path(path)?;
        self.iface.enumerate(path, &mut callback)
    }

    /// Remove a file or an empty directory in a writable storage container.
    /// Translation of `SDL_RemoveStoragePath()`.
    pub fn remove_path(&self, path: &str) -> Result<()> {
        validate_storage_path(path)?;
        self.iface.remove(path)
    }

    /// Rename a file or directory in a writable storage container.
    /// Translation of `SDL_RenameStoragePath()`.
    pub fn rename_path(&self, oldpath: &str, newpath: &str) -> Result<()> {
        validate_storage_path(oldpath)?;
        validate_storage_path(newpath)?;
        self.iface.rename(oldpath, newpath)
    }

    /// Copy a file in a writable storage container.
    /// Translation of `SDL_CopyStorageFile()`.
    pub fn copy_file(&self, oldpath: &str, newpath: &str) -> Result<()> {
        validate_storage_path(oldpath)?;
        validate_storage_path(newpath)?;
        self.iface.copy(oldpath, newpath)
    }

    /// Get information about a filesystem path in a storage container.
    /// Translation of `SDL_GetStoragePathInfo()`.
    pub fn path_info(&self, path: &str) -> Result<PathInfo> {
        validate_storage_path(path)?;
        self.iface.info(path)
    }

    /// Queries the remaining space in a storage container.
    /// Translation of `SDL_GetStorageSpaceRemaining()`.
    pub fn space_remaining(&self) -> Result<u64> {
        self.iface.space_remaining()
    }

    /// Enumerate a directory tree, filtered by pattern, and return a list.
    /// Translation of `SDL_GlobStorageDirectory()`.
    ///
    /// The pattern rules are those of
    /// [`glob_directory`](crate::filesystem::glob_directory). `None` for
    /// `path` means the root of the storage tree. The returned paths are
    /// relative to `path`.
    pub fn glob_directory(
        &self,
        path: Option<&str>,
        pattern: Option<&str>,
        flags: GlobFlags,
    ) -> Result<Vec<String>> {
        let path = path.unwrap_or(""); // we allow NULL to mean "root of the storage tree".
        validate_storage_path(path)?;

        internal_glob_directory(
            path,
            pattern,
            flags,
            // GlobStorageDirectoryEnumerator / GlobStorageDirectoryGetPathInfo
            &|p, cb| {
                validate_storage_path(p)?;
                self.iface.enumerate(p, cb)
            },
            &|p| self.path_info(p),
        )
    }
}

impl Drop for Storage {
    fn drop(&mut self) {
        if !self.closed {
            self.closed = true;
            let _ = self.iface.close();
        }
    }
}

#[cfg(test)]
mod tests;
