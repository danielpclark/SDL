// Rust translation of src/base/ftbitmap.c from FreeType (2.13.2, as SDL_ttf's
// external/freetype pins it).
// Copyright (C) 2004-2023 by David Turner, Robert Wilhelm, and Werner Lemberg.
// This is an altered (translated) version of the original software; it is
// used under the FreeType License (see FTL.TXT and LICENSE.txt).

//! FreeType utility functions for bitmaps (body).
//!
//! The `library` arguments are left out (they only supply the memory
//! manager); a bitmap's empty buffer stands for C's NULL buffer.  Rows are
//! addressed as in C: with a negative pitch the top row is the last one in
//! memory.

use super::super::fttypes::*;
use super::super::tttypes::FtColor;
use super::ftmemory::{ft_alloc, ft_qalloc};
use super::ftobjs::{FtGlyphSlotRec, FT_GLYPH_OWN_BITMAP};

/// `FT_Bitmap_Init`
pub fn ft_bitmap_init(abitmap: &mut FtBitmap) {
    *abitmap = FtBitmap::default();
}

/// `FT_Bitmap_New`: deprecated function name; retained for ABI
/// compatibility
pub fn ft_bitmap_new(abitmap: &mut FtBitmap) {
    *abitmap = FtBitmap::default();
}

/// `FT_Bitmap_Copy`
pub fn ft_bitmap_copy(source: &FtBitmap, target: &mut FtBitmap) -> FtResult<()> {
    let flip = (source.pitch < 0 && target.pitch > 0) || (source.pitch > 0 && target.pitch < 0);

    *target = FtBitmap {
        buffer: Vec::new(),
        ..*source
    };

    if flip {
        target.pitch = -target.pitch;
    }

    if source.buffer.is_empty() {
        return Ok(());
    }

    let mut pitch = source.pitch;
    if pitch < 0 {
        pitch = -pitch;
    }

    let pitch = pitch as usize;
    target.buffer = ft_qalloc((target.rows as usize * pitch) as FtLong)?;

    if flip {
        /* take care of bitmap flow */
        let mut s = 0usize;
        let mut t = pitch * (target.rows as usize).wrapping_sub(1);

        for _ in 0..target.rows {
            target.buffer[t..t + pitch].copy_from_slice(&source.buffer[s..s + pitch]);
            s += pitch;
            t = t.wrapping_sub(pitch);
        }
    } else {
        let n = source.rows as usize * pitch;
        target.buffer[..n].copy_from_slice(&source.buffer[..n]);
    }

    Ok(())
}

/// `ft_bitmap_assure_buffer`: Enlarge `bitmap' horizontally and vertically
/// by `xpixels' and `ypixels', respectively.
fn ft_bitmap_assure_buffer(
    bitmap: &mut FtBitmap,
    xpixels: FtUInt,
    ypixels: FtUInt,
) -> FtResult<()> {
    let bpp: FtUInt;
    let new_pitch: u32;

    let width = bitmap.width;
    let height = bitmap.rows;
    let pitch = bitmap.pitch.unsigned_abs();

    match bitmap.pixel_mode {
        FT_PIXEL_MODE_MONO => {
            bpp = 1;
            new_pitch = (width + xpixels + 7) >> 3;
        }
        FT_PIXEL_MODE_GRAY2 => {
            bpp = 2;
            new_pitch = (width + xpixels + 3) >> 2;
        }
        FT_PIXEL_MODE_GRAY4 => {
            bpp = 4;
            new_pitch = (width + xpixels + 1) >> 1;
        }
        FT_PIXEL_MODE_GRAY | FT_PIXEL_MODE_LCD | FT_PIXEL_MODE_LCD_V => {
            bpp = 8;
            new_pitch = width + xpixels;
        }
        _ => return Err(FT_ERR_INVALID_GLYPH_FORMAT),
    }

    /* if no need to allocate memory */
    if ypixels == 0 && new_pitch <= pitch {
        /* zero the padding */
        let bit_width = pitch * 8;
        let bit_last = (width + xpixels) * bpp;

        if bit_last < bit_width {
            let mut line = (bit_last >> 3) as usize;
            let mut end = pitch as usize;
            let shift = bit_last & 7;
            let mask = 0xFF00u32 >> shift;

            for _ in 0..height {
                let mut write = line;

                if shift > 0 {
                    bitmap.buffer[write] = (bitmap.buffer[write] as u32 & mask) as FtByte;
                    write += 1;
                }
                if write < end {
                    bitmap.buffer[write..end].fill(0);
                }

                line += pitch as usize;
                end += pitch as usize;
            }
        }

        return Ok(());
    }

    /* otherwise allocate new buffer */
    let mut buffer = ft_qalloc(((bitmap.rows + ypixels) as usize * new_pitch as usize) as FtLong)?;

    /* new rows get added at the top of the bitmap, */
    /* thus take care of the flow direction         */
    let len = ((width * bpp + 7) >> 3) as usize;
    let delta = new_pitch as usize - len;
    let limit = pitch as usize * bitmap.rows as usize;
    let np = new_pitch as usize;
    if bitmap.pitch > 0 {
        let mut in_ = 0usize;
        let mut out = 0usize;

        buffer[..np * ypixels as usize].fill(0);
        out += np * ypixels as usize;

        while in_ < limit {
            buffer[out..out + len].copy_from_slice(&bitmap.buffer[in_..in_ + len]);
            in_ += pitch as usize;
            out += len;

            /* we use FT_QALLOC_MULT, which doesn't zero out the buffer;      */
            /* consequently, we have to manually zero out the remaining bytes */
            buffer[out..out + delta].fill(0);
            out += delta;
        }
    } else {
        let mut in_ = 0usize;
        let mut out = 0usize;

        while in_ < limit {
            buffer[out..out + len].copy_from_slice(&bitmap.buffer[in_..in_ + len]);
            in_ += pitch as usize;
            out += len;

            buffer[out..out + delta].fill(0);
            out += delta;
        }

        buffer[out..out + np * ypixels as usize].fill(0);
    }

    bitmap.buffer = buffer;

    /* set pitch only, width and height are left untouched */
    if bitmap.pitch < 0 {
        bitmap.pitch = -(new_pitch as i32);
    } else {
        bitmap.pitch = new_pitch as i32;
    }

    Ok(())
}

/// `FT_Bitmap_Embolden`
pub fn ft_bitmap_embolden(
    bitmap: &mut FtBitmap,
    x_strength: FtPos,
    y_strength: FtPos,
) -> FtResult<()> {
    if bitmap.buffer.is_empty() {
        return Err(FT_ERR_INVALID_ARGUMENT);
    }

    if (ft_pix_round(x_strength) >> 6) > FtInt::MAX as FtPos
        || (ft_pix_round(y_strength) >> 6) > FtInt::MAX as FtPos
    {
        return Err(FT_ERR_INVALID_ARGUMENT);
    }

    let mut xstr = (ft_pix_round(x_strength) as FtInt) >> 6;
    let mut ystr = (ft_pix_round(y_strength) as FtInt) >> 6;

    if xstr == 0 && ystr == 0 {
        return Ok(());
    } else if xstr < 0 || ystr < 0 {
        return Err(FT_ERR_INVALID_ARGUMENT);
    }

    match bitmap.pixel_mode {
        FT_PIXEL_MODE_GRAY2 | FT_PIXEL_MODE_GRAY4 => {
            let mut tmp = FtBitmap::default();

            /* convert to 8bpp */
            ft_bitmap_init(&mut tmp);
            ft_bitmap_convert(bitmap, &mut tmp, 1)?;

            ft_bitmap_done(bitmap);
            *bitmap = tmp;
        }

        FT_PIXEL_MODE_MONO => {
            if xstr > 8 {
                xstr = 8;
            }
        }

        FT_PIXEL_MODE_LCD => {
            xstr *= 3;
        }

        FT_PIXEL_MODE_LCD_V => {
            ystr *= 3;
        }

        FT_PIXEL_MODE_BGRA => {
            /* We don't embolden color glyphs. */
            return Ok(());
        }

        _ => {}
    }

    ft_bitmap_assure_buffer(bitmap, xstr as FtUInt, ystr as FtUInt)?;

    /* take care of bitmap flow */
    let mut pitch = bitmap.pitch;
    let mut p: isize = if pitch > 0 {
        (pitch * ystr) as isize
    } else {
        pitch = -pitch;
        (pitch as usize * (bitmap.rows as usize).wrapping_sub(1)) as isize
    };

    let num_grays = bitmap.num_grays as i32;
    let is_mono = bitmap.pixel_mode == FT_PIXEL_MODE_MONO;
    let bpitch = bitmap.pitch as isize;
    let buf = &mut bitmap.buffer;

    /* for each row */
    for _ in 0..bitmap.rows {
        let row = p as usize;

        /*
         * Horizontally:
         *
         * From the last pixel on, make each pixel or'ed with the
         * `xstr' pixels before it.
         */
        let mut x = pitch - 1;
        while x >= 0 {
            let xu = row + x as usize;
            let tmp = buf[xu];
            for i in 1..=xstr {
                if is_mono {
                    buf[xu] |= tmp >> i;

                    /* the maximum value of 8 for `xstr' comes from here */
                    if x > 0 {
                        buf[xu] |= ((buf[xu - 1] as u32) << (8 - i)) as u8;
                    }
                } else if x - i >= 0 {
                    let prev = buf[row + (x - i) as usize] as i32;
                    if buf[xu] as i32 + prev > num_grays - 1 {
                        buf[xu] = (num_grays - 1) as u8;
                        break;
                    } else {
                        buf[xu] = (buf[xu] as i32 + prev) as u8;
                        if buf[xu] as i32 == num_grays - 1 {
                            break;
                        }
                    }
                } else {
                    break;
                }
            }
            x -= 1;
        }

        /*
         * Vertically:
         *
         * Make the above `ystr' rows or'ed with it.
         */
        for x in 1..=ystr {
            let q = (p - bpitch * x as isize) as usize;
            for i in 0..pitch as usize {
                buf[q + i] |= buf[row + i];
            }
        }

        p += bpitch;
    }

    bitmap.width += xstr as FtUInt;
    bitmap.rows += ystr as FtUInt;

    Ok(())
}

/// `ft_gray_for_premultiplied_srgb_bgra`
fn ft_gray_for_premultiplied_srgb_bgra(bgra: &[u8]) -> FtByte {
    let a = bgra[3] as FtUInt;

    /* Short-circuit transparent color to avoid division by zero. */
    if a == 0 {
        return 0;
    }

    /*
     * Luminosity for sRGB is defined using ~0.2126,0.7152,0.0722
     * coefficients for RGB channels *on the linear colors*.
     * A gamma of 2.2 is fair to assume.  And then, we need to
     * undo the premultiplication too.
     *
     *   http://www.brucelindbloom.com/index.html?WorkingSpaceInfo.html#SideNotes
     *
     * We do the computation with integers only, applying a gamma of 2.0.
     * We guarantee 32-bit arithmetic to avoid overflow but the resulting
     * luminosity fits into 16 bits.
     *
     */
    let b = bgra[0] as FtULong;
    let g = bgra[1] as FtULong;
    let r = bgra[2] as FtULong;
    let l = ((4731 /* 0.072186 * 65536 */ * b * b
        + 46868 /* 0.715158 * 65536 */ * g * g
        + 13937 /* 0.212656 * 65536 */ * r * r)
        >> 16) as FtUInt;

    /*
     * Final transparency can be determined as follows.
     *
     * - If alpha is zero, we want 0.
     * - If alpha is zero and luminosity is zero, we want 255.
     * - If alpha is zero and luminosity is one, we want 0.
     *
     * So the formula is a * (1 - l) = a - l * a.
     *
     * We still need to undo premultiplication by dividing l by a*a.
     *
     */
    a.wrapping_sub(l / a) as FtByte
}

/// `FT_Bitmap_Convert`
pub fn ft_bitmap_convert(
    source: &FtBitmap,
    target: &mut FtBitmap,
    alignment: FtInt,
) -> FtResult<()> {
    let mut error = Ok(());

    match source.pixel_mode {
        FT_PIXEL_MODE_MONO | FT_PIXEL_MODE_GRAY | FT_PIXEL_MODE_GRAY2 | FT_PIXEL_MODE_GRAY4
        | FT_PIXEL_MODE_LCD | FT_PIXEL_MODE_LCD_V | FT_PIXEL_MODE_BGRA => {
            let mut width = source.width as FtInt;
            let neg = (target.pitch == 0 && source.pitch < 0) || target.pitch < 0;

            ft_bitmap_done(target);

            target.pixel_mode = FT_PIXEL_MODE_GRAY;
            target.rows = source.rows;
            target.width = source.width;

            if alignment != 0 {
                let rem = width % alignment;

                if rem != 0 {
                    width = if alignment > 0 {
                        width - rem + alignment
                    } else {
                        width - rem - alignment
                    };
                }
            }

            target.buffer = ft_qalloc(target.rows as FtLong * width as FtLong)?;

            target.pitch = if neg { -width } else { width };
        }
        _ => error = Err(FT_ERR_INVALID_ARGUMENT),
    }

    let mut s: isize = 0;
    let mut t: isize = 0;

    /* take care of bitmap flow */
    if source.pitch < 0 {
        s -= source.pitch as isize * (source.rows as isize - 1);
    }
    if target.pitch < 0 {
        t -= target.pitch as isize * (target.rows as isize - 1);
    }

    let sbuf = &source.buffer;
    let spitch = source.pitch as isize;
    let tpitch = target.pitch as isize;

    match source.pixel_mode {
        FT_PIXEL_MODE_MONO => {
            target.num_grays = 2;
            let tbuf = &mut target.buffer;

            for _ in 0..source.rows {
                let mut ss = s as usize;
                let mut tt = t as usize;

                /* get the full bytes */
                for _ in 0..source.width >> 3 {
                    let val = sbuf[ss] as FtInt; /* avoid a byte->int cast on each line */

                    tbuf[tt] = ((val & 0x80) >> 7) as FtByte;
                    tbuf[tt + 1] = ((val & 0x40) >> 6) as FtByte;
                    tbuf[tt + 2] = ((val & 0x20) >> 5) as FtByte;
                    tbuf[tt + 3] = ((val & 0x10) >> 4) as FtByte;
                    tbuf[tt + 4] = ((val & 0x08) >> 3) as FtByte;
                    tbuf[tt + 5] = ((val & 0x04) >> 2) as FtByte;
                    tbuf[tt + 6] = ((val & 0x02) >> 1) as FtByte;
                    tbuf[tt + 7] = (val & 0x01) as FtByte;

                    tt += 8;
                    ss += 1;
                }

                /* get remaining pixels (if any) */
                let j = source.width & 7;
                if j > 0 {
                    let mut val = sbuf[ss] as FtInt;

                    for _ in 0..j {
                        tbuf[tt] = ((val & 0x80) >> 7) as FtByte;
                        val <<= 1;
                        tt += 1;
                    }
                }

                s += spitch;
                t += tpitch;
            }
        }

        FT_PIXEL_MODE_GRAY | FT_PIXEL_MODE_LCD | FT_PIXEL_MODE_LCD_V => {
            let width = source.width as usize;
            target.num_grays = 256;
            let tbuf = &mut target.buffer;

            for _ in 0..source.rows {
                tbuf[t as usize..t as usize + width]
                    .copy_from_slice(&sbuf[s as usize..s as usize + width]);

                s += spitch;
                t += tpitch;
            }
        }

        FT_PIXEL_MODE_GRAY2 => {
            target.num_grays = 4;
            let tbuf = &mut target.buffer;

            for _ in 0..source.rows {
                let mut ss = s as usize;
                let mut tt = t as usize;

                /* get the full bytes */
                for _ in 0..source.width >> 2 {
                    let val = sbuf[ss] as FtInt;

                    tbuf[tt] = ((val & 0xC0) >> 6) as FtByte;
                    tbuf[tt + 1] = ((val & 0x30) >> 4) as FtByte;
                    tbuf[tt + 2] = ((val & 0x0C) >> 2) as FtByte;
                    tbuf[tt + 3] = (val & 0x03) as FtByte;

                    ss += 1;
                    tt += 4;
                }

                let j = source.width & 3;
                if j > 0 {
                    let mut val = sbuf[ss] as FtInt;

                    for _ in 0..j {
                        tbuf[tt] = ((val & 0xC0) >> 6) as FtByte;
                        val <<= 2;
                        tt += 1;
                    }
                }

                s += spitch;
                t += tpitch;
            }
        }

        FT_PIXEL_MODE_GRAY4 => {
            target.num_grays = 16;
            let tbuf = &mut target.buffer;

            for _ in 0..source.rows {
                let mut ss = s as usize;
                let mut tt = t as usize;

                /* get the full bytes */
                for _ in 0..source.width >> 1 {
                    let val = sbuf[ss] as FtInt;

                    tbuf[tt] = ((val & 0xF0) >> 4) as FtByte;
                    tbuf[tt + 1] = (val & 0x0F) as FtByte;

                    ss += 1;
                    tt += 2;
                }

                if source.width & 1 != 0 {
                    tbuf[tt] = ((sbuf[ss] & 0xF0) >> 4) as FtByte;
                }

                s += spitch;
                t += tpitch;
            }
        }

        FT_PIXEL_MODE_BGRA => {
            target.num_grays = 256;
            let tbuf = &mut target.buffer;

            for _ in 0..source.rows {
                let mut ss = s as usize;
                let mut tt = t as usize;

                for _ in 0..source.width {
                    tbuf[tt] = ft_gray_for_premultiplied_srgb_bgra(&sbuf[ss..ss + 4]);

                    ss += 4;
                    tt += 1;
                }

                s += spitch;
                t += tpitch;
            }
        }

        _ => {}
    }

    error
}

/// `FT_Bitmap_Blend`
pub fn ft_bitmap_blend(
    source_: &FtBitmap,
    source_offset_: FtVector,
    target: &mut FtBitmap,
    atarget_offset: &mut FtVector,
    color: FtColor,
) -> FtResult<()> {
    let mut source_bitmap = FtBitmap::default();
    let mut free_target_bitmap_on_error = false;

    if !(target.pixel_mode == FT_PIXEL_MODE_NONE
        || (target.pixel_mode == FT_PIXEL_MODE_BGRA && !target.buffer.is_empty()))
    {
        return Err(FT_ERR_INVALID_ARGUMENT);
    }

    if source_.pixel_mode == FT_PIXEL_MODE_NONE {
        return Ok(()); /* nothing to do */
    }

    /* pitches must have the same sign */
    if target.pixel_mode == FT_PIXEL_MODE_BGRA && (source_.pitch ^ target.pitch) < 0 {
        return Err(FT_ERR_INVALID_ARGUMENT);
    }

    if !(source_.width != 0 && source_.rows != 0) {
        return Ok(()); /* nothing to do */
    }

    /* assure integer pixel offsets */
    let source_offset = FtVector {
        x: ft_pix_floor(source_offset_.x),
        y: ft_pix_floor(source_offset_.y),
    };
    let target_offset = FtVector {
        x: ft_pix_floor(atarget_offset.x),
        y: ft_pix_floor(atarget_offset.y),
    };

    /* get source bitmap dimensions */
    let mut source_llx = source_offset.x;
    if FtPos::MIN + ((source_.rows as FtPos) << 6) + 64 > source_offset.y {
        return Err(FT_ERR_INVALID_ARGUMENT);
    }
    let mut source_lly = source_offset.y - ((source_.rows as FtPos) << 6);
    if FtPos::MAX - ((source_.width as FtPos) << 6) - 64 < source_llx {
        return Err(FT_ERR_INVALID_ARGUMENT);
    }
    let source_urx = source_llx + ((source_.width as FtPos) << 6);
    let source_ury = source_offset.y;

    /* get target bitmap dimensions */
    let (mut target_llx, mut target_lly, target_urx, target_ury);
    if target.width != 0 && target.rows != 0 {
        target_llx = target_offset.x;
        if FtPos::MIN + ((target.rows as FtPos) << 6) > target_offset.y {
            return Err(FT_ERR_INVALID_ARGUMENT);
        }
        target_lly = target_offset.y - ((target.rows as FtPos) << 6);
        if FtPos::MAX - ((target.width as FtPos) << 6) < target_llx {
            return Err(FT_ERR_INVALID_ARGUMENT);
        }
        target_urx = target_llx + ((target.width as FtPos) << 6);
        target_ury = target_offset.y;
    } else {
        target_llx = FtPos::MAX;
        target_lly = FtPos::MAX;
        target_urx = FtPos::MIN;
        target_ury = FtPos::MIN;
    }

    /* compute final bitmap dimensions */
    let final_llx = source_llx.min(target_llx);
    let final_lly = source_lly.min(target_lly);
    let final_urx = source_urx.max(target_urx);
    let final_ury = source_ury.max(target_ury);

    let final_width = (final_urx.wrapping_sub(final_llx) >> 6) as u32;
    let final_rows = (final_ury.wrapping_sub(final_lly) >> 6) as u32;

    if !(final_width != 0 && final_rows != 0) {
        return Ok(()); /* nothing to do */
    }

    /* for blending, set offset vector of final bitmap */
    /* temporarily to (0,0)                            */
    source_llx -= final_llx;
    source_lly -= final_lly;

    if target.width != 0 && target.rows != 0 {
        target_llx -= final_llx;
        target_lly -= final_lly;
    }

    let r = (|| -> FtResult<()> {
        /* set up target bitmap */
        if target.pixel_mode == FT_PIXEL_MODE_NONE {
            /* create new empty bitmap */
            target.width = final_width;
            target.rows = final_rows;
            target.pixel_mode = FT_PIXEL_MODE_BGRA;
            target.pitch = (final_width as i32).wrapping_mul(4);
            target.num_grays = 256;

            if FtLong::MAX / (target.pitch as FtLong) < target.rows as i32 as FtLong {
                return Err(FT_ERR_INVALID_ARGUMENT);
            }

            target.buffer = ft_alloc((target.pitch as FtLong) * target.rows as i32 as FtLong)?;

            free_target_bitmap_on_error = true;
        } else if target.width != final_width || target.rows != final_rows {
            /* adjust old bitmap to enlarged size */
            let mut pitch = target.pitch;

            if pitch < 0 {
                pitch = -pitch;
            }

            let new_pitch = (final_width as i32).wrapping_mul(4);

            if FtLong::MAX / (new_pitch as FtLong) < final_rows as i32 as FtLong {
                return Err(FT_ERR_INVALID_ARGUMENT);
            }

            /* TODO: provide an in-buffer solution for large bitmaps */
            /*       to avoid allocation of a new buffer             */
            let mut buffer = ft_alloc((new_pitch as FtLong) * final_rows as i32 as FtLong)?;

            /* copy data to new buffer */
            let x = target_llx >> 6;
            let y = target_lly >> 6;

            /* the bitmap flow is from top to bottom, */
            /* but y is measured from bottom to top   */
            if target.pitch < 0 {
                /* XXX */
            } else {
                let mut p = 0usize;
                let mut q = ((final_rows as FtLong - y - target.rows as FtLong)
                    * new_pitch as FtLong
                    + x * 4) as usize;
                let limit_p = pitch as usize * target.rows as usize;

                while p < limit_p {
                    buffer[q..q + pitch as usize]
                        .copy_from_slice(&target.buffer[p..p + pitch as usize]);

                    p += pitch as usize;
                    q += new_pitch as usize;
                }
            }

            target.width = final_width;
            target.rows = final_rows;

            if target.pitch < 0 {
                target.pitch = -new_pitch;
            } else {
                target.pitch = new_pitch;
            }

            target.buffer = buffer;
        }

        /* adjust source bitmap if necessary */
        let source: &FtBitmap = if source_.pixel_mode != FT_PIXEL_MODE_GRAY {
            ft_bitmap_init(&mut source_bitmap);
            ft_bitmap_convert(source_, &mut source_bitmap, 1)?;
            &source_bitmap
        } else {
            source_
        };

        /* do blending; the code below returns pre-multiplied channels, */
        /* similar to what FreeType gets from `CBDT' tables             */
        let x = source_llx >> 6;
        let y = source_lly >> 6;

        /* the bitmap flow is from top to bottom, */
        /* but y is measured from bottom to top   */
        if target.pitch < 0 {
            /* XXX */
        } else {
            let mut p = 0usize;
            let mut q = ((target.rows as FtLong - y - source.rows as FtLong)
                * target.pitch as FtLong
                + x * 4) as usize;
            let limit_p = source.pitch as usize * source.rows as usize;

            while p < limit_p {
                let mut s = q;

                for &aa in &source.buffer[p..p + source.width as usize] {
                    let aa = aa as i32;
                    let fa = color.alpha as i32 * aa / 255;

                    let fb = color.blue as i32 * fa / 255;
                    let fg = color.green as i32 * fa / 255;
                    let fr = color.red as i32 * fa / 255;

                    let ba2 = 255 - fa;

                    let t = &mut target.buffer[s..s + 4];
                    let bb = t[0] as i32;
                    let bg = t[1] as i32;
                    let br = t[2] as i32;
                    let ba = t[3] as i32;

                    t[0] = (bb * ba2 / 255 + fb) as u8;
                    t[1] = (bg * ba2 / 255 + fg) as u8;
                    t[2] = (br * ba2 / 255 + fr) as u8;
                    t[3] = (ba * ba2 / 255 + fa) as u8;
                    s += 4;
                }

                p += source.pitch as usize;
                q += target.pitch as usize;
            }
        }

        atarget_offset.x = final_llx;
        atarget_offset.y = final_lly + ((final_rows as FtPos) << 6);

        Ok(())
    })();

    /* Error: */
    if r.is_err() && free_target_bitmap_on_error {
        ft_bitmap_done(target);
    }

    r
}

/// `FT_GlyphSlot_Own_Bitmap`
pub fn ft_glyphslot_own_bitmap(slot: &mut FtGlyphSlotRec) -> FtResult<()> {
    if slot.format == FT_GLYPH_FORMAT_BITMAP && slot.internal.flags & FT_GLYPH_OWN_BITMAP == 0 {
        let mut bitmap = FtBitmap::default();

        ft_bitmap_init(&mut bitmap);
        ft_bitmap_copy(&slot.bitmap, &mut bitmap)?;

        slot.bitmap = bitmap;
        slot.internal.flags |= FT_GLYPH_OWN_BITMAP;
    }

    Ok(())
}

/// `FT_Bitmap_Done`
pub fn ft_bitmap_done(bitmap: &mut FtBitmap) {
    *bitmap = FtBitmap::default();
}
