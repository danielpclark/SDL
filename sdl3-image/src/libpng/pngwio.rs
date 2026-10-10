// Rust translation of pngwio.c from libpng 1.6.59.
// Copyright (c) 2018-2026 Cosmin Truta
// Copyright (c) 1998-2002,2004,2006-2014,2016,2018 Glenn Randers-Pehrson
// Copyright (c) 1996-1997 Andreas Dilger
// Copyright (c) 1995-1996 Guy Eric Schalnat, Group 42, Inc.
// This code is released under the libpng license (see LICENSE.txt).
// This is an altered (translated) version of the original software.

//! pngwio.c - functions for data output
//!
//! This file provides a location for all output.  Users who need
//! special handling are expected to write functions that have the same
//! arguments as these and perform similar functions, but that possibly
//! use different output methods.  Note that you shouldn't change these
//! functions, but rather write replacement functions and then change
//! them at run time with png_set_write_fn(...).
//!
//! (There are no default stdio writers: SDL_image always sets its own.)

use sdl3::io::IoStream;

use super::png::{PngFlushPtr, PngResult, PngWritePtr};
use super::pngerror::{png_error, png_warning};
use super::pngstruct::PngStruct;

/// Write the data to whatever output you are using.  The default routine
/// writes to a file pointer.  Note that this routine sometimes gets called
/// with very small lengths, so you should implement some kind of simple
/// buffering if you are using unbuffered writes.  This should never be asked
/// to write more than 64K on a 16-bit machine. (`png_write_data`)
pub(crate) fn png_write_data(png_ptr: &mut PngStruct<'_, '_>, data: &[u8]) -> PngResult<()> {
    /* NOTE: write_data_fn must not change the buffer! */
    match png_ptr.write_data_fn {
        Some(write_data_fn) => write_data_fn(png_ptr, data),
        None => Err(png_error(png_ptr, "Call to NULL write function")),
    }
}

/// This function is called to output any data pending writing (normally
/// to disk).  After png_flush is called, there should be no data pending
/// writing in any buffers. (`png_flush`)
pub(crate) fn png_flush(png_ptr: &mut PngStruct<'_, '_>) -> PngResult<()> {
    match png_ptr.output_flush_fn {
        Some(output_flush_fn) => output_flush_fn(png_ptr),
        None => Ok(()),
    }
}

/// This function allows the user to replace the default data output
/// functions. (`png_set_write_fn`)
pub(crate) fn png_set_write_fn<'s, 'b>(
    png_ptr: &mut PngStruct<'s, 'b>,
    io_ptr: Option<&'s mut IoStream<'b>>,
    write_data_fn: Option<PngWritePtr>,
    output_flush_fn: Option<PngFlushPtr>,
) {
    png_ptr.io_ptr = io_ptr;

    png_ptr.write_data_fn = write_data_fn;

    png_ptr.output_flush_fn = output_flush_fn;

    /* It is an error to read while writing a png file */
    if png_ptr.read_data_fn.is_some() {
        png_ptr.read_data_fn = None;

        png_warning(
            png_ptr,
            "Can't set both read_data_fn and write_data_fn in the same structure",
        );
    }
}
