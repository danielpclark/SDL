// Rust translation of src/render/software/SDL_render_sw.c from Simple
// DirectMedia Layer.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! The software renderer: draws into a surface (the output surface, or a
//! target texture's surface) with the surface blitters and the drawing
//! primitives.

use std::any::Any;

use crate::error::{Error, Result};
use crate::hints;
use crate::render::software::draw;
use crate::render::software::triangle::{
    sw_blit_triangle, sw_fill_triangle, trianglepoint_2_fixedpoint,
};
use crate::render::sysrender::{
    CopyEx, DrawCmd, DrawKind, Geometry, RenderBackend, RenderCommand, TextureCreateProps,
    TextureData, TextureStore,
};
use crate::render::{Texture, TextureAccess};
use crate::video::pixels::{Color, Colorspace, FColor, Palette, PixelFormat};
use crate::video::rect::{FPoint, FRect, Point, Rect};
use crate::video::rotate::{rotate_surface, rotozoom_surface_size_trig};
use crate::video::surface::{
    duplicate_pixels, share_palette, write_palette, ScaleMode, SharedPalette, Surface,
};
use crate::video::{BlendMode, FlipMode, Window, WindowSurface};

/// The name of the software renderer. Translation of `SDL_SOFTWARE_RENDERER`.
pub const SOFTWARE_RENDERER: &str = "software";

/// The vertex data the software renderer queues.
#[derive(Clone, Copy, Debug)]
enum SwVert {
    Point(Point),
    Rect(Rect),
    CopyEx(CopyExData),
    Fill(GeometryFillData),
    Copy(GeometryCopyData),
}

/// Translation of `CopyExData`.
#[derive(Clone, Copy, Debug)]
struct CopyExData {
    srcrect: Rect,
    dstrect: Rect,
    angle: f64,
    center: FPoint,
    flip: FlipMode,
    scale_x: f32,
    scale_y: f32,
}

/// Translation of `GeometryFillData`.
#[derive(Clone, Copy, Debug)]
struct GeometryFillData {
    dst: Point,
    color: Color,
}

/// Translation of `GeometryCopyData`.
#[derive(Clone, Copy, Debug)]
struct GeometryCopyData {
    src: Point,
    dst: Point,
    color: Color,
}

/// Translation of `SW_DrawStateCache`.
struct SwDrawStateCache {
    viewport: Option<Rect>,
    cliprect: Option<Rect>,
    surface_cliprect_dirty: bool,
    color: Color,
}

/// The software renderer's data. Translation of `SW_RenderData`: `surface`
/// is the output surface (`data->window` for a surface renderer) and
/// `target` the texture drawn into instead, if any.
///
/// A renderer for a window draws into the window's surface: `window_surface`
/// is the handle to it (`data->window`), moved into `surface` while
/// commands run.
pub(crate) struct SwRenderer {
    surface: Option<Surface<'static>>,
    target: Option<Texture>,
    verts: Vec<SwVert>,
    window: Option<Window>,
    window_surface: Option<WindowSurface>,
}

/// `(Uint8)SDL_roundf(SDL_clamp(v, 0.0f, 1.0f) * 255.0f)`.
fn unit_to_byte(v: f32) -> u8 {
    (v.clamp(0.0, 1.0) * 255.0).round() as u8
}

/// A draw color for the surface functions.
fn color_bytes(color: FColor, color_scale: f32) -> Color {
    Color::new(
        unit_to_byte(color.r * color_scale),
        unit_to_byte(color.g * color_scale),
        unit_to_byte(color.b * color_scale),
        unit_to_byte(color.a),
    )
}

/// The texture's software surface.
fn texture_surface(texture: &TextureData) -> &Surface<'static> {
    texture
        .internal
        .as_ref()
        .and_then(|i| i.downcast_ref::<Surface<'static>>())
        .expect("software texture without a surface")
}

fn texture_surface_mut(texture: &mut TextureData) -> &mut Surface<'static> {
    texture
        .internal
        .as_mut()
        .and_then(|i| i.downcast_mut::<Surface<'static>>())
        .expect("software texture without a surface")
}

impl SwRenderer {
    /// The surface drawn into (`SW_ActivateRenderer()`).
    fn activate<'s>(
        &'s mut self,
        textures: &'s mut TextureStore,
    ) -> Option<&'s mut Surface<'static>> {
        match self.target {
            Some(t) => textures.get_mut(t).map(texture_surface_mut),
            None => self.surface.as_mut(),
        }
    }

    /// The texture formats a software renderer for a surface of `format`
    /// supports, best first. Translation of `SW_SelectBestFormats()`.
    pub(crate) fn select_best_formats(format: PixelFormat) -> Vec<PixelFormat> {
        use PixelFormat as F;
        let mut formats = Vec::new();

        // Prefer the format used by the framebuffer by default.
        formats.push(format);

        let alt = match format {
            F::XRGB4444 => Some(F::ARGB4444),
            F::XBGR4444 => Some(F::ABGR4444),
            F::ARGB4444 => Some(F::XRGB4444),
            F::ABGR4444 => Some(F::XBGR4444),
            F::XRGB1555 => Some(F::ARGB1555),
            F::XBGR1555 => Some(F::ABGR1555),
            F::ARGB1555 => Some(F::XRGB1555),
            F::ABGR1555 => Some(F::XBGR1555),
            F::XRGB8888 => Some(F::ARGB8888),
            F::RGBX8888 => Some(F::RGBA8888),
            F::XBGR8888 => Some(F::ABGR8888),
            F::BGRX8888 => Some(F::BGRA8888),
            F::ARGB8888 => Some(F::XRGB8888),
            F::RGBA8888 => Some(F::RGBX8888),
            F::ABGR8888 => Some(F::XBGR8888),
            F::BGRA8888 => Some(F::BGRX8888),
            _ => None,
        };
        formats.extend(alt);

        /* Ensure that we always have a SDL_PACKEDLAYOUT_8888 format. Having a matching component order increases the
         * chances of getting a fast path for blitting.
         */
        use crate::video::pixels::{PackedLayout, PackedOrder};
        if format.is_packed() {
            if format.pixel_layout() != PackedLayout::L8888 {
                let order = format.pixel_order();
                if order == PackedOrder::Bgrx as u32 || order == PackedOrder::Bgra as u32 {
                    formats.extend([F::BGRX8888, F::BGRA8888]);
                } else if order == PackedOrder::Rgbx as u32 || order == PackedOrder::Rgba as u32 {
                    formats.extend([F::RGBX8888, F::RGBA8888]);
                } else if order == PackedOrder::Xbgr as u32 || order == PackedOrder::Abgr as u32 {
                    formats.extend([F::XBGR8888, F::ABGR8888]);
                } else {
                    // SDL_PACKEDORDER_XRGB, SDL_PACKEDORDER_ARGB and default
                    formats.extend([F::XRGB8888, F::ARGB8888]);
                }
            }
        } else {
            formats.extend([F::XRGB8888, F::ARGB8888]);
        }

        // Add 8-bit palettized format
        formats.push(F::INDEX8);
        formats
    }

    /// The software renderer for an output surface. Translation of
    /// `SW_CreateRendererForSurface()` (the checks; the front end does the
    /// rest).
    pub(crate) fn for_surface(surface: Surface<'static>) -> Result<SwRenderer> {
        let bits = surface.format().bits_per_pixel();
        if !(8..=32).contains(&bits) {
            return Err(Error::new("Unsupported surface format"));
        }
        Ok(SwRenderer {
            surface: Some(surface),
            target: None,
            verts: Vec::new(),
            window: None,
            window_surface: None,
        })
    }

    /// The software renderer for a window. Translation of
    /// `SW_CreateRenderer()`: the window surface is created with the vsync
    /// hint set from `present_vsync` (unless the app set it).
    pub(crate) fn for_window(
        window: Window,
        present_vsync: bool,
    ) -> Result<(SwRenderer, PixelFormat)> {
        // Set the vsync hint based on our flags, if it's not already set
        let hint = hints::get(hints::RENDER_VSYNC);
        let no_hint_set = hint.as_deref().is_none_or(str::is_empty);

        if no_hint_set {
            let _ = hints::set(hints::RENDER_VSYNC, if present_vsync { "1" } else { "0" });
        }

        let surface = window.surface();

        // Reset the vsync hint if we set it above
        if no_hint_set {
            let _ = hints::set(hints::RENDER_VSYNC, "");
        }

        let surface = surface?;
        let format = surface.lock().format();
        let bits = format.bits_per_pixel();
        if !(8..=32).contains(&bits) {
            let _ = window.destroy_surface();
            return Err(Error::new("Unsupported surface format"));
        }

        Ok((
            SwRenderer {
                surface: None,
                target: None,
                verts: Vec::new(),
                window: Some(window),
                window_surface: Some(surface),
            },
            format,
        ))
    }

    /// Make sure a window renderer has the window's current surface.
    /// Translation of the window part of `SW_ActivateRenderer()`.
    fn activate_window(&mut self) {
        if let Some(window) = self.window {
            let valid = self
                .window_surface
                .as_ref()
                .is_some_and(|s| window.is_current_surface(s));
            if !valid {
                if let Ok(surface) = window.surface() {
                    self.window_surface = Some(surface);
                }
            }
        }
    }

    /// Run `f` with the output surface in `self.surface` (for a window
    /// renderer, the window's surface, locked meanwhile).
    fn with_output<R>(&mut self, f: impl FnOnce(&mut SwRenderer) -> R) -> R {
        self.activate_window();
        let Some(shared) = self.window_surface.clone() else {
            return f(self);
        };
        let mut guard = shared.lock();
        let placeholder = match Surface::without_pixels(1, 1, guard.format()) {
            Ok(p) => p,
            Err(_) => return f(self),
        };
        self.surface = Some(std::mem::replace(&mut *guard, placeholder));
        let r = f(self);
        if let Some(surface) = self.surface.take() {
            *guard = surface;
        }
        r
    }

    /// Translation of `SW_RunCommandQueue()`, with the output surface in place.
    fn run_commands(&mut self, cmds: &[RenderCommand], textures: &mut TextureStore) -> Result<()> {
        let target = self.target;
        let mut verts = std::mem::take(&mut self.verts);
        let result = (|| -> Result<()> {
            let SwRenderer {
                surface: output, ..
            } = self;
            let mut drawstate = SwDrawStateCache {
                viewport: None,
                cliprect: None,
                surface_cliprect_dirty: true,
                color: Color::new(0, 0, 0, 0),
            };

            if target.is_none() && output.is_none() {
                return Err(Error::invalid_param("surface"));
            }

            for cmd in cmds {
                match *cmd {
                    RenderCommand::SetDrawColor {
                        color, color_scale, ..
                    } => {
                        drawstate.color = color_bytes(color, color_scale);
                    }
                    RenderCommand::SetViewport { rect, .. } => {
                        drawstate.viewport = Some(rect);
                        drawstate.surface_cliprect_dirty = true;
                    }
                    RenderCommand::SetClipRect { enabled, rect } => {
                        drawstate.cliprect = enabled.then_some(rect);
                        drawstate.surface_cliprect_dirty = true;
                    }
                    RenderCommand::Clear {
                        color, color_scale, ..
                    } => {
                        let c = color_bytes(color, color_scale);
                        let surface = active(output, target, textures)?;
                        // By definition the clear ignores the clip rect
                        surface.set_clip_rect(None);
                        let pixel = surface.map_rgba(c.r, c.g, c.b, c.a);
                        let _ = surface.fill_rect(None, pixel);
                        drawstate.surface_cliprect_dirty = true;
                    }
                    RenderCommand::Draw(kind @ (DrawKind::Points | DrawKind::Lines), d) => {
                        let Color { r, g, b, a } = drawstate.color;
                        let blend = d.blend;
                        let surface = active(output, target, textures)?;
                        set_draw_state(surface, &mut drawstate);

                        let mut points: Vec<Point> = verts[d.first..d.first + d.count]
                            .iter()
                            .map(|v| match v {
                                SwVert::Point(p) => *p,
                                _ => unreachable!(),
                            })
                            .collect();

                        // Apply viewport
                        if let Some(vp) = drawstate.viewport.filter(|vp| vp.x != 0 || vp.y != 0) {
                            for p in points.iter_mut() {
                                p.x += vp.x;
                                p.y += vp.y;
                            }
                        }

                        let _ = if kind == DrawKind::Points {
                            if blend == BlendMode::NONE {
                                let pixel = surface.map_rgba(r, g, b, a);
                                draw::draw_points(surface, &points, pixel)
                            } else {
                                draw::blend_points(surface, &points, blend, r, g, b, a)
                            }
                        } else if blend == BlendMode::NONE {
                            let pixel = surface.map_rgba(r, g, b, a);
                            draw::draw_lines(surface, &points, pixel)
                        } else {
                            draw::blend_lines(surface, &points, blend, r, g, b, a)
                        };
                    }
                    RenderCommand::Draw(DrawKind::FillRects, d) => {
                        let Color { r, g, b, a } = drawstate.color;
                        let blend = d.blend;
                        let surface = active(output, target, textures)?;
                        set_draw_state(surface, &mut drawstate);

                        let mut rects: Vec<Rect> = verts[d.first..d.first + d.count]
                            .iter()
                            .map(|v| match v {
                                SwVert::Rect(r) => *r,
                                _ => unreachable!(),
                            })
                            .collect();

                        // Apply viewport
                        if let Some(vp) = drawstate.viewport.filter(|vp| vp.x != 0 || vp.y != 0) {
                            for rc in rects.iter_mut() {
                                rc.x += vp.x;
                                rc.y += vp.y;
                            }
                        }

                        let _ = if blend == BlendMode::NONE {
                            let pixel = surface.map_rgba(r, g, b, a);
                            surface.fill_rects(&rects, pixel)
                        } else {
                            draw::blend_fill_rects(surface, &rects, blend, r, g, b, a)
                        };
                    }
                    RenderCommand::Draw(DrawKind::Copy, d) => {
                        let (SwVert::Rect(srcrect), SwVert::Rect(mut dstrect)) =
                            (verts[d.first], verts[d.first + 1])
                        else {
                            unreachable!()
                        };
                        let Some(texture) = d.texture else { continue };
                        with_source(output, target, textures, texture, |surface, tex| {
                            set_draw_state(surface, &mut drawstate);
                            prep_texture_for_copy(&d, &drawstate, tex, Some(&srcrect));

                            // Apply viewport
                            if let Some(vp) = drawstate.viewport.filter(|vp| vp.x != 0 || vp.y != 0)
                            {
                                dstrect.x += vp.x;
                                dstrect.y += vp.y;
                            }

                            let src = texture_surface_mut(tex);
                            if srcrect.w == dstrect.w && srcrect.h == dstrect.h {
                                let _ = src.blit(Some(&srcrect), surface, Some(&dstrect));
                            } else if dstrect.x < 0
                                || dstrect.y < 0
                                || dstrect.x + dstrect.w > surface.width()
                                || dstrect.y + dstrect.h > surface.height()
                            {
                                // Prevent to do scaling + clipping on viewport boundaries as it may lose proportion
                                let tmp_format = if src.format().has_alpha() {
                                    PixelFormat::ARGB8888
                                } else {
                                    surface.format()
                                };
                                // Scale to an intermediate surface, then blit
                                if let Ok(mut tmp) =
                                    Surface::new_uninitialized(dstrect.w, dstrect.h, tmp_format)
                                {
                                    // Upstream leaves an indexed intermediate
                                    // surface without a palette, and the scaled
                                    // blit into it dereferences the missing
                                    // palette. Fixed here: it uses the output's.
                                    if tmp_format.is_indexed() {
                                        let _ = tmp.set_palette(surface.palette().cloned());
                                    }
                                    let r = Rect::new(0, 0, dstrect.w, dstrect.h);
                                    let blendmode = src.blend_mode();
                                    let alpha_mod = src.alpha_mod();
                                    let (r_mod, g_mod, b_mod) = src.color_mod();

                                    let _ = src.set_blend_mode(BlendMode::NONE);
                                    src.set_color_mod(255, 255, 255);
                                    src.set_alpha_mod(255);

                                    let _ = src.blit_scaled(
                                        Some(&srcrect),
                                        &mut tmp,
                                        Some(&r),
                                        d.texture_scale_mode,
                                    );

                                    tmp.set_color_mod(r_mod, g_mod, b_mod);
                                    tmp.set_alpha_mod(alpha_mod);
                                    let _ = tmp.set_blend_mode(blendmode);

                                    let _ = tmp.blit(None, surface, Some(&dstrect));
                                    // No need to set back r/g/b/a/blendmode to 'src' since it's done in PrepTextureForCopy()
                                }
                            } else {
                                let _ = src.blit_scaled(
                                    Some(&srcrect),
                                    surface,
                                    Some(&dstrect),
                                    d.texture_scale_mode,
                                );
                            }
                        })?;
                    }
                    RenderCommand::Draw(DrawKind::CopyEx, d) => {
                        let SwVert::CopyEx(mut copydata) = verts[d.first] else {
                            unreachable!()
                        };
                        let Some(texture) = d.texture else { continue };
                        with_source(output, target, textures, texture, |surface, tex| {
                            set_draw_state(surface, &mut drawstate);
                            prep_texture_for_copy(&d, &drawstate, tex, Some(&copydata.srcrect));

                            // Apply viewport
                            if let Some(vp) = drawstate.viewport {
                                if (vp.x != 0 || vp.y != 0)
                                    && (copydata.scale_x > 0.0 && copydata.scale_y > 0.0)
                                {
                                    copydata.dstrect.x += (vp.x as f32 / copydata.scale_x) as i32;
                                    copydata.dstrect.y += (vp.y as f32 / copydata.scale_y) as i32;
                                }
                            }

                            let _ = sw_render_copy_ex(
                                surface,
                                texture_surface_mut(tex),
                                &copydata.srcrect,
                                &copydata.dstrect,
                                copydata.angle,
                                &copydata.center,
                                copydata.flip,
                                copydata.scale_x,
                                copydata.scale_y,
                                d.texture_scale_mode,
                            );
                        })?;
                    }
                    RenderCommand::Draw(DrawKind::Geometry, d) => {
                        let count = d.count;
                        let blend = d.blend;
                        let vp = drawstate
                            .viewport
                            .filter(|vp| vp.x != 0 || vp.y != 0)
                            .map(|vp| {
                                let mut p = Point { x: vp.x, y: vp.y };
                                trianglepoint_2_fixedpoint(&mut p);
                                p
                            });
                        match d.texture {
                            Some(texture) => {
                                let mut data: Vec<GeometryCopyData> = verts
                                    [d.first..d.first + count]
                                    .iter()
                                    .map(|v| match v {
                                        SwVert::Copy(c) => *c,
                                        _ => unreachable!(),
                                    })
                                    .collect();
                                with_source(output, target, textures, texture, |surface, tex| {
                                    set_draw_state(surface, &mut drawstate);
                                    prep_texture_for_copy(&d, &drawstate, tex, None);

                                    // Apply viewport
                                    if let Some(vp) = vp {
                                        for v in data.iter_mut() {
                                            v.dst.x += vp.x;
                                            v.dst.y += vp.y;
                                        }
                                    }

                                    let src = texture_surface_mut(tex);
                                    for t in data.chunks_exact(3) {
                                        let _ = sw_blit_triangle(
                                            src,
                                            &t[0].src,
                                            &t[1].src,
                                            &t[2].src,
                                            surface,
                                            &t[0].dst,
                                            &t[1].dst,
                                            &t[2].dst,
                                            t[0].color,
                                            t[1].color,
                                            t[2].color,
                                            d.texture_address_mode_u,
                                            d.texture_address_mode_v,
                                        );
                                    }
                                })?;
                            }
                            None => {
                                let surface = active(output, target, textures)?;
                                set_draw_state(surface, &mut drawstate);
                                let mut data: Vec<GeometryFillData> = verts
                                    [d.first..d.first + count]
                                    .iter()
                                    .map(|v| match v {
                                        SwVert::Fill(f) => *f,
                                        _ => unreachable!(),
                                    })
                                    .collect();

                                // Apply viewport
                                if let Some(vp) = vp {
                                    for v in data.iter_mut() {
                                        v.dst.x += vp.x;
                                        v.dst.y += vp.y;
                                    }
                                }

                                for t in data.chunks_exact(3) {
                                    let _ = sw_fill_triangle(
                                        surface, &t[0].dst, &t[1].dst, &t[2].dst, blend,
                                        t[0].color, t[1].color, t[2].color,
                                    );
                                }
                            }
                        }
                    }
                    RenderCommand::NoOp => {}
                }
            }
            Ok(())
        })();
        verts.clear();
        self.verts = verts;
        result
    }

    /// Translation of `SW_RenderReadPixels()`, with the output surface in place.
    fn read_output_pixels(
        &mut self,
        rect: &Rect,
        textures: &mut TextureStore,
    ) -> Option<Result<Surface<'static>>> {
        let Some(surface) = self.activate(textures) else {
            return Some(Err(Error::invalid_param("surface")));
        };

        /* NOTE: The rect is already adjusted according to the viewport by
         * SDL_RenderReadPixels.
         */

        if rect.x < 0
            || rect.x + rect.w > surface.width()
            || rect.y < 0
            || rect.y + rect.h > surface.height()
        {
            return Some(Err(Error::new("Tried to read outside of surface bounds")));
        }

        let offset = rect.y as usize * surface.pitch() as usize
            + rect.x as usize * surface.format().bytes_per_pixel() as usize;
        let pixels = surface.raw_pixels().map(|p| &p[offset..]);
        Some(duplicate_pixels(
            rect.w,
            rect.h,
            surface.format(),
            Colorspace::SRGB,
            pixels,
            surface.pitch(),
        ))
    }

    fn push(&mut self, v: SwVert) {
        self.verts.push(v);
    }
}

/// Translation of `PrepTextureForCopy()`.
fn prep_texture_for_copy(
    cmd: &DrawCmd,
    drawstate: &SwDrawStateCache,
    texture: &mut TextureData,
    srcrect: Option<&Rect>,
) {
    let Color { r, g, b, a } = drawstate.color;
    let blend = cmd.blend;
    let access = texture.access;
    let surface = texture_surface_mut(texture);

    if surface.has_rle() && access == TextureAccess::Static && surface.format().has_alpha() {
        if let Some(srcrect) = srcrect {
            if srcrect.x != 0
                || srcrect.y != 0
                || srcrect.w != surface.width()
                || srcrect.h != surface.height()
            {
                let _ = surface.set_rle(false);
            }
        }
    }

    // !!! FIXME: we can probably avoid some of these calls.
    surface.set_color_mod(r, g, b);
    surface.set_alpha_mod(a);
    let _ = surface.set_blend_mode(blend);
}

/// Translation of `SetDrawState()`.
fn set_draw_state(surface: &mut Surface<'_>, drawstate: &mut SwDrawStateCache) {
    if drawstate.surface_cliprect_dirty {
        let viewport = drawstate.viewport;
        let cliprect = drawstate.cliprect;
        crate::sdl_assert_release!(viewport.is_some()); // the higher level should have forced a SDL_RENDERCMD_SETVIEWPORT

        match (cliprect, viewport) {
            (Some(cliprect), Some(viewport)) => {
                let clip = Rect::new(
                    cliprect.x + viewport.x,
                    cliprect.y + viewport.y,
                    cliprect.w,
                    cliprect.h,
                );
                let mut clip_rect = clip;
                viewport.intersect_into(&clip, &mut clip_rect);
                surface.set_clip_rect(Some(&clip_rect));
            }
            _ => {
                surface.set_clip_rect(viewport.as_ref());
            }
        }
        drawstate.surface_cliprect_dirty = false;
    }
}

/// Translation of `Blit_to_Screen()`.
fn blit_to_screen(
    src: &mut Surface<'_>,
    srcrect: Option<&Rect>,
    surface: &mut Surface<'_>,
    dstrect: &Rect,
    scale_x: f32,
    scale_y: f32,
    scale_mode: ScaleMode,
) -> Result<()> {
    // Renderer scaling, if needed
    if scale_x != 1.0 || scale_y != 1.0 {
        let r = Rect::new(
            (dstrect.x as f32 * scale_x) as i32,
            (dstrect.y as f32 * scale_y) as i32,
            (dstrect.w as f32 * scale_x) as i32,
            (dstrect.h as f32 * scale_y) as i32,
        );
        src.blit_scaled(srcrect, surface, Some(&r), scale_mode)
    } else {
        src.blit(srcrect, surface, Some(dstrect))
    }
}

/// A fresh surface over another surface's pixels (`SDL_CreateSurfaceFrom()`
/// on its pixel pointer): same format, none of its blit settings.
fn surface_from<'s>(src: &'s Surface<'_>) -> Result<Surface<'s>> {
    Surface::from_const_pixels(
        src.width(),
        src.height(),
        src.format(),
        Colorspace::UNKNOWN,
        None,
        src.raw_pixels().unwrap_or(&[]),
        src.pitch(),
    )
}

/// Translation of `SW_RenderCopyEx()`.
#[allow(clippy::too_many_arguments)]
fn sw_render_copy_ex(
    surface: &mut Surface<'_>,
    src: &mut Surface<'_>,
    srcrect: &Rect,
    final_rect: &Rect,
    angle: f64,
    center: &FPoint,
    flip: FlipMode,
    scale_x: f32,
    scale_y: f32,
    scale_mode: ScaleMode,
) -> Result<()> {
    let mut tmp_rect = Rect::new(0, 0, final_rect.w, final_rect.h);

    /* It is possible to encounter an RLE encoded surface here and locking it is
     * necessary because this code is going to access the pixel buffer directly.
     */
    let src_locked = src.must_lock();
    if src_locked {
        src.lock_raw()?;
    }

    let result = (|| -> Result<()> {
        /* Clone the source surface but use its pixel buffer directly.
         * The original source surface must be treated as read-only.
         */
        let mut src_clone = surface_from(src)?;
        if let Some(p) = src.palette() {
            src_clone.set_palette(Some(p.clone()))?;
        }

        let blendmode = src.blend_mode();
        let alpha_mod = src.alpha_mod();
        let (r_mod, g_mod, b_mod) = src.color_mod();

        let mut blit_required = false;
        let mut apply_modulation = false;
        let mut is_opaque = false;

        // SDLgfx_rotateSurface only accepts 32-bit surfaces with a 8888 layout. Everything else has to be converted.
        use crate::video::pixels::PackedLayout;
        if !(src.format().bits_per_pixel() == 32
            && src.format().pixel_layout() == PackedLayout::L8888)
        {
            blit_required = true;
        }

        // If scaling and cropping is necessary, it has to be taken care of before the rotation.
        if !(srcrect.w == final_rect.w
            && srcrect.h == final_rect.h
            && srcrect.x == 0
            && srcrect.y == 0)
        {
            blit_required = true;
        }

        // srcrect is not selecting the whole src surface, so cropping is needed
        if !(srcrect.w == src.width()
            && srcrect.h == src.height()
            && srcrect.x == 0
            && srcrect.y == 0)
        {
            blit_required = true;
        }

        // The color and alpha modulation has to be applied before the rotation when using the NONE, MOD or MUL blend modes.
        if (blendmode == BlendMode::NONE
            || blendmode == BlendMode::MOD
            || blendmode == BlendMode::MUL)
            && (alpha_mod & r_mod & g_mod & b_mod) != 255
        {
            apply_modulation = true;
            src_clone.set_alpha_mod(alpha_mod);
            src_clone.set_color_mod(r_mod, g_mod, b_mod);
        }

        // Opaque surfaces are much easier to handle with the NONE blend mode.
        if blendmode == BlendMode::NONE && !src.format().has_alpha() && alpha_mod == 255 {
            is_opaque = true;
        }

        /* The NONE blend mode requires a mask for non-opaque surfaces. This mask will be used
         * to clear the pixels in the destination surface. The other steps are explained below.
         */
        let mut mask = None;
        if blendmode == BlendMode::NONE && !is_opaque {
            let mut m = Surface::new(final_rect.w, final_rect.h, PixelFormat::ARGB8888)?;
            m.set_blend_mode(BlendMode::MOD)?;
            mask = Some(m);
        }

        /* Create a new surface should there be a format mismatch or if scaling, cropping,
         * or modulation is required. It's possible to use the source surface directly otherwise.
         */
        if blit_required || apply_modulation {
            let scale_rect = tmp_rect;
            let mut src_scaled =
                Surface::new_uninitialized(final_rect.w, final_rect.h, PixelFormat::ARGB8888)?;
            src_clone.set_blend_mode(BlendMode::NONE)?;
            let r = src_clone.blit_scaled(
                Some(srcrect),
                &mut src_scaled,
                Some(&scale_rect),
                scale_mode,
            );
            src_clone = src_scaled;
            r?;
        }

        // SDLgfx_rotateSurface is going to make decisions depending on the blend mode.
        let _ = src_clone.set_blend_mode(blendmode);

        let (rect_dest, cangle, sangle) =
            rotozoom_surface_size_trig(tmp_rect.w, tmp_rect.h, angle, center);
        let smooth = !(scale_mode == ScaleMode::Nearest || scale_mode == ScaleMode::PixelArt);
        let flipx = matches!(flip, FlipMode::Horizontal | FlipMode::HorizontalAndVertical);
        let flipy = matches!(flip, FlipMode::Vertical | FlipMode::HorizontalAndVertical);
        let mut src_rotated = rotate_surface(
            &mut src_clone,
            angle,
            smooth,
            flipx,
            flipy,
            &rect_dest,
            cangle,
            sangle,
            center,
        )
        .ok_or_else(|| Error::new("Couldn't rotate surface"))?;

        let mut mask_rotated = None;
        if let Some(mask) = &mut mask {
            // The mask needed for the NONE blend mode gets rotated with the same parameters.
            mask_rotated = Some(
                rotate_surface(
                    mask, angle, false, false, false, &rect_dest, cangle, sangle, center,
                )
                .ok_or_else(|| Error::new("Couldn't rotate surface"))?,
            );
        }

        tmp_rect.x = final_rect.x + rect_dest.x;
        tmp_rect.y = final_rect.y + rect_dest.y;
        tmp_rect.w = rect_dest.w;
        tmp_rect.h = rect_dest.h;

        /* The NONE blend mode needs some special care with non-opaque surfaces.
         * Other blend modes or opaque surfaces can be blitted directly.
         */
        if blendmode != BlendMode::NONE || is_opaque {
            if !apply_modulation {
                // If the modulation wasn't already applied, make it happen now.
                src_rotated.set_alpha_mod(alpha_mod);
                src_rotated.set_color_mod(r_mod, g_mod, b_mod);
            }
            // Renderer scaling, if needed
            blit_to_screen(
                &mut src_rotated,
                None,
                surface,
                &tmp_rect,
                scale_x,
                scale_y,
                scale_mode,
            )
        } else {
            /* The NONE blend mode requires three steps to get the pixels onto the destination surface.
             * First, the area where the rotated pixels will be blitted to get set to zero.
             * This is accomplished by simply blitting a mask with the NONE blend mode.
             * The colorkey set by the rotate function will discard the correct pixels.
             */
            let mut mask_rotated = mask_rotated.expect("mask for the NONE blend mode");
            let mask_rect = tmp_rect;
            mask_rotated.set_blend_mode(BlendMode::NONE)?;
            // Renderer scaling, if needed
            blit_to_screen(
                &mut mask_rotated,
                None,
                surface,
                &mask_rect,
                scale_x,
                scale_y,
                scale_mode,
            )?;

            /* The next step copies the alpha value. This is done with the BLEND blend mode and
             * by modulating the source colors with 0. Since the destination is all zeros, this
             * will effectively set the destination alpha to the source alpha.
             */
            src_rotated.set_color_mod(0, 0, 0);
            let mask_rect = tmp_rect;
            // Renderer scaling, if needed
            blit_to_screen(
                &mut src_rotated,
                None,
                surface,
                &mask_rect,
                scale_x,
                scale_y,
                scale_mode,
            )?;

            /* The last step gets the color values in place. The ADD blend mode simply adds them to
             * the destination (where the color values are all zero). However, because the ADD blend
             * mode modulates the colors with the alpha channel, a surface without an alpha mask needs
             * to be created. This makes all source pixels opaque and the colors get copied correctly.
             */
            let mut src_rotated_rgb = surface_from(&src_rotated)?;
            src_rotated_rgb.set_blend_mode(BlendMode::ADD)?;
            // Renderer scaling, if needed
            blit_to_screen(
                &mut src_rotated_rgb,
                None,
                surface,
                &tmp_rect,
                scale_x,
                scale_y,
                scale_mode,
            )
        }
    })();

    if src_locked {
        src.unlock_raw();
    }
    result
}

impl RenderBackend for SwRenderer {
    fn name(&self) -> &'static str {
        SOFTWARE_RENDERER
    }

    /// Translation of `SW_GetOutputSize()`. Upstream reports the size of
    /// data->surface, which is the render target's surface while one is set,
    /// although SDL_GetRenderOutputSize() is the size of the output ignoring
    /// render targets (and sizes the main view): with the target kept over a
    /// window resize, the main view would take the target's size. Fixed
    /// here: the output's size, whatever the target.
    fn output_size(&self, _textures: &TextureStore) -> Option<Result<(i32, i32)>> {
        if let Some(ws) = &self.window_surface {
            let s = ws.lock();
            return Some(Ok((s.width(), s.height())));
        }
        Some(match self.surface.as_ref() {
            Some(s) => Ok((s.width(), s.height())),
            None => match self.window {
                Some(window) => window.size_in_pixels(),
                None => Err(Error::new(
                    "Software renderer doesn't have an output surface",
                )),
            },
        })
    }

    fn create_texture(
        &mut self,
        texture: &mut TextureData,
        _props: &TextureCreateProps,
    ) -> Result<()> {
        let mut surface = Surface::new_uninitialized(texture.w, texture.h, texture.format)
            .map_err(|_| Error::new("Can't create surface"))?;

        let c = texture.color;
        surface.set_color_mod(unit_to_byte(c.r), unit_to_byte(c.g), unit_to_byte(c.b));
        surface.set_alpha_mod(unit_to_byte(c.a));
        let _ = surface.set_blend_mode(texture.blend_mode);

        if surface.format().is_indexed() {
            let palette = Palette::new(1 << surface.format().bits_per_pixel())
                .map_err(|_| Error::new("Can't create palette"))?;
            surface.set_palette(Some(share_palette(palette)))?;
        }

        if texture.access == TextureAccess::Static {
            let _ = surface.set_rle(true);
        }

        texture.internal = Some(Box::new(surface));
        Ok(())
    }

    fn queue_set_viewport(&mut self, _cmd: &mut RenderCommand) -> Result<()> {
        Ok(()) // nothing to do in this backend.
    }

    fn queue_set_draw_color(&mut self, _cmd: &mut RenderCommand) -> Result<()> {
        Ok(()) // nothing to do in this backend.
    }

    fn queue_draw_points(&mut self, cmd: &mut DrawCmd, points: &[FPoint]) -> Result<()> {
        cmd.first = self.verts.len();
        cmd.count = points.len();
        for p in points {
            self.push(SwVert::Point(Point {
                x: p.x as i32,
                y: p.y as i32,
            }));
        }
        Ok(())
    }

    fn queue_draw_lines(&mut self, cmd: &mut DrawCmd, points: &[FPoint]) -> Option<Result<()>> {
        // lines and points queue vertices the same way.
        Some(self.queue_draw_points(cmd, points))
    }

    fn has_queue_fill_rects(&self) -> bool {
        true
    }

    fn queue_fill_rects(&mut self, cmd: &mut DrawCmd, rects: &[FRect]) -> Result<()> {
        cmd.first = self.verts.len();
        cmd.count = rects.len();
        for r in rects {
            let mut v = Rect::new(r.x as i32, r.y as i32, r.w as i32, r.h as i32);
            if v.w < 0 {
                v.w = -v.w;
                v.x -= v.w;
            }
            if v.h < 0 {
                v.h = -v.h;
                v.y -= v.h;
            }
            self.push(SwVert::Rect(v));
        }
        Ok(())
    }

    fn has_queue_copy(&self) -> bool {
        true
    }

    fn queue_copy(
        &mut self,
        cmd: &mut DrawCmd,
        _texture: &TextureData,
        srcrect: &FRect,
        dstrect: &FRect,
    ) -> Result<()> {
        cmd.first = self.verts.len();
        cmd.count = 1;
        let int_rect = |r: &FRect| Rect::new(r.x as i32, r.y as i32, r.w as i32, r.h as i32);
        self.push(SwVert::Rect(int_rect(srcrect)));
        self.push(SwVert::Rect(int_rect(dstrect)));
        Ok(())
    }

    fn has_queue_copy_ex(&self) -> bool {
        true
    }

    fn queue_copy_ex(
        &mut self,
        cmd: &mut DrawCmd,
        _texture: &TextureData,
        copy: &CopyEx,
    ) -> Result<()> {
        cmd.first = self.verts.len();
        cmd.count = 1;
        let int_rect = |r: &FRect| Rect::new(r.x as i32, r.y as i32, r.w as i32, r.h as i32);
        self.push(SwVert::CopyEx(CopyExData {
            srcrect: int_rect(&copy.srcquad),
            dstrect: int_rect(&copy.dstrect),
            angle: copy.angle,
            center: copy.center,
            flip: copy.flip,
            scale_x: copy.scale_x,
            scale_y: copy.scale_y,
        }));
        Ok(())
    }

    fn has_queue_geometry(&self) -> bool {
        true
    }

    fn queue_geometry(
        &mut self,
        cmd: &mut DrawCmd,
        texture: Option<&TextureData>,
        geometry: &Geometry<'_>,
        scale_x: f32,
        scale_y: f32,
    ) -> Result<()> {
        let count = geometry.count();
        let color_scale = cmd.color_scale;
        cmd.first = self.verts.len();
        cmd.count = count;

        let color = |c: FColor| color_bytes(c, color_scale);
        for i in 0..count {
            let j = geometry.vertex(i);
            let (x, y) = geometry.xy(j);
            let col = geometry.color(j);
            let mut dst = Point {
                x: (x * scale_x) as i32,
                y: (y * scale_y) as i32,
            };
            trianglepoint_2_fixedpoint(&mut dst);
            match texture {
                Some(texture) => {
                    let (u, v) = geometry.uv(j);
                    self.push(SwVert::Copy(GeometryCopyData {
                        src: Point {
                            x: (u * texture.w as f32) as i32,
                            y: (v * texture.h as f32) as i32,
                        },
                        dst,
                        color: color(col),
                    }));
                }
                None => self.push(SwVert::Fill(GeometryFillData {
                    dst,
                    color: color(col),
                })),
            }
        }
        Ok(())
    }

    fn invalidate_cached_state(&mut self) {
        // SW_DrawStateCache only lives during SW_RunCommandQueue, so nothing to do here!
    }

    fn run_command_queue(
        &mut self,
        cmds: &[RenderCommand],
        textures: &mut TextureStore,
    ) -> Result<()> {
        self.with_output(|this| this.run_commands(cmds, textures))
    }

    fn reset_vertices(&mut self) {
        self.verts.clear();
    }

    fn create_palette(&mut self) -> Result<Box<dyn Any>> {
        let surface_palette = Palette::new(256)?;
        Ok(Box::new(share_palette(surface_palette)))
    }

    fn update_palette(&mut self, palette: &mut dyn Any, colors: &[Color]) -> Result<()> {
        let surface_palette = palette
            .downcast_mut::<SharedPalette>()
            .expect("software palette");
        write_palette(surface_palette).set_colors(0, colors)
    }

    fn change_texture_palette(
        &mut self,
        texture: &mut TextureData,
        palette: Option<&dyn Any>,
    ) -> Option<Result<()>> {
        let surface = texture_surface_mut(texture);
        let surface_palette = palette
            .and_then(|p| p.downcast_ref::<SharedPalette>())
            .cloned();
        Some(surface.set_palette(surface_palette))
    }

    fn update_texture(
        &mut self,
        texture: &mut TextureData,
        rect: &Rect,
        pixels: &[u8],
        pitch: usize,
    ) -> Result<()> {
        let surface = texture_surface_mut(texture);
        let locked = surface.must_lock();
        if locked {
            surface.lock_raw()?;
        }

        let bpp = surface.format().bytes_per_pixel() as usize;
        let spitch = surface.pitch() as usize;
        let length = rect.w as usize * bpp;
        let dst = surface
            .pixels
            .bytes_mut()
            .ok_or_else(|| Error::new("Texture pixels are not writable"))?;
        let mut d = rect.y as usize * spitch + rect.x as usize * bpp;
        let mut s = 0;
        for _ in 0..rect.h {
            dst[d..d + length].copy_from_slice(&pixels[s..s + length]);
            s += pitch;
            d += spitch;
        }

        if locked {
            surface.unlock_raw();
        }
        Ok(())
    }

    fn lock_texture(&mut self, texture: &mut TextureData, rect: &Rect) -> Result<(usize, i32)> {
        let surface = texture_surface(texture);
        let offset = rect.y as usize * surface.pitch() as usize
            + rect.x as usize * surface.format().bytes_per_pixel() as usize;
        Ok((offset, surface.pitch()))
    }

    fn texture_pixels_mut<'t>(&mut self, texture: &'t mut TextureData) -> Option<&'t mut [u8]> {
        texture_surface_mut(texture).pixels.bytes_mut()
    }

    fn unlock_texture(&mut self, _texture: &mut TextureData) {}

    fn set_render_target(&mut self, target: Option<Texture>, _: &TextureStore) -> Result<()> {
        self.target = target;
        Ok(())
    }

    fn read_pixels(
        &mut self,
        rect: &Rect,
        textures: &mut TextureStore,
    ) -> Option<Result<Surface<'static>>> {
        self.with_output(|this| this.read_output_pixels(rect, textures))
    }

    fn present(&mut self) -> bool {
        match self.window {
            None => false,
            Some(window) => window.update_surface().is_ok(),
        }
    }

    fn window_event(&mut self, event_type: crate::events::EventType) {
        if event_type == crate::events::EventType::WINDOW_PIXEL_SIZE_CHANGED {
            // Upstream drops data->surface, which is also the render
            // target's surface while one is set, so the target is dropped
            // too: until the app sets a target again, drawing goes to the
            // window. Fixed here: the target is kept.
            self.surface = None;
            self.window_surface = None;
        }
    }

    fn destroy(&mut self) {
        if let Some(window) = self.window {
            let _ = window.destroy_surface();
        }
    }

    fn destroy_texture(&mut self, texture: &mut TextureData) {
        texture.internal = None;
    }

    fn surface(&self) -> Option<&Surface<'static>> {
        self.surface.as_ref()
    }

    fn surface_mut(&mut self) -> Option<&mut Surface<'static>> {
        self.surface.as_mut()
    }

    fn into_surface(self: Box<Self>) -> Option<Surface<'static>> {
        self.surface
    }
}

/// The surface drawn into.
fn active<'s>(
    output: &'s mut Option<Surface<'static>>,
    target: Option<Texture>,
    textures: &'s mut TextureStore,
) -> Result<&'s mut Surface<'static>> {
    match target {
        Some(t) => textures.get_mut(t).map(texture_surface_mut),
        None => output.as_mut(),
    }
    .ok_or_else(|| Error::invalid_param("surface"))
}

/// Run `f` with the surface drawn into and a source texture. A texture
/// drawn into itself is drawn from a copy of its pixels (upstream blits a
/// surface onto itself).
fn with_source(
    output: &mut Option<Surface<'static>>,
    target: Option<Texture>,
    textures: &mut TextureStore,
    source: Texture,
    f: impl FnOnce(&mut Surface<'static>, &mut TextureData),
) -> Result<()> {
    match target {
        None => {
            let surface = output
                .as_mut()
                .ok_or_else(|| Error::invalid_param("surface"))?;
            let tex = textures
                .get_mut(source)
                .ok_or_else(crate::render::sysrender::invalid_texture)?;
            f(surface, tex);
        }
        Some(t) if t == source => {
            let tex = textures
                .get_mut(source)
                .ok_or_else(crate::render::sysrender::invalid_texture)?;
            let copy = texture_surface(tex).duplicate()?;
            let mut target_surface = std::mem::replace(texture_surface_mut(tex), copy);
            f(&mut target_surface, tex);
            *texture_surface_mut(tex) = target_surface;
        }
        Some(t) => {
            let (dst, tex) = textures
                .get_two_mut(t, source)
                .ok_or_else(crate::render::sysrender::invalid_texture)?;
            f(texture_surface_mut(dst), tex);
        }
    }
    Ok(())
}
