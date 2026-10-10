// Rust translation of pngmem.c from libpng 1.6.59.
// Copyright (c) 2018-2026 Cosmin Truta
// Copyright (c) 1998-2002,2004,2006-2014,2016 Glenn Randers-Pehrson
// Copyright (c) 1996-1997 Andreas Dilger
// Copyright (c) 1995-1996 Guy Eric Schalnat, Group 42, Inc.
// This code is released under the libpng license (see LICENSE.txt).
// This is an altered (translated) version of the original software.

//! pngmem.c - stub functions for memory allocation
//!
//! This file provides a location for all memory allocation.  Users who
//! need special memory handling are expected to supply replacement
//! functions for png_malloc() and png_free(), and to use
//! png_create_read_struct_2() and png_create_write_struct_2() to
//! identify the replacement functions.
//!
//! (The allocations are byte vectors, zeroed where C's `malloc()` leaves
//! them as they are: libpng never reads those bytes before writing them.
//! `png_free()` is dropping them.)

use super::png::PngResult;
use super::pngerror::{png_error, png_warning};
use super::pngstruct::PngStruct;

/// Allocate memory.  For reasonable files, size should never exceed
/// 64K.  However, zlib may allocate more than 64K if you don't tell
/// it not to.  See zconf.h and png.h for more information.  zlib does
/// need to allocate exactly 64K, so whatever you call here must
/// have the ability to do that. (`png_calloc`)
pub(crate) fn png_calloc(png_ptr: &PngStruct<'_, '_>, size: usize) -> PngResult<Vec<u8>> {
    png_malloc(png_ptr, size)
}

/// png_malloc_base, an internal function added at libpng 1.6.0, does the work of
/// allocating memory, taking into account limits and PNG_USER_MEM_SUPPORTED.
/// Checking and error handling must happen outside this routine; it returns NULL
/// if the allocation cannot be done (for any reason.) (`png_malloc_base`)
pub(crate) fn png_malloc_base(_png_ptr: &PngStruct<'_, '_>, size: usize) -> Option<Vec<u8>> {
    /* (size > PNG_SIZE_MAX can't be: size is a usize) */

    /* Use the system malloc */
    let mut v = Vec::new();
    v.try_reserve_exact(size).ok()?;
    v.resize(size, 0);
    Some(v)
}

/// Various functions that have different error handling are derived from this.
/// png_malloc always exists, but if PNG_USER_MEM_SUPPORTED is defined a separate
/// function png_malloc_default is also provided. (`png_malloc`)
pub(crate) fn png_malloc(png_ptr: &PngStruct<'_, '_>, size: usize) -> PngResult<Vec<u8>> {
    match png_malloc_base(png_ptr, size) {
        Some(ret) => Ok(ret),
        None => Err(png_error(png_ptr, "Out of memory")), /* 'm' means png_malloc */
    }
}

/// This function was added at libpng version 1.2.3.  The png_malloc_warn()
/// function will issue a png_warning and return NULL instead of issuing a
/// png_error, if it fails to allocate the requested memory.
/// (`png_malloc_warn`)
#[allow(dead_code)]
pub(crate) fn png_malloc_warn(png_ptr: &PngStruct<'_, '_>, size: usize) -> Option<Vec<u8>> {
    let ret = png_malloc_base(png_ptr, size);
    if ret.is_none() {
        png_warning(png_ptr, "Out of memory");
    }
    ret
}
