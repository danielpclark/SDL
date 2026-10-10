// Rust translation of src/render/direct3d12/SDL_render_d3d12.c from Simple
// DirectMedia Layer (the shader tables of SDL_shaders_d3d12.c are in
// shaders.rs).
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! The Direct3D 12 renderer ("direct3d12", after "direct3d11" on Windows):
//! draws through a Direct3D 12 device, one command queue and command list
//! with an allocator per back buffer and a fence, and a flip-model DXGI
//! swap chain on the window's `HWND`. `dxgi.dll` and `d3d12.dll` are loaded
//! at run time, as upstream does; the COM interfaces are the GPU API's
//! Direct3D 12 declarations ([`crate::gpu::d3d12::d3d`]) and the Direct3D
//! 11 renderer's DXGI ones ([`crate::render::direct3d11::d3d`]).
//!
//! The shaders are DXIL (shader model 6.0), so the renderer needs a device
//! that takes it: without one, making the default pipeline states fails and
//! the renderer isn't created (the next driver is tried).
//!
//! What isn't translated:
//!
//! * the Xbox (GDK) paths, which upstream compiles for the Xbox only.
//! * the creation options for existing textures
//!   (`SDL_PROP_TEXTURE_CREATE_D3D12_TEXTURE_*`), which the front end
//!   doesn't take: the renderer makes all its textures.
//!
//! The front end only makes renderers with sRGB output yet, so the linear
//! and HDR10 output paths here are kept for when it does.
//!
//! Upstream keeps the per-texture data in `texture->internal` and walks the
//! renderer's texture list to free it when the device is lost. Here the
//! renderer keeps its textures (and palettes) itself, and the front end's
//! texture refers to them by a key. The COM references are [`ComPtr`]s,
//! released when dropped (upstream's `D3D_SAFE_RELEASE()`); the pointers
//! that resource barriers and copy locations hold are the renderer's own
//! references, which it keeps until the GPU is done with the command list
//! (as upstream does, by executing the list before it releases them).
//!
//! [`ComPtr`]: crate::core::windows::com::ComPtr

mod shaders;
#[cfg(test)]
mod tests;

use std::any::Any;
use std::collections::HashMap;
use std::ffi::c_void;
use std::ptr::{null, null_mut, NonNull};

use windows_sys::Win32::Foundation::{HANDLE, HWND};
use windows_sys::Win32::System::LibraryLoader::{GetModuleHandleW, GetProcAddress};
use windows_sys::Win32::System::Threading::{WaitForSingleObjectEx, INFINITE};

use shaders::{RootSignature, Shader};

use crate::core::windows::com::ComPtr;
use crate::core::windows::is_windows_8_or_greater;
use crate::error::{Error, Result};
use crate::events::window::WindowFlags;
use crate::events::{Event, EventType, RenderEvent};
use crate::gpu::d3d12::d3d::*;
use crate::hints;
use crate::loadso::SharedObject;
use crate::log::Category;
use crate::properties::Properties;
use crate::render::direct3d11::d3d::{
    check, out, raw, DxgiDebug, DxgiFactory6, DxgiFormat, DxgiInfoQueue, DxgiModeRotation,
    IDXGIDebugVtbl, IDXGIFactory6Vtbl, IDXGIInfoQueueVtbl, PfnCreateDxgiFactory2, SampleDesc,
    D3D_FEATURE_LEVEL_11_0, DXGI_COLOR_SPACE_RGB_FULL_G10_NONE_P709,
    DXGI_COLOR_SPACE_RGB_FULL_G2084_NONE_P2020, DXGI_COLOR_SPACE_RGB_FULL_G22_NONE_P709,
    DXGI_CREATE_FACTORY_DEBUG, DXGI_DEBUG_ALL, DXGI_DEBUG_RLO_DETAIL,
    DXGI_DEBUG_RLO_IGNORE_INTERNAL, DXGI_ERROR_DEVICE_REMOVED, DXGI_ERROR_INVALID_CALL,
    DXGI_ERROR_WAS_STILL_DRAWING, DXGI_FORMAT_B4G4R4A4_UNORM, DXGI_FORMAT_B5G5R5A1_UNORM,
    DXGI_FORMAT_B5G6R5_UNORM, DXGI_FORMAT_B8G8R8A8_UNORM, DXGI_FORMAT_B8G8R8A8_UNORM_SRGB,
    DXGI_FORMAT_B8G8R8X8_UNORM, DXGI_FORMAT_B8G8R8X8_UNORM_SRGB, DXGI_FORMAT_NV12,
    DXGI_FORMAT_P010, DXGI_FORMAT_R10G10B10A2_UNORM, DXGI_FORMAT_R16G16B16A16_FLOAT,
    DXGI_FORMAT_R16G16_UNORM, DXGI_FORMAT_R16_UNORM, DXGI_FORMAT_R32G32B32A32_FLOAT,
    DXGI_FORMAT_R32G32_FLOAT, DXGI_FORMAT_R8G8B8A8_UNORM, DXGI_FORMAT_R8G8B8A8_UNORM_SRGB,
    DXGI_FORMAT_R8G8_UNORM, DXGI_FORMAT_R8_UNORM, DXGI_FORMAT_UNKNOWN,
    DXGI_GPU_PREFERENCE_HIGH_PERFORMANCE, DXGI_INFO_QUEUE_MESSAGE_SEVERITY_CORRUPTION,
    DXGI_INFO_QUEUE_MESSAGE_SEVERITY_ERROR, DXGI_MODE_ROTATION_IDENTITY,
    DXGI_MODE_ROTATION_ROTATE180, DXGI_MODE_ROTATION_ROTATE270, DXGI_MODE_ROTATION_ROTATE90,
    DXGI_MODE_ROTATION_UNSPECIFIED, DXGI_SCALING_STRETCH, DXGI_SWAP_EFFECT_FLIP_SEQUENTIAL,
    IID_IDXGIDEBUG1, IID_IDXGIFACTORY6, IID_IDXGIINFOQUEUE,
};
use crate::render::direct3d11::{
    copy_rows, hr_error, plane, render_sampler_hashkey, ycbcr_matrix, Float4X4,
    RENDER_SAMPLER_COUNT,
};
use crate::render::sysrender::{
    invalid_texture, CopyEx, DrawCmd, DrawKind, Geometry, RenderBackend, RenderCommand,
    TextureCreateProps, TextureData, TextureStore,
};
use crate::render::{Texture, TextureAccess, TextureAddressMode};
use crate::video::blendmode::{BlendFactor, BlendOperation};
use crate::video::pixels::{
    convert_color_709_to_2020, pq_from_nits, srgb_to_linear, Color, Colorspace, FColor,
    PixelFormat, TransferCharacteristics,
};
use crate::video::rect::{FPoint, FRect, Rect};
use crate::video::surface::{ScaleMode, Surface};
use crate::video::window::PROP_WINDOW_WIN32_HWND_POINTER;
use crate::video::{BlendMode, Window};

/// The name of the Direct3D 12 renderer (`D3D12_RenderDriver.name`).
pub(crate) const D3D12_RENDERER: &str = "direct3d12";

/// The `ID3D12Device` of the renderer. Translation of
/// `SDL_PROP_RENDERER_D3D12_DEVICE_POINTER`.
pub const PROP_RENDERER_D3D12_DEVICE_POINTER: &str = "SDL.renderer.d3d12.device";
/// The `IDXGISwapChain4` of the renderer. Translation of
/// `SDL_PROP_RENDERER_D3D12_SWAPCHAIN_POINTER`.
pub const PROP_RENDERER_D3D12_SWAPCHAIN_POINTER: &str = "SDL.renderer.d3d12.swap_chain";
/// The `ID3D12CommandQueue` of the renderer. Translation of
/// `SDL_PROP_RENDERER_D3D12_COMMAND_QUEUE_POINTER`.
pub const PROP_RENDERER_D3D12_COMMAND_QUEUE_POINTER: &str = "SDL.renderer.d3d12.command_queue";
/// The `ID3D12Resource` of a texture (its Y plane for YUV textures).
/// Translation of `SDL_PROP_TEXTURE_D3D12_TEXTURE_POINTER`.
pub const PROP_TEXTURE_D3D12_TEXTURE_POINTER: &str = "SDL.texture.d3d12.texture";
/// The `ID3D12Resource` of the U plane of a YUV texture. Translation of
/// `SDL_PROP_TEXTURE_D3D12_TEXTURE_U_POINTER`.
pub const PROP_TEXTURE_D3D12_TEXTURE_U_POINTER: &str = "SDL.texture.d3d12.texture_u";
/// The `ID3D12Resource` of the V plane of a YUV texture. Translation of
/// `SDL_PROP_TEXTURE_D3D12_TEXTURE_V_POINTER`.
pub const PROP_TEXTURE_D3D12_TEXTURE_V_POINTER: &str = "SDL.texture.d3d12.texture_v";

const SDL_D3D12_NUM_BUFFERS: usize = 2;
const SDL_D3D12_NUM_VERTEX_BUFFERS: usize = 256;
const SDL_D3D12_MAX_NUM_TEXTURES: usize = 16384;
const SDL_D3D12_NUM_UPLOAD_BUFFERS: usize = 32;

/// The SDR white level of scRGB content (`SCRGB_NITS`).
const SCRGB_NITS: f32 = 80.0;

/// The access rights of the fence event (`EVENT_MODIFY_STATE | SYNCHRONIZE`).
const EVENT_MODIFY_STATE: u32 = 0x0002;
const SYNCHRONIZE: u32 = 0x0010_0000;

/// The error of a renderer whose device was lost and not recovered.
fn device_lost() -> Error {
    Error::new("Device lost and couldn't be recovered")
}

/// The error of a texture without renderer data.
fn not_available() -> Error {
    Error::new("Texture is not currently available")
}

/* !!! FIXME: vertex buffer bandwidth could be lower; only use UV coords when
!!! FIXME:  textures are needed. */

/// Vertex shader, common values. Translation of
/// `D3D12_VertexShaderConstants`.
#[repr(C)]
#[derive(Clone, Copy, Default)]
struct VertexShaderConstants {
    mpv: Float4X4,
}

impl VertexShaderConstants {
    /// The constants as the 32-bit values the root constants take.
    fn words(&self) -> [u32; 16] {
        let mut words = [0; 16];
        for (w, f) in words.iter_mut().zip(self.mpv.m.as_flattened()) {
            *w = f.to_bits();
        }
        words
    }
}

// These should mirror the definitions in D3D12_PixelShader_Common.hlsli
const TONEMAP_NONE: f32 = 0.0;
//const TONEMAP_LINEAR: f32 = 1.0;
const TONEMAP_CHROME: f32 = 2.0;

//const TEXTURETYPE_NONE: f32 = 0.0;
const TEXTURETYPE_RGB: f32 = 1.0;
const TEXTURETYPE_RGB_PIXELART: f32 = 2.0;
const TEXTURETYPE_PALETTE_NEAREST: f32 = 3.0;
const TEXTURETYPE_PALETTE_LINEAR: f32 = 4.0;
const TEXTURETYPE_PALETTE_PIXELART: f32 = 5.0;
const TEXTURETYPE_NV12: f32 = 6.0;
const TEXTURETYPE_NV21: f32 = 7.0;
const TEXTURETYPE_YUV: f32 = 8.0;

const INPUTTYPE_UNSPECIFIED: f32 = 0.0;
const INPUTTYPE_SRGB: f32 = 1.0;
const INPUTTYPE_SCRGB: f32 = 2.0;
const INPUTTYPE_HDR10: f32 = 3.0;

/// Pixel shader constants. Translation of `D3D12_PixelShaderConstants`.
#[repr(C)]
#[derive(Clone, Copy, Default, Debug)]
struct PixelShaderConstants {
    sc_rgb_output: f32,
    texture_type: f32,
    input_type: f32,
    color_scale: f32,

    texel_width: f32,
    texel_height: f32,
    texture_width: f32,
    texture_height: f32,

    tonemap_method: f32,
    tonemap_factor1: f32,
    tonemap_factor2: f32,
    sdr_white_point: f32,

    ycbcr_matrix: [f32; 16],
}

impl PixelShaderConstants {
    /// The constants as the 32-bit values the root constants take.
    fn words(&self) -> [u32; 28] {
        let mut words = [0; 28];
        let head = [
            self.sc_rgb_output,
            self.texture_type,
            self.input_type,
            self.color_scale,
            self.texel_width,
            self.texel_height,
            self.texture_width,
            self.texture_height,
            self.tonemap_method,
            self.tonemap_factor1,
            self.tonemap_factor2,
            self.sdr_white_point,
        ];
        for (w, f) in words.iter_mut().zip(head.iter().chain(&self.ycbcr_matrix)) {
            *w = f.to_bits();
        }
        words
    }
}

/// Per-vertex data. Translation of `D3D12_VertexPositionColor`.
#[repr(C)]
#[derive(Clone, Copy, Default, Debug)]
struct VertexPositionColor {
    pos: [f32; 2],
    tex: [f32; 2],
    color: [f32; 4],
}

impl VertexPositionColor {
    fn new(x: f32, y: f32, u: f32, v: f32, color: FColor) -> VertexPositionColor {
        VertexPositionColor {
            pos: [x, y],
            tex: [u, v],
            color: [color.r, color.g, color.b, color.a],
        }
    }
}

/// Per-palette data. Translation of `D3D12_PaletteData`.
struct PaletteData {
    texture: Option<D3d12Resource>,
    resource_view: CpuDescriptorHandle,
    resource_state: u32,
    srv_index: Option<usize>,
}

/// The front end's handle of a palette: its key in the renderer.
struct PaletteRef(u32);

/// Per-texture data. Translation of `D3D12_TextureData` (without the
/// `stagingResourceState` upstream never reads). The SRV indices are
/// `None` until allocated.
#[derive(Default)]
struct D3d12Texture {
    w: i32,
    h: i32,
    main_texture: Option<D3d12Resource>,
    main_texture_resource_view: CpuDescriptorHandle,
    main_resource_state: u32,
    main_srv_index: Option<usize>,
    main_texture_render_target_view: CpuDescriptorHandle,
    main_texture_format: DxgiFormat,
    staging_buffer: Option<D3d12Resource>,
    ycbcr_matrix: Option<[f32; 16]>,
    // YV12 texture support
    yuv: bool,
    main_texture_u: Option<D3d12Resource>,
    main_texture_resource_view_u: CpuDescriptorHandle,
    main_resource_state_u: u32,
    main_srv_index_u: Option<usize>,
    main_texture_v: Option<D3d12Resource>,
    main_texture_resource_view_v: CpuDescriptorHandle,
    main_resource_state_v: u32,
    main_srv_index_v: Option<usize>,

    // NV12 texture support
    nv12: bool,
    main_texture_resource_view_nv: CpuDescriptorHandle,
    main_srv_index_nv: Option<usize>,

    pitch: i32,
    locked_rect: Rect,

    /// The key of the texture's palette (`texture->palette->internal`),
    /// as the front end last changed it.
    palette: Option<u32>,
}

/// A plane of a texture: its main (Y) texture, or the U or V one.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Plane {
    Main,
    U,
    V,
}

impl D3d12Texture {
    /// The resource of a plane and its state (a new reference, so the
    /// renderer can record commands while it updates the state).
    fn plane(&self, plane: Plane) -> Option<(D3d12Resource, u32)> {
        let (resource, state) = match plane {
            Plane::Main => (&self.main_texture, self.main_resource_state),
            Plane::U => (&self.main_texture_u, self.main_resource_state_u),
            Plane::V => (&self.main_texture_v, self.main_resource_state_v),
        };
        Some((resource.clone()?, state))
    }

    /// The state of a plane's resource.
    fn state_mut(&mut self, plane: Plane) -> &mut u32 {
        match plane {
            Plane::Main => &mut self.main_resource_state,
            Plane::U => &mut self.main_resource_state_u,
            Plane::V => &mut self.main_resource_state_v,
        }
    }
}

/// What the front end's texture holds (`texture->internal`): the key of
/// the texture in the renderer, and the pixels its lock writes.
struct TextureRef {
    id: u32,
    /// The pixels of a YUV texture (`textureData->pixels`), kept from its
    /// first lock on.
    pixels: Vec<u8>,
    /// The mapped staging buffer of a locked texture.
    staging: Option<(NonNull<u8>, usize)>,
}

fn texture_ref(texture: &TextureData) -> Option<&TextureRef> {
    texture.internal.as_ref()?.downcast_ref::<TextureRef>()
}

fn texture_ref_mut(texture: &mut TextureData) -> Option<&mut TextureRef> {
    texture.internal.as_mut()?.downcast_mut::<TextureRef>()
}

/// Pipeline State Object data. Translation of `D3D12_PipelineState`.
struct PipelineState {
    shader: Shader,
    shader_constants: PixelShaderConstants,
    blend_mode: BlendMode,
    topology: u32,
    rtv_format: DxgiFormat,
    pipeline_state: D3d12PipelineState,
}

/// Vertex Buffer. Translation of `D3D12_VertexBuffer`.
#[derive(Default)]
struct VertexBuffer {
    resource: Option<D3d12Resource>,
    view: VertexBufferView,
    size: usize,
}

/// For SRV pool allocator. Translation of `D3D12_SRVPoolNode` (`next` is
/// the index of the next node).
#[derive(Clone, Copy, Default)]
struct SrvPoolNode {
    index: usize,
    next: Option<usize>,
}

/// The front end's view of the texture a draw copies from (what upstream
/// reads of `SDL_Texture`).
struct SourceTexture {
    format: PixelFormat,
    w: i32,
    h: i32,
    colorspace: Colorspace,
    sdr_white_point: f32,
    hdr_headroom: f32,
    has_palette: bool,
}

impl SourceTexture {
    fn of(texture: &TextureData) -> SourceTexture {
        SourceTexture {
            format: texture.format,
            w: texture.w,
            h: texture.h,
            colorspace: texture.colorspace,
            sdr_white_point: texture.sdr_white_point,
            hdr_headroom: texture.hdr_headroom,
            has_palette: texture.palette.is_some(),
        }
    }
}

/// Private renderer data. Translation of `D3D12_RenderData`, with the
/// parts of `SDL_Renderer` the backend reads (`window`,
/// `output_colorspace`, `current_colorspace`, the HDR headroom, `target`),
/// its textures and palettes, and its vertex storage.
///
/// The objects made on the device come before it, and the libraries
/// last, so that a renderer dropped without [`RenderBackend::destroy`]
/// still releases them in an order that works.
pub(crate) struct D3d12Renderer {
    window: Window,
    output_colorspace: Colorspace,
    current_colorspace: Colorspace,
    /// `renderer->HDR_headroom`: 1 for the sRGB output the front end makes.
    hdr_headroom: f32,
    /// `renderer->target`, the front end's texture.
    target: Option<Texture>,

    /// The textures, by the key their [`TextureRef`] holds.
    textures: HashMap<u32, D3d12Texture>,
    next_texture: u32,
    /// The palettes, by the key their [`PaletteRef`] holds.
    palettes: HashMap<u32, PaletteData>,
    next_palette: u32,
    /// The vertex data of the queued commands (`first` indexes it).
    verts: Vec<VertexPositionColor>,
    texture_formats: Vec<PixelFormat>,
    /// The renderer's properties, once the front end has made them.
    props: Option<Properties>,

    swap_effect: u32,
    swap_flags: u32,
    sync_interval: u32,
    present_flags: u32,
    render_target_format: DxgiFormat,
    pixel_size_changed: bool,

    // Descriptor heaps
    rtv_descriptor_heap: Option<D3d12DescriptorHeap>,
    rtv_descriptor_size: u32,
    texture_rtv_descriptor_heap: Option<D3d12DescriptorHeap>,
    srv_descriptor_heap: Option<D3d12DescriptorHeap>,
    srv_descriptor_size: u32,
    sampler_descriptor_heap: Option<D3d12DescriptorHeap>,
    sampler_descriptor_size: u32,

    // Data needed per backbuffer
    command_allocators: [Option<D3d12CommandAllocator>; SDL_D3D12_NUM_BUFFERS],
    render_targets: [Option<D3d12Resource>; SDL_D3D12_NUM_BUFFERS],
    fence_value: u64,
    current_back_buffer_index: usize,

    // Fences
    fence: Option<D3d12Fence>,
    fence_event: HANDLE,

    // Root signature and pipeline state data
    root_signatures: [Option<D3d12RootSignature>; RootSignature::COUNT],
    pipeline_states: Vec<PipelineState>,
    /// The index of the current pipeline state in `pipeline_states`.
    current_pipeline_state: Option<usize>,

    vertex_buffers: Vec<VertexBuffer>,
    samplers: [CpuDescriptorHandle; RENDER_SAMPLER_COUNT],
    samplers_created: [bool; RENDER_SAMPLER_COUNT],

    // Data for staging/allocating textures
    upload_buffers: [Option<D3d12Resource>; SDL_D3D12_NUM_UPLOAD_BUFFERS],
    current_upload_buffer: usize,

    // Pool allocator to handle reusing SRV heap indices
    srv_pool_head: Option<usize>,
    srv_pool_nodes: Vec<SrvPoolNode>,

    // Vertex buffer constants
    projection_and_view: Float4X4,

    // Cached renderer properties
    rotation: DxgiModeRotation,
    /// The key of the target texture (`textureRenderTarget`).
    texture_render_target: Option<u32>,
    current_render_target_view: CpuDescriptorHandle,
    num_current_shader_resources: usize,
    current_shader_resource: CpuDescriptorHandle,
    num_current_shader_samplers: usize,
    current_shader_sampler: CpuDescriptorHandle,
    cliprect_dirty: bool,
    current_cliprect_enabled: bool,
    current_cliprect: Rect,
    current_viewport: Rect,
    current_viewport_rotation: DxgiModeRotation,
    viewport_dirty: bool,
    #[allow(dead_code)] // (set, never read, as upstream)
    identity: Float4X4,
    current_vertex_buffer: usize,
    issue_batch: bool,

    command_list: Option<D3d12GraphicsCommandList>,
    command_queue: Option<D3d12CommandQueue>,
    debug_interface: Option<D3d12Debug>,
    d3d_device: Option<D3d12Device>,
    swap_chain: Option<DxgiSwapChain3>,
    dxgi_debug: Option<DxgiDebug>,
    dxgi_adapter: Option<ComPtr<OpaqueVtbl>>,
    dxgi_factory: Option<DxgiFactory6>,

    h_dxgi_mod: Option<SharedObject>,
    h_d3d12_mod: Option<SharedObject>,
}

/// Translation of `D3D12_Align()`.
fn d3d12_align(location: u32, alignment: u32) -> u32 {
    (location + (alignment - 1)) & !(alignment - 1)
}

/// Translation of `dxgi_format_map`: (SDL format, UNORM format, sRGB format).
const DXGI_FORMAT_MAP: &[(PixelFormat, DxgiFormat, DxgiFormat)] = &[
    (
        PixelFormat::ARGB8888,
        DXGI_FORMAT_B8G8R8A8_UNORM,
        DXGI_FORMAT_B8G8R8A8_UNORM_SRGB,
    ),
    (
        PixelFormat::ABGR8888,
        DXGI_FORMAT_R8G8B8A8_UNORM,
        DXGI_FORMAT_R8G8B8A8_UNORM_SRGB,
    ),
    (
        PixelFormat::XRGB8888,
        DXGI_FORMAT_B8G8R8X8_UNORM,
        DXGI_FORMAT_B8G8R8X8_UNORM_SRGB,
    ),
    (
        PixelFormat::ABGR2101010,
        DXGI_FORMAT_R10G10B10A2_UNORM,
        DXGI_FORMAT_R10G10B10A2_UNORM,
    ),
    (
        PixelFormat::RGBA64_FLOAT,
        DXGI_FORMAT_R16G16B16A16_FLOAT,
        DXGI_FORMAT_R16G16B16A16_FLOAT,
    ),
    (
        PixelFormat::RGB565,
        DXGI_FORMAT_B5G6R5_UNORM,
        DXGI_FORMAT_B5G6R5_UNORM,
    ),
    (
        PixelFormat::ARGB1555,
        DXGI_FORMAT_B5G5R5A1_UNORM,
        DXGI_FORMAT_B5G5R5A1_UNORM,
    ),
    (
        PixelFormat::ARGB4444,
        DXGI_FORMAT_B4G4R4A4_UNORM,
        DXGI_FORMAT_B4G4R4A4_UNORM,
    ),
];

/// Translation of `D3D12_DXGIFormatToSDLPixelFormat()`.
fn dxgi_format_to_sdl_pixel_format(dxgi_format: DxgiFormat) -> PixelFormat {
    DXGI_FORMAT_MAP
        .iter()
        .find(|&&(_, unorm, srgb)| unorm == dxgi_format || srgb == dxgi_format)
        .map_or(PixelFormat::UNKNOWN, |&(sdl, _, _)| sdl)
}

/// Translation of `SDLPixelFormatToDXGITextureFormat()`.
fn sdl_pixel_format_to_dxgi_texture_format(
    format: PixelFormat,
    output_colorspace: Colorspace,
) -> DxgiFormat {
    use PixelFormat as F;
    match format {
        F::INDEX8 | F::YV12 | F::IYUV | F::I444 => DXGI_FORMAT_R8_UNORM,
        F::NV12 | F::NV21 => DXGI_FORMAT_NV12,
        F::P010 => DXGI_FORMAT_P010,
        F::I0FL | F::I4FL => DXGI_FORMAT_R16_UNORM,
        _ => DXGI_FORMAT_MAP
            .iter()
            .find(|&&(sdl, _, _)| sdl == format)
            .map_or(DXGI_FORMAT_UNKNOWN, |&(_, unorm, srgb)| {
                if output_colorspace == Colorspace::SRGB_LINEAR
                    || output_colorspace == Colorspace::HDR10
                {
                    srgb
                } else {
                    unorm
                }
            }),
    }
}

/// Translation of `SDLPixelFormatToDXGIMainResourceViewFormat()`.
fn sdl_pixel_format_to_dxgi_main_resource_view_format(
    format: PixelFormat,
    colorspace: Colorspace,
) -> DxgiFormat {
    use PixelFormat as F;
    match format {
        F::NV12 | F::NV21 => DXGI_FORMAT_R8_UNORM, // For the Y texture
        F::P010 => DXGI_FORMAT_R16_UNORM,          // For the Y texture
        _ => sdl_pixel_format_to_dxgi_texture_format(format, colorspace),
    }
}

/// Translation of `D3D12_CPUtoGPUHandle()`.
fn cpu_to_gpu_handle(
    heap: &D3d12DescriptorHeap,
    cpu_handle: CpuDescriptorHandle,
) -> GpuDescriptorHandle {
    // Calculate the correct offset into the heap
    let cpu_heap_start = heap.cpu_descriptor_handle_for_heap_start();
    let offset = cpu_handle.ptr - cpu_heap_start.ptr;

    let mut gpu_handle = heap.gpu_descriptor_handle_for_heap_start();
    gpu_handle.ptr += offset as u64;

    gpu_handle
}

/// Translation of `GetBlendFunc()` (0 for no factor).
fn get_blend_func(factor: Option<BlendFactor>) -> u32 {
    match factor {
        Some(BlendFactor::Zero) => D3D12_BLEND_ZERO,
        Some(BlendFactor::One) => D3D12_BLEND_ONE,
        Some(BlendFactor::SrcColor) => D3D12_BLEND_SRC_COLOR,
        Some(BlendFactor::OneMinusSrcColor) => D3D12_BLEND_INV_SRC_COLOR,
        Some(BlendFactor::SrcAlpha) => D3D12_BLEND_SRC_ALPHA,
        Some(BlendFactor::OneMinusSrcAlpha) => D3D12_BLEND_INV_SRC_ALPHA,
        Some(BlendFactor::DstColor) => D3D12_BLEND_DEST_COLOR,
        Some(BlendFactor::OneMinusDstColor) => D3D12_BLEND_INV_DEST_COLOR,
        Some(BlendFactor::DstAlpha) => D3D12_BLEND_DEST_ALPHA,
        Some(BlendFactor::OneMinusDstAlpha) => D3D12_BLEND_INV_DEST_ALPHA,
        None => 0,
    }
}

/// Translation of `GetBlendEquation()` (0 for no operation).
fn get_blend_equation(operation: Option<BlendOperation>) -> u32 {
    match operation {
        Some(BlendOperation::Add) => D3D12_BLEND_OP_ADD,
        Some(BlendOperation::Subtract) => D3D12_BLEND_OP_SUBTRACT,
        Some(BlendOperation::RevSubtract) => D3D12_BLEND_OP_REV_SUBTRACT,
        Some(BlendOperation::Minimum) => D3D12_BLEND_OP_MIN,
        Some(BlendOperation::Maximum) => D3D12_BLEND_OP_MAX,
        None => 0,
    }
}

/// Translation of `D3D12_CreateBlendState()`.
fn create_blend_state(blend_mode: BlendMode) -> BlendDesc {
    let src_color_factor = blend_mode.src_color_factor();
    let src_alpha_factor = blend_mode.src_alpha_factor();
    let color_operation = blend_mode.color_operation();
    let dst_color_factor = blend_mode.dst_color_factor();
    let dst_alpha_factor = blend_mode.dst_alpha_factor();
    let alpha_operation = blend_mode.alpha_operation();

    let mut out_blend_desc = BlendDesc {
        alpha_to_coverage_enable: 0,
        independent_blend_enable: 0,
        ..Default::default()
    };
    out_blend_desc.render_target[0] = RenderTargetBlendDesc {
        blend_enable: 1,
        src_blend: get_blend_func(src_color_factor),
        dest_blend: get_blend_func(dst_color_factor),
        blend_op: get_blend_equation(color_operation),
        src_blend_alpha: get_blend_func(src_alpha_factor),
        dest_blend_alpha: get_blend_func(dst_alpha_factor),
        blend_op_alpha: get_blend_equation(alpha_operation),
        render_target_write_mask: D3D12_COLOR_WRITE_ENABLE_ALL,
        ..Default::default()
    };
    out_blend_desc
}

/// Translation of `D3D12_GetCurrentRotation()`.
fn get_current_rotation() -> DxgiModeRotation {
    // FIXME
    DXGI_MODE_ROTATION_IDENTITY
}

/// Translation of `D3D12_IsDisplayRotated90Degrees()`.
fn is_display_rotated_90_degrees(rotation: DxgiModeRotation) -> bool {
    matches!(
        rotation,
        DXGI_MODE_ROTATION_ROTATE90 | DXGI_MODE_ROTATION_ROTATE270
    )
}

/// Translation of `PQShaderScalesInput()`.
fn pq_shader_scales_input(shader_constants: &PixelShaderConstants) -> bool {
    if shader_constants.tonemap_method != 0.0 {
        // Tone mapping always scales
        return true;
    }

    // The shader normalizes the PQ input using the SDR white point and then multiplies by the color scale
    if (shader_constants.sdr_white_point - (shader_constants.color_scale * SCRGB_NITS)).abs() > 1.0
    {
        return true;
    }

    false
}

/// Translation of `SelectShader()`, for the colorspace drawn in.
fn select_shader(
    current_colorspace: Colorspace,
    shader_constants: Option<&PixelShaderConstants>,
) -> Shader {
    match shader_constants {
        Some(shader_constants) => {
            if current_colorspace == Colorspace::HDR10 {
                if shader_constants.texture_type == TEXTURETYPE_RGB
                    && shader_constants.input_type == INPUTTYPE_HDR10
                    && !pq_shader_scales_input(shader_constants)
                {
                    // Do a simple 1-1 copy
                    return Shader::RgbSimple;
                } else {
                    return Shader::RgbPq;
                }
            }

            if shader_constants.texture_type == TEXTURETYPE_RGB
                && shader_constants.input_type == INPUTTYPE_UNSPECIFIED
                && shader_constants.tonemap_method == TONEMAP_NONE
            {
                return Shader::Rgb;
            }

            Shader::Advanced
        }
        None => {
            if current_colorspace == Colorspace::HDR10 {
                return Shader::SolidPq;
            }

            Shader::Solid
        }
    }
}

/// The sampler description of `D3D12_GetSamplerState()`.
fn sampler_desc(
    scale_mode: ScaleMode,
    address_u: TextureAddressMode,
    address_v: TextureAddressMode,
) -> Result<SamplerDesc> {
    let address_mode = |mode: TextureAddressMode| match mode {
        TextureAddressMode::Clamp => Ok(D3D12_TEXTURE_ADDRESS_MODE_CLAMP),
        TextureAddressMode::Wrap => Ok(D3D12_TEXTURE_ADDRESS_MODE_WRAP),
        other => Err(Error::new(format!(
            "Unknown texture address mode: {}",
            other as i32
        ))),
    };
    Ok(SamplerDesc {
        address_w: D3D12_TEXTURE_ADDRESS_MODE_CLAMP,
        mip_lod_bias: 0.0,
        max_anisotropy: 1,
        // (the Xbox's D3D12_COMPARISON_FUNC_ALWAYS isn't translated)
        comparison_func: D3D12_COMPARISON_FUNC_NONE,
        min_lod: 0.0,
        max_lod: D3D12_FLOAT32_MAX,
        filter: match scale_mode {
            ScaleMode::Nearest => D3D12_FILTER_MIN_MAG_MIP_POINT,
            // Uses linear sampling
            ScaleMode::PixelArt | ScaleMode::Linear => D3D12_FILTER_MIN_MAG_MIP_LINEAR,
        },
        address_u: address_mode(address_u)?,
        address_v: address_mode(address_v)?,
        ..Default::default()
    })
}

/// A transition barrier of all of `resource`'s subresources.
fn transition_barrier(resource: &D3d12Resource, before: u32, after: u32) -> ResourceBarrier {
    ResourceBarrier {
        ty: D3D12_RESOURCE_BARRIER_TYPE_TRANSITION,
        flags: D3D12_RESOURCE_BARRIER_FLAG_NONE,
        u: ResourceBarrierUnion {
            transition: ResourceTransitionBarrier {
                resource: raw(resource),
                subresource: D3D12_RESOURCE_BARRIER_ALL_SUBRESOURCES,
                state_before: before,
                state_after: after,
            },
        },
    }
}

/// The description of an upload or readback buffer
/// (`uploadDesc`/`readbackDesc` and `vbufferDesc`) of `width` bytes.
fn buffer_desc(width: u64) -> ResourceDesc {
    ResourceDesc {
        dimension: D3D12_RESOURCE_DIMENSION_BUFFER,
        alignment: D3D12_DEFAULT_RESOURCE_PLACEMENT_ALIGNMENT,
        width,
        height: 1,
        depth_or_array_size: 1,
        mip_levels: 1,
        format: DXGI_FORMAT_UNKNOWN,
        sample_desc: SampleDesc {
            count: 1,
            quality: 0,
        },
        layout: D3D12_TEXTURE_LAYOUT_ROW_MAJOR,
        flags: D3D12_RESOURCE_FLAG_NONE,
    }
}

/// The heap properties of `heap_type` on the first node.
fn heap_properties(heap_type: u32) -> HeapProperties {
    HeapProperties {
        ty: heap_type,
        creation_node_mask: 1,
        visible_node_mask: 1,
        ..Default::default()
    }
}

/// A copy location of a subresource of a texture.
fn subresource_location(resource: &D3d12Resource, index: u32) -> TextureCopyLocation {
    TextureCopyLocation {
        resource: raw(resource),
        ty: D3D12_TEXTURE_COPY_TYPE_SUBRESOURCE_INDEX,
        u: TextureCopyLocationUnion {
            subresource_index: index,
        },
    }
}

/// A copy location of a footprint in a buffer.
fn footprint_location(
    resource: &D3d12Resource,
    footprint: PlacedSubresourceFootprint,
) -> TextureCopyLocation {
    TextureCopyLocation {
        resource: raw(resource),
        ty: D3D12_TEXTURE_COPY_TYPE_PLACED_FOOTPRINT,
        u: TextureCopyLocationUnion {
            placed_footprint: footprint,
        },
    }
}

/// The description of a 2D texture's shader resource view.
fn texture_2d_srv_desc(
    format: DxgiFormat,
    mip_levels: u32,
    plane_slice: u32,
) -> ShaderResourceViewDesc {
    ShaderResourceViewDesc {
        format,
        view_dimension: D3D12_SRV_DIMENSION_TEXTURE2D,
        shader_4_component_mapping: D3D12_DEFAULT_SHADER_4_COMPONENT_MAPPING,
        u: SrvUnion {
            texture_2d: Tex2dSrv {
                most_detailed_mip: 0,
                mip_levels,
                plane_slice,
                resource_min_lod_clamp: 0.0,
            },
        },
    }
}

/// The description of a 2D texture's render target view.
fn texture_2d_rtv_desc(format: DxgiFormat) -> RenderTargetViewDesc {
    RenderTargetViewDesc {
        format,
        view_dimension: D3D12_RTV_DIMENSION_TEXTURE2D,
        ..Default::default()
    }
}

/// The address of the descriptor `index` of a heap whose descriptors are
/// `size` bytes apart.
fn descriptor_at(heap: &D3d12DescriptorHeap, index: usize, size: u32) -> CpuDescriptorHandle {
    let mut handle = heap.cpu_descriptor_handle_for_heap_start();
    handle.ptr += index * size as usize;
    handle
}

impl D3d12Renderer {
    /// The device, or the error of a lost one.
    fn device(&self) -> Result<&D3d12Device> {
        self.d3d_device.as_ref().ok_or_else(device_lost)
    }

    /// Translation of `SDL_RenderingLinearSpace()`.
    fn rendering_linear_space(&self) -> bool {
        self.current_colorspace.transfer() == TransferCharacteristics::Linear
    }

    /// Translation of `SDL_ConvertToLinear()`.
    fn convert_to_linear(&self, color: &mut FColor) {
        color.r = srgb_to_linear(color.r);
        color.g = srgb_to_linear(color.g);
        color.b = srgb_to_linear(color.b);
        if self.current_colorspace == Colorspace::HDR10 {
            let [r, g, b] = convert_color_709_to_2020([color.r, color.g, color.b]);
            (color.r, color.g, color.b) = (r, g, b);
        }
    }

    /// The window's `HWND` (`SDL_PROP_WINDOW_WIN32_HWND_POINTER`).
    fn hwnd(&self) -> Option<HWND> {
        let props = self.window.properties().ok()?;
        let hwnd = props
            .get_number(PROP_WINDOW_WIN32_HWND_POINTER)
            .unwrap_or(0);
        (hwnd != 0).then_some(hwnd as HWND)
    }

    /// Set (or, without the objects, clear) the device, command queue and
    /// swap chain properties of the renderer (`SDL_SetPointerProperty()`
    /// in `D3D12_CreateDeviceResources()`, `D3D12_CreateSwapChain()` and
    /// `D3D12_ReleaseAll()`).
    fn publish_properties(&self) {
        let Some(props) = &self.props else {
            return;
        };
        let set = |name: &str, ptr: Option<*mut c_void>| match ptr {
            Some(ptr) => {
                let _ = props.set(name, ptr as i64);
            }
            None => {
                let _ = props.remove(name);
            }
        };
        set(
            PROP_RENDERER_D3D12_DEVICE_POINTER,
            self.d3d_device.as_ref().map(raw),
        );
        set(
            PROP_RENDERER_D3D12_COMMAND_QUEUE_POINTER,
            self.command_queue.as_ref().map(raw),
        );
        // (upstream leaves the swap chain's set when it releases it)
        if let Some(swap_chain) = &self.swap_chain {
            set(PROP_RENDERER_D3D12_SWAPCHAIN_POINTER, Some(raw(swap_chain)));
        }
    }

    /// Translation of `D3D12_ReleaseAll()`.
    fn release_all(&mut self) {
        // (the device and command queue properties are cleared once they
        // are released, below)

        // Release all textures
        let ids: Vec<u32> = self.textures.keys().copied().collect();
        for id in ids {
            self.destroy_texture_data(id);
        }

        // Release/reset everything else
        self.dxgi_factory = None;
        self.dxgi_adapter = None;
        self.swap_chain = None;
        self.d3d_device = None;
        self.debug_interface = None;
        self.command_queue = None;
        self.command_list = None;
        self.rtv_descriptor_heap = None;
        self.texture_rtv_descriptor_heap = None;
        self.srv_descriptor_heap = None;
        self.sampler_descriptor_heap = None;
        self.samplers_created = [false; RENDER_SAMPLER_COUNT];
        self.fence = None;
        // FIXME (upstream): the fence event is never closed; the next
        // D3D12_CreateDeviceResources() makes another.

        self.command_allocators = Default::default();
        self.render_targets = Default::default();

        self.pipeline_states.clear();
        self.current_pipeline_state = None;

        self.root_signatures = Default::default();

        for vertex_buffer in &mut self.vertex_buffers {
            vertex_buffer.resource = None;
            vertex_buffer.size = 0;
        }
        // (upstream releases them in D3D12_ResetCommandList(), which the
        // next device's list starts with; the old device's go with it here)
        self.upload_buffers = Default::default();
        self.current_upload_buffer = 0;

        self.swap_effect = 0;
        self.swap_flags = 0;
        self.current_render_target_view.ptr = 0;
        self.num_current_shader_resources = 0;
        self.current_shader_resource.ptr = 0;
        self.num_current_shader_samplers = 0;
        self.current_shader_sampler.ptr = 0;

        self.publish_properties();

        // Check for any leaks if in debug mode
        if let Some(dxgi_debug) = self.dxgi_debug.take() {
            let rlo_flags = DXGI_DEBUG_RLO_DETAIL | DXGI_DEBUG_RLO_IGNORE_INTERNAL;
            dxgi_debug.report_live_objects(DXGI_DEBUG_ALL, rlo_flags);
        }

        /* Unload the D3D libraries.  This should be done last, in order
         * to prevent IUnknown::Release() calls from crashing.
         */
        self.h_d3d12_mod = None;
        self.h_dxgi_mod = None;
    }

    /// Translation of `D3D12_WaitForGPU()`.
    fn wait_for_gpu(&mut self) {
        if let (Some(command_queue), Some(fence)) = (&self.command_queue, &self.fence) {
            if self.fence_event.is_null() {
                return;
            }
            command_queue.signal(fence, self.fence_value);
            if fence.completed_value() < self.fence_value {
                fence.set_event_on_completion(self.fence_value, self.fence_event);
                // SAFETY: the renderer's event handle.
                unsafe { WaitForSingleObjectEx(self.fence_event, INFINITE, 0) };
            }

            self.fence_value += 1;
        }
    }

    /// Translation of `D3D12_GetCurrentRenderTargetView()`.
    fn get_current_render_target_view(&self) -> CpuDescriptorHandle {
        if let Some(id) = self.texture_render_target {
            return self
                .textures
                .get(&id)
                .map_or_else(CpuDescriptorHandle::default, |t| {
                    t.main_texture_render_target_view
                });
        }

        let Some(heap) = &self.rtv_descriptor_heap else {
            return CpuDescriptorHandle::default();
        };
        descriptor_at(
            heap,
            self.current_back_buffer_index,
            self.rtv_descriptor_size,
        )
    }

    /// Translation of `D3D12_TransitionResource()`.
    fn transition_resource(&self, resource: &D3d12Resource, before_state: u32, after_state: u32) {
        if before_state != after_state {
            if let Some(command_list) = &self.command_list {
                command_list.resource_barrier(&[transition_barrier(
                    resource,
                    before_state,
                    after_state,
                )]);
            }
        }
    }

    /// Translation of `D3D12_ResetCommandList()`.
    fn reset_command_list(&mut self) {
        let (Some(command_list), Some(command_allocator)) = (
            &self.command_list,
            &self.command_allocators[self.current_back_buffer_index],
        ) else {
            return;
        };

        command_allocator.reset();
        command_list.reset(command_allocator);
        self.current_pipeline_state = None;
        self.current_vertex_buffer = 0;
        self.issue_batch = false;
        self.cliprect_dirty = true;
        self.viewport_dirty = true;
        self.current_render_target_view.ptr = 0;
        // FIXME should we also clear currentSampler.ptr and currentRenderTargetView.ptr ? (and use D3D12_InvalidateCachedState() instead)

        // Release any upload buffers that were inflight
        for upload_buffer in &mut self.upload_buffers[..self.current_upload_buffer] {
            *upload_buffer = None;
        }
        self.current_upload_buffer = 0;

        if let (Some(srv), Some(sampler)) =
            (&self.srv_descriptor_heap, &self.sampler_descriptor_heap)
        {
            command_list.set_descriptor_heaps(&[srv, sampler]);
        }
    }

    /// Translation of `D3D12_IssueBatch()`.
    fn issue_batch(&mut self) -> Result<()> {
        let (Some(command_list), Some(command_queue)) = (&self.command_list, &self.command_queue)
        else {
            return Err(device_lost());
        };

        // Issue the command list
        let result = command_list.close();
        if result < 0 {
            return Err(hr_error("D3D12_IssueBatch", result));
        }
        command_queue.execute_graphics_command_list(command_list);

        self.wait_for_gpu();

        self.reset_command_list();

        Ok(())
    }

    /// Translation of `D3D12_CreatePipelineState()`: the index of the new
    /// pipeline state in `pipeline_states`.
    fn create_pipeline_state(
        &mut self,
        shader: Shader,
        blend_mode: BlendMode,
        topology: u32,
        rtv_format: DxgiFormat,
    ) -> Result<usize> {
        let element = |name: &'static std::ffi::CStr, format, offset| InputElementDesc {
            semantic_name: name.as_ptr(),
            semantic_index: 0,
            format,
            input_slot: 0,
            aligned_byte_offset: offset,
            input_slot_class: D3D12_INPUT_CLASSIFICATION_PER_VERTEX_DATA,
            instance_data_step_rate: 0,
        };
        let vertex_desc = [
            element(c"POSITION", DXGI_FORMAT_R32G32_FLOAT, 0),
            element(c"TEXCOORD", DXGI_FORMAT_R32G32_FLOAT, 8),
            element(c"COLOR", DXGI_FORMAT_R32G32B32A32_FLOAT, 16),
        ];
        let root_signature = self.root_signatures[shader.root_signature_type() as usize]
            .as_ref()
            .ok_or_else(device_lost)?;

        let mut pipeline_desc = GraphicsPipelineStateDesc {
            root_signature: raw(root_signature),
            vs: ShaderBytecode::new(shader.vertex_shader()),
            ps: ShaderBytecode::new(shader.pixel_shader()),
            blend_state: create_blend_state(blend_mode),
            sample_mask: 0xffffffff,
            ..Default::default()
        };

        pipeline_desc.rasterizer_state.antialiased_line_enable = 0;
        pipeline_desc.rasterizer_state.cull_mode = D3D12_CULL_MODE_NONE;
        pipeline_desc.rasterizer_state.depth_bias = 0;
        pipeline_desc.rasterizer_state.depth_bias_clamp = 0.0;
        pipeline_desc.rasterizer_state.depth_clip_enable = 1;
        pipeline_desc.rasterizer_state.fill_mode = D3D12_FILL_MODE_SOLID;
        pipeline_desc.rasterizer_state.front_counter_clockwise = 0;
        pipeline_desc.rasterizer_state.multisample_enable = 0;
        pipeline_desc.rasterizer_state.slope_scaled_depth_bias = 0.0;

        pipeline_desc.input_layout.input_element_descs = vertex_desc.as_ptr();
        pipeline_desc.input_layout.num_elements = 3;

        pipeline_desc.primitive_topology_type = topology;

        pipeline_desc.num_render_targets = 1;
        pipeline_desc.rtv_formats[0] = rtv_format;
        pipeline_desc.sample_desc.count = 1;
        pipeline_desc.sample_desc.quality = 0;

        // SAFETY: the description's pointers (the root signature, the
        // static bytecode and the input layout above) are live for the call.
        let pipeline_state = unsafe {
            self.device()?
                .create_graphics_pipeline_state(&pipeline_desc)
        }
        .map_err(|hr| hr_error("ID3D12Device::CreateGraphicsPipelineState", hr))?;

        self.pipeline_states.push(PipelineState {
            shader,
            shader_constants: PixelShaderConstants::default(),
            blend_mode,
            topology,
            rtv_format,
            pipeline_state,
        });

        Ok(self.pipeline_states.len() - 1)
    }

    /// Translation of `D3D12_CreateVertexBuffer()`.
    fn create_vertex_buffer(&mut self, vbidx: usize, size: usize) -> Result<()> {
        self.vertex_buffers[vbidx].resource = None;

        let vbuffer_heap_props = heap_properties(D3D12_HEAP_TYPE_UPLOAD);
        let vbuffer_desc = buffer_desc(size as u64);

        let resource = self
            .device()?
            .create_committed_resource(
                &vbuffer_heap_props,
                D3D12_HEAP_FLAG_NONE,
                &vbuffer_desc,
                D3D12_RESOURCE_STATE_GENERIC_READ,
                None,
            )
            .map_err(|hr| hr_error("ID3D12Device::CreatePlacedResource [vertex buffer]", hr))?;

        let vertex_buffer = &mut self.vertex_buffers[vbidx];
        vertex_buffer.view.buffer_location = resource.gpu_virtual_address();
        vertex_buffer.view.stride_in_bytes = size_of::<VertexPositionColor>() as u32;
        vertex_buffer.size = size;
        vertex_buffer.resource = Some(resource);

        Ok(())
    }

    /// Create resources that depend on the device. Translation of
    /// `D3D12_CreateDeviceResources()` (the failure of a failed `HRESULT`).
    fn create_device_resources(&mut self) -> Result<()> {
        type PfnCreateEventExW =
            unsafe extern "system" fn(*const c_void, *const u16, u32, u32) -> HANDLE;

        let mut creation_flags = 0;

        // See if we need debug interfaces
        let create_debug = hints::get_bool(hints::RENDER_DIRECT3D11_DEBUG, false);

        // CreateEventExW() arrived in Vista, so we need to load it with GetProcAddress for XP.
        let p_create_event_ex_w: Option<PfnCreateEventExW> = {
            let name: Vec<u16> = "kernel32.dll\0".encode_utf16().collect();
            // SAFETY: a NUL-terminated module name.
            let kernel32 = unsafe { GetModuleHandleW(name.as_ptr()) };
            if kernel32.is_null() {
                None
            } else {
                // SAFETY: a loaded module and a NUL-terminated name.
                let f = unsafe { GetProcAddress(kernel32, c"CreateEventExW".as_ptr().cast()) };
                // SAFETY: kernel32's CreateEventExW has this signature.
                f.map(|f| unsafe {
                    std::mem::transmute::<unsafe extern "system" fn() -> isize, PfnCreateEventExW>(
                        f,
                    )
                })
            }
        };
        let Some(p_create_event_ex_w) = p_create_event_ex_w else {
            return Err(Error::new("CreateEventExW not found"));
        };

        let dxgi_mod = self.h_dxgi_mod.insert(SharedObject::load("dxgi.dll")?);

        // SAFETY: the signature of dxgi.dll's export.
        let p_create_dxgi_factory2 =
            unsafe { dxgi_mod.function::<PfnCreateDxgiFactory2>("CreateDXGIFactory2") }?;
        // SAFETY: as above (upstream calls it through the same type).
        let p_dxgi_get_debug_interface1 = if create_debug {
            Some(unsafe { dxgi_mod.function::<PfnCreateDxgiFactory2>("DXGIGetDebugInterface1") }?)
        } else {
            None
        };

        let d3d12_mod = self.h_d3d12_mod.insert(SharedObject::load("D3D12.dll")?);

        // SAFETY: the signature of d3d12.dll's export.
        let p_d3d12_create_device =
            unsafe { d3d12_mod.function::<PfnD3d12CreateDevice>("D3D12CreateDevice") }?;

        if create_debug {
            // SAFETY: the signature of d3d12.dll's export.
            let d3d12_get_debug_interface_func = unsafe {
                d3d12_mod.function::<PfnD3d12GetDebugInterface>("D3D12GetDebugInterface")
            }?;
            // SAFETY: the call stores an owned ID3D12Debug, the interface
            // asked for.
            if let Ok(debug_interface) = unsafe {
                out::<ID3D12DebugVtbl>(|o| d3d12_get_debug_interface_func(&IID_ID3D12DEBUG, o))
            } {
                debug_interface.enable_debug_layer();
                self.debug_interface = Some(debug_interface);
            }
        }

        if let Some(dxgi_get_debug_interface_func) = p_dxgi_get_debug_interface1 {
            // If the debug hint is set, also create the DXGI factory in debug mode
            // SAFETY: DXGIGetDebugInterface1 stores an owned interface of the
            // IID asked for.
            let dxgi_debug = unsafe {
                out::<IDXGIDebugVtbl>(|o| dxgi_get_debug_interface_func(0, &IID_IDXGIDEBUG1, o))
            }
            .map_err(|hr| hr_error("DXGIGetDebugInterface1", hr))?;
            self.dxgi_debug = Some(dxgi_debug);

            // SAFETY: as above.
            let dxgi_info_queue: DxgiInfoQueue = unsafe {
                out::<IDXGIInfoQueueVtbl>(|o| {
                    dxgi_get_debug_interface_func(0, &IID_IDXGIINFOQUEUE, o)
                })
            }
            .map_err(|hr| hr_error("DXGIGetDebugInterface1", hr))?;

            dxgi_info_queue.set_break_on_severity(
                DXGI_DEBUG_ALL,
                DXGI_INFO_QUEUE_MESSAGE_SEVERITY_ERROR,
                true,
            );
            dxgi_info_queue.set_break_on_severity(
                DXGI_DEBUG_ALL,
                DXGI_INFO_QUEUE_MESSAGE_SEVERITY_CORRUPTION,
                true,
            );
            drop(dxgi_info_queue);
            creation_flags = DXGI_CREATE_FACTORY_DEBUG;
        }

        // SAFETY: CreateDXGIFactory2 stores an owned IDXGIFactory6, the
        // interface asked for.
        let factory = unsafe {
            out::<IDXGIFactory6Vtbl>(|o| {
                p_create_dxgi_factory2(creation_flags, &IID_IDXGIFACTORY6, o)
            })
        }
        .map_err(|hr| hr_error("CreateDXGIFactory", hr))?;
        let factory = self.dxgi_factory.insert(factory);

        // Prefer a high performance adapter if there are multiple choices
        let adapter = factory
            .enum_adapter4_by_gpu_preference(0, DXGI_GPU_PREFERENCE_HIGH_PERFORMANCE)
            .map_err(|hr| hr_error("IDXGIFactory6::EnumAdapterByGpuPreference", hr))?;
        let adapter = self.dxgi_adapter.insert(adapter);

        // SAFETY: a live adapter; on success the call stores an owned
        // ID3D12Device1, which is an ID3D12Device.
        let d3d_device = unsafe {
            out::<ID3D12DeviceVtbl>(|o| {
                p_d3d12_create_device(
                    raw(adapter),
                    D3D_FEATURE_LEVEL_11_0, // Request minimum feature level 11.0 for maximum compatibility
                    &IID_ID3D12DEVICE1,
                    o,
                )
            })
        }
        .map_err(|hr| hr_error("D3D12CreateDevice", hr))?;

        // Setup the info queue if in debug mode
        if create_debug {
            let severities = [D3D12_MESSAGE_SEVERITY_INFO];

            let info_queue: D3d12InfoQueue = d3d_device
                .query(&IID_ID3D12INFOQUEUE)
                .map_err(|hr| hr_error("ID3D12Device to ID3D12InfoQueue", hr))?;

            let mut filter = InfoQueueFilter::default();
            filter.deny_list.num_severities = 1;
            filter.deny_list.severity_list = severities.as_ptr();
            info_queue.push_storage_filter(&filter);

            info_queue.set_break_on_severity(D3D12_MESSAGE_SEVERITY_ERROR, true);
            info_queue.set_break_on_severity(D3D12_MESSAGE_SEVERITY_CORRUPTION, true);
        }

        let device: D3d12Device = d3d_device
            .query(&IID_ID3D12DEVICE1)
            .map_err(|hr| hr_error("ID3D12Device to ID3D12Device1", hr))?;
        let device = self.d3d_device.insert(device).clone();

        // Create a command queue
        let queue_desc = CommandQueueDesc {
            flags: D3D12_COMMAND_QUEUE_FLAG_NONE,
            ty: D3D12_COMMAND_LIST_TYPE_DIRECT,
            ..Default::default()
        };

        let command_queue = device
            .create_command_queue(&queue_desc)
            .map_err(|hr| hr_error("ID3D12Device::CreateCommandQueue", hr))?;
        self.command_queue = Some(command_queue);

        // Create the descriptor heaps for the render target view, texture SRVs, and samplers
        let mut descriptor_heap_desc = DescriptorHeapDesc {
            num_descriptors: SDL_D3D12_NUM_BUFFERS as u32,
            ty: D3D12_DESCRIPTOR_HEAP_TYPE_RTV,
            ..Default::default()
        };
        self.rtv_descriptor_heap = Some(
            device
                .create_descriptor_heap(&descriptor_heap_desc)
                .map_err(|hr| hr_error("ID3D12Device::CreateDescriptorHeap [rtv]", hr))?,
        );
        self.rtv_descriptor_size =
            d3d_device.descriptor_handle_increment_size(D3D12_DESCRIPTOR_HEAP_TYPE_RTV);

        descriptor_heap_desc.num_descriptors = SDL_D3D12_MAX_NUM_TEXTURES as u32;
        self.texture_rtv_descriptor_heap = Some(
            device
                .create_descriptor_heap(&descriptor_heap_desc)
                .map_err(|hr| hr_error("ID3D12Device::CreateDescriptorHeap [texture rtv]", hr))?,
        );

        let descriptor_heap_desc = DescriptorHeapDesc {
            num_descriptors: SDL_D3D12_MAX_NUM_TEXTURES as u32,
            ty: D3D12_DESCRIPTOR_HEAP_TYPE_CBV_SRV_UAV,
            flags: D3D12_DESCRIPTOR_HEAP_FLAG_SHADER_VISIBLE,
            ..Default::default()
        };
        let srv_descriptor_heap = device
            .create_descriptor_heap(&descriptor_heap_desc)
            .map_err(|hr| hr_error("ID3D12Device::CreateDescriptorHeap  [srv]", hr))?;
        self.srv_descriptor_size =
            d3d_device.descriptor_handle_increment_size(D3D12_DESCRIPTOR_HEAP_TYPE_CBV_SRV_UAV);

        let descriptor_heap_desc = DescriptorHeapDesc {
            num_descriptors: self.samplers.len() as u32,
            ty: D3D12_DESCRIPTOR_HEAP_TYPE_SAMPLER,
            flags: D3D12_DESCRIPTOR_HEAP_FLAG_SHADER_VISIBLE,
            ..Default::default()
        };
        let sampler_descriptor_heap = device
            .create_descriptor_heap(&descriptor_heap_desc)
            .map_err(|hr| hr_error("ID3D12Device::CreateDescriptorHeap  [sampler]", hr))?;
        self.sampler_descriptor_size =
            d3d_device.descriptor_handle_increment_size(D3D12_DESCRIPTOR_HEAP_TYPE_SAMPLER);
        let srv_descriptor_heap = self.srv_descriptor_heap.insert(srv_descriptor_heap).clone();
        let sampler_descriptor_heap = self
            .sampler_descriptor_heap
            .insert(sampler_descriptor_heap)
            .clone();

        // Create a command allocator for each back buffer
        for command_allocator in &mut self.command_allocators {
            *command_allocator = Some(
                device
                    .create_command_allocator(D3D12_COMMAND_LIST_TYPE_DIRECT)
                    .map_err(|hr| hr_error("ID3D12Device::CreateCommandAllocator", hr))?,
            );
        }

        // Create the command list
        let Some(command_allocator) = &self.command_allocators[0] else {
            return Err(device_lost());
        };
        let command_list = device
            .create_command_list2(0, D3D12_COMMAND_LIST_TYPE_DIRECT, command_allocator)
            .map_err(|hr| hr_error("ID3D12Device::CreateCommandList", hr))?;

        // Set the descriptor heaps to the correct initial value
        command_list.set_descriptor_heaps(&[&srv_descriptor_heap, &sampler_descriptor_heap]);
        self.command_list = Some(command_list);

        // Create the fence and fence event
        self.fence = Some(
            device
                .create_fence(self.fence_value, D3D12_FENCE_FLAG_NONE)
                .map_err(|hr| hr_error("ID3D12Device::CreateFence", hr))?,
        );

        self.fence_value += 1;

        // SAFETY: CreateEventExW without attributes or a name.
        self.fence_event =
            unsafe { p_create_event_ex_w(null(), null(), 0, EVENT_MODIFY_STATE | SYNCHRONIZE) };
        if self.fence_event.is_null() {
            // FIXME (upstream): this `goto done` returns the S_OK of
            // CreateFence() (with the error set from it), so the renderer
            // is made without its root signatures, pipeline states, vertex
            // buffers and samplers, and never waits for the GPU.
            // (WIN_SetErrorFromHRESULT("CreateEventEx", result))
            return Ok(());
        }

        // Create all the root signatures
        for root_sig in RootSignature::ALL {
            self.root_signatures[root_sig as usize] = Some(
                device
                    .create_root_signature(root_sig.data())
                    .map_err(|hr| hr_error("ID3D12Device::CreateRootSignature", hr))?,
            );
        }

        {
            let default_blend_modes = [BlendMode::BLEND];
            let default_rtv_formats = [DXGI_FORMAT_B8G8R8A8_UNORM];

            // Create a few default pipeline state objects, to verify that this renderer will work
            for shader in Shader::ALL {
                for &blend_mode in &default_blend_modes {
                    for topology in
                        D3D12_PRIMITIVE_TOPOLOGY_TYPE_POINT..D3D12_PRIMITIVE_TOPOLOGY_TYPE_PATCH
                    {
                        for &rtv_format in &default_rtv_formats {
                            // D3D12_CreatePipelineState will set the SDL error, if it fails
                            self.create_pipeline_state(shader, blend_mode, topology, rtv_format)?;
                        }
                    }
                }
            }
        }

        // Create default vertex buffers
        for i in 0..SDL_D3D12_NUM_VERTEX_BUFFERS {
            let _ =
                self.create_vertex_buffer(i, D3D12_DEFAULT_RESOURCE_PLACEMENT_ALIGNMENT as usize);
        }

        // Create samplers to use when drawing textures:
        let sampler_size = self.sampler_descriptor_size;
        for (i, sampler) in self.samplers.iter_mut().enumerate() {
            *sampler = descriptor_at(&sampler_descriptor_heap, i, sampler_size);
        }

        // Initialize the pool allocator for SRVs
        self.init_srv_pool();

        self.publish_properties();

        Ok(())
    }

    /// Translation of `D3D12_GetRotationForCurrentRenderTarget()`.
    fn get_rotation_for_current_render_target(&self) -> DxgiModeRotation {
        if self.texture_render_target.is_some() {
            DXGI_MODE_ROTATION_IDENTITY
        } else {
            self.rotation
        }
    }

    /// Translation of `D3D12_GetViewportAlignedD3DRect()`.
    fn get_viewport_aligned_d3d_rect(
        &self,
        sdl_rect: &Rect,
        include_viewport_offset: bool,
    ) -> Result<D3d12Rect> {
        let rotation = self.get_rotation_for_current_render_target();
        let viewport = &self.current_viewport;

        let mut out_rect = D3d12Rect::default();
        match rotation {
            DXGI_MODE_ROTATION_IDENTITY => {
                out_rect.left = sdl_rect.x;
                out_rect.right = sdl_rect.x + sdl_rect.w;
                out_rect.top = sdl_rect.y;
                out_rect.bottom = sdl_rect.y + sdl_rect.h;
                if include_viewport_offset {
                    out_rect.left += viewport.x;
                    out_rect.right += viewport.x;
                    out_rect.top += viewport.y;
                    out_rect.bottom += viewport.y;
                }
            }
            DXGI_MODE_ROTATION_ROTATE270 => {
                out_rect.left = sdl_rect.y;
                out_rect.right = sdl_rect.y + sdl_rect.h;
                out_rect.top = viewport.w - sdl_rect.x - sdl_rect.w;
                out_rect.bottom = viewport.w - sdl_rect.x;
            }
            DXGI_MODE_ROTATION_ROTATE180 => {
                out_rect.left = viewport.w - sdl_rect.x - sdl_rect.w;
                out_rect.right = viewport.w - sdl_rect.x;
                out_rect.top = viewport.h - sdl_rect.y - sdl_rect.h;
                out_rect.bottom = viewport.h - sdl_rect.y;
            }
            DXGI_MODE_ROTATION_ROTATE90 => {
                out_rect.left = viewport.h - sdl_rect.y - sdl_rect.h;
                out_rect.right = viewport.h - sdl_rect.y;
                out_rect.top = sdl_rect.x;
                out_rect.bottom = sdl_rect.x + sdl_rect.h;
            }
            _ => {
                return Err(Error::new(
                    "The physical display is in an unknown or unsupported rotation",
                ))
            }
        }
        Ok(out_rect)
    }

    /// Translation of `D3D12_CreateSwapChain()`.
    fn create_swap_chain(&mut self, w: i32, h: i32) -> Result<()> {
        let factory = self.dxgi_factory.clone().ok_or_else(device_lost)?;
        let command_queue = self.command_queue.clone().ok_or_else(device_lost)?;

        // Create a swap chain using the same adapter as the existing Direct3D device.
        let mut swap_chain_desc = SwapChainDesc1 {
            width: w as u32,
            height: h as u32,
            ..Default::default()
        };
        match self.output_colorspace {
            Colorspace::SRGB_LINEAR => {
                swap_chain_desc.format = DXGI_FORMAT_R16G16B16A16_FLOAT;
                self.render_target_format = DXGI_FORMAT_R16G16B16A16_FLOAT;
            }
            Colorspace::HDR10 => {
                swap_chain_desc.format = DXGI_FORMAT_R10G10B10A2_UNORM;
                self.render_target_format = DXGI_FORMAT_R10G10B10A2_UNORM;
            }
            _ => {
                swap_chain_desc.format = DXGI_FORMAT_B8G8R8A8_UNORM; // This is the most common swap chain format.
                self.render_target_format = DXGI_FORMAT_B8G8R8A8_UNORM;
            }
        }
        swap_chain_desc.stereo = 0;
        swap_chain_desc.sample_desc.count = 1; // Don't use multi-sampling.
        swap_chain_desc.sample_desc.quality = 0;
        swap_chain_desc.buffer_usage = DXGI_USAGE_RENDER_TARGET_OUTPUT;
        swap_chain_desc.buffer_count = 2; // Use double-buffering to minimize latency.
        if is_windows_8_or_greater() {
            swap_chain_desc.scaling = DXGI_SCALING_NONE;
        } else {
            swap_chain_desc.scaling = DXGI_SCALING_STRETCH;
        }
        swap_chain_desc.swap_effect = DXGI_SWAP_EFFECT_FLIP_SEQUENTIAL; // All Windows Store apps must use this SwapEffect.
        swap_chain_desc.flags = DXGI_SWAP_CHAIN_FLAG_FRAME_LATENCY_WAITABLE_OBJECT | // To support SetMaximumFrameLatency
                                DXGI_SWAP_CHAIN_FLAG_ALLOW_TEARING; // To support presenting with allow tearing on

        let Some(hwnd) = self.hwnd() else {
            return Err(Error::new("Couldn't get window handle"));
        };

        let swap_chain = factory
            .create_swap_chain_for_hwnd(&command_queue, hwnd, &swap_chain_desc)
            .map_err(|hr| hr_error("IDXGIFactory2::CreateSwapChainForHwnd", hr))?;

        factory.make_window_association(hwnd, DXGI_MWA_NO_WINDOW_CHANGES);

        let swap_chain: DxgiSwapChain3 = swap_chain
            .query(&IID_IDXGISWAPCHAIN4)
            .map_err(|hr| hr_error("IDXGISwapChain1::QueryInterface", hr))?;
        let swap_chain = self.swap_chain.insert(swap_chain).clone();

        /* Ensure that the swapchain does not queue more than one frame at a time. This both reduces latency
         * and ensures that the application will only render after each VSync, minimizing power consumption.
         */
        check(swap_chain.set_maximum_frame_latency(1))
            .map_err(|hr| hr_error("IDXGISwapChain4::SetMaximumFrameLatency", hr))?;

        self.swap_effect = swap_chain_desc.swap_effect;
        self.swap_flags = swap_chain_desc.flags;

        let mut result = Ok(());
        let colorspace = match self.output_colorspace {
            Colorspace::SRGB_LINEAR => DXGI_COLOR_SPACE_RGB_FULL_G10_NONE_P709,
            Colorspace::HDR10 => DXGI_COLOR_SPACE_RGB_FULL_G2084_NONE_P2020,
            // sRGB
            _ => DXGI_COLOR_SPACE_RGB_FULL_G22_NONE_P709,
        };
        let colorspace_support = swap_chain.check_color_space_support(colorspace);
        if colorspace_support & DXGI_SWAP_CHAIN_COLOR_SPACE_SUPPORT_FLAG_PRESENT != 0 {
            check(swap_chain.set_color_space1(colorspace))
                .map_err(|hr| hr_error("IDXGISwapChain3::SetColorSpace1", hr))?;
        } else {
            // Not the default, we're not going to be able to present in this colorspace
            // (failing with DXGI_ERROR_UNSUPPORTED)
            result = Err(Error::new("Unsupported output colorspace"));
        }

        self.publish_properties();

        result
    }

    /// Initialize all resources that change when the window's size changes.
    /// Translation of `D3D12_CreateWindowSizeDependentResources()`.
    fn create_window_size_dependent_resources(&mut self) -> Result<()> {
        // Release resources in the current command list
        let _ = self.issue_batch();
        if let Some(command_list) = &self.command_list {
            command_list.om_set_render_targets(&[], None);
        }

        // Release render targets
        self.render_targets = Default::default();

        /* The width and height of the swap chain must be based on the display's
         * non-rotated size.
         */
        let (mut w, mut h) = self.window.size_in_pixels().unwrap_or((0, 0));
        self.rotation = get_current_rotation();
        if is_display_rotated_90_degrees(self.rotation) {
            std::mem::swap(&mut w, &mut h);
        }

        if let Some(swap_chain) = &self.swap_chain {
            // If the swap chain already exists, resize it.
            check(swap_chain.resize_buffers(
                0,
                w as u32,
                h as u32,
                DXGI_FORMAT_UNKNOWN,
                self.swap_flags,
            ))
            .map_err(|hr| hr_error("IDXGISwapChain::ResizeBuffers", hr))?;
        } else {
            self.create_swap_chain(w, h)?;
        }
        let swap_chain = self.swap_chain.clone().ok_or_else(device_lost)?;

        // Set the proper rotation for the swap chain.
        if is_windows_8_or_greater() && self.swap_effect == DXGI_SWAP_EFFECT_FLIP_SEQUENTIAL {
            check(swap_chain.set_rotation(self.rotation))
                .map_err(|hr| hr_error("IDXGISwapChain4::SetRotation", hr))?;
        }

        // Get each back buffer render target and create render target views
        let device = self.device()?.clone();
        let rtv_heap = self.rtv_descriptor_heap.clone().ok_or_else(device_lost)?;
        for i in 0..SDL_D3D12_NUM_BUFFERS {
            let render_target = swap_chain
                .buffer(i as u32)
                .map_err(|hr| hr_error("IDXGISwapChain4::GetBuffer", hr))?;

            let rtv_desc = texture_2d_rtv_desc(self.render_target_format);

            let rtv_descriptor = descriptor_at(&rtv_heap, i, self.rtv_descriptor_size);
            device.create_render_target_view(&render_target, &rtv_desc, rtv_descriptor);
            self.render_targets[i] = Some(render_target);
        }

        // Set back buffer index to current buffer
        self.current_back_buffer_index = swap_chain.current_back_buffer_index() as usize;

        /* Set the swap chain target immediately, so that a target is always set
         * even before we get to SetDrawState. Without this it's possible to hit
         * null references in places like ReadPixels!
         */
        self.current_render_target_view = self.get_current_render_target_view();
        if let Some(command_list) = &self.command_list {
            command_list.om_set_render_targets(&[self.current_render_target_view], None);
        }
        if let Some(render_target) = &self.render_targets[self.current_back_buffer_index] {
            self.transition_resource(
                render_target,
                D3D12_RESOURCE_STATE_PRESENT,
                D3D12_RESOURCE_STATE_RENDER_TARGET,
            );
        }

        self.viewport_dirty = true;

        Ok(())
    }

    /// Translation of `D3D12_HandleDeviceLost()`.
    fn handle_device_lost(&mut self) -> bool {
        let mut recovered = false;

        self.release_all();

        match self
            .create_device_resources()
            .and_then(|()| self.create_window_size_dependent_resources())
        {
            Ok(()) => recovered = true,
            Err(e) => {
                crate::log::error!(
                    Category::Render,
                    "Renderer couldn't recover from device lost: {}",
                    e
                );
                self.release_all();
            }
        }

        // Let the application know that the device has been reset or lost
        let _ = crate::events::push(Event::Render(RenderEvent {
            event_type: if recovered {
                EventType::RENDER_DEVICE_RESET
            } else {
                EventType::RENDER_DEVICE_LOST
            },
            window_id: self.window.id(),
            ..Default::default()
        }));

        recovered
    }

    /// This method is called when the window's size changes. Translation of
    /// `D3D12_UpdateForWindowSizeChange()`.
    fn update_for_window_size_change(&mut self) -> Result<()> {
        // If the GPU has previous work, wait for it to be done first
        self.wait_for_gpu();
        self.create_window_size_dependent_resources()
    }

    /// Initialize the pool allocator for SRVs (in
    /// `D3D12_CreateDeviceResources()`).
    fn init_srv_pool(&mut self) {
        for (i, node) in self.srv_pool_nodes.iter_mut().enumerate() {
            node.index = i;
            if i != SDL_D3D12_MAX_NUM_TEXTURES - 1 {
                node.next = Some(i + 1);
            }
        }
        // FIXME (upstream): the last node's next isn't reset, so after a
        // device loss it still points where it did when it was freed.
        self.srv_pool_head = Some(0);
    }

    /// Translation of `D3D12_GetAvailableSRVIndex()`.
    fn get_available_srv_index(&mut self) -> Result<usize> {
        if let Some(head) = self.srv_pool_head {
            let index = self.srv_pool_nodes[head].index;
            self.srv_pool_head = self.srv_pool_nodes[head].next;
            Ok(index)
        } else {
            // FIXME (upstream): this returns SDL_D3D12_MAX_NUM_TEXTURES + 1,
            // which the callers make a view at, past the end of the heap
            // (and later free into the pool, past the end of its nodes).
            Err(Error::new(format!(
                "[d3d12] Cannot allocate more than {} textures!",
                SDL_D3D12_MAX_NUM_TEXTURES
            )))
        }
    }

    /// Translation of `D3D12_FreeSRVIndex()`.
    fn free_srv_index(&mut self, index: usize) {
        self.srv_pool_nodes[index].next = self.srv_pool_head;
        self.srv_pool_head = Some(index);
    }

    /// A shader resource view of `resource` in a free slot of the SRV heap:
    /// the slot's index and descriptor.
    fn create_srv(
        &mut self,
        resource: &D3d12Resource,
        desc: &ShaderResourceViewDesc,
    ) -> Result<(usize, CpuDescriptorHandle)> {
        let heap = self.srv_descriptor_heap.clone().ok_or_else(device_lost)?;
        let index = self.get_available_srv_index()?;
        let view = descriptor_at(&heap, index, self.srv_descriptor_size);

        self.device()?
            .create_shader_resource_view(resource, desc, view);
        Ok((index, view))
    }

    /// The renderer's part of `D3D12_CreateTexture()` (from the main texture
    /// on), on `texture_data`, which the caller frees if it fails.
    fn create_texture_resources(
        &mut self,
        texture: &mut TextureData,
        texture_data: &mut D3d12Texture,
        mut texture_desc: ResourceDesc,
    ) -> Result<()> {
        use PixelFormat as F;
        let device = self.device()?.clone();

        let heap_props = heap_properties(D3D12_HEAP_TYPE_DEFAULT);
        let create = |desc: &ResourceDesc| {
            device
                .create_committed_resource(
                    &heap_props,
                    D3D12_HEAP_FLAG_NONE,
                    desc,
                    D3D12_RESOURCE_STATE_COPY_DEST,
                    None,
                )
                .map_err(|hr| hr_error("ID3D12Device::CreateCommittedResource [texture]", hr))
        };

        // (SDL_PROP_TEXTURE_CREATE_D3D12_TEXTURE_POINTER isn't taken)
        let main_texture = create(&texture_desc)?;
        texture_data.main_resource_state = D3D12_RESOURCE_STATE_COPY_DEST;
        let props = texture.props.get_or_insert_with(Properties::new).clone();
        let _ = props.set(
            PROP_TEXTURE_D3D12_TEXTURE_POINTER,
            raw(&main_texture) as i64,
        );
        texture_data.main_texture = Some(main_texture);

        if matches!(texture.format, F::YV12 | F::IYUV | F::I0FL) {
            texture_data.yuv = true;

            texture_desc.width = texture_desc.width.div_ceil(2);
            texture_desc.height = texture_desc.height.div_ceil(2);
        }
        if matches!(
            texture.format,
            F::YV12 | F::IYUV | F::I0FL | F::I444 | F::I4FL
        ) {
            texture_data.yuv = true;

            // (SDL_PROP_TEXTURE_CREATE_D3D12_TEXTURE_U_POINTER isn't taken)
            let main_texture_u = create(&texture_desc)?;
            texture_data.main_resource_state_u = D3D12_RESOURCE_STATE_COPY_DEST;
            let _ = props.set(
                PROP_TEXTURE_D3D12_TEXTURE_U_POINTER,
                raw(&main_texture_u) as i64,
            );
            texture_data.main_texture_u = Some(main_texture_u);

            // (SDL_PROP_TEXTURE_CREATE_D3D12_TEXTURE_V_POINTER isn't taken)
            let main_texture_v = create(&texture_desc)?;
            texture_data.main_resource_state_v = D3D12_RESOURCE_STATE_COPY_DEST;
            let _ = props.set(
                PROP_TEXTURE_D3D12_TEXTURE_V_POINTER,
                raw(&main_texture_v) as i64,
            );
            texture_data.main_texture_v = Some(main_texture_v);

            let bits_per_pixel = if matches!(texture.format, F::I0FL | F::I4FL) {
                16
            } else {
                8
            };
            texture_data.ycbcr_matrix = ycbcr_matrix(texture, bits_per_pixel);
            if texture_data.ycbcr_matrix.is_none() {
                return Err(Error::new("Unsupported YUV colorspace"));
            }
        }

        if matches!(texture.format, F::NV12 | F::NV21 | F::P010) {
            texture_data.nv12 = true;

            let bits_per_pixel = if texture.format == F::P010 { 10 } else { 8 };
            texture_data.ycbcr_matrix = ycbcr_matrix(texture, bits_per_pixel);
            if texture_data.ycbcr_matrix.is_none() {
                return Err(Error::new("Unsupported YUV colorspace"));
            }
        }
        let resource_view_desc = texture_2d_srv_desc(
            sdl_pixel_format_to_dxgi_main_resource_view_format(
                texture.format,
                self.output_colorspace,
            ),
            texture_desc.mip_levels as u32,
            0,
        );

        let main = texture_data
            .main_texture
            .clone()
            .ok_or_else(not_available)?;
        let (index, view) = self.create_srv(&main, &resource_view_desc)?;
        texture_data.main_srv_index = Some(index);
        texture_data.main_texture_resource_view = view;

        if texture_data.yuv {
            let u = texture_data
                .main_texture_u
                .clone()
                .ok_or_else(not_available)?;
            let (index, view) = self.create_srv(&u, &resource_view_desc)?;
            texture_data.main_srv_index_u = Some(index);
            texture_data.main_texture_resource_view_u = view;

            let v = texture_data
                .main_texture_v
                .clone()
                .ok_or_else(not_available)?;
            let (index, view) = self.create_srv(&v, &resource_view_desc)?;
            texture_data.main_srv_index_v = Some(index);
            texture_data.main_texture_resource_view_v = view;
        }

        if texture_data.nv12 {
            let mut nv_format = resource_view_desc.format;

            if texture.format == F::NV12 || texture.format == F::NV21 {
                nv_format = DXGI_FORMAT_R8G8_UNORM;
            } else if texture.format == F::P010 {
                nv_format = DXGI_FORMAT_R16G16_UNORM;
            }
            // (and Texture2D.PlaneSlice = 1)
            let nv_resource_view_desc =
                texture_2d_srv_desc(nv_format, texture_desc.mip_levels as u32, 1);

            let (index, view) = self.create_srv(&main, &nv_resource_view_desc)?;
            texture_data.main_srv_index_nv = Some(index);
            texture_data.main_texture_resource_view_nv = view;
        }

        if texture.access == TextureAccess::Target {
            let render_target_view_desc = texture_2d_rtv_desc(texture_desc.format);

            let heap = self
                .texture_rtv_descriptor_heap
                .as_ref()
                .ok_or_else(device_lost)?;
            let index = texture_data.main_srv_index.unwrap_or(0);
            let view = descriptor_at(heap, index, self.rtv_descriptor_size);
            texture_data.main_texture_render_target_view = view;

            device.create_render_target_view(&main, &render_target_view_desc, view);
        }

        Ok(())
    }

    /// Free the SRV slots of a texture's data.
    fn free_texture_srvs(&mut self, texture_data: &D3d12Texture) {
        if let Some(index) = texture_data.main_srv_index {
            self.free_srv_index(index);
        }
        if texture_data.yuv {
            for index in [texture_data.main_srv_index_u, texture_data.main_srv_index_v]
                .into_iter()
                .flatten()
            {
                self.free_srv_index(index);
            }
        }
        if texture_data.nv12 {
            if let Some(index) = texture_data.main_srv_index_nv {
                self.free_srv_index(index);
            }
        }
    }

    /// Translation of `D3D12_DestroyTexture()`, for the renderer's data of
    /// a texture (its resources are released as it's dropped).
    fn destroy_texture_data(&mut self, id: u32) {
        if !self.textures.contains_key(&id) {
            return;
        }

        /* Because SDL_DestroyTexture might be called while the data is in-flight, we need to issue the batch first
        Unfortunately, this means that deleting a lot of textures mid-frame will have poor performance. */
        let _ = self.issue_batch();

        let Some(texture_data) = self.textures.remove(&id) else {
            return;
        };
        self.free_texture_srvs(&texture_data);
        drop(texture_data);
        if self.texture_render_target == Some(id) {
            // (upstream keeps the dangling pointer, which is never used
            // again: the front end resets the target first)
            self.texture_render_target = None;
        }
    }

    /// Translation of `D3D12_UpdateTextureInternal()` (the plane's state
    /// is updated in `resource_state`).
    #[allow(clippy::too_many_arguments)]
    fn update_texture_internal(
        &mut self,
        texture: &D3d12Resource,
        plane: u32,
        (x, y, w, h): (i32, i32, i32, i32),
        pixels: &[u8],
        pitch: i32,
        resource_state: &mut u32,
    ) -> Result<()> {
        let device = self.device()?.clone();

        // Create an upload buffer, which will be used to write to the main texture.
        let mut texture_desc = texture.desc();
        texture_desc.width = w as u64;
        texture_desc.height = h as u32;
        if texture_desc.format == DXGI_FORMAT_NV12 || texture_desc.format == DXGI_FORMAT_P010 {
            texture_desc.width = (texture_desc.width + 1) & !1;
            texture_desc.height = (texture_desc.height + 1) & !1;
        }

        // Figure out how much we need to allocate for the upload buffer
        let (placed_texture_desc, num_rows, row_length, upload_size) =
            device.copyable_footprints(&texture_desc, plane, 0);
        let upload_desc = buffer_desc(upload_size);
        let row_pitch = placed_texture_desc.footprint.row_pitch;

        let heap_props = heap_properties(D3D12_HEAP_TYPE_UPLOAD);

        // Create the upload buffer
        let upload_buffer = device
            .create_committed_resource(
                &heap_props,
                D3D12_HEAP_FLAG_NONE,
                &upload_desc,
                D3D12_RESOURCE_STATE_GENERIC_READ,
                None,
            )
            .map_err(|hr| {
                hr_error(
                    "ID3D12Device::CreateCommittedResource [create upload buffer]",
                    hr,
                )
            })?;
        self.upload_buffers[self.current_upload_buffer] = Some(upload_buffer.clone());

        // Get a write-only pointer to data in the upload buffer:
        let texture_memory = match upload_buffer.map() {
            Ok(memory) => memory,
            Err(hr) => {
                self.upload_buffers[self.current_upload_buffer] = None;
                return Err(hr_error("ID3D12Resource::Map [map staging texture]", hr));
            }
        };

        let mut length = row_length as u32;
        let copied = if length == pitch as u32 && length == row_pitch {
            // SAFETY: the mapped upload buffer holds NumRows rows of
            // RowPitch (= length) bytes: one block of them.
            unsafe {
                copy_rows(
                    texture_memory,
                    0,
                    pixels,
                    0,
                    length as usize * num_rows as usize,
                    1,
                )
            }
        } else {
            if length > pitch as u32 {
                length = pitch as u32;
            }
            if length > row_pitch {
                length = row_pitch;
            }
            // SAFETY: the mapped upload buffer holds NumRows rows of
            // RowPitch bytes, and length <= RowPitch.
            unsafe {
                copy_rows(
                    texture_memory,
                    row_pitch as usize,
                    pixels,
                    pitch as usize,
                    length as usize,
                    num_rows as usize,
                )
            }
        };

        // Commit the changes back to the upload buffer:
        upload_buffer.unmap();
        // (upstream copies whatever the caller passes; a short buffer is an
        // error here, before anything is recorded)
        copied?;

        // Make sure the destination is in the correct resource state
        self.transition_resource(texture, *resource_state, D3D12_RESOURCE_STATE_COPY_DEST);
        *resource_state = D3D12_RESOURCE_STATE_COPY_DEST;

        let dst_location = subresource_location(texture, plane);
        let src_location = footprint_location(&upload_buffer, placed_texture_desc);

        if let Some(command_list) = &self.command_list {
            command_list.copy_texture_region(
                &dst_location,
                x as u32,
                y as u32,
                0,
                &src_location,
                None,
            );
        }

        // Transition the texture to be shader accessible
        self.transition_resource(
            texture,
            *resource_state,
            D3D12_RESOURCE_STATE_PIXEL_SHADER_RESOURCE,
        );
        *resource_state = D3D12_RESOURCE_STATE_PIXEL_SHADER_RESOURCE;

        self.current_upload_buffer += 1;
        // If we've used up all the upload buffers, we need to issue the batch
        if self.current_upload_buffer == SDL_D3D12_NUM_UPLOAD_BUFFERS {
            let _ = self.issue_batch();
        }

        Ok(())
    }

    /// `D3D12_UpdateTextureInternal()` on a plane of the texture `id`.
    #[allow(clippy::too_many_arguments)]
    fn update_plane(
        &mut self,
        id: u32,
        which: Plane,
        plane: u32,
        rect: (i32, i32, i32, i32),
        pixels: &[u8],
        pitch: i32,
    ) -> Result<()> {
        let texture_data = self.textures.get(&id).ok_or_else(not_available)?;
        let (resource, mut state) = texture_data.plane(which).ok_or_else(not_available)?;
        let result =
            self.update_texture_internal(&resource, plane, rect, pixels, pitch, &mut state);
        if let Some(texture_data) = self.textures.get_mut(&id) {
            *texture_data.state_mut(which) = state;
        }
        result
    }

    /// The key of the renderer's data of a texture.
    fn texture_id(texture: &TextureData) -> Result<u32> {
        texture_ref(texture).map(|r| r.id).ok_or_else(not_available)
    }

    /// After a texture's update: `We'll need to rebind this resource after
    /// updating it`.
    fn forget_bound_resource(&mut self, id: u32) {
        if let Some(texture_data) = self.textures.get(&id) {
            if texture_data.main_texture_resource_view.ptr == self.current_shader_resource.ptr {
                // We'll need to rebind this resource after updating it
                self.current_shader_resource.ptr = 0;
            }
        }
    }

    /// Translation of `D3D12_UpdateTexture()`.
    fn update_texture_pixels(
        &mut self,
        texture: &TextureData,
        rect: &Rect,
        src_pixels: &[u8],
        mut src_pitch: i32,
    ) -> Result<()> {
        use PixelFormat as F;
        let id = Self::texture_id(texture)?;
        let texture_data = self.textures.get(&id).ok_or_else(not_available)?;
        let (yuv, nv12) = (texture_data.yuv, texture_data.nv12);
        let rows = rect.h.max(0) as usize;
        let full = (rect.x, rect.y, rect.w, rect.h);

        self.update_plane(id, Plane::Main, 0, full, src_pixels, src_pitch)?;
        let mut src_pixels = src_pixels;
        if yuv {
            if texture.format == F::I444 || texture.format == F::I4FL {
                // Skip to the correct offset into the next texture
                src_pixels = plane(src_pixels, rows * src_pitch as usize)?;
                self.update_plane(id, Plane::U, 0, full, src_pixels, src_pitch)?;

                // Skip to the correct offset into the next texture
                src_pixels = plane(src_pixels, rows * src_pitch as usize)?;
                self.update_plane(id, Plane::V, 0, full, src_pixels, src_pitch)?;
            } else {
                let bpp = texture.format.bytes_per_pixel() as i32;
                let uv_pitch = ((src_pitch / bpp + 1) / 2) * bpp;
                let half = (rect.x / 2, rect.y / 2, (rect.w + 1) / 2, (rect.h + 1) / 2);
                let (first, second) = if texture.format == F::YV12 {
                    (Plane::V, Plane::U)
                } else {
                    (Plane::U, Plane::V)
                };

                // Skip to the correct offset into the next texture
                src_pixels = plane(src_pixels, rows * src_pitch as usize)?;
                self.update_plane(id, first, 0, half, src_pixels, uv_pitch)?;

                // Skip to the correct offset into the next texture
                src_pixels = plane(src_pixels, rows.div_ceil(2) * uv_pitch as usize)?;
                self.update_plane(id, second, 0, half, src_pixels, uv_pitch)?;
            }
        }

        if nv12 {
            // Skip to the correct offset into the next texture
            src_pixels = plane(src_pixels, rows * src_pitch as usize)?;

            if texture.format == F::P010 {
                src_pitch = (src_pitch + 3) & !3;
            } else {
                src_pitch = (src_pitch + 1) & !1;
            }
            let even = (rect.x, rect.y, (rect.w + 1) & !1, (rect.h + 1) & !1);
            self.update_plane(id, Plane::Main, 1, even, src_pixels, src_pitch)?;
        }
        self.forget_bound_resource(id);
        Ok(())
    }

    /// Translation of `D3D12_UpdateTextureYUV()`.
    fn update_texture_yuv_planes(
        &mut self,
        texture: &TextureData,
        rect: &Rect,
        (y_plane, y_pitch): (&[u8], i32),
        (u_plane, u_pitch): (&[u8], i32),
        (v_plane, v_pitch): (&[u8], i32),
    ) -> Result<()> {
        let id = Self::texture_id(texture)?;
        if !self.textures.contains_key(&id) {
            return Err(not_available());
        }

        let full = (rect.x, rect.y, rect.w, rect.h);
        self.update_plane(id, Plane::Main, 0, full, y_plane, y_pitch)?;
        if texture.format == PixelFormat::I444 || texture.format == PixelFormat::I4FL {
            self.update_plane(id, Plane::U, 0, full, u_plane, u_pitch)?;
            self.update_plane(id, Plane::V, 0, full, v_plane, v_pitch)?;
        } else {
            let half = (rect.x / 2, rect.y / 2, (rect.w + 1) / 2, (rect.h + 1) / 2);
            self.update_plane(id, Plane::U, 0, half, u_plane, u_pitch)?;
            self.update_plane(id, Plane::V, 0, half, v_plane, v_pitch)?;
        }
        self.forget_bound_resource(id);
        Ok(())
    }

    /// Translation of `D3D12_UpdateTextureNV()`.
    fn update_texture_nv_planes(
        &mut self,
        texture: &TextureData,
        rect: &Rect,
        (y_plane, y_pitch): (&[u8], i32),
        (uv_plane, uv_pitch): (&[u8], i32),
    ) -> Result<()> {
        let id = Self::texture_id(texture)?;
        if !self.textures.contains_key(&id) {
            return Err(not_available());
        }

        let full = (rect.x, rect.y, rect.w, rect.h);
        self.update_plane(id, Plane::Main, 0, full, y_plane, y_pitch)?;
        let even = (rect.x, rect.y, (rect.w + 1) & !1, (rect.h + 1) & !1);
        self.update_plane(id, Plane::Main, 1, even, uv_plane, uv_pitch)?;
        self.forget_bound_resource(id);
        Ok(())
    }

    /// Translation of `D3D12_UpdateVertexBuffer()`.
    fn update_vertex_buffer(&mut self) -> Result<()> {
        let vbidx = self.current_vertex_buffer;
        let data_size_in_bytes = size_of_val(&self.verts[..]);

        if data_size_in_bytes == 0 {
            return Ok(()); // nothing to do.
        }

        if self.issue_batch && self.issue_batch().is_err() {
            return Err(Error::new("Failed to issue intermediate batch"));
        }

        // If the existing vertex buffer isn't big enough, we need to recreate a big enough one
        if data_size_in_bytes > self.vertex_buffers[vbidx].size {
            let _ = self.create_vertex_buffer(vbidx, data_size_in_bytes);
        }

        // FIXME (upstream): when the buffer couldn't be made, its resource
        // is NULL here, and mapped.
        let vertex_buffer = self.vertex_buffers[vbidx]
            .resource
            .clone()
            .ok_or_else(device_lost)?;
        let vertex_buffer_data = vertex_buffer
            .map_write_only()
            .map_err(|hr| hr_error("ID3D12Resource::Map [vertex buffer]", hr))?;
        // SAFETY: the mapped buffer holds at least `data_size_in_bytes`
        // (its size); the vertices are that many bytes.
        unsafe {
            std::ptr::copy_nonoverlapping(
                self.verts.as_ptr().cast::<u8>(),
                vertex_buffer_data,
                data_size_in_bytes,
            )
        };
        vertex_buffer.unmap();

        self.vertex_buffers[vbidx].view.size_in_bytes = data_size_in_bytes as u32;

        if let Some(command_list) = &self.command_list {
            command_list.ia_set_vertex_buffers(0, &[self.vertex_buffers[vbidx].view]);
        }

        self.current_vertex_buffer += 1;
        if self.current_vertex_buffer >= SDL_D3D12_NUM_VERTEX_BUFFERS {
            self.current_vertex_buffer = 0;
            self.issue_batch = true;
        }

        Ok(())
    }

    /// Translation of `D3D12_UpdateViewport()`.
    fn update_viewport(&mut self) -> bool {
        let viewport = self.current_viewport;
        let rotation = self.get_rotation_for_current_render_target();

        if viewport.w == 0 || viewport.h == 0 {
            /* If the viewport is empty, assume that it is because
             * SDL_CreateRenderer is calling it, and will call it again later
             * with a non-empty viewport.
             */
            // SDL_Log("%s, no viewport was set!", SDL_FUNCTION);
            return false;
        }

        /* Make sure the SDL viewport gets rotated to that of the physical display's rotation.
         * Keep in mind here that the Y-axis will be been inverted (from Direct3D's
         * default coordinate system) so rotations will be done in the opposite
         * direction of the DXGI_MODE_ROTATION enumeration.
         */
        let projection = match rotation {
            DXGI_MODE_ROTATION_IDENTITY => Float4X4::identity(),
            DXGI_MODE_ROTATION_ROTATE270 => Float4X4::rotation_z(std::f32::consts::PI * 0.5),
            DXGI_MODE_ROTATION_ROTATE180 => Float4X4::rotation_z(std::f32::consts::PI),
            DXGI_MODE_ROTATION_ROTATE90 => Float4X4::rotation_z(-std::f32::consts::PI * 0.5),
            _ => {
                // (SDL_SetError("An unknown DisplayOrientation is being used"))
                return false;
            }
        };

        // Update the view matrix
        let mut view = Float4X4::default();
        view.m[0][0] = 2.0 / viewport.w as f32;
        view.m[1][1] = -2.0 / viewport.h as f32;
        view.m[2][2] = 1.0;
        view.m[3][0] = -1.0;
        view.m[3][1] = 1.0;
        view.m[3][3] = 1.0;

        /* Combine the projection + view matrix together now, as both only get
         * set here (as of this writing, on Dec 26, 2013).  When done, store it
         * for eventual transfer to the GPU.
         */
        self.projection_and_view = Float4X4::multiply(&view, &projection);

        /* Update the Direct3D viewport, which seems to be aligned to the
         * swap buffer's coordinate space, which is always in either
         * a landscape mode, for all Windows 8/RT devices, or a portrait mode,
         * for Windows Phone devices.
         */
        let swap_dimensions = is_display_rotated_90_degrees(rotation);
        let orientation_aligned_viewport = if swap_dimensions {
            FRect::new(
                viewport.y as f32,
                viewport.x as f32,
                viewport.h as f32,
                viewport.w as f32,
            )
        } else {
            FRect::new(
                viewport.x as f32,
                viewport.y as f32,
                viewport.w as f32,
                viewport.h as f32,
            )
        };

        let d3dviewport = D3d12Viewport {
            top_left_x: orientation_aligned_viewport.x,
            top_left_y: orientation_aligned_viewport.y,
            width: orientation_aligned_viewport.w,
            height: orientation_aligned_viewport.h,
            min_depth: 0.0,
            max_depth: 1.0,
        };
        // SDL_Log("%s: D3D viewport = {%f,%f,%f,%f}", SDL_FUNCTION, d3dviewport.TopLeftX, d3dviewport.TopLeftY, d3dviewport.Width, d3dviewport.Height);
        if let Some(command_list) = &self.command_list {
            command_list.rs_set_viewport(&d3dviewport);
        }

        self.viewport_dirty = false;

        true
    }

    /// The HDR headroom of the output (`renderer->target->HDR_headroom` or
    /// `renderer->HDR_headroom`).
    fn output_headroom(&self, textures: &TextureStore) -> f32 {
        match self.target.and_then(|t| textures.get(t)) {
            Some(target) => target.hdr_headroom,
            None => self.hdr_headroom,
        }
    }

    /// Translation of `D3D12_SetupShaderConstants()`.
    fn setup_shader_constants(
        &self,
        cmd: &DrawCmd,
        texture: Option<(&SourceTexture, &D3d12Texture)>,
        output_headroom: f32,
    ) -> PixelShaderConstants {
        use PixelFormat as F;
        let mut constants = PixelShaderConstants {
            sc_rgb_output: self.rendering_linear_space() as i32 as f32,
            color_scale: cmd.color_scale,
            ..Default::default()
        };

        if let Some((texture, texture_data)) = texture {
            match texture.format {
                F::INDEX8 => match cmd.texture_scale_mode {
                    ScaleMode::Nearest => constants.texture_type = TEXTURETYPE_PALETTE_NEAREST,
                    ScaleMode::Linear => constants.texture_type = TEXTURETYPE_PALETTE_LINEAR,
                    ScaleMode::PixelArt => constants.texture_type = TEXTURETYPE_PALETTE_PIXELART,
                },
                F::YV12 | F::IYUV | F::I444 => {
                    constants.texture_type = TEXTURETYPE_YUV;
                    constants.input_type = INPUTTYPE_SRGB;
                }
                F::NV12 => {
                    constants.texture_type = TEXTURETYPE_NV12;
                    constants.input_type = INPUTTYPE_SRGB;
                }
                F::NV21 => {
                    constants.texture_type = TEXTURETYPE_NV21;
                    constants.input_type = INPUTTYPE_SRGB;
                }
                F::P010 => {
                    constants.texture_type = TEXTURETYPE_NV12;
                    constants.input_type = INPUTTYPE_HDR10;
                }
                F::I0FL | F::I4FL => {
                    constants.texture_type = TEXTURETYPE_YUV;
                    constants.input_type = INPUTTYPE_HDR10;
                }
                _ => {
                    if cmd.texture_scale_mode == ScaleMode::PixelArt {
                        constants.texture_type = TEXTURETYPE_RGB_PIXELART;
                    } else {
                        constants.texture_type = TEXTURETYPE_RGB;
                    }
                    if texture.colorspace == Colorspace::SRGB_LINEAR {
                        constants.input_type = INPUTTYPE_SCRGB;
                    } else if texture.colorspace == Colorspace::HDR10 {
                        constants.input_type = INPUTTYPE_HDR10;
                    } else {
                        // The sampler will convert from sRGB to linear on load if working in linear colorspace
                        constants.input_type = INPUTTYPE_UNSPECIFIED;
                    }
                }
            }

            if constants.texture_type == TEXTURETYPE_PALETTE_LINEAR
                || constants.texture_type == TEXTURETYPE_PALETTE_PIXELART
                || constants.texture_type == TEXTURETYPE_RGB_PIXELART
            {
                constants.texture_width = texture.w as f32;
                constants.texture_height = texture.h as f32;
                constants.texel_width = 1.0 / constants.texture_width;
                constants.texel_height = 1.0 / constants.texture_height;
            }

            constants.sdr_white_point = texture.sdr_white_point;

            if texture.hdr_headroom > output_headroom && output_headroom > 0.0 {
                constants.tonemap_method = TONEMAP_CHROME;
                constants.tonemap_factor1 =
                    output_headroom / (texture.hdr_headroom * texture.hdr_headroom);
                constants.tonemap_factor2 = 1.0 / output_headroom;
            }

            if let Some(matrix) = &texture_data.ycbcr_matrix {
                constants.ycbcr_matrix = *matrix;
            }
        }
        constants
    }

    /// Translation of `D3D12_SetDrawState()`.
    fn set_draw_state(
        &mut self,
        cmd: &DrawCmd,
        shader_constants: Option<&PixelShaderConstants>,
        topology: u32,
        shader_resources: &[CpuDescriptorHandle],
        shader_samplers: &[CpuDescriptorHandle],
    ) -> Result<()> {
        let render_target_view = self.get_current_render_target_view();
        let blend_mode = cmd.blend;
        let mut update_subresource = false;
        let mut rtv_format = self.render_target_format;
        let mut current_pipeline_state = self.current_pipeline_state;
        let shader = select_shader(self.current_colorspace, shader_constants);
        let command_list = self.command_list.clone().ok_or_else(device_lost)?;

        let mut shader_resources_changed = false;
        if shader_resources.len() != self.num_current_shader_resources
            || (!shader_resources.is_empty()
                && shader_resources[0].ptr != self.current_shader_resource.ptr)
        {
            shader_resources_changed = true;
        }

        let mut shader_samplers_changed = false;
        if shader_samplers.len() != self.num_current_shader_samplers
            || (!shader_samplers.is_empty()
                && shader_samplers[0].ptr != self.current_shader_sampler.ptr)
        {
            shader_samplers_changed = true;
        }

        if let Some(id) = self.texture_render_target {
            if let Some(target) = self.textures.get(&id) {
                rtv_format = target.main_texture_format;
            }
        }

        // See if we need to change the pipeline state
        let matches = |p: &PipelineState| {
            p.shader == shader
                && p.blend_mode == blend_mode
                && p.topology == topology
                && p.rtv_format == rtv_format
        };
        if !current_pipeline_state.is_some_and(|i| matches(&self.pipeline_states[i])) {
            /* Find the matching pipeline.
               NOTE: Although it may seem inefficient to linearly search through ~450 pipelines
               to find the correct one, in profiling this doesn't come up at all.
               It's unlikely that using a hash table would affect performance a measurable amount unless
               it's a degenerate case that's changing the pipeline state dozens of times per frame.
            */
            current_pipeline_state = self.pipeline_states.iter().position(matches);

            // If we didn't find a match, create a new one -- it must mean the blend mode is non-standard
            let index = match current_pipeline_state {
                Some(index) => index,
                // The error has been set inside D3D12_CreatePipelineState()
                None => self.create_pipeline_state(shader, blend_mode, topology, rtv_format)?,
            };
            current_pipeline_state = Some(index);

            let pipeline = &self.pipeline_states[index];
            command_list.set_pipeline_state(&pipeline.pipeline_state);
            if let Some(root_signature) =
                &self.root_signatures[pipeline.shader.root_signature_type() as usize]
            {
                command_list.set_graphics_root_signature(root_signature);
            }
            // When we change these we will need to re-upload the constant buffer and reset any descriptors
            update_subresource = true;
            shader_resources_changed = true;
            shader_samplers_changed = true;
            self.current_pipeline_state = current_pipeline_state;
        }
        let Some(current_pipeline_state) = current_pipeline_state else {
            return Err(device_lost());
        };

        if render_target_view.ptr != self.current_render_target_view.ptr {
            command_list.om_set_render_targets(&[render_target_view], None);
            self.current_render_target_view = render_target_view;
        }

        if self.viewport_dirty && self.update_viewport() {
            // vertexShaderConstantsData.projectionAndView has changed
            update_subresource = true;
        }

        if self.cliprect_dirty {
            // D3D12_GetViewportAlignedD3DRect will have set the SDL error
            let scissor_rect = self.get_viewport_aligned_d3d_rect(&self.current_cliprect, true)?;
            command_list.rs_set_scissor_rect(&scissor_rect);
            self.cliprect_dirty = false;
        }

        if shader_resources_changed {
            if let Some(heap) = &self.srv_descriptor_heap {
                for (i, &resource) in shader_resources.iter().enumerate() {
                    let gpu_handle = cpu_to_gpu_handle(heap, resource);
                    command_list.set_graphics_root_descriptor_table(i as i32 + 2, gpu_handle);
                }
            }
            self.num_current_shader_resources = shader_resources.len();
            if let Some(&first) = shader_resources.first() {
                self.current_shader_resource.ptr = first.ptr;
            }
        }

        if shader_samplers_changed {
            let heap = self
                .sampler_descriptor_heap
                .clone()
                .ok_or_else(device_lost)?;
            if let Some(&sampler) = shader_samplers.first() {
                let gpu_handle = cpu_to_gpu_handle(&heap, sampler);

                // Figure out the correct sampler descriptor table index based on the type of shader
                let table_index = match shader {
                    Shader::Rgb | Shader::RgbSimple => 3,
                    Shader::Advanced | Shader::RgbPq => 5,
                    _ => return Err(Error::new(
                        "[direct3d12] Trying to set a sampler for a shader which doesn't have one",
                    )),
                };

                command_list.set_graphics_root_descriptor_table(table_index, gpu_handle);
            }

            if let Some(&sampler) = shader_samplers.get(1) {
                let gpu_handle = cpu_to_gpu_handle(&heap, sampler);
                let table_index = 6;
                command_list.set_graphics_root_descriptor_table(table_index, gpu_handle);
            }

            self.num_current_shader_samplers = shader_samplers.len();
            if let Some(&first) = shader_samplers.first() {
                self.current_shader_sampler.ptr = first.ptr;
            }
        }

        if update_subresource {
            // Our model matrix is always identity
            let vertex_constants = VertexShaderConstants {
                mpv: self.projection_and_view,
            };
            command_list.set_graphics_root_32bit_constants(0, &vertex_constants.words(), 0);
        }

        let solid_constants;
        let shader_constants = match shader_constants {
            Some(constants) => constants,
            None => {
                solid_constants = self.setup_shader_constants(cmd, None, 0.0);
                &solid_constants
            }
        };

        let pipeline = &mut self.pipeline_states[current_pipeline_state];
        if update_subresource || shader_constants.words() != pipeline.shader_constants.words() {
            command_list.set_graphics_root_32bit_constants(1, &shader_constants.words(), 0);

            pipeline.shader_constants = *shader_constants;
        }

        Ok(())
    }

    /// Translation of `D3D12_GetSamplerState()`.
    fn get_sampler_state(
        &mut self,
        format: PixelFormat,
        mut scale_mode: ScaleMode,
        address_u: TextureAddressMode,
        address_v: TextureAddressMode,
    ) -> Result<CpuDescriptorHandle> {
        if format == PixelFormat::INDEX8 {
            // We'll do linear sampling in the shader if needed
            scale_mode = ScaleMode::Nearest;
        }

        let key = render_sampler_hashkey(scale_mode, address_u, address_v);
        crate::sdl_assert!(key < self.samplers.len());
        if !self.samplers_created[key] {
            let sampler_desc = sampler_desc(scale_mode, address_u, address_v)?;
            self.device()?
                .create_sampler(&sampler_desc, self.samplers[key]);
            self.samplers_created[key] = true;
        }
        Ok(self.samplers[key])
    }

    /// `D3D12_TransitionResource()` of a plane of the texture `id` to the
    /// pixel shader resource state, which it's then in.
    fn transition_plane_to_shader_resource(&mut self, id: u32, plane: Plane) {
        if let Some((resource, state)) = self.textures.get(&id).and_then(|t| t.plane(plane)) {
            self.transition_resource(&resource, state, D3D12_RESOURCE_STATE_PIXEL_SHADER_RESOURCE);
        }
        if let Some(texture_data) = self.textures.get_mut(&id) {
            *texture_data.state_mut(plane) = D3D12_RESOURCE_STATE_PIXEL_SHADER_RESOURCE;
        }
    }

    /// Translation of `D3D12_SetCopyState()`.
    fn set_copy_state(&mut self, cmd: &DrawCmd, textures: &TextureStore) -> Result<()> {
        let handle = cmd.texture.ok_or_else(invalid_texture)?;
        let texture = textures.get(handle).ok_or_else(invalid_texture)?;
        let source = SourceTexture::of(texture);
        let output_headroom = self.output_headroom(textures);
        let id = Self::texture_id(texture)?;
        let texture_data = self.textures.get(&id).ok_or_else(not_available)?;
        let mut shader_resources = Vec::with_capacity(3);
        let mut shader_samplers = Vec::with_capacity(2);

        let constants =
            self.setup_shader_constants(cmd, Some((&source, texture_data)), output_headroom);
        let (yuv, nv12, palette) = (texture_data.yuv, texture_data.nv12, texture_data.palette);
        let views = [
            texture_data.main_texture_resource_view,
            texture_data.main_texture_resource_view_u,
            texture_data.main_texture_resource_view_v,
            texture_data.main_texture_resource_view_nv,
        ];

        self.transition_plane_to_shader_resource(id, Plane::Main);
        shader_resources.push(views[0]);

        shader_samplers.push(self.get_sampler_state(
            source.format,
            cmd.texture_scale_mode,
            cmd.texture_address_mode_u,
            cmd.texture_address_mode_v,
        )?);

        if source.has_palette {
            // Note (upstream): a texture whose palette the front end hasn't
            // given it here is dereferenced as NULL there.
            let palette_data = palette
                .and_then(|p| self.palettes.get_mut(&p))
                .ok_or_else(|| Error::invalid_param("palette"))?;
            let state = palette_data.resource_state;
            palette_data.resource_state = D3D12_RESOURCE_STATE_PIXEL_SHADER_RESOURCE;
            let view = palette_data.resource_view;
            if let Some(resource) = palette_data.texture.clone() {
                self.transition_resource(
                    &resource,
                    state,
                    D3D12_RESOURCE_STATE_PIXEL_SHADER_RESOURCE,
                );
            }
            shader_resources.push(view);

            shader_samplers.push(self.get_sampler_state(
                PixelFormat::UNKNOWN,
                ScaleMode::Nearest,
                TextureAddressMode::Clamp,
                TextureAddressMode::Clamp,
            )?);
        }

        if yuv {
            self.transition_plane_to_shader_resource(id, Plane::U);
            shader_resources.push(views[1]);

            self.transition_plane_to_shader_resource(id, Plane::V);
            shader_resources.push(views[2]);
        } else if nv12 {
            self.transition_plane_to_shader_resource(id, Plane::Main);
            shader_resources.push(views[3]);
        }
        self.set_draw_state(
            cmd,
            Some(&constants),
            D3D12_PRIMITIVE_TOPOLOGY_TYPE_TRIANGLE,
            &shader_resources,
            &shader_samplers,
        )
    }

    /// Translation of `D3D12_DrawPrimitives()`.
    fn draw_primitives(&self, primitive_topology: u32, vertex_start: usize, vertex_count: usize) {
        if let Some(command_list) = &self.command_list {
            command_list.ia_set_primitive_topology(primitive_topology);
            command_list.draw_instanced(vertex_count as u32, 1, vertex_start as u32, 0);
        }
    }

    /// Translation of `D3D12_QueueDrawPoints()` (lines and points queue
    /// vertices the same way).
    fn queue_points(&mut self, cmd: &mut DrawCmd, points: &[FPoint]) {
        let mut color = cmd.color;
        let convert_color = self.rendering_linear_space();

        cmd.first = self.verts.len();
        cmd.count = points.len();

        if convert_color {
            self.convert_to_linear(&mut color);
        }

        for p in points {
            self.verts.push(VertexPositionColor::new(
                p.x + 0.5,
                p.y + 0.5,
                0.0,
                0.0,
                color,
            ));
        }
    }

    /// Translation of `D3D12_CreateRenderer()` for a window.
    pub(crate) fn for_window(
        window: Window,
        output_colorspace: Colorspace,
    ) -> Result<D3d12Renderer> {
        let mut data = D3d12Renderer::new(window, output_colorspace);
        if data.hwnd().is_none() {
            return Err(Error::new("Couldn't get window handle"));
        }

        if data
            .window
            .flags()
            .unwrap_or_default()
            .contains(WindowFlags::TRANSPARENT)
        {
            // D3D12 removed the swap effect needed to support transparent windows, use D3D11 instead
            return Err(Error::new(
                "The direct3d12 renderer doesn't work with transparent windows",
            ));
        }

        // (SDL_SetupRendererColorspace())
        if output_colorspace != Colorspace::SRGB
            && output_colorspace != Colorspace::SRGB_LINEAR
            && output_colorspace != Colorspace::HDR10
        {
            return Err(Error::new("Unsupported output colorspace"));
        }

        // (renderer->window is the window from the start)

        // Initialize Direct3D resources
        let result = data
            .create_device_resources()
            .and_then(|()| data.create_window_size_dependent_resources());
        if let Err(e) = result {
            // (SDL_CreateRenderer() destroys the renderer that failed)
            data.destroy();
            return Err(e);
        }

        let device = data.device()?;
        let mut formats = Vec::new();
        for &(sdl, unorm, srgb) in DXGI_FORMAT_MAP {
            let mut unorm = FeatureDataFormatSupport {
                format: unorm,
                ..Default::default()
            };
            if device.check_feature_support(D3D12_FEATURE_FORMAT_SUPPORT, &mut unorm) < 0 {
                continue;
            }

            let mut srgb = FeatureDataFormatSupport {
                format: srgb,
                ..Default::default()
            };
            if device.check_feature_support(D3D12_FEATURE_FORMAT_SUPPORT, &mut srgb) < 0 {
                continue;
            }

            if (unorm.support1 & D3D12_FORMAT_SUPPORT1_TEXTURE2D) != 0
                && (srgb.support1 & D3D12_FORMAT_SUPPORT1_TEXTURE2D) != 0
            {
                formats.push(sdl);
            }
        }
        formats.extend([
            PixelFormat::INDEX8,
            PixelFormat::YV12,
            PixelFormat::IYUV,
            PixelFormat::I444,
            PixelFormat::NV12,
            PixelFormat::NV21,
            PixelFormat::P010,
            PixelFormat::I0FL,
            PixelFormat::I4FL,
        ]);
        data.texture_formats = formats;

        Ok(data)
    }

    /// The renderer's data before the device exists (the `SDL_calloc()`
    /// and the setup of `D3D12_CreateRenderer()`).
    fn new(window: Window, output_colorspace: Colorspace) -> D3d12Renderer {
        let mut data = D3d12Renderer {
            window,
            output_colorspace,
            current_colorspace: output_colorspace,
            hdr_headroom: 1.0,
            target: None,
            textures: HashMap::new(),
            next_texture: 1,
            palettes: HashMap::new(),
            next_palette: 1,
            verts: Vec::new(),
            texture_formats: Vec::new(),
            props: None,
            swap_effect: 0,
            swap_flags: 0,
            sync_interval: 0,
            present_flags: DXGI_PRESENT_ALLOW_TEARING,
            render_target_format: DXGI_FORMAT_UNKNOWN,
            pixel_size_changed: false,
            rtv_descriptor_heap: None,
            rtv_descriptor_size: 0,
            texture_rtv_descriptor_heap: None,
            srv_descriptor_heap: None,
            srv_descriptor_size: 0,
            sampler_descriptor_heap: None,
            sampler_descriptor_size: 0,
            command_allocators: Default::default(),
            render_targets: Default::default(),
            fence_value: 0,
            current_back_buffer_index: 0,
            fence: None,
            fence_event: null_mut(),
            root_signatures: Default::default(),
            pipeline_states: Vec::new(),
            current_pipeline_state: None,
            vertex_buffers: (0..SDL_D3D12_NUM_VERTEX_BUFFERS)
                .map(|_| VertexBuffer::default())
                .collect(),
            samplers: [CpuDescriptorHandle::default(); RENDER_SAMPLER_COUNT],
            samplers_created: [false; RENDER_SAMPLER_COUNT],
            upload_buffers: Default::default(),
            current_upload_buffer: 0,
            srv_pool_head: None,
            srv_pool_nodes: vec![SrvPoolNode::default(); SDL_D3D12_MAX_NUM_TEXTURES],
            projection_and_view: Float4X4::default(),
            rotation: DXGI_MODE_ROTATION_UNSPECIFIED,
            texture_render_target: None,
            current_render_target_view: CpuDescriptorHandle::default(),
            num_current_shader_resources: 0,
            current_shader_resource: CpuDescriptorHandle::default(),
            num_current_shader_samplers: 0,
            current_shader_sampler: CpuDescriptorHandle::default(),
            cliprect_dirty: false,
            current_cliprect_enabled: false,
            current_cliprect: Rect::default(),
            current_viewport: Rect::default(),
            current_viewport_rotation: 0,
            viewport_dirty: false,
            identity: Float4X4::identity(),
            current_vertex_buffer: 0,
            issue_batch: false,
            command_list: None,
            command_queue: None,
            debug_interface: None,
            d3d_device: None,
            swap_chain: None,
            dxgi_debug: None,
            dxgi_adapter: None,
            dxgi_factory: None,
            h_dxgi_mod: None,
            h_d3d12_mod: None,
        };
        data.invalidate_cached_state();
        data
    }

    /// Translation of `D3D12_RenderReadPixels()`.
    fn render_read_pixels(&mut self, rect: &Rect) -> Result<Surface<'static>> {
        let back_buffer = match self.texture_render_target {
            Some(id) => self.textures.get(&id).and_then(|t| t.main_texture.clone()),
            None => self.render_targets[self.current_back_buffer_index].clone(),
        };
        let back_buffer = back_buffer.ok_or_else(device_lost)?;
        let device = self.device()?.clone();

        // Create a staging texture to copy the screen's data to:
        let mut texture_desc = back_buffer.desc();
        texture_desc.width = rect.w as u64;
        texture_desc.height = rect.h as u32;

        // Figure out how much we need to allocate for the upload buffer
        let (_, _, _, readback_size) = device.copyable_footprints(&texture_desc, 0, 0);
        let readback_desc = buffer_desc(readback_size);

        let heap_props = heap_properties(D3D12_HEAP_TYPE_READBACK);

        let readback_buffer = device
            .create_committed_resource(
                &heap_props,
                D3D12_HEAP_FLAG_NONE,
                &readback_desc,
                D3D12_RESOURCE_STATE_COPY_DEST,
                None,
            )
            .map_err(|hr| hr_error("ID3D12Device::CreateTexture2D [create staging texture]", hr))?;

        // Transition the render target to be copyable from
        self.transition_resource(
            &back_buffer,
            D3D12_RESOURCE_STATE_RENDER_TARGET,
            D3D12_RESOURCE_STATE_COPY_SOURCE,
        );

        // Copy the desired portion of the back buffer to the staging texture:
        // FIXME (upstream): on this error the render target is left in the
        // copy source state (no rotation makes it fail yet).
        // (D3D12_GetViewportAlignedD3DRect will have set the SDL error)
        let src_rect = self.get_viewport_aligned_d3d_rect(rect, false)?;
        let src_box = D3d12Box {
            left: src_rect.left as u32,
            right: src_rect.right as u32,
            top: src_rect.top as u32,
            bottom: src_rect.bottom as u32,
            front: 0,
            back: 1,
        };

        // Issue the copy texture region
        let format = dxgi_format_to_sdl_pixel_format(texture_desc.format);
        let bpp = format.bytes_per_pixel();
        let pitched_desc = SubresourceFootprint {
            format: texture_desc.format,
            width: texture_desc.width as u32,
            height: texture_desc.height,
            depth: 1,
            row_pitch: d3d12_align(
                texture_desc.width as u32 * bpp,
                D3D12_TEXTURE_DATA_PITCH_ALIGNMENT,
            ),
        };

        let placed_texture_desc = PlacedSubresourceFootprint {
            offset: 0,
            footprint: pitched_desc,
        };

        let dst_location = footprint_location(&readback_buffer, placed_texture_desc);
        let src_location = subresource_location(&back_buffer, 0);

        if let Some(command_list) = &self.command_list {
            command_list.copy_texture_region(&dst_location, 0, 0, 0, &src_location, Some(&src_box));
        }

        // We need to issue the command list for the copy to finish
        let _ = self.issue_batch();

        // Transition the render target back to a render target
        self.transition_resource(
            &back_buffer,
            D3D12_RESOURCE_STATE_COPY_SOURCE,
            D3D12_RESOURCE_STATE_RENDER_TARGET,
        );

        // Map the staging texture's data to CPU-accessible memory:
        let texture_memory = readback_buffer
            .map()
            .map_err(|hr| hr_error("ID3D12Resource::Map [map staging texture]", hr))?;

        let row = rect.w.max(0) as usize * bpp as usize;
        let len = match rect.h.max(0) as usize {
            0 => 0,
            h => ((h - 1) * pitched_desc.row_pitch as usize + row).min(readback_size as usize),
        };
        // SAFETY: the mapped readback buffer holds `readback_size` bytes,
        // until it's unmapped below.
        let pixels = unsafe { std::slice::from_raw_parts(texture_memory, len) };
        let output = crate::video::surface::duplicate_pixels(
            rect.w,
            rect.h,
            format,
            self.current_colorspace,
            Some(pixels),
            pitched_desc.row_pitch as i32,
        );

        // Unmap the texture:
        readback_buffer.unmap();

        output
    }

    /// Translation of `D3D12_RenderPresent()` (the error with it).
    fn render_present(&mut self) -> Result<()> {
        if self.d3d_device.is_none() {
            return Err(device_lost());
        }
        let (Some(command_list), Some(command_queue), Some(swap_chain), Some(fence)) = (
            self.command_list.clone(),
            self.command_queue.clone(),
            self.swap_chain.clone(),
            self.fence.clone(),
        ) else {
            return Err(device_lost());
        };

        // Transition the render target to present state
        if let Some(render_target) = &self.render_targets[self.current_back_buffer_index] {
            self.transition_resource(
                render_target,
                D3D12_RESOURCE_STATE_RENDER_TARGET,
                D3D12_RESOURCE_STATE_PRESENT,
            );
        }

        // Issue the command list
        command_list.close();
        command_queue.execute_graphics_command_list(&command_list);

        /* The application may optionally specify "dirty" or "scroll"
         * rects to improve efficiency in certain scenarios.
         */
        let result = swap_chain.present(self.sync_interval, self.present_flags);

        if result < 0 && result != DXGI_ERROR_WAS_STILL_DRAWING {
            // (the references taken here go before the device does)
            drop((command_list, command_queue, swap_chain, fence));

            /* If the device was removed either by a disconnect or a driver upgrade, we
             * must recreate all device resources.
             */
            if result == DXGI_ERROR_DEVICE_REMOVED {
                if self.handle_device_lost() {
                    Err(Error::new("Present failed, device lost"))
                } else {
                    // Recovering from device lost failed, error is already set
                    Err(device_lost())
                }
            } else if result == DXGI_ERROR_INVALID_CALL {
                // We probably went through a fullscreen <-> windowed transition
                let _ = self.create_window_size_dependent_resources();
                Err(hr_error("IDXGISwapChain::Present", result))
            } else {
                Err(hr_error("IDXGISwapChain::Present", result))
            }
        } else {
            // Wait for the GPU and move to the next frame
            command_queue.signal(&fence, self.fence_value);

            if fence.completed_value() < self.fence_value {
                fence.set_event_on_completion(self.fence_value, self.fence_event);
                // SAFETY: the renderer's event handle.
                unsafe { WaitForSingleObjectEx(self.fence_event, INFINITE, 0) };
            }

            self.fence_value += 1;
            self.current_back_buffer_index = swap_chain.current_back_buffer_index() as usize;

            // Reset the command allocator and command list, and transition back to render target
            self.reset_command_list();
            if let Some(render_target) = &self.render_targets[self.current_back_buffer_index] {
                self.transition_resource(
                    render_target,
                    D3D12_RESOURCE_STATE_PRESENT,
                    D3D12_RESOURCE_STATE_RENDER_TARGET,
                );
            }

            Ok(())
        }
    }
}

impl RenderBackend for D3d12Renderer {
    fn name(&self) -> &'static str {
        D3D12_RENDERER
    }

    fn output_size(&self, _textures: &TextureStore) -> Option<Result<(i32, i32)>> {
        None // (the window's size in pixels)
    }

    fn texture_formats(&self) -> Option<Vec<PixelFormat>> {
        Some(self.texture_formats.clone())
    }

    /// `SDL_PROP_RENDERER_MAX_TEXTURE_SIZE_NUMBER`, set in
    /// `D3D12_CreateRenderer()`.
    fn max_texture_size(&self) -> Option<i32> {
        Some(16384)
    }

    fn set_properties(&mut self, props: &Properties) {
        self.props = Some(props.clone());
        self.publish_properties();
    }

    /// Translation of `D3D12_SupportsBlendMode()`.
    fn supports_blend_mode(&self, blend_mode: BlendMode) -> bool {
        let src_color_factor = blend_mode.src_color_factor();
        let src_alpha_factor = blend_mode.src_alpha_factor();
        let color_operation = blend_mode.color_operation();
        let dst_color_factor = blend_mode.dst_color_factor();
        let dst_alpha_factor = blend_mode.dst_alpha_factor();
        let alpha_operation = blend_mode.alpha_operation();

        !(get_blend_func(src_color_factor) == 0
            || get_blend_func(src_alpha_factor) == 0
            || get_blend_equation(color_operation) == 0
            || get_blend_func(dst_color_factor) == 0
            || get_blend_func(dst_alpha_factor) == 0
            || get_blend_equation(alpha_operation) == 0)
    }

    /// Translation of `D3D12_CreateTexture()`.
    fn create_texture(
        &mut self,
        texture: &mut TextureData,
        _props: &TextureCreateProps,
    ) -> Result<()> {
        use PixelFormat as F;
        let texture_format =
            sdl_pixel_format_to_dxgi_texture_format(texture.format, self.output_colorspace);

        if self.d3d_device.is_none() {
            return Err(device_lost());
        }

        if texture_format == DXGI_FORMAT_UNKNOWN {
            return Err(Error::new(format!(
                "D3D12_CreateTexture, An unsupported SDL pixel format (0x{:x}) was specified",
                texture.format.0
            )));
        }

        let mut texture_data = D3d12Texture {
            main_texture_format: texture_format,
            ..Default::default()
        };

        let mut texture_desc = ResourceDesc {
            width: texture.w as u64,
            height: texture.h as u32,
            mip_levels: 1,
            depth_or_array_size: 1,
            format: texture_format,
            sample_desc: SampleDesc {
                count: 1,
                quality: 0,
            },
            dimension: D3D12_RESOURCE_DIMENSION_TEXTURE2D,
            flags: D3D12_RESOURCE_FLAG_NONE,
            ..Default::default()
        };

        // NV12 textures must have even width and height
        if matches!(texture.format, F::NV12 | F::NV21 | F::P010) {
            texture_desc.width = (texture_desc.width + 1) & !1;
            texture_desc.height = (texture_desc.height + 1) & !1;
        }
        texture_data.w = texture_desc.width as i32;
        texture_data.h = texture_desc.height as i32;

        if texture.access == TextureAccess::Target {
            texture_desc.flags = D3D12_RESOURCE_FLAG_ALLOW_RENDER_TARGET;
        }

        if let Err(e) = self.create_texture_resources(texture, &mut texture_data, texture_desc) {
            // (SDL_CreateTexture() destroys the texture that failed, which
            // frees its SRV slots)
            // FIXME (upstream): D3D12_DestroyTexture() frees the slot of
            // mainSRVIndex even if it was never taken (index 0, then handed
            // out twice).
            self.free_texture_srvs(&texture_data);
            return Err(e);
        }

        let id = self.next_texture;
        self.next_texture += 1;
        self.textures.insert(id, texture_data);
        texture.internal = Some(Box::new(TextureRef {
            id,
            pixels: Vec::new(),
            staging: None,
        }));
        Ok(())
    }

    fn queue_set_viewport(&mut self, _cmd: &mut RenderCommand) -> Result<()> {
        Ok(()) // nothing to do in this backend.
    }

    fn queue_set_draw_color(&mut self, _cmd: &mut RenderCommand) -> Result<()> {
        Ok(()) // nothing to do in this backend.
    }

    /// Translation of `D3D12_QueueDrawPoints()`.
    fn queue_draw_points(&mut self, cmd: &mut DrawCmd, points: &[FPoint]) -> Result<()> {
        self.queue_points(cmd, points);
        Ok(())
    }

    /// `D3D12_QueueDrawPoints()`: lines and points queue vertices the same way.
    fn queue_draw_lines(&mut self, cmd: &mut DrawCmd, points: &[FPoint]) -> Option<Result<()>> {
        self.queue_points(cmd, points);
        Some(Ok(()))
    }

    fn has_queue_fill_rects(&self) -> bool {
        false
    }

    fn queue_fill_rects(&mut self, _cmd: &mut DrawCmd, _rects: &[FRect]) -> Result<()> {
        Err(Error::unsupported())
    }

    fn has_queue_copy(&self) -> bool {
        false
    }

    fn queue_copy(
        &mut self,
        _cmd: &mut DrawCmd,
        _texture: &TextureData,
        _srcrect: &FRect,
        _dstrect: &FRect,
    ) -> Result<()> {
        Err(Error::unsupported())
    }

    fn has_queue_copy_ex(&self) -> bool {
        false
    }

    fn queue_copy_ex(
        &mut self,
        _cmd: &mut DrawCmd,
        _texture: &TextureData,
        _copy: &CopyEx,
    ) -> Result<()> {
        Err(Error::unsupported())
    }

    fn has_queue_geometry(&self) -> bool {
        true
    }

    /// Translation of `D3D12_QueueGeometry()`.
    fn queue_geometry(
        &mut self,
        cmd: &mut DrawCmd,
        texture: Option<&TextureData>,
        geometry: &Geometry<'_>,
        scale_x: f32,
        scale_y: f32,
    ) -> Result<()> {
        let count = geometry.count();
        let convert_color = self.rendering_linear_space();
        let texture_data = texture
            .and_then(texture_ref)
            .and_then(|r| self.textures.get(&r.id));
        let (u_scale, v_scale) = match (texture, texture_data) {
            (Some(t), Some(d)) => (t.w as f32 / d.w as f32, t.h as f32 / d.h as f32),
            _ => (0.0, 0.0),
        };

        cmd.first = self.verts.len();
        cmd.count = count;

        for i in 0..count {
            let j = geometry.vertex(i);
            let (x, y) = geometry.xy(j);
            let mut color = geometry.color(j);
            if convert_color {
                self.convert_to_linear(&mut color);
            }

            let (u, v) = if texture.is_some() {
                let (u, v) = geometry.uv(j);
                (u * u_scale, v * v_scale)
            } else {
                (0.0, 0.0)
            };

            self.verts.push(VertexPositionColor::new(
                x * scale_x,
                y * scale_y,
                u,
                v,
                color,
            ));
        }
        Ok(())
    }

    /// Translation of `D3D12_InvalidateCachedState()`.
    fn invalidate_cached_state(&mut self) {
        self.current_render_target_view.ptr = 0;
        self.num_current_shader_resources = 0;
        self.current_shader_resource.ptr = 0;
        self.num_current_shader_samplers = 0;
        self.current_shader_sampler.ptr = 0;
        self.cliprect_dirty = true;
        self.viewport_dirty = true;
    }

    /// Translation of `D3D12_RunCommandQueue()`.
    fn run_command_queue(
        &mut self,
        cmds: &[RenderCommand],
        textures: &mut TextureStore,
        _gpu_render_states: &crate::render::sysrender::GpuRenderStates,
    ) -> Result<()> {
        let viewport_rotation = self.get_rotation_for_current_render_target();

        if self.d3d_device.is_none() {
            return Err(device_lost());
        }

        if self.pixel_size_changed {
            let _ = self.update_for_window_size_change();
            self.pixel_size_changed = false;
        }

        if self.current_viewport_rotation != viewport_rotation {
            self.current_viewport_rotation = viewport_rotation;
            self.viewport_dirty = true;
        }

        self.update_vertex_buffer()?;

        // (upstream draws whether or not the draw state could be set)
        let mut i = 0;
        while i < cmds.len() {
            match cmds[i] {
                RenderCommand::SetDrawColor { .. } => {
                    // this isn't currently used in this render backend.
                }

                RenderCommand::SetViewport { rect, .. } => {
                    if self.current_viewport != rect {
                        self.current_viewport = rect;
                        self.viewport_dirty = true;
                        self.cliprect_dirty = true;
                    }
                }

                RenderCommand::SetClipRect { enabled, rect } => {
                    let mut rect = rect;
                    if self.current_cliprect_enabled != enabled {
                        self.current_cliprect_enabled = enabled;
                        self.cliprect_dirty = true;
                    }
                    if !self.current_cliprect_enabled {
                        /* If the clip rect is disabled, then the scissor rect should be the whole viewport,
                        since direct3d12 doesn't allow disabling the scissor rectangle */
                        rect = Rect::new(0, 0, self.current_viewport.w, self.current_viewport.h);
                    }
                    if self.current_cliprect != rect {
                        self.current_cliprect = rect;
                        self.cliprect_dirty = true;
                    }
                }

                RenderCommand::Clear {
                    mut color,
                    color_scale,
                    ..
                } => {
                    let rtv_descriptor = self.get_current_render_target_view();
                    if self.rendering_linear_space() {
                        self.convert_to_linear(&mut color);
                    }

                    color.r *= color_scale;
                    color.g *= color_scale;
                    color.b *= color_scale;

                    if self.current_colorspace == Colorspace::HDR10 {
                        color.r = pq_from_nits(color.r * SCRGB_NITS);
                        color.g = pq_from_nits(color.g * SCRGB_NITS);
                        color.b = pq_from_nits(color.b * SCRGB_NITS);
                    }

                    if let Some(command_list) = &self.command_list {
                        command_list.clear_render_target_view(
                            rtv_descriptor,
                            &[color.r, color.g, color.b, color.a],
                        );
                    }
                }

                RenderCommand::Draw(DrawKind::Lines, d) if d.count > 0 => {
                    let mut count = d.count;
                    let start = d.first;
                    let mut have_point_draw_state = false;

                    // Add the final point in the line
                    let mut line_start = 0;
                    let mut line_end = line_start + count - 1;
                    let verts = &self.verts[start..];
                    if verts[line_start].pos != verts[line_end].pos {
                        let _ = self.set_draw_state(
                            &d,
                            None,
                            D3D12_PRIMITIVE_TOPOLOGY_TYPE_POINT,
                            &[],
                            &[],
                        );
                        self.draw_primitives(D3D_PRIMITIVE_TOPOLOGY_POINTLIST, start + line_end, 1);
                        have_point_draw_state = true;
                    }

                    if count > 2 {
                        // joined lines cannot be grouped
                        let _ = self.set_draw_state(
                            &d,
                            None,
                            D3D12_PRIMITIVE_TOPOLOGY_TYPE_LINE,
                            &[],
                            &[],
                        );
                        self.draw_primitives(D3D_PRIMITIVE_TOPOLOGY_LINESTRIP, start, count);
                    } else {
                        // let's group non joined lines
                        let mut finalcmd = i;
                        let thiscolorscale = d.color_scale;
                        let thisblend = d.blend;

                        for (j, next) in cmds.iter().enumerate().skip(i + 1) {
                            match next {
                                RenderCommand::Draw(DrawKind::Lines, n) => {
                                    if n.count != 2 {
                                        break; // can't go any further on this draw call, those are joined lines
                                    } else if n.blend != thisblend
                                        || n.color_scale != thiscolorscale
                                    {
                                        break; // can't go any further on this draw call, different blendmode copy up next.
                                    } else {
                                        finalcmd = j; // we can combine copy operations here. Mark this one as the furthest okay command.

                                        // Add the final point in the line
                                        line_start = count;
                                        line_end = line_start + n.count - 1;
                                        let verts = &self.verts[start..];
                                        if verts[line_start].pos != verts[line_end].pos {
                                            if !have_point_draw_state {
                                                let _ = self.set_draw_state(
                                                    &d,
                                                    None,
                                                    D3D12_PRIMITIVE_TOPOLOGY_TYPE_POINT,
                                                    &[],
                                                    &[],
                                                );
                                                have_point_draw_state = true;
                                            }
                                            self.draw_primitives(
                                                D3D_PRIMITIVE_TOPOLOGY_POINTLIST,
                                                start + line_end,
                                                1,
                                            );
                                        }
                                        count += n.count;
                                    }
                                }
                                RenderCommand::SetDrawColor { .. } => {
                                    // The vertex data has the draw color built in, ignore this
                                    continue;
                                }
                                _ => break, // can't go any further on this draw call, different render command up next.
                            }
                        }

                        let _ = self.set_draw_state(
                            &d,
                            None,
                            D3D12_PRIMITIVE_TOPOLOGY_TYPE_LINE,
                            &[],
                            &[],
                        );
                        self.draw_primitives(D3D_PRIMITIVE_TOPOLOGY_LINELIST, start, count);
                        i = finalcmd; // skip any copy commands we just combined in here.
                    }
                }

                RenderCommand::Draw(DrawKind::Lines, _) => {
                    // Note (upstream): without vertices, upstream reads the
                    // one before the command's first.
                }

                RenderCommand::Draw(DrawKind::FillRects | DrawKind::Copy | DrawKind::CopyEx, _) => {
                    // unused
                }

                RenderCommand::Draw(thiscmdtype @ (DrawKind::Points | DrawKind::Geometry), d) => {
                    /* as long as we have the same copy command in a row, with the
                    same texture, we can combine them all into a single draw call. */
                    let mut finalcmd = i;
                    let mut count = d.count;
                    let start = d.first;
                    for (j, next) in cmds.iter().enumerate().skip(i + 1) {
                        match next {
                            RenderCommand::Draw(nextcmdtype, n) if *nextcmdtype == thiscmdtype => {
                                if n.texture != d.texture
                                    || n.texture_scale_mode != d.texture_scale_mode
                                    || n.texture_address_mode_u != d.texture_address_mode_u
                                    || n.texture_address_mode_v != d.texture_address_mode_v
                                    || n.blend != d.blend
                                    || n.color_scale != d.color_scale
                                {
                                    break; // can't go any further on this draw call, different texture/blendmode copy up next.
                                } else {
                                    finalcmd = j; // we can combine copy operations here. Mark this one as the furthest okay command.
                                    count += n.count;
                                }
                            }
                            RenderCommand::SetDrawColor { .. } => {
                                // The vertex data has the draw color built in, ignore this
                                continue;
                            }
                            _ => break, // can't go any further on this draw call, different render command up next.
                        }
                    }

                    if thiscmdtype == DrawKind::Geometry {
                        if d.texture.is_some() {
                            let _ = self.set_copy_state(&d, textures);
                        } else {
                            let _ = self.set_draw_state(
                                &d,
                                None,
                                D3D12_PRIMITIVE_TOPOLOGY_TYPE_TRIANGLE,
                                &[],
                                &[],
                            );
                        }
                        self.draw_primitives(D3D_PRIMITIVE_TOPOLOGY_TRIANGLELIST, start, count);
                    } else {
                        let _ = self.set_draw_state(
                            &d,
                            None,
                            D3D12_PRIMITIVE_TOPOLOGY_TYPE_POINT,
                            &[],
                            &[],
                        );
                        self.draw_primitives(D3D_PRIMITIVE_TOPOLOGY_POINTLIST, start, count);
                    }
                    i = finalcmd; // skip any copy commands we just combined in here.
                }

                RenderCommand::NoOp => {}
            }

            i += 1;
        }

        Ok(())
    }

    fn reset_vertices(&mut self) {
        self.verts.clear();
    }

    /// Translation of `D3D12_CreatePalette()`.
    fn create_palette(&mut self) -> Result<Box<dyn Any>> {
        let id = self.next_palette;
        self.next_palette += 1;
        self.palettes.insert(
            id,
            PaletteData {
                texture: None,
                resource_view: CpuDescriptorHandle::default(),
                resource_state: 0,
                srv_index: None,
            },
        );
        let palette = Box::new(PaletteRef(id));

        // (the palette's data stays the palette's whether or not this fails,
        // as upstream's)
        let device = self.device()?.clone();

        let texture_desc = ResourceDesc {
            width: 256,
            height: 1,
            mip_levels: 1,
            depth_or_array_size: 1,
            format: sdl_pixel_format_to_dxgi_texture_format(
                PixelFormat::RGBA32,
                self.output_colorspace,
            ),
            sample_desc: SampleDesc {
                count: 1,
                quality: 0,
            },
            dimension: D3D12_RESOURCE_DIMENSION_TEXTURE2D,
            flags: D3D12_RESOURCE_FLAG_NONE,
            ..Default::default()
        };

        let heap_props = heap_properties(D3D12_HEAP_TYPE_DEFAULT);

        let texture = device
            .create_committed_resource(
                &heap_props,
                D3D12_HEAP_FLAG_NONE,
                &texture_desc,
                D3D12_RESOURCE_STATE_COPY_DEST,
                None,
            )
            .map_err(|hr| hr_error("ID3D12Device::CreateCommittedResource [texture]", hr))?;
        if let Some(palettedata) = self.palettes.get_mut(&id) {
            palettedata.texture = Some(texture.clone());
            palettedata.resource_state = D3D12_RESOURCE_STATE_COPY_DEST;
        }

        let resource_view_desc = texture_2d_srv_desc(
            sdl_pixel_format_to_dxgi_main_resource_view_format(
                PixelFormat::RGBA32,
                self.output_colorspace,
            ),
            texture_desc.mip_levels as u32,
            0,
        );

        let (index, view) = self.create_srv(&texture, &resource_view_desc)?;
        if let Some(palettedata) = self.palettes.get_mut(&id) {
            palettedata.srv_index = Some(index);
            palettedata.resource_view = view;
        }

        Ok(palette)
    }

    /// Translation of `D3D12_UpdatePalette()`.
    fn update_palette(&mut self, palette: &mut dyn Any, colors: &[Color]) -> Result<()> {
        let id = palette
            .downcast_ref::<PaletteRef>()
            .map(|p| p.0)
            .ok_or_else(|| Error::invalid_param("palette"))?;
        let palettedata = self
            .palettes
            .get(&id)
            .ok_or_else(|| Error::invalid_param("palette"))?;
        let texture = palettedata.texture.clone().ok_or_else(device_lost)?;
        let mut state = palettedata.resource_state;

        let bytes: Vec<u8> = colors.iter().flat_map(|c| [c.r, c.g, c.b, c.a]).collect();
        let result = self.update_texture_internal(
            &texture,
            0,
            (0, 0, colors.len() as i32, 1),
            &bytes,
            bytes.len() as i32,
            &mut state,
        );
        if let Some(palettedata) = self.palettes.get_mut(&id) {
            palettedata.resource_state = state;
        }
        result
    }

    /// Translation of `D3D12_DestroyPalette()`.
    fn destroy_palette(&mut self, palette: Box<dyn Any>) {
        let Ok(palette) = palette.downcast::<PaletteRef>() else {
            return;
        };
        if let Some(palettedata) = self.palettes.remove(&palette.0) {
            // FIXME (upstream): the palette's texture is released without
            // the batch being issued first (as D3D12_DestroyTexture() does),
            // while a recorded draw may still refer to it.
            if let Some(index) = palettedata.srv_index {
                self.free_srv_index(index);
            }
        }
    }

    /// Record the texture's palette, which the draws bind
    /// (`texture->palette->internal`).
    fn change_texture_palette(
        &mut self,
        texture: &mut TextureData,
        palette: Option<&dyn Any>,
    ) -> Option<Result<()>> {
        let palette = palette
            .and_then(|p| p.downcast_ref::<PaletteRef>())
            .map(|p| p.0);
        if let Some(t) = texture_ref(texture).and_then(|r| self.textures.get_mut(&r.id)) {
            t.palette = palette;
        }
        Some(Ok(()))
    }

    fn update_texture(
        &mut self,
        texture: &mut TextureData,
        rect: &Rect,
        pixels: &[u8],
        pitch: usize,
    ) -> Result<()> {
        self.update_texture_pixels(texture, rect, pixels, pitch as i32)
    }

    fn update_texture_yuv(
        &mut self,
        texture: &mut TextureData,
        rect: &Rect,
        (y, y_pitch): (&[u8], usize),
        (u, u_pitch): (&[u8], usize),
        (v, v_pitch): (&[u8], usize),
    ) -> Option<Result<()>> {
        Some(self.update_texture_yuv_planes(
            texture,
            rect,
            (y, y_pitch as i32),
            (u, u_pitch as i32),
            (v, v_pitch as i32),
        ))
    }

    fn update_texture_nv(
        &mut self,
        texture: &mut TextureData,
        rect: &Rect,
        (y, y_pitch): (&[u8], usize),
        (uv, uv_pitch): (&[u8], usize),
    ) -> Option<Result<()>> {
        Some(self.update_texture_nv_planes(
            texture,
            rect,
            (y, y_pitch as i32),
            (uv, uv_pitch as i32),
        ))
    }

    /// Translation of `D3D12_LockTexture()`.
    fn lock_texture(&mut self, texture: &mut TextureData, rect: &Rect) -> Result<(usize, i32)> {
        let id = Self::texture_id(texture)?;
        let texture_data = self.textures.get(&id).ok_or_else(not_available)?;

        if texture_data.yuv || texture_data.nv12 {
            // It's more efficient to upload directly...
            let bpp = texture.format.bytes_per_pixel() as usize;
            let (format, w, h) = (texture.format, texture.w, texture.h);
            let tref = texture_ref_mut(texture).ok_or_else(not_available)?;
            let texture_data = self.textures.get_mut(&id).ok_or_else(not_available)?;
            if tref.pixels.is_empty() {
                let (size, calculated_pitch) =
                    crate::video::surface::calculate_yuv_size(format, w, h)?;
                texture_data.pitch = calculated_pitch as i32;
                tref.pixels = vec![0; size];
            }
            texture_data.locked_rect = *rect;
            let offset = rect.y as usize * texture_data.pitch as usize + rect.x as usize * bpp;
            return Ok((offset, texture_data.pitch));
        }
        if texture_data.staging_buffer.is_some() {
            return Err(Error::new("texture is already locked"));
        }
        let main_texture = texture_data
            .main_texture
            .as_ref()
            .ok_or_else(not_available)?;
        let device = self.device()?;

        // Create an upload buffer, which will be used to write to the main texture.
        let mut texture_desc = main_texture.desc();
        texture_desc.width = rect.w as u64;
        texture_desc.height = rect.h as u32;

        // Figure out how much we need to allocate for the upload buffer
        let (_, _, _, upload_size) = device.copyable_footprints(&texture_desc, 0, 0);
        let upload_desc = buffer_desc(upload_size);

        let heap_props = heap_properties(D3D12_HEAP_TYPE_UPLOAD);

        // Create the upload buffer
        let staging_buffer = device
            .create_committed_resource(
                &heap_props,
                D3D12_HEAP_FLAG_NONE,
                &upload_desc,
                D3D12_RESOURCE_STATE_GENERIC_READ,
                None,
            )
            .map_err(|hr| {
                hr_error(
                    "ID3D12Device::CreateCommittedResource [create upload buffer]",
                    hr,
                )
            })?;
        let texture_data = self.textures.get_mut(&id).ok_or_else(not_available)?;
        texture_data.staging_buffer = Some(staging_buffer.clone());

        // Get a write-only pointer to data in the upload buffer:
        let texture_memory = staging_buffer.map().map_err(|hr| {
            // FIXME (upstream): this releases the next upload buffer (none)
            // instead of the staging buffer, so the texture stays locked.
            hr_error("ID3D12Resource::Map [map staging texture]", hr)
        })?;

        let bpp = if texture_desc.format == DXGI_FORMAT_R8_UNORM {
            1
        } else {
            texture.format.bytes_per_pixel()
        };
        let row_pitch = d3d12_align(rect.w as u32 * bpp, D3D12_TEXTURE_DATA_PITCH_ALIGNMENT);

        /* Make note of where the staging texture will be written to
         * (on a call to SDL_UnlockTexture):
         */
        texture_data.locked_rect = *rect;

        /* Make sure the caller has information on the texture's pixel buffer,
         * then return:
         */
        let staging = NonNull::new(texture_memory).map(|p| (p, upload_size as usize));
        if let Some(tref) = texture_ref_mut(texture) {
            tref.staging = staging;
        }
        Ok((0, row_pitch as i32))
    }

    fn texture_pixels_mut<'t>(&mut self, texture: &'t mut TextureData) -> Option<&'t mut [u8]> {
        let tref = texture_ref_mut(texture)?;
        match tref.staging {
            // SAFETY: the mapped staging buffer of the lock, which stays
            // mapped until the texture is unlocked or destroyed; both take
            // the texture mutably, which the returned borrow prevents.
            Some((ptr, len)) => Some(unsafe { std::slice::from_raw_parts_mut(ptr.as_ptr(), len) }),
            None => Some(&mut tref.pixels[..]),
        }
    }

    /// Translation of `D3D12_UnlockTexture()`.
    fn unlock_texture(&mut self, texture: &mut TextureData) {
        let Ok(id) = Self::texture_id(texture) else {
            return;
        };
        let Some(texture_data) = self.textures.get(&id) else {
            return;
        };
        if texture_data.yuv || texture_data.nv12 {
            let rect = texture_data.locked_rect;
            let pitch = texture_data.pitch;
            let offset = rect.y as usize * pitch as usize
                + rect.x as usize * texture.format.bytes_per_pixel() as usize;
            let Some(tref) = texture_ref_mut(texture) else {
                return;
            };
            let pixels = std::mem::take(&mut tref.pixels);
            if let Some(locked) = pixels.get(offset..) {
                let _ = self.update_texture_pixels(texture, &rect, locked, pitch);
            }
            if let Some(tref) = texture_ref_mut(texture) {
                tref.pixels = pixels;
            }
            return;
        }
        if let Some(tref) = texture_ref_mut(texture) {
            tref.staging = None;
        }
        let Some(texture_data) = self.textures.get_mut(&id) else {
            return;
        };
        let (Some(staging_buffer), Some(main_texture)) = (
            texture_data.staging_buffer.clone(),
            texture_data.main_texture.clone(),
        ) else {
            return;
        };
        let locked_rect = texture_data.locked_rect;
        let main_resource_state = texture_data.main_resource_state;

        // Commit the pixel buffer's changes back to the staging texture:
        staging_buffer.unmap();

        let mut texture_desc = main_texture.desc();
        texture_desc.width = locked_rect.w as u64;
        texture_desc.height = locked_rect.h as u32;

        let bpp = if texture_desc.format == DXGI_FORMAT_R8_UNORM {
            1
        } else {
            texture.format.bytes_per_pixel()
        };
        let pitched_desc = SubresourceFootprint {
            format: texture_desc.format,
            width: texture_desc.width as u32,
            height: texture_desc.height,
            depth: 1,
            row_pitch: d3d12_align(
                locked_rect.w as u32 * bpp,
                D3D12_TEXTURE_DATA_PITCH_ALIGNMENT,
            ),
        };

        let placed_texture_desc = PlacedSubresourceFootprint {
            offset: 0,
            footprint: pitched_desc,
        };

        self.transition_resource(
            &main_texture,
            main_resource_state,
            D3D12_RESOURCE_STATE_COPY_DEST,
        );

        let dst_location = subresource_location(&main_texture, 0);
        let src_location = footprint_location(&staging_buffer, placed_texture_desc);

        if let Some(command_list) = &self.command_list {
            command_list.copy_texture_region(
                &dst_location,
                locked_rect.x as u32,
                locked_rect.y as u32,
                0,
                &src_location,
                None,
            );
        }

        // Transition the texture to be shader accessible
        self.transition_resource(
            &main_texture,
            D3D12_RESOURCE_STATE_COPY_DEST,
            D3D12_RESOURCE_STATE_PIXEL_SHADER_RESOURCE,
        );
        if let Some(texture_data) = self.textures.get_mut(&id) {
            texture_data.main_resource_state = D3D12_RESOURCE_STATE_PIXEL_SHADER_RESOURCE;
        }

        // Execute the command list before releasing the staging buffer
        let _ = self.issue_batch();
        if let Some(texture_data) = self.textures.get_mut(&id) {
            texture_data.staging_buffer = None;
        }
    }

    /// Translation of `D3D12_SetRenderTarget()`.
    fn set_render_target(
        &mut self,
        target: Option<Texture>,
        textures: &TextureStore,
    ) -> Result<()> {
        let Some(t) = target else {
            if let Some(id) = self.texture_render_target {
                if let Some((resource, state)) =
                    self.textures.get(&id).and_then(|t| t.plane(Plane::Main))
                {
                    self.transition_resource(
                        &resource,
                        state,
                        D3D12_RESOURCE_STATE_PIXEL_SHADER_RESOURCE,
                    );
                }
                if let Some(texture_data) = self.textures.get_mut(&id) {
                    texture_data.main_resource_state = D3D12_RESOURCE_STATE_PIXEL_SHADER_RESOURCE;
                }
            }
            self.texture_render_target = None;
            self.target = None;
            self.current_colorspace = self.output_colorspace;
            return Ok(());
        };

        let texture = textures.get(t).ok_or_else(invalid_texture)?;
        // Note (upstream): a texture whose data went with a lost device is
        // dereferenced there.
        let id = Self::texture_id(texture)?;
        let texture_data = self.textures.get(&id).ok_or_else(not_available)?;

        if texture_data.main_texture_render_target_view.ptr == 0 {
            return Err(Error::new("specified texture is not a render target"));
        }

        self.texture_render_target = Some(id);
        if let Some((resource, state)) = texture_data.plane(Plane::Main) {
            self.transition_resource(&resource, state, D3D12_RESOURCE_STATE_RENDER_TARGET);
        }
        if let Some(texture_data) = self.textures.get_mut(&id) {
            texture_data.main_resource_state = D3D12_RESOURCE_STATE_RENDER_TARGET;
        }
        self.target = Some(t);
        self.current_colorspace = texture.colorspace;

        Ok(())
    }

    /// Translation of `D3D12_RenderReadPixels()`.
    fn read_pixels(
        &mut self,
        rect: &Rect,
        _textures: &mut TextureStore,
    ) -> Option<Result<Surface<'static>>> {
        Some(self.render_read_pixels(rect))
    }

    /// Translation of `D3D12_RenderPresent()`.
    fn present(&mut self) -> bool {
        // (SDL_RenderPresent() goes on without the error)
        self.render_present().is_ok()
    }

    /// Translation of `D3D12_DestroyTexture()`.
    fn destroy_texture(&mut self, texture: &mut TextureData) {
        let Some(internal) = texture.internal.take() else {
            return;
        };
        let Ok(tref) = internal.downcast::<TextureRef>() else {
            return;
        };
        self.destroy_texture_data(tref.id);
    }

    /// Translation of `D3D12_SetVSync()`.
    fn set_vsync(&mut self, vsync: i32) -> Option<Result<()>> {
        if vsync < 0 {
            return Some(Err(Error::unsupported()));
        }

        if vsync > 0 {
            self.sync_interval = vsync as u32;
            self.present_flags = 0;
        } else {
            self.sync_interval = 0;
            self.present_flags = DXGI_PRESENT_ALLOW_TEARING;
        }
        Some(Ok(()))
    }

    /// Translation of `D3D12_WindowEvent()`.
    fn window_event(&mut self, event_type: EventType) {
        if event_type == EventType::WINDOW_PIXEL_SIZE_CHANGED {
            self.pixel_size_changed = true;
        }
    }

    /// Translation of `D3D12_DestroyRenderer()`.
    fn destroy(&mut self) {
        self.wait_for_gpu();
        // (the front end destroys the palettes first; any left go before
        // the libraries are unloaded)
        let ids: Vec<u32> = self.palettes.keys().copied().collect();
        for id in ids {
            self.destroy_palette(Box::new(PaletteRef(id)));
        }
        self.release_all();
    }
}
