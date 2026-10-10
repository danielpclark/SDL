// Rust translation of src/render/SDL_render.c and include/SDL3/SDL_render.h
// from Simple DirectMedia Layer.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! The 2D rendering system.
//!
//! A [`Renderer`] queues drawing commands and runs them through a backend
//! when it flushes (before reading pixels, presenting, or changing state a
//! queued command depends on). Textures belong to the renderer that created
//! them and are addressed with [`Texture`] handles; a handle stays valid
//! until [`Renderer::destroy_texture`] or the renderer is dropped, after
//! which the renderer reports it as an invalid parameter (as upstream's
//! object validity checks do).
//!
//! Renderers use the software backend, drawing into a [`Surface`]
//! ([`Renderer::software`]) or a window's surface, or for windows the
//! Direct3D 11 backend ("direct3d11", Windows only, tried first by
//! [`Renderer::for_window`]), the Direct3D 12 backend ("direct3d12",
//! Windows only, next), the OpenGL backend ("opengl", first
//! elsewhere), the OpenGL ES 2.0 backend ("opengles2"), the Vulkan backend
//! ("vulkan") or the GPU backend ("gpu", through the [GPU API](crate::gpu));
//! the other GPU backends come with the platform layer.
//!
//! A window renderer applies the window's changes (size, visibility, HDR
//! state) at the start of its next drawing, presenting or state-setting
//! call, so the plain getters report the state as of the last such call.
//! Once the window is destroyed, the renderer's calls fail; it can be
//! dropped before or after its window.

mod debug_font;
#[cfg(windows)]
pub(crate) mod direct3d11;
#[cfg(windows)]
pub(crate) mod direct3d12;
pub(crate) mod gpu;
mod gpu_render_state;
pub(crate) mod opengl;
pub(crate) mod opengles2;
pub(crate) mod software;
pub(crate) mod sysrender;
mod texture;
pub(crate) mod vulkan;
mod window;
pub(crate) mod yuv_sw;

use std::collections::HashMap;
use std::sync::atomic::{AtomicU32, Ordering};

use crate::error::{Error, Result};
use crate::hints;
use crate::properties::Properties;
use crate::video::pixels::{
    convert_color_709_to_2020, srgb_to_linear, Colorspace, FColor, PixelFormat,
    TransferCharacteristics,
};
use crate::video::rect::{FPoint, FRect, Point, Rect};
use crate::video::surface::{ScaleMode, Surface};
use crate::video::BlendMode;

#[cfg(windows)]
pub use direct3d11::{
    PROP_RENDERER_D3D11_DEVICE_POINTER, PROP_RENDERER_D3D11_SWAPCHAIN_POINTER,
    PROP_TEXTURE_D3D11_TEXTURE_POINTER, PROP_TEXTURE_D3D11_TEXTURE_U_POINTER,
    PROP_TEXTURE_D3D11_TEXTURE_V_POINTER,
};
#[cfg(windows)]
pub use direct3d12::{
    PROP_RENDERER_D3D12_COMMAND_QUEUE_POINTER, PROP_RENDERER_D3D12_DEVICE_POINTER,
    PROP_RENDERER_D3D12_SWAPCHAIN_POINTER, PROP_TEXTURE_D3D12_TEXTURE_POINTER,
    PROP_TEXTURE_D3D12_TEXTURE_U_POINTER, PROP_TEXTURE_D3D12_TEXTURE_V_POINTER,
};
pub use gpu::{
    PROP_RENDERER_GPU_DEVICE_POINTER, PROP_TEXTURE_GPU_TEXTURE_POINTER,
    PROP_TEXTURE_GPU_TEXTURE_UV_POINTER, PROP_TEXTURE_GPU_TEXTURE_U_POINTER,
    PROP_TEXTURE_GPU_TEXTURE_V_POINTER,
};
pub use gpu_render_state::{
    GpuRenderState, GpuRenderStateCreateInfo, GpuRenderStateSamplerBinding,
};
pub use opengl::{
    PROP_TEXTURE_OPENGL_TEXTURE_NUMBER, PROP_TEXTURE_OPENGL_TEXTURE_TARGET_NUMBER,
    PROP_TEXTURE_OPENGL_TEXTURE_UV_NUMBER, PROP_TEXTURE_OPENGL_TEXTURE_U_NUMBER,
    PROP_TEXTURE_OPENGL_TEXTURE_V_NUMBER, PROP_TEXTURE_OPENGL_TEX_H_FLOAT,
    PROP_TEXTURE_OPENGL_TEX_W_FLOAT,
};
pub use opengles2::{
    PROP_TEXTURE_OPENGLES2_TEXTURE_NUMBER, PROP_TEXTURE_OPENGLES2_TEXTURE_TARGET_NUMBER,
    PROP_TEXTURE_OPENGLES2_TEXTURE_UV_NUMBER, PROP_TEXTURE_OPENGLES2_TEXTURE_U_NUMBER,
    PROP_TEXTURE_OPENGLES2_TEXTURE_V_NUMBER,
};
pub use software::render_sw::SOFTWARE_RENDERER;
pub use sysrender::Indices;
pub use texture::{TextureCreateInfo, TextureLock, TextureSurfaceLock};
pub use vulkan::{
    PROP_RENDERER_VULKAN_DEVICE_POINTER, PROP_RENDERER_VULKAN_GRAPHICS_QUEUE_FAMILY_INDEX_NUMBER,
    PROP_RENDERER_VULKAN_INSTANCE_POINTER, PROP_RENDERER_VULKAN_PHYSICAL_DEVICE_POINTER,
    PROP_RENDERER_VULKAN_PRESENT_QUEUE_FAMILY_INDEX_NUMBER, PROP_RENDERER_VULKAN_SURFACE_NUMBER,
    PROP_RENDERER_VULKAN_SWAPCHAIN_IMAGE_COUNT_NUMBER, PROP_TEXTURE_VULKAN_TEXTURE_NUMBER,
    PROP_TEXTURE_VULKAN_TEXTURE_U_NUMBER, PROP_TEXTURE_VULKAN_TEXTURE_V_NUMBER,
};
pub use window::create_window_and_renderer;
pub(crate) use window::{destroy_window_renderer, quit_render};

use software::render_sw::SwRenderer;
use sysrender::{
    CopyEx, DrawCmd, DrawKind, Geometry, GpuRenderStates, RenderBackend, RenderCommand,
    RenderLineMethod, RenderViewState, TexturePalette, TextureStore,
};

/// The name of the GPU renderer. Translation of `SDL_GPU_RENDERER`.
pub const GPU_RENDERER: &str = "gpu";

/// The size, in pixels, of a single [`Renderer::render_debug_text`]
/// character. Translation of `SDL_DEBUG_TEXT_FONT_CHARACTER_SIZE`.
pub const DEBUG_TEXT_FONT_CHARACTER_SIZE: i32 = 8;

/// Translation of `SDL_PROP_RENDERER_NAME_STRING`.
pub const PROP_RENDERER_NAME_STRING: &str = "SDL.renderer.name";
/// The window of a window renderer (an `Any` [`Window`](crate::video::Window)).
/// Translation of `SDL_PROP_RENDERER_WINDOW_POINTER`.
pub const PROP_RENDERER_WINDOW_POINTER: &str = "SDL.renderer.window";
/// Translation of `SDL_PROP_RENDERER_VSYNC_NUMBER`.
pub const PROP_RENDERER_VSYNC_NUMBER: &str = "SDL.renderer.vsync";
/// Translation of `SDL_PROP_RENDERER_MAX_TEXTURE_SIZE_NUMBER`.
pub const PROP_RENDERER_MAX_TEXTURE_SIZE_NUMBER: &str = "SDL.renderer.max_texture_size";
/// Translation of `SDL_PROP_RENDERER_TEXTURE_WRAPPING_BOOLEAN`.
pub const PROP_RENDERER_TEXTURE_WRAPPING_BOOLEAN: &str = "SDL.renderer.texture_wrapping";
/// Translation of `SDL_PROP_RENDERER_OUTPUT_COLORSPACE_NUMBER`.
pub const PROP_RENDERER_OUTPUT_COLORSPACE_NUMBER: &str = "SDL.renderer.output_colorspace";
/// Translation of `SDL_PROP_RENDERER_HDR_ENABLED_BOOLEAN`.
pub const PROP_RENDERER_HDR_ENABLED_BOOLEAN: &str = "SDL.renderer.HDR_enabled";
/// Translation of `SDL_PROP_RENDERER_SDR_WHITE_POINT_FLOAT`.
pub const PROP_RENDERER_SDR_WHITE_POINT_FLOAT: &str = "SDL.renderer.SDR_white_point";
/// Translation of `SDL_PROP_RENDERER_HDR_HEADROOM_FLOAT`.
pub const PROP_RENDERER_HDR_HEADROOM_FLOAT: &str = "SDL.renderer.HDR_headroom";
/// Translation of `SDL_PROP_TEXTURE_COLORSPACE_NUMBER`.
pub const PROP_TEXTURE_COLORSPACE_NUMBER: &str = "SDL.texture.colorspace";
/// Translation of `SDL_PROP_TEXTURE_FORMAT_NUMBER`.
pub const PROP_TEXTURE_FORMAT_NUMBER: &str = "SDL.texture.format";
/// Translation of `SDL_PROP_TEXTURE_ACCESS_NUMBER`.
pub const PROP_TEXTURE_ACCESS_NUMBER: &str = "SDL.texture.access";
/// Translation of `SDL_PROP_TEXTURE_WIDTH_NUMBER`.
pub const PROP_TEXTURE_WIDTH_NUMBER: &str = "SDL.texture.width";
/// Translation of `SDL_PROP_TEXTURE_HEIGHT_NUMBER`.
pub const PROP_TEXTURE_HEIGHT_NUMBER: &str = "SDL.texture.height";
/// Translation of `SDL_PROP_TEXTURE_SDR_WHITE_POINT_FLOAT`.
pub const PROP_TEXTURE_SDR_WHITE_POINT_FLOAT: &str = "SDL.texture.SDR_white_point";
/// Translation of `SDL_PROP_TEXTURE_HDR_HEADROOM_FLOAT`.
pub const PROP_TEXTURE_HDR_HEADROOM_FLOAT: &str = "SDL.texture.HDR_headroom";

/// How texture coordinates outside [0, 1] are handled.
/// Translation of `SDL_TextureAddressMode`.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Default)]
pub enum TextureAddressMode {
    /// Not a valid mode
    Invalid = -1,
    /// Wrapping is enabled if texture coordinates are outside [0, 1], this is the default
    #[default]
    Auto,
    /// Texture coordinates are clamped to the [0, 1] range
    Clamp,
    /// The texture is repeated (tiled)
    Wrap,
}

/// The access pattern allowed for a texture. Translation of `SDL_TextureAccess`.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Default)]
pub enum TextureAccess {
    /// Changes rarely, not lockable
    #[default]
    Static,
    /// Changes frequently, lockable
    Streaming,
    /// Texture can be used as a render target
    Target,
}

/// How the logical size is mapped to the output.
/// Translation of `SDL_RendererLogicalPresentation`.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Default)]
pub enum LogicalPresentation {
    /// There is no logical size in effect
    #[default]
    Disabled,
    /// The rendered content is stretched to the output resolution
    Stretch,
    /// The rendered content is fit to the largest dimension and the other dimension is letterboxed with the clear color
    Letterbox,
    /// The rendered content is fit to the smallest dimension and the other dimension extends beyond the output bounds
    Overscan,
    /// The rendered content is scaled up by integer multiples to fit the output resolution
    IntegerScale,
}

/// Vertex structure. Translation of `SDL_Vertex`.
#[derive(Clone, Copy, PartialEq, Debug, Default)]
pub struct Vertex {
    /// Vertex position, in renderer coordinates
    pub position: FPoint,
    /// Vertex color
    pub color: FColor,
    /// Normalized texture coordinates, if needed
    pub tex_coord: FPoint,
}

/// A texture of a [`Renderer`]: a handle, valid until the texture is
/// destroyed. Translation of `SDL_Texture *`.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct Texture {
    pub(crate) renderer: u32,
    pub(crate) index: u32,
    pub(crate) generation: u32,
}

/// Options for creating a renderer (the `SDL_PROP_RENDERER_CREATE_*`
/// properties).
#[derive(Clone, Debug, Default)]
pub struct RendererCreateInfo {
    /// The name of the rendering driver (`SDL_PROP_RENDERER_CREATE_NAME_STRING`)
    pub name: Option<String>,
    /// The output colorspace (`SDL_PROP_RENDERER_CREATE_OUTPUT_COLORSPACE_NUMBER`), sRGB by default
    pub output_colorspace: Option<Colorspace>,
    /// The vsync interval (`SDL_PROP_RENDERER_CREATE_PRESENT_VSYNC_NUMBER`)
    pub present_vsync: i32,
    /// The GPU device for the GPU renderer to use instead of creating one
    /// (`SDL_PROP_RENDERER_CREATE_GPU_DEVICE_POINTER`); the renderer
    /// shares it with the application
    pub gpu_device: Option<crate::gpu::Device>,
    /// The application can give the GPU renderer's render states SPIR-V
    /// shaders (`SDL_PROP_RENDERER_CREATE_GPU_SHADERS_SPIRV_BOOLEAN`)
    pub gpu_shaders_spirv: bool,
    /// The application can give the GPU renderer's render states DXIL
    /// shaders (`SDL_PROP_RENDERER_CREATE_GPU_SHADERS_DXIL_BOOLEAN`)
    pub gpu_shaders_dxil: bool,
    /// The application can give the GPU renderer's render states MSL
    /// shaders (`SDL_PROP_RENDERER_CREATE_GPU_SHADERS_MSL_BOOLEAN`)
    pub gpu_shaders_msl: bool,
}

/// The rendering drivers compiled in, in order of preference.
const RENDER_DRIVERS: &[&str] = &[
    #[cfg(windows)]
    direct3d11::D3D11_RENDERER,
    #[cfg(windows)]
    direct3d12::D3D12_RENDERER,
    opengl::OPENGL_RENDERER,
    opengles2::GLES2_RENDERER,
    vulkan::VULKAN_RENDERER,
    GPU_RENDERER,
    SOFTWARE_RENDERER,
];

/// The number of 2D rendering drivers available.
/// Translation of `SDL_GetNumRenderDrivers()`.
pub fn num_render_drivers() -> usize {
    RENDER_DRIVERS.len()
}

/// The name of a built in 2D rendering driver. Translation of `SDL_GetRenderDriver()`.
pub fn render_driver(index: usize) -> Result<&'static str> {
    RENDER_DRIVERS
        .get(index)
        .copied()
        .ok_or_else(|| Error::invalid_param("index"))
}

/// The vsync interval to create a renderer with: the request, or the
/// [`hints::RENDER_VSYNC`] hint if set.
fn present_vsync_with_hint(info: &RendererCreateInfo) -> i64 {
    let mut present_vsync = info.present_vsync as i64;
    if let Some(hint) = hints::get(hints::RENDER_VSYNC) {
        if !hint.is_empty() {
            present_vsync = hints::get_bool(hints::RENDER_VSYNC, true) as i64;
        }
    }
    present_vsync
}

/// A color as the four floats of an `SDL_FColor`.
fn fcolor_floats(c: FColor) -> [f32; 4] {
    [c.r, c.g, c.b, c.a]
}

/// rect_index_order: the two triangles of a quad.
const RECT_INDEX_ORDER: [i32; 6] = [0, 1, 2, 0, 2, 3];

static NEXT_RENDERER_ID: AtomicU32 = AtomicU32::new(1);

/// A 2D rendering context. Translation of `SDL_Renderer`.
pub struct Renderer {
    id: u32,
    pub(crate) backend: Box<dyn RenderBackend>,

    // The current renderer info
    texture_formats: Vec<PixelFormat>,
    software: bool,
    npot_texture_wrap_unsupported: bool,

    hidden: bool,

    // Whether we should simulate vsync
    wanted_vsync: bool,
    simulate_vsync: bool,
    simulate_vsync_interval_ns: u64,
    last_present: u64,

    main_view: RenderViewState,

    /// The window pixel to point coordinate scale
    dpi_scale: FPoint,

    /// The method of drawing lines
    line_method: RenderLineMethod,

    /// Default scale mode for textures created with this renderer
    scale_mode: ScaleMode,

    // The list of textures
    pub(crate) textures: TextureStore,
    /// The GPU render states made for the renderer
    /// (`SDL_CreateGPURenderState()`).
    gpu_render_states: GpuRenderStates,
    /// The GPU render state of the next draws (`gpu_render_state`).
    gpu_render_state: Option<GpuRenderState>,
    /// The render target (a native texture); its view is the current view.
    target: Option<Texture>,

    /// The list of palettes, by public palette
    palettes: HashMap<usize, TexturePalette>,

    current_colorspace: Colorspace,
    output_colorspace: Colorspace,
    sdr_white_point: f32,
    hdr_headroom: f32,
    desired_color_scale: f32,
    color_scale: f32,
    /// Color for drawing operations values
    color: FColor,
    /// The drawing blend mode
    blend_mode: BlendMode,
    texture_address_mode_u: TextureAddressMode,
    texture_address_mode_v: TextureAddressMode,

    render_commands: Vec<RenderCommand>,
    render_command_generation: u32,
    last_queued_color: FColor,
    last_queued_viewport: Rect,
    last_queued_cliprect: Rect,
    last_queued_cliprect_enabled: bool,
    color_queued: bool,
    viewport_queued: bool,
    cliprect_queued: bool,

    props: Properties,

    debug_char_texture_atlas: Option<Texture>,

    /// The window rendered into, if any (`renderer->window`), with the
    /// link its event watcher feeds.
    window: Option<window::WindowLink>,
    /// The window was destroyed (`renderer->destroyed`).
    destroyed: bool,
    /// Freed by `SDL_QuitRender()` (the object is no longer valid).
    freed: bool,
    /// Update the main view even while a target is set (the event watcher
    /// points `renderer->view` at the main view).
    force_main_view: bool,
    transparent_window: bool,
    /// The window shape the shape texture was made from.
    shape_surface: Option<std::sync::Arc<Surface<'static>>>,
    shape_texture: Option<Texture>,
}

impl std::fmt::Debug for Renderer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Renderer")
            .field("name", &self.backend.name())
            .field("target", &self.target)
            .finish_non_exhaustive()
    }
}

/// `UpdatePixelViewport()`.
fn update_pixel_viewport(view: &mut RenderViewState) {
    view.pixel_viewport.x =
        ((view.viewport.x * view.current_scale.x) + view.logical_offset.x).floor() as i32;
    view.pixel_viewport.y =
        ((view.viewport.y * view.current_scale.y) + view.logical_offset.y).floor() as i32;
    if view.viewport.w >= 0.0 {
        view.pixel_viewport.w = (view.viewport.w * view.current_scale.x).ceil() as i32;
    } else {
        view.pixel_viewport.w = view.pixel_w;
    }
    if view.viewport.h >= 0.0 {
        view.pixel_viewport.h = (view.viewport.h * view.current_scale.y).ceil() as i32;
    } else {
        view.pixel_viewport.h = view.pixel_h;
    }
}

/// `UpdatePixelClipRect()`.
fn update_pixel_clip_rect(view: &mut RenderViewState) {
    let scale_x = view.current_scale.x;
    let scale_y = view.current_scale.y;
    view.pixel_clip_rect.x = (view.clip_rect.x * scale_x).floor() as i32;
    view.pixel_clip_rect.y = (view.clip_rect.y * scale_y).floor() as i32;
    view.pixel_clip_rect.w = (view.clip_rect.w * scale_x).ceil() as i32;
    view.pixel_clip_rect.h = (view.clip_rect.h * scale_y).ceil() as i32;
}

/// `SDL_GetRectIntersectionFloat(a, b, &result)` with its output-parameter
/// behaviour.
fn intersect_float(a: &FRect, b: &FRect, result: &mut FRect) -> bool {
    match a.intersection(b) {
        Some(r) => {
            *result = r;
            true
        }
        None => {
            // (the result is set to the empty intersection, unused here)
            false
        }
    }
}

/// `SDL_GetRenderLineMethod()`.
fn render_line_method_from_hint() -> RenderLineMethod {
    let method = hints::get(hints::RENDER_LINE_METHOD)
        .map(|h| crate::stdlib::atoi(&h))
        .unwrap_or(0);
    match method {
        1 => RenderLineMethod::Points,
        2 => RenderLineMethod::Lines,
        3 => RenderLineMethod::Geometry,
        _ => RenderLineMethod::Points,
    }
}

impl Renderer {
    /// A software renderer drawing into `surface`. Translation of
    /// `SDL_CreateSoftwareRenderer()`; the renderer owns the surface
    /// ([`Renderer::surface`], [`Renderer::into_surface`]).
    pub fn software(surface: Surface<'static>) -> Result<Renderer> {
        Renderer::software_with(surface, &RendererCreateInfo::default())
    }

    /// A software renderer drawing into `surface`, with creation options.
    /// Translation of `SDL_CreateRendererWithProperties()` with
    /// `SDL_PROP_RENDERER_CREATE_SURFACE_POINTER`.
    pub fn software_with(surface: Surface<'static>, info: &RendererCreateInfo) -> Result<Renderer> {
        let present_vsync = present_vsync_with_hint(info);

        // SW_CreateRendererForSurface()
        let (w, h, format) = (surface.width(), surface.height(), surface.format());
        let backend = SwRenderer::for_surface(surface)?;
        Renderer::finish_create(Box::new(backend), format, (w, h), info, present_vsync, None)
    }

    /// The common part of `SDL_CreateRendererWithProperties()` once the
    /// backend exists.
    fn finish_create(
        backend: Box<dyn RenderBackend>,
        format: PixelFormat,
        (w, h): (i32, i32),
        info: &RendererCreateInfo,
        present_vsync: i64,
        window: Option<crate::video::Window>,
    ) -> Result<Renderer> {
        // (SW_CreateRendererForSurface())
        let output_colorspace = info.output_colorspace.unwrap_or(Colorspace::SRGB);
        if output_colorspace != Colorspace::SRGB {
            return Err(Error::new("Unsupported output colorspace"));
        }

        let texture_formats = backend
            .texture_formats()
            .unwrap_or_else(|| SwRenderer::select_best_formats(format));
        let software = backend.name() == SOFTWARE_RENDERER;
        let npot_texture_wrap_unsupported = backend.npot_texture_wrap_unsupported();
        let max_texture_size = backend.max_texture_size();
        let mut renderer = Renderer {
            id: NEXT_RENDERER_ID.fetch_add(1, Ordering::Relaxed),
            backend,
            texture_formats,
            software,
            npot_texture_wrap_unsupported,
            hidden: false,
            wanted_vsync: false,
            simulate_vsync: false,
            simulate_vsync_interval_ns: 0,
            last_present: 0,
            main_view: RenderViewState::new(w, h),
            dpi_scale: FPoint { x: 1.0, y: 1.0 },
            line_method: RenderLineMethod::Lines,
            scale_mode: ScaleMode::Linear,
            textures: TextureStore::default(),
            gpu_render_states: GpuRenderStates::default(),
            gpu_render_state: None,
            target: None,
            palettes: HashMap::new(),
            current_colorspace: output_colorspace,
            output_colorspace,
            sdr_white_point: 1.0,
            hdr_headroom: 1.0,
            desired_color_scale: 1.0,
            color_scale: 1.0,
            color: FColor::default(),
            blend_mode: BlendMode::NONE,
            texture_address_mode_u: TextureAddressMode::Auto,
            texture_address_mode_v: TextureAddressMode::Auto,
            render_commands: Vec::new(),
            render_command_generation: 0,
            last_queued_color: FColor::default(),
            last_queued_viewport: Rect::default(),
            last_queued_cliprect: Rect::default(),
            last_queued_cliprect_enabled: false,
            color_queued: false,
            viewport_queued: false,
            cliprect_queued: false,
            props: Properties::new(),
            debug_char_texture_atlas: None,
            window: None,
            destroyed: false,
            freed: false,
            force_main_view: false,
            transparent_window: false,
            shape_surface: None,
            shape_texture: None,
        };
        renderer.window = window.map(window::WindowLink::new);

        update_pixel_viewport(&mut renderer.main_view);
        update_pixel_clip_rect(&mut renderer.main_view);
        renderer.update_main_view_dimensions();

        // new textures start at zero, so we start at 1 so first render doesn't flush by accident.
        renderer.render_command_generation = 1;

        if renderer.software {
            // Software renderer always uses line method, for speed
            renderer.line_method = RenderLineMethod::Lines;
        } else {
            renderer.line_method = render_line_method_from_hint();
        }

        if let Some(window) = renderer.window_handle() {
            let flags = window.flags()?;
            if flags.contains(crate::events::window::WindowFlags::TRANSPARENT) {
                renderer.transparent_window = true;
            }

            if flags.intersects(
                crate::events::window::WindowFlags::HIDDEN
                    | crate::events::window::WindowFlags::MINIMIZED,
            ) {
                renderer.hidden = true;
            }
        }

        let props = renderer.props.clone();
        if let Some(max_texture_size) = max_texture_size {
            props.set(
                PROP_RENDERER_MAX_TEXTURE_SIZE_NUMBER,
                max_texture_size as i64,
            )?;
        }
        props.set(PROP_RENDERER_NAME_STRING, renderer.backend.name())?;
        if let Some(window) = renderer.window_handle() {
            props.set_any(PROP_RENDERER_WINDOW_POINTER, window)?;
        }
        props.set(
            PROP_RENDERER_OUTPUT_COLORSPACE_NUMBER,
            renderer.output_colorspace.0 as i64,
        )?;
        props.set(
            PROP_RENDERER_TEXTURE_WRAPPING_BOOLEAN,
            !renderer.npot_texture_wrap_unsupported,
        )?;
        renderer.backend.set_properties(&props);

        if renderer.window.is_some() {
            renderer.update_hdr_properties();
            if let Some(link) = &renderer.window {
                link.register();
            }
        }

        renderer.set_viewport(None)?;

        if let Some(link) = &mut renderer.window {
            link.watch();
        }

        let _ = renderer.set_vsync(present_vsync as i32);
        renderer.calculate_simulated_vsync_interval();

        crate::log::info!(
            crate::log::Category::Render,
            "Created renderer: {}",
            renderer.backend.name()
        );

        Ok(renderer)
    }

    /// Translation of `SDL_CalculateSimulatedVSyncInterval()`: the refresh
    /// rate of the window's display (or of the primary display).
    fn calculate_simulated_vsync_interval(&mut self) {
        let mut display_id = self
            .window_handle()
            .and_then(|w| w.display().ok())
            .unwrap_or(0);
        if display_id == 0 {
            display_id = crate::video::primary_display().unwrap_or(0);
        }
        let (refresh_num, refresh_den) = match crate::video::desktop_display_mode(display_id) {
            Ok(mode) if mode.refresh_rate_numerator > 0 && mode.refresh_rate_denominator > 0 => (
                mode.refresh_rate_numerator as u64,
                mode.refresh_rate_denominator as u64,
            ),
            // Pick a good default refresh rate
            _ => (60u64, 1u64),
        };
        // Flip numerator and denominator to change from framerate to interval
        self.simulate_vsync_interval_ns =
            (crate::timer::NS_PER_SECOND as u64 * refresh_den) / refresh_num;
    }

    /// The output surface of a software renderer.
    pub fn surface(&mut self) -> Result<&Surface<'static>> {
        // Make sure all drawing to the surface is complete
        self.flush_render_commands()?;
        self.backend
            .surface()
            .ok_or_else(|| Error::new("Renderer has no output surface"))
    }

    /// The output surface of a software renderer, for drawing into it directly.
    pub fn surface_mut(&mut self) -> Result<&mut Surface<'static>> {
        self.flush_render_commands()?;
        self.backend
            .surface_mut()
            .ok_or_else(|| Error::new("Renderer has no output surface"))
    }

    /// Destroy the renderer and give back its output surface.
    pub fn into_surface(mut self) -> Option<Surface<'static>> {
        self.destroy_without_freeing();
        let backend = std::mem::replace(&mut self.backend, Box::new(NullBackend));
        backend.into_surface()
    }

    /// The name of the renderer. Translation of `SDL_GetRendererName()`.
    pub fn name(&self) -> &'static str {
        self.backend.name()
    }

    /// The renderer's properties. Translation of `SDL_GetRendererProperties()`.
    pub fn properties(&self) -> Properties {
        self.props.clone()
    }

    /// The texture formats the renderer supports natively, best first
    /// (`SDL_PROP_RENDERER_TEXTURE_FORMATS_POINTER`).
    pub fn texture_formats(&self) -> &[PixelFormat] {
        &self.texture_formats
    }

    /// Translation of `SDL_AddSupportedTextureFormat()`.
    #[allow(dead_code)] // for the backends that add formats after creation
    pub(crate) fn add_supported_texture_format(&mut self, format: PixelFormat) {
        self.texture_formats.push(format);
    }

    /// Translation of `SDL_ConvertToLinear()`.
    #[allow(dead_code)] // for the GPU backends
    pub(crate) fn convert_to_linear(&self, color: &mut FColor) {
        color.r = srgb_to_linear(color.r);
        color.g = srgb_to_linear(color.g);
        color.b = srgb_to_linear(color.b);
        if self.current_colorspace == Colorspace::HDR10 {
            let [r, g, b] = convert_color_709_to_2020([color.r, color.g, color.b]);
            (color.r, color.g, color.b) = (r, g, b);
        }
    }

    /// The current view: the target texture's, or the main view.
    fn view(&self) -> &RenderViewState {
        if self.force_main_view {
            return &self.main_view;
        }
        match self.target.and_then(|t| self.textures.get(t)) {
            Some(t) => &t.view,
            None => &self.main_view,
        }
    }

    fn view_mut(&mut self) -> &mut RenderViewState {
        if self.force_main_view {
            return &mut self.main_view;
        }
        match self.target.and_then(|t| self.textures.get_mut(t)) {
            Some(t) => &mut t.view,
            None => &mut self.main_view,
        }
    }

    fn is_main_view(&self) -> bool {
        self.force_main_view || self.target.is_none()
    }

    /// Translation of `FlushRenderCommands()`.
    fn flush_render_commands(&mut self) -> Result<()> {
        if self.render_commands.is_empty() {
            // nothing to do!
            return Ok(());
        }

        let result = self.backend.run_command_queue(
            &self.render_commands,
            &mut self.textures,
            &self.gpu_render_states,
        );

        // Move the whole render command queue to the unused pool so we can reuse them next time.
        self.render_commands.clear();
        self.backend.reset_vertices();
        self.render_command_generation = self.render_command_generation.wrapping_add(1);
        self.color_queued = false;
        self.viewport_queued = false;
        self.cliprect_queued = false;
        result
    }

    /// Translation of `FlushRenderCommandsIfTextureNeeded()`.
    fn flush_if_texture_needed(&mut self, texture: Texture) -> Result<()> {
        let needed = self
            .textures
            .get(texture)
            .is_some_and(|t| t.last_command_generation == self.render_command_generation);
        if needed {
            // the current command queue depends on this texture, flush the queue now before it changes
            return self.flush_render_commands();
        }
        Ok(())
    }

    /// Translation of `FlushRenderCommandsIfPaletteNeeded()`.
    fn flush_if_palette_needed(&mut self, palette: usize) -> Result<()> {
        let needed = self
            .palettes
            .get(&palette)
            .is_some_and(|p| p.last_command_generation == self.render_command_generation);
        if needed {
            // the current command queue depends on this palette, flush the queue now before it changes
            return self.flush_render_commands();
        }
        Ok(())
    }

    /// Run the queued commands now. Translation of `SDL_FlushRenderer()`.
    pub fn flush(&mut self) -> Result<()> {
        self.sync_window()?;
        self.flush_render_commands()?;
        self.backend.invalidate_cached_state();
        Ok(())
    }

    /// Translation of `AllocateRenderCommand()`: queue `cmd`, returning its index.
    fn allocate_render_command(&mut self, cmd: RenderCommand) -> usize {
        self.render_commands.push(cmd);
        self.render_commands.len() - 1
    }

    /// Translation of `QueueCmdSetViewport()`.
    fn queue_cmd_set_viewport(&mut self) -> Result<()> {
        let viewport = self.view().pixel_viewport;
        if !self.viewport_queued || viewport != self.last_queued_viewport {
            let i = self.allocate_render_command(RenderCommand::SetViewport {
                first: 0,
                rect: viewport,
            });
            match self
                .backend
                .queue_set_viewport(&mut self.render_commands[i])
            {
                Ok(()) => {
                    self.last_queued_viewport = viewport;
                    self.viewport_queued = true;
                }
                Err(e) => {
                    self.render_commands[i] = RenderCommand::NoOp;
                    return Err(e);
                }
            }
        }
        Ok(())
    }

    /// Translation of `QueueCmdSetClipRect()`.
    fn queue_cmd_set_clip_rect(&mut self) -> Result<()> {
        let view = *self.view();
        let clip_rect = view.pixel_clip_rect;
        if !self.cliprect_queued
            || view.clipping_enabled != self.last_queued_cliprect_enabled
            || clip_rect != self.last_queued_cliprect
        {
            self.allocate_render_command(RenderCommand::SetClipRect {
                enabled: view.clipping_enabled,
                rect: clip_rect,
            });
            self.last_queued_cliprect = clip_rect;
            self.last_queued_cliprect_enabled = view.clipping_enabled;
            self.cliprect_queued = true;
        }
        Ok(())
    }

    /// Translation of `QueueCmdSetDrawColor()`.
    fn queue_cmd_set_draw_color(&mut self, color: FColor) -> Result<()> {
        if !self.color_queued
            || color.r != self.last_queued_color.r
            || color.g != self.last_queued_color.g
            || color.b != self.last_queued_color.b
            || color.a != self.last_queued_color.a
        {
            let i = self.allocate_render_command(RenderCommand::SetDrawColor {
                first: 0, // render backend will fill this in.
                color_scale: self.color_scale,
                color,
            });
            match self
                .backend
                .queue_set_draw_color(&mut self.render_commands[i])
            {
                Ok(()) => {
                    self.last_queued_color = color;
                    self.color_queued = true;
                }
                Err(e) => {
                    self.render_commands[i] = RenderCommand::NoOp;
                    return Err(e);
                }
            }
        }
        Ok(())
    }

    /// Translation of `QueueCmdClear()`.
    fn queue_cmd_clear(&mut self) -> Result<()> {
        self.allocate_render_command(RenderCommand::Clear {
            first: 0,
            color_scale: self.color_scale,
            color: self.color,
        });
        Ok(())
    }

    /// Translation of `PrepQueueCmdDraw()`: queue the state a draw needs
    /// and an empty draw command, returning its index.
    fn prep_queue_cmd_draw(&mut self, kind: DrawKind, texture: Option<Texture>) -> Result<usize> {
        let (color, blend_mode, texture_scale_mode) =
            match texture.and_then(|t| self.textures.get(t)) {
                Some(t) => (t.color, t.blend_mode, t.scale_mode),
                None => (self.color, self.blend_mode, ScaleMode::Linear),
            };

        if kind != DrawKind::Geometry {
            self.queue_cmd_set_draw_color(color)?;
        }

        /* Set the viewport and clip rect directly before draws, so the backends
         * don't have to worry about that state not being valid at draw time. */
        if !self.viewport_queued {
            self.queue_cmd_set_viewport()?;
        }
        if !self.cliprect_queued {
            self.queue_cmd_set_clip_rect()?;
        }

        let gpu_render_state = self.gpu_render_state;
        if let Some(state) = gpu_render_state.and_then(|s| self.gpu_render_states.get_mut(s)) {
            state.last_command_generation = self.render_command_generation;
        }
        Ok(self.allocate_render_command(RenderCommand::Draw(
            kind,
            DrawCmd {
                first: 0, // render backend will fill this in.
                count: 0, // render backend will fill this in.
                color_scale: self.color_scale,
                color,
                blend: blend_mode,
                texture,
                texture_scale_mode,
                texture_address_mode_u: TextureAddressMode::Clamp,
                texture_address_mode_v: TextureAddressMode::Clamp,
                gpu_render_state,
            },
        )))
    }

    /// Run a backend queue function on draw command `i`, turning the
    /// command into a no-op if it fails.
    fn queue_into(
        &mut self,
        i: usize,
        f: impl FnOnce(&mut dyn RenderBackend, &mut DrawCmd, &TextureStore) -> Result<()>,
    ) -> Result<()> {
        let Renderer {
            backend,
            render_commands,
            textures,
            ..
        } = self;
        let RenderCommand::Draw(_, cmd) = &mut render_commands[i] else {
            unreachable!()
        };
        let result = f(backend.as_mut(), cmd, textures);
        if result.is_err() {
            render_commands[i] = RenderCommand::NoOp;
        }
        result
    }

    /// Translation of `QueueCmdDrawPoints()`.
    fn queue_cmd_draw_points(&mut self, points: &[FPoint]) -> Result<()> {
        let i = self.prep_queue_cmd_draw(DrawKind::Points, None)?;
        self.queue_into(i, |b, cmd, _| b.queue_draw_points(cmd, points))
    }

    /// Translation of `QueueCmdDrawLines()`.
    fn queue_cmd_draw_lines(&mut self, points: &[FPoint]) -> Result<()> {
        let i = self.prep_queue_cmd_draw(DrawKind::Lines, None)?;
        self.queue_into(i, |b, cmd, _| {
            b.queue_draw_lines(cmd, points)
                .unwrap_or_else(|| Err(Error::unsupported()))
        })
    }

    /// Translation of `QueueCmdFillRects()`.
    fn queue_cmd_fill_rects(&mut self, rects: &[FRect]) -> Result<()> {
        let use_rendergeometry = !self.backend.has_queue_fill_rects();
        let kind = if use_rendergeometry {
            DrawKind::Geometry
        } else {
            DrawKind::FillRects
        };
        let i = self.prep_queue_cmd_draw(kind, None)?;
        if use_rendergeometry {
            let mut xy = Vec::with_capacity(8 * rects.len());
            let mut indices = Vec::with_capacity(6 * rects.len());
            let mut cur_index = 0;
            for r in rects {
                let (minx, miny) = (r.x, r.y);
                let (maxx, maxy) = (r.x + r.w, r.y + r.h);
                xy.extend_from_slice(&[minx, miny, maxx, miny, maxx, maxy, minx, maxy]);
                indices.extend(RECT_INDEX_ORDER.iter().map(|o| cur_index + o));
                cur_index += 4;
            }
            let color = fcolor_floats(self.color);
            let geometry = Geometry {
                xy: &xy,
                xy_stride: 2,
                color: &color,
                color_stride: 0,
                uv: &[],
                uv_stride: 0,
                num_vertices: 4 * rects.len(),
                indices: Some(Indices::I32(&indices)),
            };
            self.queue_into(i, |b, cmd, _| {
                b.queue_geometry(cmd, None, &geometry, 1.0, 1.0)
            })
        } else {
            self.queue_into(i, |b, cmd, _| b.queue_fill_rects(cmd, rects))
        }
    }

    /// Translation of `QueueCmdCopy()`.
    fn queue_cmd_copy(&mut self, texture: Texture, srcrect: &FRect, dstrect: &FRect) -> Result<()> {
        let i = self.prep_queue_cmd_draw(DrawKind::Copy, Some(texture))?;
        self.queue_into(i, |b, cmd, textures| {
            let tex = textures
                .get(texture)
                .ok_or_else(sysrender::invalid_texture)?;
            b.queue_copy(cmd, tex, srcrect, dstrect)
        })
    }

    /// Translation of `QueueCmdCopyEx()`.
    fn queue_cmd_copy_ex(&mut self, texture: Texture, copy: &CopyEx) -> Result<()> {
        let i = self.prep_queue_cmd_draw(DrawKind::CopyEx, Some(texture))?;
        self.queue_into(i, |b, cmd, textures| {
            let tex = textures
                .get(texture)
                .ok_or_else(sysrender::invalid_texture)?;
            b.queue_copy_ex(cmd, tex, copy)
        })
    }

    /// Translation of `QueueCmdGeometry()`.
    fn queue_cmd_geometry(
        &mut self,
        texture: Option<Texture>,
        geometry: &Geometry<'_>,
        scale_x: f32,
        scale_y: f32,
        texture_address_mode_u: TextureAddressMode,
        texture_address_mode_v: TextureAddressMode,
    ) -> Result<()> {
        let i = self.prep_queue_cmd_draw(DrawKind::Geometry, texture)?;
        self.queue_into(i, |b, cmd, textures| {
            cmd.texture_address_mode_u = texture_address_mode_u;
            cmd.texture_address_mode_v = texture_address_mode_v;
            let tex = match texture {
                Some(t) => Some(textures.get(t).ok_or_else(sysrender::invalid_texture)?),
                None => None,
            };
            b.queue_geometry(cmd, tex, geometry, scale_x, scale_y)
        })
    }

    /// Translation of `UpdateMainViewDimensions()`.
    fn update_main_view_dimensions(&mut self) {
        let (window_w, window_h) = self
            .window_handle()
            .and_then(|w| w.size().ok())
            .unwrap_or((0, 0));

        let (w, h) = self.backend_output_size().unwrap_or((0, 0));
        self.main_view.pixel_w = w;
        self.main_view.pixel_h = h;

        if window_w > 0 && window_h > 0 {
            self.dpi_scale = FPoint {
                x: w as f32 / window_w as f32,
                y: h as f32 / window_h as f32,
            };
        } else {
            self.dpi_scale = FPoint { x: 1.0, y: 1.0 };
        }
        update_pixel_viewport(&mut self.main_view);
    }

    /// Translation of `UpdateColorScale()`.
    fn update_color_scale(&mut self) {
        let sdr_white_point = match self.target.and_then(|t| self.textures.get(t)) {
            Some(t) => t.sdr_white_point,
            None => self.sdr_white_point,
        };
        self.color_scale = self.desired_color_scale * sdr_white_point;
    }

    /// The output size in pixels. Translation of `SDL_GetRenderOutputSize()`.
    pub fn output_size(&mut self) -> Result<(i32, i32)> {
        self.sync_window()?;
        self.backend_output_size()
    }

    /// `SDL_GetRenderOutputSize()` without the window check.
    fn backend_output_size(&self) -> Result<(i32, i32)> {
        match self.backend.output_size(&self.textures) {
            Some(r) => r,
            None => match self.window_handle() {
                Some(window) => window.size_in_pixels(),
                // We don't have any output size, this might be an offscreen-only renderer
                None => Ok((0, 0)),
            },
        }
    }

    /// The size of the current target in pixels.
    /// Translation of `SDL_GetCurrentRenderOutputSize()`.
    pub fn current_output_size(&mut self) -> (i32, i32) {
        let _ = self.sync_window();
        let view = self.view();
        (view.pixel_w, view.pixel_h)
    }

    /// Translation of `IsSupportedBlendMode()`.
    fn is_supported_blend_mode(&self, blend_mode: BlendMode) -> bool {
        match blend_mode {
            // These are required to be supported by all renderers
            BlendMode::NONE
            | BlendMode::BLEND
            | BlendMode::BLEND_PREMULTIPLIED
            | BlendMode::ADD
            | BlendMode::ADD_PREMULTIPLIED
            | BlendMode::MOD
            | BlendMode::MUL => true,
            _ => self.backend.supports_blend_mode(blend_mode),
        }
    }

    /// Translation of `UpdateLogicalPresentation()`.
    fn update_logical_presentation(&mut self) {
        let is_main_view = self.is_main_view();
        let (iwidth, iheight) = if is_main_view {
            self.backend_output_size().unwrap_or((0, 0))
        } else {
            let t = self.textures.get(self.target.unwrap()).unwrap();
            (t.w, t.h)
        };

        let view = self.view_mut();
        let logical_w = view.logical_w as f32;
        let logical_h = view.logical_h as f32;

        view.logical_src_rect = FRect {
            x: 0.0,
            y: 0.0,
            w: logical_w,
            h: logical_h,
        };

        if view.logical_presentation_mode == LogicalPresentation::Disabled {
            view.logical_dst_rect = FRect {
                x: 0.0,
                y: 0.0,
                w: iwidth as f32,
                h: iheight as f32,
            };
            view.logical_offset = FPoint { x: 0.0, y: 0.0 };
            view.logical_scale = FPoint { x: 1.0, y: 1.0 };
            view.current_scale = view.scale; // skip the multiplications against 1.0f.
        } else {
            let output_w = iwidth as f32;
            let output_h = iheight as f32;
            let want_aspect = logical_w / logical_h;
            let real_aspect = output_w / output_h;
            let d = &mut view.logical_dst_rect;

            if logical_w <= 0.0 || logical_h <= 0.0 {
                *d = FRect {
                    x: 0.0,
                    y: 0.0,
                    w: output_w,
                    h: output_h,
                };
            } else if view.logical_presentation_mode == LogicalPresentation::IntegerScale {
                let mut scale = if want_aspect > real_aspect {
                    ((output_w as i32) / (logical_w as i32)) as f32 // This an integer division!
                } else {
                    ((output_h as i32) / (logical_h as i32)) as f32 // This an integer division!
                };

                if scale < 1.0 {
                    scale = 1.0;
                }

                d.w = (logical_w * scale).floor();
                d.x = (output_w - d.w) / 2.0;
                d.h = (logical_h * scale).floor();
                d.y = (output_h - d.h) / 2.0;
            } else if view.logical_presentation_mode == LogicalPresentation::Stretch
                || (want_aspect - real_aspect).abs() < 0.0001
            {
                *d = FRect {
                    x: 0.0,
                    y: 0.0,
                    w: output_w,
                    h: output_h,
                };
            } else if want_aspect > real_aspect {
                if view.logical_presentation_mode == LogicalPresentation::Letterbox {
                    // We want a wider aspect ratio than is available - letterbox it
                    let scale = output_w / logical_w;
                    d.x = 0.0;
                    d.w = output_w;
                    d.h = (logical_h * scale).floor();
                    d.y = (output_h - d.h) / 2.0;
                } else {
                    // LogicalPresentation::Overscan
                    /* We want a wider aspect ratio than is available -
                      zoom so logical height matches the real height
                      and the width will grow off the screen
                    */
                    let scale = output_h / logical_h;
                    d.y = 0.0;
                    d.h = output_h;
                    d.w = (logical_w * scale).floor();
                    d.x = (output_w - d.w) / 2.0;
                }
            } else if view.logical_presentation_mode == LogicalPresentation::Letterbox {
                // We want a narrower aspect ratio than is available - use side-bars
                let scale = output_h / logical_h;
                d.y = 0.0;
                d.h = output_h;
                d.w = (logical_w * scale).floor();
                d.x = (output_w - d.w) / 2.0;
            } else {
                // LogicalPresentation::Overscan
                /* We want a narrower aspect ratio than is available -
                  zoom so logical width matches the real width
                  and the height will grow off the screen
                */
                let scale = output_w / logical_w;
                d.x = 0.0;
                d.w = output_w;
                d.h = (logical_h * scale).floor();
                d.y = (output_h - d.h) / 2.0;
            }

            view.logical_scale.x = if logical_w > 0.0 {
                view.logical_dst_rect.w / logical_w
            } else {
                0.0
            };
            view.logical_scale.y = if logical_h > 0.0 {
                view.logical_dst_rect.h / logical_h
            } else {
                0.0
            };
            view.current_scale.x = view.scale.x * view.logical_scale.x;
            view.current_scale.y = view.scale.y * view.logical_scale.y;
            view.logical_offset.x = view.logical_dst_rect.x;
            view.logical_offset.y = view.logical_dst_rect.y;
        }

        if is_main_view {
            // This makes sure the dpi_scale is right. It also sets pixel_w and pixel_h, but we're going to change them directly below here.
            self.update_main_view_dimensions();
        }

        let view = self.view_mut();
        view.pixel_w = view.logical_dst_rect.w as i32;
        view.pixel_h = view.logical_dst_rect.h as i32;
        update_pixel_viewport(view);
        update_pixel_clip_rect(view);
        let _ = self.queue_cmd_set_viewport();
        let _ = self.queue_cmd_set_clip_rect();
    }

    /// Set a device-independent resolution and presentation mode.
    /// Translation of `SDL_SetRenderLogicalPresentation()`.
    pub fn set_logical_presentation(
        &mut self,
        w: i32,
        h: i32,
        mode: LogicalPresentation,
    ) -> Result<()> {
        self.sync_window()?;
        let view = self.view_mut();
        if mode == LogicalPresentation::Disabled {
            view.logical_w = 0;
            view.logical_h = 0;
        } else {
            view.logical_w = w;
            view.logical_h = h;
        }
        view.logical_presentation_mode = mode;

        self.update_logical_presentation();
        Ok(())
    }

    /// The logical size and presentation mode.
    /// Translation of `SDL_GetRenderLogicalPresentation()`.
    pub fn logical_presentation(&self) -> (i32, i32, LogicalPresentation) {
        let view = self.view();
        (
            view.logical_w,
            view.logical_h,
            view.logical_presentation_mode,
        )
    }

    /// The final presentation rectangle of the logical size.
    /// Translation of `SDL_GetRenderLogicalPresentationRect()`.
    pub fn logical_presentation_rect(&self) -> FRect {
        self.view().logical_dst_rect
    }

    /// Translation of `SDL_RenderVectorFromWindow()`.
    #[allow(dead_code)] // for SDL_ConvertEventToRenderCoordinates()
    fn render_vector_from_window(&self, window_dx: f32, window_dy: f32) -> (f32, f32) {
        // Convert from window coordinates to pixels within the window
        let mut window_dx = window_dx * self.dpi_scale.x;
        let mut window_dy = window_dy * self.dpi_scale.y;

        // Convert from pixels within the window to pixels within the view
        let view = &self.main_view;
        if view.logical_presentation_mode != LogicalPresentation::Disabled {
            let src = &view.logical_src_rect;
            let dst = &view.logical_dst_rect;
            window_dx = (window_dx * src.w) / dst.w;
            window_dy = (window_dy * src.h) / dst.h;
        }

        window_dx /= view.scale.x;
        window_dy /= view.scale.y;

        (window_dx, window_dy)
    }

    /// Window coordinates to render coordinates.
    /// Translation of `SDL_RenderCoordinatesFromWindow()`.
    pub fn coordinates_from_window(&self, window_x: f32, window_y: f32) -> (f32, f32) {
        // Convert from window coordinates to pixels within the window
        let mut render_x = window_x * self.dpi_scale.x;
        let mut render_y = window_y * self.dpi_scale.y;

        // Convert from pixels within the window to pixels within the view
        let view = &self.main_view;
        if view.logical_presentation_mode != LogicalPresentation::Disabled {
            let src = &view.logical_src_rect;
            let dst = &view.logical_dst_rect;
            render_x = ((render_x - dst.x) * src.w) / dst.w;
            render_y = ((render_y - dst.y) * src.h) / dst.h;
        }

        render_x = (render_x / view.scale.x) - view.viewport.x;
        render_y = (render_y / view.scale.y) - view.viewport.y;

        (render_x, render_y)
    }

    /// Render coordinates to window coordinates.
    /// Translation of `SDL_RenderCoordinatesToWindow()`.
    pub fn coordinates_to_window(&self, x: f32, y: f32) -> (f32, f32) {
        let view = &self.main_view;
        let mut x = (view.viewport.x + x) * view.scale.x;
        let mut y = (view.viewport.y + y) * view.scale.y;

        // Convert from render coordinates to pixels within the window
        if view.logical_presentation_mode != LogicalPresentation::Disabled {
            let src = &view.logical_src_rect;
            let dst = &view.logical_dst_rect;
            x = dst.x + ((x * dst.w) / src.w);
            y = dst.y + ((y * dst.h) / src.h);
        }

        // Convert from pixels within the window to window coordinates
        x /= self.dpi_scale.x;
        y /= self.dpi_scale.y;

        (x, y)
    }

    /// Set the drawing area (`None` = the whole target).
    /// Translation of `SDL_SetRenderViewport()`.
    pub fn set_viewport(&mut self, rect: Option<&Rect>) -> Result<()> {
        match rect {
            Some(r) => self.set_viewport_float(Some(&FRect {
                x: r.x as f32,
                y: r.y as f32,
                w: r.w as f32,
                h: r.h as f32,
            })),
            None => self.set_viewport_float(None),
        }
    }

    /// The drawing area. Translation of `SDL_GetRenderViewport()`.
    pub fn viewport(&self) -> Rect {
        let frect = self.viewport_float();
        Rect::new(
            frect.x.floor() as i32,
            frect.y.floor() as i32,
            frect.w.ceil() as i32,
            frect.h.ceil() as i32,
        )
    }

    /// Set the drawing area with float coordinates (`None` = the whole target).
    /// Translation of `SDL_SetRenderViewportFloat()`.
    pub fn set_viewport_float(&mut self, rect: Option<&FRect>) -> Result<()> {
        self.sync_window()?;
        let view = self.view_mut();
        match rect {
            Some(r) => {
                if r.w < 0.0 || r.h < 0.0 {
                    return Err(Error::new("rect has a negative size"));
                }
                view.viewport = *r;
            }
            None => {
                view.viewport = FRect {
                    x: 0.0,
                    y: 0.0,
                    w: -1.0,
                    h: -1.0,
                };
            }
        }
        update_pixel_viewport(view);

        self.queue_cmd_set_viewport()
    }

    /// The drawing area. Translation of `SDL_GetRenderViewportFloat()`.
    pub fn viewport_float(&self) -> FRect {
        let view = self.view();
        FRect {
            x: view.viewport.x,
            y: view.viewport.y,
            w: if view.viewport.w >= 0.0 {
                view.viewport.w
            } else {
                view.pixel_w as f32 / view.current_scale.x
            },
            h: if view.viewport.h >= 0.0 {
                view.viewport.h
            } else {
                view.pixel_h as f32 / view.current_scale.y
            },
        }
    }

    /// Whether an explicit viewport is set. Translation of `SDL_RenderViewportSet()`.
    pub fn viewport_set(&self) -> bool {
        let view = self.view();
        view.viewport.w >= 0.0 && view.viewport.h >= 0.0
    }

    /// Translation of `GetRenderViewportSize()`.
    fn render_viewport_size(&self) -> FRect {
        let view = self.view();
        let scale_x = view.current_scale.x;
        let scale_y = view.current_scale.y;
        FRect {
            x: 0.0,
            y: 0.0,
            w: if view.viewport.w >= 0.0 {
                view.viewport.w
            } else {
                view.pixel_w as f32 / scale_x
            },
            h: if view.viewport.h >= 0.0 {
                view.viewport.h
            } else {
                view.pixel_h as f32 / scale_y
            },
        }
    }

    /// The area safe for rendering: the window's safe area in render
    /// coordinates (the viewport, for a render target or a renderer without
    /// a window). Translation of `SDL_GetRenderSafeArea()`.
    pub fn safe_area(&mut self) -> Result<Rect> {
        self.sync_window()?;
        let window = match self.window_handle() {
            Some(window) if self.target.is_none() => window,
            // The entire viewport is safe for rendering
            _ => return Ok(self.viewport()),
        };

        // Get the window safe rect
        let safe = window.safe_area()?;

        // Convert the coordinates into the render space
        let (minx, miny) = self.coordinates_from_window(safe.x as f32, safe.y as f32);
        let (maxx, maxy) = self
            .coordinates_from_window(safe.x as f32 + safe.w as f32, safe.y as f32 + safe.h as f32);

        let mut rect = Rect::new(
            minx.ceil() as i32,
            miny.ceil() as i32,
            (maxx - minx).ceil() as i32,
            (maxy - miny).ceil() as i32,
        );

        // Clip with the viewport
        let viewport = self.viewport();
        if let Some(r) = rect.intersection(&viewport) {
            rect = r;
        } else {
            rect = Rect::default();
        }
        Ok(rect)
    }

    /// Set the clip rectangle (`None` disables clipping).
    /// Translation of `SDL_SetRenderClipRect()`.
    pub fn set_clip_rect(&mut self, rect: Option<&Rect>) -> Result<()> {
        match rect {
            Some(r) => self.set_clip_rect_float(Some(&FRect {
                x: r.x as f32,
                y: r.y as f32,
                w: r.w as f32,
                h: r.h as f32,
            })),
            None => self.set_clip_rect_float(None),
        }
    }

    /// The clip rectangle. Translation of `SDL_GetRenderClipRect()`.
    pub fn clip_rect(&self) -> Rect {
        let frect = self.clip_rect_float();
        Rect::new(
            frect.x.floor() as i32,
            frect.y.floor() as i32,
            frect.w.ceil() as i32,
            frect.h.ceil() as i32,
        )
    }

    /// Set the clip rectangle with float coordinates (`None` disables
    /// clipping). Translation of `SDL_SetRenderClipRectFloat()`.
    pub fn set_clip_rect_float(&mut self, rect: Option<&FRect>) -> Result<()> {
        self.sync_window()?;
        let view = self.view_mut();
        match rect {
            Some(r) if r.w >= 0.0 && r.h >= 0.0 => {
                view.clipping_enabled = true;
                view.clip_rect = *r;
            }
            _ => {
                view.clipping_enabled = false;
                view.clip_rect = FRect::default();
            }
        }
        update_pixel_clip_rect(view);

        self.queue_cmd_set_clip_rect()
    }

    /// The clip rectangle. Translation of `SDL_GetRenderClipRectFloat()`.
    pub fn clip_rect_float(&self) -> FRect {
        self.view().clip_rect
    }

    /// Whether clipping is enabled. Translation of `SDL_RenderClipEnabled()`.
    pub fn clip_enabled(&self) -> bool {
        self.view().clipping_enabled
    }

    /// Set the drawing scale. Translation of `SDL_SetRenderScale()`.
    pub fn set_scale(&mut self, scale_x: f32, scale_y: f32) -> Result<()> {
        self.sync_window()?;
        let view = self.view_mut();
        if view.scale.x == scale_x && view.scale.y == scale_y {
            return Ok(());
        }

        view.scale.x = scale_x;
        view.scale.y = scale_y;
        view.current_scale.x = scale_x * view.logical_scale.x;
        view.current_scale.y = scale_y * view.logical_scale.y;
        update_pixel_viewport(view);
        update_pixel_clip_rect(view);

        // The scale affects the existing viewport and clip rectangle
        let a = self.queue_cmd_set_viewport();
        let b = self.queue_cmd_set_clip_rect();
        a.and(b)
    }

    /// The drawing scale. Translation of `SDL_GetRenderScale()`.
    pub fn scale(&self) -> (f32, f32) {
        let view = self.view();
        (view.scale.x, view.scale.y)
    }

    /// Set the color for drawing and clearing.
    /// Translation of `SDL_SetRenderDrawColor()`.
    pub fn set_draw_color(&mut self, r: u8, g: u8, b: u8, a: u8) {
        self.set_draw_color_float(
            r as f32 / 255.0,
            g as f32 / 255.0,
            b as f32 / 255.0,
            a as f32 / 255.0,
        );
    }

    /// Set the color for drawing and clearing (0.0-1.0 and beyond).
    /// Translation of `SDL_SetRenderDrawColorFloat()`.
    pub fn set_draw_color_float(&mut self, r: f32, g: f32, b: f32, a: f32) {
        self.color = FColor { r, g, b, a };
    }

    /// The color for drawing and clearing. Translation of `SDL_GetRenderDrawColor()`.
    pub fn draw_color(&self) -> (u8, u8, u8, u8) {
        let c = self.color;
        // (a plain float to integer conversion, as upstream)
        (
            (c.r * 255.0) as u8,
            (c.g * 255.0) as u8,
            (c.b * 255.0) as u8,
            (c.a * 255.0) as u8,
        )
    }

    /// The color for drawing and clearing. Translation of `SDL_GetRenderDrawColorFloat()`.
    pub fn draw_color_float(&self) -> FColor {
        self.color
    }

    /// Set the scale applied to colors when rendering.
    /// Translation of `SDL_SetRenderColorScale()`.
    pub fn set_color_scale(&mut self, scale: f32) {
        self.desired_color_scale = scale;
        self.update_color_scale();
    }

    /// The color scale. Translation of `SDL_GetRenderColorScale()`.
    #[allow(clippy::misnamed_getters)] // (the scale set, not the one in effect)
    pub fn color_scale(&self) -> f32 {
        self.desired_color_scale
    }

    /// Set the blend mode for drawing operations.
    /// Translation of `SDL_SetRenderDrawBlendMode()`.
    pub fn set_draw_blend_mode(&mut self, blend_mode: BlendMode) -> Result<()> {
        if blend_mode == BlendMode::INVALID {
            return Err(Error::invalid_param("blendMode"));
        }

        if !self.is_supported_blend_mode(blend_mode) {
            return Err(Error::unsupported());
        }

        self.blend_mode = blend_mode;
        Ok(())
    }

    /// The blend mode for drawing operations.
    /// Translation of `SDL_GetRenderDrawBlendMode()`.
    pub fn draw_blend_mode(&self) -> BlendMode {
        self.blend_mode
    }

    /// Clear the target with the draw color. Translation of `SDL_RenderClear()`.
    pub fn clear(&mut self) -> Result<()> {
        self.sync_window()?;
        self.queue_cmd_clear()
    }

    /// Draw a point. Translation of `SDL_RenderPoint()`.
    pub fn render_point(&mut self, x: f32, y: f32) -> Result<()> {
        self.sync_window()?;
        self.render_points(&[FPoint { x, y }])
    }

    /// Translation of `RenderPointsWithRects()`.
    fn render_points_with_rects(&mut self, fpoints: &[FPoint]) -> Result<()> {
        if fpoints.is_empty() {
            return Ok(());
        }

        let view = self.view();
        let scale_x = view.current_scale.x;
        let scale_y = view.current_scale.y;
        let frects: Vec<FRect> = fpoints
            .iter()
            .map(|p| FRect {
                x: p.x * scale_x,
                y: p.y * scale_y,
                w: scale_x,
                h: scale_y,
            })
            .collect();

        self.queue_cmd_fill_rects(&frects)
    }

    /// Draw points. Translation of `SDL_RenderPoints()`.
    pub fn render_points(&mut self, points: &[FPoint]) -> Result<()> {
        self.sync_window()?;
        if points.is_empty() {
            return Ok(());
        }

        let view = self.view();
        if view.current_scale.x != 1.0 || view.current_scale.y != 1.0 {
            self.render_points_with_rects(points)
        } else {
            self.queue_cmd_draw_points(points)
        }
    }

    /// Draw a line. Translation of `SDL_RenderLine()`.
    pub fn render_line(&mut self, x1: f32, y1: f32, x2: f32, y2: f32) -> Result<()> {
        self.sync_window()?;
        self.render_lines(&[FPoint { x: x1, y: y1 }, FPoint { x: x2, y: y2 }])
    }

    /// Translation of `RenderLineBresenham()`.
    fn render_line_bresenham(
        &mut self,
        x1: i32,
        y1: i32,
        x2: i32,
        y2: i32,
        draw_last: bool,
    ) -> Result<()> {
        let view = *self.view();
        let max_pixels = view.pixel_w.max(view.pixel_h) * 4;

        /* the backend might clip this further to the clipping rect, but we
        just want a basic safety against generating millions of points for
        massive lines. */
        let mut viewport = view.pixel_viewport;
        viewport.x = 0;
        viewport.y = 0;
        let Some((p1, p2)) = viewport.clip_line(Point { x: x1, y: y1 }, Point { x: x2, y: y2 })
        else {
            return Ok(());
        };
        let (x1, y1, x2, y2) = (p1.x, p1.y, p2.x, p2.y);

        let deltax = (x2 - x1).abs();
        let deltay = (y2 - y1).abs();

        let (mut numpixels, mut d, dinc1, dinc2, mut xinc1, mut xinc2, mut yinc1, mut yinc2);
        if deltax >= deltay {
            numpixels = deltax + 1;
            d = (2 * deltay) - deltax;
            dinc1 = deltay * 2;
            dinc2 = (deltay - deltax) * 2;
            xinc1 = 1;
            xinc2 = 1;
            yinc1 = 0;
            yinc2 = 1;
        } else {
            numpixels = deltay + 1;
            d = (2 * deltax) - deltay;
            dinc1 = deltax * 2;
            dinc2 = (deltax - deltay) * 2;
            xinc1 = 0;
            xinc2 = 1;
            yinc1 = 1;
            yinc2 = 1;
        }

        if x1 > x2 {
            xinc1 = -xinc1;
            xinc2 = -xinc2;
        }
        if y1 > y2 {
            yinc1 = -yinc1;
            yinc2 = -yinc2;
        }

        let mut x = x1;
        let mut y = y1;

        if !draw_last {
            numpixels -= 1;
        }

        if numpixels > max_pixels {
            return Err(Error::new(format!(
                "Line too long (tried to draw {numpixels} pixels, max {max_pixels})"
            )));
        }

        let mut points = Vec::with_capacity(numpixels.max(0) as usize);
        for _ in 0..numpixels {
            points.push(FPoint {
                x: x as f32,
                y: y as f32,
            });
            if d < 0 {
                d += dinc1;
                x += xinc1;
                y += yinc1;
            } else {
                d += dinc2;
                x += xinc2;
                y += yinc2;
            }
        }

        if view.current_scale.x != 1.0 || view.current_scale.y != 1.0 {
            self.render_points_with_rects(&points)
        } else {
            self.queue_cmd_draw_points(&points)
        }
    }

    /// Translation of `RenderLinesWithRectsF()`.
    fn render_lines_with_rects(&mut self, points: &[FPoint]) -> Result<()> {
        let view = *self.view();
        let scale_x = view.current_scale.x;
        let scale_y = view.current_scale.y;
        let count = points.len();
        let mut frects = Vec::with_capacity(count - 1);
        let mut result = Ok(());
        let mut drew_line = false;
        let mut draw_last = false;

        for i in 0..count - 1 {
            let same_x = points[i].x == points[i + 1].x;
            let same_y = points[i].y == points[i + 1].y;

            if i == count - 2 {
                if !drew_line || points[i + 1].x != points[0].x || points[i + 1].y != points[0].y {
                    draw_last = true;
                }
            } else if same_x && same_y {
                continue;
            }

            if same_x {
                let min_y = points[i].y.min(points[i + 1].y);
                let max_y = points[i].y.max(points[i + 1].y);

                let mut frect = FRect {
                    x: points[i].x * scale_x,
                    y: min_y * scale_y,
                    w: scale_x,
                    h: (max_y - min_y + draw_last as i32 as f32) * scale_y,
                };
                if !draw_last && points[i + 1].y < points[i].y {
                    frect.y += scale_y;
                }
                frects.push(frect);
            } else if same_y {
                let min_x = points[i].x.min(points[i + 1].x);
                let max_x = points[i].x.max(points[i + 1].x);

                let mut frect = FRect {
                    x: min_x * scale_x,
                    y: points[i].y * scale_y,
                    w: (max_x - min_x + draw_last as i32 as f32) * scale_x,
                    h: scale_y,
                };
                if !draw_last && points[i + 1].x < points[i].x {
                    frect.x += scale_x;
                }
                frects.push(frect);
            } else {
                let r = self.render_line_bresenham(
                    points[i].x.round() as i32,
                    points[i].y.round() as i32,
                    points[i + 1].x.round() as i32,
                    points[i + 1].y.round() as i32,
                    draw_last,
                );
                result = result.and(r);
            }
            drew_line = true;
        }

        if !frects.is_empty() {
            result = result.and(self.queue_cmd_fill_rects(&frects));
        }

        result
    }

    /// Draw connected lines. Translation of `SDL_RenderLines()`.
    pub fn render_lines(&mut self, points: &[FPoint]) -> Result<()> {
        self.sync_window()?;
        let count = points.len();
        if count < 2 {
            return Ok(());
        }

        let view = *self.view();
        let islogical = view.logical_presentation_mode != LogicalPresentation::Disabled;

        if islogical || self.line_method == RenderLineMethod::Geometry {
            let scale_x = view.current_scale.x;
            let scale_y = view.current_scale.y;
            let mut xy = Vec::with_capacity(4 * 2 * count);
            let mut indices: Vec<i32> = Vec::with_capacity(4 * 3 * (count - 1) + 2 * 3 * count);
            let mut cur_index: i32 = -4;
            let is_looping =
                points[0].x == points[count - 1].x && points[0].y == points[count - 1].y;
            let mut p = FPoint { x: 0.0, y: 0.0 }; // previous point

            /*       p            q
                    0----1------ 4----5
                    | \  |``\    | \  |
                    |  \ |   ` `\|  \ |
                    3----2-------7----6
            */
            for (i, point) in points.iter().enumerate() {
                let mut q = *point; // current point
                q.x *= scale_x;
                q.y *= scale_y;

                xy.extend_from_slice(&[
                    q.x,
                    q.y,
                    q.x + scale_x,
                    q.y,
                    q.x + scale_x,
                    q.y + scale_y,
                    q.x,
                    q.y + scale_y,
                ]);

                let mut add_triangle = |i1: i32, i2: i32, i3: i32| {
                    indices.extend([cur_index + i1, cur_index + i2, cur_index + i3]);
                };

                // closed polyline, don´t draw twice the point
                if i != 0 || !is_looping {
                    add_triangle(4, 5, 6);
                    add_triangle(4, 6, 7);
                }

                // first point only, no segment
                if i == 0 {
                    p = q;
                    cur_index += 4;
                    continue;
                }

                // draw segment
                if p.y == q.y {
                    if p.x < q.x {
                        add_triangle(1, 4, 7);
                        add_triangle(1, 7, 2);
                    } else {
                        add_triangle(5, 0, 3);
                        add_triangle(5, 3, 6);
                    }
                } else if p.x == q.x {
                    if p.y < q.y {
                        add_triangle(2, 5, 4);
                        add_triangle(2, 4, 3);
                    } else {
                        add_triangle(6, 1, 0);
                        add_triangle(6, 0, 7);
                    }
                } else if p.y < q.y {
                    if p.x < q.x {
                        add_triangle(1, 5, 4);
                        add_triangle(1, 4, 2);
                        add_triangle(2, 4, 7);
                        add_triangle(2, 7, 3);
                    } else {
                        add_triangle(4, 0, 5);
                        add_triangle(5, 0, 3);
                        add_triangle(5, 3, 6);
                        add_triangle(6, 3, 2);
                    }
                } else if p.x < q.x {
                    add_triangle(0, 4, 7);
                    add_triangle(0, 7, 1);
                    add_triangle(1, 7, 6);
                    add_triangle(1, 6, 2);
                } else {
                    add_triangle(6, 5, 1);
                    add_triangle(6, 1, 0);
                    add_triangle(7, 6, 0);
                    add_triangle(7, 0, 3);
                }

                p = q;
                cur_index += 4;
            }

            let color = fcolor_floats(self.color);
            let geometry = Geometry {
                xy: &xy,
                xy_stride: 2,
                color: &color,
                color_stride: 0,
                uv: &[],
                uv_stride: 0,
                num_vertices: 4 * count,
                indices: Some(Indices::I32(&indices)),
            };
            self.queue_cmd_geometry(
                None,
                &geometry,
                1.0,
                1.0,
                TextureAddressMode::Clamp,
                TextureAddressMode::Clamp,
            )
        } else if self.line_method == RenderLineMethod::Points
            || view.scale.x != 1.0
            || view.scale.y != 1.0
        {
            // (we checked for logical scale elsewhere.)
            self.render_lines_with_rects(points)
        } else {
            self.queue_cmd_draw_lines(points)
        }
    }

    /// Draw a rectangle outline (`None` = the whole viewport).
    /// Translation of `SDL_RenderRect()`.
    pub fn render_rect(&mut self, rect: Option<&FRect>) -> Result<()> {
        self.sync_window()?;
        // If 'rect' == NULL, then outline the whole surface
        let rect = rect.copied().unwrap_or_else(|| self.render_viewport_size());

        let points = [
            FPoint {
                x: rect.x,
                y: rect.y,
            },
            FPoint {
                x: rect.x + rect.w - 1.0,
                y: rect.y,
            },
            FPoint {
                x: rect.x + rect.w - 1.0,
                y: rect.y + rect.h - 1.0,
            },
            FPoint {
                x: rect.x,
                y: rect.y + rect.h - 1.0,
            },
            FPoint {
                x: rect.x,
                y: rect.y,
            },
        ];
        self.render_lines(&points)
    }

    /// Draw rectangle outlines. Translation of `SDL_RenderRects()`.
    pub fn render_rects(&mut self, rects: &[FRect]) -> Result<()> {
        self.sync_window()?;
        for r in rects {
            self.render_rect(Some(r))?;
        }
        Ok(())
    }

    /// Fill a rectangle (`None` = the whole viewport).
    /// Translation of `SDL_RenderFillRect()`.
    pub fn render_fill_rect(&mut self, rect: Option<&FRect>) -> Result<()> {
        self.sync_window()?;
        // If 'rect' == NULL, then fill the whole surface
        let rect = rect.copied().unwrap_or_else(|| self.render_viewport_size());
        self.render_fill_rects(&[rect])
    }

    /// Fill rectangles. Translation of `SDL_RenderFillRects()`.
    pub fn render_fill_rects(&mut self, rects: &[FRect]) -> Result<()> {
        self.sync_window()?;
        if rects.is_empty() {
            return Ok(());
        }

        let view = self.view();
        let scale_x = view.current_scale.x;
        let scale_y = view.current_scale.y;
        let frects: Vec<FRect> = rects
            .iter()
            .map(|r| FRect {
                x: r.x * scale_x,
                y: r.y * scale_y,
                w: r.w * scale_x,
                h: r.h * scale_y,
            })
            .collect();

        self.queue_cmd_fill_rects(&frects)
    }

    /// Read pixels from the current target (`None` = the whole viewport).
    /// Translation of `SDL_RenderReadPixels()`.
    pub fn read_pixels(&mut self, rect: Option<&Rect>) -> Result<Surface<'static>> {
        self.sync_window()?;
        // we need to render before we read the results.
        let _ = self.flush_render_commands();

        let mut real_rect = self.view().pixel_viewport;
        if let Some(rect) = rect {
            let viewport = real_rect;
            if !rect.intersect_into(&viewport, &mut real_rect) {
                return Err(Error::new("Can't read outside the current viewport"));
            }
        }

        let mut surface = match self.backend.read_pixels(&real_rect, &mut self.textures) {
            Some(r) => r?,
            None => return Err(Error::unsupported()),
        };

        let props = surface.properties();
        use crate::video::surface::{
            PROP_SURFACE_HDR_HEADROOM_FLOAT, PROP_SURFACE_SDR_WHITE_POINT_FLOAT,
        };
        const SCRGB_NITS: f32 = 80.0;
        match self.target.and_then(|t| self.textures.get(t)) {
            Some(target) => {
                let expected_format = match target.parent.and_then(|p| self.textures.get(p)) {
                    Some(parent) => parent.format,
                    None => target.format,
                };
                if target.colorspace.transfer() == TransferCharacteristics::Pq {
                    props.set(
                        PROP_SURFACE_SDR_WHITE_POINT_FLOAT,
                        target.sdr_white_point * SCRGB_NITS,
                    )?;
                } else {
                    props.set(PROP_SURFACE_SDR_WHITE_POINT_FLOAT, target.sdr_white_point)?;
                }
                props.set(PROP_SURFACE_HDR_HEADROOM_FLOAT, target.hdr_headroom)?;

                // Set the expected surface format
                use PixelFormat as F;
                let f = surface.format();
                if (f == F::ARGB8888 && expected_format == F::XRGB8888)
                    || (f == F::RGBA8888 && expected_format == F::RGBX8888)
                    || (f == F::ABGR8888 && expected_format == F::XBGR8888)
                    || (f == F::BGRA8888 && expected_format == F::BGRX8888)
                {
                    surface.reinterpret_format(expected_format)?;
                }
            }
            None => {
                if self.output_colorspace.transfer() == TransferCharacteristics::Pq {
                    props.set(
                        PROP_SURFACE_SDR_WHITE_POINT_FLOAT,
                        self.sdr_white_point * SCRGB_NITS,
                    )?;
                } else {
                    props.set(PROP_SURFACE_SDR_WHITE_POINT_FLOAT, self.sdr_white_point)?;
                }
                props.set(PROP_SURFACE_HDR_HEADROOM_FLOAT, self.hdr_headroom)?;
            }
        }
        Ok(surface)
    }

    /// Translation of `SDL_SimulateRenderVSync()`.
    fn simulate_render_vsync(&mut self) {
        let interval = self.simulate_vsync_interval_ns;
        if interval == 0 {
            // We can't do sub-ns delay, so just return here
            return;
        }

        let mut now = crate::timer::ticks_ns();
        let elapsed = now.wrapping_sub(self.last_present);
        if elapsed < interval {
            let duration = interval - elapsed;
            crate::timer::delay_precise(std::time::Duration::from_nanos(duration));
            now = crate::timer::ticks_ns();
        }

        let elapsed = now.wrapping_sub(self.last_present);
        if self.last_present == 0 || elapsed > crate::timer::NS_PER_SECOND as u64
        /* SDL_MS_TO_NS(1000) */
        {
            // It's been too long, reset the presentation timeline
            self.last_present = now;
        } else {
            self.last_present += (elapsed / interval) * interval;
        }
    }

    /// Run the queued commands and present the result.
    /// Translation of `SDL_RenderPresent()`.
    pub fn present(&mut self) -> Result<()> {
        self.sync_window()?;
        if self.target.is_some() {
            return Err(Error::new("You can't present on a render target"));
        }

        if self.transparent_window {
            self.apply_window_shape();
        }

        // time to send everything to the GPU!
        let _ = self.flush_render_commands();

        // (DONT_DRAW_WHILE_HIDDEN is only set on mobile platforms, where
        // nothing is presented while the renderer is hidden)
        let presented = if cfg!(any(target_os = "ios", target_os = "android")) && self.hidden {
            false
        } else {
            self.backend.present()
        };

        if self.simulate_vsync || (!presented && self.wanted_vsync) {
            self.simulate_render_vsync();
        }
        Ok(())
    }

    /// Translation of `SDL_DestroyRendererWithoutFreeing()`.
    fn destroy_without_freeing(&mut self) {
        self.destroyed = true;

        if let Some(link) = &mut self.window {
            link.unregister();
        }

        if self.software {
            // Make sure all drawing to a surface is complete
            let _ = self.flush_render_commands();
        }
        // SDL_DiscardAllCommands()
        self.render_commands.clear();
        self.backend.reset_vertices();

        if let Some(atlas) = self.debug_char_texture_atlas.take() {
            self.destroy_texture(atlas);
        }

        // Free existing textures for this renderer
        // (upstream walks its texture list, where a texture comes before its
        // native texture and destroys it; native textures are skipped here)
        for t in self.textures.handles(self.id) {
            if self.textures.get(t).is_some_and(|d| d.parent.is_none()) {
                self.destroy_texture_internal(t, true);
            }
        }

        // Free palette cache, which should be empty now
        crate::sdl_assert!(self.palettes.is_empty());

        // Clean up renderer-specific resources
        self.backend.destroy();
    }

    /// Add the Vulkan semaphores to wait for before rendering the current
    /// frame, and to signal once it's rendered (0: none). Translation of
    /// `SDL_AddVulkanRenderSemaphores()`.
    pub fn add_vulkan_render_semaphores(
        &mut self,
        wait_stage_mask: u32,
        wait_semaphore: i64,
        signal_semaphore: i64,
    ) -> Result<()> {
        self.sync_window()?;
        self.backend
            .add_vulkan_render_semaphores(wait_stage_mask, wait_semaphore, signal_semaphore)
            .unwrap_or_else(|| Err(Error::unsupported()))
    }

    /// Set the vsync interval (0 disables it). Translation of `SDL_SetRenderVSync()`.
    pub fn set_vsync(&mut self, vsync: i32) -> Result<()> {
        self.wanted_vsync = vsync != 0;

        // for the software renderer, forward the call to the WindowTexture renderer
        // (with a window, SDL_SetWindowTextureVSync() would handle it, but
        // only a window framebuffer through a GPU texture can, and there are
        // none yet)
        if self.software && self.window.is_none() {
            if vsync == 0 {
                return Ok(());
            } else {
                return Err(Error::unsupported());
            }
        }

        let done = matches!(self.backend.set_vsync(vsync), Some(Ok(())));
        if !done {
            match vsync {
                0 => self.simulate_vsync = false,
                1 => self.simulate_vsync = true,
                _ => return Err(Error::unsupported()),
            }
        }
        self.props.set(PROP_RENDERER_VSYNC_NUMBER, vsync as i64)?;
        Ok(())
    }

    /// The vsync interval. Translation of `SDL_GetRenderVSync()`.
    pub fn vsync(&self) -> i32 {
        self.props
            .get_number(PROP_RENDERER_VSYNC_NUMBER)
            .unwrap_or(0) as i32
    }

    /// Set the scale mode of new textures. Translation of `SDL_SetDefaultTextureScaleMode()`.
    pub fn set_default_texture_scale_mode(&mut self, scale_mode: ScaleMode) {
        self.scale_mode = scale_mode;
    }

    /// The scale mode of new textures. Translation of `SDL_GetDefaultTextureScaleMode()`.
    pub fn default_texture_scale_mode(&self) -> ScaleMode {
        self.scale_mode
    }

    /// Set the texture addressing of geometry.
    /// Translation of `SDL_SetRenderTextureAddressMode()`.
    pub fn set_texture_address_mode(
        &mut self,
        u_mode: TextureAddressMode,
        v_mode: TextureAddressMode,
    ) {
        self.texture_address_mode_u = u_mode;
        self.texture_address_mode_v = v_mode;
    }

    /// The texture addressing of geometry.
    /// Translation of `SDL_GetRenderTextureAddressMode()`.
    pub fn texture_address_mode(&self) -> (TextureAddressMode, TextureAddressMode) {
        (self.texture_address_mode_u, self.texture_address_mode_v)
    }
}

impl Drop for Renderer {
    fn drop(&mut self) {
        // Translation of `SDL_DestroyRenderer()`.
        // if we've already destroyed the renderer through SDL_DestroyWindow, we just need
        // to free the renderer pointer. This lets apps destroy the window and renderer
        // in either order.
        if !self.destroyed && !self.window.as_ref().is_some_and(|l| l.window_destroyed()) {
            self.destroy_without_freeing();
        }
    }
}

/// The backend of a renderer that gave its output surface away.
struct NullBackend;

impl RenderBackend for NullBackend {
    fn name(&self) -> &'static str {
        SOFTWARE_RENDERER
    }
    fn output_size(&self, _: &TextureStore) -> Option<Result<(i32, i32)>> {
        None
    }
    fn create_texture(
        &mut self,
        _: &mut sysrender::TextureData,
        _: &sysrender::TextureCreateProps,
    ) -> Result<()> {
        Err(Error::unsupported())
    }
    fn queue_set_viewport(&mut self, _: &mut RenderCommand) -> Result<()> {
        Ok(())
    }
    fn queue_set_draw_color(&mut self, _: &mut RenderCommand) -> Result<()> {
        Ok(())
    }
    fn queue_draw_points(&mut self, _: &mut DrawCmd, _: &[FPoint]) -> Result<()> {
        Ok(())
    }
    fn queue_draw_lines(&mut self, _: &mut DrawCmd, _: &[FPoint]) -> Option<Result<()>> {
        Some(Ok(()))
    }
    fn has_queue_fill_rects(&self) -> bool {
        true
    }
    fn queue_fill_rects(&mut self, _: &mut DrawCmd, _: &[FRect]) -> Result<()> {
        Ok(())
    }
    fn has_queue_copy(&self) -> bool {
        true
    }
    fn queue_copy(
        &mut self,
        _: &mut DrawCmd,
        _: &sysrender::TextureData,
        _: &FRect,
        _: &FRect,
    ) -> Result<()> {
        Ok(())
    }
    fn has_queue_copy_ex(&self) -> bool {
        true
    }
    fn queue_copy_ex(
        &mut self,
        _: &mut DrawCmd,
        _: &sysrender::TextureData,
        _: &CopyEx,
    ) -> Result<()> {
        Ok(())
    }
    fn has_queue_geometry(&self) -> bool {
        true
    }
    fn queue_geometry(
        &mut self,
        _: &mut DrawCmd,
        _: Option<&sysrender::TextureData>,
        _: &Geometry<'_>,
        _: f32,
        _: f32,
    ) -> Result<()> {
        Ok(())
    }
    fn invalidate_cached_state(&mut self) {}
    fn run_command_queue(
        &mut self,
        _: &[RenderCommand],
        _: &mut TextureStore,
        _: &GpuRenderStates,
    ) -> Result<()> {
        Ok(())
    }
    fn reset_vertices(&mut self) {}
    fn create_palette(&mut self) -> Result<Box<dyn std::any::Any>> {
        Err(Error::unsupported())
    }
    fn update_palette(
        &mut self,
        _: &mut dyn std::any::Any,
        _: &[crate::video::pixels::Color],
    ) -> Result<()> {
        Ok(())
    }
    fn change_texture_palette(
        &mut self,
        _: &mut sysrender::TextureData,
        _: Option<&dyn std::any::Any>,
    ) -> Option<Result<()>> {
        None
    }
    fn update_texture(
        &mut self,
        _: &mut sysrender::TextureData,
        _: &Rect,
        _: &[u8],
        _: usize,
    ) -> Result<()> {
        Ok(())
    }
    fn lock_texture(&mut self, _: &mut sysrender::TextureData, _: &Rect) -> Result<(usize, i32)> {
        Err(Error::unsupported())
    }
    fn texture_pixels_mut<'t>(
        &mut self,
        _: &'t mut sysrender::TextureData,
    ) -> Option<&'t mut [u8]> {
        None
    }
    fn unlock_texture(&mut self, _: &mut sysrender::TextureData) {}
    fn set_render_target(&mut self, _: Option<Texture>, _: &TextureStore) -> Result<()> {
        Ok(())
    }
    fn read_pixels(&mut self, _: &Rect, _: &mut TextureStore) -> Option<Result<Surface<'static>>> {
        None
    }
    fn present(&mut self) -> bool {
        false
    }
    fn destroy_texture(&mut self, _: &mut sysrender::TextureData) {}
}

// Blend mode composition, declared in SDL_blendmode.h and implemented in
// SDL_render.c.

/// `SDL_COMPOSE_BLENDMODE()`.
const fn compose_blendmode(
    src_color_factor: u32,
    dst_color_factor: u32,
    color_operation: u32,
    src_alpha_factor: u32,
    dst_alpha_factor: u32,
    alpha_operation: u32,
) -> BlendMode {
    BlendMode(
        color_operation
            | (src_color_factor << 4)
            | (dst_color_factor << 8)
            | (alpha_operation << 16)
            | (src_alpha_factor << 20)
            | (dst_alpha_factor << 24),
    )
}

use crate::video::blendmode::{BlendFactor as BF, BlendOperation as BO};

const BLENDMODE_NONE_FULL: BlendMode = compose_blendmode(
    BF::One as u32,
    BF::Zero as u32,
    BO::Add as u32,
    BF::One as u32,
    BF::Zero as u32,
    BO::Add as u32,
);
const BLENDMODE_BLEND_FULL: BlendMode = compose_blendmode(
    BF::SrcAlpha as u32,
    BF::OneMinusSrcAlpha as u32,
    BO::Add as u32,
    BF::One as u32,
    BF::OneMinusSrcAlpha as u32,
    BO::Add as u32,
);
const BLENDMODE_BLEND_PREMULTIPLIED_FULL: BlendMode = compose_blendmode(
    BF::One as u32,
    BF::OneMinusSrcAlpha as u32,
    BO::Add as u32,
    BF::One as u32,
    BF::OneMinusSrcAlpha as u32,
    BO::Add as u32,
);
const BLENDMODE_ADD_FULL: BlendMode = compose_blendmode(
    BF::SrcAlpha as u32,
    BF::One as u32,
    BO::Add as u32,
    BF::Zero as u32,
    BF::One as u32,
    BO::Add as u32,
);
const BLENDMODE_ADD_PREMULTIPLIED_FULL: BlendMode = compose_blendmode(
    BF::One as u32,
    BF::One as u32,
    BO::Add as u32,
    BF::Zero as u32,
    BF::One as u32,
    BO::Add as u32,
);
const BLENDMODE_MOD_FULL: BlendMode = compose_blendmode(
    BF::Zero as u32,
    BF::SrcColor as u32,
    BO::Add as u32,
    BF::Zero as u32,
    BF::One as u32,
    BO::Add as u32,
);
const BLENDMODE_MUL_FULL: BlendMode = compose_blendmode(
    BF::DstColor as u32,
    BF::OneMinusSrcAlpha as u32,
    BO::Add as u32,
    BF::Zero as u32,
    BF::One as u32,
    BO::Add as u32,
);

/// Translation of `SDL_GetShortBlendMode()`.
fn short_blend_mode(blend_mode: BlendMode) -> BlendMode {
    match blend_mode {
        BLENDMODE_NONE_FULL => BlendMode::NONE,
        BLENDMODE_BLEND_FULL => BlendMode::BLEND,
        BLENDMODE_BLEND_PREMULTIPLIED_FULL => BlendMode::BLEND_PREMULTIPLIED,
        BLENDMODE_ADD_FULL => BlendMode::ADD,
        BLENDMODE_ADD_PREMULTIPLIED_FULL => BlendMode::ADD_PREMULTIPLIED,
        BLENDMODE_MOD_FULL => BlendMode::MOD,
        BLENDMODE_MUL_FULL => BlendMode::MUL,
        _ => blend_mode,
    }
}

/// Translation of `SDL_GetLongBlendMode()`.
fn long_blend_mode(blend_mode: BlendMode) -> BlendMode {
    match blend_mode {
        BlendMode::NONE => BLENDMODE_NONE_FULL,
        BlendMode::BLEND => BLENDMODE_BLEND_FULL,
        BlendMode::BLEND_PREMULTIPLIED => BLENDMODE_BLEND_PREMULTIPLIED_FULL,
        BlendMode::ADD => BLENDMODE_ADD_FULL,
        BlendMode::ADD_PREMULTIPLIED => BLENDMODE_ADD_PREMULTIPLIED_FULL,
        BlendMode::MOD => BLENDMODE_MOD_FULL,
        BlendMode::MUL => BLENDMODE_MUL_FULL,
        _ => blend_mode,
    }
}

impl BlendMode {
    /// A custom blend mode from factors and operations.
    /// Translation of `SDL_ComposeCustomBlendMode()`.
    pub fn compose_custom(
        src_color_factor: BF,
        dst_color_factor: BF,
        color_operation: BO,
        src_alpha_factor: BF,
        dst_alpha_factor: BF,
        alpha_operation: BO,
    ) -> BlendMode {
        let blend_mode = compose_blendmode(
            src_color_factor as u32,
            dst_color_factor as u32,
            color_operation as u32,
            src_alpha_factor as u32,
            dst_alpha_factor as u32,
            alpha_operation as u32,
        );
        short_blend_mode(blend_mode)
    }

    /// Translation of `SDL_GetBlendModeSrcColorFactor()`.
    pub fn src_color_factor(self) -> Option<BF> {
        BF::from_u32((long_blend_mode(self).0 >> 4) & 0xF)
    }

    /// Translation of `SDL_GetBlendModeDstColorFactor()`.
    pub fn dst_color_factor(self) -> Option<BF> {
        BF::from_u32((long_blend_mode(self).0 >> 8) & 0xF)
    }

    /// Translation of `SDL_GetBlendModeColorOperation()`.
    pub fn color_operation(self) -> Option<BO> {
        BO::from_u32(long_blend_mode(self).0 & 0xF)
    }

    /// Translation of `SDL_GetBlendModeSrcAlphaFactor()`.
    pub fn src_alpha_factor(self) -> Option<BF> {
        BF::from_u32((long_blend_mode(self).0 >> 20) & 0xF)
    }

    /// Translation of `SDL_GetBlendModeDstAlphaFactor()`.
    pub fn dst_alpha_factor(self) -> Option<BF> {
        BF::from_u32((long_blend_mode(self).0 >> 24) & 0xF)
    }

    /// Translation of `SDL_GetBlendModeAlphaOperation()`.
    pub fn alpha_operation(self) -> Option<BO> {
        BO::from_u32((long_blend_mode(self).0 >> 16) & 0xF)
    }
}

#[cfg(test)]
mod tests;
