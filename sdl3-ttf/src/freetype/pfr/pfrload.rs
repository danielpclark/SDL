// Rust translation of src/pfr/pfrload.c and src/pfr/pfrload.h from
// FreeType (2.13.2, as SDL_ttf's external/freetype pins it).
// Copyright (C) 2002-2023 by David Turner, Robert Wilhelm, and Werner Lemberg.
// This is an altered (translated) version of the original software; it is
// used under the FreeType License (see FTL.TXT and LICENSE.txt).

//! FreeType PFR loader (body).
//!
//! The `FT_Byte*` cursors into a stream frame are offsets into the frame's
//! bytes; `PFR_CONFIG_NO_CHECKS` is not defined, so `PFR_CHECK` checks.

use super::super::base::ftmemory::{ft_new_array, ft_qalloc, ft_renew_array};
use super::super::base::ftstream::*;
use super::super::fttypes::*;
use super::pfrtypes::*;

/* pfrload.h */

/// `PFR_CHECK_SIZE` (and `PFR_CHECK`): whether `x` more bytes from `p`
/// stay within `limit`
#[inline]
pub fn pfr_check(p: usize, x: u64, limit: usize) -> bool {
    p as u64 + x <= limit as u64
}

/// `PFR_NEXT_BYTE`
#[inline]
pub fn pfr_next_byte(b: &[u8], p: &mut usize) -> FtUInt {
    ft_next_byte(b, p) as FtUInt
}

/// `PFR_NEXT_INT8`
#[inline]
pub fn pfr_next_int8(b: &[u8], p: &mut usize) -> FtInt {
    ft_next_char(b, p) as FtInt
}

/// `PFR_NEXT_SHORT`
#[inline]
pub fn pfr_next_short(b: &[u8], p: &mut usize) -> FtInt {
    ft_next_short(b, p) as FtInt
}

/// `PFR_NEXT_USHORT`
#[inline]
pub fn pfr_next_ushort(b: &[u8], p: &mut usize) -> FtUInt {
    ft_next_ushort(b, p) as FtUInt
}

/// `PFR_NEXT_LONG`
#[inline]
pub fn pfr_next_long(b: &[u8], p: &mut usize) -> FtInt32 {
    ft_next_off3(b, p)
}

/// `PFR_NEXT_ULONG`
#[inline]
pub fn pfr_next_ulong(b: &[u8], p: &mut usize) -> FtUInt32 {
    ft_next_uoff3(b, p)
}

/* handling extra items */

/// `PFR_ExtraItem_ParseFunc`: parses the item from `p` to `limit` in
/// `base` (a stream frame)
pub type PfrExtraItemParseFunc =
    fn(base: &[u8], p: usize, limit: usize, data: &mut PfrPhyFontRec) -> FtResult<()>;

/// `PFR_ExtraItemRec`
#[derive(Debug, Clone, Copy)]
pub struct PfrExtraItemRec {
    pub type_: FtUInt,
    pub parser: PfrExtraItemParseFunc,
}

/*
 * The overall structure of a PFR file is as follows.
 *
 *   PFR header
 *     58 bytes (contains nPhysFonts)
 *
 *   Logical font directory (size at most 2^16 bytes)
 *     2 bytes (nLogFonts)
 *     + nLogFonts * 5 bytes
 *
 *        ==>  nLogFonts <= 13106
 *
 *   Logical font section (size at most 2^24 bytes)
 *     nLogFonts * logFontRecord
 *
 *     logFontRecord (size at most 2^16 bytes)
 *       12 bytes (fontMatrix)
 *       + 1 byte (flags)
 *       + 0-5 bytes (depending on `flags')
 *       + 0-(1+255*(2+255)) = 0-65536 (depending on `flags')
 *       + 5 bytes (physical font info)
 *       + 0-1 bytes (depending on PFR header)
 *
 *        ==>  minimum size 18 bytes
 *
 *   Physical font section (size at most 2^24 bytes)
 *     nPhysFonts * (physFontRecord
 *                   + nBitmapSizes * nBmapChars * bmapCharRecord)
 *
 *     physFontRecord (size at most 2^24 bytes)
 *       14 bytes (font info)
 *       + 1 byte (flags)
 *       + 0-2 (depending on `flags')
 *       + 0-? (structure too complicated to be shown here; depending on
 *              `flags'; contains `nBitmapSizes' and `nBmapChars')
 *       + 3 bytes (nAuxBytes)
 *       + nAuxBytes
 *       + 1 byte (nBlueValues)
 *       + 2 * nBlueValues
 *       + 6 bytes (hinting data)
 *       + 2 bytes (nCharacters)
 *       + nCharacters * (4-10 bytes) (depending on `flags')
 *
 *        ==>  minimum size 27 bytes
 *
 *     bmapCharRecord
 *       4-7 bytes
 *
 *   Glyph program strings (three possible types: simpleGps, compoundGps,
 *                          and bitmapGps; size at most 2^24 bytes)
 *     simpleGps (size at most 2^16 bytes)
 *       1 byte (flags)
 *       1-2 bytes (n[XY]orus, depending on `flags')
 *       0-(64+512*2) = 0-1088 bytes (depending on `n[XY]orus')
 *       0-? (structure too complicated to be shown here; depending on
 *            `flags')
 *       1-? glyph data (faintly resembling PS Type 1 charstrings)
 *
 *        ==>  minimum size 3 bytes
 *
 *     compoundGps (size at most 2^16 bytes)
 *       1 byte (nElements <= 63, flags)
 *       + 0-(1+255*(2+255)) = 0-65536 (depending on `flags')
 *       + nElements * (6-14 bytes)
 *
 *     bitmapGps (size at most 2^16 bytes)
 *       1 byte (flags)
 *       3-13 bytes (position info, depending on `flags')
 *       0-? bitmap data
 *
 *        ==>  minimum size 4 bytes
 *
 *   PFR trailer
 *       8 bytes
 *
 *
 * ==>  minimum size of a valid PFR:
 *        58 (header)
 *        + 2 (nLogFonts)
 *        + 27 (1 physFontRecord)
 *        + 8 (trailer)
 *       -----
 *        95 bytes
 *
 */

/*************************************************************************/
/*************************************************************************/
/*****                                                               *****/
/*****                          EXTRA ITEMS                          *****/
/*****                                                               *****/
/*************************************************************************/
/*************************************************************************/

/// `pfr_extra_items_skip`
pub fn pfr_extra_items_skip(base: &[u8], pp: &mut usize, limit: usize) -> FtResult<()> {
    pfr_extra_items_parse(base, pp, limit, None, None)
}

/// `pfr_extra_items_parse`
pub fn pfr_extra_items_parse(
    base: &[u8],
    pp: &mut usize,
    limit: usize,
    item_list: Option<&[PfrExtraItemRec]>,
    mut item_data: Option<&mut PfrPhyFontRec>,
) -> FtResult<()> {
    let mut error: FtResult<()> = Ok(());
    let mut p = *pp;

    'exit: {
        'too_short: {
            if !pfr_check(p, 1, limit) {
                break 'too_short;
            }
            let mut num_items = pfr_next_byte(base, &mut p);

            while num_items > 0 {
                if !pfr_check(p, 2, limit) {
                    break 'too_short;
                }
                let item_size = pfr_next_byte(base, &mut p);
                let item_type = pfr_next_byte(base, &mut p);

                if !pfr_check(p, item_size as u64, limit) {
                    break 'too_short;
                }

                if let Some(item_list) = item_list {
                    for extra in item_list {
                        if extra.type_ == item_type {
                            if let Some(data) = item_data.as_deref_mut() {
                                error = (extra.parser)(base, p, p + item_size as usize, data);
                            }
                            if error.is_err() {
                                break 'exit;
                            }
                            break;
                        }
                    }
                }

                p += item_size as usize;
                num_items -= 1;
            }
            break 'exit;
        }

        /* Too_Short: */
        error = Err(FT_ERR_INVALID_TABLE);
    }

    /* Exit: */
    *pp = p;
    error
}

/*************************************************************************/
/*************************************************************************/
/*****                                                               *****/
/*****                          PFR HEADER                           *****/
/*****                                                               *****/
/*************************************************************************/
/*************************************************************************/

/// `pfr_header_load` (the `pfr_header_fields` frame of 58 bytes, read)
pub fn pfr_header_load(header: &mut PfrHeaderRec, stream: &mut FtStreamRec) -> FtResult<()> {
    /* read header directly */
    stream.seek(0)?;
    stream.enter_frame(58)?;

    header.signature = stream.get_ulong();
    header.version = stream.get_ushort() as FtUInt;
    header.signature2 = stream.get_ushort() as FtUInt;
    header.header_size = stream.get_ushort() as FtUInt;
    header.log_dir_size = stream.get_ushort() as FtUInt;
    header.log_dir_offset = stream.get_ushort() as FtUInt;
    header.log_font_max_size = stream.get_ushort() as FtUInt;
    header.log_font_section_size = stream.get_uoffset();
    header.log_font_section_offset = stream.get_uoffset();
    header.phy_font_max_size = stream.get_ushort() as FtUInt32;
    header.phy_font_section_size = stream.get_uoffset();
    header.phy_font_section_offset = stream.get_uoffset();
    header.gps_max_size = stream.get_ushort() as FtUInt;
    header.gps_section_size = stream.get_uoffset();
    header.gps_section_offset = stream.get_uoffset();
    header.max_blue_values = stream.get_byte() as FtUInt;
    header.max_x_orus = stream.get_byte() as FtUInt;
    header.max_y_orus = stream.get_byte() as FtUInt;
    header.phy_font_max_size_high = stream.get_byte() as FtUInt;
    header.color_flags = stream.get_byte() as FtUInt;
    header.bct_max_size = stream.get_uoffset();
    header.bct_set_max_size = stream.get_uoffset();
    header.phy_bct_set_max_size = stream.get_uoffset();
    header.num_phy_fonts = stream.get_ushort() as FtUInt;
    header.max_vert_stem_snap = stream.get_byte() as FtUInt;
    header.max_horz_stem_snap = stream.get_byte() as FtUInt;
    header.max_chars = stream.get_ushort() as FtUInt;

    stream.exit_frame();

    /* make a few adjustments to the header */
    header.phy_font_max_size = header
        .phy_font_max_size
        .wrapping_add(header.phy_font_max_size_high << 16);

    Ok(())
}

/// `pfr_header_check`
pub fn pfr_header_check(header: &PfrHeaderRec) -> bool {
    let mut result = true;

    /* check signature and header size */
    if header.signature != 0x50465230 ||   /* "PFR0" */
       header.version > 4 ||
       header.header_size < 58 ||
       header.signature2 != 0x0D0A
    /* CR/LF  */
    {
        result = false;
    }

    result
}

/***********************************************************************/
/***********************************************************************/
/*****                                                             *****/
/*****                    PFR LOGICAL FONTS                        *****/
/*****                                                             *****/
/***********************************************************************/
/***********************************************************************/

/// `pfr_log_font_count`: return number of logical fonts in this file
pub fn pfr_log_font_count(
    stream: &mut FtStreamRec,
    section_offset: FtUInt32,
    acount: &mut FtLong,
) -> FtResult<()> {
    let mut result: FtUInt = 0;

    let error = (|| {
        stream.seek(section_offset as FtULong)?;
        let count = stream.read_ushort()? as FtUInt;

        /* check maximum value and a rough minimum size:     */
        /* - no more than 13106 log fonts                    */
        /* - we need 5 bytes for a log header record         */
        /* - we need at least 18 bytes for a log font record */
        /* - the overall size is at least 95 bytes plus the  */
        /*   log header and log font records                 */
        if count > ((1 << 16) - 2) / 5
            || 2 + count as FtULong * 5 >= stream.size.wrapping_sub(section_offset as FtULong)
            || 95 + count as FtULong * (5 + 18) >= stream.size
        {
            return Err(FT_ERR_INVALID_TABLE);
        }

        result = count;
        Ok(())
    })();

    /* Exit: */
    *acount = result as FtLong;
    error
}

/// `pfr_log_font_load`: load a pfr logical font entry
pub fn pfr_log_font_load(
    log_font: &mut PfrLogFontRec,
    stream: &mut FtStreamRec,
    idx: FtUInt,
    section_offset: FtUInt32,
    size_increment: bool,
) -> FtResult<()> {
    stream.seek(section_offset as FtULong)?;
    let num_log_fonts = stream.read_ushort()? as FtUInt;

    if idx >= num_log_fonts {
        return Err(FT_ERR_INVALID_ARGUMENT);
    }

    stream.skip(idx as FtLong * 5)?;
    let size = stream.read_ushort()? as FtUInt32;
    let offset = stream.read_uoffset()?;

    /* save logical font size and offset */
    log_font.size = size;
    log_font.offset = offset;

    /* now, check the rest of the table before loading it */
    stream.seek(offset as FtULong)?;
    stream.enter_frame(size as FtULong)?;

    let error = {
        let base = stream.frame_data();
        let mut p = 0usize;
        let limit = size as usize;

        'fail: {
            'too_short: {
                if !pfr_check(p, 13, limit) {
                    break 'too_short;
                }

                log_font.matrix[0] = pfr_next_long(base, &mut p);
                log_font.matrix[1] = pfr_next_long(base, &mut p);
                log_font.matrix[2] = pfr_next_long(base, &mut p);
                log_font.matrix[3] = pfr_next_long(base, &mut p);

                let flags = pfr_next_byte(base, &mut p);

                let mut local: FtUInt = 0;
                if flags & PFR_LOG_STROKE != 0 {
                    local += 1;
                    if flags & PFR_LOG_2BYTE_STROKE != 0 {
                        local += 1;
                    }

                    if (flags & PFR_LINE_JOIN_MASK) == PFR_LINE_JOIN_MITER {
                        local += 3;
                    }
                }
                if flags & PFR_LOG_BOLD != 0 {
                    local += 1;
                    if flags & PFR_LOG_2BYTE_BOLD != 0 {
                        local += 1;
                    }
                }

                if !pfr_check(p, local as u64, limit) {
                    break 'too_short;
                }

                if flags & PFR_LOG_STROKE != 0 {
                    log_font.stroke_thickness = if flags & PFR_LOG_2BYTE_STROKE != 0 {
                        pfr_next_short(base, &mut p)
                    } else {
                        pfr_next_byte(base, &mut p) as FtInt
                    };

                    if (flags & PFR_LINE_JOIN_MASK) == PFR_LINE_JOIN_MITER {
                        log_font.miter_limit = pfr_next_long(base, &mut p);
                    }
                }

                if flags & PFR_LOG_BOLD != 0 {
                    log_font.bold_thickness = if flags & PFR_LOG_2BYTE_BOLD != 0 {
                        pfr_next_short(base, &mut p)
                    } else {
                        pfr_next_byte(base, &mut p) as FtInt
                    };
                }

                if flags & PFR_LOG_EXTRA_ITEMS != 0 {
                    let error = pfr_extra_items_skip(base, &mut p, limit);
                    if error.is_err() {
                        break 'fail error;
                    }
                }

                if !pfr_check(p, 5, limit) {
                    break 'too_short;
                }
                log_font.phys_size = pfr_next_ushort(base, &mut p);
                log_font.phys_offset = pfr_next_ulong(base, &mut p);
                if size_increment {
                    if !pfr_check(p, 1, limit) {
                        break 'too_short;
                    }
                    log_font.phys_size += pfr_next_byte(base, &mut p) << 16;
                }

                break 'fail Ok(());
            }

            /* Too_Short: */
            Err(FT_ERR_INVALID_TABLE)
        }
    };

    /* Fail: */
    stream.exit_frame();

    /* Exit: */
    error
}

/***********************************************************************/
/***********************************************************************/
/*****                                                             *****/
/*****                    PFR PHYSICAL FONTS                       *****/
/*****                                                             *****/
/***********************************************************************/
/***********************************************************************/

/// `pfr_extra_item_load_bitmap_info`: load bitmap strikes lists
fn pfr_extra_item_load_bitmap_info(
    base: &[u8],
    mut p: usize,
    limit: usize,
    phy_font: &mut PfrPhyFontRec,
) -> FtResult<()> {
    if !pfr_check(p, 5, limit) {
        /* Too_Short: */
        return Err(FT_ERR_INVALID_TABLE);
    }

    p += 3; /* skip bctSize */
    let flags0 = pfr_next_byte(base, &mut p);
    let count = pfr_next_byte(base, &mut p);

    /* re-allocate when needed */
    if phy_font.num_strikes + count > phy_font.max_strikes {
        let new_max = ft_pad_ceil((phy_font.num_strikes + count) as i64, 4) as FtUInt;

        ft_renew_array(&mut phy_font.strikes, new_max as FtLong)?;

        phy_font.max_strikes = new_max;
    }

    let mut size1: FtUInt = 1 + 1 + 1 + 2 + 2 + 1;
    if flags0 & PFR_STRIKE_2BYTE_XPPM != 0 {
        size1 += 1;
    }

    if flags0 & PFR_STRIKE_2BYTE_YPPM != 0 {
        size1 += 1;
    }

    if flags0 & PFR_STRIKE_3BYTE_SIZE != 0 {
        size1 += 1;
    }

    if flags0 & PFR_STRIKE_3BYTE_OFFSET != 0 {
        size1 += 1;
    }

    if flags0 & PFR_STRIKE_2BYTE_COUNT != 0 {
        size1 += 1;
    }

    let first = phy_font.num_strikes as usize;

    if !pfr_check(p, count as u64 * size1 as u64, limit) {
        /* Too_Short: */
        return Err(FT_ERR_INVALID_TABLE);
    }

    for strike in &mut phy_font.strikes[first..first + count as usize] {
        strike.x_ppm = if flags0 & PFR_STRIKE_2BYTE_XPPM != 0 {
            pfr_next_ushort(base, &mut p)
        } else {
            pfr_next_byte(base, &mut p)
        };

        strike.y_ppm = if flags0 & PFR_STRIKE_2BYTE_YPPM != 0 {
            pfr_next_ushort(base, &mut p)
        } else {
            pfr_next_byte(base, &mut p)
        };

        strike.flags = pfr_next_byte(base, &mut p);

        strike.bct_size = if flags0 & PFR_STRIKE_3BYTE_SIZE != 0 {
            pfr_next_ulong(base, &mut p)
        } else {
            pfr_next_ushort(base, &mut p)
        };

        strike.bct_offset = if flags0 & PFR_STRIKE_3BYTE_OFFSET != 0 {
            pfr_next_ulong(base, &mut p)
        } else {
            pfr_next_ushort(base, &mut p)
        };

        strike.num_bitmaps = if flags0 & PFR_STRIKE_2BYTE_COUNT != 0 {
            pfr_next_ushort(base, &mut p)
        } else {
            pfr_next_byte(base, &mut p)
        };
    }

    phy_font.num_strikes += count;

    /* Exit: */
    Ok(())
}

/* Load font ID.  This is a so-called `unique' name that is rather
 * long and descriptive (like `Tiresias ScreenFont v7.51').
 *
 * Note that a PFR font's family name is contained in an *undocumented*
 * string of the `auxiliary data' portion of a physical font record.  This
 * may also contain the `real' style name!
 *
 * If no family name is present, the font ID is used instead for the
 * family.
 */
/// `pfr_extra_item_load_font_id`
fn pfr_extra_item_load_font_id(
    base: &[u8],
    p: usize,
    limit: usize,
    phy_font: &mut PfrPhyFontRec,
) -> FtResult<()> {
    let len = limit - p;

    if phy_font.font_id.is_some() {
        return Ok(());
    }

    let mut font_id = ft_qalloc(len as FtLong + 1)?;

    /* copy font ID name, and terminate it for safety */
    font_id[..len].copy_from_slice(&base[p..limit]);
    font_id[len] = 0;
    phy_font.font_id = Some(font_id);

    /* Exit: */
    Ok(())
}

/// `pfr_extra_item_load_stem_snaps`: load stem snap tables
fn pfr_extra_item_load_stem_snaps(
    base: &[u8],
    mut p: usize,
    limit: usize,
    phy_font: &mut PfrPhyFontRec,
) -> FtResult<()> {
    if phy_font.vertical.stem_snaps.is_some() {
        return Ok(());
    }

    if !pfr_check(p, 1, limit) {
        /* Too_Short: */
        return Err(FT_ERR_INVALID_TABLE);
    }
    let mut count = pfr_next_byte(base, &mut p);

    let num_vert = count & 15;
    let _num_horz = count >> 4;
    count = num_vert + _num_horz;

    if !pfr_check(p, count as u64 * 2, limit) {
        /* Too_Short: */
        return Err(FT_ERR_INVALID_TABLE);
    }

    let mut snaps: Vec<FtInt> = ft_new_array(count as FtLong)?;

    for snap in snaps.iter_mut() {
        *snap = ft_next_short(base, &mut p) as FtInt;
    }

    /* (a block of no values is C's NULL) */
    if count > 0 {
        phy_font.vertical.stem_snaps = Some(0);
        phy_font.horizontal.stem_snaps = Some(num_vert as usize);
        phy_font.stem_snaps = Some(snaps);
    }

    /* Exit: */
    Ok(())
}

/// `pfr_extra_item_load_kerning_pairs`: load kerning pair data
fn pfr_extra_item_load_kerning_pairs(
    base: &[u8],
    mut p: usize,
    limit: usize,
    phy_font: &mut PfrPhyFontRec,
) -> FtResult<()> {
    /* (FT_NEW( item )) */
    if phy_font.kern_items.try_reserve(1).is_err() {
        return Err(FT_ERR_OUT_OF_MEMORY);
    }
    let mut item = PfrKernItemRec::default();

    if !pfr_check(p, 4, limit) {
        /* Too_Short: */
        return Err(FT_ERR_INVALID_TABLE);
    }

    item.pair_count = pfr_next_byte(base, &mut p) as FtByte;
    item.base_adj = pfr_next_short(base, &mut p) as FtShort;
    item.flags = pfr_next_byte(base, &mut p) as FtByte;
    item.offset = (phy_font.offset as usize).wrapping_add(p);

    /* (PFR_CONFIG_NO_CHECKS is not defined) */
    item.pair_size = 3;

    if item.flags & PFR_KERN_2BYTE_CHAR != 0 {
        item.pair_size += 2;
    }

    if item.flags & PFR_KERN_2BYTE_ADJ != 0 {
        item.pair_size += 1;
    }

    if !pfr_check(p, item.pair_count as u64 * item.pair_size as u64, limit) {
        /* Too_Short: */
        return Err(FT_ERR_INVALID_TABLE);
    }

    /* load first and last pairs into the item to speed up */
    /* lookup later...                                     */
    if item.pair_count > 0 {
        if item.flags & PFR_KERN_2BYTE_CHAR != 0 {
            let mut q = p;
            let char1 = pfr_next_ushort(base, &mut q);
            let char2 = pfr_next_ushort(base, &mut q);

            item.pair1 = pfr_kern_index(char1, char2);

            q = p + item.pair_size as usize * (item.pair_count as usize - 1);
            let char1 = pfr_next_ushort(base, &mut q);
            let char2 = pfr_next_ushort(base, &mut q);

            item.pair2 = pfr_kern_index(char1, char2);
        } else {
            let mut q = p;
            let char1 = pfr_next_byte(base, &mut q);
            let char2 = pfr_next_byte(base, &mut q);

            item.pair1 = pfr_kern_index(char1, char2);

            q = p + item.pair_size as usize * (item.pair_count as usize - 1);
            let char1 = pfr_next_byte(base, &mut q);
            let char2 = pfr_next_byte(base, &mut q);

            item.pair2 = pfr_kern_index(char1, char2);
        }

        /* add new item to the current list */
        phy_font.kern_items.push(item);
        phy_font.num_kern_pairs += item.pair_count as FtUInt;
    } else {
        /* empty item! */
    }

    /* Exit: */
    Ok(())
}

/// `pfr_phy_font_extra_items`
static PFR_PHY_FONT_EXTRA_ITEMS: [PfrExtraItemRec; 4] = [
    PfrExtraItemRec {
        type_: 1,
        parser: pfr_extra_item_load_bitmap_info,
    },
    PfrExtraItemRec {
        type_: 2,
        parser: pfr_extra_item_load_font_id,
    },
    PfrExtraItemRec {
        type_: 3,
        parser: pfr_extra_item_load_stem_snaps,
    },
    PfrExtraItemRec {
        type_: 4,
        parser: pfr_extra_item_load_kerning_pairs,
    },
];

/*
 * Load a name from the auxiliary data.  Since this extracts undocumented
 * strings from the font file, we need to be careful here.
 */
/// `pfr_aux_name_load`
fn pfr_aux_name_load(p: &[u8], mut len: usize, astring: &mut Option<Vec<u8>>) -> FtResult<()> {
    let mut result = None;

    *astring = None;

    if len > 0 && p[len - 1] == 0 {
        len -= 1;
    }

    /* check that each character is ASCII  */
    /* for making sure not to load garbage */
    let mut ok = len > 0;
    for &c in &p[..len] {
        if !(32..=127).contains(&c) {
            ok = false;
            break;
        }
    }

    if ok {
        let mut s = ft_qalloc(len as FtLong + 1)?;

        s[..len].copy_from_slice(&p[..len]);
        s[len] = 0;
        result = Some(s);
    }

    /* Exit: */
    *astring = result;
    Ok(())
}

/// `pfr_phy_font_done`: finalize a physical font
pub fn pfr_phy_font_done(phy_font: &mut PfrPhyFontRec) {
    phy_font.font_id = None;
    phy_font.family_name = None;
    phy_font.style_name = None;

    phy_font.stem_snaps = None;
    phy_font.vertical.stem_snaps = None;
    phy_font.vertical.num_stem_snaps = 0;

    phy_font.horizontal.stem_snaps = None;
    phy_font.horizontal.num_stem_snaps = 0;

    phy_font.strikes = Vec::new();
    phy_font.num_strikes = 0;
    phy_font.max_strikes = 0;

    phy_font.chars = Vec::new();
    phy_font.num_chars = 0;
    phy_font.chars_offset = 0;

    phy_font.blue_values = Vec::new();
    phy_font.num_blue_values = 0;

    phy_font.kern_items = Vec::new();

    phy_font.num_kern_pairs = 0;
}

/// `pfr_phy_font_load`: load a physical font entry
pub fn pfr_phy_font_load(
    phy_font: &mut PfrPhyFontRec,
    stream: &mut FtStreamRec,
    offset: FtUInt32,
    size: FtUInt32,
) -> FtResult<()> {
    phy_font.offset = offset;

    phy_font.kern_items = Vec::new();

    stream.seek(offset as FtULong)?;
    stream.enter_frame(size as FtULong)?;

    let mut set_bct_offset = true;
    let error = {
        let base = stream.frame_data();
        let mut p = 0usize;
        let limit = size as usize;

        'fail: {
            'too_short: {
                if !pfr_check(p, 15, limit) {
                    break 'too_short;
                }
                phy_font.font_ref_number = pfr_next_ushort(base, &mut p);
                phy_font.outline_resolution = pfr_next_ushort(base, &mut p);
                phy_font.metrics_resolution = pfr_next_ushort(base, &mut p);
                phy_font.bbox.xMin = pfr_next_short(base, &mut p) as FtPos;
                phy_font.bbox.yMin = pfr_next_short(base, &mut p) as FtPos;
                phy_font.bbox.xMax = pfr_next_short(base, &mut p) as FtPos;
                phy_font.bbox.yMax = pfr_next_short(base, &mut p) as FtPos;
                let flags = pfr_next_byte(base, &mut p);
                phy_font.flags = flags;

                if phy_font.outline_resolution == 0 || phy_font.metrics_resolution == 0 {
                    break 'fail Err(FT_ERR_INVALID_TABLE);
                }

                /* get the standard advance for non-proportional fonts */
                if flags & PFR_PHY_PROPORTIONAL == 0 {
                    if !pfr_check(p, 2, limit) {
                        break 'too_short;
                    }
                    phy_font.standard_advance = pfr_next_short(base, &mut p);
                }

                /* load the extra items when present */
                if flags & PFR_PHY_EXTRA_ITEMS != 0 {
                    let error = pfr_extra_items_parse(
                        base,
                        &mut p,
                        limit,
                        Some(&PFR_PHY_FONT_EXTRA_ITEMS),
                        Some(phy_font),
                    );

                    if error.is_err() {
                        break 'fail error;
                    }
                }

                /* In certain fonts, the auxiliary bytes contain interesting   */
                /* information.  These are not in the specification but can be */
                /* guessed by looking at the content of a few 'PFR0' fonts.    */
                if !pfr_check(p, 3, limit) {
                    break 'too_short;
                }
                let mut num_aux = pfr_next_ulong(base, &mut p) as FtULong;

                if num_aux > 0 {
                    let mut q = p;

                    if !pfr_check(p, num_aux, limit) {
                        break 'too_short;
                    }

                    p += num_aux as usize;

                    while num_aux > 0 {
                        if q + 4 > p {
                            break;
                        }

                        let length = pfr_next_ushort(base, &mut q);
                        if length < 4 || length as FtULong > num_aux {
                            break;
                        }

                        let q2 = q + length as usize - 2;
                        let type_ = pfr_next_ushort(base, &mut q);

                        match type_ {
                            1 => {
                                /* this seems to correspond to the font's family name, padded to */
                                /* an even number of bytes with a zero byte appended if needed   */
                                if let Err(e) = pfr_aux_name_load(
                                    &base[q..],
                                    length as usize - 4,
                                    &mut phy_font.family_name,
                                ) {
                                    /* (goto Exit) */
                                    set_bct_offset = false;
                                    break 'fail Err(e);
                                }
                            }

                            2 => {
                                if q + 32 <= q2 {
                                    q += 10;
                                    phy_font.ascent = pfr_next_short(base, &mut q);
                                    phy_font.descent = pfr_next_short(base, &mut q);
                                    phy_font.leading = pfr_next_short(base, &mut q);
                                }
                            }

                            3 => {
                                /* this seems to correspond to the font's style name, padded to */
                                /* an even number of bytes with a zero byte appended if needed  */
                                if let Err(e) = pfr_aux_name_load(
                                    &base[q..],
                                    length as usize - 4,
                                    &mut phy_font.style_name,
                                ) {
                                    /* (goto Exit) */
                                    set_bct_offset = false;
                                    break 'fail Err(e);
                                }
                            }

                            _ => {}
                        }

                        q = q2;
                        num_aux -= length as FtULong;
                    }
                }

                /* read the blue values */
                {
                    if !pfr_check(p, 1, limit) {
                        break 'too_short;
                    }
                    let count = pfr_next_byte(base, &mut p);
                    phy_font.num_blue_values = count;

                    if !pfr_check(p, count as u64 * 2, limit) {
                        break 'too_short;
                    }

                    match ft_new_array(count as FtLong) {
                        Ok(v) => phy_font.blue_values = v,
                        Err(e) => break 'fail Err(e),
                    }

                    for n in 0..count as usize {
                        phy_font.blue_values[n] = pfr_next_short(base, &mut p);
                    }
                }

                if !pfr_check(p, 8, limit) {
                    break 'too_short;
                }
                phy_font.blue_fuzz = pfr_next_byte(base, &mut p);
                phy_font.blue_scale = pfr_next_byte(base, &mut p);

                phy_font.vertical.standard = pfr_next_ushort(base, &mut p);
                phy_font.horizontal.standard = pfr_next_ushort(base, &mut p);

                /* read the character descriptors */
                {
                    let count = pfr_next_ushort(base, &mut p);
                    phy_font.num_chars = count;
                    phy_font.chars_offset = offset as usize + p;

                    if phy_font.num_chars == 0 {
                        break 'fail Err(FT_ERR_INVALID_TABLE);
                    }

                    let mut size: FtUInt = 1 + 1 + 2;
                    if flags & PFR_PHY_2BYTE_CHARCODE != 0 {
                        size += 1;
                    }

                    if flags & PFR_PHY_PROPORTIONAL != 0 {
                        size += 2;
                    }

                    if flags & PFR_PHY_ASCII_CODE != 0 {
                        size += 1;
                    }

                    if flags & PFR_PHY_2BYTE_GPS_SIZE != 0 {
                        size += 1;
                    }

                    if flags & PFR_PHY_3BYTE_GPS_OFFSET != 0 {
                        size += 1;
                    }

                    if !pfr_check(p, count as u64 * size as u64, limit) {
                        break 'too_short;
                    }

                    match ft_new_array(count as FtLong) {
                        Ok(v) => phy_font.chars = v,
                        Err(e) => break 'fail Err(e),
                    }

                    for n in 0..count as usize {
                        let standard_advance = phy_font.standard_advance;
                        let cur = &mut phy_font.chars[n];

                        cur.char_code = if flags & PFR_PHY_2BYTE_CHARCODE != 0 {
                            pfr_next_ushort(base, &mut p)
                        } else {
                            pfr_next_byte(base, &mut p)
                        };

                        cur.advance = if flags & PFR_PHY_PROPORTIONAL != 0 {
                            pfr_next_short(base, &mut p)
                        } else {
                            standard_advance
                        };

                        /* (the `#if 0'ed `ascii' field) */
                        if flags & PFR_PHY_ASCII_CODE != 0 {
                            p += 1;
                        }

                        cur.gps_size = if flags & PFR_PHY_2BYTE_GPS_SIZE != 0 {
                            pfr_next_ushort(base, &mut p)
                        } else {
                            pfr_next_byte(base, &mut p)
                        };

                        cur.gps_offset = if flags & PFR_PHY_3BYTE_GPS_OFFSET != 0 {
                            pfr_next_ulong(base, &mut p)
                        } else {
                            pfr_next_ushort(base, &mut p)
                        };
                    }
                }

                /* that's it! */
                break 'fail Ok(());
            }

            /* Too_Short: */
            Err(FT_ERR_INVALID_TABLE)
        }
    };

    /* Fail: */
    stream.exit_frame();

    if set_bct_offset {
        /* save position of bitmap info */
        phy_font.bct_offset = stream.pos();
    }

    /* Exit: */
    error
}
