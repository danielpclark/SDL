// Rust translation of src/core/android/SDL_android.c from Simple
// DirectMedia Layer (part: the APK file tree and the assets' file I/O
// through AAssetManager).
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! The app package's assets (its `assets/` directory): opened and read
//! through the NDK's `AAssetManager` (`Android_JNI_FileOpen()` and the
//! stream functions, [`AssetStream`]: `SDL_IOFromFile()`'s last resort on
//! Android), and enumerated through a tree of the package's directories
//! and files that the package (a zip file) itself gives, from its central
//! directory: Android's AAssetManager can enumerate all files in any known
//! directory, but it won't enumerate subdirectories, so the tree is upstream's
//! own ("APK file tree discovery", `SDL_EnumerateDirectory()` and
//! `SDL_GetPathInfo()` on `assets://` paths).
//!
//! A program that isn't an app (spawned from the shell) has no Java VM and
//! so no package: its asset manager can't be made (opening an asset fails),
//! and its tree is empty. Note: upstream dereferences a null environment
//! there.

use std::ffi::CString;
use std::ops::ControlFlow;
use std::ptr::NonNull;
use std::sync::{Mutex, MutexGuard};

use super::env::{self, Global, LocalReferenceHolder, Value};
use super::ndk::{self, AAsset, AAssetManager, AASSET_MODE_UNKNOWN};
use crate::error::{Error, Result};
use crate::filesystem::{EnumerationResult, PathInfo, PathType};
use crate::io::{IoInterface, IoStatus, IoStop, IoStream, IoWhence};
use crate::properties::Properties;
use crate::time::Time;

// ---------------------------------------------------------------------------
// APK file tree discovery...

// APK files are just .zip files, so try to find parse out the file tree from it. We'll still
// use Android's system APIs to actually access data, but those APIs aren't reliable for
// enumerating the tree for various reasons.

// we only care about the directories; Android's AAssetManager can enumerate all files in any
// known directory, but it won't enumerate subdirectories, so we track that by ourselves.

/// A node of the tree. Translation of `APKNode` (its children and
/// siblings as indices of the tree's nodes, in the order they were added).
#[derive(Debug)]
struct ApkNode {
    name: String,
    info: PathInfo,
    parent: Option<usize>,
    children: Vec<usize>,
}

/// The tree (`APKRootNode` and the nodes under it): node 0 is the root.
#[derive(Debug)]
struct ApkTree {
    nodes: Vec<ApkNode>,
}

const ROOT: usize = 0;

/// `APKRootNode`: none until [`prepare_apk`] has run.
static APK_ROOT: Mutex<Option<ApkTree>> = Mutex::new(None);

fn apk_root() -> MutexGuard<'static, Option<ApkTree>> {
    APK_ROOT.lock().unwrap_or_else(|e| e.into_inner())
}

impl ApkTree {
    /// Translation of `FindAPKChildNode()`.
    fn find_child_node(&self, parent: usize, child: &str) -> Option<usize> {
        if child == "." {
            // "." paths just return the current dir.
            return Some(parent);
        } else if child == ".." {
            // ".." paths return the parent dir (or the current dir if at the root).
            return Some(self.nodes[parent].parent.unwrap_or(parent));
        }

        self.nodes[parent]
            .children
            .iter()
            .copied()
            .find(|&node| self.nodes[node].name == child)
    }

    /// The node of a path (`assets://` and its slashes dropped). Translation
    /// of `FindAPKNode()`, whose errors (`No such file or directory`, `%s is
    /// not a directory`) its callers replace with theirs.
    fn find_node(&self, constpath: &str) -> Option<usize> {
        //SDL_Log("FindAPKNode('%s') ...", constpath);

        let constpath = constpath.strip_prefix("assets://").unwrap_or(constpath);

        let mut parent = ROOT;
        if constpath.is_empty() {
            return Some(parent);
        }

        let mut path = constpath;
        loop {
            path = path.trim_start_matches('/'); // just in case there are absolute paths or double-slashes, drop them.

            if path.is_empty() {
                // ended with a '/'? We're done.
                break;
            }

            // (the subdir's name, up to the next '/')
            let (name, rest) = match path.split_once('/') {
                Some((name, rest)) => (name, Some(rest)),
                None => (path, None),
            };

            let node = self.find_child_node(parent, name)?; // ("No such file or directory")
            if self.nodes[node].info.kind == PathType::File && rest.is_some() {
                // file where we want a directory? ("%s is not a directory")
                return None;
            }
            parent = node;
            match rest {
                None => break,
                Some(rest) => path = rest,
            }
        }

        Some(parent)
    }

    /// Translation of `AddAPKChildNode()`.
    fn add_child_node(&mut self, parent: usize, child: &str) -> usize {
        if let Some(node) = self.find_child_node(parent, child) {
            return node;
        }
        // don't have this one yet, make a new node.
        let node = self.nodes.len();
        self.nodes.push(ApkNode {
            name: child.to_owned(),
            info: self.nodes[parent].info, // you probably need to update this afterwards.
            parent: Some(parent),
            children: Vec::new(),
        });
        // APKs tend to be sorted alphabetically, so insert new nodes at the end of the list to maintain this order.
        self.nodes[parent].children.push(node);
        node
    }

    /// The node of the directory of `path`'s last segment (its parent
    /// directories made as needed). Translation of `AddAPKDirs()`.
    fn add_dirs(&mut self, path: &str, mut parent: usize) -> Result<usize> {
        // zip files specify explicit directories by just having a path that ends with a dir separator,
        // which works nicely for our needs here; if the last segment of the path doesn't end with a
        // '/', it's a file and we can drop it, or we filled in the final subdirectory and the '/' at
        // the end will put us at an empty string to be dropped.
        //
        // directories do not need to be explicitly specified if something uses some deeper path, they
        // may need to be inferred from those references, so we build out the tree by looking at all
        // files and filling in nodes they mention.

        crate::sdl_assert!(self.nodes[parent].info.kind == PathType::Directory);
        crate::sdl_assert!(self.nodes[parent].info.size == 0);

        let mut path = path;
        loop {
            // while still subdirectories to handle...
            path = path.trim_start_matches('/'); // just in case there are absolute paths or double-slashes, drop them.

            let Some((name, rest)) = path.split_once('/') else {
                break; // last thing is either an empty string (we ended with a '/'), or an actual file's name, so drop it.
            };

            // FIXME (upstream): the check of "." and ".." looks at the bytes
            // after the '/' too (the segment isn't cut there yet): "./" passes,
            // and so does "..x/"'s segment's sibling ".x" when the next byte is
            // one; any segment that starts with ".." (as "..x") fails.
            let b = path.as_bytes();
            let at = |i: usize| b.get(i).copied().unwrap_or(0);
            if at(0) == b'.' && (at(1) == 0 || at(1) == b'.' || at(2) == 0) {
                // whoa, there's a "." or ".." subdir in a zip entry's path? Fail!
                return Err(Error::new("bogus file path in APK file entry"));
            }

            parent = self.add_child_node(parent, name);
            path = rest; // point to start of next section.
        }

        Ok(parent)
    }
}

/// Note that DOS-style datetimes in a zip don't have timezone info, etc,
/// so these are treated as UTC for lack of a better option. Translation of
/// `ZipDosTimeToSDLTime()`. FIXME (upstream): it counts the day of the
/// month from 1 one too many (one day late), and gives seconds where
/// `SDL_Time` is nanoseconds.
fn zip_dos_time_to_sdl_time(dostime: u32) -> Time {
    // Note that DOS-style datetimes in a zip don't have timezone info, etc,
    // so these are treated as UTC for lack of a better option. Also, they have
    // 2-second precision and the year bits are going to roll over in 2108.
    // There are possibly better timestamps in extended fields (!!! FIXME: add
    // support for those), but this field is guaranteed to exist with all its
    // flaws and is probably Good Enough anyhow.
    let dosdate = (dostime >> 16) & 0xFFFF;
    let dostime16 = dostime & 0xFFFF;

    let m = ((dosdate >> 5) & 0x0F) as i32;
    let d = ((dosdate) & 0x1F) as i32 + 1;
    let y = (((dosdate >> 9) & 0x7F) + 1980) as i32 - if m <= 2 { 1 } else { 0 };
    let hour = ((dostime16 >> 11) & 0x1F) as i32;
    let minute = ((dostime16 >> 5) & 0x3F) as i32;
    let sec = ((dostime16 << 1) & 0x3E) as i32;

    // days since 1/1/1970: https://howardhinnant.github.io/date_algorithms.html#days_from_civil
    let era = (if y >= 0 { y } else { y - 399 }) / 400;
    let yoe = (y - era * 400) as u32; // [0, 399]
    let doy = ((153u32
        .wrapping_mul(if m > 2 {
            (m - 3) as u32
        } else {
            (m + 9) as u32
        })
        .wrapping_add(2))
        / 5)
    .wrapping_add((d - 1) as u32); // [0, 365]
    let doe = (yoe * 365) + (yoe / 4) - (yoe / 100) + doy; // [0, 146096]
    let days = (era * 146097) + (doe as i32) - 719468;
    let seconds = (hour * (60 * 60)) + (minute * 60) + (sec);
    Time::from_nanos((days as i64) * 86400 + seconds as i64)
}

const ZIP_CENTRAL_DIR_SIG: u32 = 0x02014b50;
#[allow(dead_code)] // (upstream defines it; the record is found by its place)
const ZIP_END_OF_CENTRAL_DIR_SIG: u32 = 0x06054b50;
const ZIP64_END_OF_CENTRAL_DIR_SIG: u32 = 0x06064b50;
const ZIP64_END_OF_CENTRAL_DIRECTORY_LOCATOR_SIG: u32 = 0x07064b50;
const ZIP64_EXTENDED_INFO_EXTRA_FIELD_SIG: u16 = 0x0001;

/// How [`process_zip`] stops early: `corrupterr`, `ioerr`, or with its
/// own message logged.
enum ZipFailure {
    Corrupt,
    Io(Error),
    Logged,
}

impl From<Error> for ZipFailure {
    fn from(e: Error) -> ZipFailure {
        ZipFailure::Io(e)
    }
}

/// `SDL_SeekIO()`, failing below 0.
fn seek(
    io: &mut IoStream<'_>,
    offset: i64,
    whence: IoWhence,
) -> std::result::Result<i64, ZipFailure> {
    Ok(io.seek(offset, whence)?)
}

/// The package's entries under `assets/`, added to the tree. Translation
/// of `ProcessZip()`, whose failures are logged (`Filesystem enumeration
/// will fail.`): the entries added until then stay.
fn process_zip(io: &mut IoStream<'_>, root: &mut ApkTree) -> bool {
    match process_zip_entries(io, root) {
        Ok(()) => true,
        Err(ZipFailure::Corrupt) => {
            // corrupterr:
            crate::log!(
                "ANDROID: Unexpected or corrupt data in APK. Filesystem enumeration will fail."
            );
            false
        }
        Err(ZipFailure::Logged) => false,
        Err(ZipFailure::Io(e)) => {
            // ioerr:
            crate::log!(
                "ANDROID: i/o error in APK ({}). Filesystem enumeration will fail.",
                e.message()
            );
            false
        }
    }
}

fn process_zip_entries(
    io: &mut IoStream<'_>,
    root: &mut ApkTree,
) -> std::result::Result<(), ZipFailure> {
    // There is a record at the end of a .zip file we can use to find the central directory;
    // unfortunately it's before the variable-length comment field, so we might need to read
    // from the end of the file until we see a magic signature. For now, we assume APKs don't
    // have a comment, so we don't have to search backwards for the signature and it's always
    // 22 bytes from EOF.
    let size = io.size();
    let eocd = size.as_ref().map_or(-1, |s| *s) - 22;
    if eocd < 0 {
        crate::log!(
            "ANDROID: Couldn't find End Of Central Directory in APK ({}). Filesystem enumeration will fail.",
            size.err().map_or_else(String::new, |e| e.message().to_owned())
        );
        return Err(ZipFailure::Logged);
    }

    let zip64;
    let centraldir: i64;
    let num_entries: u64;

    // First, check if this is actually zip64 format instead. The zip64 magic is 20 bytes back.
    if eocd < 20 {
        // presumably we always _are_ > 20, but let's be defensive here.
        return Err(ZipFailure::Corrupt);
    }
    seek(io, eocd - 20, IoWhence::Set)?;
    let val32 = io.read_u32_le()?;
    if val32 == ZIP64_END_OF_CENTRAL_DIRECTORY_LOCATOR_SIG {
        // this is a zip64 archive?
        let val32 = io.read_u32_le()?; // disk number with start of central directory.
        if val32 != 0 {
            return Err(ZipFailure::Corrupt);
        }
        let val64 = io.read_u64_le()?; // file offset of zip64 end-of-central-dir record

        // note that this gets significantly more complex if there is data prepended to the .zip file
        //  (like a self-extracting .exe, etc), but until that happens, we're keeping this as simple
        //  as possible and assuming the file offset in val64 is correct.

        seek(io, val64 as i64, IoWhence::Set)?;
        let val32 = io.read_u32_le()?; // zip64 end-of-central-dir signature.
        if val32 != ZIP64_END_OF_CENTRAL_DIR_SIG {
            return Err(ZipFailure::Corrupt);
        }
        seek(io, 28, IoWhence::Cur)?; // we don't care about several of the fields, skip over them.
        num_entries = io.read_u64_le()?; // total entries in the central dir.
        let _ = io.read_u64_le()?; // size of the central dir.
        let val64 = io.read_u64_le()?; // offset of the central dir.

        zip64 = true;
        centraldir = val64 as i64;
    } else {
        seek(io, eocd + 4 + 6, IoWhence::Set)?; // skip back to where we were, plus skip some fields we don't care about.
        let val16 = io.read_u16_le()?;
        let _ = io.read_u32_le()?; // size of the central dir.
        let val32 = io.read_u32_le()?; // offset of the central dir.
        zip64 = false;
        num_entries = val16 as u64;
        centraldir = val32 as i64;
    }

    // okay, we know where the central dir is now, go there and start reading entries.
    crate::sdl_assert!(centraldir > 0); // negative means we failed, zero is impossible since there should be something else there.
    let mut centraldir = centraldir;

    for _ in 0..num_entries {
        // we don't care about most of this information, just parse through it to get what we need.
        seek(io, centraldir, IoWhence::Set)?;
        if io.read_u32_le()? != ZIP_CENTRAL_DIR_SIG {
            // central dir item signature.
            return Err(ZipFailure::Corrupt);
        }
        let _ = io.read_u16_le()?; // version made by
        let _ = io.read_u16_le()?; // version needed
        let _ = io.read_u16_le()?; // general bits
        let _ = io.read_u16_le()?; // compression method
        let dosmodtime = io.read_u32_le()?; // last mod date/time
        let _ = io.read_u32_le()?; // CRC-32
        let _ = io.read_u32_le()?; // compressed size
        let uncompressed32 = io.read_u32_le()?; // uncompressed size
        let fnamelen = io.read_u16_le()?; // filename length
        let extralen = io.read_u16_le()?; // extra length
        let commentlen = io.read_u16_le()?; // comment length
        let _ = io.read_u16_le()?; // disk number start
        let _ = io.read_u16_le()?; // internal file attributes
        let _ = io.read_u32_le()?; // external file attributes
        let _ = io.read_u32_le()?; // relative offset of local header

        let mut fnamebuf = vec![0u8; fnamelen as usize]; // just eat 64k of stack like a boss until someone complains.
        if io.read(&mut fnamebuf) != fnamelen as usize {
            return Err(ZipFailure::Io(
                io.last_error().cloned().unwrap_or_else(|| Error::new("")),
            ));
        }

        // technically zip files might have '\\' dir separators, but these were mostly old DOS files and not Android APKs, I think. Revisit if necessary.

        // make sure the string is null-terminated. (a NUL in the name ends it, as C reads it)
        if let Some(nul) = fnamebuf.iter().position(|&b| b == 0) {
            fnamebuf.truncate(nul);
        }
        let fname = String::from_utf8_lossy(&fnamebuf).into_owned();

        //SDL_Log("ANDROID: Saw ZIP entry '%s'", fnamebuf);

        let mut uncompressed64 = uncompressed32 as u64;
        if zip64 && uncompressed32 == 0xFFFFFFFF {
            // file is larger than 4gig, find the zip64 extended info field in the extra section.
            let mut found = false;
            let mut remaining = extralen;
            while remaining > 4 {
                // Two 16-bit values at a minimum, tag and len.
                let tag = io.read_u16_le()?;
                let len = io.read_u16_le()?;
                if (remaining as u32) < (len as u32 + 4) {
                    return Err(ZipFailure::Corrupt);
                } else if tag != ZIP64_EXTENDED_INFO_EXTRA_FIELD_SIG {
                    // not the field we need, skip over it.
                    seek(io, len as i64, IoWhence::Cur)?;
                } else if len < 8 {
                    return Err(ZipFailure::Corrupt);
                } else {
                    uncompressed64 = io.read_u64_le()?;

                    // there are other values in here, but we don't care about them and we're done, so don't try to skip over them.
                    found = true;
                    break; // got what we need, drop out.
                }
                remaining -= len + 4;
            }

            if !found {
                return Err(ZipFailure::Corrupt);
            }
        }

        let ptr = fname.trim_start_matches('/'); // drop absolute paths.

        if ptr.starts_with("assets/") {
            //  we only care about things under 'assets' for now. Drop everything else.
            let path = &ptr[6..]; // keep the '/' so strrchr never returns NULL.
            let node = root.add_dirs(path, ROOT).map_err(ZipFailure::Io)?; // this builds out any missing subdirs, returns parent dir's node.

            let modtime = zip_dos_time_to_sdl_time(dosmodtime);
            let last = &path[path.rfind('/').map_or(0, |i| i + 1)..];
            let node = if last.is_empty() {
                // explicit directory entry paths end with '/' ...`node` is the new node.
                let info = &mut root.nodes[node].info;
                info.kind = PathType::Directory;
                info.size = 0;
                node
            } else {
                let node = root.add_child_node(node, last);
                let info = &mut root.nodes[node].info;
                info.kind = PathType::File;
                info.size = uncompressed64;
                node
            };
            let info = &mut root.nodes[node].info;
            info.create_time = modtime;
            info.modify_time = modtime;
            info.access_time = modtime;
        }

        centraldir += 46 + fnamelen as i64 + extralen as i64 + commentlen as i64;
        // will seek to next file entry.
    }

    Ok(())
}

/// Translation of `CreateAPKNodes()`.
fn create_apk_nodes(path: &str, root: &mut ApkTree) -> bool {
    //SDL_Log("ANDROID: Parsing APK file '%s' ...", path);

    match IoStream::from_file(path, "rb") {
        Err(e) => {
            crate::log!(
                "ANDROID: Can't open APK '{}' for reading ({}). Filesystem enumeration will fail.",
                path,
                e.message()
            );
        }
        Ok(mut io) => {
            process_zip(&mut io, root);
            let _ = io.close();
        }
    }
    true
}

/// An empty tree: the root alone, a directory.
fn empty_tree() -> ApkTree {
    ApkTree {
        nodes: vec![ApkNode {
            name: String::new(),
            info: PathInfo {
                kind: PathType::Directory,
                ..PathInfo::default()
            },
            parent: None,
            children: Vec::new(),
        }],
    }
}

/// The tree, parsed from the app's package the first time (with the
/// package's file times for the root). Translation of `PrepareAPK()`: when
/// the package's path can't be had (a Java exception), the tree stays
/// empty and the first call fails (Note (upstream): without an error of
/// its own, the last one set standing; here its message is empty). The
/// empty root is there while the tree is made (as upstream allocates it
/// first), unlocked: the package's path info, which may look at the
/// assets, finds it. Note: a program that isn't an app (no Java VM, where
/// upstream dereferences a null environment) gets an empty tree.
fn prepare_apk() -> Result<MutexGuard<'static, Option<ApkTree>>> {
    // the assetmanager isn't useful for enumerating directories, so parse the APK directly for that info upfront.
    {
        let mut guard = apk_root();
        if guard.is_some() {
            return Ok(guard);
        }
        // allocate this upfront, so if there's a failure, we'll not try again and just have an empty file tree.
        *guard = Some(empty_tree());
    }
    let mut root = empty_tree();

    let mut retval = false;
    if let Some(env) = env::get_env() {
        let mut refs = LocalReferenceHolder::setup("PrepareAPK");
        if let Ok(env) = refs.init(env) {
            let path = super::java::activity::get_context(env)
                .ok_or(())
                .and_then(|context| {
                    env.call_method(
                        context.obj(),
                        c"getPackageResourcePath",
                        c"()Ljava/lang/String;",
                        &[],
                    )
                    .map_err(|thrown| thrown.clear()) // oh well
                })
                .map(Value::object);
            if let Ok(Some(jstr)) = path {
                if let Ok(apkpath) = env.get_string(jstr.obj()) {
                    crate::sdl_assert!(apkpath.starts_with('/')); // So SDL_GetPathInfo goes through the `stat` path and doesn't try to dig into the APK.
                    if let Ok(apkinfo) = crate::filesystem::get_path_info(&apkpath) {
                        // we just want the file times here, so oh well if it fails.
                        let info = &mut root.nodes[ROOT].info;
                        info.create_time = apkinfo.create_time;
                        info.modify_time = apkinfo.modify_time;
                        info.access_time = apkinfo.access_time;
                    }
                    create_apk_nodes(&apkpath, &mut root);
                    retval = true;
                }
            }
        }
    } else {
        retval = true; // (no app: no package, an empty tree)
    }
    let mut guard = apk_root();
    *guard = Some(root);
    if retval {
        Ok(guard)
    } else {
        // even on failure, leave an empty root node so we have zero files and don't try to load the .zip again.
        Err(Error::new(""))
    }
}

// ---------------------------------------------------------------------------
// The asset manager

/// The Java `AssetManager` (`javaAssetManagerRef`, a global reference that
/// keeps it) and its native `AAssetManager` (`asset_manager`).
struct AssetManager {
    _java: Global,
    native: NonNull<AAssetManager>,
}

// SAFETY: an AAssetManager may be used from any thread; the global
// reference too.
unsafe impl Send for AssetManager {}

static ASSET_MANAGER: Mutex<Option<AssetManager>> = Mutex::new(None);

fn asset_manager() -> MutexGuard<'static, Option<AssetManager>> {
    ASSET_MANAGER.lock().unwrap_or_else(|e| e.into_inner())
}

/// Translation of `Internal_Android_Create_AssetManager()`.
fn create_asset_manager(manager: &mut Option<AssetManager>) {
    let Some(env) = env::get_env() else {
        return;
    };
    let mut refs = LocalReferenceHolder::setup("Internal_Android_Create_AssetManager");
    let Ok(env) = refs.init(env) else {
        return;
    };

    // context = SDLActivity.getContext();
    let Some(context) = super::java::activity::get_context(env) else {
        // (Note (upstream): a null context's GetObjectClass() is an error
        // that ART aborts the process on)
        return;
    };

    // javaAssetManager = context.getAssets();
    let java_asset_manager = match env.call_method(
        context.obj(),
        c"getAssets",
        c"()Landroid/content/res/AssetManager;",
        &[],
    ) {
        Ok(manager) => manager.object(),
        Err(thrown) => {
            drop(thrown);
            let _ = env::exception_occurred(env, true);
            return;
        }
    };
    let Some(java_asset_manager) = java_asset_manager else {
        return;
    };

    /*
     * Given a Dalvik AssetManager object, obtain the corresponding native AAssetManager
     * object.  Note that the caller is responsible for obtaining and holding a VM reference
     * to the jobject to prevent its being garbage collected while the native object is
     * in use.
     */
    let Some(java_asset_manager_ref) = Global::new(env, java_asset_manager.obj()) else {
        return;
    };
    // SAFETY: a valid environment and a global reference to an AssetManager.
    let native =
        unsafe { ndk::AAssetManager_fromJava(env.raw(), java_asset_manager_ref.obj().raw()) };

    match NonNull::new(native) {
        Some(native) => {
            *manager = Some(AssetManager {
                _java: java_asset_manager_ref,
                native,
            });
        }
        None => {
            drop(java_asset_manager_ref);
            let _ = env::exception_occurred(env, true);
        }
    }
}

/// The asset manager and the tree, gone (`nativeQuit()`). Translation of
/// `Internal_Android_Destroy_AssetManager()`.
pub(crate) fn destroy_asset_manager() {
    *asset_manager() = None;
    *apk_root() = None;
}

/// A path of the asset system: a leading `./` (or a lone `.`) or `assets://`
/// dropped, then any slashes. Translation of `GetAssetPath()`.
fn get_asset_path(path: &str) -> &str {
    let path = if path == "." || path.starts_with("./") {
        &path[1..]
    } else if let Some(rest) = path.strip_prefix("assets://") {
        rest
    } else {
        path
    };
    path.trim_start_matches('/')
}

// ---------------------------------------------------------------------------
// The assets' file I/O

/// An asset opened for reading: `SDL_IOFromFile()`'s stream of an asset,
/// whose functions are `Android_JNI_FileSize()`, `_FileSeek()`,
/// `_FileRead()`, `_FileWrite()` and `_FileClose()`.
#[derive(Debug)]
pub(crate) struct AssetStream {
    asset: Option<NonNull<AAsset>>,
}

// SAFETY: the asset is this stream's only (AAsset isn't thread-safe, but
// it's used by one thread at a time, as the stream is).
unsafe impl Send for AssetStream {}

/// `SDL_PROP_IOSTREAM_ANDROID_AASSET_POINTER`.
pub(crate) use crate::io::PROP_IOSTREAM_ANDROID_AASSET_POINTER;

impl AssetStream {
    fn asset(&self) -> *mut AAsset {
        self.asset.expect("an open asset").as_ptr()
    }
}

impl IoInterface for AssetStream {
    fn has_size(&self) -> bool {
        true
    }

    /// Translation of `Android_JNI_FileSize()`.
    fn size(&mut self) -> Result<i64> {
        // SAFETY: an open asset.
        Ok(unsafe { ndk::AAsset_getLength64(self.asset()) })
    }

    /// Translation of `Android_JNI_FileSeek()`. Note (upstream): a seek
    /// that fails (out of the asset) is -1 without an error set; here the
    /// error's message is empty.
    fn seek(&mut self, offset: i64, whence: IoWhence) -> Result<i64> {
        let whence = match whence {
            IoWhence::Set => libc::SEEK_SET,
            IoWhence::Cur => libc::SEEK_CUR,
            IoWhence::End => libc::SEEK_END,
        };
        // SAFETY: an open asset.
        let pos = unsafe { ndk::AAsset_seek64(self.asset(), offset, whence) };
        if pos < 0 {
            return Err(Error::new(""));
        }
        Ok(pos)
    }

    fn can_read(&self) -> bool {
        true
    }

    /// Translation of `Android_JNI_FileRead()`.
    fn read(&mut self, buf: &mut [u8]) -> std::result::Result<usize, IoStop> {
        let size = buf.len();
        // SAFETY: an open asset, and room for `size` bytes.
        let bytes = unsafe { ndk::AAsset_read(self.asset(), buf.as_mut_ptr().cast(), size) };
        if bytes < 0 {
            return Err(IoStop::error(0, Error::new("AAsset_read() failed")));
        } else if (bytes as usize) < size {
            return Err(IoStop::status(bytes as usize, IoStatus::Eof));
        }
        Ok(bytes as usize)
    }

    fn can_write(&self) -> bool {
        true
    }

    /// Translation of `Android_JNI_FileWrite()`.
    fn write(&mut self, _buf: &[u8]) -> std::result::Result<usize, IoStop> {
        Err(IoStop::error(
            0,
            Error::new("Cannot write to Android package filesystem"),
        ))
    }

    /// Translation of `Android_JNI_FileClose()`.
    fn close(&mut self) -> Result<()> {
        if let Some(asset) = self.asset.take() {
            // SAFETY: an open asset, closed once.
            unsafe { ndk::AAsset_close(asset.as_ptr()) };
        }
        Ok(())
    }

    /// `android_setioprops()`: the asset (`AAsset *`).
    fn set_properties(&mut self, props: &Properties) -> Result<()> {
        let asset = self.asset.map_or(0, |a| a.as_ptr() as usize as i64);
        props.set(PROP_IOSTREAM_ANDROID_AASSET_POINTER, asset)
    }
}

impl Drop for AssetStream {
    fn drop(&mut self) {
        let _ = IoInterface::close(self);
    }
}

/// Open an asset (`assets://`, a leading `./` and slashes dropped from
/// `file_name`), whatever the mode. Translation of `Android_JNI_FileOpen()`.
pub(crate) fn file_open(file_name: &str, _mode: &str) -> Result<AssetStream> {
    let mut manager = asset_manager();
    if manager.is_none() {
        create_asset_manager(&mut manager);
        if manager.is_none() {
            return Err(Error::new("Couldn't create asset manager"));
        }
    }
    let native = manager.as_ref().expect("an asset manager").native;

    let file_name = get_asset_path(file_name);

    // (as a C string: a NUL ends it)
    let c_name = CString::new(file_name.split('\0').next().unwrap_or("")).unwrap_or_default();
    // SAFETY: an asset manager and a C string.
    let asset =
        unsafe { ndk::AAssetManager_open(native.as_ptr(), c_name.as_ptr(), AASSET_MODE_UNKNOWN) };
    let Some(asset) = NonNull::new(asset) else {
        return Err(Error::new(format!("Couldn't open asset '{file_name}'")));
    };

    Ok(AssetStream { asset: Some(asset) })
}

/// Enumerate an asset directory (`path`'s node in the tree), the callback
/// given `path` as it is (with its separator at the end) and each entry's
/// name, in the package's order. Translation of
/// `Android_JNI_EnumerateAssetDirectory()`.
pub(crate) fn enumerate_asset_directory(
    path: &str,
    cb: &mut dyn FnMut(&str, &str) -> EnumerationResult,
) -> Result<()> {
    let children: Vec<String> = {
        let guard = prepare_apk()?;
        let tree = guard.as_ref().expect("the tree, prepared");

        let asset_path = get_asset_path(path);

        let Some(apknode) = tree.find_node(asset_path) else {
            return Err(Error::new("No such directory"));
        };
        if tree.nodes[apknode].info.kind != PathType::Directory {
            return Err(Error::new("Not a directory"));
        }
        // (the names, so that the callback runs without the tree locked: it
        // may ask for an asset's path info, as a glob does)
        tree.nodes[apknode]
            .children
            .iter()
            .map(|&node| tree.nodes[node].name.clone())
            .collect()
    };
    for name in &children {
        match cb(path, name)? {
            ControlFlow::Continue(()) => {}
            ControlFlow::Break(()) => break,
        }
    }
    Ok(())
}

/// An asset's (file or directory's) information. Translation of
/// `Android_JNI_GetAssetPathInfo()`.
pub(crate) fn get_asset_path_info(path: &str) -> Result<PathInfo> {
    let guard = prepare_apk()?;
    let tree = guard.as_ref().expect("the tree, prepared");

    let path = get_asset_path(path);
    match tree.find_node(path) {
        Some(apknode) => Ok(tree.nodes[apknode].info),
        None => Err(Error::new("No such file or directory")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn asset_paths() {
        assert_eq!(get_asset_path("assets://a/b"), "a/b");
        assert_eq!(get_asset_path("assets:///a"), "a");
        assert_eq!(get_asset_path("./a"), "a");
        assert_eq!(get_asset_path("."), "");
        assert_eq!(get_asset_path(".a"), ".a");
        assert_eq!(get_asset_path("//x"), "x");
    }

    #[test]
    fn dos_times() {
        // (upstream's: one day late, in seconds) 1981-01-01 01:01:02
        let dos = ((1u32 << 9) | (1 << 5) | 1) << 16 | (1 << 11) | (1 << 5) | 1;
        assert_eq!(
            zip_dos_time_to_sdl_time(dos).as_nanos(),
            347_158_862 + 86400
        );
    }

    #[test]
    fn tree() {
        let mut tree = empty_tree();
        for path in ["/a/b/c.txt", "/a/d.txt", "/e/", "/f.bin"] {
            let dir = tree.add_dirs(path, ROOT).unwrap();
            let last = &path[path.rfind('/').unwrap() + 1..];
            if !last.is_empty() {
                let node = tree.add_child_node(dir, last);
                tree.nodes[node].info.kind = PathType::File;
            }
        }
        let names = |n: usize| -> Vec<&str> {
            tree.nodes[n]
                .children
                .iter()
                .map(|&c| tree.nodes[c].name.as_str())
                .collect()
        };
        assert_eq!(names(ROOT), ["a", "e", "f.bin"]);
        let a = tree.find_node("assets://a").unwrap();
        assert_eq!(names(a), ["b", "d.txt"]);
        assert_eq!(tree.find_node("a/b/../d.txt"), tree.find_node("a/d.txt"));
        assert_eq!(tree.find_node("a//b/./"), tree.find_node("a/b"));
        assert_eq!(tree.find_node("../a"), Some(a));
        assert!(tree.find_node("a/d.txt/x").is_none());
        assert!(tree.find_node("nope").is_none());
        assert_eq!(tree.find_node(""), Some(ROOT));
        assert!(tree.add_dirs("/../x/y", ROOT).is_err());
        assert!(tree.add_dirs("/..x/y", ROOT).is_err());
        assert!(tree.add_dirs("/./x", ROOT).is_ok()); // (FIXME (upstream))
    }
}
