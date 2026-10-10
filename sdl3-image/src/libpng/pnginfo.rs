// Rust translation of pnginfo.h from libpng 1.6.59.
// Copyright (c) 2018-2026 Cosmin Truta
// Copyright (c) 1998-2002,2004,2006-2013,2018 Glenn Randers-Pehrson
// Copyright (c) 1996-1997 Andreas Dilger
// Copyright (c) 1995-1996 Guy Eric Schalnat, Group 42, Inc.
// This code is released under the libpng license (see LICENSE.txt).
// This is an altered (translated) version of the original software.

//! pnginfo.h: header file for PNG reference library, with the fields of
//! png_info that the translated read and write paths use.

use super::png::{PngColor, PngColor16};

/// `png_info`: information about the PNG file, as read or to be written.
#[derive(Debug, Default)]
pub(crate) struct PngInfo {
    /* The following are necessary for every PNG file */
    pub(crate) width: u32,      /* width of image in pixels (from IHDR) */
    pub(crate) height: u32,     /* height of image in pixels (from IHDR) */
    pub(crate) valid: u32,      /* valid chunk data (see PNG_INFO_ below) */
    pub(crate) rowbytes: usize, /* bytes needed to hold an untransformed row */
    pub(crate) palette: Option<Vec<PngColor>>, /* array of color values (valid & PNG_INFO_PLTE) */
    pub(crate) num_palette: u16, /* number of color entries in "palette" (PLTE) */
    pub(crate) num_trans: u16,  /* number of transparent palette color (tRNS) */
    pub(crate) bit_depth: u8,   /* 1, 2, 4, 8, or 16 bits/channel (from IHDR) */
    pub(crate) color_type: u8,  /* see PNG_COLOR_TYPE_ below (from IHDR) */
    /* The following three should have been named *_method not *_type */
    pub(crate) compression_type: u8, /* must be PNG_COMPRESSION_TYPE_BASE (IHDR) */
    pub(crate) filter_type: u8,      /* must be PNG_FILTER_TYPE_BASE (from IHDR) */
    pub(crate) interlace_type: u8,   /* One of PNG_INTERLACE_NONE, PNG_INTERLACE_ADAM7 */

    /* The following are set by png_set_IHDR, called from the application on
     * write, but the are never actually used by the write code.
     */
    pub(crate) channels: u8, /* number of data channels per pixel (1, 2, 3, 4) */
    pub(crate) pixel_depth: u8, /* number of bits per pixel */

    /* This is never set during write */
    pub(crate) signature: [u8; 8], /* magic bytes read by libpng from start of file */

    /* The gamma the file is encoded with (png_read_transform_info) */
    pub(crate) gamma: i32,

    /* The tRNS chunk supplies transparency data for paletted images and
     * other image types that don't need a full alpha channel.  There are
     * "num_trans" transparency values for a paletted image, stored in the
     * same order as the palette colors, starting from index 0.  Values
     * for the data are in the range [0, 255], ranging from fully transparent
     * to fully opaque, respectively.  For non-paletted images, there is a
     * single color specified that should be treated as fully transparent.
     * Data is valid if (valid & PNG_INFO_tRNS) is non-zero.
     */
    pub(crate) trans_alpha: Option<Vec<u8>>, /* alpha values for paletted image */
    pub(crate) trans_color: PngColor16,      /* transparent color for non-palette image */
}
