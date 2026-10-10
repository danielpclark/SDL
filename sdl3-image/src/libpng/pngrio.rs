// Rust translation of pngrio.c from libpng 1.6.59.
// Copyright (c) 2018-2026 Cosmin Truta
// Copyright (c) 1998-2002,2004,2006-2016,2018 Glenn Randers-Pehrson
// Copyright (c) 1996-1997 Andreas Dilger
// Copyright (c) 1995-1996 Guy Eric Schalnat, Group 42, Inc.
// This code is released under the libpng license (see LICENSE.txt).
// This is an altered (translated) version of the original software.

//! pngrio.c - functions for data input
//!
//! This file provides a location for all input.  Users who need
//! special handling are expected to write a function that has the same
//! arguments as this and performs a similar function, but that possibly
//! has a different input method.  Note that you shouldn't change this
//! function, but rather write a replacement function and then make
//! libpng use it at run time with png_set_read_fn(...).
//!
//! (There is no default stdio reader: SDL_image always sets its own.)

use sdl3::io::IoStream;

use super::png::{PngReadPtr, PngResult};
use super::pngerror::{png_error, png_warning};
use super::pngstruct::PngStruct;

/// Read the data from whatever input you are using.  The default routine
/// reads from a file pointer.  Note that this routine sometimes gets called
/// with very small lengths, so you should implement some kind of simple
/// buffering if you are using unbuffered reads.  This should never be asked
/// to read more than 64K on a 16-bit machine. (`png_read_data`)
pub(crate) fn png_read_data(png_ptr: &mut PngStruct<'_, '_>, data: &mut [u8]) -> PngResult<()> {
    match png_ptr.read_data_fn {
        Some(read_data_fn) => read_data_fn(png_ptr, data),
        None => Err(png_error(png_ptr, "Call to NULL read function")),
    }
}

/// This function allows the application to supply a new input function
/// for libpng if standard C streams aren't being used.
/// (`png_set_read_fn`)
pub(crate) fn png_set_read_fn<'s, 'b>(
    png_ptr: &mut PngStruct<'s, 'b>,
    io_ptr: Option<&'s mut IoStream<'b>>,
    read_data_fn: Option<PngReadPtr>,
) {
    png_ptr.io_ptr = io_ptr;

    png_ptr.read_data_fn = read_data_fn;

    /* It is an error to write to a read device */
    if png_ptr.write_data_fn.is_some() {
        png_ptr.write_data_fn = None;
        png_warning(
            png_ptr,
            "Can't set both read_data_fn and write_data_fn in the same structure",
        );
    }

    png_ptr.output_flush_fn = None;
}
