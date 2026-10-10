// Rust translation of pngtrans.c from libpng 1.6.59 (the transformations
// SDL_image's APNG paths use).
// Copyright (c) 2018-2026 Cosmin Truta
// Copyright (c) 1998-2002,2004,2006-2018 Glenn Randers-Pehrson
// Copyright (c) 1996-1997 Andreas Dilger
// Copyright (c) 1995-1996 Guy Eric Schalnat, Group 42, Inc.
// This code is released under the libpng license (see LICENSE.txt).
// This is an altered (translated) version of the original software.

//! pngtrans.c - transforms the data in a row (used by both readers and
//! writers): the interlace handling switch, the filler setting and the
//! palette index check. (The byte swapping, packing, inverting, shifting
//! and BGR transforms are for options SDL_image doesn't set, and are not
//! translated.)

use super::png::*;
use super::pngerror::png_app_error;
use super::pngpriv::*;
use super::pngstruct::PngStruct;

/// `png_set_interlace_handling`
pub(crate) fn png_set_interlace_handling(png_ptr: &mut PngStruct<'_, '_>) -> i32 {
    if png_ptr.interlaced != 0 {
        png_ptr.transformations |= PNG_INTERLACE;
        return 7;
    }

    1
}

/// Add a filler byte on read, or remove a filler or alpha byte on write.
/// The filler type has changed in v0.95 to allow future 2-byte fillers
/// for 48-bit input data, as well as to avoid problems with some compilers
/// that don't like bytes as parameters. (`png_set_filler`)
pub(crate) fn png_set_filler(
    png_ptr: &mut PngStruct<'_, '_>,
    filler: u32,
    filler_loc: i32,
) -> PngResult<()> {
    /* In libpng 1.6 it is possible to determine whether this is a read or write
     * operation and therefore to do more checking here for a valid call.
     */
    if (png_ptr.mode & PNG_IS_READ_STRUCT) != 0 {
        /* On read png_set_filler is always valid, regardless of the base PNG
         * format, because other transformations can give a format where the
         * filler code can execute (basically an 8 or 16-bit component RGB or G
         * format.)
         *
         * NOTE: usr_channels is not used by the read code!  (This has led to
         * confusion in the past.)  The filler is only used in the read code.
         */
        png_ptr.filler = filler as u16;
    } else {
        /* write */
        /* On write the usr_channels parameter must be set correctly at the
         * start to record the number of channels in the app-supplied data.
         */
        match png_ptr.color_type {
            PNG_COLOR_TYPE_RGB => {
                png_ptr.usr_channels = 4;
            }

            PNG_COLOR_TYPE_GRAY => {
                if png_ptr.bit_depth >= 8 {
                    png_ptr.usr_channels = 2;
                } else {
                    /* There simply isn't any code in libpng to strip out bits
                     * from bytes when the components are less than a byte in
                     * size!
                     */
                    return png_app_error(
                        png_ptr,
                        "png_set_filler is invalid for low bit depth gray output",
                    );
                }
            }

            _ => {
                return png_app_error(png_ptr, "png_set_filler: inappropriate color type");
            }
        }
    }

    /* Here on success - libpng supports the operation, set the transformation
     * and the flag to say where the filler channel is.
     */
    png_ptr.transformations |= PNG_FILLER;

    if filler_loc == PNG_FILLER_AFTER {
        png_ptr.flags |= PNG_FLAG_FILLER_AFTER;
    } else {
        png_ptr.flags &= !PNG_FLAG_FILLER_AFTER;
    }
    Ok(())
}

/// Added at libpng-1.5.10 (`png_do_check_palette_indexes`): the largest
/// palette index in the row in `png_ptr->row_buf`.
pub(crate) fn png_do_check_palette_indexes(png_ptr: &mut PngStruct<'_, '_>, row_info: &PngRowInfo) {
    if (png_ptr.num_palette as u32) < (1u32 << row_info.bit_depth) && png_ptr.num_palette > 0
    /* num_palette can be 0 in MNG files */
    {
        /* Calculations moved outside switch in an attempt to stop different
         * compiler warnings.  'padding' is in *bits* within the last byte, it is
         * an 'int' because pixel_depth becomes an 'int' in the expression below,
         * and this calculation is used because it avoids warnings that other
         * forms produced on either GCC or MSVC.
         */
        let mut padding = png_padbits(row_info.pixel_depth as u32, row_info.width);
        let row = &png_ptr.row_buf;
        let mut rp = row_info.rowbytes;
        let mut max = png_ptr.num_palette_max;

        match row_info.bit_depth {
            1 => {
                /* in this case, all bytes must be 0 so we don't need
                 * to unpack the pixels except for the rightmost one.
                 */
                while rp > 0 {
                    if (row[rp] >> padding) != 0 {
                        max = 1;
                    }
                    padding = 0;
                    rp -= 1;
                }
            }

            2 => {
                while rp > 0 {
                    let v = row[rp] >> padding;
                    for shift in [0, 2, 4, 6] {
                        let i = ((v >> shift) & 0x03) as i32;

                        if i > max {
                            max = i;
                        }
                    }

                    padding = 0;
                    rp -= 1;
                }
            }

            4 => {
                while rp > 0 {
                    let v = row[rp] >> padding;
                    for shift in [0, 4] {
                        let i = ((v >> shift) & 0x0f) as i32;

                        if i > max {
                            max = i;
                        }
                    }

                    padding = 0;
                    rp -= 1;
                }
            }

            8 => {
                while rp > 0 {
                    if row[rp] as i32 > max {
                        max = row[rp] as i32;
                    }
                    rp -= 1;
                }
            }

            _ => {}
        }
        png_ptr.num_palette_max = max;
    }
}
