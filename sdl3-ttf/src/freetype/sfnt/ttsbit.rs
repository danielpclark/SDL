// Rust translation of src/sfnt/ttsbit.c from FreeType (2.13.2, as SDL_ttf's
// external/freetype pins it).
// Copyright (C) 2005-2023 by David Turner, Robert Wilhelm, and Werner Lemberg.
// Copyright 2013 by Google, Inc.
// Google Author(s): Behdad Esfahbod.
// This is an altered (translated) version of the original software; it is
// used under the FreeType License (see FTL.TXT and LICENSE.txt).

//! TrueType and OpenType embedded bitmap support (body).
//!
//! `TT_CONFIG_OPTION_EMBEDDED_BITMAPS` is defined;
//! `FT_CONFIG_OPTION_USE_PNG` is not (SDL_ttf's bundled build disables
//! libpng), so PNG strikes (CBDT formats 17-19, `sbix` `png `) report
//! `Unimplemented_Feature`, as upstream's build does.

use super::super::base::ftbitmap::{ft_bitmap_convert, ft_bitmap_init};
use super::super::base::ftobjs::*;
use super::super::base::ftstream::*;
use super::super::fttypes::*;
use super::super::tttables::*;
use super::super::tttypes::*;
use super::ttload::tt_face_goto_table;
use super::ttmtx::tt_face_get_metrics;

/// `tt_face_load_sbit`
pub fn tt_face_load_sbit(face: &mut TtFaceRec, stream: &mut FtStreamRec) -> FtResult<()> {
    face.sbit_table = Vec::new();
    face.sbit_table_size = 0;
    face.sbit_table_type = TT_SBIT_TABLE_TYPE_NONE;
    face.sbit_num_strikes = 0;

    let r = (|| -> FtResult<()> {
        let mut res = tt_face_goto_table(face, TTAG_CBLC as FtULong, stream);
        if res.is_ok() {
            face.sbit_table_type = TT_SBIT_TABLE_TYPE_CBLC;
        } else {
            res = tt_face_goto_table(face, TTAG_EBLC as FtULong, stream);
            if res.is_err() {
                res = tt_face_goto_table(face, TTAG_bloc as FtULong, stream);
            }

            if res.is_ok() {
                face.sbit_table_type = TT_SBIT_TABLE_TYPE_EBLC;
            }
        }

        if res.is_err() {
            res = tt_face_goto_table(face, TTAG_sbix as FtULong, stream);
            if res.is_ok() {
                face.sbit_table_type = TT_SBIT_TABLE_TYPE_SBIX;
            }
        }
        let table_size = res?;

        if table_size < 8 {
            return Err(FT_ERR_INVALID_FILE_FORMAT);
        }

        let table_start = stream.pos();

        match face.sbit_table_type {
            TT_SBIT_TABLE_TYPE_EBLC | TT_SBIT_TABLE_TYPE_CBLC => {
                face.sbit_table = stream.extract_frame(table_size)?;
                face.sbit_table_size = table_size;

                let t = &face.sbit_table;
                let mut p = 0;

                let version = ft_next_long(t, &mut p) as FtFixed;
                let num_strikes = ft_next_ulong(t, &mut p) as FtULong;

                /* there's at least one font (FZShuSong-Z01, version 3)   */
                /* that uses the wrong byte order for the `version' field */
                if (version as FtULong & 0xFFFF0000) != 0x00020000
                    && (version as FtULong & 0x0000FFFF) != 0x00000200
                    && (version as FtULong & 0xFFFF0000) != 0x00030000
                    && (version as FtULong & 0x0000FFFF) != 0x00000300
                {
                    return Err(FT_ERR_UNKNOWN_FILE_FORMAT);
                }

                if num_strikes >= 0x10000 {
                    return Err(FT_ERR_INVALID_FILE_FORMAT);
                }

                /*
                 * Count the number of strikes available in the table.  We are a bit
                 * paranoid there and don't trust the data.
                 */
                let mut count = num_strikes as FtUInt;
                if 8 + 48 * count as FtULong > table_size {
                    count = ((table_size - 8) / 48) as FtUInt;
                }

                face.sbit_num_strikes = count;
            }

            TT_SBIT_TABLE_TYPE_SBIX => {
                stream.enter_frame(8)?;

                let version = stream.get_ushort();
                let flags = stream.get_ushort();
                let num_strikes = stream.get_ulong() as FtULong;

                stream.exit_frame();

                if version < 1 {
                    return Err(FT_ERR_UNKNOWN_FILE_FORMAT);
                }

                /* Bit 0 must always be `1'.                            */
                /* Bit 1 controls the overlay of bitmaps with outlines. */
                /* All other bits should be zero.                       */
                if !(flags == 1 || flags == 3) || num_strikes >= 0x10000 {
                    return Err(FT_ERR_INVALID_FILE_FORMAT);
                }

                if flags == 3 {
                    face.root.face_flags |= FT_FACE_FLAG_SBIX_OVERLAY;
                }

                /*
                 * Count the number of strikes available in the table.  We are a bit
                 * paranoid there and don't trust the data.
                 */
                let mut count = num_strikes as FtUInt;
                if 8 + 4 * count as FtULong > table_size {
                    count = ((table_size - 8) / 4) as FtUInt;
                }

                stream.seek(stream.pos() - 8)?;

                face.sbit_table_size = 8 + count as FtULong * 4;
                face.sbit_table = stream.extract_frame(face.sbit_table_size)?;

                face.sbit_num_strikes = count;
            }

            _ => {
                /* we ignore unknown table formats */
                return Err(FT_ERR_UNKNOWN_FILE_FORMAT);
            }
        }

        face.ebdt_start = 0;
        face.ebdt_size = 0;

        if face.sbit_table_type == TT_SBIT_TABLE_TYPE_SBIX {
            /* the `sbix' table is self-contained; */
            /* it has no associated data table     */
            face.ebdt_start = table_start;
            face.ebdt_size = table_size;
        } else if face.sbit_table_type != TT_SBIT_TABLE_TYPE_NONE {
            let mut res = tt_face_goto_table(face, TTAG_CBDT as FtULong, stream);
            if res.is_err() {
                res = tt_face_goto_table(face, TTAG_EBDT as FtULong, stream);
            }
            if res.is_err() {
                res = tt_face_goto_table(face, TTAG_bdat as FtULong, stream);
            }

            if let Ok(ebdt_size) = res {
                face.ebdt_start = stream.pos();
                face.ebdt_size = ebdt_size;
            }
        }

        if face.ebdt_size == 0 {
            face.sbit_num_strikes = 0;
        }

        Ok(())
    })();

    /* Exit: */
    if r.is_err() {
        face.sbit_table = Vec::new();
        face.sbit_table_size = 0;
        face.sbit_table_type = TT_SBIT_TABLE_TYPE_NONE;
    }

    r
}

/// `tt_face_free_sbit`
pub fn tt_face_free_sbit(face: &mut TtFaceRec) {
    face.sbit_table = Vec::new();
    face.sbit_table_size = 0;
    face.sbit_table_type = TT_SBIT_TABLE_TYPE_NONE;
    face.sbit_num_strikes = 0;
}

/// `tt_face_set_sbit_strike`
pub fn tt_face_set_sbit_strike(face: &TtFaceRec, req: &FtSizeRequestRec) -> FtResult<FtULong> {
    let mut astrike_index: FtULong = 0;
    ft_match_size(&face.root, req, false, Some(&mut astrike_index))?;
    Ok(astrike_index)
}

/// `tt_face_load_strike_metrics`
pub fn tt_face_load_strike_metrics(
    face: &TtFaceRec,
    stream: &mut FtStreamRec,
    mut strike_index: FtULong,
    metrics: &mut FtSizeMetrics,
) -> FtResult<()> {
    /* we have to test for the existence of `sbit_strike_map'    */
    /* because the function gets also used at the very beginning */
    /* to construct `sbit_strike_map' itself                     */
    if !face.sbit_strike_map.is_empty() {
        if strike_index >= face.root.num_fixed_sizes as FtULong {
            return Err(FT_ERR_INVALID_ARGUMENT);
        }

        /* map to real index */
        strike_index = face.sbit_strike_map[strike_index as usize] as FtULong;
    } else if strike_index >= face.sbit_num_strikes as FtULong {
        return Err(FT_ERR_INVALID_ARGUMENT);
    }

    match face.sbit_table_type {
        TT_SBIT_TABLE_TYPE_EBLC | TT_SBIT_TABLE_TYPE_CBLC => {
            let t = &face.sbit_table;
            let strike = 8 + strike_index as usize * 48;
            let b = |i: usize| -> u8 { t.get(strike + i).copied().unwrap_or(0) };

            metrics.x_ppem = b(44) as FtUShort;
            metrics.y_ppem = b(45) as FtUShort;

            metrics.ascender = b(16) as FtChar as FtPos * 64; /* hori.ascender  */
            metrics.descender = b(17) as FtChar as FtPos * 64; /* hori.descender */

            /* Due to fuzzy wording in the EBLC documentation, we find both */
            /* positive and negative values for `descender'.  Additionally, */
            /* many fonts have both `ascender' and `descender' set to zero  */
            /* (which is definitely wrong).  MS Windows simply ignores all  */
            /* those values...  For these reasons we apply some heuristics  */
            /* to get a reasonable, non-zero value for the height.          */

            let max_before_bl = b(24) as FtChar;
            let min_after_bl = b(25) as FtChar;

            if metrics.descender > 0 {
                /* compare sign of descender with `min_after_bl' */
                if min_after_bl < 0 {
                    metrics.descender = -metrics.descender;
                }
            } else if metrics.descender == 0 && metrics.ascender == 0 {
                /* sanitize buggy ascender and descender values */
                if max_before_bl != 0 || min_after_bl != 0 {
                    metrics.ascender = max_before_bl as FtPos * 64;
                    metrics.descender = min_after_bl as FtPos * 64;
                } else {
                    metrics.ascender = metrics.y_ppem as FtPos * 64;
                    metrics.descender = 0;
                }
            }

            metrics.height = metrics.ascender - metrics.descender;
            if metrics.height == 0 {
                metrics.height = metrics.y_ppem as FtPos * 64;
                metrics.descender = metrics.ascender - metrics.height;
            }

            /* Is this correct? */
            metrics.max_advance = (b(22) as FtChar as FtPos /* min_origin_SB  */
                + b(18) as FtPos /* max_width      */
                + b(23) as FtChar as FtPos/* min_advance_SB */)
                * 64;

            /* set the scale values (in 16.16 units) so advances */
            /* from the hmtx and vmtx table are scaled correctly */
            metrics.x_scale = ft_div_fix(
                metrics.x_ppem as FtLong * 64,
                face.header.Units_Per_EM as FtLong,
            );
            metrics.y_scale = ft_div_fix(
                metrics.y_ppem as FtLong * 64,
                face.header.Units_Per_EM as FtLong,
            );

            Ok(())
        }

        TT_SBIT_TABLE_TYPE_SBIX => {
            let mut p = 8 + 4 * strike_index as usize;
            let offset = ft_next_ulong(&face.sbit_table, &mut p) as FtULong;

            if offset + 4 > face.ebdt_size {
                return Err(FT_ERR_INVALID_FILE_FORMAT);
            }

            stream.seek(face.ebdt_start + offset)?;
            stream.enter_frame(4)?;

            let ppem = stream.get_ushort();
            let _resolution = stream.get_ushort(); /* What to do with this? */

            stream.exit_frame();

            metrics.x_ppem = ppem;
            metrics.y_ppem = ppem;

            let scale = ft_div_fix(ppem as FtLong * 64, face.header.Units_Per_EM as FtLong);
            let hori = &face.horizontal;

            metrics.ascender = ft_mul_fix(hori.Ascender as FtLong, scale);
            metrics.descender = ft_mul_fix(hori.Descender as FtLong, scale);
            metrics.height = ft_mul_fix(
                hori.Ascender as FtLong - hori.Descender as FtLong + hori.Line_Gap as FtLong,
                scale,
            );
            metrics.max_advance = ft_mul_fix(hori.advance_Width_Max as FtLong, scale);

            /* set the scale values (in 16.16 units) so advances */
            /* from the hmtx and vmtx table are scaled correctly */
            metrics.x_scale = scale;
            metrics.y_scale = scale;

            Ok(())
        }

        _ => Err(FT_ERR_UNKNOWN_FILE_FORMAT),
    }
}

use super::super::base::ftcalc::{ft_div_fix, ft_mul_fix};

/// `TT_SBitDecoderRec` (its `face`, `stream`, `bitmap` and `metrics` are
/// passed to the functions; `eblc_base` is the face's `sbit_table`)
#[derive(Debug, Clone, Copy, Default)]
struct TtSBitDecoderRec {
    metrics_loaded: bool,
    bitmap_allocated: bool,
    bit_depth: FtByte,

    ebdt_start: FtULong,
    ebdt_size: FtULong,

    strike_index_array: FtULong,
    strike_index_count: FtULong,
    eblc_limit: usize,
}

/// `tt_sbit_decoder_init`
fn tt_sbit_decoder_init(
    decoder: &mut TtSBitDecoderRec,
    face: &TtFaceRec,
    stream: &mut FtStreamRec,
    strike_index: FtULong,
) -> FtResult<()> {
    let strike_index = face.sbit_strike_map[strike_index as usize] as FtULong;

    if face.ebdt_size == 0 {
        return Err(FT_ERR_TABLE_MISSING);
    }

    stream.seek(face.ebdt_start)?;

    decoder.metrics_loaded = false;
    decoder.bitmap_allocated = false;

    decoder.ebdt_start = face.ebdt_start;
    decoder.ebdt_size = face.ebdt_size;

    decoder.eblc_limit = face.sbit_table_size as usize;

    /* now find the strike corresponding to the index */
    if 8 + 48 * strike_index + 3 * 4 + 34 + 1 > face.sbit_table_size {
        return Err(FT_ERR_INVALID_FILE_FORMAT);
    }

    let t = &face.sbit_table;
    let mut p = 8 + 48 * strike_index as usize;

    decoder.strike_index_array = ft_next_ulong(t, &mut p) as FtULong;
    p += 4;
    decoder.strike_index_count = ft_next_ulong(t, &mut p) as FtULong;
    p += 34;
    decoder.bit_depth = t[p];

    /* decoder->strike_index_array +                               */
    /*   8 * decoder->strike_index_count > face->sbit_table_size ? */
    if decoder.strike_index_array > face.sbit_table_size
        || decoder.strike_index_count > (face.sbit_table_size - decoder.strike_index_array) / 8
    {
        return Err(FT_ERR_INVALID_FILE_FORMAT);
    }

    Ok(())
}

/// `tt_sbit_decoder_alloc_bitmap`
fn tt_sbit_decoder_alloc_bitmap(
    decoder: &mut TtSBitDecoderRec,
    slot: &mut FtGlyphSlotRec,
    metrics: &TtSBitMetricsRec,
    metrics_only: bool,
) -> FtResult<()> {
    if !decoder.metrics_loaded {
        return Err(FT_ERR_INVALID_ARGUMENT);
    }

    let width = metrics.width as u32;
    let height = metrics.height as u32;

    let map = &mut slot.bitmap;
    map.width = width;
    map.rows = height;

    match decoder.bit_depth {
        1 => {
            map.pixel_mode = FT_PIXEL_MODE_MONO;
            map.pitch = ((map.width + 7) >> 3) as i32;
            map.num_grays = 2;
        }

        2 => {
            map.pixel_mode = FT_PIXEL_MODE_GRAY2;
            map.pitch = ((map.width + 3) >> 2) as i32;
            map.num_grays = 4;
        }

        4 => {
            map.pixel_mode = FT_PIXEL_MODE_GRAY4;
            map.pitch = ((map.width + 1) >> 1) as i32;
            map.num_grays = 16;
        }

        8 => {
            map.pixel_mode = FT_PIXEL_MODE_GRAY;
            map.pitch = map.width as i32;
            map.num_grays = 256;
        }

        32 => {
            map.pixel_mode = FT_PIXEL_MODE_BGRA;
            map.pitch = (map.width * 4) as i32;
            map.num_grays = 256;
        }

        _ => return Err(FT_ERR_INVALID_FILE_FORMAT),
    }

    let size = map.rows as FtULong * map.pitch as FtULong;

    /* check that there is no empty image */
    if size == 0 {
        return Ok(()); /* exit successfully! */
    }

    if metrics_only {
        return Ok(()); /* only metrics are requested */
    }

    ft_glyphslot_alloc_bitmap(slot, size)?;

    decoder.bitmap_allocated = true;

    Ok(())
}

/// `tt_sbit_decoder_load_metrics`
fn tt_sbit_decoder_load_metrics(
    decoder: &mut TtSBitDecoderRec,
    metrics: &mut TtSBitMetricsRec,
    t: &[u8],
    pp: &mut usize,
    limit: usize,
    big: bool,
) -> FtResult<()> {
    let mut p = *pp;

    if p + 5 > limit {
        return Err(FT_ERR_INVALID_ARGUMENT);
    }

    metrics.height = t[p] as FtUShort;
    metrics.width = t[p + 1] as FtUShort;
    metrics.horiBearingX = t[p + 2] as FtChar as FtShort;
    metrics.horiBearingY = t[p + 3] as FtChar as FtShort;
    metrics.horiAdvance = t[p + 4] as FtUShort;

    p += 5;
    if big {
        if p + 3 > limit {
            return Err(FT_ERR_INVALID_ARGUMENT);
        }

        metrics.vertBearingX = t[p] as FtChar as FtShort;
        metrics.vertBearingY = t[p + 1] as FtChar as FtShort;
        metrics.vertAdvance = t[p + 2] as FtUShort;

        p += 3;
    } else {
        /* avoid uninitialized data in case there is no vertical info -- */
        metrics.vertBearingX = 0;
        metrics.vertBearingY = 0;
        metrics.vertAdvance = 0;
    }

    decoder.metrics_loaded = true;
    *pp = p;

    Ok(())
}

#[inline]
fn or_at(buf: &mut [u8], i: isize, v: u8) {
    if i >= 0 {
        if let Some(b) = buf.get_mut(i as usize) {
            *b |= v;
        }
    }
}

#[inline]
fn at(t: &[u8], i: usize) -> u32 {
    t.get(i).copied().unwrap_or(0) as u32
}

/// `tt_sbit_decoder_load_byte_aligned`
fn tt_sbit_decoder_load_byte_aligned(
    decoder: &TtSBitDecoderRec,
    bitmap: &mut FtBitmap,
    metrics: &TtSBitMetricsRec,
    t: &[u8],
    mut p: usize,
    limit: usize,
    mut x_pos: FtInt,
    y_pos: FtInt,
) -> FtResult<()> {
    /* check that we can write the glyph into the bitmap */
    let bit_width = bitmap.width;
    let bit_height = bitmap.rows;
    let pitch = bitmap.pitch as isize;

    if bitmap.buffer.is_empty() {
        return Ok(());
    }

    let width = metrics.width as FtInt;
    let height = metrics.height as FtInt;

    let line_bits = width * decoder.bit_depth as FtInt;

    if x_pos < 0
        || (x_pos + width) as FtUInt > bit_width
        || y_pos < 0
        || (y_pos + height) as FtUInt > bit_height
    {
        return Err(FT_ERR_INVALID_FILE_FORMAT);
    }

    if p + (((line_bits + 7) >> 3) * height) as usize > limit {
        return Err(FT_ERR_INVALID_FILE_FORMAT);
    }

    /* now do the blit */
    let mut line: isize = y_pos as isize * pitch + (x_pos >> 3) as isize;
    x_pos &= 7;

    let buf = &mut bitmap.buffer[..];

    if x_pos == 0 {
        /* the easy one */
        for _ in 0..height {
            let mut pwrite = line;
            let mut w = line_bits;

            while w >= 8 {
                or_at(buf, pwrite, at(t, p) as u8);
                p += 1;
                pwrite += 1;
                w -= 8;
            }

            if w > 0 {
                or_at(buf, pwrite, (at(t, p) & (0xFF00u32 >> w)) as u8);
                p += 1;
            }
            line += pitch;
        }
    } else {
        /* x_pos > 0 */
        for _ in 0..height {
            let mut pwrite = line;
            let mut w = line_bits;
            let mut wval: FtUInt = 0;

            while w >= 8 {
                wval |= at(t, p);
                p += 1;
                or_at(buf, pwrite, (wval >> x_pos) as u8);
                pwrite += 1;
                wval <<= 8;
                w -= 8;
            }

            if w > 0 {
                wval |= at(t, p) & (0xFF00u32 >> w);
                p += 1;
            }

            /* all bits read and there are `x_pos + w' bits to be written */

            or_at(buf, pwrite, (wval >> x_pos) as u8);

            if x_pos + w > 8 {
                pwrite += 1;
                wval <<= 8;
                or_at(buf, pwrite, (wval >> x_pos) as u8);
            }
            line += pitch;
        }
    }

    Ok(())
}

/*
 * Load a bit-aligned bitmap (with pointer `p') into a line-aligned bitmap
 * (with pointer `pwrite').  In the example below, the width is 3 pixel,
 * and `x_pos' is 1 pixel.
 *
 *       p                               p+1
 *     |                               |                               |
 *     | 7   6   5   4   3   2   1   0 | 7   6   5   4   3   2   1   0 |...
 *     |                               |                               |
 *       +-------+   +-------+   +-------+ ...
 *           .           .           .
 *           .           .           .
 *           v           .           .
 *       +-------+       .           .
 * |                               | .
 * | 7   6   5   4   3   2   1   0 | .
 * |                               | .
 *   pwrite              .           .
 *                       .           .
 *                       v           .
 *                   +-------+       .
 *             |                               |
 *             | 7   6   5   4   3   2   1   0 |
 *             |                               |
 *               pwrite+1            .
 *                                   .
 *                                   v
 *                               +-------+
 *                         |                               |
 *                         | 7   6   5   4   3   2   1   0 |
 *                         |                               |
 *                           pwrite+2
 *
 */

/// `tt_sbit_decoder_load_bit_aligned`
fn tt_sbit_decoder_load_bit_aligned(
    decoder: &TtSBitDecoderRec,
    bitmap: &mut FtBitmap,
    metrics: &TtSBitMetricsRec,
    t: &[u8],
    mut p: usize,
    limit: usize,
    mut x_pos: FtInt,
    y_pos: FtInt,
) -> FtResult<()> {
    /* check that we can write the glyph into the bitmap */
    let bit_width = bitmap.width;
    let bit_height = bitmap.rows;
    let pitch = bitmap.pitch as isize;

    let width = metrics.width as FtInt;
    let height = metrics.height as FtInt;

    let line_bits = width * decoder.bit_depth as FtInt;

    if x_pos < 0
        || (x_pos + width) as FtUInt > bit_width
        || y_pos < 0
        || (y_pos + height) as FtUInt > bit_height
    {
        return Err(FT_ERR_INVALID_FILE_FORMAT);
    }

    if p + ((line_bits * height + 7) >> 3) as usize > limit {
        return Err(FT_ERR_INVALID_FILE_FORMAT);
    }

    if line_bits == 0 || height == 0 {
        /* nothing to do */
        return Ok(());
    }

    /* now do the blit */

    /* adjust `line' to point to the first byte of the bitmap */
    let mut line: isize = y_pos as isize * pitch + (x_pos >> 3) as isize;
    x_pos &= 7;

    let buf = &mut bitmap.buffer[..];

    /* the higher byte of `rval' is used as a buffer */
    let mut rval: u16 = 0;
    let mut nbits: FtInt = 0;

    for h in (1..=height).rev() {
        let mut pwrite = line;
        let mut w = line_bits;

        /* handle initial byte (in target bitmap) specially if necessary */
        if x_pos != 0 {
            w = if line_bits < 8 - x_pos {
                line_bits
            } else {
                8 - x_pos
            };

            if h == height {
                rval = at(t, p) as u16;
                p += 1;
                nbits = x_pos;
            } else if nbits < w {
                if p < limit {
                    rval |= at(t, p) as u16;
                    p += 1;
                }
                nbits += 8 - w;
            } else {
                rval >>= 8;
                nbits -= w;
            }

            or_at(
                buf,
                pwrite,
                ((((rval as u32) >> nbits) & 0xFF) & (!(0xFFu32 << w) << (8 - w - x_pos))) as u8,
            );
            pwrite += 1;
            rval <<= 8;

            w = line_bits - w;
        }

        /* handle medial bytes */
        while w >= 8 {
            rval |= at(t, p) as u16;
            p += 1;
            or_at(buf, pwrite, (((rval as u32) >> nbits) & 0xFF) as u8);
            pwrite += 1;

            rval <<= 8;
            w -= 8;
        }

        /* handle final byte if necessary */
        if w > 0 {
            if nbits < w {
                if p < limit {
                    rval |= at(t, p) as u16;
                    p += 1;
                }
                or_at(
                    buf,
                    pwrite,
                    ((((rval as u32) >> nbits) & 0xFF) & (0xFF00u32 >> w)) as u8,
                );
                nbits += 8 - w;

                rval <<= 8;
            } else {
                or_at(
                    buf,
                    pwrite,
                    ((((rval as u32) >> nbits) & 0xFF) & (0xFF00u32 >> w)) as u8,
                );
                nbits -= w;
            }
        }
        line += pitch;
    }

    Ok(())
}

/// The context of a decoder's image loading.
struct SbitCtx<'a> {
    face: &'a mut TtFaceRec,
    stream: &'a mut FtStreamRec,
    metrics: &'a mut TtSBitMetricsRec,
}

/// `tt_sbit_decoder_load_compound`
fn tt_sbit_decoder_load_compound(
    decoder: &mut TtSBitDecoderRec,
    ctx: &mut SbitCtx,
    t: &[u8],
    mut p: usize,
    limit: usize,
    x_pos: FtInt,
    y_pos: FtInt,
    recurse_count: FtUInt,
) -> FtResult<()> {
    let mut error: FtResult<()> = Ok(());

    let hori_bearing_x = ctx.metrics.horiBearingX as FtChar;
    let hori_bearing_y = ctx.metrics.horiBearingY as FtChar;
    let hori_advance = ctx.metrics.horiAdvance as FtByte;
    let vert_bearing_x = ctx.metrics.vertBearingX as FtChar;
    let vert_bearing_y = ctx.metrics.vertBearingY as FtChar;
    let vert_advance = ctx.metrics.vertAdvance as FtByte;

    if p + 2 > limit {
        return Err(FT_ERR_INVALID_FILE_FORMAT);
    }

    let num_components = ft_next_ushort(t, &mut p) as FtUInt;
    if p + 4 * num_components as usize > limit {
        return Err(FT_ERR_INVALID_FILE_FORMAT);
    }

    for _ in 0..num_components {
        let gindex = ft_next_ushort(t, &mut p) as FtUInt;
        let dx = ft_next_char(t, &mut p) as FtInt;
        let dy = ft_next_char(t, &mut p) as FtInt;

        /* NB: a recursive call */
        error = tt_sbit_decoder_load_image(
            decoder,
            ctx,
            gindex,
            x_pos + dx,
            y_pos + dy,
            recurse_count + 1,
            /* request full bitmap image */
            false,
        );
        if error.is_err() {
            break;
        }
    }

    ctx.metrics.horiBearingX = hori_bearing_x as FtShort;
    ctx.metrics.horiBearingY = hori_bearing_y as FtShort;
    ctx.metrics.horiAdvance = hori_advance as FtUShort;
    ctx.metrics.vertBearingX = vert_bearing_x as FtShort;
    ctx.metrics.vertBearingY = vert_bearing_y as FtShort;
    ctx.metrics.vertAdvance = vert_advance as FtUShort;
    ctx.metrics.width = ctx.face.root.glyph.bitmap.width as FtByte as FtUShort;
    ctx.metrics.height = ctx.face.root.glyph.bitmap.rows as FtByte as FtUShort;

    error
}

/// `TT_SBitDecoder_LoadFunc`
#[derive(Clone, Copy)]
enum SbitLoader {
    ByteAligned,
    BitAligned,
    Compound,
}

/// `tt_sbit_decoder_load_bitmap`
fn tt_sbit_decoder_load_bitmap(
    decoder: &mut TtSBitDecoderRec,
    ctx: &mut SbitCtx,
    glyph_format: FtUInt,
    glyph_start: FtULong,
    glyph_size: FtULong,
    x_pos: FtInt,
    y_pos: FtInt,
    recurse_count: FtUInt,
    metrics_only: bool,
) -> FtResult<()> {
    /* seek into the EBDT table now */
    if glyph_size == 0 || glyph_start + glyph_size > decoder.ebdt_size {
        return Err(FT_ERR_INVALID_ARGUMENT);
    }

    ctx.stream.seek(decoder.ebdt_start + glyph_start)?;
    let data = ctx.stream.extract_frame(glyph_size)?;

    let t = &data[..];
    let mut p = 0;
    let p_limit = glyph_size as usize;

    /* read the data, depending on the glyph format */
    match glyph_format {
        1 | 2 | 8 | 17 => {
            tt_sbit_decoder_load_metrics(decoder, ctx.metrics, t, &mut p, p_limit, false)?;
        }

        6 | 7 | 9 | 18 => {
            tt_sbit_decoder_load_metrics(decoder, ctx.metrics, t, &mut p, p_limit, true)?;
        }

        _ => {}
    }

    let loader = match glyph_format {
        1 | 6 => SbitLoader::ByteAligned,

        2 | 7 => {
            /* Don't trust `glyph_format'.  For example, Apple's main Korean */
            /* system font, `AppleMyungJo.ttf' (version 7.0d2e6), uses glyph */
            /* format 7, but the data is format 6.  We check whether we have */
            /* an excessive number of bytes in the image: If it is equal to  */
            /* the value for a byte-aligned glyph, use the other loading     */
            /* routine.                                                      */
            /*                                                               */
            /* Note that for some (width,height) combinations, where the     */
            /* width is not a multiple of 8, the sizes for bit- and          */
            /* byte-aligned data are equal, for example (7,7) or (15,6).  We */
            /* then prefer what `glyph_format' specifies.                    */

            let width = ctx.metrics.width as FtUInt;
            let height = ctx.metrics.height as FtUInt;

            let bit_size = (width * height + 7) >> 3;
            let byte_size = height * ((width + 7) >> 3);

            if bit_size < byte_size && byte_size as usize == p_limit - p {
                SbitLoader::ByteAligned
            } else {
                SbitLoader::BitAligned
            }
        }

        5 => SbitLoader::BitAligned,

        8 | 9 => {
            if glyph_format == 8 {
                if p + 1 > p_limit {
                    return Ok(());
                }

                p += 1; /* skip padding */
            }
            SbitLoader::Compound
        }

        17 | 18 | 19 => {
            /* small metrics, PNG image data   */
            /* big metrics, PNG image data     */
            /* metrics in EBLC, PNG image data */
            return Err(FT_ERR_UNIMPLEMENTED_FEATURE);
        }

        _ => return Err(FT_ERR_INVALID_TABLE),
    };

    if !decoder.bitmap_allocated {
        tt_sbit_decoder_alloc_bitmap(decoder, &mut ctx.face.root.glyph, ctx.metrics, metrics_only)?;
    }

    if metrics_only {
        return Ok(()); /* this is not an error */
    }

    match loader {
        SbitLoader::ByteAligned => tt_sbit_decoder_load_byte_aligned(
            decoder,
            &mut ctx.face.root.glyph.bitmap,
            ctx.metrics,
            t,
            p,
            p_limit,
            x_pos,
            y_pos,
        ),
        SbitLoader::BitAligned => tt_sbit_decoder_load_bit_aligned(
            decoder,
            &mut ctx.face.root.glyph.bitmap,
            ctx.metrics,
            t,
            p,
            p_limit,
            x_pos,
            y_pos,
        ),
        SbitLoader::Compound => {
            tt_sbit_decoder_load_compound(decoder, ctx, t, p, p_limit, x_pos, y_pos, recurse_count)
        }
    }
}

/// `tt_sbit_decoder_load_image`
fn tt_sbit_decoder_load_image(
    decoder: &mut TtSBitDecoderRec,
    ctx: &mut SbitCtx,
    glyph_index: FtUInt,
    x_pos: FtInt,
    y_pos: FtInt,
    recurse_count: FtUInt,
    metrics_only: bool,
) -> FtResult<()> {
    let t = std::mem::take(&mut ctx.face.sbit_table);
    let r = tt_sbit_decoder_load_image_inner(
        decoder,
        ctx,
        &t,
        glyph_index,
        x_pos,
        y_pos,
        recurse_count,
        metrics_only,
    );
    ctx.face.sbit_table = t;
    r
}

fn tt_sbit_decoder_load_image_inner(
    decoder: &mut TtSBitDecoderRec,
    ctx: &mut SbitCtx,
    t: &[u8],
    glyph_index: FtUInt,
    x_pos: FtInt,
    y_pos: FtInt,
    recurse_count: FtUInt,
    metrics_only: bool,
) -> FtResult<()> {
    let mut p = decoder.strike_index_array as usize;
    let p_limit = decoder.eblc_limit;
    let mut num_ranges = decoder.strike_index_count;
    let mut start: FtUInt = 0;
    let mut image_start: FtULong = 0;
    let mut image_end: FtULong = 0;

    let no_bitmap = || -> FtResult<()> {
        if recurse_count != 0 {
            return Err(FT_ERR_INVALID_COMPOSITE);
        }

        Err(FT_ERR_MISSING_BITMAP)
    };

    /* arbitrary recursion limit */
    if recurse_count > 100 {
        return Err(FT_ERR_INVALID_TABLE);
    }

    /* First, we find the correct strike range that applies to this */
    /* glyph index.                                                 */
    let mut found = false;
    while num_ranges > 0 {
        start = ft_next_ushort(t, &mut p) as FtUInt;
        let end = ft_next_ushort(t, &mut p) as FtUInt;

        if glyph_index >= start && glyph_index <= end {
            found = true;
            break;
        }

        p += 4; /* ignore index offset */
        num_ranges -= 1;
    }
    if !found {
        return no_bitmap();
    }

    /* FoundRange: */
    let mut image_offset = ft_next_ulong(t, &mut p) as FtULong;

    /* overflow check */
    p = decoder.strike_index_array as usize;
    if image_offset > (p_limit - p) as FtULong {
        return Err(FT_ERR_INVALID_TABLE);
    }

    p += image_offset as usize;
    if p + 8 > p_limit {
        return no_bitmap();
    }

    /* now find the glyph's location and extend within the ebdt table */
    let index_format = ft_next_ushort(t, &mut p) as FtUInt;
    let image_format = ft_next_ushort(t, &mut p) as FtUInt;
    image_offset = ft_next_ulong(t, &mut p) as FtULong;

    match index_format {
        1 => {
            /* 4-byte offsets relative to `image_offset' */
            p += 4 * (glyph_index - start) as usize;
            if p + 8 > p_limit {
                return no_bitmap();
            }

            image_start = ft_next_ulong(t, &mut p) as FtULong;
            image_end = ft_next_ulong(t, &mut p) as FtULong;

            if image_start == image_end {
                /* missing glyph */
                return no_bitmap();
            }
        }

        2 => {
            /* big metrics, constant image size */
            if p + 12 > p_limit {
                return no_bitmap();
            }

            let image_size = ft_next_ulong(t, &mut p) as FtULong;

            if tt_sbit_decoder_load_metrics(decoder, ctx.metrics, t, &mut p, p_limit, true).is_err()
            {
                return no_bitmap();
            }

            image_start = image_size.wrapping_mul((glyph_index - start) as FtULong);
            image_end = image_start.wrapping_add(image_size);
        }

        3 => {
            /* 2-byte offsets relative to 'image_offset' */
            p += 2 * (glyph_index - start) as usize;
            if p + 4 > p_limit {
                return no_bitmap();
            }

            image_start = ft_next_ushort(t, &mut p) as FtULong;
            image_end = ft_next_ushort(t, &mut p) as FtULong;

            if image_start == image_end {
                /* missing glyph */
                return no_bitmap();
            }
        }

        4 => {
            /* sparse glyph array with (glyph,offset) pairs */
            if p + 4 > p_limit {
                return no_bitmap();
            }

            let num_glyphs = ft_next_ulong(t, &mut p) as FtULong;

            /* overflow check for p + ( num_glyphs + 1 ) * 4 */
            if p + 4 > p_limit || num_glyphs > (((p_limit - p) >> 2) - 1) as FtULong {
                return no_bitmap();
            }

            let mut mm: FtULong = 0;
            while mm < num_glyphs {
                let gindex = ft_next_ushort(t, &mut p) as FtUInt;

                if gindex == glyph_index {
                    image_start = ft_next_ushort(t, &mut p) as FtULong;
                    p += 2;
                    image_end = ft_peek_ushort(t, p) as FtULong;
                    break;
                }
                p += 2;
                mm += 1;
            }

            if mm >= num_glyphs {
                return no_bitmap();
            }
        }

        5 | 19 => {
            /* constant metrics with sparse glyph codes */
            if p + 16 > p_limit {
                return no_bitmap();
            }

            let image_size = ft_next_ulong(t, &mut p) as FtULong;

            if tt_sbit_decoder_load_metrics(decoder, ctx.metrics, t, &mut p, p_limit, true).is_err()
            {
                return no_bitmap();
            }

            let num_glyphs = ft_next_ulong(t, &mut p) as FtULong;

            /* overflow check for p + 2 * num_glyphs */
            if num_glyphs > ((p_limit - p) >> 1) as FtULong {
                return no_bitmap();
            }

            let mut mm: FtULong = 0;
            while mm < num_glyphs {
                let gindex = ft_next_ushort(t, &mut p) as FtUInt;

                if gindex == glyph_index {
                    break;
                }
                mm += 1;
            }

            if mm >= num_glyphs {
                return no_bitmap();
            }

            image_start = image_size.wrapping_mul(mm);
            image_end = image_start.wrapping_add(image_size);
        }

        _ => return no_bitmap(),
    }

    if image_start > image_end {
        return no_bitmap();
    }

    image_end -= image_start;
    image_start = image_offset.wrapping_add(image_start);

    tt_sbit_decoder_load_bitmap(
        decoder,
        ctx,
        image_format,
        image_start,
        image_end,
        x_pos,
        y_pos,
        recurse_count,
        metrics_only,
    )
}

/// `tt_face_load_sbix_image`
fn tt_face_load_sbix_image(
    face: &mut TtFaceRec,
    stream: &mut FtStreamRec,
    strike_index: FtULong,
    mut glyph_index: FtUInt,
    metrics: &mut TtSBitMetricsRec,
    _metrics_only: bool,
) -> FtResult<()> {
    let mut recurse_depth: FtInt = 0;

    let strike_index = face.sbit_strike_map[strike_index as usize] as FtULong;

    metrics.width = 0;
    metrics.height = 0;

    let mut p = 8 + 4 * strike_index as usize;
    let strike_offset = ft_next_ulong(&face.sbit_table, &mut p) as FtULong;

    let (origin_offset_x, origin_offset_y);
    let error: FtResult<()>;

    loop {
        /* retry: */
        if glyph_index > face.root.num_glyphs as FtUInt {
            return Err(FT_ERR_INVALID_ARGUMENT);
        }

        if strike_offset >= face.ebdt_size
            || face.ebdt_size - strike_offset < 4 + glyph_index as FtULong * 4 + 8
        {
            return Err(FT_ERR_INVALID_FILE_FORMAT);
        }

        stream.seek(face.ebdt_start + strike_offset + 4 + glyph_index as FtULong * 4)?;
        stream.enter_frame(8)?;

        let glyph_start = stream.get_ulong() as FtULong;
        let glyph_end = stream.get_ulong() as FtULong;

        stream.exit_frame();

        if glyph_start == glyph_end {
            return Err(FT_ERR_MISSING_BITMAP);
        }
        if glyph_start > glyph_end
            || glyph_end - glyph_start < 8
            || face.ebdt_size - strike_offset < glyph_end
        {
            return Err(FT_ERR_INVALID_FILE_FORMAT);
        }

        stream.seek(face.ebdt_start + strike_offset + glyph_start)?;
        stream.enter_frame(glyph_end - glyph_start)?;

        let ox = stream.get_short() as FtInt;
        let oy = stream.get_short() as FtInt;

        let graphic_type = stream.get_ulong();

        match graphic_type {
            x if x == ft_make_tag(b'd', b'u', b'p', b'e') => {
                if recurse_depth < 4 {
                    glyph_index = stream.get_ushort() as FtUInt;
                    stream.exit_frame();
                    recurse_depth += 1;
                    continue;
                }
                error = Err(FT_ERR_INVALID_FILE_FORMAT);
            }

            x if x == ft_make_tag(b'p', b'n', b'g', b' ') => {
                error = Err(FT_ERR_UNIMPLEMENTED_FEATURE);
            }

            x if x == ft_make_tag(b'j', b'p', b'g', b' ')
                || x == ft_make_tag(b't', b'i', b'f', b'f')
                || x == ft_make_tag(b'r', b'g', b'b', b'l') =>
            {
                /* used on iOS 7.1 */
                error = Err(FT_ERR_UNKNOWN_FILE_FORMAT);
            }

            _ => {
                error = Err(FT_ERR_UNIMPLEMENTED_FEATURE);
            }
        }

        stream.exit_frame();
        origin_offset_x = ox;
        origin_offset_y = oy;
        break;
    }

    if error.is_ok() {
        let (_abearing, mut aadvance) = tt_face_get_metrics(face, stream, false, glyph_index);

        metrics.horiBearingX = origin_offset_x as FtShort;
        metrics.vertBearingX = origin_offset_x as FtShort;
        metrics.horiBearingY = (origin_offset_y + metrics.height as FtInt) as FtShort;
        metrics.vertBearingY = origin_offset_y as FtShort;
        metrics.horiAdvance = (aadvance as FtUInt * face.root.size.metrics.x_ppem as FtUInt
            / face.header.Units_Per_EM as FtUInt) as FtUShort;

        if face.vertical_info {
            aadvance = tt_face_get_metrics(face, stream, true, glyph_index).1;
        } else if face.os2.version != 0xFFFF {
            aadvance = ft_abs(face.os2.sTypoAscender as FtLong - face.os2.sTypoDescender as FtLong)
                as FtUShort;
        } else {
            aadvance =
                ft_abs(face.horizontal.Ascender as FtLong - face.horizontal.Descender as FtLong)
                    as FtUShort;
        }

        metrics.vertAdvance = (aadvance as FtUInt * face.root.size.metrics.x_ppem as FtUInt
            / face.header.Units_Per_EM as FtUInt) as FtUShort;
    }

    error
}

/// `tt_face_load_sbit_image` (into `face->root.glyph->bitmap`)
pub fn tt_face_load_sbit_image(
    face: &mut TtFaceRec,
    stream: &mut FtStreamRec,
    strike_index: FtULong,
    glyph_index: FtUInt,
    load_flags: FtUInt,
    metrics: &mut TtSBitMetricsRec,
) -> FtResult<()> {
    let mut error = match face.sbit_table_type {
        TT_SBIT_TABLE_TYPE_EBLC | TT_SBIT_TABLE_TYPE_CBLC => {
            let mut decoder = TtSBitDecoderRec::default();

            match tt_sbit_decoder_init(&mut decoder, face, stream, strike_index) {
                Ok(()) => {
                    let mut ctx = SbitCtx {
                        face,
                        stream,
                        metrics,
                    };
                    tt_sbit_decoder_load_image(
                        &mut decoder,
                        &mut ctx,
                        glyph_index,
                        0,
                        0,
                        0,
                        (load_flags & FT_LOAD_BITMAP_METRICS_ONLY as FtUInt) != 0,
                    )
                }
                Err(e) => Err(e),
            }
        }

        TT_SBIT_TABLE_TYPE_SBIX => tt_face_load_sbix_image(
            face,
            stream,
            strike_index,
            glyph_index,
            metrics,
            (load_flags & FT_LOAD_BITMAP_METRICS_ONLY as FtUInt) != 0,
        ),

        _ => Err(FT_ERR_UNKNOWN_FILE_FORMAT),
    };

    /* Flatten color bitmaps if color was not requested. */
    if error.is_ok()
        && load_flags & FT_LOAD_COLOR as FtUInt == 0
        && load_flags & FT_LOAD_BITMAP_METRICS_ONLY as FtUInt == 0
        && face.root.glyph.bitmap.pixel_mode == FT_PIXEL_MODE_BGRA
    {
        let mut new_map = FtBitmap::default();
        ft_bitmap_init(&mut new_map);

        /* Convert to 8bit grayscale. */
        error = ft_bitmap_convert(&face.root.glyph.bitmap, &mut new_map, 1);
        if error.is_ok() {
            let map = &mut face.root.glyph.bitmap;
            map.pixel_mode = new_map.pixel_mode;
            map.pitch = new_map.pitch;
            map.num_grays = new_map.num_grays;

            ft_glyphslot_set_bitmap(&mut face.root.glyph, new_map.buffer);
            face.root.glyph.internal.flags |= FT_GLYPH_OWN_BITMAP;
        }
    }

    error
}
