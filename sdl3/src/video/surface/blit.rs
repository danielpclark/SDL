// Rust translation of the blit entry points of src/video/SDL_surface.c from
// Simple DirectMedia Layer.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

use super::*;
use crate::video::blit::{map::validate_map, soft_blit, MapBlit, COPY_NEAREST};
use crate::video::stretch::stretch_surface;

/*
 * Set up a blit between two surfaces -- split into three parts:
 * The upper part, SDL_BlitSurface(), performs clipping and rectangle
 * verification.  The lower part is a pointer to a low level
 * accelerated blitting function.
 *
 * These parts are separated out and each used internally by this
 * library in the optimum places.  They are exported so that if
 * you know exactly what you are doing, you can optimize your code
 * by calling the one(s) you need.
 */
impl Surface<'_> {
    fn check_blit_surfaces(&self, dst: &Surface<'_>, need_pixels: bool) -> Result<()> {
        // Make sure the surfaces aren't locked
        if need_pixels && !self.pixels.is_some() && !self.must_lock() {
            return Err(Error::invalid_param("src"));
        }
        if need_pixels && !dst.pixels.is_some() && !dst.must_lock() {
            return Err(Error::invalid_param("dst"));
        }
        if self.flags.contains(SurfaceFlags::LOCKED) || dst.flags.contains(SurfaceFlags::LOCKED) {
            return Err(Error::new("Surfaces must not be locked during blit"));
        }
        Ok(())
    }

    /// Low-level blit with no clipping: the rectangles must already be
    /// clipped to both surfaces. Translation of `SDL_BlitSurfaceUnchecked()`.
    ///
    /// # Panics
    ///
    /// If the rectangles reach outside either surface (upstream reads and
    /// writes out of bounds).
    pub fn blit_unchecked(
        &mut self,
        srcrect: &Rect,
        dst: &mut Surface<'_>,
        dstrect: &Rect,
    ) -> Result<()> {
        // Check to make sure the blit mapping is valid
        validate_map(self, dst)?;
        match self.map.blit {
            MapBlit::Soft(f) => soft_blit(self, srcrect, dst, dstrect, f),
            MapBlit::Rle(kind) => crate::video::rle::rle_blit(self, srcrect, dst, dstrect, kind),
            MapBlit::None => Err(Error::new("Blit combination not supported")),
        }
    }

    /// Copy `srcrect` (`None` = everything) of this surface onto `dst` at the
    /// position of `dstrect` (`None` = the origin), clipped to both surfaces
    /// and `dst`'s clip rectangle, with this surface's blend mode, colorkey
    /// and modulation. Translation of `SDL_BlitSurface()`.
    ///
    /// Only `dstrect`'s position is used; the size is `srcrect`'s.
    pub fn blit(
        &mut self,
        srcrect: Option<&Rect>,
        dst: &mut Surface<'_>,
        dstrect: Option<&Rect>,
    ) -> Result<()> {
        self.check_blit_surfaces(dst, true)?;

        // Full src surface
        let mut r_src = Rect::new(0, 0, self.w, self.h);

        let mut r_dst = match dstrect {
            Some(d) => Rect::new(d.x, d.y, 0, 0),
            None => Rect::new(0, 0, 0, 0),
        };

        // clip the source rectangle to the source surface
        if let Some(srcrect) = srcrect {
            let mut tmp = Rect::default();
            if !srcrect.intersect_into(&r_src, &mut tmp) {
                return Ok(());
            }

            // Shift dstrect, if srcrect origin has changed
            r_dst.x += tmp.x - srcrect.x;
            r_dst.y += tmp.y - srcrect.y;

            // Update srcrect
            r_src = tmp;
        }

        // There're no dstrect.w/h parameters. It's the same as srcrect
        r_dst.w = r_src.w;
        r_dst.h = r_src.h;

        // clip the destination rectangle against the clip rectangle
        {
            let mut tmp = Rect::default();
            if !r_dst.intersect_into(&dst.clip_rect, &mut tmp) {
                return Ok(());
            }

            // Shift srcrect, if dstrect has changed
            r_src.x += tmp.x - r_dst.x;
            r_src.y += tmp.y - r_dst.y;
            r_src.w = tmp.w;
            r_src.h = tmp.h;

            // Update dstrect
            r_dst = tmp;
        }

        if r_dst.w <= 0 || r_dst.h <= 0 {
            // No-op.
            return Ok(());
        }

        // Switch back to a fast blit if we were previously stretching
        if self.map.flags & COPY_NEAREST != 0 {
            self.map.flags &= !COPY_NEAREST;
            self.map.invalidate();
        }

        self.blit_unchecked(&r_src, dst, &r_dst)
    }

    /// Translation of `SDL_BlitSurfaceClippedScaled()`.
    fn blit_clipped_scaled(
        &self,
        srcrect: &Rect,
        dst: &mut Surface<'_>,
        dstrect: &Rect,
        scale_mode: ScaleMode,
    ) -> Result<()> {
        // We need to scale first, then blit into dst because we're clipping in the destination surface pixel coordinates
        let view = self.view(Some(srcrect))?;
        let mut scaled = view.scale(dstrect.w, dstrect.h, scale_mode)?;
        scaled.blit(None, dst, Some(dstrect))
    }

    /// Copy `srcrect` of this surface to `dstrect` of `dst`, scaling to fit
    /// (`None` = the whole surface). Translation of `SDL_BlitSurfaceScaled()`.
    pub fn blit_scaled(
        &mut self,
        srcrect: Option<&Rect>,
        dst: &mut Surface<'_>,
        dstrect: Option<&Rect>,
        mut scale_mode: ScaleMode,
    ) -> Result<()> {
        self.check_blit_surfaces(dst, true)?;

        if scale_mode == ScaleMode::PixelArt {
            scale_mode = ScaleMode::Nearest;
        }

        let src_w = srcrect.map_or(self.w, |r| r.w);
        let src_h = srcrect.map_or(self.h, |r| r.h);
        let dst_w = dstrect.map_or(dst.w, |r| r.w);
        let dst_h = dstrect.map_or(dst.h, |r| r.h);
        if src_w == dst_w && src_h == dst_h {
            // No scaling, defer to regular blit
            return self.blit(srcrect, dst, dstrect);
        }

        if self.w == 0 || self.h == 0 {
            // Nothing to do
            return Ok(());
        }

        // Full src surface
        let mut r_src = Rect::new(0, 0, self.w, self.h);

        let mut r_dst = match dstrect {
            Some(d) => *d,
            None => Rect::new(0, 0, dst.w, dst.h),
        };

        // clip the source rectangle to the source surface
        if let Some(srcrect) = srcrect {
            let desired = Rect::new(srcrect.x, srcrect.y, srcrect.w.max(1), srcrect.h.max(1));
            let mut tmp = Rect::default();
            if !desired.intersect_into(&r_src, &mut tmp) {
                return Ok(());
            }

            // Shift dstrect, if srcrect origin has changed
            r_dst.x += (tmp.x - desired.x) * r_dst.w / desired.w;
            r_dst.y += (tmp.y - desired.y) * r_dst.h / desired.h;
            r_dst.w += (tmp.w - desired.w) * r_dst.w / desired.w;
            r_dst.h += (tmp.h - desired.h) * r_dst.h / desired.h;

            // Update srcrect
            r_src = tmp;
        }

        let mut tmp = Rect::default();
        if !r_dst.intersect_into(&dst.clip_rect, &mut tmp) {
            return Ok(());
        }

        if tmp != r_dst {
            // Need to do a clipped and scaled blit
            return self.blit_clipped_scaled(&r_src, dst, &r_dst, scale_mode);
        }

        let src_must_lock = self.must_lock();
        if src_must_lock {
            self.lock_raw()?;
        }
        let dst_must_lock = dst.must_lock();
        if dst_must_lock {
            if let Err(e) = dst.lock_raw() {
                if src_must_lock {
                    self.unlock_raw();
                }
                return Err(e);
            }
        }

        let result = self.blit_unchecked_scaled(&r_src, dst, &r_dst, scale_mode);

        if self.must_lock() {
            self.unlock_raw();
        }
        if dst.must_lock() {
            dst.unlock_raw();
        }
        result
    }

    /// Low-level scaled blit with no clipping. Translation of `SDL_BlitSurfaceUncheckedScaled()`.
    pub fn blit_unchecked_scaled(
        &mut self,
        srcrect: &Rect,
        dst: &mut Surface<'_>,
        dstrect: &Rect,
        scale_mode: ScaleMode,
    ) -> Result<()> {
        const COMPLEX_COPY_FLAGS: u32 =
            crate::video::blit::COPY_MODULATE_MASK | COPY_BLEND_MASK | COPY_COLORKEY;

        if srcrect.w > u16::MAX as i32
            || srcrect.h > u16::MAX as i32
            || dstrect.w > u16::MAX as i32
            || dstrect.h > u16::MAX as i32
        {
            return Err(Error::new("Size too large for scaling"));
        }

        if self.map.flags & COPY_NEAREST == 0 {
            self.map.flags |= COPY_NEAREST;
            self.map.invalidate();
        }

        if scale_mode == ScaleMode::Nearest || scale_mode == ScaleMode::PixelArt {
            if self.map.flags & COMPLEX_COPY_FLAGS == 0
                && self.format == dst.format
                && (!self.format.is_indexed()
                    || (self.format.bits_per_pixel() == 8
                        && same_palette(&self.palette, &dst.palette)))
                && self.format.bytes_per_pixel() <= 4
            {
                stretch_surface(self, Some(srcrect), dst, Some(dstrect), ScaleMode::Nearest)
            } else if self.format.bits_per_pixel() < 8 {
                // Scaling bitmap not yet supported, convert to RGBA for blit
                let mut tmp = self.convert(PixelFormat::ARGB8888)?;
                tmp.blit_unchecked_scaled(srcrect, dst, dstrect, ScaleMode::Nearest)
            } else {
                self.blit_unchecked(srcrect, dst, dstrect)
            }
        } else if self.map.flags & COMPLEX_COPY_FLAGS == 0
            && self.format == dst.format
            && !self.format.is_indexed()
            && self.format.bytes_per_pixel() == 4
            && self.format != PixelFormat::ARGB2101010
        {
            // fast path
            stretch_surface(self, Some(srcrect), dst, Some(dstrect), ScaleMode::Linear)
        } else if self.format.bits_per_pixel() < 8 {
            // Scaling bitmap not yet supported, convert to RGBA for blit
            let mut tmp = self.convert(PixelFormat::ARGB8888)?;
            tmp.blit_unchecked_scaled(srcrect, dst, dstrect, scale_mode)
        } else {
            // Use intermediate surface(s)
            let is_complex_copy_flags = self.map.flags & COMPLEX_COPY_FLAGS;

            // Save source infos
            let (r, g, b) = self.color_mod();
            let alpha = self.alpha_mod();
            let blend_mode = self.blend_mode();
            let mut srcrect2 = *srcrect;

            // Change source format if not appropriate for scaling
            let mut tmp1: Option<Surface<'static>> = None;
            if self.format.bytes_per_pixel() != 4 || self.format == PixelFormat::ARGB2101010 {
                let fmt = if dst.format.bytes_per_pixel() == 4
                    && dst.format != PixelFormat::ARGB2101010
                    && (dst.format.has_alpha() || is_complex_copy_flags == 0)
                {
                    dst.format
                } else {
                    PixelFormat::ARGB8888
                };
                tmp1 = Some(self.convert_rect(Some(srcrect), fmt)?);
                srcrect2.x = 0;
                srcrect2.y = 0;
            }
            #[allow(clippy::too_many_arguments)]
            fn intermediate(
                src: &mut Surface<'_>,
                srcrect2: &Rect,
                dst: &mut Surface<'_>,
                dstrect: &Rect,
                is_complex_copy_flags: u32,
                (r, g, b, alpha): (u8, u8, u8, u8),
                blend_mode: BlendMode,
            ) -> Result<()> {
                // Intermediate scaling
                if is_complex_copy_flags != 0 || src.format != dst.format {
                    let mut tmp2 = Surface::new_uninitialized(dstrect.w, dstrect.h, src.format)?;
                    let _ =
                        stretch_surface(src, Some(srcrect2), &mut tmp2, None, ScaleMode::Linear);

                    tmp2.set_color_mod(r, g, b);
                    tmp2.set_alpha_mod(alpha);
                    let _ = tmp2.set_blend_mode(blend_mode);

                    let tmprect = Rect::new(0, 0, dstrect.w, dstrect.h);
                    tmp2.blit_unchecked(&tmprect, dst, dstrect)
                } else {
                    stretch_surface(src, Some(srcrect2), dst, Some(dstrect), ScaleMode::Linear)
                }
            }
            let mods = (r, g, b, alpha);
            match tmp1.as_mut() {
                Some(t) => intermediate(
                    t,
                    &srcrect2,
                    dst,
                    dstrect,
                    is_complex_copy_flags,
                    mods,
                    blend_mode,
                ),
                None => intermediate(
                    self,
                    &srcrect2,
                    dst,
                    dstrect,
                    is_complex_copy_flags,
                    mods,
                    blend_mode,
                ),
            }
        }
    }

    /// Fill `dstrect` of `dst` (`None` = all of it) with repeated copies of
    /// `srcrect` of this surface, without scaling. Translation of `SDL_BlitSurfaceTiled()`.
    pub fn blit_tiled(
        &mut self,
        srcrect: Option<&Rect>,
        dst: &mut Surface<'_>,
        dstrect: Option<&Rect>,
    ) -> Result<()> {
        self.check_blit_surfaces(dst, false)?;

        // Full src surface
        let mut r_src = Rect::new(0, 0, self.w, self.h);

        let mut r_dst = match dstrect {
            Some(d) => *d,
            None => Rect::new(0, 0, dst.w, dst.h),
        };

        // clip the source rectangle to the source surface
        if let Some(srcrect) = srcrect {
            let full = r_src;
            if !srcrect.intersect_into(&full, &mut r_src) {
                return Ok(());
            }

            // For tiling we don't adjust the destination rectangle
        }

        // clip the destination rectangle against the clip rectangle
        {
            let unclipped = r_dst;
            if !unclipped.intersect_into(&dst.clip_rect, &mut r_dst) {
                return Ok(());
            }

            // For tiling we don't adjust the source rectangle
        }

        // Switch back to a fast blit if we were previously stretching
        if self.map.flags & COPY_NEAREST != 0 {
            self.map.flags &= !COPY_NEAREST;
            self.map.invalidate();
        }

        let rows = r_dst.h / r_src.h;
        let cols = r_dst.w / r_src.w;
        let remaining_w = r_dst.w % r_src.w;
        let remaining_h = r_dst.h % r_src.h;

        let mut curr_src = r_src;
        let mut curr_dst = Rect::new(0, r_dst.y, r_src.w, r_src.h);
        for _ in 0..rows {
            curr_dst.x = r_dst.x;
            for _ in 0..cols {
                self.blit_unchecked(&curr_src, dst, &curr_dst)?;
                curr_dst.x += curr_dst.w;
            }
            if remaining_w != 0 {
                curr_src.w = remaining_w;
                curr_dst.w = remaining_w;
                self.blit_unchecked(&curr_src, dst, &curr_dst)?;
                curr_src.w = r_src.w;
                curr_dst.w = r_src.w;
            }
            curr_dst.y += curr_dst.h;
        }
        if remaining_h != 0 {
            curr_src.h = remaining_h;
            curr_dst.h = remaining_h;
            curr_dst.x = r_dst.x;
            for _ in 0..cols {
                self.blit_unchecked(&curr_src, dst, &curr_dst)?;
                curr_dst.x += curr_dst.w;
            }
            if remaining_w != 0 {
                curr_src.w = remaining_w;
                curr_dst.w = remaining_w;
                self.blit_unchecked(&curr_src, dst, &curr_dst)?;
            }
        }
        Ok(())
    }

    /// Tiled blit with each tile scaled by `scale`. Translation of `SDL_BlitSurfaceTiledWithScale()`.
    pub fn blit_tiled_with_scale(
        &mut self,
        srcrect: Option<&Rect>,
        scale: f32,
        scale_mode: ScaleMode,
        dst: &mut Surface<'_>,
        dstrect: Option<&Rect>,
    ) -> Result<()> {
        self.check_blit_surfaces(dst, false)?;
        if scale < 0.0 {
            return Err(Error::invalid_param("scale"));
        }

        // Full src surface
        let mut r_src = Rect::new(0, 0, self.w, self.h);

        let mut r_dst = match dstrect {
            Some(d) => *d,
            None => Rect::new(0, 0, dst.w, dst.h),
        };

        // clip the source rectangle to the source surface
        if let Some(srcrect) = srcrect {
            let full = r_src;
            if !srcrect.intersect_into(&full, &mut r_src) {
                return Ok(());
            }

            // For tiling we don't adjust the destination rectangle
        }

        // clip the destination rectangle against the clip rectangle
        {
            let unclipped = r_dst;
            if !unclipped.intersect_into(&dst.clip_rect, &mut r_dst) {
                return Ok(());
            }

            // For tiling we don't adjust the source rectangle
        }

        // Switch back to a fast blit if we were previously stretching
        if self.map.flags & COPY_NEAREST != 0 {
            self.map.flags &= !COPY_NEAREST;
            self.map.invalidate();
        }

        let tile_width = (r_src.w as f32 * scale).round() as i32;
        let tile_height = (r_src.h as f32 * scale).round() as i32;
        if tile_width <= 0 || tile_height <= 0 {
            // Nothing to do
            return Ok(());
        }
        let rows = r_dst.h / tile_height;
        let cols = r_dst.w / tile_width;
        let remaining_dst_w = r_dst.w - cols * tile_width;
        let remaining_dst_h = r_dst.h - rows * tile_height;
        let remaining_src_w = (remaining_dst_w as f32 / scale) as i32;
        let remaining_src_h = (remaining_dst_h as f32 / scale) as i32;

        let mut curr_src = r_src;
        let mut curr_dst = Rect::new(0, r_dst.y, tile_width, tile_height);
        for _ in 0..rows {
            curr_dst.x = r_dst.x;
            for _ in 0..cols {
                self.blit_unchecked_scaled(&curr_src, dst, &curr_dst, scale_mode)?;
                curr_dst.x += curr_dst.w;
            }
            if remaining_dst_w > 0 {
                curr_src.w = remaining_src_w;
                curr_dst.w = remaining_dst_w;
                self.blit_unchecked_scaled(&curr_src, dst, &curr_dst, scale_mode)?;
                curr_src.w = r_src.w;
                curr_dst.w = tile_width;
            }
            curr_dst.y += curr_dst.h;
        }
        if remaining_dst_h > 0 {
            curr_src.h = remaining_src_h;
            curr_dst.h = remaining_dst_h;
            curr_dst.x = r_dst.x;
            for _ in 0..cols {
                self.blit_unchecked_scaled(&curr_src, dst, &curr_dst, scale_mode)?;
                curr_dst.x += curr_dst.w;
            }
            if remaining_dst_w > 0 {
                curr_src.w = remaining_src_w;
                curr_dst.w = remaining_dst_w;
                self.blit_unchecked_scaled(&curr_src, dst, &curr_dst, scale_mode)?;
            }
        }
        Ok(())
    }

    /// Nine-patch blit: the corners are copied (scaled by `scale`), the edges
    /// stretched along one axis and the center along both.
    /// Translation of `SDL_BlitSurface9Grid()`.
    #[allow(clippy::too_many_arguments)]
    pub fn blit_9grid(
        &mut self,
        srcrect: Option<&Rect>,
        left_width: i32,
        right_width: i32,
        top_height: i32,
        bottom_height: i32,
        scale: f32,
        scale_mode: ScaleMode,
        dst: &mut Surface<'_>,
        dstrect: Option<&Rect>,
    ) -> Result<()> {
        let srcrect = *srcrect.unwrap_or(&Rect::new(0, 0, self.w, self.h));
        let dstrect = *dstrect.unwrap_or(&Rect::new(0, 0, dst.w, dst.h));

        let (dst_left_width, dst_right_width, dst_top_height, dst_bottom_height) =
            if scale <= 0.0 || scale == 1.0 {
                (left_width, right_width, top_height, bottom_height)
            } else {
                (
                    (left_width as f32 * scale).round() as i32,
                    (right_width as f32 * scale).round() as i32,
                    (top_height as f32 * scale).round() as i32,
                    (bottom_height as f32 * scale).round() as i32,
                )
            };

        // Upper-left corner
        let mut curr_src = Rect::new(srcrect.x, srcrect.y, left_width, top_height);
        let mut curr_dst = Rect::new(dstrect.x, dstrect.y, dst_left_width, dst_top_height);
        self.blit_scaled(Some(&curr_src), dst, Some(&curr_dst), scale_mode)?;

        // Upper-right corner
        curr_src.x = srcrect.x + srcrect.w - right_width;
        curr_src.w = right_width;
        curr_dst.x = dstrect.x + dstrect.w - dst_right_width;
        curr_dst.w = dst_right_width;
        self.blit_scaled(Some(&curr_src), dst, Some(&curr_dst), scale_mode)?;

        // Lower-right corner
        curr_src.y = srcrect.y + srcrect.h - bottom_height;
        curr_dst.y = dstrect.y + dstrect.h - dst_bottom_height;
        curr_dst.h = dst_bottom_height;
        self.blit_scaled(Some(&curr_src), dst, Some(&curr_dst), scale_mode)?;

        // Lower-left corner
        curr_src.x = srcrect.x;
        curr_src.w = left_width;
        curr_dst.x = dstrect.x;
        curr_dst.w = dst_left_width;
        self.blit_scaled(Some(&curr_src), dst, Some(&curr_dst), scale_mode)?;

        // Left
        curr_src.y = srcrect.y + top_height;
        curr_src.h = srcrect.h - top_height - bottom_height;
        curr_dst.y = dstrect.y + dst_top_height;
        curr_dst.h = dstrect.h - dst_top_height - dst_bottom_height;
        self.blit_scaled(Some(&curr_src), dst, Some(&curr_dst), scale_mode)?;

        // Right
        curr_src.x = srcrect.x + srcrect.w - right_width;
        curr_src.w = right_width;
        curr_dst.x = dstrect.x + dstrect.w - dst_right_width;
        curr_dst.w = dst_right_width;
        self.blit_scaled(Some(&curr_src), dst, Some(&curr_dst), scale_mode)?;

        // Top
        curr_src = Rect::new(
            srcrect.x + left_width,
            srcrect.y,
            srcrect.w - left_width - right_width,
            top_height,
        );
        curr_dst = Rect::new(
            dstrect.x + dst_left_width,
            dstrect.y,
            dstrect.w - dst_left_width - dst_right_width,
            dst_top_height,
        );
        self.blit_scaled(Some(&curr_src), dst, Some(&curr_dst), scale_mode)?;

        // Bottom
        curr_src.y = srcrect.y + srcrect.h - bottom_height;
        curr_dst.y = dstrect.y + dstrect.h - dst_bottom_height;
        curr_dst.h = dst_bottom_height;
        self.blit_scaled(Some(&curr_src), dst, Some(&curr_dst), scale_mode)?;

        // Center
        curr_src = Rect::new(
            srcrect.x + left_width,
            srcrect.y + top_height,
            srcrect.w - left_width - right_width,
            srcrect.h - top_height - bottom_height,
        );
        curr_dst = Rect::new(
            dstrect.x + dst_left_width,
            dstrect.y + dst_top_height,
            dstrect.w - dst_left_width - dst_right_width,
            dstrect.h - dst_top_height - dst_bottom_height,
        );
        self.blit_scaled(Some(&curr_src), dst, Some(&curr_dst), scale_mode)?;

        Ok(())
    }
}

/// `SDL_IsSamePalette()` on two optional surface palettes (upstream
/// dereferences them; a missing palette never matches here).
fn same_palette(a: &Option<SharedPalette>, b: &Option<SharedPalette>) -> bool {
    match (a, b) {
        (Some(a), Some(b)) => Arc::ptr_eq(a, b) || read_palette(a).is_prefix_of(&read_palette(b)),
        _ => false,
    }
}
