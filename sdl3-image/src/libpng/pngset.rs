// Rust translation of pngset.c from libpng 1.6.59 (the IHDR, PLTE and tRNS
// storage functions).
// Copyright (c) 2018-2026 Cosmin Truta
// Copyright (c) 1998-2018 Glenn Randers-Pehrson
// Copyright (c) 1996-1997 Andreas Dilger
// Copyright (c) 1995-1996 Guy Eric Schalnat, Group 42, Inc.
// This code is released under the libpng license (see LICENSE.txt).
// This is an altered (translated) version of the original software.

//! pngset.c - storage of image information into info struct
//!
//! The functions here are used during reads to store data from the file
//! into the info struct, and during writes to store application data
//! into the info struct for writing into the file.  This abstracts the
//! info struct and allows us to change the structure in the future.

use super::png::*;
use super::pngerror::{png_error, png_warning};
use super::pnginfo::PngInfo;
use super::pngpriv::*;
use super::pngstruct::PngStruct;

/// `png_set_IHDR`
#[allow(non_snake_case, clippy::too_many_arguments)]
pub(crate) fn png_set_IHDR(
    png_ptr: &PngStruct<'_, '_>,
    info_ptr: &mut PngInfo,
    width: u32,
    height: u32,
    bit_depth: i32,
    color_type: i32,
    interlace_type: i32,
    compression_type: i32,
    filter_type: i32,
) -> PngResult<()> {
    info_ptr.width = width;
    info_ptr.height = height;
    info_ptr.bit_depth = bit_depth as u8;
    info_ptr.color_type = color_type as u8;
    info_ptr.compression_type = compression_type as u8;
    info_ptr.filter_type = filter_type as u8;
    info_ptr.interlace_type = interlace_type as u8;

    png_check_IHDR(
        png_ptr,
        info_ptr.width,
        info_ptr.height,
        info_ptr.bit_depth as i32,
        info_ptr.color_type as i32,
        info_ptr.interlace_type as i32,
        info_ptr.compression_type as i32,
        info_ptr.filter_type as i32,
    )?;

    if info_ptr.color_type == PNG_COLOR_TYPE_PALETTE {
        info_ptr.channels = 1;
    } else if (info_ptr.color_type & PNG_COLOR_MASK_COLOR) != 0 {
        info_ptr.channels = 3;
    } else {
        info_ptr.channels = 1;
    }

    if (info_ptr.color_type & PNG_COLOR_MASK_ALPHA) != 0 {
        info_ptr.channels += 1;
    }

    info_ptr.pixel_depth = info_ptr.channels.wrapping_mul(info_ptr.bit_depth);

    info_ptr.rowbytes = png_rowbytes(info_ptr.pixel_depth as u32, width as usize);
    Ok(())
}

/// A zero-filled palette of `PNG_MAX_PALETTE_LENGTH` entries
/// (`png_calloc(png_ptr, PNG_MAX_PALETTE_LENGTH * (sizeof (png_color)))`).
fn png_calloc_palette(png_ptr: &PngStruct<'_, '_>) -> PngResult<Vec<PngColor>> {
    let mut v = Vec::new();
    if v.try_reserve_exact(PNG_MAX_PALETTE_LENGTH).is_err() {
        return Err(png_error(png_ptr, "Out of memory"));
    }
    v.resize(PNG_MAX_PALETTE_LENGTH, PngColor::default());
    Ok(v)
}

/// `png_set_PLTE`
#[allow(non_snake_case)]
pub(crate) fn png_set_PLTE(
    png_ptr: &mut PngStruct<'_, '_>,
    info_ptr: &mut PngInfo,
    palette: &[PngColor],
    num_palette: i32,
) -> PngResult<()> {
    let max_palette_length: u32 = if info_ptr.color_type == PNG_COLOR_TYPE_PALETTE {
        1 << info_ptr.bit_depth
    } else {
        PNG_MAX_PALETTE_LENGTH as u32
    };

    if num_palette < 0 || num_palette > max_palette_length as i32 {
        if info_ptr.color_type == PNG_COLOR_TYPE_PALETTE {
            return Err(png_error(png_ptr, "Invalid palette length"));
        } else {
            png_warning(png_ptr, "Invalid palette length");

            return Ok(());
        }
    }

    if (num_palette > 0 && palette.len() < num_palette as usize)
        || (num_palette == 0 && (png_ptr.mng_features_permitted & PNG_FLAG_MNG_EMPTY_PLTE) == 0)
    {
        return Err(png_error(png_ptr, "Invalid palette"));
    }

    /* Snapshot the caller's palette before freeing, in case it points to
     * info_ptr->palette (getter-to-setter aliasing).
     */
    let n = num_palette as usize;
    let safe_palette = &palette[..n];

    /* (png_free_data(png_ptr, info_ptr, PNG_FREE_PLTE, 0)) */
    info_ptr.palette = None;
    info_ptr.num_palette = 0;
    info_ptr.valid &= !PNG_INFO_PLTE;

    /* Changed in libpng-1.2.1 to allocate PNG_MAX_PALETTE_LENGTH instead
     * of num_palette entries, in case of an invalid PNG file or incorrect
     * call to png_set_PLTE() with too-large sample values.
     *
     * Allocate independent buffers for info_ptr and png_ptr so that the
     * lifetime of png_ptr->palette is decoupled from the lifetime of
     * info_ptr->palette.  Previously, these two pointers were aliased,
     * which caused a use-after-free vulnerability if png_free_data freed
     * info_ptr->palette while png_ptr->palette was still in use by the
     * row transform functions (e.g. png_do_expand_palette).
     *
     * Both buffers are allocated with png_calloc to zero-fill, because
     * the ARM NEON palette riffle reads all 256 entries unconditionally,
     * regardless of num_palette.
     */
    png_ptr.palette = None;
    let mut png_palette = png_calloc_palette(png_ptr)?;
    let mut info_palette = png_calloc_palette(png_ptr)?;
    png_ptr.num_palette = num_palette as u16;
    info_ptr.num_palette = num_palette as u16;

    if num_palette > 0 {
        info_palette[..n].copy_from_slice(safe_palette);
        png_palette[..n].copy_from_slice(safe_palette);
    }
    png_ptr.palette = Some(png_palette);
    info_ptr.palette = Some(info_palette);

    info_ptr.valid |= PNG_INFO_PLTE;
    Ok(())
}

/// `png_set_tRNS`
#[allow(non_snake_case)]
pub(crate) fn png_set_tRNS(
    png_ptr: &mut PngStruct<'_, '_>,
    info_ptr: &mut PngInfo,
    trans_alpha: Option<&[u8]>,
    mut num_trans: i32,
    trans_color: Option<&PngColor16>,
) -> PngResult<()> {
    if let Some(trans_alpha) = trans_alpha {
        /* Snapshot the caller's trans_alpha before freeing, in case it
         * points to info_ptr->trans_alpha (getter-to-setter aliasing).
         */
        let mut safe_trans = [0u8; PNG_MAX_PALETTE_LENGTH];

        if num_trans > 0 && num_trans <= PNG_MAX_PALETTE_LENGTH as i32 {
            safe_trans[..num_trans as usize].copy_from_slice(&trans_alpha[..num_trans as usize]);
        }

        /* (png_free_data(png_ptr, info_ptr, PNG_FREE_TRNS, 0)) */
        info_ptr.trans_alpha = None;
        info_ptr.num_trans = 0;
        info_ptr.valid &= !PNG_INFO_tRNS;

        if num_trans > 0 && num_trans <= PNG_MAX_PALETTE_LENGTH as i32 {
            let n = num_trans as usize;

            /* Allocate info_ptr's copy of the transparency data.
             * Initialize all entries to fully opaque (0xff), then overwrite
             * the first num_trans entries with the actual values.
             */
            let mut info_trans = super::pngmem::png_malloc(png_ptr, PNG_MAX_PALETTE_LENGTH)?;
            info_trans.fill(0xff);
            info_trans[..n].copy_from_slice(&safe_trans[..n]);
            info_ptr.trans_alpha = Some(info_trans);
            info_ptr.valid |= PNG_INFO_tRNS;

            /* Allocate an independent copy for png_struct, so that the
             * lifetime of png_ptr->trans_alpha is decoupled from the
             * lifetime of info_ptr->trans_alpha.  Previously these two
             * pointers were aliased, which caused a use-after-free if
             * png_free_data freed info_ptr->trans_alpha while
             * png_ptr->trans_alpha was still in use by the row transform
             * functions (e.g. png_do_expand_palette).
             */
            png_ptr.trans_alpha = None;
            let mut png_trans = super::pngmem::png_malloc(png_ptr, PNG_MAX_PALETTE_LENGTH)?;
            png_trans.fill(0xff);
            png_trans[..n].copy_from_slice(&safe_trans[..n]);
            png_ptr.trans_alpha = Some(png_trans);
        } else {
            png_ptr.trans_alpha = None;
        }
    }

    if let Some(trans_color) = trans_color {
        if info_ptr.bit_depth < 16 {
            let sample_max: i32 = (1 << info_ptr.bit_depth) - 1;

            if (info_ptr.color_type == PNG_COLOR_TYPE_GRAY && trans_color.gray as i32 > sample_max)
                || (info_ptr.color_type == PNG_COLOR_TYPE_RGB
                    && (trans_color.red as i32 > sample_max
                        || trans_color.green as i32 > sample_max
                        || trans_color.blue as i32 > sample_max))
            {
                png_warning(png_ptr, "tRNS chunk has out-of-range samples for bit_depth");
            }
        }

        info_ptr.trans_color = *trans_color;

        if num_trans == 0 {
            num_trans = 1;
        }
    }

    info_ptr.num_trans = num_trans as u16;

    if num_trans != 0 {
        info_ptr.valid |= PNG_INFO_tRNS;
    }
    Ok(())
}
