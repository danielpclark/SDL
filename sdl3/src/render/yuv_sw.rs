// Rust translation of src/render/SDL_yuv_sw.c from Simple DirectMedia Layer.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! The software implementation of the YUV texture support: the planes of a
//! YUV texture, updated in place and converted to the renderer's format.

use crate::error::{Error, Result};
use crate::video::pixels::{Colorspace, PixelFormat};
use crate::video::rect::Rect;
use crate::video::surface::{
    calculate_yuv_size, convert_pixels_and_colorspace, ScaleMode, Surface,
};

/// Translation of `SDL_SW_YUVTexture`.
#[derive(Debug)]
pub(crate) struct SwYuvTexture {
    pub(crate) format: PixelFormat,
    colorspace: Colorspace,
    target_format: PixelFormat,
    pub(crate) w: i32,
    pub(crate) h: i32,
    pub(crate) pixels: Vec<u8>,
    // These are just so we don't have to allocate them separately
    pub(crate) pitches: [usize; 3],
    /// byte offsets of the planes in `pixels`
    planes: [usize; 3],
    // This is a temporary surface in case we have to stretch copy
    stretch: Option<Surface<'static>>,
}

/// Copy `rows` rows of `length` bytes; the source must hold them all
/// (upstream reads through the caller's pointer), and so must the
/// destination.
#[allow(clippy::too_many_arguments)]
fn copy_rows(
    src: &[u8],
    mut s: usize,
    src_pitch: usize,
    dst: &mut [u8],
    mut d: usize,
    dst_pitch: usize,
    rows: usize,
    length: usize,
) -> Result<()> {
    if rows > 0 && src.len() < s + (rows - 1) * src_pitch + length {
        return Err(Error::invalid_param("pixels"));
    }
    if rows > 0 && dst.len() < d + (rows - 1) * dst_pitch + length {
        return Err(Error::new("YUV update outside the texture"));
    }
    for _ in 0..rows {
        dst[d..d + length].copy_from_slice(&src[s..s + length]);
        s += src_pitch;
        d += dst_pitch;
    }
    Ok(())
}

impl SwYuvTexture {
    /// Translation of `SDL_SW_CreateYUVTexture()`.
    pub(crate) fn new(
        format: PixelFormat,
        colorspace: Colorspace,
        w: i32,
        h: i32,
    ) -> Result<SwYuvTexture> {
        match format {
            PixelFormat::YV12
            | PixelFormat::IYUV
            | PixelFormat::I444
            | PixelFormat::I0FL
            | PixelFormat::I4FL
            | PixelFormat::YUY2
            | PixelFormat::UYVY
            | PixelFormat::YVYU
            | PixelFormat::NV12
            | PixelFormat::NV21 => {}
            _ => return Err(Error::new("Unsupported YUV format")),
        }

        let (dst_size, _) = calculate_yuv_size(format, w, h)?;
        let pixels = vec![0u8; dst_size];

        // Find the pitch and offset values for the texture
        let bpp = format.bytes_per_pixel() as usize;
        let (w_, h_) = (w as usize, h as usize);
        let mut pitches = [0usize; 3];
        let mut planes = [0usize; 3];
        match format {
            PixelFormat::YV12 | PixelFormat::IYUV | PixelFormat::I0FL => {
                pitches[0] = w_ * bpp;
                pitches[1] = (pitches[0] / bpp).div_ceil(2) * bpp;
                pitches[2] = pitches[1];
                planes[1] = planes[0] + pitches[0] * h_;
                planes[2] = planes[1] + pitches[1] * h_.div_ceil(2);
            }
            PixelFormat::I444 | PixelFormat::I4FL => {
                pitches[0] = w_ * bpp;
                pitches[1] = pitches[0];
                pitches[2] = pitches[1];
                planes[1] = planes[0] + pitches[0] * h_;
                planes[2] = planes[1] + pitches[1] * h_;
            }
            PixelFormat::YUY2 | PixelFormat::UYVY | PixelFormat::YVYU => {
                pitches[0] = w_.div_ceil(2) * 4;
            }
            _ => {
                // NV12, NV21
                pitches[0] = w_;
                pitches[1] = 2 * pitches[0].div_ceil(2);
                planes[1] = planes[0] + pitches[0] * h_;
            }
        }

        // We're all done..
        Ok(SwYuvTexture {
            format,
            colorspace,
            target_format: PixelFormat::UNKNOWN,
            w,
            h,
            pixels,
            pitches,
            planes,
            stretch: None,
        })
    }

    /// Translation of `SDL_SW_UpdateYUVTexture()`.
    pub(crate) fn update(&mut self, rect: &Rect, pixels: &[u8], pitch: usize) -> Result<()> {
        let bpp = self.format.bytes_per_pixel() as usize;
        let (w, h) = (self.w as usize, self.h as usize);
        let (rx, ry, rw, rh) = (
            rect.x as usize,
            rect.y as usize,
            rect.w as usize,
            rect.h as usize,
        );
        let p = &mut self.pixels;
        match self.format {
            PixelFormat::YV12 | PixelFormat::IYUV | PixelFormat::I0FL => {
                if rx == 0 && ry == 0 && rw == w && rh == h && pitch == self.pitches[0] {
                    let n = h * w * bpp + 2 * h.div_ceil(2) * w.div_ceil(2) * bpp;
                    p[..n].copy_from_slice(
                        pixels
                            .get(..n)
                            .ok_or_else(|| Error::invalid_param("pixels"))?,
                    );
                } else {
                    let uv_pitch = (pitch / bpp).div_ceil(2) * bpp;

                    // Copy the Y plane
                    copy_rows(
                        pixels,
                        0,
                        pitch,
                        p,
                        ry * w * bpp + rx * bpp,
                        self.pitches[0],
                        rh,
                        rw * bpp,
                    )?;

                    // Copy the next plane
                    let s = rh * pitch;
                    let d = h * self.pitches[0] + (ry / 2) * self.pitches[1] + (rx / 2) * bpp;
                    copy_rows(
                        pixels,
                        s,
                        uv_pitch,
                        p,
                        d,
                        self.pitches[1],
                        rh.div_ceil(2),
                        rw.div_ceil(2) * bpp,
                    )?;

                    // Copy the next plane
                    // (upstream finds the plane with the chroma height of the
                    // rect instead of the texture's, writing the rows into the
                    // wrong place; fixed here)
                    let s = rh * pitch + rh.div_ceil(2) * uv_pitch;
                    let d = h * self.pitches[0]
                        + h.div_ceil(2) * self.pitches[1]
                        + (ry / 2) * self.pitches[2]
                        + (rx / 2) * bpp;
                    copy_rows(
                        pixels,
                        s,
                        uv_pitch,
                        p,
                        d,
                        self.pitches[2],
                        rh.div_ceil(2),
                        rw.div_ceil(2) * bpp,
                    )?;
                }
            }
            PixelFormat::I444 | PixelFormat::I4FL => {
                if rx == 0 && ry == 0 && rw == w && rh == h && pitch == self.pitches[0] {
                    let n = h * pitch * 3;
                    p[..n].copy_from_slice(
                        pixels
                            .get(..n)
                            .ok_or_else(|| Error::invalid_param("pixels"))?,
                    );
                } else {
                    let length = rw * bpp;

                    // Copy the Y plane
                    let d = ry * self.pitches[0] + rx * bpp;
                    copy_rows(pixels, 0, pitch, p, d, self.pitches[0], rh, length)?;

                    // Copy the next plane
                    let d = h * self.pitches[0] + ry * self.pitches[1] + rx * bpp;
                    copy_rows(pixels, rh * pitch, pitch, p, d, self.pitches[1], rh, length)?;

                    // Copy the next plane
                    let d =
                        h * self.pitches[0] + h * self.pitches[1] + ry * self.pitches[2] + rx * bpp;
                    copy_rows(
                        pixels,
                        2 * rh * pitch,
                        pitch,
                        p,
                        d,
                        self.pitches[2],
                        rh,
                        length,
                    )?;
                }
            }
            PixelFormat::YUY2 | PixelFormat::UYVY | PixelFormat::YVYU => {
                let d = self.planes[0] + ry * self.pitches[0] + rx * 2;
                copy_rows(
                    pixels,
                    0,
                    pitch,
                    p,
                    d,
                    self.pitches[0],
                    rh,
                    4 * rw.div_ceil(2),
                )?;
            }
            PixelFormat::NV12 | PixelFormat::NV21 => {
                if rx == 0 && ry == 0 && rw == w && rh == h {
                    let n = h * w + 2 * h.div_ceil(2) * w.div_ceil(2);
                    p[..n].copy_from_slice(
                        pixels
                            .get(..n)
                            .ok_or_else(|| Error::invalid_param("pixels"))?,
                    );
                } else {
                    // Copy the Y plane
                    copy_rows(pixels, 0, pitch, p, ry * w + rx, w, rh, rw)?;

                    // Copy the next plane
                    // (upstream rounds the chroma row up, (y + 1) / 2, which
                    // puts the rows of an update at an odd row one row down
                    // and can write past the end of the planes; fixed here
                    // to y / 2, as the column and the other formats)
                    let d = h * w + 2 * (ry / 2) * w.div_ceil(2) + 2 * (rx / 2);
                    copy_rows(
                        pixels,
                        rh * pitch,
                        2 * pitch.div_ceil(2),
                        p,
                        d,
                        2 * w.div_ceil(2),
                        rh.div_ceil(2),
                        2 * rw.div_ceil(2),
                    )?;
                }
            }
            _ => return Err(Error::new("Unsupported YUV format")),
        }
        Ok(())
    }

    /// Translation of `SDL_SW_UpdateYUVTexturePlanar()`.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn update_planar(
        &mut self,
        rect: &Rect,
        y_plane: &[u8],
        y_pitch: usize,
        u_plane: &[u8],
        u_pitch: usize,
        v_plane: &[u8],
        v_pitch: usize,
    ) -> Result<()> {
        let bpp = self.format.bytes_per_pixel() as usize;
        let (w, h) = (self.w as usize, self.h as usize);
        let (rx, ry, rw, rh) = (
            rect.x as usize,
            rect.y as usize,
            rect.w as usize,
            rect.h as usize,
        );
        let full = matches!(self.format, PixelFormat::I444 | PixelFormat::I4FL);
        let p = &mut self.pixels;

        // Copy the Y plane
        let d = ry * self.pitches[0] + rx * bpp;
        copy_rows(y_plane, 0, y_pitch, p, d, self.pitches[0], rh, rw * bpp)?;

        // Copy the U plane
        if full {
            let d = h * self.pitches[0] + ry * self.pitches[1] + rx * bpp;
            copy_rows(u_plane, 0, u_pitch, p, d, self.pitches[1], rh, rw * bpp)?;
        } else {
            let mut d = if self.format == PixelFormat::IYUV || self.format == PixelFormat::I0FL {
                h * self.pitches[0]
            } else {
                h * self.pitches[0] + h.div_ceil(2) * self.pitches[1]
            };
            d += ry / 2 * w.div_ceil(2) * bpp + (rx / 2) * bpp;
            copy_rows(
                u_plane,
                0,
                u_pitch,
                p,
                d,
                self.pitches[1],
                rh.div_ceil(2),
                rw.div_ceil(2) * bpp,
            )?;
        }

        // Copy the V plane
        if full {
            let d = h * self.pitches[0] + h * self.pitches[1] + ry * self.pitches[2] + rx * bpp;
            copy_rows(v_plane, 0, v_pitch, p, d, self.pitches[2], rh, rw * bpp)?;
        } else {
            let mut d = if self.format == PixelFormat::YV12 {
                h * self.pitches[0]
            } else {
                h * self.pitches[0] + h.div_ceil(2) * self.pitches[1]
            };
            d += ry / 2 * w.div_ceil(2) * bpp + (rx / 2) * bpp;
            copy_rows(
                v_plane,
                0,
                v_pitch,
                p,
                d,
                self.pitches[2],
                rh.div_ceil(2),
                rw.div_ceil(2) * bpp,
            )?;
        }
        Ok(())
    }

    /// Translation of `SDL_SW_UpdateNVTexturePlanar()`.
    pub(crate) fn update_nv_planar(
        &mut self,
        rect: &Rect,
        y_plane: &[u8],
        y_pitch: usize,
        uv_plane: &[u8],
        uv_pitch: usize,
    ) -> Result<()> {
        let (w, h) = (self.w as usize, self.h as usize);
        let (rx, ry, rw, rh) = (
            rect.x as usize,
            rect.y as usize,
            rect.w as usize,
            rect.h as usize,
        );
        let p = &mut self.pixels;

        // Copy the Y plane
        copy_rows(y_plane, 0, y_pitch, p, ry * w + rx, w, rh, rw)?;

        // Copy the UV or VU plane
        // (upstream offsets the plane by y * ((w + 1) / 2) + x, which is only
        // the chroma row and column for an even x and y: an odd x swaps U
        // and V and an odd y starts mid row; fixed here)
        let d = h * w + 2 * (ry / 2) * w.div_ceil(2) + 2 * (rx / 2);
        copy_rows(
            uv_plane,
            0,
            uv_pitch,
            p,
            d,
            2 * w.div_ceil(2),
            rh.div_ceil(2),
            2 * rw.div_ceil(2),
        )?;
        Ok(())
    }

    /// The byte offset and pitch of a lock of `rect`.
    /// Translation of `SDL_SW_LockYUVTexture()`.
    pub(crate) fn lock(&self, rect: Option<&Rect>) -> Result<(usize, usize)> {
        if matches!(
            self.format,
            PixelFormat::YV12
                | PixelFormat::IYUV
                | PixelFormat::I444
                | PixelFormat::I4FL
                | PixelFormat::NV12
                | PixelFormat::NV21
        ) {
            if let Some(r) = rect {
                if r.x != 0 || r.y != 0 || r.w != self.w || r.h != self.h {
                    return Err(Error::new(
                        "YV12, IYUV, I444, I4FL, NV12, NV21 textures only support full surface locks",
                    ));
                }
            }
        }

        let offset = match rect {
            Some(r) => {
                self.planes[0]
                    + r.y as usize * self.pitches[0]
                    + r.x as usize * self.format.bytes_per_pixel() as usize
            }
            None => self.planes[0],
        };
        Ok((offset, self.pitches[0]))
    }

    /// Convert `srcrect` of the texture to `w` x `h` pixels of
    /// `target_format`. Translation of `SDL_SW_CopyYUVToRGB()`.
    pub(crate) fn copy_to_rgb(
        &mut self,
        srcrect: &Rect,
        target_format: PixelFormat,
        w: i32,
        h: i32,
        pixels: &mut [u8],
        pitch: i32,
    ) -> Result<()> {
        // Make sure we're set up to display in the desired format
        if target_format != self.target_format {
            self.target_format = target_format;
        }

        let mut stretch = false;
        if srcrect.x != 0 || srcrect.y != 0 || srcrect.w < self.w || srcrect.h < self.h {
            /* The source rectangle has been clipped.
              Using a scratch surface is easier than adding clipped
              source support to all the blitters, plus that would
              slow them down in the general unclipped case.
            */
            stretch = true;
        } else if srcrect.w != w || srcrect.h != h {
            stretch = true;
        }

        if !stretch {
            return convert_pixels_and_colorspace(
                self.w,
                self.h,
                self.format,
                self.colorspace,
                None,
                &self.pixels,
                self.pitches[0] as i32,
                target_format,
                Colorspace::SRGB,
                None,
                pixels,
                pitch,
            );
        }

        // (upstream keeps a scratch surface of the first target format it
        // sees; it is recreated here when the format changes)
        if self
            .stretch
            .as_ref()
            .is_none_or(|s| s.format() != target_format)
        {
            self.stretch = Some(Surface::new(self.w, self.h, target_format)?);
        }
        let stretch_surface = self.stretch.as_mut().unwrap();
        let stretch_pitch = stretch_surface.pitch();
        convert_pixels_and_colorspace(
            self.w,
            self.h,
            self.format,
            self.colorspace,
            None,
            &self.pixels,
            self.pitches[0] as i32,
            target_format,
            Colorspace::SRGB,
            None,
            stretch_surface.pixels_mut().unwrap_or(&mut []),
            stretch_pitch,
        )?;

        let mut display = Surface::from_pixels(w, h, target_format, pixels, pitch)?;
        stretch_surface.stretch(Some(srcrect), &mut display, None, ScaleMode::Nearest)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn texture(format: PixelFormat) -> SwYuvTexture {
        let mut t = SwYuvTexture::new(format, Colorspace::BT601_LIMITED, 4, 4).unwrap();
        t.pixels.fill(0);
        t
    }

    // A partial update of a planar texture: the last plane comes after the
    // texture's middle plane, not the rect's.
    #[test]
    fn planar_update_last_plane() {
        let mut t = texture(PixelFormat::IYUV);
        // 2x2 rect: two Y rows of 2, then one U row and one V row of 1
        let pixels = [1, 1, 1, 1, 2, 3];
        t.update(&Rect::new(0, 0, 2, 2), &pixels, 2).unwrap();
        let (y, u, v) = (&t.pixels[..16], &t.pixels[16..20], &t.pixels[20..24]);
        assert_eq!(y, [1, 1, 0, 0, 1, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
        assert_eq!(u, [2, 0, 0, 0]);
        assert_eq!(v, [3, 0, 0, 0]);
    }

    // The chroma row of an NV update at an odd row is row / 2.
    #[test]
    fn nv_update_odd_row() {
        let mut t = texture(PixelFormat::NV12);
        // rows 1..4: three Y rows, then two UV rows
        let mut pixels = vec![1; 12];
        pixels.extend([2, 3, 4, 5, 6, 7, 8, 9]);
        t.update(&Rect::new(0, 1, 4, 3), &pixels, 4).unwrap();
        assert_eq!(t.pixels[16..24], [2, 3, 4, 5, 6, 7, 8, 9]);

        let mut t = texture(PixelFormat::NV12);
        t.update(&Rect::new(0, 1, 4, 1), &pixels, 4).unwrap();
        assert_eq!(t.pixels[16..24], [1, 1, 1, 1, 0, 0, 0, 0]);
    }

    // The UV plane of an NV planar update starts at the chroma row and
    // column: an odd column must not swap U and V.
    #[test]
    fn nv_planar_update_offset() {
        let y = [1; 4];
        let uv = [2, 3];
        for (rect, at) in [
            (Rect::new(1, 0, 2, 2), 16),
            (Rect::new(0, 1, 2, 2), 16),
            (Rect::new(2, 2, 2, 2), 22),
            (Rect::new(3, 3, 1, 1), 22),
        ] {
            let mut t = texture(PixelFormat::NV12);
            t.update_nv_planar(&rect, &y, 2, &uv, 2).unwrap();
            let mut expected = [0u8; 8];
            expected[at - 16..at - 14].copy_from_slice(&uv);
            assert_eq!(t.pixels[16..24], expected, "{rect:?}");
        }
    }
}
