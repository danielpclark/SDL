// Rust translation of src/video/SDL_fillrect.c from Simple DirectMedia Layer.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! The 1- and 2-byte fills widen the color to 32 bits (`color |= color << 8`
//! ...) without masking it first, so a color with bits above the pixel size
//! is stored differently by the 4-byte stores (the widened pattern) and the
//! single-pixel stores (its low bytes). Which pixels get which depends on
//! the loop (plain or SSE, chosen by `SDL_HasSSE()`) and on the address
//! alignment of each row, exactly as upstream; the loops below reproduce
//! both, using the real addresses of the pixel memory. For in-range colors
//! all of them store the same bytes.

use super::*;

/// Store the low `bytes` bytes of `v` (native order) at `i`.
fn put(pixels: &mut [u8], i: usize, v: u32, bytes: usize) {
    store_low_bytes(&mut pixels[i..i + bytes], v);
}

/// `SDL_memset4(p, color, dwords)`.
fn memset4(pixels: &mut [u8], i: usize, color: u32, dwords: usize) {
    for k in 0..dwords {
        crate::video::blit::wr32(pixels, i + 4 * k, color);
    }
}

/// `SDL_memset(p, color, n)`: the low byte of `color`.
fn memset(pixels: &mut [u8], i: usize, color: u32, n: usize) {
    pixels[i..i + n].fill(color as u8);
}

/// The `SSE_WORK` loop: `n / 64` blocks of 64 bytes of the 32-bit pattern.
fn sse_work(pixels: &mut [u8], i: &mut usize, color: u32, n: usize) {
    for _ in 0..n / 64 {
        memset4(pixels, *i, color, 16);
        *i += 64;
    }
}

/// Translation of `SDL_FillSurfaceRect1SSE()`.
fn fill_rect1_sse(
    pixels: &mut [u8],
    base: usize,
    mut start: usize,
    pitch: usize,
    color: u32,
    w: usize,
    h: usize,
) {
    for _ in 0..h {
        let mut p = start;
        let mut n = w;

        if n > 63 {
            let adjust = 16 - ((base + p) & 15);
            if adjust != 0 {
                n -= adjust;
                memset(pixels, p, color, adjust);
                p += adjust;
            }
            sse_work(pixels, &mut p, color, n);
        }
        if n & 63 != 0 {
            let remainder = n & 63;
            memset(pixels, p, color, remainder);
        }
        start += pitch;
    }
}

/// Translation of the `DEFINE_SSE_FILLRECT(bpp, type)` functions
/// (`SDL_FillSurfaceRect2SSE()`, `SDL_FillSurfaceRect4SSE()`).
#[allow(clippy::too_many_arguments)]
fn fill_rect_n_sse(
    pixels: &mut [u8],
    base: usize,
    mut start: usize,
    pitch: usize,
    color: u32,
    mut w: usize,
    mut h: usize,
    bpp: usize,
) {
    // If the number of bytes per row is equal to the pitch, treat
    // all rows as one long continuous row (for better performance)
    if w * bpp == pitch {
        w *= h;
        h = 1;
    }

    for _ in 0..h {
        let mut n = w * bpp;
        let mut p = start;

        if n > 63 {
            let mut adjust = 16 - ((base + p) & 15);
            if adjust < 16 {
                n -= adjust;
                adjust /= bpp;
                for _ in 0..adjust {
                    put(pixels, p, color, bpp);
                    p += bpp;
                }
            }
            sse_work(pixels, &mut p, color, n);
        }
        if n & 63 != 0 {
            let remainder = (n & 63) / bpp;
            for _ in 0..remainder {
                put(pixels, p, color, bpp);
                p += bpp;
            }
        }
        start += pitch;
    }
}

/// Translation of `SDL_FillSurfaceRect1()`.
fn fill_rect1(
    pixels: &mut [u8],
    base: usize,
    mut start: usize,
    pitch: usize,
    color: u32,
    w: usize,
    h: usize,
) {
    for _ in 0..h {
        let mut n = w;
        let mut p = start;

        if n > 3 {
            let lead = match (base + p) & 3 {
                1 => 3,
                2 => 2,
                3 => 1,
                _ => 0,
            };
            memset(pixels, p, color, lead);
            p += lead;
            n -= lead;
            memset4(pixels, p, color, n >> 2);
        }
        if n & 3 != 0 {
            p += n & !3;
            memset(pixels, p, color, n & 3);
        }
        start += pitch;
    }
}

/// Translation of `SDL_FillSurfaceRect2()`.
fn fill_rect2(
    pixels: &mut [u8],
    base: usize,
    mut start: usize,
    pitch: usize,
    color: u32,
    w: usize,
    h: usize,
) {
    for _ in 0..h {
        let mut n = w;
        let mut p = start;

        if n > 1 {
            if (base + p) & 2 != 0 {
                put(pixels, p, color, 2);
                p += 2;
                n -= 1;
            }
            memset4(pixels, p, color, n >> 1);
        }
        if n & 1 != 0 {
            put(pixels, p + 2 * (n - 1), color, 2);
        }
        start += pitch;
    }
}

/// Translation of `SDL_FillSurfaceRect3()`.
fn fill_rect3(pixels: &mut [u8], mut start: usize, pitch: usize, color: u32, w: usize, h: usize) {
    let (b1, b2, b3) = if cfg!(target_endian = "little") {
        (color as u8, (color >> 8) as u8, (color >> 16) as u8)
    } else {
        ((color >> 16) as u8, (color >> 8) as u8, color as u8)
    };
    for _ in 0..h {
        for k in 0..w {
            let p = start + 3 * k;
            pixels[p] = b1;
            pixels[p + 1] = b2;
            pixels[p + 2] = b3;
        }
        start += pitch;
    }
}

/// Translation of `SDL_FillSurfaceRect4()`.
fn fill_rect4(pixels: &mut [u8], mut start: usize, pitch: usize, color: u32, w: usize, h: usize) {
    for _ in 0..h {
        memset4(pixels, start, color, w);
        start += pitch;
    }
}

impl Surface<'_> {
    /// Fill `rect` (`None` = the clip rectangle) with a pixel value, clipped
    /// to the clip rectangle. Translation of `SDL_FillSurfaceRect()`.
    pub fn fill_rect(&mut self, rect: Option<&Rect>, color: u32) -> Result<()> {
        // If 'rect' == NULL, then fill the whole surface
        let rect = match rect {
            Some(r) => *r,
            None => {
                let r = self.clip_rect;
                // Don't attempt to fill if the surface's clip_rect is empty
                if r.is_empty() {
                    return Ok(());
                }
                r
            }
        };

        self.fill_rects(&[rect], color)
    }

    /// Fill several rectangles with a pixel value, each clipped to the clip
    /// rectangle. Translation of `SDL_FillSurfaceRects()`.
    ///
    /// Like upstream, this writes the pixels of an RLE-accelerated surface
    /// with preallocated memory without decoding it first, so its RLE data
    /// goes stale (FIXME (upstream)).
    pub fn fill_rects(&mut self, rects: &[Rect], mut color: u32) -> Result<()> {
        if !self.pixels.is_some() && self.must_lock() {
            return Err(Error::new(
                "SDL_FillSurfaceRects(): You must lock the surface",
            ));
        }

        // Nothing to do
        if self.w == 0 || self.h == 0 || !self.pixels.is_some() {
            return Ok(());
        }

        /* This function doesn't usually work on surfaces < 8 bpp
         * Except: support for 4bits, when filling full size.
         */
        if self.format.bits_per_pixel() < 8 {
            if rects.len() == 1 {
                let r = &rects[0];
                if r.x == 0
                    && r.y == 0
                    && r.w == self.w
                    && r.h == self.h
                    && self.format.bits_per_pixel() == 4
                {
                    let b = ((color as u8) << 4) | color as u8;
                    let n = self.h as usize * self.pitch as usize;
                    if let Some(px) = self.pixels.bytes_mut() {
                        px[..n].fill(b);
                    }
                    return Ok(());
                }
            }
            return Err(Error::new(
                "SDL_FillSurfaceRects(): Unsupported surface format",
            ));
        }

        let bpp = self.format.bytes_per_pixel() as usize;
        let sse = crate::video::blit::simd_support().sse;
        match bpp {
            1 => {
                color |= color << 8;
                color |= color << 16;
            }
            2 => {
                color |= color << 16;
            }
            // 24-bit RGB is a slow path, at least for now.
            3 | 4 => {}
            _ => return Err(Error::new("Unsupported pixel format")),
        }

        let clip = self.clip_rect;
        let pitch = self.pitch as usize;
        let Some(pixels) = self.pixels.bytes_mut() else {
            return Ok(());
        };
        let base = pixels.as_ptr() as usize;
        for rect in rects {
            // Perform clipping
            let mut clipped = Rect::default();
            if !rect.intersect_into(&clip, &mut clipped) {
                continue;
            }

            let start = clipped.y as usize * pitch + clipped.x as usize * bpp;
            let (w, h) = (clipped.w as usize, clipped.h as usize);
            match (bpp, sse) {
                (1, true) => fill_rect1_sse(pixels, base, start, pitch, color, w, h),
                (1, false) => fill_rect1(pixels, base, start, pitch, color, w, h),
                (2, true) => fill_rect_n_sse(pixels, base, start, pitch, color, w, h, 2),
                (2, false) => fill_rect2(pixels, base, start, pitch, color, w, h),
                (3, _) => fill_rect3(pixels, start, pitch, color, w, h),
                (_, true) => fill_rect_n_sse(pixels, base, start, pitch, color, w, h, 4),
                (_, false) => fill_rect4(pixels, start, pitch, color, w, h),
            }
        }

        // We're done!
        Ok(())
    }
}
