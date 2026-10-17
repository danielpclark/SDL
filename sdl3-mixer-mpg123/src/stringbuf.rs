/*
    stringbuf: mimicking a bit of C++ to more safely handle strings

    copyright 2006-20 by the mpg123 project
        - free software under the terms of the LGPL 2.1
    see COPYING and AUTHORS files in distribution or http://mpg123.org
    initially written by Thomas Orgis
*/
// Modified 2026-10-17: translated into Rust from stringbuf.c in SDL_mixer 3.3.0's libmpg123 1.33.7 (see NOTICE).

//! `mpg123_string`, the string type of the ID3 data: the functions of
//! `stringbuf.c` that the ID3 parser uses. (`mpg123_new_string()` and
//! `mpg123_delete_string()` are `Default` and `Drop`; the others are only
//! used by `mpg123_store_utf8()` and the API for callers.)

/// Translation of `mpg123_string`: a byte buffer (`p`, whose length is
/// `size`) of which `fill` bytes are used, including the closing zero.
#[derive(Clone, Debug, Default)]
pub struct Mpg123String {
    pub(crate) p: Vec<u8>,
    pub(crate) fill: usize,
}

impl Mpg123String {
    /// The used bytes (`p[0..fill]`, including the closing zero).
    pub fn bytes(&self) -> &[u8] {
        &self.p[..self.fill]
    }

    /// `fill`: the number of used bytes, including the closing zero.
    pub fn fill(&self) -> usize {
        self.fill
    }

    /// `size`: the number of bytes allocated.
    pub fn size(&self) -> usize {
        self.p.len()
    }
}

/// Translation of `mpg123_init_string()`.
pub(crate) fn mpg123_init_string(sb: &mut Mpg123String) {
    /* Handing in NULL here is a fatal mistake and rightfully so. */
    sb.p = Vec::new();
    sb.fill = 0;
}

/// Translation of `mpg123_free_string()`.
pub(crate) fn mpg123_free_string(sb: &mut Mpg123String) {
    mpg123_init_string(sb);
}

/// Translation of `mpg123_grow_string()`.
pub(crate) fn mpg123_grow_string(sb: &mut Mpg123String, new: usize) -> bool {
    if sb.p.len() < new {
        mpg123_resize_string(sb, new)
    } else {
        true
    }
}

/// Translation of `mpg123_resize_string()`.
pub(crate) fn mpg123_resize_string(sb: &mut Mpg123String, new: usize) -> bool {
    if new == 0 {
        mpg123_init_string(sb);
        return true;
    }
    if sb.p.len() != new {
        // (realloc(): the new size, keeping what fits)
        if new > sb.p.len() && sb.p.try_reserve_exact(new - sb.p.len()).is_err() {
            return false;
        }
        sb.p.resize(new, 0);
        if sb.p.len() < sb.fill {
            // Cut short the existing data, properly.
            sb.fill = sb.p.len();
            sb.p[sb.fill - 1] = 0;
        }
        true
    } else {
        true /* success */
    }
}

/// Translation of `mpg123_move_string()`.
pub(crate) fn mpg123_move_string(from: &mut Mpg123String, to: &mut Mpg123String) -> bool {
    mpg123_free_string(to);
    *to = std::mem::take(from);
    mpg123_init_string(from);
    true
}

/// Translation of `mpg123_strlen()`.
pub fn mpg123_strlen(sb: &Mpg123String, utf8: bool) -> usize {
    /* Notions of empty string. If there's only a single character, it has to be the trailing zero, and if the first is the trailing zero anyway, we got empty. */
    if sb.fill < 2 || sb.p[0] == 0 {
        return 0;
    }
    /* Find the first non-null character from the back.
    We already established that the first character is non-null
    That at fill-2 has to be null, though. */
    let mut i = sb.fill - 2;
    while i > 0 {
        if sb.p[i] != 0 {
            break;
        }
        i -= 1;
    }
    /* For simple byte strings, we are done now. */
    let bytelen = i + 1;
    if !utf8 {
        bytelen
    } else {
        /* Work out the actual count of UTF8 bytes.
        This employs no particular encoding error checking. */
        let mut len = 0;
        for &c in &sb.p[..bytelen] {
            /* Every byte that is not a continuation byte ( 0xc0 == 10xx xxxx ) stands for a character. */
            if (c & 0xc0) != 0x80 {
                len += 1;
            }
        }
        len
    }
}

/// Translation of `mpg123_same_string()`.
pub(crate) fn mpg123_same_string(a: &Mpg123String, b: &Mpg123String) -> bool {
    if a.fill != b.fill {
        return false;
    }
    if a.p[..a.fill] != b.p[..b.fill] {
        return false;
    }
    true
}
