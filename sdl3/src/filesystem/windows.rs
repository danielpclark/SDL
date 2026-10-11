// Rust translation of src/filesystem/windows/SDL_sysfilesystem.c and
// src/filesystem/windows/SDL_sysfsops.c from Simple DirectMedia Layer.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! The Windows `SDL_SYS_*` path lookups (the module path,
//! `SHGetFolderPathW()`, `SHGetKnownFolderPath()`,
//! `GetCurrentDirectoryW()`) and filesystem operations
//! (`FindFirstFileExW()`, `GetFileAttributesExW()`, `MoveFileExW()`,
//! `CopyFileExW()`, ...).

use std::ops::ControlFlow;

use super::{EnumerationResult, Folder, PathInfo, PathType};
use crate::core::windows::{
    error_from_hresult, get_module_path, set_error, utf8_to_wide, wide_to_utf8,
};
use crate::error::{Error, Result};
use crate::time::Time;
use windows_sys::core::{GUID, HRESULT, PWSTR};
use windows_sys::Win32::Foundation::{
    FreeLibrary, GetLastError, ERROR_ALREADY_EXISTS, ERROR_FILE_NOT_FOUND, FILETIME, HANDLE,
    INVALID_HANDLE_VALUE, MAX_PATH,
};
use windows_sys::Win32::Storage::FileSystem::{
    CopyFileExW, CreateDirectoryW, DeleteFileW, FindClose, FindExInfoStandard,
    FindExSearchNameMatch, FindFirstFileExW, FindNextFileW, GetFileAttributesExW,
    GetFileExInfoStandard, GetLogicalDrives, MoveFileExW, RemoveDirectoryW,
    COPY_FILE_ALLOW_DECRYPTED_DESTINATION, FILE_ATTRIBUTE_DEVICE, FILE_ATTRIBUTE_DIRECTORY,
    FILE_ATTRIBUTE_OFFLINE, MOVEFILE_REPLACE_EXISTING, WIN32_FILE_ATTRIBUTE_DATA, WIN32_FIND_DATAW,
};
use windows_sys::Win32::System::Com::CoTaskMemFree;
use windows_sys::Win32::System::Environment::GetCurrentDirectoryW;
use windows_sys::Win32::System::LibraryLoader::{GetProcAddress, LoadLibraryW};
use windows_sys::Win32::UI::Shell::{
    SHGetFolderPathW, CSIDL_APPDATA, CSIDL_DESKTOP, CSIDL_FLAG_CREATE, CSIDL_MYDOCUMENTS,
    CSIDL_MYMUSIC, CSIDL_MYPICTURES, CSIDL_MYVIDEO, CSIDL_PROFILE, CSIDL_TEMPLATES,
    SHGFP_TYPE_CURRENT,
};

// * * * SDL_sysfilesystem.c

// These aren't all defined in older SDKs, so define them here
const SDL_FOLDERID_PROFILE: GUID = GUID::from_u128(0x5E6C858F_0E22_4760_9AFE_EA3317B67173);
const SDL_FOLDERID_DESKTOP: GUID = GUID::from_u128(0xB4BFCC3A_DB2C_424C_B029_7FE99A87C641);
const SDL_FOLDERID_DOCUMENTS: GUID = GUID::from_u128(0xFDD39AD0_238F_46AF_ADB4_6C85480369C7);
const SDL_FOLDERID_DOWNLOADS: GUID = GUID::from_u128(0x374de290_123f_4565_9164_39c4925e467b);
const SDL_FOLDERID_MUSIC: GUID = GUID::from_u128(0x4BD8D571_6D19_48D3_BE97_422220080E43);
const SDL_FOLDERID_PICTURES: GUID = GUID::from_u128(0x33E28130_4E1E_4676_835A_98395C3BC3BB);
const SDL_FOLDERID_SAVEDGAMES: GUID = GUID::from_u128(0x4c5c32ff_bb9d_43b0_b5b4_2d72e54eaaa4);
const SDL_FOLDERID_SCREENSHOTS: GUID = GUID::from_u128(0xb7bede81_df94_4682_a7d8_57a526208b6f);
const SDL_FOLDERID_TEMPLATES: GUID = GUID::from_u128(0xA63293E8_664E_48DB_A079_DF759E0509F7);
const SDL_FOLDERID_VIDEOS: GUID = GUID::from_u128(0x18989B1D_99B5_455B_841C_AB7C74E4DDFC);

/// The module path, up to the last `\`, and what follows it.
fn split_module_path() -> Result<(String, String)> {
    let path = get_module_path(std::ptr::null_mut())?; // look up full path of the current process's EXE file.
                                                       // Should have been an absolute path.
    let ptr = path
        .rfind('\\')
        .ok_or_else(|| Error::new("Couldn't locate our executable"))?;
    Ok((path[..=ptr].to_owned(), path[ptr + 1..].to_owned()))
}

/// Translation of `SDL_SYS_GetBasePath()` (Windows).
pub(super) fn get_base_path() -> Result<String> {
    let (dir, _) = split_module_path()?; // chop off filename, leave '\\'.
    Ok(dir)
}

/// Translation of `SDL_SYS_GetExeName()` (Windows).
pub(super) fn get_exe_name() -> Result<String> {
    let (_, name) = split_module_path()?;
    Ok(name)
}

/// The UTF-16 units of a NUL-terminated buffer, up to the NUL.
fn wcs(buf: &[u16]) -> &[u16] {
    &buf[..buf.iter().position(|&c| c == 0).unwrap_or(buf.len())]
}

/// `CreateDirectoryW()`, where an existing directory counts as success.
fn create_pref_directory(path: &[u16]) -> Result<()> {
    let mut wpath = path.to_vec();
    wpath.push(0);
    // SAFETY: wpath is NUL-terminated; no security attributes.
    if unsafe { CreateDirectoryW(wpath.as_ptr(), std::ptr::null()) } == 0 {
        // SAFETY: GetLastError has no preconditions.
        if unsafe { GetLastError() } != ERROR_ALREADY_EXISTS {
            return Err(set_error("Couldn't create a prefpath."));
        }
    }
    Ok(())
}

/// Translation of `SDL_SYS_GetPrefPath()` (Windows).
pub(super) fn get_pref_path(org: &str, app: &str) -> Result<String> {
    /*
     * Vista and later has a new API for this, but SHGetFolderPath works there,
     *  and apparently just wraps the new API. This is the new way to do it:
     *
     *     SHGetKnownFolderPath(SDL_FOLDERID_RoamingAppData, KF_FLAG_CREATE,
     *                          NULL, &wszPath);
     */

    let mut path = [0u16; MAX_PATH as usize];
    // SAFETY: path holds MAX_PATH units, as SHGetFolderPathW requires.
    let hr = unsafe {
        SHGetFolderPathW(
            std::ptr::null_mut(),
            (CSIDL_APPDATA | CSIDL_FLAG_CREATE) as i32,
            std::ptr::null_mut(),
            0,
            path.as_mut_ptr(),
        )
    };
    if hr < 0 {
        return Err(error_from_hresult(Some("Couldn't locate our prefpath"), hr));
    }

    let worg = utf8_to_wide(org);
    let wapp = utf8_to_wide(app);
    let (worg, wapp) = (wcs(&worg), wcs(&wapp));
    let mut path = wcs(&path).to_vec();

    let new_wpath_len = worg.len() + wapp.len() + path.len() + 3;

    if (new_wpath_len + 1) > MAX_PATH as usize {
        return Err(set_error("Path too long."));
    }

    if !worg.is_empty() {
        path.push(u16::from(b'\\'));
        path.extend_from_slice(worg);
    }

    create_pref_directory(&path)?;

    path.push(u16::from(b'\\'));
    path.extend_from_slice(wapp);

    create_pref_directory(&path)?;

    path.push(u16::from(b'\\'));

    Ok(wide_to_utf8(&path))
}

type PfnSHGetKnownFolderPath =
    unsafe extern "system" fn(*const GUID, u32, HANDLE, *mut PWSTR) -> HRESULT;

/// Translation of `SDL_SYS_GetUserFolder()` (Windows).
pub(super) fn get_user_folder(folder: Folder) -> Result<String> {
    let name = utf8_to_wide("Shell32.dll");
    // SAFETY: name is NUL-terminated.
    let lib = unsafe { LoadLibraryW(name.as_ptr()) };
    let mut p_sh_get_known_folder_path: Option<PfnSHGetKnownFolderPath> = None;

    if !lib.is_null() {
        // SAFETY: lib is loaded; the export has this signature.
        p_sh_get_known_folder_path = unsafe {
            GetProcAddress(lib, c"SHGetKnownFolderPath".as_ptr().cast()).map(|f| {
                std::mem::transmute::<unsafe extern "system" fn() -> isize, PfnSHGetKnownFolderPath>(
                    f,
                )
            })
        };
    }

    let result = (|| {
        let result = if let Some(sh_get_known_folder_path) = p_sh_get_known_folder_path {
            let r#type = match folder {
                Folder::Home => SDL_FOLDERID_PROFILE,
                Folder::Desktop => SDL_FOLDERID_DESKTOP,
                Folder::Documents => SDL_FOLDERID_DOCUMENTS,
                Folder::Downloads => SDL_FOLDERID_DOWNLOADS,
                Folder::Music => SDL_FOLDERID_MUSIC,
                Folder::Pictures => SDL_FOLDERID_PICTURES,
                Folder::PublicShare => {
                    return Err(Error::new("Public share unavailable on Windows"));
                }
                Folder::SavedGames => SDL_FOLDERID_SAVEDGAMES,
                Folder::Screenshots => SDL_FOLDERID_SCREENSHOTS,
                Folder::Templates => SDL_FOLDERID_TEMPLATES,
                Folder::Videos => SDL_FOLDERID_VIDEOS,
            };

            let mut path: PWSTR = std::ptr::null_mut();
            // SAFETY: valid GUID and out-pointer; 0x00008000 is KF_FLAG_CREATE.
            let hr = unsafe {
                sh_get_known_folder_path(&r#type, 0x00008000, std::ptr::null_mut(), &mut path)
            };
            if hr >= 0 {
                // SAFETY: on success path is a NUL-terminated string from
                // CoTaskMemAlloc().
                let s = unsafe {
                    let len = (0..).take_while(|&i| *path.add(i) != 0).count();
                    wide_to_utf8(std::slice::from_raw_parts(path, len))
                };
                // Note (upstream): upstream never frees `path`; it is
                // freed here, as SHGetKnownFolderPath() asks.
                // SAFETY: path came from SHGetKnownFolderPath().
                unsafe { CoTaskMemFree(path.cast()) };
                s
            } else {
                // SAFETY: CoTaskMemFree accepts NULL (and frees what a
                // failed call may still have allocated).
                unsafe { CoTaskMemFree(path.cast()) };
                return Err(error_from_hresult(Some("Couldn't get folder"), hr));
            }
        } else {
            let mut r#type = match folder {
                Folder::Home => CSIDL_PROFILE,
                Folder::Desktop => CSIDL_DESKTOP,
                Folder::Documents => CSIDL_MYDOCUMENTS,
                Folder::Downloads => {
                    return Err(Error::new("Downloads folder unavailable before Vista"));
                }
                Folder::Music => CSIDL_MYMUSIC,
                Folder::Pictures => CSIDL_MYPICTURES,
                Folder::PublicShare => {
                    return Err(Error::new("Public share unavailable on Windows"));
                }
                Folder::SavedGames => {
                    return Err(Error::new("Saved games unavailable before Vista"));
                }
                Folder::Screenshots => {
                    return Err(Error::new("Screenshots folder unavailable before Vista"));
                }
                Folder::Templates => CSIDL_TEMPLATES,
                Folder::Videos => CSIDL_MYVIDEO,
            };

            // Create the OS-specific folder if it doesn't already exist
            r#type |= CSIDL_FLAG_CREATE;

            /* Windows 2000/XP and later, deprecated as of Windows 10 (still
            available), available in Wine (tested 6.0.3) */
            let mut path = [0u16; MAX_PATH as usize];
            // SAFETY: path holds MAX_PATH units, as SHGetFolderPathW requires.
            let hr = unsafe {
                SHGetFolderPathW(
                    std::ptr::null_mut(),
                    r#type as i32,
                    std::ptr::null_mut(),
                    SHGFP_TYPE_CURRENT as u32,
                    path.as_mut_ptr(),
                )
            };

            // use `== TRUE` for SHGetSpecialFolderPath
            if hr >= 0 {
                wide_to_utf8(&path)
            } else {
                return Err(error_from_hresult(Some("Couldn't get folder"), hr));
            }
        };
        Ok(format!("{result}\\"))
    })();

    if !lib.is_null() {
        // SAFETY: balances the LoadLibraryW() above.
        unsafe { FreeLibrary(lib) };
    }
    result
}

/// Translation of `SDL_SYS_GetCurrentDirectory()` (Windows).
pub(super) fn get_current_directory() -> Result<String> {
    let mut wstr: Vec<u16> = Vec::new();
    let mut buflen: u32 = 0;
    loop {
        // SAFETY: wstr holds at least buflen units (or the pointer is
        // unused when buflen is 0).
        let bw = unsafe {
            GetCurrentDirectoryW(
                buflen,
                if wstr.is_empty() {
                    std::ptr::null_mut()
                } else {
                    wstr.as_mut_ptr()
                },
            )
        };
        if bw == 0 {
            return Err(set_error("GetCurrentDirectoryW failed"));
        } else if bw < buflen {
            // we got it!
            // make sure there's a path separator at the end.
            wstr.truncate(bw as usize);
            if bw == 0 || wstr[bw as usize - 1] != u16::from(b'\\') {
                wstr.push(u16::from(b'\\'));
            }
            break;
        }

        wstr = vec![0u16; bw as usize + 1];
        buflen = bw;
    }

    Ok(wide_to_utf8(&wstr))
}

// * * * SDL_sysfsops.c

const COPY_FILE_NO_BUFFERING: u32 = 0x00001000;

/// Translation of `SDL_SYS_EnumerateDirectory()` (Windows).
pub(super) fn enumerate_directory(
    path: &str,
    cb: &mut dyn FnMut(&str, &str) -> EnumerationResult,
) -> Result<()> {
    if path.is_empty() {
        // if empty (completely at the root), we need to enumerate drive letters.
        // SAFETY: GetLogicalDrives has no preconditions.
        let drives = unsafe { GetLogicalDrives() };
        for i in b'A'..=b'Z' {
            if drives & (1 << (i - b'A')) != 0 {
                let name = format!("{}:\\", i as char);
                if cb("", &name)?.is_break() {
                    break;
                }
            }
        }
    } else {
        // you need a wildcard to enumerate through FindFirstFileEx(), but the wildcard is only checked in the
        // filename element at the end of the path string, so always tack on a "\\*" to get everything, and
        // also prevent any wildcards inserted by the app from being respected.
        let pattern = format!("{path}\\\\"); // we'll replace that second '\\' in the trimdown.

        // trim down to a single path separator at the end, in case the caller added one or more.
        let bytes = pattern.as_bytes();
        let mut patternlen = bytes.len();
        while patternlen > 1 && matches!(bytes[patternlen - 1], b'\\' | b'/') {
            patternlen -= 1;
        }
        let dirname = format!("{}\\", &pattern[..patternlen]);
        let wpattern = utf8_to_wide(&format!("{dirname}*"));

        // (dirname is the pattern with the '*' chopped off, so we just have
        // the dirname with a path separator.)

        // SAFETY: WIN32_FIND_DATAW is plain data; all zeroes is a valid value.
        let mut entw: WIN32_FIND_DATAW = unsafe { std::mem::zeroed() };
        // SAFETY: wpattern is NUL-terminated; entw receives the first entry.
        let dir = unsafe {
            FindFirstFileExW(
                wpattern.as_ptr(),
                FindExInfoStandard,
                (&mut entw as *mut WIN32_FIND_DATAW).cast(),
                FindExSearchNameMatch,
                std::ptr::null(),
                0,
            )
        };
        if dir == INVALID_HANDLE_VALUE {
            return Err(set_error("Failed to enumerate directory"));
        }

        let mut result = Ok(ControlFlow::Continue(()));
        loop {
            let fn_ = wcs(&entw.cFileName);
            let dot = u16::from(b'.');

            // ignore "." and ".."
            if !(fn_ == [dot] || fn_ == [dot, dot]) {
                let utf8fn = wide_to_utf8(fn_);
                result = cb(&dirname, &utf8fn);
            }
            if !matches!(result, Ok(ControlFlow::Continue(()))) {
                break;
            }
            // SAFETY: dir is an open search handle; entw receives the entry.
            if unsafe { FindNextFileW(dir, &mut entw) } == 0 {
                break;
            }
        }

        // SAFETY: dir is an open search handle, closed once.
        unsafe { FindClose(dir) };
        let _ = result?; // (FAILURE from the callback)
    }

    Ok(())
}

/// `GetFileAttributesExW(path, GetFileExInfoStandard)`.
fn file_attributes(wpath: &[u16]) -> Option<WIN32_FILE_ATTRIBUTE_DATA> {
    // SAFETY: WIN32_FILE_ATTRIBUTE_DATA is plain data.
    let mut info: WIN32_FILE_ATTRIBUTE_DATA = unsafe { std::mem::zeroed() };
    // SAFETY: wpath is NUL-terminated; info receives the attributes.
    let ok = unsafe {
        GetFileAttributesExW(
            wpath.as_ptr(),
            GetFileExInfoStandard,
            (&mut info as *mut WIN32_FILE_ATTRIBUTE_DATA).cast(),
        )
    } != 0;
    ok.then_some(info)
}

/// Translation of `SDL_SYS_RemovePath()` (Windows).
pub(super) fn remove_path(path: &str) -> Result<()> {
    let wpath = utf8_to_wide(path);

    let Some(info) = file_attributes(&wpath) else {
        // SAFETY: GetLastError has no preconditions.
        if unsafe { GetLastError() } == ERROR_FILE_NOT_FOUND {
            // Note that ERROR_PATH_NOT_FOUND means a parent dir is missing, and we consider that an error.
            return Ok(()); // thing is already gone, call it a success.
        }
        return Err(set_error("Couldn't get path's attributes"));
    };

    let isdir = info.dwFileAttributes & FILE_ATTRIBUTE_DIRECTORY != 0;
    // SAFETY: wpath is NUL-terminated.
    let rc = unsafe {
        if isdir {
            RemoveDirectoryW(wpath.as_ptr())
        } else {
            DeleteFileW(wpath.as_ptr())
        }
    };
    if rc == 0 {
        return Err(set_error("Couldn't remove path"));
    }
    Ok(())
}

/// Translation of `SDL_SYS_RenamePath()` (Windows).
pub(super) fn rename_path(oldpath: &str, newpath: &str) -> Result<()> {
    let woldpath = utf8_to_wide(oldpath);
    let wnewpath = utf8_to_wide(newpath);

    // SAFETY: both paths are NUL-terminated.
    let rc = unsafe {
        MoveFileExW(
            woldpath.as_ptr(),
            wnewpath.as_ptr(),
            MOVEFILE_REPLACE_EXISTING,
        )
    };
    if rc == 0 {
        return Err(set_error("Couldn't rename path"));
    }
    Ok(())
}

/// Translation of `SDL_SYS_CopyFile()` (Windows).
pub(super) fn copy_file(oldpath: &str, newpath: &str) -> Result<()> {
    let woldpath = utf8_to_wide(oldpath);
    let wnewpath = utf8_to_wide(newpath);

    // SAFETY: both paths are NUL-terminated; no progress routine or cancel flag.
    let rc = unsafe {
        CopyFileExW(
            woldpath.as_ptr(),
            wnewpath.as_ptr(),
            None,
            std::ptr::null(),
            std::ptr::null_mut(),
            COPY_FILE_ALLOW_DECRYPTED_DESTINATION | COPY_FILE_NO_BUFFERING,
        )
    };
    if rc == 0 {
        return Err(set_error("Couldn't copy path"));
    }
    Ok(())
}

/// Translation of `SDL_SYS_CreateDirectory()` (Windows).
pub(super) fn create_directory(path: &str) -> Result<()> {
    let wpath = utf8_to_wide(path);

    // SAFETY: wpath is NUL-terminated; no security attributes.
    let mut rc = unsafe { CreateDirectoryW(wpath.as_ptr(), std::ptr::null()) } != 0;
    // SAFETY: GetLastError has no preconditions.
    if !rc && unsafe { GetLastError() } == ERROR_ALREADY_EXISTS {
        if let Some(winstat) = file_attributes(&wpath) {
            if winstat.dwFileAttributes & FILE_ATTRIBUTE_DIRECTORY != 0 {
                rc = true; // exists and is already a directory: cool.
            }
        }
    }

    if !rc {
        return Err(set_error("Couldn't create directory"));
    }
    Ok(())
}

/// `SDL_TimeFromWindows()` of a `FILETIME`.
fn time_from_filetime(ft: &FILETIME) -> Time {
    Time::from_windows_filetime(((ft.dwHighDateTime as u64) << 32) | ft.dwLowDateTime as u64)
}

/// Translation of `SDL_SYS_GetPathInfo()` (Windows).
pub(super) fn get_path_info(path: &str) -> Result<PathInfo> {
    let wpath = utf8_to_wide(path);

    let Some(winstat) = file_attributes(&wpath) else {
        return Err(set_error("Can't stat"));
    };

    let size = ((winstat.nFileSizeHigh as u64) << 32) | winstat.nFileSizeLow as u64;
    let (kind, size) = if winstat.dwFileAttributes & FILE_ATTRIBUTE_DIRECTORY != 0 {
        (PathType::Directory, 0)
    } else if winstat.dwFileAttributes & (FILE_ATTRIBUTE_OFFLINE | FILE_ATTRIBUTE_DEVICE) != 0 {
        (PathType::Other, size)
    } else {
        (PathType::File, size)
    };

    Ok(PathInfo {
        kind,
        size,
        create_time: time_from_filetime(&winstat.ftCreationTime),
        modify_time: time_from_filetime(&winstat.ftLastWriteTime),
        access_time: time_from_filetime(&winstat.ftLastAccessTime),
    })
}
