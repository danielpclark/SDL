// Rust translation of src/dataio.c from libxmp 4.7.3 (as vendored by SDL_mixer).
// Extended Module Player
// Copyright (C) 1996-2026 Claudio Matsuoka and Hipolito Carraro Jr
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! Reading little- and big-endian integers from memory. (The `FILE *`
//! readers and writers aren't used by anything translated here.)

/// Translation of `readmem16l()`.
#[inline]
pub(crate) fn readmem16l(m: &[u8]) -> u16 {
    let a = m[0] as u32;
    let b = m[1] as u32;

    ((b << 8) | a) as u16
}

/// Translation of `readmem16b()`.
#[inline]
pub(crate) fn readmem16b(m: &[u8]) -> u16 {
    let a = m[0] as u32;
    let b = m[1] as u32;

    ((a << 8) | b) as u16
}

/// Translation of `readmem24l()`.
#[inline]
pub(crate) fn readmem24l(m: &[u8]) -> u32 {
    let a = m[0] as u32;
    let b = m[1] as u32;
    let c = m[2] as u32;

    (c << 16) | (b << 8) | a
}

/// Translation of `readmem24b()`.
#[inline]
pub(crate) fn readmem24b(m: &[u8]) -> u32 {
    let a = m[0] as u32;
    let b = m[1] as u32;
    let c = m[2] as u32;

    (a << 16) | (b << 8) | c
}

/// Translation of `readmem32l()`.
#[inline]
pub(crate) fn readmem32l(m: &[u8]) -> u32 {
    let a = m[0] as u32;
    let b = m[1] as u32;
    let c = m[2] as u32;
    let d = m[3] as u32;

    (d << 24) | (c << 16) | (b << 8) | a
}

/// Translation of `readmem32b()`.
#[inline]
pub(crate) fn readmem32b(m: &[u8]) -> u32 {
    let a = m[0] as u32;
    let b = m[1] as u32;
    let c = m[2] as u32;
    let d = m[3] as u32;

    (a << 24) | (b << 16) | (c << 8) | d
}
