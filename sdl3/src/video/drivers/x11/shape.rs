// Rust translation of src/video/x11/SDL_x11shape.c and SDL_x11shape.h from
// Simple DirectMedia Layer.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! Window shapes (the input shape of transparent windows), with the X
//! Shape extension.

use std::ffi::{c_char, c_uint};

use super::sys::*;
use super::video::X11Video;
use super::window::with_x11_window;
use crate::error::{Error, Result};
use crate::events::WindowID;
use crate::video::surface::ScaleMode;
use crate::video::{PixelFormat, Surface};

/// `SDL_ALPHA_TRANSPARENT`.
const SDL_ALPHA_TRANSPARENT: u8 = 0;

/// Translation of `GenerateShapeMask()`: one bit per pixel, set where the
/// (ARGB32) shape isn't transparent.
fn generate_shape_mask(shape: &Surface<'_>) -> Vec<u8> {
    let ppb: usize = 8;
    let w = shape.width().max(0) as usize;
    let h = shape.height().max(0) as usize;
    let bytes_per_scanline = w.div_ceil(ppb);
    let pitch = shape.pitch().max(0) as usize;
    let pixels = shape.pixels().unwrap_or(&[]);

    let mut mask = vec![0u8; h * bytes_per_scanline];
    for y in 0..h {
        let mask_scanline = &mut mask[y * bytes_per_scanline..(y + 1) * bytes_per_scanline];
        for x in 0..w {
            // (the first byte of an ARGB32 pixel is its alpha)
            let a = pixels
                .get(y * pitch + x * 4)
                .copied()
                .unwrap_or(SDL_ALPHA_TRANSPARENT);
            let mask_value = if a == SDL_ALPHA_TRANSPARENT { 0 } else { 1 };
            mask_scanline[x / ppb] |= mask_value << (x % ppb);
        }
    }
    mask
}

impl X11Video {
    /// Translation of `X11_UpdateWindowShape()`.
    pub(crate) fn x11_update_window_shape(
        &self,
        window: WindowID,
        shape: Option<&Surface<'static>>,
    ) -> Result<()> {
        // FIXME (upstream): SDL_X11_HAVE_XSHAPE isn't checked before the
        // XShape functions are called (NULL when libXext lacks them).
        let Some(xshape) = &self.x.xshape else {
            return Err(Error::unsupported());
        };
        let display = self.display;
        let (xwindow, ww, wh) = with_x11_window(window, |w, d| (d.xwindow, w.core.w, w.core.h))?;

        // Generate a set of spans for the region
        if let Some(shape) = shape {
            let stretched;
            let shape: &Surface<'_> = if shape.width() != ww || shape.height() != wh {
                let mut s = Surface::new(ww, wh, PixelFormat::ARGB32)?;
                let mut src = shape.duplicate()?;
                src.stretch(Option::None, &mut s, Option::None, ScaleMode::Linear)?;
                stretched = s;
                &stretched
            } else {
                shape
            };

            let mask = generate_shape_mask(shape);
            // SAFETY: the display is open and the window ours; the mask has
            // h rows of (w + 7) / 8 bytes.
            unsafe {
                let pixmap = (self.x.XCreateBitmapFromData)(
                    display,
                    xwindow,
                    mask.as_ptr() as *const c_char,
                    shape.width() as c_uint,
                    shape.height() as c_uint,
                );
                (xshape.XShapeCombineMask)(display, xwindow, ShapeInput, 0, 0, pixmap, ShapeSet);
                // FIXME (upstream): the mask pixmap is never freed.
            }
        } else {
            // SAFETY: the display is open and the window ours; the region is
            // destroyed after use.
            unsafe {
                let region = (self.x.XCreateRegion)();
                let mut rect = XRectangle {
                    x: 0,
                    y: 0,
                    width: ww as u16,
                    height: wh as u16,
                };
                (self.x.XUnionRectWithRegion)(&mut rect, region, region);
                (xshape.XShapeCombineRegion)(display, xwindow, ShapeInput, 0, 0, region, ShapeSet);
                (self.x.XDestroyRegion)(region);
            }
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shape_mask() {
        let mut s = Surface::new(9, 2, PixelFormat::ARGB32).unwrap();
        {
            let p = s.pixels_mut().unwrap();
            p[0] = 0xff; // (0, 0) opaque
            p[8 * 4] = 0x80; // (8, 0)
        }
        let mask = generate_shape_mask(&s);
        assert_eq!(mask, vec![0x01, 0x01, 0x00, 0x00]);
    }
}
