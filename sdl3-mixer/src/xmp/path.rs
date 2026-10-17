// Rust translation of src/path.c and src/path.h from libxmp 4.7.3 (as vendored by SDL_mixer).
// Extended Module Player
// Copyright (C) 1996-2026 Claudio Matsuoka and Hipolito Carraro Jr (path.c)
// Copyright (C) 1996-2025 Claudio Matsuoka and Hipolito Carraro Jr (path.h)
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! Building the paths of instrument files.

use super::common::strlen;

/// Translation of `struct libxmp_path`: `path` holds `length` bytes and a
/// NUL (`alloc` is the `Vec`'s).
#[derive(Clone, Debug, Default)]
pub(crate) struct LibxmpPath {
    pub path: Option<Vec<u8>>,
    pub length: usize,
}

/// Translation of `libxmp_path_init()`.
pub(crate) fn libxmp_path_init(p: &mut LibxmpPath) {
    *p = LibxmpPath::default();
}

/// Translation of `libxmp_path_free()`.
pub(crate) fn libxmp_path_free(p: &mut LibxmpPath) {
    p.path = None;
}

/// Translation of `libxmp_path_move()`.
pub(crate) fn libxmp_path_move(dest: &mut LibxmpPath, src: &mut LibxmpPath) {
    libxmp_path_free(dest);
    *dest = src.clone();
    src.path = None;
}

/// Translation of `fix_size()`.
fn fix_size(p: &mut LibxmpPath, sz: usize) -> i32 {
    let path = p.path.get_or_insert_with(Vec::new);
    if path.len() < sz {
        if path.try_reserve(sz - path.len()).is_err() {
            return -1;
        }
        path.resize(sz, 0);
    }
    0
}

/// Translation of `is_slash()`.
fn is_slash(c: u8) -> bool {
    c == b'\\' || c == b'/'
}

/* Some console SDKs (3DS, possibly others) handle duplicate and trailing
 * slashes very poorly. This function should be used to guarantee any path
 * created by these functions will not have these issues. */
/// Translation of `clean_slashes()`.
fn clean_slashes(p: &mut LibxmpPath, pos: usize) {
    if pos >= p.length {
        return;
    }
    let length = p.length;
    let path = p.path.as_mut().unwrap();

    /* Normalize to /, detect duplicates */
    let mut i = pos;
    while i < length {
        if is_slash(path[i]) {
            path[i] = b'/';
            if i + 1 < length && is_slash(path[i + 1]) {
                break;
            }
        }
        i += 1;
    }
    /* Clean duplicates if they exist */
    let mut j = i;
    while i < length {
        if is_slash(path[i]) {
            path[j] = b'/';
            j += 1;
            i += 1;
            while i < length && is_slash(path[i]) {
                i += 1;
            }
        } else {
            path[j] = path[i];
            j += 1;
            i += 1;
        }
    }
    /* Trim trailing slash, except at index 0 */
    if j > 1 && is_slash(path[j - 1]) {
        j -= 1;
    }
    path[j] = 0;
    p.length = j;
}

/// Translation of `libxmp_path_set()`.
pub(crate) fn libxmp_path_set(p: &mut LibxmpPath, new_path: Option<&[u8]>) -> i32 {
    let Some(new_path) = new_path else {
        return -1;
    };
    let len = strlen(new_path);
    let sz = len + 1;

    if fix_size(p, sz) < 0 {
        return -1;
    }
    let path = p.path.as_mut().unwrap();
    path[..len].copy_from_slice(&new_path[..len]);
    path[len] = 0;
    p.length = sz - 1;

    clean_slashes(p, 0);
    0
}

/// Translation of `libxmp_path_truncate()`.
pub(crate) fn libxmp_path_truncate(p: &mut LibxmpPath, new_sz: usize) -> i32 {
    if new_sz > p.length {
        return 0;
    }
    if fix_size(p, new_sz + 1) < 0 {
        return -1;
    }
    p.path.as_mut().unwrap()[new_sz] = 0;
    p.length = new_sz;
    if new_sz != 0 {
        clean_slashes(p, new_sz - 1);
    }
    0
}

/// Translation of `libxmp_path_suffix_at()`.
pub(crate) fn libxmp_path_suffix_at(p: &mut LibxmpPath, ext_pos: usize, ext: Option<&[u8]>) -> i32 {
    let Some(ext) = ext else {
        return -1;
    };
    if ext_pos > p.length {
        return -1;
    }
    let len = strlen(ext);
    let sz = len + 1;

    if fix_size(p, ext_pos + sz) < 0 {
        return -1;
    }
    let path = p.path.as_mut().unwrap();
    path[ext_pos..ext_pos + len].copy_from_slice(&ext[..len]);
    path[ext_pos + len] = 0;
    p.length = ext_pos + sz - 1;

    clean_slashes(p, if ext_pos != 0 { ext_pos - 1 } else { 0 });
    0
}

/// Translation of `libxmp_path_append()`.
pub(crate) fn libxmp_path_append(p: &mut LibxmpPath, append_path: Option<&[u8]>) -> i32 {
    let Some(append_path) = append_path else {
        return -1;
    };
    let len = strlen(append_path);
    let append_sz = len + 1;
    let new_sz = p.length + 1 + append_sz;
    let old_sz = p.length;

    if fix_size(p, new_sz) < 0 {
        return -1;
    }
    let path = p.path.as_mut().unwrap();
    path[p.length] = b'/';
    p.length += 1;
    path[p.length..p.length + len].copy_from_slice(&append_path[..len]);
    path[p.length + len] = 0;
    p.length = new_sz - 1;

    clean_slashes(p, if old_sz != 0 { old_sz - 1 } else { 0 });
    0
}

/// Translation of `libxmp_path_join()`.
pub(crate) fn libxmp_path_join(
    p: &mut LibxmpPath,
    prefix_path: Option<&[u8]>,
    suffix_path: Option<&[u8]>,
) -> i32 {
    let (Some(prefix_path), Some(suffix_path)) = (prefix_path, suffix_path) else {
        return -1;
    };

    let prefix = &prefix_path[..strlen(prefix_path)];
    let suffix = &suffix_path[..strlen(suffix_path)];

    if prefix.is_empty() {
        return libxmp_path_set(p, Some(suffix));
    }

    let ret = prefix.len() + 1 + suffix.len();
    if fix_size(p, ret + 1) < 0 {
        return -1;
    }
    let path = p.path.as_mut().unwrap();
    path[..prefix.len()].copy_from_slice(prefix);
    path[prefix.len()] = b'/';
    path[prefix.len() + 1..ret].copy_from_slice(suffix);
    path[ret] = 0;
    p.length = ret;

    clean_slashes(p, 0);
    0
}
