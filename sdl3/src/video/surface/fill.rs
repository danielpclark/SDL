// Rust translation of src/video/SDL_fillrect.c from Simple DirectMedia Layer.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! Upstream has 1-, 2-, 3- and 4-byte fill loops plus SSE and LSX versions
//! that align the start of each row before storing 16 bytes at a time. All
//! of them store the same bytes; the loops below are the plain versions.

use super::*;

/// The bytes of one pixel of `color` in memory order: the low `bpp` bytes
/// of the native-endian value (for 24-bit pixels, `color & 0xFF` first on
/// little endian, `(color >> 16) & 0xFF` first on big endian).
fn pixel_bytes(color: u32, bpp: usize) -> [u8; 4] {
    let mut out = [0u8; 4];
    store_low_bytes(&mut out[..bpp], color);
    out
}

/// Translation of `SDL_FillSurfaceRect1()` ... `SDL_FillSurfaceRect4()`.
fn fill(pixels: &mut [u8], start: usize, pitch: usize, color: u32, bpp: usize, w: usize, h: usize) {
    let px = pixel_bytes(color, bpp);
    for row in 0..h {
        let line = &mut pixels[start + row * pitch..start + row * pitch + w * bpp];
        for p in line.chunks_exact_mut(bpp) {
            p.copy_from_slice(&px[..bpp]);
        }
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
        for rect in rects {
            // Perform clipping
            let mut clipped = Rect::default();
            if !rect.intersect_into(&clip, &mut clipped) {
                continue;
            }

            let start = clipped.y as usize * pitch + clipped.x as usize * bpp;
            fill(
                pixels,
                start,
                pitch,
                color,
                bpp,
                clipped.w as usize,
                clipped.h as usize,
            );
        }

        // We're done!
        Ok(())
    }
}
