// Sets up common environment for Shay Green's libraries.
// Game_Music_Emu https://bitbucket.org/mpyne/game-music-emu/
// Modified 2026-10-17: translated into Rust from SDL_mixer 3.3.0's Game_Music_Emu 0.6.5 (see NOTICE).

//! `blargg_common.h`, `blargg_source.h` and `blargg_endian.h`: the error
//! type, the byte order helpers and the four-character constants.

/// `blargg_err_t` (`Ok` on success, otherwise the error string). The
/// `RETURN_ERR()` macro is `?`.
pub type BlarggErr = Result<(), &'static str>;

/// `BLARGG_4CHAR('a','b','c','d')` = 'abcd' (four character integer constant)
pub const fn blargg_4char(a: u8, b: u8, c: u8, d: u8) -> i64 {
    (a as i64) * 0x1000000 + (b as i64) * 0x10000 + (c as i64) * 0x100 + d as i64
}

/// `BLARGG_2CHAR( a, b )`
pub const fn blargg_2char(a: u8, b: u8) -> i64 {
    (a as i64) * 0x100 + b as i64
}

#[inline]
pub fn get_le16(p: &[u8]) -> u32 {
    (p[1] as u32) << 8 | p[0] as u32
}

#[inline]
pub fn get_be16(p: &[u8]) -> u32 {
    (p[0] as u32) << 8 | p[1] as u32
}

#[inline]
pub fn get_le32(p: &[u8]) -> u32 {
    (p[3] as u32) << 24 | (p[2] as u32) << 16 | (p[1] as u32) << 8 | p[0] as u32
}

#[inline]
pub fn get_be32(p: &[u8]) -> u32 {
    (p[0] as u32) << 24 | (p[1] as u32) << 16 | (p[2] as u32) << 8 | p[3] as u32
}

#[inline]
pub fn set_le16(p: &mut [u8], n: u32) {
    p[1] = (n >> 8) as u8;
    p[0] = n as u8;
}

#[inline]
pub fn set_be16(p: &mut [u8], n: u32) {
    p[0] = (n >> 8) as u8;
    p[1] = n as u8;
}

/// A vector of `n` zeroed items, or "Out of memory" (`blargg_vector::resize()`,
/// whose sizes can come from file headers).
pub fn try_vec<T: Clone + Default>(n: usize) -> Result<Vec<T>, &'static str> {
    let mut v = Vec::new();
    if v.try_reserve_exact(n).is_err() {
        return Err("Out of memory");
    }
    v.resize(n, T::default());
    Ok(v)
}

/// `blargg_vector::resize()`: grow or shrink `v` to `n` items, new ones
/// zeroed (upstream's are uninitialized, and never read before they are
/// written).
pub fn try_resize<T: Clone + Default>(v: &mut Vec<T>, n: usize) -> Result<(), &'static str> {
    if n > v.len() && v.try_reserve_exact(n - v.len()).is_err() {
        return Err("Out of memory");
    }
    v.resize(n, T::default());
    Ok(())
}

/// The bytes of a C string in `s`: up to its first NUL (or all of it).
pub fn cstr(s: &[u8]) -> &[u8] {
    match s.iter().position(|&c| c == 0) {
        Some(n) => &s[..n],
        None => s,
    }
}
