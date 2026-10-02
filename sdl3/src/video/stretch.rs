// Rust translation of src/video/SDL_stretch.c from Simple DirectMedia Layer.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! Nearest and bilinear stretching between surfaces of the same format.
//!
//! Upstream has three bilinear kernels: portable C, SSE2 and NEON. The SSE2
//! kernel keeps the vertical pass at full precision before the horizontal
//! pass, so its results differ from the portable kernel's. Both are
//! translated (the SSE2 one in portable code) and chosen like upstream, by
//! whether the CPU has SSE2; NEON is not translated, so other architectures
//! get the portable kernel.

use crate::error::{Error, Result};
use crate::video::pixels::{Colorspace, PixelFormat};
use crate::video::rect::Rect;
use crate::video::surface::{convert_pixels_and_colorspace, ScaleMode, Surface};

impl Surface<'_> {
    /// Copy `srcrect` of this surface to `dstrect` of `dst`, which must have
    /// the same format, scaling as needed (no blending or colorkey).
    /// Translation of `SDL_StretchSurface()`.
    pub fn stretch(
        &mut self,
        srcrect: Option<&Rect>,
        dst: &mut Surface<'_>,
        dstrect: Option<&Rect>,
        scale_mode: ScaleMode,
    ) -> Result<()> {
        stretch_surface(self, srcrect, dst, dstrect, scale_mode)
    }
}

/// Translation of `SDL_StretchSurface()`.
pub(crate) fn stretch_surface(
    src: &mut Surface<'_>,
    srcrect: Option<&Rect>,
    dst: &mut Surface<'_>,
    dstrect: Option<&Rect>,
    mut scale_mode: ScaleMode,
) -> Result<()> {
    if src.format != dst.format {
        // Slow!
        let dst_palette = dst.palette.clone();
        let mut src_tmp = src.convert_with_colorspace(
            dst.format,
            dst_palette.as_ref(),
            dst.colorspace,
            dst.props.as_ref(),
        )?;
        return stretch_surface(&mut src_tmp, srcrect, dst, dstrect, scale_mode);
    }

    if src.format.is_fourcc() {
        // Slow!
        let full_dst = Rect::new(0, 0, dst.w, dst.h);
        let dstrect = *dstrect.unwrap_or(&full_dst);

        let mut src_tmp = src.convert(PixelFormat::XRGB8888)?;
        let mut dst_tmp = Surface::new_uninitialized(dstrect.w, dstrect.h, PixelFormat::XRGB8888)?;
        stretch_surface(&mut src_tmp, srcrect, &mut dst_tmp, None, scale_mode)?;
        let props = dst.properties();
        let (dst_format, dst_colorspace, dst_pitch) = (dst.format, dst.colorspace, dst.pitch);
        let offset = dstrect.y as isize * dst_pitch as isize
            + dstrect.x as isize * dst_format.bytes_per_pixel() as isize;
        let dst_pixels = dst
            .pixels
            .bytes_mut()
            .ok_or_else(|| Error::invalid_param("dst"))?;
        return convert_pixels_and_colorspace(
            dstrect.w,
            dstrect.h,
            dst_tmp.format,
            Colorspace::SRGB,
            None,
            dst_tmp.pixels.bytes().unwrap_or(&[]),
            dst_tmp.pitch,
            dst_format,
            dst_colorspace,
            Some(&props),
            &mut dst_pixels[offset as usize..],
            dst_pitch,
        );
    }

    if scale_mode == ScaleMode::PixelArt {
        scale_mode = ScaleMode::Nearest;
    }

    if scale_mode == ScaleMode::Linear
        && (src.format.bytes_per_pixel() != 4 || src.format == PixelFormat::ARGB2101010)
    {
        return Err(Error::new("Wrong format"));
    }

    // Verify the blit rectangles
    let srcrect = match srcrect {
        Some(r) => {
            if r.x < 0 || r.y < 0 || (r.x + r.w) > src.w || (r.y + r.h) > src.h {
                return Err(Error::new("Invalid source blit rectangle"));
            }
            *r
        }
        None => Rect::new(0, 0, src.w, src.h),
    };
    let dstrect = match dstrect {
        Some(r) => {
            if r.x < 0 || r.y < 0 || (r.x + r.w) > dst.w || (r.y + r.h) > dst.h {
                return Err(Error::new("Invalid destination blit rectangle"));
            }
            *r
        }
        None => Rect::new(0, 0, dst.w, dst.h),
    };

    if dstrect.w <= 0 || dstrect.h <= 0 {
        return Ok(());
    }

    if srcrect.w > u16::MAX as i32
        || srcrect.h > u16::MAX as i32
        || dstrect.w > u16::MAX as i32
        || dstrect.h > u16::MAX as i32
    {
        return Err(Error::new("Size too large for scaling"));
    }

    // Lock the destination if it's in hardware
    let mut dst_locked = false;
    if dst.must_lock() {
        if dst.lock_raw().is_err() {
            return Err(Error::new("Unable to lock destination surface"));
        }
        dst_locked = true;
    }
    // Lock the source if it's in hardware
    let mut src_locked = false;
    if src.must_lock() {
        if src.lock_raw().is_err() {
            if dst_locked {
                dst.unlock_raw();
            }
            return Err(Error::new("Unable to lock source surface"));
        }
        src_locked = true;
    }

    let result = if scale_mode == ScaleMode::Nearest {
        stretch_surface_unchecked_nearest(src, &srcrect, dst, &dstrect)
    } else {
        stretch_surface_unchecked_linear(src, &srcrect, dst, &dstrect)
    };

    // We need to unlock the surfaces if they're locked
    if dst_locked {
        dst.unlock_raw();
    }
    if src_locked {
        src.unlock_raw();
    }

    result
}

/* bilinear interpolation precision must be < 8
Because with SSE: add-multiply: _mm_madd_epi16 works with signed int
so pixels 0xb1...... are negatives and false the result
same in NEON probably */
const PRECISION: u32 = 7;

const fn fixed_point(i: i32) -> u32 {
    (i as u32) << 16
}
const fn src_index(fp: i64) -> u32 {
    (fp as u32) >> 16
}
const fn frac(fp: i64) -> u32 {
    ((fp >> (16 - PRECISION)) as u32) & ((1 << PRECISION) - 1)
}
const FRAC_ZERO: u32 = 0;
const FRAC_ONE: u32 = 1 << PRECISION;
const FP_ONE: u32 = fixed_point(1);

/// Translation of `get_scaler_datas()`: `(fp_start, fp_step, left_pad, right_pad)`.
fn get_scaler_datas(src_nb: i32, dst_nb: i32) -> (i64, i32, i32, i32) {
    let step = fixed_point(src_nb) as i32 / dst_nb; // source step in fixed point
    let mut x0 = (FP_ONE / 2) as i32; // dst first pixel center at 0.5 in fixed point

    // Use this code for perfect match with pixman
    let tmp0 = step as i64 * (x0 >> 16) as i64;
    let tmp1 = step as i64 * (x0 & 0xFFFF) as i64;
    x0 = (tmp0 + ((tmp1 + 0x8000) >> 16)) as i32; // x0 == (step + 1) / 2

    // -= 0.5, get back the pixel origin, in source coordinates
    x0 -= (FP_ONE / 2) as i32;

    let (mut left_pad, mut right_pad) = (0, 0);
    let mut fp_sum = x0 as i64;
    for _ in 0..dst_nb {
        if fp_sum < 0 {
            left_pad += 1;
        } else {
            let index = src_index(fp_sum) as i32;
            if index > src_nb - 2 {
                right_pad += 1;
            }
        }
        fp_sum += step as i64;
    }
    (x0 as i64, step, left_pad, right_pad)
}

/// A 32-bit pixel's four bytes, or zeros outside the buffer. Upstream's
/// SSE loads read the neighbouring pixel even when its weight is zero, which
/// can fall just outside the row (or the buffer) at the edges.
fn load_px(buf: &[u8], at: isize) -> [u8; 4] {
    if at < 0 || at as usize + 4 > buf.len() {
        return [0; 4];
    }
    let i = at as usize;
    [buf[i], buf[i + 1], buf[i + 2], buf[i + 3]]
}

/// Translation of `INTERPOL_BILINEAR_SSE()`, lane by lane.
fn interpol_bilinear_sse(
    src: &[u8],
    s0: isize,
    s1: isize,
    frac_w: u32,
    frac_h0: u32,
    frac_h1: u32,
) -> [u8; 4] {
    let f = frac_w as i32;
    let f2 = (FRAC_ONE - frac_w) as i32;

    let x00 = load_px(src, s0);
    let x01 = load_px(src, s0 + 4);
    let x10 = load_px(src, s1);
    let x11 = load_px(src, s1 + 4);

    /* Interpolated == x0 + frac * (x1 - x0) == x0 * (1 - frac) + x1 * frac */
    let mut out = [0u8; 4];
    for c in 0..4 {
        // Interpolation vertical (_mm_mullo_epi16 / _mm_add_epi16)
        let k_left =
            (x00[c] as i32 * frac_h1 as i32 + x10[c] as i32 * frac_h0 as i32) as i16 as i32;
        let k_right =
            (x01[c] as i32 * frac_h1 as i32 + x11[c] as i32 * frac_h0 as i32) as i16 as i32;
        // Interpolation horizontal (_mm_madd_epi16)
        let k = k_left
            .wrapping_mul(f2)
            .wrapping_add(k_right.wrapping_mul(f));
        // Store 1 pixel (_mm_srli_epi32, _mm_packs_epi32, _mm_packus_epi16)
        let d = ((k as u32) >> (PRECISION * 2)) as i32;
        let e = d.clamp(i16::MIN as i32, i16::MAX as i32);
        out[c] = e.clamp(0, 255) as u8;
    }
    out
}

/// Translation of `INTERPOL()`: `INTEGER(frac1 * c0 + frac0 * c1)` per byte.
fn interpol(c0: [u8; 4], c1: [u8; 4], frac0: u32, frac1: u32) -> [u8; 4] {
    let mut cx = [0u8; 4];
    for k in 0..4 {
        cx[k] = ((frac1 * c0[k] as u32 + frac0 * c1[k] as u32) >> PRECISION) as u8;
    }
    cx
}

/// Translation of `INTERPOL_BILINEAR()`.
fn interpol_bilinear(
    src: &[u8],
    s0: isize,
    s1: isize,
    frac_w0: u32,
    frac_h0: u32,
    frac_h1: u32,
) -> [u8; 4] {
    let frac_w1 = FRAC_ONE - frac_w0;

    // Vertical first, store to 'tmp'
    let tmp0 = interpol(load_px(src, s0), load_px(src, s1), frac_h0, frac_h1);
    let tmp1 = interpol(load_px(src, s0 + 4), load_px(src, s1 + 4), frac_h0, frac_h1);

    // Horizontal, store to 'dst'
    interpol(tmp0, tmp1, frac_w0, frac_w1)
}

/// Translation of `scale_mat()` and `scale_mat_SSE()`, which walk the
/// image identically and differ in the per-pixel kernel.
#[allow(clippy::too_many_arguments)]
fn scale_mat(
    src: &[u8],
    src_base: isize,
    src_w: i32,
    src_h: i32,
    src_pitch: i32,
    dst: &mut [u8],
    dst_base: usize,
    dst_w: i32,
    dst_h: i32,
    dst_pitch: i32,
    sse2: bool,
) -> Result<()> {
    let kernel = if sse2 {
        interpol_bilinear_sse
    } else {
        interpol_bilinear
    };

    // BILINEAR___START
    let (mut fp_sum_h, fp_step_h, left_pad_h, right_pad_h) = get_scaler_datas(src_h, dst_h);
    let (fp_sum_w0, fp_step_w, left_pad_w_init, right_pad_w_init) = get_scaler_datas(src_w, dst_w);
    let fp_sum_w_init = fp_sum_w0 + left_pad_w_init as i64 * fp_step_w as i64;
    let dst_gap = dst_pitch - 4 * dst_w;
    let middle_init = dst_w - left_pad_w_init - right_pad_w_init;

    let mut d = dst_base as isize;
    let put = |dst: &mut [u8], d: &mut isize, px: [u8; 4]| {
        let i = *d as usize;
        dst[i..i + 4].copy_from_slice(&px);
        *d += 4;
    };

    for i in 0..dst_h {
        // BILINEAR___HEIGHT
        let no_padding = !(i < left_pad_h || i > dst_h - 1 - right_pad_h);
        let mut index_h = src_index(fp_sum_h) as i32;
        let mut frac_h0 = frac(fp_sum_h);

        index_h = if no_padding {
            index_h
        } else if i < left_pad_h {
            0
        } else {
            src_h - 1
        };
        frac_h0 = if no_padding { frac_h0 } else { 0 };
        let incr_h1: isize = if no_padding { src_pitch as isize } else { 0 };
        let incr_h0: isize = index_h as isize * src_pitch as isize;

        let src_h0 = src_base + incr_h0;
        let src_h1 = src_h0 + incr_h1;

        fp_sum_h += fp_step_h as i64;

        let frac_h1 = FRAC_ONE - frac_h0;
        let mut fp_sum_w = fp_sum_w_init;
        let middle = middle_init;

        for _ in 0..left_pad_w_init {
            let px = kernel(src, src_h0, src_h1, FRAC_ZERO, frac_h0, frac_h1);
            put(dst, &mut d, px);
        }

        // The SSE kernel works in pairs plus a last point; each pixel is
        // computed independently, so this is the same as one at a time.
        // (Both kernels read the neighbouring pixel even at zero weight.)
        for _ in 0..middle.max(0) {
            let index_w = 4 * src_index(fp_sum_w) as isize;
            let frac_w = frac(fp_sum_w);
            fp_sum_w += fp_step_w as i64;
            let px = kernel(
                src,
                src_h0 + index_w,
                src_h1 + index_w,
                frac_w,
                frac_h0,
                frac_h1,
            );
            put(dst, &mut d, px);
        }

        for _ in 0..right_pad_w_init {
            let index_w = 4 * (src_w as isize - 2);
            let px = kernel(
                src,
                src_h0 + index_w,
                src_h1 + index_w,
                FRAC_ONE,
                frac_h0,
                frac_h1,
            );
            put(dst, &mut d, px);
        }
        d += dst_gap as isize;
    }
    Ok(())
}

/// Translation of `SDL_StretchSurfaceUncheckedLinear()`.
fn stretch_surface_unchecked_linear(
    s: &Surface<'_>,
    srcrect: &Rect,
    d: &mut Surface<'_>,
    dstrect: &Rect,
) -> Result<()> {
    let src_pitch = s.pitch;
    let dst_pitch = d.pitch;
    let src_base = srcrect.x as isize * 4 + srcrect.y as isize * src_pitch as isize;
    let dst_base = dstrect.x as usize * 4 + dstrect.y as usize * dst_pitch as usize;
    let src = s
        .pixels
        .bytes()
        .ok_or_else(|| Error::invalid_param("src"))?;
    let dst = d
        .pixels
        .bytes_mut()
        .ok_or_else(|| Error::invalid_param("dst"))?;
    let sse2 = crate::video::blit::simd_support().sse2;
    scale_mat(
        src, src_base, srcrect.w, srcrect.h, src_pitch, dst, dst_base, dstrect.w, dstrect.h,
        dst_pitch, sse2,
    )
}

/// Translation of `SDL_StretchSurfaceUncheckedNearest()` and the
/// `scale_mat_nearest_N()` functions.
fn stretch_surface_unchecked_nearest(
    s: &Surface<'_>,
    srcrect: &Rect,
    d: &mut Surface<'_>,
    dstrect: &Rect,
) -> Result<()> {
    let (src_w, src_h, dst_w, dst_h) = (srcrect.w, srcrect.h, dstrect.w, dstrect.h);
    let src_pitch = s.pitch as usize;
    let dst_pitch = d.pitch;
    let bpp = (d.format.bytes_per_pixel() as usize).clamp(1, 4);

    let src_ptr = srcrect.x as usize * bpp + srcrect.y as usize * src_pitch;
    let mut dst_i = dstrect.x as usize * bpp + dstrect.y as usize * dst_pitch as usize;
    let src = s
        .pixels
        .bytes()
        .ok_or_else(|| Error::invalid_param("src"))?;
    let dst = d
        .pixels
        .bytes_mut()
        .ok_or_else(|| Error::invalid_param("dst"))?;

    // SDL_SCALE_NEAREST__START
    let incy = ((src_h as u64) << 16) / dst_h as u64;
    let incx = ((src_w as u64) << 16) / dst_w as u64;
    let dst_gap = dst_pitch as isize - (bpp * dst_w as usize) as isize;
    let mut posy = incy / 2;

    for _ in 0..dst_h {
        // SDL_SCALE_NEAREST__HEIGHT
        let srcy = posy >> 16;
        let src_h0 = src_ptr + srcy as usize * src_pitch;
        posy += incy;
        let mut posx = incx / 2;
        for _ in 0..dst_w {
            let srcx = bpp * (posx >> 16) as usize;
            posx += incx;
            let si = src_h0 + srcx;
            dst[dst_i..dst_i + bpp].copy_from_slice(&src[si..si + bpp]);
            dst_i += bpp;
        }
        dst_i = (dst_i as isize + dst_gap) as usize;
    }
    Ok(())
}
