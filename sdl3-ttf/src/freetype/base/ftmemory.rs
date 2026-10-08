// Rust translation of the memory management of src/base/ftutil.c and
// include/freetype/internal/ftmemory.h from FreeType (2.13.2, as SDL_ttf's
// external/freetype pins it).
// Copyright (C) 2002-2023 by David Turner, Robert Wilhelm, and Werner Lemberg.
// This is an altered (translated) version of the original software; it is
// used under the FreeType License (see FTL.TXT and LICENSE.txt).

//! Memory management: FreeType's blocks become `Vec`s, allocated
//! fallibly with the same size checks (`ft_mem_qrealloc`): a negative size
//! is an invalid argument, an array of more than `FT_INT_MAX` bytes is
//! too large, and an allocation the system refuses is out of memory.

use super::super::fttypes::*;

const FT_INT_MAX: i64 = i32::MAX as i64;

/// The checks of `ft_mem_qrealloc` for a new array of `new_count` items of
/// `item_size` bytes.
fn ft_mem_check(item_size: i64, new_count: i64) -> FtResult<()> {
    if new_count < 0 || item_size < 0 {
        /* may help catch/prevent nasty security issues */
        return Err(FT_ERR_INVALID_ARGUMENT);
    }
    if new_count != 0 && item_size != 0 && new_count > FT_INT_MAX / item_size {
        return Err(FT_ERR_ARRAY_TOO_LARGE);
    }
    Ok(())
}

/// A vector of `count` copies of `value`, allocated fallibly.
fn ft_vec_filled<T: Clone>(count: usize, value: T) -> FtResult<Vec<T>> {
    let mut v = Vec::new();
    if v.try_reserve_exact(count).is_err() {
        return Err(FT_ERR_OUT_OF_MEMORY);
    }
    v.resize(count, value);
    Ok(v)
}

/// `FT_ALLOC` / `FT_QALLOC` (`ft_mem_alloc`, `ft_mem_qalloc`): a block of
/// `size` bytes (zeroed; C's `QALLOC` leaves it uninitialized).
pub fn ft_qalloc(size: FtLong) -> FtResult<Vec<u8>> {
    if size < 0 {
        /* may help catch/prevent security issues */
        return Err(FT_ERR_INVALID_ARGUMENT);
    }
    ft_vec_filled(size as usize, 0u8)
}

/// `FT_ALLOC`
pub fn ft_alloc(size: FtLong) -> FtResult<Vec<u8>> {
    ft_qalloc(size)
}

/// `FT_ALLOC_MULT` / `FT_QALLOC_MULT`: a block of `count` items of
/// `item_size` bytes.
pub fn ft_alloc_mult(count: FtLong, item_size: FtLong) -> FtResult<Vec<u8>> {
    ft_mem_check(item_size, count)?;
    ft_vec_filled((count * item_size) as usize, 0u8)
}

/// `FT_NEW_ARRAY` / `FT_QNEW_ARRAY`: `count` default items.
pub fn ft_new_array<T: Clone + Default>(count: FtLong) -> FtResult<Vec<T>> {
    ft_mem_check(std::mem::size_of::<T>().max(1) as i64, count)?;
    ft_vec_filled(count as usize, T::default())
}

/// `FT_RENEW_ARRAY` / `FT_QRENEW_ARRAY`: resizes `v` to `new_count` items,
/// the new ones default.
pub fn ft_renew_array<T: Clone + Default>(v: &mut Vec<T>, new_count: FtLong) -> FtResult<()> {
    ft_mem_check(std::mem::size_of::<T>().max(1) as i64, new_count)?;
    let new_count = new_count as usize;
    if new_count > v.len() && v.try_reserve_exact(new_count - v.len()).is_err() {
        return Err(FT_ERR_OUT_OF_MEMORY);
    }
    v.resize(new_count, T::default());
    Ok(())
}

/// `ft_mem_strcpyn`: copies at most `size - 1` bytes of the NUL-terminated
/// `src` into `dst` and terminates it; returns whether `src` was cut.
pub fn ft_mem_strcpyn(dst: &mut [u8], src: &[u8], size: usize) -> bool {
    let mut size = size;
    let mut d = 0;
    let mut s = 0;
    while size > 1 && s < src.len() && src[s] != 0 {
        dst[d] = src[s];
        d += 1;
        s += 1;
        size -= 1;
    }
    dst[d] = 0; /* always zero-terminate */

    s < src.len() && src[s] != 0
}
