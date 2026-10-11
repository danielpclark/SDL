// Rust translation of src/pfr/pfrsbit.c and src/pfr/pfrsbit.h from
// FreeType (2.13.2, as SDL_ttf's external/freetype pins it).
// Copyright (C) 2002-2023 by David Turner, Robert Wilhelm, and Werner Lemberg.
// This is an altered (translated) version of the original software; it is
// used under the FreeType License (see FTL.TXT and LICENSE.txt).

//! FreeType PFR bitmap loader (body).
//!
//! The bit writer's line pointer is an offset into the bitmap's buffer
//! (that may move past either end after the last row, as C's does,
//! without being written through).

use super::super::base::ftcalc::ft_mul_div;
use super::super::base::ftobjs::*;
use super::super::fttypes::*;
use super::pfrload::*;
use super::pfrobjs::PfrFaceRec;
use super::pfrtypes::*;

/// `FT_INT_MAX`
const FT_INT_MAX: FtLong = i32::MAX as FtLong;
/// `FT_INT_MIN`
const FT_INT_MIN: FtLong = i32::MIN as FtLong;

/*************************************************************************/
/*************************************************************************/
/*****                                                               *****/
/*****                      PFR BIT WRITER                           *****/
/*****                                                               *****/
/*************************************************************************/
/*************************************************************************/

/// `PFR_BitWriter`
struct PfrBitWriterRec {
    line: isize,   /* current line start               */
    pitch: isize,  /* line size in bytes               */
    width: FtUInt, /* width in pixels/bits             */
    #[allow(dead_code)]
    rows: FtUInt, /* number of remaining rows to scan */
    total: FtUInt, /* total number of bits to draw     */
}

/// Writes `c` at `cur` in `buffer`.
#[inline]
fn put(buffer: &mut [u8], cur: isize, c: FtUInt) {
    if let Some(b) = usize::try_from(cur).ok().and_then(|i| buffer.get_mut(i)) {
        *b = c as FtByte;
    }
}

/// `pfr_bitwriter_init`
fn pfr_bitwriter_init(target: &FtBitmap, decreasing: bool) -> PfrBitWriterRec {
    let mut writer = PfrBitWriterRec {
        line: 0,
        pitch: target.pitch as isize,
        width: target.width,
        rows: target.rows,
        total: target.width.wrapping_mul(target.rows),
    };

    if !decreasing {
        writer.line += writer.pitch * (target.rows as FtInt - 1) as isize;
        writer.pitch = -writer.pitch;
    }

    writer
}

/// `pfr_bitwriter_decode_bytes`
fn pfr_bitwriter_decode_bytes(
    writer: &mut PfrBitWriterRec,
    buffer: &mut [u8],
    base: &[u8],
    mut p: usize,
    limit: usize,
) {
    let mut left = writer.width;
    let mut cur = writer.line;
    let mut mask: FtUInt = 0x80;
    let mut val: FtUInt = 0;
    let mut c: FtUInt = 0;

    let mut n = (limit - p) as FtUInt * 8;
    if n > writer.total {
        n = writer.total;
    }

    let reload = n & 7;

    while n > 0 {
        if (n & 7) == reload {
            val = base[p] as FtUInt;
            p += 1;
        }

        if val & 0x80 != 0 {
            c |= mask;
        }

        val <<= 1;
        mask >>= 1;

        left -= 1;
        if left == 0 {
            put(buffer, cur, c);
            left = writer.width;
            mask = 0x80;

            writer.line += writer.pitch;
            cur = writer.line;
            c = 0;
        } else if mask == 0 {
            put(buffer, cur, c);
            mask = 0x80;
            c = 0;
            cur += 1;
        }
        n -= 1;
    }

    if mask != 0x80 {
        put(buffer, cur, c);
    }
}

/// `pfr_bitwriter_decode_rle1`
fn pfr_bitwriter_decode_rle1(
    writer: &mut PfrBitWriterRec,
    buffer: &mut [u8],
    base: &[u8],
    mut p: usize,
    limit: usize,
) {
    let mut left = writer.width;
    let mut cur = writer.line;
    let mut mask: FtUInt = 0x80;
    let mut c: FtUInt = 0;

    let mut n = writer.total;

    let mut phase = 1;
    let mut counts: [FtInt; 2] = [0, 0];
    let mut count: FtInt = 0;
    let mut reload = true;

    while n > 0 {
        if reload {
            loop {
                if phase != 0 {
                    if p >= limit {
                        break;
                    }

                    let v = base[p] as FtInt;
                    p += 1;
                    counts[0] = v >> 4;
                    counts[1] = v & 15;
                    phase = 0;
                    count = counts[0];
                } else {
                    phase = 1;
                    count = counts[1];
                }

                if count != 0 {
                    break;
                }
            }
        }

        if phase != 0 {
            c |= mask;
        }

        mask >>= 1;

        left = left.wrapping_sub(1);
        if left == 0 {
            put(buffer, cur, c);
            left = writer.width;
            mask = 0x80;

            writer.line += writer.pitch;
            cur = writer.line;
            c = 0;
        } else if mask == 0 {
            put(buffer, cur, c);
            mask = 0x80;
            c = 0;
            cur += 1;
        }

        count = count.wrapping_sub(1);
        reload = count <= 0;
        n -= 1;
    }

    if mask != 0x80 {
        put(buffer, cur, c);
    }
}

/// `pfr_bitwriter_decode_rle2`
fn pfr_bitwriter_decode_rle2(
    writer: &mut PfrBitWriterRec,
    buffer: &mut [u8],
    base: &[u8],
    mut p: usize,
    limit: usize,
) {
    let mut left = writer.width;
    let mut cur = writer.line;
    let mut mask: FtUInt = 0x80;
    let mut c: FtUInt = 0;

    let mut n = writer.total;

    let mut phase = 1;
    let mut count: FtInt = 0;
    let mut reload = true;

    while n > 0 {
        if reload {
            loop {
                if p >= limit {
                    break;
                }

                count = base[p] as FtInt;
                p += 1;
                phase ^= 1;

                if count != 0 {
                    break;
                }
            }
        }

        if phase != 0 {
            c |= mask;
        }

        mask >>= 1;

        left = left.wrapping_sub(1);
        if left == 0 {
            put(buffer, cur, c);
            c = 0;
            mask = 0x80;
            left = writer.width;

            writer.line += writer.pitch;
            cur = writer.line;
        } else if mask == 0 {
            put(buffer, cur, c);
            c = 0;
            mask = 0x80;
            cur += 1;
        }

        count = count.wrapping_sub(1);
        reload = count <= 0;
        n -= 1;
    }

    if mask != 0x80 {
        put(buffer, cur, c);
    }
}

/*************************************************************************/
/*************************************************************************/
/*****                                                               *****/
/*****                  BITMAP DATA DECODING                         *****/
/*****                                                               *****/
/*************************************************************************/
/*************************************************************************/

/// `pfr_lookup_bitmap_data` (the records from the start of `base` to
/// `limit`)
fn pfr_lookup_bitmap_data(
    base: &[u8],
    limit: usize,
    count: FtUInt,
    flags: &mut FtUInt,
    char_code: FtUInt,
    found_offset: &mut FtULong,
    found_size: &mut FtULong,
) {
    let two = *flags & PFR_BITMAP_2BYTE_CHARCODE != 0;
    let mut buff: usize;

    let mut char_len: FtUInt = 4;
    if two {
        char_len += 1;
    }
    if *flags & PFR_BITMAP_2BYTE_SIZE != 0 {
        char_len += 1;
    }
    if *flags & PFR_BITMAP_3BYTE_OFFSET != 0 {
        char_len += 1;
    }

    if *flags & PFR_BITMAP_CHARCODES_VALIDATED == 0 {
        *flags |= PFR_BITMAP_VALID_CHARCODES;

        let mut prev_code: FtLong = -1;
        let lim = count as u64 * char_len as u64;

        if lim > limit as u64 {
            *flags &= !PFR_BITMAP_VALID_CHARCODES;
        } else {
            /* check whether records are sorted by code */
            let mut p = 0usize;
            while (p as u64) < lim {
                let code: FtUInt = if two {
                    ft_peek_ushort16(base, p)
                } else {
                    base[p] as FtUInt
                };

                if code as FtLong <= prev_code {
                    *flags &= !PFR_BITMAP_VALID_CHARCODES;
                    break;
                }

                prev_code = code as FtLong;
                p += char_len as usize;
            }
        }

        *flags |= PFR_BITMAP_CHARCODES_VALIDATED;
    }

    'fail: {
        /* ignore bitmaps in case table is not valid     */
        /* (this might be sanitized, but PFR is dead...) */
        if *flags & PFR_BITMAP_VALID_CHARCODES == 0 {
            break 'fail;
        }

        let mut min: FtUInt = 0;
        let mut max: FtUInt = count;
        let mut mid: FtUInt = min + (max - min) / 2;

        /* binary search */
        while min < max {
            buff = mid as usize * char_len as usize;
            let code: FtUInt = if two {
                pfr_next_ushort(base, &mut buff)
            } else {
                pfr_next_byte(base, &mut buff)
            };

            if char_code < code {
                max = mid;
            } else if char_code > code {
                min = mid + 1;
            } else {
                /* Found_It: */
                if *flags & PFR_BITMAP_2BYTE_SIZE != 0 {
                    *found_size = pfr_next_ushort(base, &mut buff) as FtULong;
                } else {
                    *found_size = pfr_next_byte(base, &mut buff) as FtULong;
                }

                if *flags & PFR_BITMAP_3BYTE_OFFSET != 0 {
                    *found_offset = pfr_next_ulong(base, &mut buff) as FtULong;
                } else {
                    *found_offset = pfr_next_ushort(base, &mut buff) as FtULong;
                }
                return;
            }

            /* reasonable prediction in a continuous block */
            mid = mid.wrapping_add(char_code.wrapping_sub(code));
            if mid >= max || mid < min {
                mid = min + (max - min) / 2;
            }
        }
    }

    /* Fail: */
    /* Not found */
    *found_size = 0;
    *found_offset = 0;
}

/// `FT_PEEK_USHORT`, as an `FT_UInt`
fn ft_peek_ushort16(base: &[u8], p: usize) -> FtUInt {
    super::super::base::ftstream::ft_peek_ushort(base, p) as FtUInt
}

/// The bitmap metrics `pfr_load_bitmap_metrics` reads.
#[derive(Debug, Clone, Copy, Default)]
struct PfrBitmapMetrics {
    xpos: FtLong,
    ypos: FtLong,
    xsize: FtUInt,
    ysize: FtUInt,
    advance: FtLong,
    format: FtUInt,
}

/// `pfr_load_bitmap_metrics`: load bitmap metrics.  `*aadvance' must be
/// set to the default value before calling this function
fn pfr_load_bitmap_metrics(
    base: &[u8],
    pdata: &mut usize,
    limit: usize,
    scaled_advance: FtLong,
    out: &mut PfrBitmapMetrics,
) -> FtResult<()> {
    let mut p = *pdata;

    macro_rules! check {
        ($x:expr) => {
            if !pfr_check(p, ($x) as u64, limit) {
                /* Too_Short: */
                return Err(FT_ERR_INVALID_TABLE);
            }
        };
    }

    check!(1);
    let mut flags = pfr_next_byte(base, &mut p) as FtByte;

    let mut xpos: FtLong = 0;
    let mut ypos: FtLong = 0;
    let mut xsize: FtUInt = 0;
    let mut ysize: FtUInt = 0;
    let mut advance: FtLong = 0;

    match flags & 3 {
        0 => {
            check!(1);
            let b = pfr_next_byte(base, &mut p) as FtByte;
            xpos = ((b as FtChar) >> 4) as FtLong;
            ypos = (((b << 4) as FtChar) >> 4) as FtLong;
        }

        1 => {
            check!(2);
            xpos = pfr_next_int8(base, &mut p) as FtLong;
            ypos = pfr_next_int8(base, &mut p) as FtLong;
        }

        2 => {
            check!(4);
            xpos = pfr_next_short(base, &mut p) as FtLong;
            ypos = pfr_next_short(base, &mut p) as FtLong;
        }

        3 => {
            check!(6);
            xpos = pfr_next_long(base, &mut p) as FtLong;
            ypos = pfr_next_long(base, &mut p) as FtLong;
        }

        _ => {}
    }

    flags >>= 2;
    match flags & 3 {
        0 => {
            /* blank image */
            xsize = 0;
            ysize = 0;
        }

        1 => {
            check!(1);
            let b = pfr_next_byte(base, &mut p);
            xsize = (b >> 4) & 0xF;
            ysize = b & 0xF;
        }

        2 => {
            check!(2);
            xsize = pfr_next_byte(base, &mut p);
            ysize = pfr_next_byte(base, &mut p);
        }

        3 => {
            check!(4);
            xsize = pfr_next_ushort(base, &mut p);
            ysize = pfr_next_ushort(base, &mut p);
        }

        _ => {}
    }

    flags >>= 2;
    match flags & 3 {
        0 => {
            advance = scaled_advance;
        }

        1 => {
            check!(1);
            advance = pfr_next_int8(base, &mut p) as FtLong * 256;
        }

        2 => {
            check!(2);
            advance = pfr_next_short(base, &mut p) as FtLong;
        }

        3 => {
            check!(3);
            advance = pfr_next_long(base, &mut p) as FtLong;
        }

        _ => {}
    }

    out.xpos = xpos;
    out.ypos = ypos;
    out.xsize = xsize;
    out.ysize = ysize;
    out.advance = advance;
    out.format = (flags >> 2) as FtUInt;
    *pdata = p;

    /* Exit: */
    Ok(())
}

/// `pfr_load_bitmap_bits`
fn pfr_load_bitmap_bits(
    base: &[u8],
    p: usize,
    limit: usize,
    format: FtUInt,
    decreasing: bool,
    target: &mut FtBitmap,
) -> FtResult<()> {
    if target.rows > 0 && target.width > 0 {
        let mut writer = pfr_bitwriter_init(target, decreasing);
        let buffer = &mut target.buffer;

        match format {
            0 => {
                /* packed bits */
                pfr_bitwriter_decode_bytes(&mut writer, buffer, base, p, limit);
            }

            1 => {
                /* RLE1 */
                pfr_bitwriter_decode_rle1(&mut writer, buffer, base, p, limit);
            }

            2 => {
                /* RLE2 */
                pfr_bitwriter_decode_rle2(&mut writer, buffer, base, p, limit);
            }

            _ => {}
        }
    }

    Ok(())
}

/*************************************************************************/
/*************************************************************************/
/*****                                                               *****/
/*****                     BITMAP LOADING                            *****/
/*****                                                               *****/
/*************************************************************************/
/*************************************************************************/

/// `pfr_slot_load_bitmap`
pub fn pfr_slot_load_bitmap(
    face: &mut PfrFaceRec,
    glyph_index: FtUInt,
    metrics_only: bool,
) -> FtResult<()> {
    let phys = &mut face.phy_font;
    let root = &mut face.root;
    let x_ppem = root.size.metrics.x_ppem;
    let y_ppem = root.size.metrics.y_ppem;
    let size_height = root.size.metrics.height;
    let Some(stream) = root.stream.as_mut() else {
        return Err(FT_ERR_INVALID_STREAM_HANDLE);
    };
    let glyph = &mut root.glyph;

    let character = phys.chars[glyph_index as usize];

    /* look up a bitmap strike corresponding to the current */
    /* character dimensions                                 */
    let mut found = None;
    for (n, strike) in phys.strikes[..phys.num_strikes as usize].iter().enumerate() {
        if strike.x_ppm == x_ppem as FtUInt && strike.y_ppm == y_ppem as FtUInt {
            found = Some(n);
            break;
        }
    }

    /* couldn't find it */
    let Some(n) = found else {
        return Err(FT_ERR_INVALID_ARGUMENT);
    };

    /* Found_Strike: */
    let gps_offset: FtULong;
    let gps_size: FtULong;

    /* now look up the glyph's position within the file */
    {
        let strike = &mut phys.strikes[n];
        let mut char_len: FtUInt = 4;
        if strike.flags & PFR_BITMAP_2BYTE_CHARCODE != 0 {
            char_len += 1;
        }
        if strike.flags & PFR_BITMAP_2BYTE_SIZE != 0 {
            char_len += 1;
        }
        if strike.flags & PFR_BITMAP_3BYTE_OFFSET != 0 {
            char_len += 1;
        }

        /* access data directly in the frame to speed up lookups */
        stream.seek(phys.bct_offset.wrapping_add(strike.bct_offset as FtULong))?;
        stream.enter_frame(char_len as FtULong * strike.num_bitmaps as FtULong)?;

        let mut offset: FtULong = 0;
        let mut size: FtULong = 0;
        pfr_lookup_bitmap_data(
            stream.frame_data(),
            stream.limit(),
            strike.num_bitmaps,
            &mut strike.flags,
            character.char_code,
            &mut offset,
            &mut size,
        );

        stream.exit_frame();

        if size == 0 {
            /* could not find a bitmap program string for this glyph */
            return Err(FT_ERR_INVALID_ARGUMENT);
        }

        gps_offset = offset;
        gps_size = size;
    }

    /* get the bitmap metrics */
    {
        let mut m = PfrBitmapMetrics::default();

        /* compute linear advance */
        let mut advance: FtLong = character.advance as FtLong;
        if phys.metrics_resolution != phys.outline_resolution {
            advance = ft_mul_div(
                advance,
                phys.outline_resolution as FtLong,
                phys.metrics_resolution as FtLong,
            );
        }

        glyph.linearHoriAdvance = advance;

        /* compute default advance, i.e., scaled advance; this can be */
        /* overridden in the bitmap header of certain glyphs          */
        advance = ft_mul_div(
            (x_ppem as FtFixed) << 8,
            character.advance as FtLong,
            phys.metrics_resolution as FtLong,
        );

        stream.seek((face.header.gps_section_offset as FtULong).wrapping_add(gps_offset))?;
        stream.enter_frame(gps_size)?;

        let base = stream.frame_data();
        let limit = stream.limit();
        let mut p = 0usize;

        let mut error = pfr_load_bitmap_metrics(base, &mut p, limit, advance, &mut m);

        'exit1: {
            if error.is_err() {
                break 'exit1;
            }

            let xsize = m.xsize;
            let ysize = m.ysize;
            let xpos = m.xpos;
            let ypos = m.ypos;
            let format = m.format;
            let advance = m.advance;

            /*
             * Before allocating the target bitmap, we check whether the given
             * bitmap dimensions are valid, depending on the image format.
             *
             * Format 0: We have a stream of pixels (with 8 pixels per byte).
             *
             *             (xsize * ysize + 7) / 8 <= gps_size
             *
             * Format 1: Run-length encoding; the high nibble holds the number of
             *           white bits, the low nibble the number of black bits.  In
             *           other words, a single byte can represent at most 15
             *           pixels.
             *
             *             xsize * ysize <= 15 * gps_size
             *
             * Format 2: Run-length encoding; the high byte holds the number of
             *           white bits, the low byte the number of black bits.  In
             *           other words, two bytes can represent at most 255 pixels.
             *
             *             xsize * ysize <= 255 * (gps_size + 1) / 2
             */
            match format {
                0 => {
                    if (xsize as FtULong * ysize as FtULong).div_ceil(8) > gps_size {
                        error = Err(FT_ERR_INVALID_TABLE);
                    }
                }
                1 => {
                    if xsize as FtULong * ysize as FtULong > 15 * gps_size {
                        error = Err(FT_ERR_INVALID_TABLE);
                    }
                }
                2 => {
                    if xsize as FtULong * ysize as FtULong > 255 * gps_size.div_ceil(2) {
                        error = Err(FT_ERR_INVALID_TABLE);
                    }
                }
                _ => {
                    error = Err(FT_ERR_INVALID_TABLE);
                }
            }

            if error.is_err() {
                break 'exit1;
            }

            /*
             * XXX: on 16bit systems we return an error for huge bitmaps
             *      that cause size truncation, because truncated
             *      size properties make bitmap glyphs broken.
             */
            if xpos > FT_INT_MAX
                || xpos < FT_INT_MIN
                || ysize as FtLong > FT_INT_MAX
                || ypos > FT_INT_MAX - ysize as FtLong
                || ypos + (ysize as FtLong) < FT_INT_MIN
            {
                error = Err(FT_ERR_INVALID_PIXEL_SIZE);
            }

            if error.is_ok() {
                glyph.format = FT_GLYPH_FORMAT_BITMAP;

                /* Set up glyph bitmap and metrics */

                /* XXX: needs casts to fit FT_Bitmap.{width|rows|pitch} */
                glyph.bitmap.width = xsize;
                glyph.bitmap.rows = ysize;
                glyph.bitmap.pitch = (xsize as FtInt + 7) >> 3;
                glyph.bitmap.pixel_mode = FT_PIXEL_MODE_MONO;

                /* XXX: needs casts to fit FT_Glyph_Metrics.{width|height} */
                glyph.metrics.width = (xsize as FtPos) << 6;
                glyph.metrics.height = (ysize as FtPos) << 6;
                glyph.metrics.horiBearingX = xpos * 64;
                glyph.metrics.horiBearingY = ypos * 64;
                glyph.metrics.horiAdvance = ft_pix_round(advance >> 2);
                glyph.metrics.vertBearingX = -glyph.metrics.width >> 1;
                glyph.metrics.vertBearingY = 0;
                glyph.metrics.vertAdvance = size_height;

                /* XXX: needs casts fit FT_GlyphSlotRec.bitmap_{left|top} */
                glyph.bitmap_left = xpos as FtInt;
                glyph.bitmap_top = (ypos + ysize as FtLong) as FtInt;

                if metrics_only {
                    break 'exit1;
                }

                /* Allocate and read bitmap data */
                {
                    let len: FtULong = glyph.bitmap.pitch as FtULong * ysize as FtULong;

                    error = ft_glyphslot_alloc_bitmap(glyph, len);
                    if error.is_ok() {
                        error = pfr_load_bitmap_bits(
                            base,
                            p,
                            limit,
                            format,
                            face.header.color_flags & PFR_FLAG_INVERT_BITMAP != 0,
                            &mut glyph.bitmap,
                        );
                    }
                }
            }
        }

        /* Exit1: */
        stream.exit_frame();

        error
    }

    /* Exit: */
}
