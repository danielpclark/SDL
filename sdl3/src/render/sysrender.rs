// Rust translation of src/render/SDL_sysrender.h from Simple DirectMedia Layer.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! The internal structures of the 2D renderer and the interface between
//! the renderer front end and its backends.

use std::any::Any;

use crate::error::{Error, Result};
use crate::properties::Properties;
use crate::render::yuv_sw::SwYuvTexture;
use crate::render::{LogicalPresentation, Texture, TextureAccess, TextureAddressMode};
use crate::video::pixels::{Color, Colorspace, FColor, PixelFormat};
use crate::video::rect::{FPoint, FRect, Rect};
use crate::video::surface::{ScaleMode, SharedPalette, Surface};
use crate::video::{BlendMode, FlipMode};

/// Rendering view state. Translation of `SDL_RenderViewState`.
#[derive(Clone, Copy, Debug)]
pub(crate) struct RenderViewState {
    pub(crate) pixel_w: i32,
    pub(crate) pixel_h: i32,
    pub(crate) viewport: FRect,
    pub(crate) pixel_viewport: Rect,
    pub(crate) clip_rect: FRect,
    pub(crate) pixel_clip_rect: Rect,
    pub(crate) clipping_enabled: bool,
    pub(crate) scale: FPoint,

    // Support for logical output coordinates
    pub(crate) logical_presentation_mode: LogicalPresentation,
    pub(crate) logical_w: i32,
    pub(crate) logical_h: i32,
    pub(crate) logical_src_rect: FRect,
    pub(crate) logical_dst_rect: FRect,
    pub(crate) logical_scale: FPoint,
    pub(crate) logical_offset: FPoint,
    /// this is just `scale * logical_scale`, precalculated, since we use it a lot.
    pub(crate) current_scale: FPoint,
}

impl RenderViewState {
    /// A view of `pixel_w` x `pixel_h` pixels with the default viewport
    /// (the whole output) and unit scales.
    pub(crate) fn new(pixel_w: i32, pixel_h: i32) -> RenderViewState {
        let one = FPoint { x: 1.0, y: 1.0 };
        RenderViewState {
            pixel_w,
            pixel_h,
            viewport: FRect {
                x: 0.0,
                y: 0.0,
                w: -1.0,
                h: -1.0,
            },
            pixel_viewport: Rect::default(),
            clip_rect: FRect::default(),
            pixel_clip_rect: Rect::default(),
            clipping_enabled: false,
            scale: one,
            logical_presentation_mode: LogicalPresentation::Disabled,
            logical_w: 0,
            logical_h: 0,
            logical_src_rect: FRect::default(),
            logical_dst_rect: FRect::default(),
            logical_scale: one,
            logical_offset: FPoint::default(),
            current_scale: one,
        }
    }
}

/// A palette shared by the textures using the same public palette.
/// Translation of `SDL_TexturePalette`.
pub(crate) struct TexturePalette {
    pub(crate) refcount: i32,
    pub(crate) version: u32,
    /// last command queue generation this palette was in.
    pub(crate) last_command_generation: u32,
    /// Driver specific palette representation
    pub(crate) internal: Box<dyn Any>,
}

/// The state of a texture. Translation of `struct SDL_Texture`.
pub(crate) struct TextureData {
    pub(crate) format: PixelFormat,
    pub(crate) w: i32,
    pub(crate) h: i32,

    /// The colorspace of the texture
    pub(crate) colorspace: Colorspace,
    /// The SDR white point for this content
    pub(crate) sdr_white_point: f32,
    /// The HDR headroom needed by this content
    pub(crate) hdr_headroom: f32,
    /// The texture access mode
    pub(crate) access: TextureAccess,
    /// The texture blend mode
    pub(crate) blend_mode: BlendMode,
    /// The texture scale mode
    pub(crate) scale_mode: ScaleMode,
    /// Texture modulation values
    pub(crate) color: FColor,
    /// Target texture view state
    pub(crate) view: RenderViewState,

    pub(crate) public_palette: Option<SharedPalette>,
    /// The key of the shared palette in the renderer's palette cache.
    pub(crate) palette: Option<usize>,
    pub(crate) palette_version: u32,
    pub(crate) palette_surface: Option<Surface<'static>>,

    // Support for formats not supported directly by the renderer
    pub(crate) native: Option<Texture>,
    /// The texture this one is the native texture of
    /// (`SDL_PROP_TEXTURE_PARENT_POINTER`).
    pub(crate) parent: Option<Texture>,
    pub(crate) yuv: Option<SwYuvTexture>,
    pub(crate) pixels: Vec<u8>,
    pub(crate) pitch: i32,
    pub(crate) locked_rect: Rect,

    /// last command queue generation this texture was in.
    pub(crate) last_command_generation: u32,
    pub(crate) props: Option<Properties>,
    /// Driver specific texture representation
    pub(crate) internal: Option<Box<dyn Any>>,
}

/// The textures of a renderer: slots addressed by [`Texture`] handles, with
/// generations so that a stale handle is reported as invalid.
#[derive(Default)]
pub(crate) struct TextureStore {
    slots: Vec<(u32, Option<TextureData>)>,
}

impl TextureStore {
    pub(crate) fn insert(&mut self, renderer: u32, data: TextureData) -> Texture {
        if let Some(index) = self.slots.iter().position(|(_, d)| d.is_none()) {
            let slot = &mut self.slots[index];
            slot.0 += 1;
            slot.1 = Some(data);
            return Texture {
                renderer,
                index: index as u32,
                generation: slot.0,
            };
        }
        self.slots.push((1, Some(data)));
        Texture {
            renderer,
            index: self.slots.len() as u32 - 1,
            generation: 1,
        }
    }

    pub(crate) fn get(&self, t: Texture) -> Option<&TextureData> {
        match self.slots.get(t.index as usize) {
            Some((generation, Some(d))) if *generation == t.generation => Some(d),
            _ => None,
        }
    }

    pub(crate) fn get_mut(&mut self, t: Texture) -> Option<&mut TextureData> {
        match self.slots.get_mut(t.index as usize) {
            Some((generation, Some(d))) if *generation == t.generation => Some(d),
            _ => None,
        }
    }

    /// Two distinct textures at once.
    pub(crate) fn get_two_mut(
        &mut self,
        a: Texture,
        b: Texture,
    ) -> Option<(&mut TextureData, &mut TextureData)> {
        if a.index == b.index {
            return None;
        }
        let (ia, ib) = (a.index as usize, b.index as usize);
        let (lo, hi) = if ia < ib { (ia, ib) } else { (ib, ia) };
        if hi >= self.slots.len() {
            return None;
        }
        let (left, right) = self.slots.split_at_mut(hi);
        let (slot_lo, slot_hi) = (&mut left[lo], &mut right[0]);
        let (slot_a, slot_b) = if ia < ib {
            (slot_lo, slot_hi)
        } else {
            (slot_hi, slot_lo)
        };
        match (slot_a, slot_b) {
            ((ga, Some(da)), (gb, Some(db))) if *ga == a.generation && *gb == b.generation => {
                Some((da, db))
            }
            _ => None,
        }
    }

    pub(crate) fn remove(&mut self, t: Texture) -> Option<TextureData> {
        match self.slots.get_mut(t.index as usize) {
            Some((generation, d)) if *generation == t.generation => d.take(),
            _ => None,
        }
    }

    /// The handles of all live textures.
    pub(crate) fn handles(&self, renderer: u32) -> Vec<Texture> {
        self.slots
            .iter()
            .enumerate()
            .filter(|(_, (_, d))| d.is_some())
            .map(|(index, (generation, _))| Texture {
                renderer,
                index: index as u32,
                generation: *generation,
            })
            .collect()
    }
}

/// The draw state of a queued draw command (`data.draw` of `SDL_RenderCommand`).
#[derive(Clone, Copy, Debug)]
pub(crate) struct DrawCmd {
    /// where the backend's vertex data for this command starts
    pub(crate) first: usize,
    pub(crate) count: usize,
    pub(crate) color_scale: f32,
    #[allow(dead_code)] // (the software backend uses the queued draw color)
    pub(crate) color: FColor,
    pub(crate) blend: BlendMode,
    pub(crate) texture: Option<Texture>,
    pub(crate) texture_scale_mode: ScaleMode,
    pub(crate) texture_address_mode_u: TextureAddressMode,
    pub(crate) texture_address_mode_v: TextureAddressMode,
}

/// The kinds of draw commands.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum DrawKind {
    Points,
    Lines,
    FillRects,
    Copy,
    CopyEx,
    Geometry,
}

/// A queued render command. Translation of `SDL_RenderCommand` (its type
/// and data union).
#[derive(Clone, Copy, Debug)]
#[allow(dead_code)] // (`first`: the software backend queues no vertices for these)
pub(crate) enum RenderCommand {
    NoOp,
    SetViewport {
        first: usize,
        rect: Rect,
    },
    SetClipRect {
        enabled: bool,
        rect: Rect,
    },
    SetDrawColor {
        first: usize,
        color_scale: f32,
        color: FColor,
    },
    Clear {
        first: usize,
        color_scale: f32,
        color: FColor,
    },
    Draw(DrawKind, DrawCmd),
}

/// The method of drawing lines. Translation of `SDL_RenderLineMethod`.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum RenderLineMethod {
    Points,
    Lines,
    Geometry,
}

/// The indices of a geometry draw.
#[derive(Clone, Copy, Debug)]
pub enum Indices<'a> {
    /// 8-bit indices
    U8(&'a [u8]),
    /// 16-bit indices
    U16(&'a [u16]),
    /// 32-bit indices
    U32(&'a [u32]),
    /// 32-bit signed indices
    I32(&'a [i32]),
}

impl Indices<'_> {
    pub(crate) fn len(&self) -> usize {
        match self {
            Indices::U8(i) => i.len(),
            Indices::U16(i) => i.len(),
            Indices::U32(i) => i.len(),
            Indices::I32(i) => i.len(),
        }
    }

    /// The index at `i`, read as the C code does (`int` from the element type).
    pub(crate) fn get(&self, i: usize) -> i32 {
        match self {
            Indices::U8(v) => v[i] as i32,
            Indices::U16(v) => v[i] as i32,
            Indices::U32(v) => v[i] as i32,
            Indices::I32(v) => v[i],
        }
    }
}

/// The vertices of a geometry draw, as `SDL_RenderGeometryRaw()` takes
/// them: positions and texture coordinates are pairs of floats, colors
/// four floats (an `SDL_FColor`), each `*_stride` floats apart (the C
/// byte strides divided by four, so interleaved [`Vertex`] data can be
/// described as upstream describes it). A color stride of 0 is one color
/// for all vertices, and `uv` is empty when there are no texture
/// coordinates.
///
/// [`Vertex`]: crate::render::Vertex
#[derive(Clone, Copy, Debug)]
pub(crate) struct Geometry<'a> {
    pub(crate) xy: &'a [f32],
    pub(crate) xy_stride: usize,
    pub(crate) color: &'a [f32],
    pub(crate) color_stride: usize,
    pub(crate) uv: &'a [f32],
    pub(crate) uv_stride: usize,
    pub(crate) num_vertices: usize,
    pub(crate) indices: Option<Indices<'a>>,
}

impl Geometry<'_> {
    pub(crate) fn xy(&self, j: usize) -> (f32, f32) {
        (self.xy[j * self.xy_stride], self.xy[j * self.xy_stride + 1])
    }

    pub(crate) fn color(&self, j: usize) -> FColor {
        let c = &self.color[j * self.color_stride..];
        FColor::new(c[0], c[1], c[2], c[3])
    }

    pub(crate) fn uv(&self, j: usize) -> (f32, f32) {
        (self.uv[j * self.uv_stride], self.uv[j * self.uv_stride + 1])
    }

    /// The number of vertices drawn.
    pub(crate) fn count(&self) -> usize {
        self.indices.map_or(self.num_vertices, |i| i.len())
    }

    /// The vertex of draw position `i`.
    pub(crate) fn vertex(&self, i: usize) -> usize {
        self.indices.map_or(i, |ind| ind.get(i) as usize)
    }
}

/// The data a copy-ex draw queues (`SDL_RenderTextureRotated()`).
#[derive(Clone, Copy, Debug)]
pub(crate) struct CopyEx {
    pub(crate) srcquad: FRect,
    pub(crate) dstrect: FRect,
    pub(crate) angle: f64,
    pub(crate) center: FPoint,
    pub(crate) flip: FlipMode,
    pub(crate) scale_x: f32,
    pub(crate) scale_y: f32,
}

/// The functions a render backend implements: the function pointers of
/// `struct SDL_Renderer`.
///
/// The queue functions record the data a draw command needs in the
/// backend's own vertex storage and set the command's `first` and `count`;
/// [`RenderBackend::run_command_queue`] then executes the queue and
/// [`RenderBackend::reset_vertices`] empties the storage (upstream resets
/// `vertex_data_used`).
pub(crate) trait RenderBackend {
    /// The backend's name (`renderer->name`).
    fn name(&self) -> &'static str;

    /// `GetOutputSize`; `None` when the backend has no output size of its own.
    fn output_size(&self, textures: &TextureStore) -> Option<Result<(i32, i32)>>;

    /// The texture formats the backend supports, best first (its
    /// `SDL_AddSupportedTextureFormat()` calls); `None` for the software
    /// backend, whose formats follow its output (`SW_SelectBestFormats()`).
    fn texture_formats(&self) -> Option<Vec<PixelFormat>> {
        None
    }

    /// `renderer->npot_texture_wrap_unsupported`, as the backend sets it.
    fn npot_texture_wrap_unsupported(&self) -> bool {
        false
    }

    /// `SDL_PROP_RENDERER_MAX_TEXTURE_SIZE_NUMBER`, if the backend sets it.
    fn max_texture_size(&self) -> Option<i32> {
        None
    }

    /// Called once the renderer's properties exist, for the backend to
    /// set its own (`SDL_GetRendererProperties()` in `CreateRenderer`) and
    /// keep for later updates.
    fn set_properties(&mut self, _props: &Properties) {}

    /// `SupportsBlendMode`, for custom blend modes.
    fn supports_blend_mode(&self, _mode: BlendMode) -> bool {
        false
    }

    fn create_texture(
        &mut self,
        texture: &mut TextureData,
        props: &TextureCreateProps,
    ) -> Result<()>;

    fn queue_set_viewport(&mut self, cmd: &mut RenderCommand) -> Result<()>;
    fn queue_set_draw_color(&mut self, cmd: &mut RenderCommand) -> Result<()>;
    fn queue_draw_points(&mut self, cmd: &mut DrawCmd, points: &[FPoint]) -> Result<()>;
    /// `None` when the backend draws lines with geometry.
    fn queue_draw_lines(&mut self, cmd: &mut DrawCmd, points: &[FPoint]) -> Option<Result<()>>;
    /// Whether `queue_fill_rects` is implemented (otherwise geometry is used).
    fn has_queue_fill_rects(&self) -> bool;
    fn queue_fill_rects(&mut self, cmd: &mut DrawCmd, rects: &[FRect]) -> Result<()>;
    /// Whether `queue_copy` is implemented (otherwise geometry is used).
    fn has_queue_copy(&self) -> bool;
    fn queue_copy(
        &mut self,
        cmd: &mut DrawCmd,
        texture: &TextureData,
        srcrect: &FRect,
        dstrect: &FRect,
    ) -> Result<()>;
    /// Whether `queue_copy_ex` is implemented (otherwise geometry is used).
    fn has_queue_copy_ex(&self) -> bool;
    fn queue_copy_ex(
        &mut self,
        cmd: &mut DrawCmd,
        texture: &TextureData,
        copy: &CopyEx,
    ) -> Result<()>;
    /// Whether `queue_geometry` is implemented.
    fn has_queue_geometry(&self) -> bool;
    fn queue_geometry(
        &mut self,
        cmd: &mut DrawCmd,
        texture: Option<&TextureData>,
        geometry: &Geometry<'_>,
        scale_x: f32,
        scale_y: f32,
    ) -> Result<()>;

    fn invalidate_cached_state(&mut self);
    fn run_command_queue(
        &mut self,
        cmds: &[RenderCommand],
        textures: &mut TextureStore,
    ) -> Result<()>;
    fn reset_vertices(&mut self);

    fn create_palette(&mut self) -> Result<Box<dyn Any>>;
    fn update_palette(&mut self, palette: &mut dyn Any, colors: &[Color]) -> Result<()>;
    fn destroy_palette(&mut self, _palette: Box<dyn Any>) {}
    /// `ChangeTexturePalette`; `None` when not implemented.
    fn change_texture_palette(
        &mut self,
        texture: &mut TextureData,
        palette: Option<&dyn Any>,
    ) -> Option<Result<()>>;

    fn update_texture(
        &mut self,
        texture: &mut TextureData,
        rect: &Rect,
        pixels: &[u8],
        pitch: usize,
    ) -> Result<()>;
    /// `UpdateTextureYUV`; `None` when not implemented.
    #[allow(clippy::too_many_arguments)]
    fn update_texture_yuv(
        &mut self,
        _texture: &mut TextureData,
        _rect: &Rect,
        _y: (&[u8], usize),
        _u: (&[u8], usize),
        _v: (&[u8], usize),
    ) -> Option<Result<()>> {
        None
    }
    /// `UpdateTextureNV`; `None` when not implemented.
    fn update_texture_nv(
        &mut self,
        _texture: &mut TextureData,
        _rect: &Rect,
        _y: (&[u8], usize),
        _uv: (&[u8], usize),
    ) -> Option<Result<()>> {
        None
    }
    /// `LockTexture`: the byte offset of `rect` in the texture's pixels
    /// and the pitch.
    fn lock_texture(&mut self, texture: &mut TextureData, rect: &Rect) -> Result<(usize, i32)>;
    /// The pixels a lock refers to.
    fn texture_pixels_mut<'t>(&mut self, texture: &'t mut TextureData) -> Option<&'t mut [u8]>;
    fn unlock_texture(&mut self, texture: &mut TextureData);
    /// `SetRenderTarget`, with the textures to find the target's data in.
    fn set_render_target(&mut self, target: Option<Texture>, textures: &TextureStore)
        -> Result<()>;
    /// `RenderReadPixels`; `None` when not implemented.
    fn read_pixels(
        &mut self,
        rect: &Rect,
        textures: &mut TextureStore,
    ) -> Option<Result<Surface<'static>>>;
    /// `RenderPresent`: whether anything was presented.
    fn present(&mut self) -> bool;
    fn destroy_texture(&mut self, texture: &mut TextureData);
    /// `SetVSync`; `None` when not implemented.
    fn set_vsync(&mut self, _vsync: i32) -> Option<Result<()>> {
        None
    }
    /// `AddVulkanRenderSemaphores`; `None` when not implemented.
    fn add_vulkan_render_semaphores(
        &mut self,
        _wait_stage_mask: u32,
        _wait_semaphore: i64,
        _signal_semaphore: i64,
    ) -> Option<Result<()>> {
        None
    }
    /// `WindowEvent`: a window event for the renderer's window.
    fn window_event(&mut self, _event_type: crate::events::EventType) {}
    /// `DestroyRenderer`: clean up renderer-specific resources.
    fn destroy(&mut self) {}

    /// The output surface of a software backend, if it has one.
    fn surface(&self) -> Option<&Surface<'static>> {
        None
    }
    /// The output surface of a software backend, if it has one.
    fn surface_mut(&mut self) -> Option<&mut Surface<'static>> {
        None
    }
    /// Give up the output surface of a software backend.
    fn into_surface(self: Box<Self>) -> Option<Surface<'static>> {
        None
    }
}

/// The creation properties a backend sees (`create_props` of `CreateTexture`).
#[derive(Clone, Debug, Default)]
pub(crate) struct TextureCreateProps {
    #[allow(dead_code)] // (the software backend has no colorspaces)
    pub(crate) colorspace: Option<Colorspace>,
    /// `SDL_PROP_TEXTURE_CREATE_OPENGL_TEXTURE_NUMBER`
    pub(crate) opengl_texture: Option<u32>,
    /// `SDL_PROP_TEXTURE_CREATE_OPENGL_TEXTURE_UV_NUMBER`
    pub(crate) opengl_texture_uv: Option<u32>,
    /// `SDL_PROP_TEXTURE_CREATE_OPENGL_TEXTURE_U_NUMBER`
    pub(crate) opengl_texture_u: Option<u32>,
    /// `SDL_PROP_TEXTURE_CREATE_OPENGL_TEXTURE_V_NUMBER`
    pub(crate) opengl_texture_v: Option<u32>,
}

/// An error for a texture handle that is not (or no longer) valid.
pub(crate) fn invalid_texture() -> Error {
    Error::invalid_param("texture")
}
