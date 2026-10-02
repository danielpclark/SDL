// Rust translation of src/filesystem/SDL_filesystem.c from Simple DirectMedia Layer.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! Filesystem paths and operations. Translation of `SDL_filesystem.h`.
//!
//! Paths are UTF-8 strings, as in C SDL; `/` works as a separator
//! everywhere. File names that are not valid UTF-8 are converted lossily
//! when they are reported (by [`enumerate_directory`] and [`glob_directory`]).

mod fsops;
mod sys;

use std::ops::ControlFlow;
use std::sync::Mutex;

use crate::error::Result;
use crate::stdlib::string::{case_fold_unicode, codepoints};
use crate::time::Time;

/// The type of the OS-provided default folder for a specific purpose.
/// Translation of `SDL_Folder`.
///
/// Note that the Trash folder isn't included here, because trashing files
/// usually involves extra OS-specific functionality to remember the file's
/// original location.
///
/// The folders supported per platform are:
///
/// |             | Windows | macOS/iOS | tvOS | Unix (XDG) | Haiku | Emscripten |
/// | ----------- | ------- | --------- | ---- | ---------- | ----- | ---------- |
/// | HOME        | X       | X         |      | X          | X     | X          |
/// | DESKTOP     | X       | X         |      | X          | X     |            |
/// | DOCUMENTS   | X       | X         |      | X          |       |            |
/// | DOWNLOADS   | Vista+  | X         |      | X          |       |            |
/// | MUSIC       | X       | X         |      | X          |       |            |
/// | PICTURES    | X       | X         |      | X          |       |            |
/// | PUBLICSHARE |         | X         |      | X          |       |            |
/// | SAVEDGAMES  | Vista+  |           |      |            |       |            |
/// | SCREENSHOTS | Vista+  |           |      |            |       |            |
/// | TEMPLATES   | X       | X         |      | X          |       |            |
/// | VIDEOS      | X       | X*        |      | X          |       |            |
///
/// Note that on macOS/iOS, the Videos folder is called "Movies".
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Folder {
    /// The folder which contains all of the current user's data, preferences, and documents. It usually contains most of the other folders. If a requested folder does not exist, the home folder can be considered a safe fallback to store a user's documents.
    Home,
    /// The folder of files that are displayed on the desktop. Note that the existence of a desktop folder does not guarantee that the system does show icons on its desktop; certain GNU/Linux distros with a graphical environment may not have desktop icons.
    Desktop,
    /// User document files, possibly application-specific. This is a good place to save a user's projects.
    Documents,
    /// Standard folder for user files downloaded from the internet.
    Downloads,
    /// Music files that can be played using a standard music player (mp3, ogg...).
    Music,
    /// Image files that can be displayed using a standard viewer (png, jpg...).
    Pictures,
    /// Files that are meant to be shared with other users on the same computer.
    PublicShare,
    /// Save files for games.
    SavedGames,
    /// Application screenshots.
    Screenshots,
    /// Template files to be used when the user requests the desktop environment to create a new file in a certain folder, such as "New Text File.txt".  Any file in the Templates folder can be used as a starting point for a new file.
    Templates,
    /// Video files that can be played using a standard video player (mp4, webm...).
    Videos,
}

impl Folder {
    /// Total number of types in this enum. Translation of `SDL_FOLDER_COUNT`.
    pub const COUNT: usize = 11;

    /// Every folder, in declaration order.
    pub const ALL: [Folder; Folder::COUNT] = [
        Folder::Home,
        Folder::Desktop,
        Folder::Documents,
        Folder::Downloads,
        Folder::Music,
        Folder::Pictures,
        Folder::PublicShare,
        Folder::SavedGames,
        Folder::Screenshots,
        Folder::Templates,
        Folder::Videos,
    ];
}

/// Types of filesystem entries. Translation of `SDL_PathType`.
///
/// Note that there may be other sorts of items on a filesystem: devices,
/// symlinks, named pipes, etc. They are currently reported as
/// [`PathType::Other`].
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Default)]
pub enum PathType {
    /// path does not exist
    #[default]
    None,
    /// a normal file
    File,
    /// a directory
    Directory,
    /// something completely different like a device node (not a symlink, those are always followed)
    Other,
}

/// Information about a path on the filesystem. Translation of `SDL_PathInfo`.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Default)]
pub struct PathInfo {
    /// the path type
    pub kind: PathType,
    /// the file size in bytes
    pub size: u64,
    /// the time when the path was created
    pub create_time: Time,
    /// the last time the path was modified
    pub modify_time: Time,
    /// the last time the path was read
    pub access_time: Time,
}

/// Flags for path matching. Translation of `SDL_GlobFlags`.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Default)]
pub struct GlobFlags(pub u32);

impl GlobFlags {
    /// No flags.
    pub const NONE: GlobFlags = GlobFlags(0);
    /// Translation of `SDL_GLOB_CASEINSENSITIVE`.
    pub const CASE_INSENSITIVE: GlobFlags = GlobFlags(1 << 0);

    /// Whether every flag in `other` is set.
    pub const fn contains(self, other: GlobFlags) -> bool {
        self.0 & other.0 == other.0
    }
}

impl std::ops::BitOr for GlobFlags {
    type Output = GlobFlags;
    fn bitor(self, rhs: GlobFlags) -> GlobFlags {
        GlobFlags(self.0 | rhs.0)
    }
}

/// What a directory enumeration callback returns. Translation of
/// `SDL_EnumerationResult`: `Ok(Continue)` is `SDL_ENUM_CONTINUE`,
/// `Ok(Break)` is `SDL_ENUM_SUCCESS`, and `Err` is `SDL_ENUM_FAILURE`.
pub type EnumerationResult = Result<ControlFlow<()>>;

/// Remove a file or an empty directory. Removing a path that doesn't exist
/// succeeds. Translation of `SDL_RemovePath()`.
pub fn remove_path(path: &str) -> Result<()> {
    fsops::remove_path(path)
}

/// Rename a file or directory. Translation of `SDL_RenamePath()`.
///
/// If the file at `newpath` already exists, it will be replaced.
///
/// Note that this will not copy files across filesystems/drives/volumes, as
/// that is a much more complicated (and possibly time-consuming) operation.
///
/// Which is to say, if this function fails, [`copy_file`] to a temporary
/// file in the same directory as `newpath`, then [`rename_path`] from the
/// temporary file to `newpath` and [`remove_path`] on `oldpath` might work
/// for files. Renaming a non-empty directory across filesystems is
/// dramatically more complex, however.
pub fn rename_path(oldpath: &str, newpath: &str) -> Result<()> {
    fsops::rename_path(oldpath, newpath)
}

/// Copy a file. Translation of `SDL_CopyFile()`.
///
/// If the file at `newpath` already exists, it will be overwritten with the
/// contents of the file at `oldpath`.
///
/// This function will block until the copy is complete, which might be a
/// significant time for large files on slow disks. On some platforms, the
/// copy can be handed off to the OS itself, but on others SDL might just
/// open both paths, and read from one and write to the other.
///
/// Note that this is not an atomic operation! If something tries to read
/// from `newpath` while the copy is in progress, it will see an incomplete
/// copy of the data, and if the calling thread terminates (or the power goes
/// out) during the copy, `newpath`'s previous contents will be gone,
/// replaced with an incomplete copy of the data. To avoid this risk, it is
/// recommended that the app copy to a temporary file in the same directory
/// as `newpath`, and if the copy is successful, use [`rename_path`] to
/// replace `newpath` with the temporary file. This will ensure that reads of
/// `newpath` will either see a complete copy of the data, or it will see the
/// pre-copy state of `newpath`.
///
/// This function attempts to synchronize the newly-copied data to disk
/// before returning, if the platform allows it, so that the renaming trick
/// will not have a problem in a system crash or power failure, where the
/// file could be renamed but the contents never made it from the system
/// file cache to the physical disk.
///
/// If the copy fails for any reason, the state of `newpath` is undefined. It
/// might be half a copy, it might be the untouched data of what was already
/// there, or it might be a zero-byte file, etc.
pub fn copy_file(oldpath: &str, newpath: &str) -> Result<()> {
    fsops::copy_file(oldpath, newpath)
}

/// Create a directory, and any missing parent directories.
/// Translation of `SDL_CreateDirectory()`.
///
/// This reports success if `path` already exists as a directory.
///
/// If parent directories are missing, it will also create them. Note that if
/// this fails, it will not remove any parent directories it already made.
pub fn create_directory(path: &str) -> Result<()> {
    let mut retval = fsops::create_directory(path);
    if retval.is_err() && !path.is_empty() {
        // maybe we're missing parent directories?
        let mut parents = path.as_bytes().to_vec();

        // in case there was a separator at the end of the path and it was
        // upsetting something, chop it off.
        let slen = parents.len();
        let last = parents[slen - 1];
        if last == b'/' || (cfg!(windows) && last == b'\\') {
            parents.truncate(slen - 1);
            retval = fsops::create_directory(&String::from_utf8_lossy(&parents));
        }

        if retval.is_err() {
            for i in 0..parents.len() {
                let ch = parents[i];
                let issep = if cfg!(windows) {
                    let issep = ch == b'/' || ch == b'\\';
                    if issep && i == 2 && parents[1] == b':' {
                        continue; // it's just the drive letter, skip it.
                    }
                    issep
                } else {
                    let issep = ch == b'/';
                    if issep && i == 0 {
                        continue; // it's just the root directory, skip it.
                    }
                    issep
                };

                if issep {
                    // (this does not fail if the path already exists as a directory.)
                    retval = fsops::create_directory(&String::from_utf8_lossy(&parents[..i]));
                    if retval.is_err() {
                        // still failing when making parents? Give up.
                        break;
                    }
                }
            }

            // last chance: did it work this time?
            retval = fsops::create_directory(&String::from_utf8_lossy(&parents));
        }
    }
    retval
}

/// Enumerate a directory through a callback function.
/// Translation of `SDL_EnumerateDirectory()`.
///
/// This function provides every directory entry through an app-provided
/// callback, called once for each directory entry, until all results have
/// been provided or the callback stops.
///
/// The callback receives the directory being enumerated (with a trailing
/// path separator) and the name of the entry. It returns
/// `Ok(ControlFlow::Continue(()))` to keep going, `Ok(ControlFlow::Break(()))`
/// to stop successfully, or an error to stop and make this function fail
/// with it.
pub fn enumerate_directory(
    path: &str,
    mut callback: impl FnMut(&str, &str) -> EnumerationResult,
) -> Result<()> {
    fsops::enumerate_directory(path, &mut callback)
}

/// Get information about a filesystem path. Translation of `SDL_GetPathInfo()`.
pub fn get_path_info(path: &str) -> Result<PathInfo> {
    fsops::get_path_info(path)
}

/// Translation of `EverythingMatch()`.
fn everything_match(_pattern: &[u8], _str: &[u8], matched_to_dir: &mut bool) -> bool {
    *matched_to_dir = true;
    true // everything matches!
}

/// Translation of `WildcardMatch()`: this is just '*' and '?', with '/'
/// matching nothing.
fn wildcard_match(pattern: &[u8], s: &[u8], matched_to_dir: &mut bool) -> bool {
    // The C version walks NUL-terminated strings; reading past the end
    // here yields that NUL.
    let at = |b: &[u8], i: usize| b.get(i).copied().unwrap_or(0);

    let mut str_i = 0usize;
    let mut pat_i = 0usize;
    let mut str_backtrack: Option<usize> = None;
    let mut pattern_backtrack: Option<usize> = None;
    let mut sch_backtrack: u8 = 0;
    let mut sch = at(s, str_i);
    let mut pch = at(pattern, pat_i);

    while sch != 0 {
        if pch == b'*' {
            str_backtrack = Some(str_i);
            pat_i += 1;
            pattern_backtrack = Some(pat_i);
            sch_backtrack = sch;
            pch = at(pattern, pat_i);
        } else if pch == sch {
            if pch == b'/' {
                str_backtrack = None;
                pattern_backtrack = None;
            }
            str_i += 1;
            sch = at(s, str_i);
            pat_i += 1;
            pch = at(pattern, pat_i);
        } else if pch == b'?' && sch != b'/' {
            // end of string (checked at `while`) or path separator do not match '?'.
            str_i += 1;
            sch = at(s, str_i);
            pat_i += 1;
            pch = at(pattern, pat_i);
        } else if pattern_backtrack.is_none() || sch_backtrack == b'/' {
            // we didn't have a match. Are we in a '*' and NOT on a path separator? Keep going. Otherwise, fail.
            *matched_to_dir = false;
            return false;
        } else {
            // still here? Wasn't a match, but we're definitely in a '*' pattern.
            // (`str_backtrack` is always set along with `pattern_backtrack`.)
            let sb = str_backtrack.map_or(0, |b| b + 1);
            str_backtrack = Some(sb);
            str_i = sb;
            pat_i = pattern_backtrack.unwrap_or(0);
            sch_backtrack = sch;
            sch = at(s, str_i);
            pch = at(pattern, pat_i);
        }

        if cfg!(windows) && sch == b'\\' {
            sch = b'/';
        }
    }

    // '*' at the end can be ignored, they are allowed to match nothing.
    while pch == b'*' {
        pat_i += 1;
        pch = at(pattern, pat_i);
    }

    *matched_to_dir = pch == b'/'; // end of string and the pattern failed at a '/'? We should descend into this directory.

    pch == 0 // survived the whole pattern? That's a match!
}

/// Translation of `EncodeCodepointToUtf8()`.
///
/// Note that this will currently encode illegal codepoints: UTF-16
/// surrogates, 0xFFFE, and 0xFFFF, and a codepoint > 0x10FFFF will be
/// dropped. clean this up if you want to move this to SDL_string.c.
fn encode_codepoint_to_utf8(out: &mut Vec<u8>, cp: u32) {
    if cp < 0x80 {
        // fits in a single UTF-8 byte.
        out.push(cp as u8);
    } else if cp < 0x800 {
        // fits in 2 bytes.
        out.push(((cp >> 6) | 128 | 64) as u8);
        out.push(((cp & 0x3F) | 128) as u8);
    } else if cp < 0x10000 {
        // fits in 3 bytes.
        out.push(((cp >> 12) | 128 | 64 | 32) as u8);
        out.push((((cp >> 6) & 0x3F) | 128) as u8);
        out.push(((cp & 0x3F) | 128) as u8);
    } else if cp <= 0x10FFFF {
        // fits in 4 bytes.
        out.push(((cp >> 18) | 128 | 64 | 32 | 16) as u8);
        out.push((((cp >> 12) & 0x3F) | 128) as u8);
        out.push((((cp >> 6) & 0x3F) | 128) as u8);
        out.push(((cp & 0x3F) | 128) as u8);
    }
}

/// Translation of `CaseFoldUtf8String()`.
fn case_fold_utf8_string(fname: &[u8]) -> Vec<u8> {
    let mut result = Vec::with_capacity(fname.len());
    for codepoint in codepoints(fname) {
        for &folded in case_fold_unicode(codepoint).iter() {
            encode_codepoint_to_utf8(&mut result, folded);
        }
    }
    result
}

type Matcher = fn(&[u8], &[u8], &mut bool) -> bool;

/// A directory walker for [`internal_glob_directory`]: enumerate `path`,
/// calling the callback for each entry (`SDL_GlobEnumeratorFunc`).
pub(crate) type GlobEnumerator<'a> =
    &'a dyn Fn(&str, &mut dyn FnMut(&str, &str) -> EnumerationResult) -> Result<()>;
/// Path info for [`internal_glob_directory`] (`SDL_GlobGetPathInfoFunc`).
pub(crate) type GlobGetPathInfo<'a> = &'a dyn Fn(&str) -> Result<PathInfo>;

/// Translation of `GlobDirCallbackData`.
struct GlobDirCallbackData<'a> {
    matcher: Matcher,
    pattern: &'a [u8],
    flags: GlobFlags,
    enumerator: GlobEnumerator<'a>,
    getpathinfo: GlobGetPathInfo<'a>,
    basedirlen: usize,
    results: Vec<String>,
}

impl GlobDirCallbackData<'_> {
    /// Translation of `GlobDirectoryCallback()`.
    fn callback(&mut self, dirname: &str, fname: &str) -> EnumerationResult {
        // !!! FIXME: if we're careful, we can keep a single buffer in `data` that we push and pop paths off the end of as we walk the tree,
        // !!! FIXME: and only casefold the new pieces instead of allocating and folding full paths for all of this.

        let fullpath = format!("{dirname}{fname}");

        let folded = if self.flags.contains(GlobFlags::CASE_INSENSITIVE) {
            Some(case_fold_utf8_string(fullpath.as_bytes()))
        } else {
            None
        };

        let mut matched_to_dir = false;
        let candidate = folded.as_deref().unwrap_or(fullpath.as_bytes());
        let candidate = candidate.get(self.basedirlen..).unwrap_or(&[]);
        let matched = (self.matcher)(self.pattern, candidate, &mut matched_to_dir);

        if matched {
            let subpath = fullpath.as_bytes().get(self.basedirlen..).unwrap_or(&[]);
            self.results
                .push(String::from_utf8_lossy(subpath).into_owned());
        }

        // keep enumerating by default.
        if matched_to_dir {
            if let Ok(info) = (self.getpathinfo)(&fullpath) {
                if info.kind == PathType::Directory {
                    let enumerator = self.enumerator;
                    enumerator(&fullpath, &mut |d, f| self.callback(d, f))?;
                }
            }
        }

        Ok(ControlFlow::Continue(()))
    }
}

/// Translation of `SDL_InternalGlobDirectory()`, shared with storage.
pub(crate) fn internal_glob_directory(
    path: &str,
    pattern: Option<&str>,
    mut flags: GlobFlags,
    enumerator: GlobEnumerator<'_>,
    getpathinfo: GlobGetPathInfo<'_>,
) -> Result<Vec<String>> {
    let mut path = path;

    // if path ends with any slash, chop them off, so we don't confuse the pattern matcher later.
    if path.len() > 1 && (path.ends_with('/') || path.ends_with('\\')) {
        let trimmed = path.trim_end_matches(['/', '\\']);
        // (never chop the first character)
        path = if trimmed.is_empty() {
            &path[..1]
        } else {
            trimmed
        };
    }
    let pathlen = path.len();

    if pattern.is_none() {
        flags = GlobFlags(flags.0 & !GlobFlags::CASE_INSENSITIVE.0); // avoid some unnecessary allocations and work later.
    }

    let folded = match pattern {
        Some(p) if flags.contains(GlobFlags::CASE_INSENSITIVE) => {
            Some(case_fold_utf8_string(p.as_bytes()))
        }
        _ => None,
    };

    let matcher: Matcher = match pattern {
        None => everything_match, // no pattern? Everything matches.

        // !!! FIXME
        //} else if (flags & SDL_GLOB_GITIGNORE) {
        //    data.matcher = GitIgnoreMatch;
        Some(_) => wildcard_match,
    };

    let mut basedirlen = 0;
    if !path.is_empty() {
        if path == "/" || path == "\\" {
            basedirlen = 1;
        } else {
            basedirlen = pathlen + 1; // +1 for the '/' we'll be adding.
        }
    }

    let mut data = GlobDirCallbackData {
        matcher,
        pattern: folded
            .as_deref()
            .unwrap_or_else(|| pattern.map_or(&[][..], str::as_bytes)),
        flags,
        enumerator,
        getpathinfo,
        basedirlen,
        results: Vec::new(),
    };

    enumerator(path, &mut |d, f| data.callback(d, f))?;
    Ok(data.results)
}

/// Enumerate a directory tree, filtered by pattern, and return a list.
/// Translation of `SDL_GlobDirectory()`.
///
/// Files are filtered out if they don't match the string in `pattern`,
/// which may contain wildcard characters `*` (match everything) and `?`
/// (match one character). If pattern is `None`, no filtering is done and all
/// results are returned. Subdirectories are permitted, and are specified
/// with a path separator of `/`. Wildcard characters `*` and `?` never match
/// a path separator.
///
/// `flags` may be set to [`GlobFlags::CASE_INSENSITIVE`] to make the
/// pattern matching case-insensitive.
///
/// The returned paths are relative to `path`.
pub fn glob_directory(path: &str, pattern: Option<&str>, flags: GlobFlags) -> Result<Vec<String>> {
    internal_glob_directory(
        path,
        pattern,
        flags,
        &|p, cb| fsops::enumerate_directory(p, cb),
        &get_path_info,
    )
}

/// Translation of `CachedBasePath`, `CachedUserFolders` and `CachedExeName`.
struct Cache {
    base_path: Option<String>,
    user_folders: [Option<String>; Folder::COUNT],
    exe_name: Option<String>,
}

static CACHE: Mutex<Cache> = Mutex::new(Cache {
    base_path: None,
    user_folders: [const { None }; Folder::COUNT],
    exe_name: None,
});

fn cache() -> std::sync::MutexGuard<'static, Cache> {
    CACHE.lock().unwrap_or_else(|e| e.into_inner())
}

/// Get the directory where the application was run from.
/// Translation of `SDL_GetBasePath()`.
///
/// SDL caches the result of this call internally, but the first call to this
/// function is not necessarily fast, so plan accordingly.
///
/// **macOS and iOS Specific Functionality**: upstream returns the bundle's
/// resource directory there; this translation uses the executable's
/// directory on every platform.
///
/// The returned path is guaranteed to end with a path separator ('\\' on
/// Windows, '/' on most other platforms).
pub fn get_base_path() -> Result<String> {
    let mut cache = cache();
    if cache.base_path.is_none() {
        cache.base_path = Some(sys::get_base_path()?);
    }
    Ok(cache.base_path.clone().unwrap_or_default())
}

/// Get the user-and-app-specific path where files can be written.
/// Translation of `SDL_GetPrefPath()`.
///
/// Get the "pref dir". This is meant to be where users can write personal
/// files (preferences and save games, etc) that are specific to your
/// application. This directory is unique per user, per application.
///
/// This function will decide the appropriate location in the native
/// filesystem, create the directory if necessary, and return a string of
/// the absolute path to the directory in UTF-8 encoding.
///
/// On Windows, the string might look like:
///
/// `C:\Users\bob\AppData\Roaming\My Company\My Program Name\`
///
/// On Linux, the string might look like:
///
/// `/home/bob/.local/share/My Program Name/`
///
/// You should assume the path returned by this function is the only safe
/// place to write files (and that [`get_base_path`], while it might be
/// writable, or even the parent of the returned path, isn't where you
/// should be writing things).
///
/// Both the org and app strings may become part of a directory name, so
/// please follow these rules:
///
/// - Try to use the same org string (_including case-sensitivity_) for all
///   your applications that use this function.
/// - Always use a unique app string for each one, and make sure it never
///   changes for an app once you've decided on it.
/// - Unicode characters are legal, as long as they are UTF-8 encoded, but...
/// - ...only use letters, numbers, and spaces. Avoid punctuation like "Game
///   Name 2: Bad Guy's Revenge!" ... "Game Name 2" is sufficient.
///
/// The returned path is guaranteed to end with a path separator ('\\' on
/// Windows, '/' on most other platforms).
pub fn get_pref_path(org: Option<&str>, app: &str) -> Result<String> {
    // if org is NULL, just make it "" so backends don't have to check both.
    sys::get_pref_path(org.unwrap_or(""), app)
}

/// Finds the most suitable user folder for a specific purpose.
/// Translation of `SDL_GetUserFolder()`.
///
/// Many OSes provide certain standard folders for certain purposes, such as
/// storing pictures, music or videos for a certain user. This function gives
/// the path for many of those special locations.
///
/// This function is specifically for _user_ folders, which are meant for the
/// user to access and manage. For application-specific folders, meant to
/// hold data for the application to manage, see [`get_base_path`] and
/// [`get_pref_path`].
///
/// The returned path is guaranteed to end with a path separator ('\\' on
/// Windows, '/' on most other platforms).
///
/// If the folder doesn't exist or the OS doesn't support it, this returns
/// an error.
pub fn get_user_folder(folder: Folder) -> Result<String> {
    let idx = folder as usize;
    let mut cache = cache();
    if cache.user_folders[idx].is_none() {
        cache.user_folders[idx] = Some(sys::get_user_folder(folder)?);
    }
    Ok(cache.user_folders[idx].clone().unwrap_or_default())
}

/// The file name of the running executable. Translation of `SDL_GetExeName()`.
#[allow(dead_code)] // used by the platform layers that report process names
pub(crate) fn get_exe_name() -> Result<String> {
    let mut cache = cache();
    if cache.exe_name.is_none() {
        cache.exe_name = Some(sys::get_exe_name()?);
    }
    Ok(cache.exe_name.clone().unwrap_or_default())
}

/// Get what the system believes is the "current working directory."
/// Translation of `SDL_GetCurrentDirectory()`.
///
/// For systems without a concept of a current working directory, this will
/// still attempt to provide something reasonable.
///
/// The returned path is guaranteed to end with a path separator ('\\' on
/// Windows, '/' on most other platforms).
pub fn get_current_directory() -> Result<String> {
    fsops::get_current_directory()
}

/// Translation of `SDL_InitFilesystem()`.
pub(crate) fn init_filesystem() {}

/// Translation of `SDL_QuitFilesystem()`.
pub(crate) fn quit_filesystem() {
    let mut cache = cache();
    cache.base_path = None;
    cache.exe_name = None;
    for folder in cache.user_folders.iter_mut() {
        *folder = None;
    }
}

#[cfg(test)]
mod tests;
