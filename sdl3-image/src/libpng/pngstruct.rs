// Rust translation of pngstruct.h from libpng 1.6.59.
// Copyright (c) 2018-2026 Cosmin Truta
// Copyright (c) 1998-2002,2004,2006-2018 Glenn Randers-Pehrson
// Copyright (c) 1996-1997 Andreas Dilger
// Copyright (c) 1995-1996 Guy Eric Schalnat, Group 42, Inc.
// This code is released under the libpng license (see LICENSE.txt).
// This is an altered (translated) version of the original software.

//! pngstruct.h: internal structure of the png_struct, with the fields
//! the translated read and write paths use.
//!
//! The buffers C points into (`row_buf`, `prev_row` and their `big_`
//! allocations, the read buffer, the compression buffer list) are owned
//! vectors here; the alignment padding around the row buffers, which only
//! libpng's SIMD filters use, isn't kept.

use sdl3::io::IoStream;

use super::png::{PngColor, PngColor16, PngFlushPtr, PngReadPtr, PngWritePtr};
use crate::zlib::ZStream;

/// `png_struct`: the state of a read or write, with the stream it reads
/// or writes (`io_ptr`, borrowed for `'s`, of data borrowed for `'b`).
#[derive(Default)]
pub(crate) struct PngStruct<'s, 'b> {
    /* (jmp_buf_local and friends: errors return PngError) */
    /* (error_fn, warning_fn and error_ptr: libpng's default handlers) */
    pub(crate) write_data_fn: Option<PngWritePtr>, /* function for writing output data */
    pub(crate) read_data_fn: Option<PngReadPtr>,   /* function for reading input data */
    pub(crate) io_ptr: Option<&'s mut IoStream<'b>>, /* ptr to application struct for I/O functions */

    pub(crate) mode: u32,            /* tells us where we are in the PNG file */
    pub(crate) flags: u32,           /* flags indicating various things to libpng */
    pub(crate) transformations: u32, /* which transformations to perform */

    pub(crate) zowner: u32, /* ID (chunk type) of zstream owner, 0 if none */
    pub(crate) zstream: ZStream, /* decompression structure */

    pub(crate) zbuffer_list: Option<Vec<u8>>, /* Created on demand during write */
    pub(crate) zbuffer_size: u32,             /* size of the actual buffer */

    pub(crate) zlib_level: i32,       /* holds zlib compression level */
    pub(crate) zlib_method: i32,      /* holds zlib compression method */
    pub(crate) zlib_window_bits: i32, /* holds zlib compression window bits */
    pub(crate) zlib_mem_level: i32,   /* holds zlib compression memory level */
    pub(crate) zlib_strategy: i32,    /* holds zlib compression strategy */

    /* Added at libpng 1.5.4 */
    pub(crate) zlib_text_level: i32, /* holds zlib compression level */
    pub(crate) zlib_text_method: i32, /* holds zlib compression method */
    pub(crate) zlib_text_window_bits: i32, /* holds zlib compression window bits */
    pub(crate) zlib_text_mem_level: i32, /* holds zlib compression memory level */
    pub(crate) zlib_text_strategy: i32, /* holds zlib compression strategy */

    /* (zlib_set_*: never compared, as each struct claims its zstream once) */
    pub(crate) zlib_set_level: i32,
    pub(crate) zlib_set_method: i32,
    pub(crate) zlib_set_window_bits: i32,
    pub(crate) zlib_set_mem_level: i32,
    pub(crate) zlib_set_strategy: i32,

    pub(crate) chunks: u32, /* PNG_CF_ for every chunk read or (NYI) written */

    pub(crate) width: u32,      /* width of image in pixels */
    pub(crate) height: u32,     /* height of image in pixels */
    pub(crate) num_rows: u32,   /* number of rows in current pass */
    pub(crate) usr_width: u32,  /* width of row at start of write */
    pub(crate) rowbytes: usize, /* size of row in bytes */
    pub(crate) iwidth: u32,     /* width of current interlaced row in pixels */
    pub(crate) row_number: u32, /* current row in interlace pass */
    pub(crate) chunk_name: u32, /* PNG_CHUNK() id of current chunk */
    pub(crate) prev_row: Option<Vec<u8>>, /* buffer to save previous (unfiltered) row.
                                 * While reading this is a pointer into
                                 * big_prev_row; while writing it is separately
                                 * allocated if needed.
                                 */
    pub(crate) row_buf: Vec<u8>, /* buffer to save current (unfiltered) row.
                                  * While reading, this is a pointer into
                                  * big_row_buf; while writing it is separately
                                  * allocated.
                                  */
    pub(crate) try_row: Option<Vec<u8>>, /* buffer to save trial row when filtering */
    pub(crate) tst_row: Option<Vec<u8>>, /* buffer to save best trial row when filtering */
    pub(crate) info_rowbytes: usize,     /* Added in 1.5.4: cache of updated row bytes */

    pub(crate) idat_size: u32, /* current IDAT size for read */
    pub(crate) crc: u32,       /* current chunk CRC value */
    pub(crate) palette: Option<Vec<PngColor>>, /* palette from the input file */
    pub(crate) num_palette: u16, /* number of color entries in palette */

    /* Added at libpng-1.5.10 */
    pub(crate) num_palette_max: i32, /* maximum palette index found in IDAT */

    pub(crate) num_trans: u16,          /* number of transparency values */
    pub(crate) compression: u8,         /* file compression type (always 0) */
    pub(crate) filter: u8,              /* file filter type (always 0) */
    pub(crate) interlaced: u8,          /* PNG_INTERLACE_NONE, PNG_INTERLACE_ADAM7 */
    pub(crate) pass: u8,                /* current interlace pass (0 - 6) */
    pub(crate) do_filter: u8,           /* row filter flags (see PNG_FILTER_ in png.h ) */
    pub(crate) color_type: u8,          /* color type of file */
    pub(crate) bit_depth: u8,           /* bit depth of file */
    pub(crate) usr_bit_depth: u8,       /* bit depth of users row: write only */
    pub(crate) pixel_depth: u8,         /* number of bits per pixel */
    pub(crate) channels: u8,            /* number of channels in file */
    pub(crate) usr_channels: u8,        /* channels at start of write: write only */
    pub(crate) sig_bytes: u8,           /* magic bytes read/written from start of file */
    pub(crate) maximum_pixel_depth: u8, /* pixel depth used for the row buffers */
    pub(crate) transformed_pixel_depth: u8, /* pixel depth after read/write transforms */
    pub(crate) zstream_start: u8,       /* at start of an input zlib stream */
    pub(crate) filler: u16,             /* filler bytes for pixel expansion */

    pub(crate) trans_alpha: Option<Vec<u8>>, /* alpha values for paletted files */
    pub(crate) trans_color: PngColor16,      /* transparent color for non-paletted files */

    pub(crate) output_flush_fn: Option<PngFlushPtr>, /* Function for flushing output */
    pub(crate) flush_dist: u32, /* how many rows apart to flush, 0 - no flush */
    pub(crate) flush_rows: u32, /* number of rows written since last flush */

    /* The gamma the file is encoded with and the gamma of the screen */
    pub(crate) file_gamma: i32,    /* file gamma value */
    pub(crate) screen_gamma: i32,  /* screen gamma value (display_exponent) */
    pub(crate) chunk_gamma: i32,   /* from cICP, iCCP, sRGB or gAMA */
    pub(crate) default_gamma: i32, /* from png_set_alpha_mode */

    pub(crate) mng_features_permitted: u32,

    pub(crate) filter_type: u8, /* (MNG_FEATURES) */

    /* Options */
    pub(crate) options: u8, /* On/off state (up to 4 options) */

    pub(crate) big_row_buf_size: usize, /* (the allocated row_buf) */
    pub(crate) old_big_row_buf_size: usize,

    /* New member added in libpng-1.2.30 */
    pub(crate) read_buffer: Option<Vec<u8>>, /* buffer for reading chunk data */
    pub(crate) read_buffer_size: usize,      /* current size of the buffer */
    pub(crate) IDAT_read_size: u32,          /* limit on read buffer size for IDAT */

    pub(crate) num_chunk_list: u32, /* (the list png_set_keep_unknown_chunks() makes) */
    pub(crate) unknown_default: i32, /* As PNG_HANDLE_* */

    /* Added in 1.5.6: the IDAT reading uses the filter functions (read_filter);
     * set when they are initialized */
    pub(crate) read_filter_init: bool,

    /* New members added in libpng-1.4.0 */
    pub(crate) user_width_max: u32,
    pub(crate) user_height_max: u32,

    /* Added in libpng-1.4.0: Total number of sPLT, text, and unknown
     * chunks that can be stored (0 means unlimited).
     */
    pub(crate) user_chunk_cache_max: u32,

    /* Total memory that a zTXt, sPLT, iTXt, iCCP, or unknown chunk
     * can occupy when decompressed.  0 means unlimited.
     */
    pub(crate) user_chunk_malloc_max: usize,
}
