// Rust translation of pngrtran.c from libpng 1.6.59 (the transformations
// SDL_image's APNG frame reader asks for).
// Copyright (c) 2018-2026 Cosmin Truta
// Copyright (c) 1998-2002,2004,2006-2018 Glenn Randers-Pehrson
// Copyright (c) 1996-1997 Andreas Dilger
// Copyright (c) 1995-1996 Guy Eric Schalnat, Group 42, Inc.
// This code is released under the libpng license (see LICENSE.txt).
// This is an altered (translated) version of the original software.

//! pngrtran.c - transforms the data in a row for PNG readers
//!
//! This file contains functions optionally called by an application
//! in order to tell libpng how to handle data when reading a PNG.
//! Transformations that are used in both reading and writing are
//! in pngtrans.c.
//!
//! SDL_image asks for three transformations: `png_set_palette_to_rgb()`
//! (for color-mapped images), `png_set_strip_16()` and `png_set_filler()`,
//! and its frame streams carry no gamma, sRGB, iCCP, cHRM or cICP chunk; so
//! the gamma, background, alpha mode, quantizing, RGB to gray, gray to RGB,
//! expand-16, shift, invert, swap and user transformations (and
//! `png_do_expand()`, which only non-palette images with PNG_EXPAND reach)
//! are not translated, and `png_init_read_transformations()` keeps the
//! steps that apply.

use super::png::*;
use super::pngerror::{png_app_error, png_error};
use super::pnginfo::PngInfo;
use super::pngpriv::*;
use super::pngstruct::PngStruct;

/// Is it OK to set a transformation now?  Only if png_start_read_image or
/// png_read_update_info have not been called.  It is not necessary for the IHDR
/// to have been read in all cases; the need_IHDR parameter allows for this
/// check too. (`png_rtran_ok`)
#[allow(non_snake_case)]
fn png_rtran_ok(png_ptr: &mut PngStruct<'_, '_>, need_IHDR: bool) -> PngResult<bool> {
    if (png_ptr.flags & PNG_FLAG_ROW_INIT) != 0 {
        png_app_error(
            png_ptr,
            "invalid after png_start_read_image or png_read_update_info",
        )?;
    } else if need_IHDR && (png_ptr.mode & PNG_HAVE_IHDR) == 0 {
        png_app_error(png_ptr, "invalid before the PNG header has been read")?;
    } else {
        /* Turn on failure to initialize correctly for all transforms. */
        png_ptr.flags |= PNG_FLAG_DETECT_UNINITIALIZED;

        return Ok(true); /* Ok */
    }

    Ok(false) /* no png_error possible! */
}

/* Scale 16-bit depth files to 8-bit depth.  If both of these are set then the
 * one that pngrtran does first (scale) happens.  This is necessary to allow the
 * TRANSFORM and API behavior to be somewhat consistent, and it's simpler.
 */

/// Chop 16-bit depth files to 8-bit depth (`png_set_strip_16`)
pub(crate) fn png_set_strip_16(png_ptr: &mut PngStruct<'_, '_>) -> PngResult<()> {
    if !png_rtran_ok(png_ptr, false)? {
        return Ok(());
    }

    png_ptr.transformations |= PNG_16_TO_8;
    Ok(())
}

/* GRR 19990627:  the following three functions currently are identical
 *  to png_set_expand().  However, it is entirely reasonable that someone
 *  might wish to expand an indexed image to RGB but *not* expand a single,
 *  fully transparent palette entry to a full alpha channel--perhaps instead
 *  convert tRNS to the grayscale/RGB format (16-bit RGB value), or replace
 *  the transparent color with a particular RGB value, or drop tRNS entirely.
 *  IOW, a future version of the library may make the transformations flag
 *  a bit more fine-grained, with separate bits for each of these three
 *  functions.
 *
 *  More to the point, these functions make it obvious what libpng will be
 *  doing, whereas "expand" can (and does) mean any number of things.
 *
 *  GRP 20060307: In libpng-1.2.9, png_set_gray_1_2_4_to_8() was modified
 *  to expand only the sample depth but not to expand the tRNS to alpha
 *  and its name was changed to png_set_expand_gray_1_2_4_to_8().
 */

/// Expand paletted images to RGB. (`png_set_palette_to_rgb`)
pub(crate) fn png_set_palette_to_rgb(png_ptr: &mut PngStruct<'_, '_>) -> PngResult<()> {
    if !png_rtran_ok(png_ptr, false)? {
        return Ok(());
    }

    png_ptr.transformations |= PNG_EXPAND | PNG_EXPAND_tRNS;
    Ok(())
}

/* Initialize everything needed for the read.  This includes modifying
 * the palette.
 */

/* For the moment 'png_init_palette_transformations' and
 * 'png_init_rgb_transformations' only do some flag canceling optimizations.
 * The intent is that these two routines should have palette or rgb operations
 * extracted from 'png_init_read_transformations'.
 */
/// `png_init_palette_transformations` (the background handling, for
/// `png_set_background()`, not translated)
fn png_init_palette_transformations(png_ptr: &mut PngStruct<'_, '_>) {
    /* Called to handle the (input) palette case.  In png_do_read_transformations
     * the first step is to expand the palette if requested, so this code must
     * take care to only make changes that are invariant with respect to the
     * palette expansion, or only do them if there is no expansion.
     *
     * STRIP_ALPHA has already been handled in the caller (by setting num_trans
     * to 0.)
     */
    let mut input_has_alpha = 0;
    let mut input_has_transparency = 0;

    if png_ptr.num_trans > 0 {
        let trans_alpha = png_ptr.trans_alpha.as_deref().unwrap_or(&[]);

        /* Ignore if all the entries are opaque (unlikely!) */
        for i in 0..png_ptr.num_trans as usize {
            let a = trans_alpha.get(i).copied().unwrap_or(0xff);
            if a == 255 {
                continue;
            } else if a == 0 {
                input_has_transparency = 1;
            } else {
                input_has_transparency = 1;
                input_has_alpha = 1;
                break;
            }
        }
    }

    /* If no alpha we can optimize. */
    if input_has_alpha == 0 {
        /* Any alpha means background and associative alpha processing is
         * required, however if the alpha is 0 or 1 throughout OPTIMIZE_ALPHA
         * and ENCODE_ALPHA are irrelevant.
         */
        png_ptr.transformations &= !PNG_ENCODE_ALPHA;
        png_ptr.flags &= !PNG_FLAG_OPTIMIZE_ALPHA;

        if input_has_transparency == 0 {
            png_ptr.transformations &= !(PNG_COMPOSE | PNG_BACKGROUND_EXPAND);
        }
    }
}

/// `png_init_rgb_transformations` (the background handling, for
/// `png_set_background()`, not translated)
fn png_init_rgb_transformations(png_ptr: &mut PngStruct<'_, '_>) {
    /* Added to libpng-1.5.4: check the color type to determine whether there
     * is any alpha or transparency in the image and simply cancel the
     * background and alpha mode stuff if there isn't.
     */
    let input_has_alpha = (png_ptr.color_type & PNG_COLOR_MASK_ALPHA) != 0;
    let input_has_transparency = png_ptr.num_trans > 0;

    /* If no alpha we can optimize. */
    if !input_has_alpha {
        /* Any alpha means background and associative alpha processing is
         * required, however if the alpha is 0 or 1 throughout OPTIMIZE_ALPHA
         * and ENCODE_ALPHA are irrelevant.
         */
        png_ptr.transformations &= !PNG_ENCODE_ALPHA;
        png_ptr.flags &= !PNG_FLAG_OPTIMIZE_ALPHA;

        if !input_has_transparency {
            png_ptr.transformations &= !(PNG_COMPOSE | PNG_BACKGROUND_EXPAND);
        }
    }
}

/// `png_resolve_file_gamma`
fn png_resolve_file_gamma(png_ptr: &PngStruct<'_, '_>) -> i32 {
    /* The file gamma is determined by these precedence rules, in this order
     * (i.e. use the first value found):
     *
     *    png_set_gamma; png_struct::file_gammma if not zero, then:
     *    png_struct::chunk_gamma if not 0 (determined the PNGv3 rules), then:
     *    png_set_gamma; 1/png_struct::screen_gamma if not zero
     *
     *    0 (i.e. do no gamma handling)
     */
    let mut file_gamma = png_ptr.file_gamma;
    if file_gamma != 0 {
        return file_gamma;
    }

    file_gamma = png_ptr.chunk_gamma;
    if file_gamma != 0 {
        return file_gamma;
    }

    file_gamma = png_ptr.default_gamma;
    if file_gamma != 0 {
        return file_gamma;
    }

    /* (png_ptr->screen_gamma: never set, as SDL_image calls no gamma API) */
    file_gamma
}

/// `png_init_gamma_values` (SDL_image sets no gamma and its streams carry
/// none, so both gammas resolve to 1 and no correction is needed)
fn png_init_gamma_values(png_ptr: &mut PngStruct<'_, '_>) -> bool {
    /* The following temporary indicates if overall gamma correction is
     * required.
     */
    let gamma_correction = false;

    /* Resolve the file_gamma.  See above: if png_ptr::screen_gamma is set
     * file_gamma will always be set here:
     */
    let mut file_gamma = png_resolve_file_gamma(png_ptr);
    let mut screen_gamma = png_ptr.screen_gamma;

    if file_gamma > 0 {
        /* (a file gamma, with png_gamma_threshold() and png_reciprocal():
         * never the case here) */
        if screen_gamma <= 0 {
            screen_gamma = file_gamma;
        }
    } else {
        /* both unset, prevent corrections: */
        file_gamma = PNG_FP_1;
        screen_gamma = PNG_FP_1;
    }

    png_ptr.file_gamma = file_gamma;
    png_ptr.screen_gamma = screen_gamma;
    gamma_correction
}

/// `png_init_read_transformations`
pub(crate) fn png_init_read_transformations(png_ptr: &mut PngStruct<'_, '_>) {
    /* This internal function is called from png_read_start_row in pngrutil.c
     * and it is called before the 'rowbytes' calculation is done, so the code
     * in here can change or update the transformations flags.
     *
     * First do updates that do not depend on the details of the PNG image data
     * being processed.
     */

    /* Prior to 1.5.4 these tests were performed from png_set_gamma, 1.5.4 adds
     * png_set_alpha_mode and this is another source for a default file gamma so
     * the test needs to be performed later - here.  In addition prior to 1.5.4
     * the tests were repeated for the PALETTE color type here - this is no
     * longer necessary (and doesn't seem to have been necessary before.)
     *
     * PNGv3: the new mandatory precedence/priority rules for colour space chunks
     * are handled here (by calling the above function).
     *
     * Turn the gamma transformation on or off as appropriate.  Notice that
     * PNG_GAMMA just refers to the file->screen correction.  Alpha composition
     * may independently cause gamma correction because it needs linear data
     * (e.g. if the file has a gAMA chunk but the screen gamma hasn't been
     * specified.)  In any case this flag may get turned off in the code
     * immediately below if the transform can be handled outside the row loop.
     */
    if png_init_gamma_values(png_ptr) {
        png_ptr.transformations |= PNG_GAMMA;
    } else {
        png_ptr.transformations &= !PNG_GAMMA;
    }

    /* Certain transformations have the effect of preventing other
     * transformations that happen afterward in png_do_read_transformations;
     * resolve the interdependencies here.  From the code of
     * png_do_read_transformations the order is:
     *
     *  1) PNG_EXPAND (including PNG_EXPAND_tRNS)
     *  2) PNG_STRIP_ALPHA (if no compose)
     *  3) PNG_RGB_TO_GRAY
     *  4) PNG_GRAY_TO_RGB iff !PNG_BACKGROUND_IS_GRAY
     *  5) PNG_COMPOSE
     *  6) PNG_GAMMA
     *  7) PNG_STRIP_ALPHA (if compose)
     *  8) PNG_ENCODE_ALPHA
     *  9) PNG_SCALE_16_TO_8
     * 10) PNG_16_TO_8
     * 11) PNG_QUANTIZE (converts to palette)
     * 12) PNG_EXPAND_16
     * 13) PNG_GRAY_TO_RGB iff PNG_BACKGROUND_IS_GRAY
     * 14) PNG_INVERT_MONO
     * 15) PNG_INVERT_ALPHA
     * 16) PNG_SHIFT
     * 17) PNG_PACK
     * 18) PNG_BGR
     * 19) PNG_PACKSWAP
     * 20) PNG_FILLER (includes PNG_ADD_ALPHA)
     * 21) PNG_SWAP_ALPHA
     * 22) PNG_SWAP_BYTES
     * 23) PNG_USER_TRANSFORM [must be last]
     */
    /* (PNG_STRIP_ALPHA: not set) */

    /* If the screen gamma is about 1.0 then the OPTIMIZE_ALPHA and ENCODE_ALPHA
     * settings will have no effect.
     */
    if !png_gamma_significant(png_ptr.screen_gamma) {
        png_ptr.transformations &= !PNG_ENCODE_ALPHA;
        png_ptr.flags &= !PNG_FLAG_OPTIMIZE_ALPHA;
    }

    /* (PNG_RGB_TO_GRAY, PNG_BACKGROUND_EXPAND and PNG_COMPOSE: not set) */

    /* For indexed PNG data (PNG_COLOR_TYPE_PALETTE) many of the transformations
     * can be performed directly on the palette, and some (such as rgb to gray)
     * can be optimized inside the palette.  This is particularly true of the
     * composite (background and alpha) stuff, which can be pretty much all done
     * in the palette even if the result is expanded to RGB or gray afterward.
     *
     * NOTE: this is Not Yet Implemented, the code behaves as in 1.5.1 and
     * earlier and the palette stuff is actually handled on the first row.  This
     * leads to the reported bug that the palette returned by png_get_PLTE is not
     * updated.
     */
    if png_ptr.color_type == PNG_COLOR_TYPE_PALETTE {
        png_init_palette_transformations(png_ptr);
    } else {
        png_init_rgb_transformations(png_ptr);
    }

    /* (The background, gamma table and shift set up: for transformations
     * that are not set, and no gamma correction) */
}

/// Modify the info structure to reflect the transformations.  The
/// info should be updated so a PNG file could be written with it,
/// assuming the transformations result in valid PNG data.
/// (`png_read_transform_info`)
pub(crate) fn png_read_transform_info(
    png_ptr: &mut PngStruct<'_, '_>,
    info_ptr: &mut PngInfo,
) -> PngResult<()> {
    if info_ptr.color_type == PNG_COLOR_TYPE_PALETTE {
        if let (Some(info_palette), Some(palette)) =
            (info_ptr.palette.as_mut(), png_ptr.palette.as_ref())
        {
            /* Sync info_ptr->palette with png_ptr->palette, which may
             * have been modified by png_init_read_transformations
             * (e.g. for gamma correction or background compositing).
             */
            info_palette[..PNG_MAX_PALETTE_LENGTH]
                .copy_from_slice(&palette[..PNG_MAX_PALETTE_LENGTH]);
        }
    }

    if (png_ptr.transformations & PNG_EXPAND) != 0 {
        if info_ptr.color_type == PNG_COLOR_TYPE_PALETTE {
            /* This check must match what actually happens in
             * png_do_expand_palette; if it ever checks the tRNS chunk to see if
             * it is all opaque we must do the same (at present it does not.)
             */
            if png_ptr.num_trans > 0 {
                info_ptr.color_type = PNG_COLOR_TYPE_RGB_ALPHA;
            } else {
                info_ptr.color_type = PNG_COLOR_TYPE_RGB;
            }

            info_ptr.bit_depth = 8;
            info_ptr.num_trans = 0;

            if png_ptr.palette.is_none() {
                return Err(png_error(png_ptr, "Palette is NULL in indexed image"));
            }
        } else {
            if png_ptr.num_trans != 0 && (png_ptr.transformations & PNG_EXPAND_tRNS) != 0 {
                info_ptr.color_type |= PNG_COLOR_MASK_ALPHA;
            }
            if info_ptr.bit_depth < 8 {
                info_ptr.bit_depth = 8;
            }

            info_ptr.num_trans = 0;
        }
    }

    /* The following used to be conditional on PNG_GAMMA (prior to 1.5.4),
     * however it seems that the code in png_init_read_transformations, which has
     * been called before this from png_read_update_info->png_read_start_row
     * sometimes does the gamma transform and cancels the flag.
     *
     * TODO: this is confusing.  It only changes the result of png_get_gAMA and,
     * yes, it does return the value that the transformed data effectively has
     * but does any app really understand this?
     */
    info_ptr.gamma = png_ptr.file_gamma;

    if info_ptr.bit_depth == 16 {
        if (png_ptr.transformations & PNG_SCALE_16_TO_8) != 0 {
            info_ptr.bit_depth = 8;
        }

        if (png_ptr.transformations & PNG_16_TO_8) != 0 {
            info_ptr.bit_depth = 8;
        }
    }

    if (png_ptr.transformations & PNG_GRAY_TO_RGB) != 0 {
        info_ptr.color_type |= PNG_COLOR_MASK_COLOR;
    }

    if (png_ptr.transformations & PNG_RGB_TO_GRAY) != 0 {
        info_ptr.color_type &= !PNG_COLOR_MASK_COLOR;
    }

    /* (PNG_QUANTIZE: not set) */

    if (png_ptr.transformations & PNG_EXPAND_16) != 0
        && info_ptr.bit_depth == 8
        && info_ptr.color_type != PNG_COLOR_TYPE_PALETTE
    {
        info_ptr.bit_depth = 16;
    }

    if (png_ptr.transformations & PNG_PACK) != 0 && (info_ptr.bit_depth < 8) {
        info_ptr.bit_depth = 8;
    }

    if info_ptr.color_type == PNG_COLOR_TYPE_PALETTE {
        info_ptr.channels = 1;
    } else if (info_ptr.color_type & PNG_COLOR_MASK_COLOR) != 0 {
        info_ptr.channels = 3;
    } else {
        info_ptr.channels = 1;
    }

    if (png_ptr.transformations & PNG_STRIP_ALPHA) != 0 {
        info_ptr.color_type &= !PNG_COLOR_MASK_ALPHA;
        info_ptr.num_trans = 0;
    }

    if (info_ptr.color_type & PNG_COLOR_MASK_ALPHA) != 0 {
        info_ptr.channels += 1;
    }

    /* STRIP_ALPHA and FILLER allowed:  MASK_ALPHA bit stripped above */
    if (png_ptr.transformations & PNG_FILLER) != 0
        && (info_ptr.color_type == PNG_COLOR_TYPE_RGB || info_ptr.color_type == PNG_COLOR_TYPE_GRAY)
    {
        info_ptr.channels += 1;
        /* If adding a true alpha channel not just filler */
        if (png_ptr.transformations & PNG_ADD_ALPHA) != 0 {
            info_ptr.color_type |= PNG_COLOR_MASK_ALPHA;
        }
    }

    /* (PNG_USER_TRANSFORM: not set) */

    info_ptr.pixel_depth = info_ptr.channels.wrapping_mul(info_ptr.bit_depth);

    info_ptr.rowbytes = png_rowbytes(info_ptr.pixel_depth as u32, info_ptr.width as usize);

    /* Adding in 1.5.4: cache the above value in png_struct so that we can later
     * check in png_rowbytes that the user buffer won't get overwritten.  Note
     * that the field is not always set - if png_read_update_info isn't called
     * the application has to either not do any transforms or get the calculation
     * right itself.
     */
    png_ptr.info_rowbytes = info_ptr.rowbytes;
    Ok(())
}

/// Simply discard the low byte.  This was the default behavior prior
/// to libpng-1.5.4. (`png_do_chop`)
fn png_do_chop(row_info: &mut PngRowInfo, row: &mut [u8]) {
    if row_info.bit_depth == 16 {
        let mut sp = 0; /* source */
        let mut dp = 0; /* destination */
        let ep = row_info.rowbytes; /* end+1 */

        while sp < ep {
            row[dp] = row[sp];
            dp += 1;
            sp += 2; /* skip low byte */
        }

        row_info.bit_depth = 8;
        row_info.pixel_depth = 8u8.wrapping_mul(row_info.channels);
        row_info.rowbytes = row_info.width as usize * row_info.channels as usize;
    }
}

/// Add filler channel if we have RGB color (`png_do_read_filler`)
fn png_do_read_filler(row_info: &mut PngRowInfo, row: &mut [u8], filler: u32, flags: u32) {
    let row_width = row_info.width as usize;

    let hi_filler = (filler >> 8) as u8;
    let lo_filler = filler as u8;

    /* The copies run backwards, from the end of the row (`sp` and `dp`
     * are one past the bytes they copy next). */
    if row_info.color_type == PNG_COLOR_TYPE_GRAY {
        if row_info.bit_depth == 8 {
            if (flags & PNG_FLAG_FILLER_AFTER) != 0 {
                /* This changes the data from G to GX */
                let mut sp = row_width;
                let mut dp = sp + row_width;
                for _ in 1..row_width {
                    dp -= 1;
                    row[dp] = lo_filler;
                    dp -= 1;
                    sp -= 1;
                    row[dp] = row[sp];
                }
                dp -= 1;
                row[dp] = lo_filler;
                row_info.channels = 2;
                row_info.pixel_depth = 16;
                row_info.rowbytes = row_width * 2;
            } else {
                /* This changes the data from G to XG */
                let mut sp = row_width;
                let mut dp = sp + row_width;
                for _ in 0..row_width {
                    dp -= 1;
                    sp -= 1;
                    row[dp] = row[sp];
                    dp -= 1;
                    row[dp] = lo_filler;
                }
                row_info.channels = 2;
                row_info.pixel_depth = 16;
                row_info.rowbytes = row_width * 2;
            }
        } else if row_info.bit_depth == 16 {
            if (flags & PNG_FLAG_FILLER_AFTER) != 0 {
                /* This changes the data from GG to GGXX */
                let mut sp = row_width * 2;
                let mut dp = sp + row_width * 2;
                for _ in 1..row_width {
                    dp -= 1;
                    row[dp] = lo_filler;
                    dp -= 1;
                    row[dp] = hi_filler;
                    for _ in 0..2 {
                        dp -= 1;
                        sp -= 1;
                        row[dp] = row[sp];
                    }
                }
                dp -= 1;
                row[dp] = lo_filler;
                dp -= 1;
                row[dp] = hi_filler;
                row_info.channels = 2;
                row_info.pixel_depth = 32;
                row_info.rowbytes = row_width * 4;
            } else {
                /* This changes the data from GG to XXGG */
                let mut sp = row_width * 2;
                let mut dp = sp + row_width * 2;
                for _ in 0..row_width {
                    for _ in 0..2 {
                        dp -= 1;
                        sp -= 1;
                        row[dp] = row[sp];
                    }
                    dp -= 1;
                    row[dp] = lo_filler;
                    dp -= 1;
                    row[dp] = hi_filler;
                }
                row_info.channels = 2;
                row_info.pixel_depth = 32;
                row_info.rowbytes = row_width * 4;
            }
        }
    }
    /* COLOR_TYPE == GRAY */
    else if row_info.color_type == PNG_COLOR_TYPE_RGB {
        if row_info.bit_depth == 8 {
            if (flags & PNG_FLAG_FILLER_AFTER) != 0 {
                /* This changes the data from RGB to RGBX */
                let mut sp = row_width * 3;
                let mut dp = sp + row_width;
                for _ in 1..row_width {
                    dp -= 1;
                    row[dp] = lo_filler;
                    for _ in 0..3 {
                        dp -= 1;
                        sp -= 1;
                        row[dp] = row[sp];
                    }
                }
                dp -= 1;
                row[dp] = lo_filler;
                row_info.channels = 4;
                row_info.pixel_depth = 32;
                row_info.rowbytes = row_width * 4;
            } else {
                /* This changes the data from RGB to XRGB */
                let mut sp = row_width * 3;
                let mut dp = sp + row_width;
                for _ in 0..row_width {
                    for _ in 0..3 {
                        dp -= 1;
                        sp -= 1;
                        row[dp] = row[sp];
                    }
                    dp -= 1;
                    row[dp] = lo_filler;
                }
                row_info.channels = 4;
                row_info.pixel_depth = 32;
                row_info.rowbytes = row_width * 4;
            }
        } else if row_info.bit_depth == 16 {
            if (flags & PNG_FLAG_FILLER_AFTER) != 0 {
                /* This changes the data from RRGGBB to RRGGBBXX */
                let mut sp = row_width * 6;
                let mut dp = sp + row_width * 2;
                for _ in 1..row_width {
                    dp -= 1;
                    row[dp] = lo_filler;
                    dp -= 1;
                    row[dp] = hi_filler;
                    for _ in 0..6 {
                        dp -= 1;
                        sp -= 1;
                        row[dp] = row[sp];
                    }
                }
                dp -= 1;
                row[dp] = lo_filler;
                dp -= 1;
                row[dp] = hi_filler;
                row_info.channels = 4;
                row_info.pixel_depth = 64;
                row_info.rowbytes = row_width * 8;
            } else {
                /* This changes the data from RRGGBB to XXRRGGBB */
                let mut sp = row_width * 6;
                let mut dp = sp + row_width * 2;
                for _ in 0..row_width {
                    for _ in 0..6 {
                        dp -= 1;
                        sp -= 1;
                        row[dp] = row[sp];
                    }
                    dp -= 1;
                    row[dp] = lo_filler;
                    dp -= 1;
                    row[dp] = hi_filler;
                }

                row_info.channels = 4;
                row_info.pixel_depth = 64;
                row_info.rowbytes = row_width * 8;
            }
        }
    } /* COLOR_TYPE == RGB */
}

/// Expands a palette row to an RGB or RGBA row depending
/// upon whether you supply trans and num_trans. (`png_do_expand_palette`)
fn png_do_expand_palette(
    row_info: &mut PngRowInfo,
    row: &mut [u8],
    palette: &[PngColor],
    trans_alpha: Option<&[u8]>,
    num_trans: i32,
) {
    let mut shift: i32;
    let mut value: i32;
    let row_width = row_info.width as usize;

    if row_info.color_type == PNG_COLOR_TYPE_PALETTE {
        if row_info.bit_depth < 8 {
            match row_info.bit_depth {
                1 => {
                    let mut sp = (row_width - 1) >> 3;
                    let mut dp = row_width - 1;
                    shift = 7 - ((row_width + 7) & 0x07) as i32;
                    for i in 0..row_width {
                        if ((row[sp] >> shift) & 0x01) != 0 {
                            row[dp] = 1;
                        } else {
                            row[dp] = 0;
                        }

                        if shift == 7 {
                            shift = 0;
                            sp = sp.wrapping_sub(1);
                        } else {
                            shift += 1;
                        }

                        if i + 1 < row_width {
                            dp -= 1;
                        }
                    }
                }

                2 => {
                    let mut sp = (row_width - 1) >> 2;
                    let mut dp = row_width - 1;
                    shift = ((3 - ((row_width + 3) & 0x03)) << 1) as i32;
                    for i in 0..row_width {
                        value = ((row[sp] >> shift) & 0x03) as i32;
                        row[dp] = value as u8;
                        if shift == 6 {
                            shift = 0;
                            sp = sp.wrapping_sub(1);
                        } else {
                            shift += 2;
                        }

                        if i + 1 < row_width {
                            dp -= 1;
                        }
                    }
                }

                4 => {
                    let mut sp = (row_width - 1) >> 1;
                    let mut dp = row_width - 1;
                    shift = ((row_width & 0x01) << 2) as i32;
                    for i in 0..row_width {
                        value = ((row[sp] >> shift) & 0x0f) as i32;
                        row[dp] = value as u8;
                        if shift == 4 {
                            shift = 0;
                            sp = sp.wrapping_sub(1);
                        } else {
                            shift += 4;
                        }

                        if i + 1 < row_width {
                            dp -= 1;
                        }
                    }
                }

                _ => {}
            }
            row_info.bit_depth = 8;
            row_info.pixel_depth = 8;
            row_info.rowbytes = row_width;
        }

        if row_info.bit_depth == 8 {
            if num_trans > 0 {
                let trans_alpha = trans_alpha.unwrap_or(&[]);
                let mut sp = row_width; /* (one past *sp) */
                let mut dp = row_width << 2; /* (one past *dp) */

                /* (PNG_ARM_NEON_INTRINSICS: not on this platform) */
                for _ in 0..row_width {
                    sp -= 1;
                    let s = row[sp] as usize;
                    dp -= 1;
                    if (s as i32) >= num_trans {
                        row[dp] = 0xff;
                    } else {
                        row[dp] = trans_alpha[s];
                    }
                    dp -= 1;
                    row[dp] = palette[s].blue;
                    dp -= 1;
                    row[dp] = palette[s].green;
                    dp -= 1;
                    row[dp] = palette[s].red;
                }
                row_info.bit_depth = 8;
                row_info.pixel_depth = 32;
                row_info.rowbytes = row_width * 4;
                row_info.color_type = 6;
                row_info.channels = 4;
            } else {
                let mut sp = row_width;
                let mut dp = row_width * 3;

                for _ in 0..row_width {
                    sp -= 1;
                    let s = row[sp] as usize;
                    dp -= 1;
                    row[dp] = palette[s].blue;
                    dp -= 1;
                    row[dp] = palette[s].green;
                    dp -= 1;
                    row[dp] = palette[s].red;
                }

                row_info.bit_depth = 8;
                row_info.pixel_depth = 24;
                row_info.rowbytes = row_width * 3;
                row_info.color_type = 2;
                row_info.channels = 3;
            }
        }
    }
}

/// Transform the row.  The order of transformations is significant,
/// and is very touchy.  If you add a transformation, take care to
/// decide how it fits in with the other transformations here.
/// (`png_do_read_transformations`, for the transformations that are
/// translated; the row is `png_ptr->row_buf`)
pub(crate) fn png_do_read_transformations(
    png_ptr: &mut PngStruct<'_, '_>,
    row_info: &mut PngRowInfo,
) -> PngResult<()> {
    if png_ptr.row_buf.is_empty() {
        /* Prior to 1.5.4 this output row/pass where the NULL pointer is, but this
         * error is incredibly rare and incredibly easy to debug without this
         * information.
         */
        return Err(png_error(png_ptr, "NULL row buffer"));
    }

    /* The following is debugging; prior to 1.5.4 the code was never compiled in;
     * in 1.5.4 PNG_FLAG_DETECT_UNINITIALIZED was added and the macro
     * PNG_WARN_UNINITIALIZED_ROW removed.  In 1.6 the new flag is set only for
     * all transformations, however in practice the ROW_INIT always gets done on
     * demand, if necessary.
     */
    if (png_ptr.flags & PNG_FLAG_DETECT_UNINITIALIZED) != 0
        && (png_ptr.flags & PNG_FLAG_ROW_INIT) == 0
    {
        /* Application has failed to call either png_read_start_image() or
         * png_read_update_info() after setting transforms that expand pixels.
         * This check added to libpng-1.2.19 (but not enabled until 1.5.4).
         */
        return Err(png_error(png_ptr, "Uninitialized row"));
    }

    let mut row_buf = std::mem::take(&mut png_ptr.row_buf);
    let row = &mut row_buf[1..];

    if (png_ptr.transformations & PNG_EXPAND) != 0 && row_info.color_type == PNG_COLOR_TYPE_PALETTE
    {
        /* (PNG_ARM_NEON_INTRINSICS: not on this platform) */
        png_do_expand_palette(
            row_info,
            row,
            png_ptr.palette.as_deref().unwrap_or(&[]),
            png_ptr.trans_alpha.as_deref(),
            png_ptr.num_trans as i32,
        );
    }
    /* (png_do_expand(), for other color types: SDL_image only expands
     * palettes) */

    /* (PNG_STRIP_ALPHA, PNG_RGB_TO_GRAY, PNG_GRAY_TO_RGB, PNG_COMPOSE,
     * PNG_GAMMA, PNG_ENCODE_ALPHA and PNG_SCALE_16_TO_8: not set) */

    /* There is no harm in doing both of these because only one has any effect,
     * by putting the 'scale' option first if the app asks for scale (either by
     * calling the API or in a TRANSFORM flag) this is what happens.
     */
    if (png_ptr.transformations & PNG_16_TO_8) != 0 {
        png_do_chop(row_info, row);
    }

    /* (PNG_QUANTIZE, PNG_EXPAND_16, PNG_GRAY_TO_RGB, PNG_INVERT_MONO,
     * PNG_INVERT_ALPHA, PNG_SHIFT and PNG_PACK: not set) */

    /* (Added at libpng-1.5.10: png_do_check_palette_indexes() for rows still
     * color-mapped here, which SDL_image's never are, as it expands them) */

    /* (PNG_BGR and PNG_PACKSWAP: not set) */

    if (png_ptr.transformations & PNG_FILLER) != 0 {
        png_do_read_filler(row_info, row, png_ptr.filler as u32, png_ptr.flags);
    }

    /* (PNG_SWAP_ALPHA, PNG_SWAP_BYTES and PNG_USER_TRANSFORM: not set) */

    png_ptr.row_buf = row_buf;
    Ok(())
}
