// Rust translation of src/filesystem/cocoa/SDL_sysfilesystem.m from Simple
// DirectMedia Layer.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! The Apple `SDL_SYS_*` path lookups: the main bundle (`NSBundle`) for the
//! base path and the executable's name, and
//! `NSSearchPathForDirectoriesInDomains()` for the pref path and the user
//! folders (tvOS, which has no persistent local storage, keeps the pref
//! path in its caches and has no user folders). The current directory and
//! the file operations are POSIX, as upstream's `SDL_FSOPS_POSIX`.

use std::cmp::Ordering;
use std::ffi::{c_char, CStr};

use super::Folder;
use crate::core::apple::foundation::{ns_array_count, ns_array_get, ns_string, string_from_ns};
use crate::core::apple::objc::{autoreleasepool, class, msg_send, Bool, Id, NSUInteger, YES};
use crate::error::{Error, Result};
#[cfg(not(target_os = "tvos"))]
use crate::stdlib::getenv;
use crate::stdlib::string::strcasecmp;

/// `NSSearchPathDirectory`.
type NSSearchPathDirectory = NSUInteger;
/// `NSSearchPathDomainMask`.
type NSSearchPathDomainMask = NSUInteger;

#[cfg(not(target_os = "tvos"))]
const NS_DOCUMENT_DIRECTORY: NSSearchPathDirectory = 9;
#[cfg(not(target_os = "tvos"))]
const NS_DESKTOP_DIRECTORY: NSSearchPathDirectory = 12;
#[cfg(not(target_os = "tvos"))]
const NS_APPLICATION_SUPPORT_DIRECTORY: NSSearchPathDirectory = 14;
#[cfg(target_os = "tvos")]
const NS_CACHES_DIRECTORY: NSSearchPathDirectory = 13;
#[cfg(not(target_os = "tvos"))]
const NS_DOWNLOADS_DIRECTORY: NSSearchPathDirectory = 15;
#[cfg(not(target_os = "tvos"))]
const NS_MOVIES_DIRECTORY: NSSearchPathDirectory = 17;
#[cfg(not(target_os = "tvos"))]
const NS_MUSIC_DIRECTORY: NSSearchPathDirectory = 18;
#[cfg(not(target_os = "tvos"))]
const NS_PICTURES_DIRECTORY: NSSearchPathDirectory = 19;
#[cfg(not(target_os = "tvos"))]
const NS_SHARED_PUBLIC_DIRECTORY: NSSearchPathDirectory = 21;
const NS_USER_DOMAIN_MASK: NSSearchPathDomainMask = 1;

#[link(name = "Foundation", kind = "framework")]
unsafe extern "C" {
    fn NSSearchPathForDirectoriesInDomains(
        directory: NSSearchPathDirectory,
        domain_mask: NSSearchPathDomainMask,
        expand_tilde: Bool,
    ) -> Id;
}

/// `[str fileSystemRepresentation]`, `None` for nil.
///
/// # Safety
///
/// `s` must be nil or a valid `NSString`.
unsafe fn file_system_representation(s: Id) -> Option<String> {
    if s.is_null() {
        return None;
    }
    // SAFETY: the caller's contract; the C string lives as long as the
    // autorelease pool around us, and is copied at once.
    unsafe {
        let p: *const c_char = msg_send![s, fileSystemRepresentation];
        if p.is_null() {
            return None;
        }
        Some(CStr::from_ptr(p).to_string_lossy().into_owned())
    }
}

/// `[[bundle infoDictionary] objectForKey:key]` as a string.
///
/// # Safety
///
/// `bundle` must be a valid `NSBundle`.
unsafe fn info_string(bundle: Id, key: &str) -> Option<String> {
    // SAFETY: the caller's contract; objectForKey: answers nil for a
    // missing key, and UTF8String nil for nil.
    unsafe {
        let info: Id = msg_send![bundle, infoDictionary];
        let value: Id = msg_send![info, objectForKey: ns_string(key).get()];
        string_from_ns(value)
    }
}

/// The first directory of `NSSearchPathForDirectoriesInDomains(dir,
/// NSUserDomainMask, YES)`: `None` if there is none, `Some(None)` if it has
/// no file system representation.
fn first_search_path(dir: NSSearchPathDirectory) -> Option<Option<String>> {
    // SAFETY: a Foundation function returning an autoreleased NSArray of
    // NSStrings (the caller holds an autorelease pool).
    unsafe {
        let array = NSSearchPathForDirectoriesInDomains(dir, NS_USER_DOMAIN_MASK, YES);
        if ns_array_count(array) == 0 {
            return None;
        }
        let str = ns_array_get(array, 0);
        Some(file_system_representation(str))
    }
}

/// `mkdir(path, 0700)` for each directory along `result` (after its first
/// character), ignoring failures as upstream does.
fn mkdir_parents(result: &str) {
    use std::os::unix::fs::DirBuilderExt;
    for (i, _) in result.match_indices('/').filter(|&(i, _)| i >= 1) {
        let _ = std::fs::DirBuilder::new().mode(0o700).create(&result[..i]);
    }
}

/// Translation of `SDL_SYS_GetBasePath()` (Cocoa).
pub(super) fn get_base_path() -> Result<String> {
    autoreleasepool(|| {
        // SAFETY: NSBundle's main bundle always exists; the NSStrings are
        // autoreleased into this pool.
        unsafe {
            let bundle: Id = msg_send![class!(NSBundle), mainBundle];
            let base_type = info_string(bundle, "SDL_FILESYSTEM_BASE_DIR_TYPE")
                .unwrap_or_else(|| "resource".to_owned());
            let base = if strcasecmp(&base_type, "bundle") == Ordering::Equal {
                let path: Id = msg_send![bundle, bundlePath];
                file_system_representation(path)
            } else if strcasecmp(&base_type, "parent") == Ordering::Equal {
                let path: Id = msg_send![bundle, bundlePath];
                let parent: Id = msg_send![path, stringByDeletingLastPathComponent];
                file_system_representation(parent)
            } else {
                // this returns the exedir for non-bundled and the resourceDir for bundled apps
                let path: Id = msg_send![bundle, resourcePath];
                file_system_representation(path)
            };

            // (upstream returns NULL without setting an error)
            match base {
                Some(base) => Ok(format!("{base}/")),
                None => Err(Error::new("Couldn't locate our executable")),
            }
        }
    })
}

/// Translation of `SDL_SYS_GetExeName()` (Cocoa).
pub(super) fn get_exe_name() -> Result<String> {
    autoreleasepool(|| {
        // SAFETY: as in get_base_path(); NSProcessInfo always exists.
        unsafe {
            let bundle: Id = msg_send![class!(NSBundle), mainBundle];
            let name = info_string(bundle, "CFBundleIdentifier")
                .or_else(|| info_string(bundle, "CFBundleDisplayName"))
                .or_else(|| info_string(bundle, "CFBundleName"))
                .or_else(|| info_string(bundle, "CFBundleExecutable"))
                .or_else(|| {
                    let info: Id = msg_send![class!(NSProcessInfo), processInfo];
                    let name: Id = msg_send![info, processName];
                    string_from_ns(name) // oh well.
                });
            // (upstream returns NULL without setting an error)
            name.ok_or_else(|| Error::new("Couldn't locate our executable"))
        }
    })
}

/// Translation of `SDL_SYS_GetPrefPath()` (Cocoa).
pub(super) fn get_pref_path(org: &str, app: &str) -> Result<String> {
    autoreleasepool(|| {
        #[cfg(not(target_os = "tvos"))]
        let dir = NS_APPLICATION_SUPPORT_DIRECTORY;
        #[cfg(target_os = "tvos")]
        let dir = {
            /* tvOS does not have persistent local storage!
             * The only place on-device where we can store data is
             * a cache directory that the OS can empty at any time.
             *
             * It's therefore very likely that save data will be erased
             * between sessions. If you want your app's save data to
             * actually stick around, you'll need to use iCloud storage.
             */
            static SHOWN: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
            if !SHOWN.swap(true, std::sync::atomic::Ordering::Relaxed) {
                crate::critical!(
                    crate::log::Category::System,
                    "tvOS does not have persistent local storage! Use iCloud storage if you want your data to persist between sessions.",
                );
            }

            NS_CACHES_DIRECTORY
        };

        // we only want the first item in the list.
        let Some(Some(base)) = first_search_path(dir) else {
            // (upstream returns NULL without setting an error)
            return Err(Error::new(if cfg!(target_os = "tvos") {
                "Couldn't find the Caches directory"
            } else {
                "Couldn't find the Application Support directory"
            }));
        };
        let result = if !org.is_empty() {
            format!("{base}/{org}/{app}/")
        } else {
            format!("{base}/{app}/")
        };
        // (the loop's last stop makes the full path, which ends in '/')
        mkdir_parents(&result);
        Ok(result)
    })
}

/// Translation of `SDL_SYS_GetUserFolder()` (Cocoa, tvOS).
#[cfg(target_os = "tvos")]
pub(super) fn get_user_folder(_folder: Folder) -> Result<String> {
    Err(Error::new("tvOS does not have persistent storage"))
}

/// Translation of `SDL_SYS_GetUserFolder()` (Cocoa).
#[cfg(not(target_os = "tvos"))]
pub(super) fn get_user_folder(folder: Folder) -> Result<String> {
    autoreleasepool(|| {
        let dir = match folder {
            Folder::Home => {
                let Some(base) = getenv("HOME") else {
                    return Err(Error::new("No $HOME environment variable available"));
                };
                return Ok(append_slash(&base));
            }
            Folder::Desktop => NS_DESKTOP_DIRECTORY,
            Folder::Documents => NS_DOCUMENT_DIRECTORY,
            Folder::Downloads => NS_DOWNLOADS_DIRECTORY,
            Folder::Music => NS_MUSIC_DIRECTORY,
            Folder::Pictures => NS_PICTURES_DIRECTORY,
            Folder::PublicShare => NS_SHARED_PUBLIC_DIRECTORY,
            Folder::SavedGames => {
                return Err(Error::new("Saved games folder not supported on Cocoa"))
            }
            Folder::Screenshots => {
                return Err(Error::new("Screenshots folder not supported on Cocoa"))
            }
            Folder::Templates => return Err(Error::new("Templates folder not supported on Cocoa")),
            Folder::Videos => NS_MOVIES_DIRECTORY,
        };

        match first_search_path(dir) {
            None => Err(Error::new("Directory not found")),
            Some(None) => Err(Error::new("Couldn't get folder path")),
            Some(Some(base)) => Ok(append_slash(&base)),
        }
    })
}

/// The `append_slash:` label: `"%s/"`, then `mkdir()` each parent.
#[cfg(not(target_os = "tvos"))]
fn append_slash(base: &str) -> String {
    let result = format!("{base}/");
    mkdir_parents(&result);
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::stdlib::getenv;

    #[test]
    fn cocoa_paths() {
        // Not bundled: the resource path is the executable's directory.
        let exe = std::env::current_exe().unwrap();
        let dir = exe.parent().unwrap().canonicalize().unwrap();
        let base = get_base_path().unwrap();
        assert!(base.ends_with('/'));
        assert_eq!(
            std::path::Path::new(&base).canonicalize().unwrap(),
            dir,
            "{base}"
        );
        // Not bundled: no Info.plist keys, so the process name.
        let name = get_exe_name().unwrap();
        assert!(
            exe.file_name()
                .unwrap()
                .to_string_lossy()
                .starts_with(&name),
            "{name}"
        );

        // (tvOS has none)
        #[cfg(target_os = "tvos")]
        for folder in [Folder::Home, Folder::Documents, Folder::SavedGames] {
            assert_eq!(
                get_user_folder(folder).unwrap_err().message(),
                "tvOS does not have persistent storage"
            );
        }
        #[cfg(not(target_os = "tvos"))]
        user_folders();
    }

    #[cfg(not(target_os = "tvos"))]
    fn user_folders() {
        let home = getenv("HOME").unwrap();
        assert_eq!(get_user_folder(Folder::Home).unwrap(), format!("{home}/"));
        assert_eq!(
            get_user_folder(Folder::Desktop).unwrap(),
            format!("{home}/Desktop/")
        );
        assert_eq!(
            get_user_folder(Folder::Videos).unwrap(),
            format!("{home}/Movies/")
        );
        assert_eq!(
            get_user_folder(Folder::PublicShare).unwrap(),
            format!("{home}/Public/")
        );
        assert_eq!(
            get_user_folder(Folder::SavedGames).unwrap_err().message(),
            "Saved games folder not supported on Cocoa"
        );
        assert_eq!(
            get_user_folder(Folder::Templates).unwrap_err().message(),
            "Templates folder not supported on Cocoa"
        );
    }

    #[test]
    fn cocoa_pref_path() {
        let home = getenv("HOME").unwrap();
        // (tvOS keeps it in the caches)
        let base = if cfg!(target_os = "tvos") {
            format!("{home}/Library/Caches")
        } else {
            format!("{home}/Library/Application Support")
        };
        let org = format!("SDL3 Test Org {}", std::process::id());
        let pref = get_pref_path(&org, "Test App").unwrap();
        assert_eq!(pref, format!("{base}/{org}/Test App/"));
        assert!(std::path::Path::new(&pref).is_dir());
        let pref2 = get_pref_path("", &org).unwrap();
        assert_eq!(pref2, format!("{base}/{org}/"));
        std::fs::remove_dir(&pref).unwrap();
        std::fs::remove_dir(&pref2).unwrap();
    }
}
