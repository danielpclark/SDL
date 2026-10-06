// Rust translation of src/render/direct3d11/SDL_render_d3d11.c and the
// creation functions of SDL_shaders_d3d11.c from Simple DirectMedia Layer,
// with the parts of src/render/SDL_d3dmath.h they use.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! The Direct3D 11 renderer ("direct3d11", the first one tried on
//! Windows): draws through a Direct3D 11.1 device and a DXGI swap chain on
//! the window's `HWND`. `dxgi.dll` and `d3d11.dll` are loaded at run time,
//! as upstream does, and the COM interfaces are declared in [`d3d`].
//!
//! What isn't translated:
//!
//! * the creation options for existing textures
//!   (`SDL_PROP_TEXTURE_CREATE_D3D11_TEXTURE_*`), which the front end
//!   doesn't take yet: the renderer makes all its textures.
//! * the `CoreWindow` swap chain: upstream's `coreWindow` is always NULL,
//!   so only the `HWND` branch is.
//!
//! The front end only makes renderers with sRGB output yet, so the linear
//! and HDR10 output paths here are kept for when it does.
//!
//! Upstream keeps the per-texture data in `texture->internal` and walks the
//! renderer's texture list to free it when the device is lost. Here the
//! renderer keeps its textures (and palettes) itself, and the front end's
//! texture refers to them by a key. The COM references are [`ComPtr`]s,
//! released when dropped (upstream's `SAFE_RELEASE()`).
//!
//! [`ComPtr`]: crate::core::windows::com::ComPtr

pub(crate) mod d3d;
mod shaders;
#[cfg(test)]
mod tests;

use std::any::Any;
use std::collections::HashMap;
use std::ffi::c_void;
use std::ptr::{null_mut, NonNull};

use windows_sys::core::HRESULT;
use windows_sys::Win32::Foundation::HWND;

use d3d::*;
use shaders::Shader;

use crate::core::windows::com::{ComPtr, IUnknownVtbl};
use crate::core::windows::{error_from_hresult, is_windows_8_or_greater};
use crate::error::{Error, Result};
use crate::events::window::WindowFlags;
use crate::events::{Event, EventType, RenderEvent};
use crate::hints;
use crate::loadso::SharedObject;
use crate::log::Category;
use crate::properties::Properties;
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

/// The name of the Direct3D 11 renderer (`D3D11_RenderDriver.name`).
pub(crate) const D3D11_RENDERER: &str = "direct3d11";

/// The `ID3D11Device` of the renderer. Translation of
/// `SDL_PROP_RENDERER_D3D11_DEVICE_POINTER`.
pub const PROP_RENDERER_D3D11_DEVICE_POINTER: &str = "SDL.renderer.d3d11.device";
/// The `IDXGISwapChain1` of the renderer. Translation of
/// `SDL_PROP_RENDERER_D3D11_SWAPCHAIN_POINTER`.
pub const PROP_RENDERER_D3D11_SWAPCHAIN_POINTER: &str = "SDL.renderer.d3d11.swap_chain";
/// The `ID3D11Texture2D` of a texture (its Y plane for YUV textures).
/// Translation of `SDL_PROP_TEXTURE_D3D11_TEXTURE_POINTER`.
pub const PROP_TEXTURE_D3D11_TEXTURE_POINTER: &str = "SDL.texture.d3d11.texture";
/// The `ID3D11Texture2D` of the U plane of a YUV texture. Translation of
/// `SDL_PROP_TEXTURE_D3D11_TEXTURE_U_POINTER`.
pub const PROP_TEXTURE_D3D11_TEXTURE_U_POINTER: &str = "SDL.texture.d3d11.texture_u";
/// The `ID3D11Texture2D` of the V plane of a YUV texture. Translation of
/// `SDL_PROP_TEXTURE_D3D11_TEXTURE_V_POINTER`.
pub const PROP_TEXTURE_D3D11_TEXTURE_V_POINTER: &str = "SDL.texture.d3d11.texture_v";

/// The SDR white level of scRGB content (`SCRGB_NITS`).
const SCRGB_NITS: f32 = 80.0;

/// The number of vertex buffers the frames cycle through
/// (`vertexBuffers[8]`).
const NUM_VERTEX_BUFFERS: usize = 8;

/// Translation of `RENDER_SAMPLER_COUNT` (from `SDL_sysrender.h`).
const RENDER_SAMPLER_COUNT: usize = ((1 << 0) | (1 << 1) | (1 << 2)) + 1;

/// Translation of `RENDER_SAMPLER_HASHKEY()` (from `SDL_sysrender.h`).
fn render_sampler_hashkey(
    scale_mode: ScaleMode,
    address_u: TextureAddressMode,
    address_v: TextureAddressMode,
) -> usize {
    ((scale_mode == ScaleMode::Nearest) as usize)
        | (((address_u == TextureAddressMode::Wrap) as usize) << 1)
        | (((address_v == TextureAddressMode::Wrap) as usize) << 2)
}

/// Translation of `WIN_SetErrorFromHRESULT()` for this renderer's calls.
fn hr_error(prefix: &str, hr: HRESULT) -> Error {
    error_from_hresult(Some(prefix), hr)
}

/// The error of a renderer whose device was lost and not recovered.
fn device_lost() -> Error {
    Error::new("Device lost and couldn't be recovered")
}

/// The error of a texture without renderer data.
fn not_available() -> Error {
    Error::new("Texture is not currently available")
}

/// The bytes of `pixels` from `offset` on (an error past the end: the C
/// code trusts the caller to pass all the planes).
fn plane(pixels: &[u8], offset: usize) -> Result<&[u8]> {
    pixels
        .get(offset..)
        .ok_or_else(|| Error::invalid_param("pixels"))
}

/// A 4x4 matrix of floats. Translation of `Float4X4` (from `SDL_d3dmath.h`).
#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
struct Float4X4 {
    m: [[f32; 4]; 4],
}

impl Float4X4 {
    /// Translation of `MatrixIdentity()`.
    fn identity() -> Float4X4 {
        let mut m = Float4X4::default();
        for i in 0..4 {
            m.m[i][i] = 1.0;
        }
        m
    }

    /// Translation of `MatrixMultiply()`.
    fn multiply(m1: &Float4X4, m2: &Float4X4) -> Float4X4 {
        let mut m = Float4X4::default();
        for i in 0..4 {
            for j in 0..4 {
                m.m[i][j] = m1.m[i][0] * m2.m[0][j]
                    + m1.m[i][1] * m2.m[1][j]
                    + m1.m[i][2] * m2.m[2][j]
                    + m1.m[i][3] * m2.m[3][j];
            }
        }
        m
    }

    /// Translation of `MatrixRotationZ()`.
    fn rotation_z(r: f32) -> Float4X4 {
        let sin_r = r.sin();
        let cos_r = r.cos();
        let mut m = Float4X4::default();
        m.m[0][0] = cos_r;
        m.m[0][1] = sin_r;
        m.m[1][0] = -sin_r;
        m.m[1][1] = cos_r;
        m.m[2][2] = 1.0;
        m.m[3][3] = 1.0;
        m
    }

    /// Whether two matrices are the same bits (`SDL_memcmp()`).
    fn same_bits(&self, other: &Float4X4) -> bool {
        self.m
            .as_flattened()
            .iter()
            .zip(other.m.as_flattened())
            .all(|(a, b)| a.to_bits() == b.to_bits())
    }
}

/// Vertex shader, common values. Translation of
/// `D3D11_VertexShaderConstants`.
#[repr(C)]
#[derive(Clone, Copy, Default)]
struct VertexShaderConstants {
    model: Float4X4,
    projection_and_view: Float4X4,
}

// These should mirror the definitions in D3D11_PixelShader_Common.hlsli
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

/// Pixel shader constants. Translation of `D3D11_PixelShaderConstants`.
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
    /// The constants as the words the shader reads.
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

    /// Whether two sets of constants are the same bits (`SDL_memcmp()`).
    fn same_bits(&self, other: &PixelShaderConstants) -> bool {
        self.words() == other.words()
    }
}

/// The constant buffer of a pixel shader and the constants it holds.
/// Translation of `D3D11_PixelShaderState`.
#[derive(Default)]
struct PixelShaderState {
    constants: Option<Buffer>,
    shader_constants: PixelShaderConstants,
}

/// Per-vertex data. Translation of `D3D11_VertexPositionColor`.
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

/// Per-palette data. Translation of `D3D11_PaletteData`.
struct PaletteData {
    texture: Texture2d,
    resource_view: ShaderResourceView,
}

/// The front end's handle of a palette: its key in the renderer.
struct PaletteRef(u32);

/// Per-texture data. Translation of `D3D11_TextureData`.
#[derive(Default)]
struct D3d11Texture {
    w: i32,
    h: i32,
    main_texture: Option<Texture2d>,
    main_texture_resource_view: Option<ShaderResourceView>,
    main_texture_render_target_view: Option<RenderTargetView>,
    staging_texture: Option<Texture2d>,
    locked_texture_position_x: i32,
    locked_texture_position_y: i32,
    ycbcr_matrix: Option<[f32; 16]>,
    // YV12 texture support
    yuv: bool,
    main_texture_u: Option<Texture2d>,
    main_texture_resource_view_u: Option<ShaderResourceView>,
    main_texture_v: Option<Texture2d>,
    main_texture_resource_view_v: Option<ShaderResourceView>,

    // NV12 texture support
    nv12: bool,
    main_texture_resource_view_nv: Option<ShaderResourceView>,

    pitch: i32,
    locked_rect: Rect,

    /// The key of the texture's palette (`texture->palette->internal`),
    /// as the front end last changed it.
    palette: Option<u32>,
}

/// What the front end's texture holds (`texture->internal`): the key of
/// the texture in the renderer, and the pixels its lock writes.
struct TextureRef {
    id: u32,
    /// The pixels of a YUV texture (`textureData->pixels`), kept from its
    /// first lock on.
    pixels: Vec<u8>,
    /// The mapped staging texture of a locked texture.
    staging: Option<(NonNull<u8>, usize)>,
}

fn texture_ref(texture: &TextureData) -> Option<&TextureRef> {
    texture.internal.as_ref()?.downcast_ref::<TextureRef>()
}

fn texture_ref_mut(texture: &mut TextureData) -> Option<&mut TextureRef> {
    texture.internal.as_mut()?.downcast_mut::<TextureRef>()
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

/// The renderer's data. Translation of `D3D11_RenderData`, with the parts
/// of `SDL_Renderer` the backend reads (`window`, `output_colorspace`,
/// `current_colorspace`, the HDR headroom, `target`), its textures and
/// palettes, and its vertex storage.
///
/// The cached state upstream compares by pointer (`currentRenderTargetView`
/// and the like) is kept as the raw interface pointers, for comparison
/// only. The libraries come last, so that they are unloaded after every
/// interface is released.
pub(crate) struct D3d11Renderer {
    window: Window,
    output_colorspace: Colorspace,
    current_colorspace: Colorspace,
    /// `renderer->HDR_headroom`: 1 for the sRGB output the front end makes.
    hdr_headroom: f32,
    /// `renderer->target`, the front end's texture.
    target: Option<Texture>,

    dxgi_factory: Option<DxgiFactory2>,
    dxgi_debug: Option<DxgiDebug>,
    d3d_device: Option<Device>,
    d3d_context: Option<DeviceContext>,
    swap_chain: Option<DxgiSwapChain1>,
    swap_effect: u32,
    swap_chain_flags: u32,
    sync_interval: u32,
    present_flags: u32,
    main_render_target_view: Option<RenderTargetView>,
    /// The key of the target texture, whose view is
    /// `currentOffscreenRenderTargetView`.
    current_offscreen_render_target: Option<u32>,
    input_layout: Option<InputLayout>,
    vertex_buffers: [Option<Buffer>; NUM_VERTEX_BUFFERS],
    vertex_buffer_sizes: [usize; NUM_VERTEX_BUFFERS],
    vertex_shader: Option<VertexShader>,
    pixel_shaders: [Option<PixelShader>; Shader::COUNT],
    blend_modes: Vec<(BlendMode, BlendState)>,
    samplers: [Option<SamplerState>; RENDER_SAMPLER_COUNT],
    #[cfg_attr(not(test), allow(dead_code))] // (kept, as upstream does)
    feature_level: D3dFeatureLevel,
    pixel_size_changed: bool,

    // Rasterizers
    main_rasterizer: Option<RasterizerState>,
    clipped_rasterizer: Option<RasterizerState>,

    // Vertex buffer constants
    vertex_shader_constants_data: VertexShaderConstants,
    vertex_shader_constants: Option<Buffer>,

    // Cached renderer properties
    rotation: DxgiModeRotation,
    current_render_target_view: *mut c_void,
    current_rasterizer_state: *mut c_void,
    current_blend_state: *mut c_void,
    current_shader: Option<Shader>,
    current_shader_state: [PixelShaderState; Shader::COUNT],
    num_current_shader_resources: usize,
    current_shader_resource: *mut c_void,
    num_current_shader_samplers: usize,
    current_shader_sampler: *mut c_void,
    cliprect_dirty: bool,
    current_cliprect_enabled: bool,
    current_cliprect: Rect,
    current_viewport: Rect,
    current_viewport_rotation: DxgiModeRotation,
    viewport_dirty: bool,
    identity: Float4X4,
    current_vertex_buffer: usize,

    /// The textures, by the key their [`TextureRef`] holds.
    textures: HashMap<u32, D3d11Texture>,
    next_texture: u32,
    /// The palettes, by the key their [`PaletteRef`] holds.
    palettes: HashMap<u32, PaletteData>,
    next_palette: u32,
    /// The vertex data of the queued commands (`first` indexes it).
    verts: Vec<VertexPositionColor>,
    texture_formats: Vec<PixelFormat>,
    /// The renderer's properties, once the front end has made them.
    props: Option<Properties>,

    h_d3d11_mod: Option<SharedObject>,
    h_dxgi_mod: Option<SharedObject>,
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

/// Translation of `D3D11_DXGIFormatToSDLPixelFormat()`.
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

/// Translation of `GetBlendFunc()` (0 for no factor).
fn get_blend_func(factor: Option<BlendFactor>) -> D3d11Blend {
    match factor {
        Some(BlendFactor::Zero) => D3D11_BLEND_ZERO,
        Some(BlendFactor::One) => D3D11_BLEND_ONE,
        Some(BlendFactor::SrcColor) => D3D11_BLEND_SRC_COLOR,
        Some(BlendFactor::OneMinusSrcColor) => D3D11_BLEND_INV_SRC_COLOR,
        Some(BlendFactor::SrcAlpha) => D3D11_BLEND_SRC_ALPHA,
        Some(BlendFactor::OneMinusSrcAlpha) => D3D11_BLEND_INV_SRC_ALPHA,
        Some(BlendFactor::DstColor) => D3D11_BLEND_DEST_COLOR,
        Some(BlendFactor::OneMinusDstColor) => D3D11_BLEND_INV_DEST_COLOR,
        Some(BlendFactor::DstAlpha) => D3D11_BLEND_DEST_ALPHA,
        Some(BlendFactor::OneMinusDstAlpha) => D3D11_BLEND_INV_DEST_ALPHA,
        None => 0,
    }
}

/// Translation of `GetBlendEquation()` (0 for no operation).
fn get_blend_equation(operation: Option<BlendOperation>) -> D3d11BlendOp {
    match operation {
        Some(BlendOperation::Add) => D3D11_BLEND_OP_ADD,
        Some(BlendOperation::Subtract) => D3D11_BLEND_OP_SUBTRACT,
        Some(BlendOperation::RevSubtract) => D3D11_BLEND_OP_REV_SUBTRACT,
        Some(BlendOperation::Minimum) => D3D11_BLEND_OP_MIN,
        Some(BlendOperation::Maximum) => D3D11_BLEND_OP_MAX,
        None => 0,
    }
}

/// The blend description of `D3D11_CreateBlendState()`.
fn blend_desc(blend_mode: BlendMode) -> BlendDesc {
    let src_color_factor = blend_mode.src_color_factor();
    let src_alpha_factor = blend_mode.src_alpha_factor();
    let color_operation = blend_mode.color_operation();
    let dst_color_factor = blend_mode.dst_color_factor();
    let dst_alpha_factor = blend_mode.dst_alpha_factor();
    let alpha_operation = blend_mode.alpha_operation();

    let mut desc = BlendDesc {
        alpha_to_coverage_enable: 0,
        independent_blend_enable: 0,
        ..Default::default()
    };
    desc.render_target[0] = RenderTargetBlendDesc {
        blend_enable: 1,
        src_blend: get_blend_func(src_color_factor),
        dest_blend: get_blend_func(dst_color_factor),
        blend_op: get_blend_equation(color_operation),
        src_blend_alpha: get_blend_func(src_alpha_factor),
        dest_blend_alpha: get_blend_func(dst_alpha_factor),
        blend_op_alpha: get_blend_equation(alpha_operation),
        render_target_write_mask: D3D11_COLOR_WRITE_ENABLE_ALL,
    };
    desc
}

/// Translation of `D3D11_GetCurrentRotation()`.
fn get_current_rotation() -> DxgiModeRotation {
    // FIXME
    DXGI_MODE_ROTATION_IDENTITY
}

/// Translation of `D3D11_IsDisplayRotated90Degrees()`.
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

/// The 16 floats of `SDL_GetYCbCRtoRGBConversionMatrix()`: the offsets,
/// then the R, G and B coefficients, each in a row of four.
fn ycbcr_matrix(texture: &TextureData, bits_per_pixel: u32) -> Option<[f32; 16]> {
    let matrix = texture
        .colorspace
        .ycbcr_to_rgb_matrix(texture.h.max(0) as u32, bits_per_pixel)?;
    let mut ycbcr = [0.0; 16];
    ycbcr[..3].copy_from_slice(&matrix.offset);
    for (row, coeff) in matrix.coeff.iter().enumerate() {
        ycbcr[4 + 4 * row..7 + 4 * row].copy_from_slice(coeff);
    }
    Some(ycbcr)
}

/// Copy `rows` rows of `length` bytes from `src` (rows `src_pitch` apart)
/// to mapped memory at `dst` (rows `dst_pitch` apart): the `SDL_memcpy()`
/// loops of the upload functions. An error, before anything is written,
/// when `src` is too short.
///
/// # Safety
///
/// `dst` must be writable for `rows` rows of `length` bytes,
/// `dst_pitch` apart.
unsafe fn copy_rows(
    dst: *mut u8,
    dst_pitch: usize,
    src: &[u8],
    src_pitch: usize,
    length: usize,
    rows: usize,
) -> Result<()> {
    if rows > 0 && src.len() < (rows - 1) * src_pitch + length {
        return Err(Error::invalid_param("pixels"));
    }
    for row in 0..rows {
        let s = &src[row * src_pitch..][..length];
        // SAFETY: the caller's contract; `s` is `length` bytes.
        unsafe { std::ptr::copy_nonoverlapping(s.as_ptr(), dst.add(row * dst_pitch), length) };
    }
    Ok(())
}

impl D3d11Renderer {
    /// The device, or the error of a lost one.
    fn device(&self) -> Result<&Device> {
        self.d3d_device.as_ref().ok_or_else(device_lost)
    }

    /// The immediate context, or the error of a lost device.
    fn context(&self) -> Result<&DeviceContext> {
        self.d3d_context.as_ref().ok_or_else(device_lost)
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

    /// Whether the window is transparent (`SDL_WINDOW_TRANSPARENT`).
    fn window_transparent(&self) -> bool {
        self.window
            .flags()
            .unwrap_or_default()
            .contains(WindowFlags::TRANSPARENT)
    }

    /// Set the device and swap chain properties of the renderer, once it
    /// has them (`SDL_SetPointerProperty()` in
    /// `D3D11_CreateDeviceResources()` and `D3D11_CreateSwapChain()`).
    fn publish_properties(&self) {
        let Some(props) = &self.props else {
            return;
        };
        if let Some(device) = &self.d3d_device {
            let _ = props.set(PROP_RENDERER_D3D11_DEVICE_POINTER, raw(device) as i64);
        }
        if let Some(swap_chain) = &self.swap_chain {
            let _ = props.set(
                PROP_RENDERER_D3D11_SWAPCHAIN_POINTER,
                raw(swap_chain) as i64,
            );
        }
    }

    /// Translation of `D3D11_ReleaseAll()`.
    fn release_all(&mut self) {
        // Release all textures
        let ids: Vec<u32> = self.textures.keys().copied().collect();
        for id in ids {
            self.destroy_texture_data(id);
        }
        // FIXME (upstream): the palettes aren't released: those made on a
        // lost device are kept, and bound with the new device's textures.

        // Release/reset everything else

        // Make sure the swap chain is fully released
        if let Some(context) = &self.d3d_context {
            context.clear_state();
            context.flush();
        }

        self.vertex_shader_constants = None;
        self.clipped_rasterizer = None;
        self.main_rasterizer = None;
        self.samplers = Default::default();

        self.blend_modes.clear();
        self.pixel_shaders = Default::default();
        for state in &mut self.current_shader_state {
            state.constants = None;
        }
        self.vertex_shader = None;
        self.vertex_buffers = Default::default();
        self.input_layout = None;
        self.main_render_target_view = None;
        self.swap_chain = None;

        self.d3d_context = None;
        self.d3d_device = None;
        self.dxgi_factory = None;

        self.swap_effect = 0;
        self.rotation = DXGI_MODE_ROTATION_UNSPECIFIED;
        self.current_offscreen_render_target = None;
        self.current_render_target_view = null_mut();
        self.current_rasterizer_state = null_mut();
        self.current_blend_state = null_mut();
        self.current_shader = None;
        self.current_shader_state = Default::default();
        self.num_current_shader_resources = 0;
        self.current_shader_resource = null_mut();
        self.num_current_shader_samplers = 0;
        self.current_shader_sampler = null_mut();

        // Check for any leaks if in debug mode
        if let Some(dxgi_debug) = self.dxgi_debug.take() {
            let rlo_flags = DXGI_DEBUG_RLO_DETAIL | DXGI_DEBUG_RLO_IGNORE_INTERNAL;
            dxgi_debug.report_live_objects(DXGI_DEBUG_ALL, rlo_flags);
        }

        /* Unload the D3D libraries.  This should be done last, in order
         * to prevent IUnknown::Release() calls from crashing.
         */
        self.h_d3d11_mod = None;
        self.h_dxgi_mod = None;
    }

    /// Translation of `D3D11_CreateBlendState()`.
    fn create_blend_state(&mut self, blend_mode: BlendMode) -> Result<BlendState> {
        let desc = blend_desc(blend_mode);
        let blend_state = self
            .device()?
            .create_blend_state(&desc)
            .map_err(|hr| hr_error("ID3D11Device1::CreateBlendState", hr))?;

        self.blend_modes.push((blend_mode, blend_state.clone()));

        Ok(blend_state)
    }

    /// Translation of `D3D11_CreateVertexShader()` (`SDL_shaders_d3d11.c`).
    fn create_vertex_shader(device: &Device) -> Result<(VertexShader, InputLayout)> {
        // Declare how the input layout for SDL's vertex shader will be setup:
        let element = |name: &'static [u8], format, offset| InputElementDesc {
            semantic_name: name.as_ptr(),
            semantic_index: 0,
            format,
            input_slot: 0,
            aligned_byte_offset: offset,
            input_slot_class: D3D11_INPUT_PER_VERTEX_DATA,
            instance_data_step_rate: 0,
        };
        let vertex_desc = [
            element(b"POSITION\0", DXGI_FORMAT_R32G32_FLOAT, 0),
            element(b"TEXCOORD\0", DXGI_FORMAT_R32G32_FLOAT, 8),
            element(b"COLOR\0", DXGI_FORMAT_R32G32B32A32_FLOAT, 16),
        ];

        // Load in SDL's one and only vertex shader:
        let vertex_shader = device
            .create_vertex_shader(shaders::vertex_shader())
            .map_err(|hr| hr_error("ID3D11Device1::CreateVertexShader", hr))?;

        // Create an input layout for SDL's vertex shader:
        let input_layout = device
            .create_input_layout(&vertex_desc, shaders::vertex_shader())
            .map_err(|hr| hr_error("ID3D11Device1::CreateInputLayout", hr))?;
        Ok((vertex_shader, input_layout))
    }

    /// Translation of `D3D11_CreatePixelShader()` (`SDL_shaders_d3d11.c`).
    fn create_pixel_shader(device: &Device, shader: Shader) -> Result<PixelShader> {
        device
            .create_pixel_shader(shader.pixel_shader())
            .map_err(|hr| hr_error("ID3D11Device1::CreatePixelShader", hr))
    }

    /// Create resources that depend on the device. Translation of
    /// `D3D11_CreateDeviceResources()` (the error of a failed `HRESULT`).
    fn create_device_resources(&mut self) -> Result<()> {
        /* This array defines the set of DirectX hardware feature levels this app will support.
         * Note the ordering should be preserved.
         * Don't forget to declare your application's minimum required feature level in its
         * description. All applications are assumed to support 11.0 unless otherwise stated.
         */
        let feature_levels = [D3D_FEATURE_LEVEL_11_1, D3D_FEATURE_LEVEL_11_0];

        // See if we need debug interfaces
        let create_debug = hints::get_bool(hints::RENDER_DIRECT3D11_DEBUG, false);

        let dxgi_mod = self.h_dxgi_mod.insert(SharedObject::load("dxgi.dll")?);

        // SAFETY: the signatures of dxgi.dll's exports.
        let p_create_dxgi_factory2 =
            unsafe { dxgi_mod.function::<PfnCreateDxgiFactory2>("CreateDXGIFactory2") }.ok();
        let mut p_create_dxgi_factory = None;
        if p_create_dxgi_factory2.is_none() {
            // SAFETY: as above.
            p_create_dxgi_factory =
                Some(unsafe { dxgi_mod.function::<PfnCreateDxgiFactory>("CreateDXGIFactory") }?);
        }
        // SAFETY: as above (upstream calls it through the same type).
        let p_dxgi_get_debug_interface1 = if create_debug {
            Some(unsafe { dxgi_mod.function::<PfnCreateDxgiFactory2>("DXGIGetDebugInterface1") }?)
        } else {
            None
        };

        let d3d11_mod = self.h_d3d11_mod.insert(SharedObject::load("d3d11.dll")?);

        // SAFETY: the signature of d3d11.dll's export.
        let p_d3d11_create_device =
            unsafe { d3d11_mod.function::<PfnD3d11CreateDevice>("D3D11CreateDevice") }?;

        let mut creation_flags = 0;
        if let Some(p_dxgi_get_debug_interface1) = p_dxgi_get_debug_interface1 {
            // If the debug hint is set, also create the DXGI factory in debug mode
            // SAFETY: DXGIGetDebugInterface1 stores an owned interface of the
            // IID asked for.
            let dxgi_debug = unsafe {
                out::<IDXGIDebugVtbl>(|o| p_dxgi_get_debug_interface1(0, &IID_IDXGIDEBUG1, o))
            }
            .map_err(|hr| hr_error("DXGIGetDebugInterface1", hr))?;
            self.dxgi_debug = Some(dxgi_debug);

            // SAFETY: as above.
            let dxgi_info_queue = unsafe {
                out::<IDXGIInfoQueueVtbl>(|o| {
                    p_dxgi_get_debug_interface1(0, &IID_IDXGIINFOQUEUE, o)
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

        // SAFETY: the factory functions store an owned IDXGIFactory2, the
        // interface asked for.
        let factory = unsafe {
            out::<IDXGIFactory2Vtbl>(|o| match (p_create_dxgi_factory2, p_create_dxgi_factory) {
                (Some(f), _) => f(creation_flags, &IID_IDXGIFACTORY2, o),
                (None, Some(f)) => f(&IID_IDXGIFACTORY2, o),
                (None, None) => E_FAIL,
            })
        }
        .map_err(|hr| hr_error("CreateDXGIFactory", hr))?;
        let factory = self.dxgi_factory.insert(factory);

        // Check for tearing support, which requires the IDXGIFactory5 interface.
        self.swap_chain_flags = 0;
        if !self
            .window
            .flags()
            .unwrap_or_default()
            .contains(WindowFlags::TRANSPARENT)
        {
            if let Ok(dxgi_factory5) = factory.query::<IDXGIFactory5Vtbl>(&IID_IDXGIFACTORY5) {
                if dxgi_factory5
                    .check_feature_support_bool(DXGI_FEATURE_PRESENT_ALLOW_TEARING)
                    .unwrap_or(false)
                {
                    self.swap_chain_flags = DXGI_SWAP_CHAIN_FLAG_ALLOW_TEARING;
                }
            }
        }

        let (driver_type, dxgi_adapter) = if hints::get_bool(hints::RENDER_DIRECT3D11_WARP, false) {
            (D3D_DRIVER_TYPE_WARP, None)
        } else {
            // FIXME: Should we use the default adapter?
            let adapter = factory
                .enum_adapters(0)
                .map_err(|hr| hr_error("EnumAdapters", hr))?;
            (D3D_DRIVER_TYPE_UNKNOWN, Some(adapter))
        };

        /* This flag adds support for surfaces with a different color channel ordering
         * than the API default. It is required for compatibility with Direct2D.
         */
        creation_flags = D3D11_CREATE_DEVICE_BGRA_SUPPORT;

        // Make sure Direct3D's debugging feature gets used, if the app requests it.
        if create_debug {
            creation_flags |= D3D11_CREATE_DEVICE_DEBUG;
        }

        // Create a single-threaded device unless the app requests otherwise.
        if !hints::get_bool(hints::RENDER_DIRECT3D_THREADSAFE, false) {
            creation_flags |= D3D11_CREATE_DEVICE_SINGLETHREADED;
        }

        // Create the Direct3D 11 API device object and a corresponding context.
        let mut d3d_device: *mut c_void = null_mut();
        let mut d3d_context: *mut c_void = null_mut();
        let mut feature_level = 0;
        // SAFETY: the adapter, if any, is live; the outputs are valid; on
        // success the device and context are owned references.
        let result = unsafe {
            p_d3d11_create_device(
                raw_or_null(dxgi_adapter.as_ref()),
                driver_type,
                std::ptr::null_mut(),
                creation_flags, // Set set debug and Direct2D compatibility flags.
                feature_levels.as_ptr(), // List of feature levels this app can support.
                feature_levels.len() as u32,
                D3D11_SDK_VERSION, // Always set this to D3D11_SDK_VERSION for Windows Store apps.
                &mut d3d_device,   // Returns the Direct3D device created.
                &mut feature_level, // Returns feature level of device created.
                &mut d3d_context,  // Returns the device immediate context.
            )
        };
        // SAFETY: owned references (or NULL), whatever the result.
        let (d3d_device, d3d_context) = unsafe {
            (
                ComPtr::<IUnknownVtbl>::from_raw(d3d_device.cast()),
                ComPtr::<IUnknownVtbl>::from_raw(d3d_context.cast()),
            )
        };
        drop(dxgi_adapter);
        check(result).map_err(|hr| hr_error("D3D11CreateDevice", hr))?;
        self.feature_level = feature_level;
        let (Some(d3d_device), Some(d3d_context)) = (d3d_device, d3d_context) else {
            return Err(hr_error("D3D11CreateDevice", E_FAIL));
        };

        let device = d3d_device
            .query::<ID3D11DeviceVtbl>(&IID_ID3D11DEVICE1)
            .map_err(|hr| hr_error("ID3D11Device to ID3D11Device1", hr))?;
        let device = self.d3d_device.insert(device);

        let context = d3d_context
            .query::<ID3D11DeviceContextVtbl>(&IID_ID3D11DEVICECONTEXT1)
            .map_err(|hr| hr_error("ID3D11DeviceContext to ID3D11DeviceContext1", hr))?;
        self.d3d_context = Some(context);

        let dxgi_device = d3d_device
            .query::<IDXGIDevice1Vtbl>(&IID_IDXGIDEVICE1)
            .map_err(|hr| hr_error("ID3D11Device to IDXGIDevice1", hr))?;

        /* Ensure that DXGI does not queue more than one frame at a time. This both reduces latency and
         * ensures that the application will only render after each VSync, minimizing power consumption.
         */
        check(dxgi_device.set_maximum_frame_latency(1))
            .map_err(|hr| hr_error("IDXGIDevice1::SetMaximumFrameLatency", hr))?;

        /* Make note of the maximum texture size
         * Max texture sizes are documented on MSDN, at:
         * http://msdn.microsoft.com/en-us/library/windows/apps/ff476876.aspx
         */
        // (max_texture_size())

        match Self::create_vertex_shader(device) {
            Ok((vertex_shader, input_layout)) => {
                self.vertex_shader = Some(vertex_shader);
                self.input_layout = Some(input_layout);
            }
            Err(_) => {
                // FIXME (upstream): this `goto done` returns the S_OK of
                // SetMaximumFrameLatency(), so the renderer is made without
                // its shaders and states.
                return Ok(());
            }
        }

        // Setup space to hold vertex shader constants:
        let constant_buffer_desc = BufferDesc {
            byte_width: size_of::<VertexShaderConstants>() as u32,
            usage: D3D11_USAGE_DEFAULT,
            bind_flags: D3D11_BIND_CONSTANT_BUFFER,
            ..Default::default()
        };
        self.vertex_shader_constants = Some(
            device
                .create_buffer(&constant_buffer_desc, None)
                .map_err(|hr| {
                    hr_error("ID3D11Device1::CreateBuffer [vertex shader constants]", hr)
                })?,
        );

        // Setup Direct3D rasterizer states
        let mut raster_desc = RasterizerDesc {
            antialiased_line_enable: 0,
            cull_mode: D3D11_CULL_NONE,
            depth_bias: 0,
            depth_bias_clamp: 0.0,
            depth_clip_enable: 1,
            fill_mode: D3D11_FILL_SOLID,
            front_counter_clockwise: 0,
            multisample_enable: 0,
            scissor_enable: 0,
            slope_scaled_depth_bias: 0.0,
        };
        self.main_rasterizer =
            Some(device.create_rasterizer_state(&raster_desc).map_err(|hr| {
                hr_error("ID3D11Device1::CreateRasterizerState [main rasterizer]", hr)
            })?);

        raster_desc.scissor_enable = 1;
        self.clipped_rasterizer =
            Some(device.create_rasterizer_state(&raster_desc).map_err(|hr| {
                hr_error(
                    "ID3D11Device1::CreateRasterizerState [clipped rasterizer]",
                    hr,
                )
            })?);

        // Create blending states:
        if self.create_blend_state(BlendMode::BLEND).is_err() {
            // D3D11_CreateBlendState will set the SDL error, if it fails
            // FIXME (upstream): returning the S_OK of the rasterizer states,
            // as for the vertex shader above.
            return Ok(());
        }

        // Setup render state that doesn't change
        let context = self.context()?;
        if let (Some(input_layout), Some(vertex_shader), Some(constants)) = (
            &self.input_layout,
            &self.vertex_shader,
            &self.vertex_shader_constants,
        ) {
            context.ia_set_input_layout(input_layout);
            context.vs_set_shader(vertex_shader);
            context.vs_set_constant_buffers(0, &[raw(constants)]);
        }

        self.publish_properties();

        Ok(())
    }

    /// Translation of `D3D11_GetRotationForCurrentRenderTarget()`.
    fn get_rotation_for_current_render_target(&self) -> DxgiModeRotation {
        if self.current_offscreen_render_target.is_some() {
            DXGI_MODE_ROTATION_IDENTITY
        } else {
            self.rotation
        }
    }

    /// Translation of `D3D11_GetViewportAlignedD3DRect()`.
    fn get_viewport_aligned_d3d_rect(
        &self,
        sdl_rect: &Rect,
        include_viewport_offset: bool,
    ) -> Result<D3d11Rect> {
        let rotation = self.get_rotation_for_current_render_target();
        let viewport = &self.current_viewport;

        let mut out_rect = D3d11Rect {
            left: 0,
            top: 0,
            right: 0,
            bottom: 0,
        };
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

    /// Translation of `D3D11_CreateSwapChain()`.
    fn create_swap_chain(&mut self, w: i32, h: i32) -> Result<()> {
        let device = self.device()?.clone();
        let factory = self.dxgi_factory.clone().ok_or_else(device_lost)?;

        // Create a swap chain using the same adapter as the existing Direct3D device.
        let mut swap_chain_desc = SwapChainDesc1 {
            width: w as u32,
            height: h as u32,
            format: match self.output_colorspace {
                Colorspace::SRGB_LINEAR => DXGI_FORMAT_R16G16B16A16_FLOAT,
                Colorspace::HDR10 => DXGI_FORMAT_R10G10B10A2_UNORM,
                _ => DXGI_FORMAT_B8G8R8A8_UNORM, // This is the most common swap chain format.
            },
            stereo: 0,
            sample_desc: SampleDesc {
                count: 1, // Don't use multi-sampling.
                quality: 0,
            },
            buffer_usage: DXGI_USAGE_RENDER_TARGET_OUTPUT,
            buffer_count: 2, // Use double-buffering to minimize latency.
            ..Default::default()
        };
        if is_windows_8_or_greater() {
            swap_chain_desc.scaling = DXGI_SCALING_NONE;
        } else {
            swap_chain_desc.scaling = DXGI_SCALING_STRETCH;
        }
        if self.window_transparent() {
            swap_chain_desc.scaling = DXGI_SCALING_STRETCH;
            swap_chain_desc.swap_effect = DXGI_SWAP_EFFECT_DISCARD;
        } else {
            swap_chain_desc.swap_effect = DXGI_SWAP_EFFECT_FLIP_SEQUENTIAL; // All Windows Store apps must use this SwapEffect.
        }
        swap_chain_desc.flags = self.swap_chain_flags;

        // (the CoreWindow branch: see the module documentation)
        let Some(hwnd) = self.hwnd() else {
            return Err(Error::new("Couldn't get window handle"));
        };

        let swap_chain = factory
            .create_swap_chain_for_hwnd(&device, hwnd, &swap_chain_desc)
            .map_err(|hr| hr_error("IDXGIFactory2::CreateSwapChainForHwnd", hr))?;
        let swap_chain = self.swap_chain.insert(swap_chain);

        factory.make_window_association(hwnd, DXGI_MWA_NO_WINDOW_CHANGES);
        self.swap_effect = swap_chain_desc.swap_effect;

        let mut result = Ok(());
        // FIXME (upstream): the swap chain is asked for IDXGISwapChain2 and
        // then used as an IDXGISwapChain3, whose methods come after
        // IDXGISwapChain2's: on a system with IDXGISwapChain2 but not
        // IDXGISwapChain3 (Windows 8.1), the calls go past its vtable.
        if let Ok(swap_chain3) = swap_chain.query::<IDXGISwapChain3Vtbl>(&IID_IDXGISWAPCHAIN2) {
            let colorspace = match self.output_colorspace {
                Colorspace::SRGB_LINEAR => DXGI_COLOR_SPACE_RGB_FULL_G10_NONE_P709,
                Colorspace::HDR10 => DXGI_COLOR_SPACE_RGB_FULL_G2084_NONE_P2020,
                // sRGB
                _ => DXGI_COLOR_SPACE_RGB_FULL_G22_NONE_P709,
            };
            let colorspace_support = swap_chain3.check_color_space_support(colorspace);
            if colorspace_support.is_ok_and(|support| {
                support & DXGI_SWAP_CHAIN_COLOR_SPACE_SUPPORT_FLAG_PRESENT != 0
            }) {
                check(swap_chain3.set_color_space1(colorspace))
                    .map_err(|hr| hr_error("IDXGISwapChain3::SetColorSpace1", hr))?;
            } else if colorspace != DXGI_COLOR_SPACE_RGB_FULL_G10_NONE_P709 {
                // Not the default, we're not going to be able to present in this colorspace
                // (failing with DXGI_ERROR_UNSUPPORTED, once the property is set)
                result = Err(Error::new("Unsupported output colorspace"));
            }
        }

        self.publish_properties();

        result
    }

    /// Translation of `D3D11_ReleaseMainRenderTargetView()`.
    fn release_main_render_target_view(&mut self) {
        if let Some(context) = &self.d3d_context {
            context.om_set_render_targets(&[]);
        }
        self.main_render_target_view = None;
    }

    /// Translation of `D3D11_UpdatePresentFlags()`.
    fn update_present_flags(&mut self) {
        if self.sync_interval > 0 {
            self.present_flags = 0;
        } else {
            self.present_flags = DXGI_PRESENT_DO_NOT_WAIT;
            // Present tearing requires sync interval 0, a swap chain flag, and not in exclusive fullscreen mode.
            if self.swap_chain_flags & DXGI_SWAP_CHAIN_FLAG_ALLOW_TEARING != 0 {
                let fullscreen_state = self.swap_chain.as_ref().map(|s| s.fullscreen_state());
                if let Some(Ok(false)) = fullscreen_state {
                    self.present_flags = DXGI_PRESENT_ALLOW_TEARING;
                }
            }
        }
    }

    /// Initialize all resources that change when the window's size changes.
    /// Translation of `D3D11_CreateWindowSizeDependentResources()`.
    fn create_window_size_dependent_resources(&mut self) -> Result<()> {
        // Release the previous render target view
        self.release_main_render_target_view();

        /* The width and height of the swap chain must be based on the display's
         * non-rotated size.
         */
        let (mut w, mut h) = self.window.size_in_pixels()?;
        self.rotation = get_current_rotation();
        // SDL_Log("%s: windowSize={%d,%d}, orientation=%d", SDL_FUNCTION, w, h, (int)data->rotation);
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
                self.swap_chain_flags,
            ))
            .map_err(|hr| hr_error("IDXGISwapChain::ResizeBuffers", hr))?;
        } else {
            self.create_swap_chain(w, h)?;
        }
        let swap_chain = self.swap_chain.clone().ok_or_else(device_lost)?;

        self.update_present_flags();

        // Set the proper rotation for the swap chain.
        if is_windows_8_or_greater() && self.swap_effect == DXGI_SWAP_EFFECT_FLIP_SEQUENTIAL {
            check(swap_chain.set_rotation(self.rotation))
                .map_err(|hr| hr_error("IDXGISwapChain1::SetRotation", hr))?;
        }

        let back_buffer = swap_chain
            .buffer(0)
            .map_err(|hr| hr_error("IDXGISwapChain::GetBuffer [back-buffer]", hr))?;

        // Create a render target view of the swap chain back buffer.
        let main_render_target_view = self
            .device()?
            .create_render_target_view(&back_buffer, None)
            .map_err(|hr| hr_error("ID3D11Device::CreateRenderTargetView", hr))?;

        /* Set the swap chain target immediately, so that a target is always set
         * even before we get to SetDrawState. Without this it's possible to hit
         * null references in places like ReadPixels!
         */
        self.context()?
            .om_set_render_targets(&[raw(&main_render_target_view)]);
        self.main_render_target_view = Some(main_render_target_view);

        self.viewport_dirty = true;

        Ok(())
    }

    /// Translation of `D3D11_HandleDeviceLost()`.
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
    /// `D3D11_UpdateForWindowSizeChange()`.
    fn update_for_window_size_change(&mut self) -> Result<()> {
        self.create_window_size_dependent_resources()
    }

    /// Translation of `D3D11_DestroyTexture()`, for the renderer's data of
    /// a texture (its interfaces are released as it's dropped).
    fn destroy_texture_data(&mut self, id: u32) {
        self.textures.remove(&id);
        if self.current_offscreen_render_target == Some(id) {
            // (upstream keeps the dangling view, which is never bound
            // again: the front end resets the target first)
            self.current_offscreen_render_target = None;
        }
    }

    /// Translation of `D3D11_UpdateTextureInternal()`.
    #[allow(clippy::too_many_arguments)]
    fn update_texture_internal(
        &self,
        texture: &Texture2d,
        bpp: i32,
        (x, y, w, mut h): (i32, i32, i32, i32),
        pixels: &[u8],
        mut pitch: i32,
    ) -> Result<()> {
        let device = self.device()?;
        let context = self.context()?;

        // Create a 'staging' texture, which will be used to write to a portion of the main texture.
        let mut staging_texture_desc = texture.desc();
        staging_texture_desc.width = w as u32;
        staging_texture_desc.height = h as u32;
        staging_texture_desc.bind_flags = 0;
        staging_texture_desc.misc_flags = 0;
        staging_texture_desc.cpu_access_flags = D3D11_CPU_ACCESS_WRITE;
        staging_texture_desc.usage = D3D11_USAGE_STAGING;
        let planar = staging_texture_desc.format == DXGI_FORMAT_NV12
            || staging_texture_desc.format == DXGI_FORMAT_P010;
        if planar {
            staging_texture_desc.width = (staging_texture_desc.width + 1) & !1;
            staging_texture_desc.height = (staging_texture_desc.height + 1) & !1;
        }
        let staging_texture = device
            .create_texture_2d(&staging_texture_desc)
            .map_err(|hr| {
                hr_error(
                    "ID3D11Device1::CreateTexture2D [create staging texture]",
                    hr,
                )
            })?;

        // Get a write-only pointer to data in the staging texture:
        let texture_memory = context
            .map(&staging_texture, D3D11_MAP_WRITE)
            .map_err(|hr| hr_error("ID3D11DeviceContext1::Map [map staging texture]", hr))?;

        let dst = texture_memory.data.cast::<u8>();
        let row_pitch = texture_memory.row_pitch as usize;
        let mut length = (w * bpp) as u32;
        let mut src_offset = 0;
        let copied = if length == pitch as u32 && length == texture_memory.row_pitch {
            // SAFETY: the mapped staging texture has `h` rows of RowPitch
            // (= length) bytes: one block of them.
            unsafe { copy_rows(dst, 0, pixels, 0, length as usize * h.max(0) as usize, 1) }
        } else {
            if length > pitch as u32 {
                length = pitch as u32;
            }
            if length > texture_memory.row_pitch {
                length = texture_memory.row_pitch;
            }
            src_offset = h.max(0) as usize * pitch as usize;
            // SAFETY: the mapped staging texture has `h` rows of RowPitch
            // bytes, and length <= RowPitch.
            unsafe {
                copy_rows(
                    dst,
                    row_pitch,
                    pixels,
                    pitch as usize,
                    length as usize,
                    h.max(0) as usize,
                )
            }
        };

        let copied = copied.and_then(|()| {
            if !planar {
                return Ok(());
            }
            // Copy the UV plane as well
            // FIXME (upstream): after the single-copy branch above, `src`
            // still points at the Y plane here. No caller gets here with an
            // NV12 or P010 texture (D3D11_UpdateTexture() sends them to
            // D3D11_UpdateTextureNV()), so it doesn't show.
            h = (h + 1) / 2;
            if staging_texture_desc.format == DXGI_FORMAT_P010 {
                length = (length + 3) & !3;
                pitch = (pitch + 3) & !3;
            } else {
                length = (length + 1) & !1;
                pitch = (pitch + 1) & !1;
            }
            let uv_offset = staging_texture_desc.height as usize * row_pitch;
            // SAFETY: the chroma plane of the mapped staging texture follows
            // its Height rows, with (Height + 1) / 2 rows of RowPitch bytes.
            unsafe {
                copy_rows(
                    dst.add(uv_offset),
                    row_pitch,
                    plane(pixels, src_offset)?,
                    pitch as usize,
                    length as usize,
                    h as usize,
                )
            }
        });

        // Commit the pixel buffer's changes back to the staging texture:
        context.unmap(&staging_texture);
        copied?;

        // Copy the staging texture's contents back to the texture:
        context.copy_subresource_region(texture, (x as u32, y as u32), &staging_texture, None);

        Ok(())
    }

    /// The key of the renderer's data of a texture.
    fn texture_id(texture: &TextureData) -> Result<u32> {
        texture_ref(texture).map(|r| r.id).ok_or_else(not_available)
    }

    /// The renderer's data of a texture.
    fn texture_data(&self, texture: &TextureData) -> Result<&D3d11Texture> {
        let id = Self::texture_id(texture)?;
        self.textures.get(&id).ok_or_else(not_available)
    }

    /// Translation of `D3D11_UpdateTexture()`.
    fn update_texture_pixels(
        &self,
        texture: &TextureData,
        rect: &Rect,
        src_pixels: &[u8],
        src_pitch: i32,
    ) -> Result<()> {
        use PixelFormat as F;
        let texture_data = self.texture_data(texture)?;
        let rows = rect.h.max(0) as usize;
        let bytes_per_pixel = texture.format.bytes_per_pixel() as i32;

        if texture_data.nv12 {
            let uv_bpp = bytes_per_pixel * 2;
            let y_pitch = src_pitch;
            let uv_pitch = (src_pitch + (uv_bpp - 1)) & !(uv_bpp - 1);
            let plane0 = src_pixels;
            let plane1 = plane(plane0, rows * src_pitch as usize)?;

            return self.update_texture_nv_planes(
                texture,
                rect,
                (plane0, y_pitch),
                (plane1, uv_pitch),
            );
        } else if texture_data.yuv {
            if texture.format == F::I444 || texture.format == F::I4FL {
                let plane0 = src_pixels;
                let plane1 = plane(plane0, rows * src_pitch as usize)?;
                let plane2 = plane(plane1, rows * src_pitch as usize)?;

                let full = (rect.x, rect.y, rect.w, rect.h);
                let (u, v) = (
                    texture_data.main_texture_u.as_ref(),
                    texture_data.main_texture_v.as_ref(),
                );
                let (Some(u), Some(v)) = (u, v) else {
                    return Err(not_available());
                };
                self.update_texture_internal(u, bytes_per_pixel, full, plane1, src_pitch)?;
                self.update_texture_internal(v, bytes_per_pixel, full, plane2, src_pitch)?;
                // (and on to the Y plane below)
            } else {
                let bpp = bytes_per_pixel;
                let y_pitch = src_pitch;
                let uv_pitch = ((y_pitch / bpp + 1) / 2) * bpp;
                let plane0 = src_pixels;
                let plane1 = plane(plane0, rows * y_pitch as usize)?;
                let plane2 = plane(plane1, rows.div_ceil(2) * uv_pitch as usize)?;

                if texture.format == F::YV12 {
                    return self.update_texture_yuv_planes(
                        texture,
                        rect,
                        (plane0, y_pitch),
                        (plane2, uv_pitch),
                        (plane1, uv_pitch),
                    );
                } else {
                    return self.update_texture_yuv_planes(
                        texture,
                        rect,
                        (plane0, y_pitch),
                        (plane1, uv_pitch),
                        (plane2, uv_pitch),
                    );
                }
            }
        }

        let main_texture = texture_data
            .main_texture
            .as_ref()
            .ok_or_else(not_available)?;
        self.update_texture_internal(
            main_texture,
            bytes_per_pixel,
            (rect.x, rect.y, rect.w, rect.h),
            src_pixels,
            src_pitch,
        )
    }

    /// Translation of `D3D11_UpdateTextureYUV()`.
    fn update_texture_yuv_planes(
        &self,
        texture: &TextureData,
        rect: &Rect,
        (y_plane, y_pitch): (&[u8], i32),
        (u_plane, u_pitch): (&[u8], i32),
        (v_plane, v_pitch): (&[u8], i32),
    ) -> Result<()> {
        let texture_data = self.texture_data(texture)?;
        let bpp = texture.format.bytes_per_pixel() as i32;
        let (Some(main), Some(u), Some(v)) = (
            texture_data.main_texture.as_ref(),
            texture_data.main_texture_u.as_ref(),
            texture_data.main_texture_v.as_ref(),
        ) else {
            return Err(not_available());
        };

        let full = (rect.x, rect.y, rect.w, rect.h);
        self.update_texture_internal(main, bpp, full, y_plane, y_pitch)?;
        if texture.format == PixelFormat::I444 || texture.format == PixelFormat::I4FL {
            self.update_texture_internal(u, bpp, full, u_plane, u_pitch)?;
            self.update_texture_internal(v, bpp, full, v_plane, v_pitch)?;
        } else {
            let half = (rect.x / 2, rect.y / 2, (rect.w + 1) / 2, (rect.h + 1) / 2);
            self.update_texture_internal(u, bpp, half, u_plane, u_pitch)?;
            self.update_texture_internal(v, bpp, half, v_plane, v_pitch)?;
        }
        Ok(())
    }

    /// Translation of `D3D11_UpdateTextureNV()`.
    fn update_texture_nv_planes(
        &self,
        texture: &TextureData,
        rect: &Rect,
        (y_plane, y_pitch): (&[u8], i32),
        (uv_plane, mut uv_pitch): (&[u8], i32),
    ) -> Result<()> {
        let texture_data = self.texture_data(texture)?;
        let main_texture = texture_data
            .main_texture
            .as_ref()
            .ok_or_else(not_available)?;
        let device = self.device()?;
        let context = self.context()?;
        let bpp = texture.format.bytes_per_pixel() as i32;

        let w = rect.w;
        let mut h = rect.h;

        // Create a 'staging' texture, which will be used to write to a portion of the main texture.
        let mut staging_texture_desc = main_texture.desc();
        staging_texture_desc.width = w as u32;
        staging_texture_desc.height = h as u32;
        staging_texture_desc.bind_flags = 0;
        staging_texture_desc.misc_flags = 0;
        staging_texture_desc.cpu_access_flags = D3D11_CPU_ACCESS_WRITE;
        staging_texture_desc.usage = D3D11_USAGE_STAGING;
        if staging_texture_desc.format == DXGI_FORMAT_NV12
            || staging_texture_desc.format == DXGI_FORMAT_P010
        {
            staging_texture_desc.width = (staging_texture_desc.width + 1) & !1;
            staging_texture_desc.height = (staging_texture_desc.height + 1) & !1;
        }
        let staging_texture = device
            .create_texture_2d(&staging_texture_desc)
            .map_err(|hr| {
                hr_error(
                    "ID3D11Device1::CreateTexture2D [create staging texture]",
                    hr,
                )
            })?;

        // Get a write-only pointer to data in the staging texture:
        let texture_memory = context
            .map(&staging_texture, D3D11_MAP_WRITE)
            .map_err(|hr| hr_error("ID3D11DeviceContext1::Map [map staging texture]", hr))?;

        let dst = texture_memory.data.cast::<u8>();
        let row_pitch = texture_memory.row_pitch as usize;
        let mut length = (w * bpp) as u32;
        let copied = if length == y_pitch as u32 && length == texture_memory.row_pitch {
            // SAFETY: the mapped staging texture has `h` rows of RowPitch
            // (= length) bytes: one block of them.
            unsafe { copy_rows(dst, 0, y_plane, 0, length as usize * h.max(0) as usize, 1) }
        } else {
            if length > y_pitch as u32 {
                length = y_pitch as u32;
            }
            if length > texture_memory.row_pitch {
                length = texture_memory.row_pitch;
            }
            // SAFETY: the mapped staging texture has `h` rows of RowPitch
            // bytes, and length <= RowPitch.
            unsafe {
                copy_rows(
                    dst,
                    row_pitch,
                    y_plane,
                    y_pitch as usize,
                    length as usize,
                    h.max(0) as usize,
                )
            }
        };

        let copied = copied.and_then(|()| {
            let mut length = (((w + 1) / 2) * 2 * bpp) as u32;
            h = (h + 1) / 2;
            if staging_texture_desc.format == DXGI_FORMAT_P010 {
                length = (length + 3) & !3;
                uv_pitch = (uv_pitch + 3) & !3;
            } else {
                length = (length + 1) & !1;
                uv_pitch = (uv_pitch + 1) & !1;
            }
            let uv_offset = staging_texture_desc.height as usize * row_pitch;
            // SAFETY: the chroma plane of the mapped staging texture follows
            // its Height rows, with (Height + 1) / 2 rows of RowPitch bytes.
            unsafe {
                copy_rows(
                    dst.add(uv_offset),
                    row_pitch,
                    uv_plane,
                    uv_pitch as usize,
                    length as usize,
                    h.max(0) as usize,
                )
            }
        });

        // Commit the pixel buffer's changes back to the staging texture:
        context.unmap(&staging_texture);
        copied?;

        // Copy the staging texture's contents back to the texture:
        context.copy_subresource_region(
            main_texture,
            (rect.x as u32, rect.y as u32),
            &staging_texture,
            None,
        );

        Ok(())
    }

    /// Translation of `D3D11_UpdateViewport()`.
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
        self.vertex_shader_constants_data.projection_and_view =
            Float4X4::multiply(&view, &projection);

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

        let d3dviewport = Viewport {
            top_left_x: orientation_aligned_viewport.x,
            top_left_y: orientation_aligned_viewport.y,
            width: orientation_aligned_viewport.w,
            height: orientation_aligned_viewport.h,
            min_depth: 0.0,
            max_depth: 1.0,
        };
        // SDL_Log("%s: D3D viewport = {%f,%f,%f,%f}", SDL_FUNCTION, d3dviewport.TopLeftX, d3dviewport.TopLeftY, d3dviewport.Width, d3dviewport.Height);
        if let Some(context) = &self.d3d_context {
            context.rs_set_viewports(&[d3dviewport]);
        }

        self.viewport_dirty = false;

        true
    }

    /// Translation of `D3D11_GetCurrentRenderTargetView()`.
    fn get_current_render_target_view(&self) -> Option<&RenderTargetView> {
        match self.current_offscreen_render_target {
            Some(id) => self
                .textures
                .get(&id)
                .and_then(|t| t.main_texture_render_target_view.as_ref()),
            None => self.main_render_target_view.as_ref(),
        }
    }

    /// The HDR headroom of the output (`renderer->target->HDR_headroom` or
    /// `renderer->HDR_headroom`).
    fn output_headroom(&self, textures: &TextureStore) -> f32 {
        match self.target.and_then(|t| textures.get(t)) {
            Some(target) => target.hdr_headroom,
            None => self.hdr_headroom,
        }
    }

    /// Translation of `D3D11_SetupShaderConstants()`.
    fn setup_shader_constants(
        &self,
        cmd: &DrawCmd,
        texture: Option<(&SourceTexture, &D3d11Texture)>,
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

    /// Translation of `SelectShader()`.
    fn select_shader(&self, shader_constants: Option<&PixelShaderConstants>) -> Shader {
        select_shader(self.current_colorspace, shader_constants)
    }

    /// Translation of `D3D11_SetDrawState()`.
    fn set_draw_state(
        &mut self,
        cmd: &DrawCmd,
        shader_constants: Option<&PixelShaderConstants>,
        shader_resources: &[*mut c_void],
        shader_samplers: &[*mut c_void],
        matrix: Option<&Float4X4>,
    ) -> Result<()> {
        let newmatrix = *matrix.unwrap_or(&self.identity);
        let render_target_view = raw_or_null(self.get_current_render_target_view());
        let blend_mode = cmd.blend;
        let mut update_subresource = false;
        let shader = self.select_shader(shader_constants);
        let context = self.context()?.clone();

        let shader_resources_changed = shader_resources.len() != self.num_current_shader_resources
            || (!shader_resources.is_empty()
                && shader_resources[0] != self.current_shader_resource);
        // FIXME (upstream): only the first resource is compared, so a
        // texture given another palette is drawn with the one bound before
        // until another texture is drawn (or the cached state is
        // invalidated).

        let shader_samplers_changed = shader_samplers.len() != self.num_current_shader_samplers
            || (!shader_samplers.is_empty() && shader_samplers[0] != self.current_shader_sampler);

        // Make sure the render target isn't bound to a shader
        if shader_resources_changed {
            context.ps_set_shader_resources(0, &[null_mut()]);
        }

        if render_target_view != self.current_render_target_view {
            context.om_set_render_targets(&[render_target_view]);
            self.current_render_target_view = render_target_view;
        }

        if self.viewport_dirty && self.update_viewport() {
            // vertexShaderConstantsData.projectionAndView has changed
            update_subresource = true;
        }

        if self.cliprect_dirty {
            if !self.current_cliprect_enabled {
                context.rs_set_scissor_rects(&[]);
            } else {
                // D3D11_GetViewportAlignedD3DRect will have set the SDL error
                let scissor_rect =
                    self.get_viewport_aligned_d3d_rect(&self.current_cliprect, true)?;
                context.rs_set_scissor_rects(&[scissor_rect]);
            }
            self.cliprect_dirty = false;
        }

        let rasterizer_state = if !self.current_cliprect_enabled {
            self.main_rasterizer.as_ref()
        } else {
            self.clipped_rasterizer.as_ref()
        };
        if raw_or_null(rasterizer_state) != self.current_rasterizer_state {
            if let Some(state) = rasterizer_state {
                context.rs_set_state(state);
            }
            self.current_rasterizer_state = raw_or_null(rasterizer_state);
        }

        let mut blend_state = None;
        if blend_mode != BlendMode::NONE {
            blend_state = self
                .blend_modes
                .iter()
                .find(|(mode, _)| *mode == blend_mode)
                .map(|(_, state)| state.clone());
            if blend_state.is_none() {
                blend_state = Some(self.create_blend_state(blend_mode)?);
            }
        }
        if raw_or_null(blend_state.as_ref()) != self.current_blend_state {
            context.om_set_blend_state(blend_state.as_ref(), 0xFFFFFFFF);
            self.current_blend_state = raw_or_null(blend_state.as_ref());
        }

        let solid_constants;
        let shader_constants = match shader_constants {
            Some(constants) => constants,
            None => {
                solid_constants = self.setup_shader_constants(cmd, None, 0.0);
                &solid_constants
            }
        };

        let shader_state = &self.current_shader_state[shader as usize];
        if shader_state.constants.is_none()
            || !shader_constants.same_bits(&shader_state.shader_constants)
        {
            self.current_shader_state[shader as usize].constants = None;

            let desc = BufferDesc {
                usage: D3D11_USAGE_DEFAULT,
                byte_width: size_of::<PixelShaderConstants>() as u32,
                bind_flags: D3D11_BIND_CONSTANT_BUFFER,
                ..Default::default()
            };

            let words = shader_constants.words();
            let data = SubresourceData {
                sys_mem: words.as_ptr().cast(),
                sys_mem_pitch: 0,
                sys_mem_slice_pitch: 0,
            };

            let constants = self
                .device()?
                .create_buffer(&desc, Some(&data))
                .map_err(|hr| {
                    hr_error("ID3D11Device::CreateBuffer [create shader constants]", hr)
                })?;
            let shader_state = &mut self.current_shader_state[shader as usize];
            shader_state.constants = Some(constants);
            shader_state.shader_constants = *shader_constants;

            // Force the shader parameters to be re-set
            self.current_shader = None;
        }
        if Some(shader) != self.current_shader {
            if self.pixel_shaders[shader as usize].is_none() {
                let pixel_shader = Self::create_pixel_shader(self.device()?, shader)?;
                self.pixel_shaders[shader as usize] = Some(pixel_shader);
            }
            if let Some(pixel_shader) = &self.pixel_shaders[shader as usize] {
                context.ps_set_shader(pixel_shader);
            }
            if let Some(constants) = &self.current_shader_state[shader as usize].constants {
                context.ps_set_constant_buffers(0, &[raw(constants)]);
            }
            self.current_shader = Some(shader);
        }
        if shader_resources_changed {
            context.ps_set_shader_resources(0, shader_resources);
            self.num_current_shader_resources = shader_resources.len();
            if let Some(&first) = shader_resources.first() {
                self.current_shader_resource = first;
            }
        }
        if shader_samplers_changed {
            context.ps_set_samplers(0, shader_samplers);
            self.num_current_shader_samplers = shader_samplers.len();
            if let Some(&first) = shader_samplers.first() {
                self.current_shader_sampler = first;
            }
        }

        if update_subresource
            || !self
                .vertex_shader_constants_data
                .model
                .same_bits(&newmatrix)
        {
            self.vertex_shader_constants_data.model = newmatrix;
            if let Some(constants) = &self.vertex_shader_constants {
                context.update_subresource(constants, &self.vertex_shader_constants_data);
            }
        }

        Ok(())
    }

    /// Translation of `D3D11_GetSamplerState()`.
    fn get_sampler_state(
        &mut self,
        format: PixelFormat,
        mut scale_mode: ScaleMode,
        address_u: TextureAddressMode,
        address_v: TextureAddressMode,
    ) -> Result<*mut c_void> {
        if format == PixelFormat::INDEX8 {
            // We'll do linear sampling in the shader if needed
            scale_mode = ScaleMode::Nearest;
        }

        let key = render_sampler_hashkey(scale_mode, address_u, address_v);
        crate::sdl_assert!(key < self.samplers.len());
        if self.samplers[key].is_none() {
            let sampler_desc = sampler_desc(scale_mode, address_u, address_v)?;
            let sampler = self
                .device()?
                .create_sampler_state(&sampler_desc)
                .map_err(|hr| hr_error("ID3D11Device::CreateSamplerState", hr))?;
            self.samplers[key] = Some(sampler);
        }
        Ok(raw_or_null(self.samplers[key].as_ref()))
    }

    /// Translation of `D3D11_SetCopyState()`.
    fn set_copy_state(
        &mut self,
        cmd: &DrawCmd,
        textures: &TextureStore,
        matrix: Option<&Float4X4>,
    ) -> Result<()> {
        let handle = cmd.texture.ok_or_else(invalid_texture)?;
        let texture = textures.get(handle).ok_or_else(invalid_texture)?;
        let source = SourceTexture::of(texture);
        let output_headroom = self.output_headroom(textures);
        let texture_data = self.texture_data(texture)?;
        let mut shader_resources = Vec::with_capacity(3);
        let mut shader_samplers = Vec::with_capacity(2);

        let constants =
            self.setup_shader_constants(cmd, Some((&source, texture_data)), output_headroom);

        shader_resources.push(raw_or_null(
            texture_data.main_texture_resource_view.as_ref(),
        ));
        let (yuv, nv12, palette) = (texture_data.yuv, texture_data.nv12, texture_data.palette);
        let planes = if yuv {
            vec![
                raw_or_null(texture_data.main_texture_resource_view_u.as_ref()),
                raw_or_null(texture_data.main_texture_resource_view_v.as_ref()),
            ]
        } else if nv12 {
            vec![raw_or_null(
                texture_data.main_texture_resource_view_nv.as_ref(),
            )]
        } else {
            Vec::new()
        };

        shader_samplers.push(self.get_sampler_state(
            source.format,
            cmd.texture_scale_mode,
            cmd.texture_address_mode_u,
            cmd.texture_address_mode_v,
        )?);

        if source.has_palette {
            let palette = palette
                .and_then(|p| self.palettes.get(&p))
                .map(|p| raw(&p.resource_view));
            shader_resources.push(palette.unwrap_or(null_mut()));

            shader_samplers.push(self.get_sampler_state(
                PixelFormat::UNKNOWN,
                ScaleMode::Nearest,
                TextureAddressMode::Clamp,
                TextureAddressMode::Clamp,
            )?);
        }

        shader_resources.extend(planes);
        self.set_draw_state(
            cmd,
            Some(&constants),
            &shader_resources,
            &shader_samplers,
            matrix,
        )
    }

    /// Translation of `D3D11_DrawPrimitives()`.
    fn draw_primitives(
        &self,
        primitive_topology: D3d11PrimitiveTopology,
        vertex_start: usize,
        vertex_count: usize,
    ) {
        if let Some(context) = &self.d3d_context {
            context.ia_set_primitive_topology(primitive_topology);
            context.draw(vertex_count as u32, vertex_start as u32);
        }
    }

    /// Translation of `D3D11_UpdateVertexBuffer()`.
    fn update_vertex_buffer(&mut self) -> Result<()> {
        let vbidx = self.current_vertex_buffer;
        let stride = size_of::<VertexPositionColor>() as u32;
        let offset = 0;
        let data_size_in_bytes = size_of_val(&self.verts[..]);
        let vertex_data = self.verts.as_ptr();

        if data_size_in_bytes == 0 {
            return Ok(()); // nothing to do.
        }

        let context = self.context()?.clone();
        match &self.vertex_buffers[vbidx] {
            Some(buffer) if self.vertex_buffer_sizes[vbidx] >= data_size_in_bytes => {
                let mapped_resource = context
                    .map(buffer, D3D11_MAP_WRITE_DISCARD)
                    .map_err(|hr| hr_error("ID3D11DeviceContext1::Map [vertex buffer]", hr))?;
                // SAFETY: the mapped buffer holds at least
                // `data_size_in_bytes`; the vertices are that many bytes.
                unsafe {
                    std::ptr::copy_nonoverlapping(
                        vertex_data.cast::<u8>(),
                        mapped_resource.data.cast::<u8>(),
                        data_size_in_bytes,
                    )
                };
                context.unmap(buffer);
            }
            _ => {
                self.vertex_buffers[vbidx] = None;

                let vertex_buffer_desc = BufferDesc {
                    byte_width: data_size_in_bytes as u32,
                    usage: D3D11_USAGE_DYNAMIC,
                    bind_flags: D3D11_BIND_VERTEX_BUFFER,
                    cpu_access_flags: D3D11_CPU_ACCESS_WRITE,
                    ..Default::default()
                };

                let vertex_buffer_data = SubresourceData {
                    sys_mem: vertex_data.cast(),
                    sys_mem_pitch: 0,
                    sys_mem_slice_pitch: 0,
                };

                let buffer = self
                    .device()?
                    .create_buffer(&vertex_buffer_desc, Some(&vertex_buffer_data))
                    .map_err(|hr| hr_error("ID3D11Device1::CreateBuffer [vertex buffer]", hr))?;
                self.vertex_buffers[vbidx] = Some(buffer);

                self.vertex_buffer_sizes[vbidx] = data_size_in_bytes;
            }
        }

        if let Some(buffer) = &self.vertex_buffers[vbidx] {
            context.ia_set_vertex_buffer(0, buffer, stride, offset);
        }

        self.current_vertex_buffer += 1;
        if self.current_vertex_buffer >= NUM_VERTEX_BUFFERS {
            self.current_vertex_buffer = 0;
        }

        Ok(())
    }

    /// Translation of `D3D11_QueueDrawPoints()` (lines and points queue
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

    /// Translation of `D3D11_CreateRenderer()` for a window.
    pub(crate) fn for_window(
        window: Window,
        output_colorspace: Colorspace,
    ) -> Result<D3d11Renderer> {
        let mut data = D3d11Renderer::new(window, output_colorspace);
        if data.hwnd().is_none() {
            return Err(Error::new("Couldn't get window handle"));
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
            let Ok(unorm) = device.check_format_support(unorm) else {
                continue;
            };
            let Ok(srgb) = device.check_format_support(srgb) else {
                continue;
            };
            if (unorm & D3D11_FORMAT_SUPPORT_TEXTURE2D) != 0
                && (srgb & D3D11_FORMAT_SUPPORT_TEXTURE2D) != 0
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
    /// and the setup of `D3D11_CreateRenderer()`).
    fn new(window: Window, output_colorspace: Colorspace) -> D3d11Renderer {
        let mut data = D3d11Renderer {
            window,
            output_colorspace,
            current_colorspace: output_colorspace,
            hdr_headroom: 1.0,
            target: None,
            dxgi_factory: None,
            dxgi_debug: None,
            d3d_device: None,
            d3d_context: None,
            swap_chain: None,
            swap_effect: 0,
            swap_chain_flags: 0,
            sync_interval: 0,
            present_flags: DXGI_PRESENT_DO_NOT_WAIT,
            main_render_target_view: None,
            current_offscreen_render_target: None,
            input_layout: None,
            vertex_buffers: Default::default(),
            vertex_buffer_sizes: [0; NUM_VERTEX_BUFFERS],
            vertex_shader: None,
            pixel_shaders: Default::default(),
            blend_modes: Vec::new(),
            samplers: Default::default(),
            feature_level: 0,
            pixel_size_changed: false,
            main_rasterizer: None,
            clipped_rasterizer: None,
            vertex_shader_constants_data: VertexShaderConstants::default(),
            vertex_shader_constants: None,
            rotation: DXGI_MODE_ROTATION_UNSPECIFIED,
            current_render_target_view: null_mut(),
            current_rasterizer_state: null_mut(),
            current_blend_state: null_mut(),
            current_shader: None,
            current_shader_state: Default::default(),
            num_current_shader_resources: 0,
            current_shader_resource: null_mut(),
            num_current_shader_samplers: 0,
            current_shader_sampler: null_mut(),
            cliprect_dirty: false,
            current_cliprect_enabled: false,
            current_cliprect: Rect::default(),
            current_viewport: Rect::default(),
            current_viewport_rotation: 0,
            viewport_dirty: false,
            identity: Float4X4::identity(),
            current_vertex_buffer: 0,
            textures: HashMap::new(),
            next_texture: 1,
            palettes: HashMap::new(),
            next_palette: 1,
            verts: Vec::new(),
            texture_formats: Vec::new(),
            props: None,
            h_d3d11_mod: None,
            h_dxgi_mod: None,
        };
        data.invalidate_cached_state();
        data
    }

    /// Translation of `D3D11_RenderReadPixels()`.
    fn render_read_pixels(&self, rect: &Rect) -> Result<Surface<'static>> {
        let Some(render_target_view) = self.get_current_render_target_view() else {
            return Err(Error::new(
                "D3D11_RenderReadPixels, ID3D11DeviceContext::OMGetRenderTargets failed",
            ));
        };

        let Some(back_buffer) = render_target_view.resource_texture() else {
            return Err(Error::new(
                "D3D11_RenderReadPixels, ID3D11View::GetResource failed",
            ));
        };

        // Create a staging texture to copy the screen's data to:
        let mut staging_texture_desc = back_buffer.desc();
        staging_texture_desc.width = rect.w as u32;
        staging_texture_desc.height = rect.h as u32;
        staging_texture_desc.bind_flags = 0;
        staging_texture_desc.misc_flags = 0;
        staging_texture_desc.cpu_access_flags = D3D11_CPU_ACCESS_READ;
        staging_texture_desc.usage = D3D11_USAGE_STAGING;
        let staging_texture = self
            .device()?
            .create_texture_2d(&staging_texture_desc)
            .map_err(|hr| {
                hr_error(
                    "ID3D11Device1::CreateTexture2D [create staging texture]",
                    hr,
                )
            })?;

        // Copy the desired portion of the back buffer to the staging texture:
        // (D3D11_GetViewportAlignedD3DRect will have set the SDL error)
        let src_rect = self.get_viewport_aligned_d3d_rect(rect, false)?;

        let src_box = D3d11Box {
            left: src_rect.left as u32,
            right: src_rect.right as u32,
            top: src_rect.top as u32,
            bottom: src_rect.bottom as u32,
            front: 0,
            back: 1,
        };
        let context = self.context()?;
        context.copy_subresource_region(&staging_texture, (0, 0), &back_buffer, Some(&src_box));

        // Map the staging texture's data to CPU-accessible memory:
        let texture_memory = context
            .map(&staging_texture, D3D11_MAP_READ)
            .map_err(|hr| hr_error("ID3D11DeviceContext1::Map [map staging texture]", hr))?;

        let format = dxgi_format_to_sdl_pixel_format(staging_texture_desc.format);
        let row = rect.w.max(0) as usize * format.bytes_per_pixel() as usize;
        let len = match rect.h.max(0) as usize {
            0 => 0,
            h => (h - 1) * texture_memory.row_pitch as usize + row,
        };
        // SAFETY: the mapped staging texture has `h` rows of `row` bytes,
        // RowPitch apart, until it's unmapped below.
        let pixels = unsafe { std::slice::from_raw_parts(texture_memory.data.cast::<u8>(), len) };
        let output = crate::video::surface::duplicate_pixels(
            rect.w,
            rect.h,
            format,
            self.current_colorspace,
            Some(pixels),
            texture_memory.row_pitch as i32,
        );

        // Unmap the texture:
        context.unmap(&staging_texture);

        output
    }

    /// Translation of `D3D11_RenderPresent()` (the error with it).
    fn render_present(&mut self) -> Result<()> {
        let Some(swap_chain) = self
            .swap_chain
            .clone()
            .filter(|_| self.d3d_device.is_some())
        else {
            return Err(device_lost());
        };

        let parameters = PresentParameters::default();

        /* The application may optionally specify "dirty" or "scroll"
         * rects to improve efficiency in certain scenarios.
         */
        let result = swap_chain.present1(self.sync_interval, self.present_flags, &parameters);
        drop(swap_chain);

        // When the present flips, it unbinds the current view, so bind it again on the next draw call
        self.current_render_target_view = null_mut();

        if result < 0 && result != DXGI_ERROR_WAS_STILL_DRAWING {
            /* If the device was removed either by a disconnect or a driver upgrade, we
             * must recreate all device resources.
             */
            if result == DXGI_ERROR_DEVICE_REMOVED {
                if self.handle_device_lost() {
                    return Err(Error::new("Present failed, device lost"));
                } else {
                    // Recovering from device lost failed, error is already set
                    return Err(device_lost());
                }
            } else if result == DXGI_ERROR_INVALID_CALL {
                // We probably went through a fullscreen <-> windowed transition
                let _ = self.create_window_size_dependent_resources();
                return Err(hr_error("IDXGISwapChain::Present", result));
            } else {
                return Err(hr_error("IDXGISwapChain::Present", result));
            }
        }
        Ok(())
    }
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

/// The sampler description of `D3D11_GetSamplerState()`.
fn sampler_desc(
    scale_mode: ScaleMode,
    address_u: TextureAddressMode,
    address_v: TextureAddressMode,
) -> Result<SamplerDesc> {
    let address_mode = |mode: TextureAddressMode| match mode {
        TextureAddressMode::Clamp => Ok(D3D11_TEXTURE_ADDRESS_CLAMP),
        TextureAddressMode::Wrap => Ok(D3D11_TEXTURE_ADDRESS_WRAP),
        other => Err(Error::new(format!(
            "Unknown texture address mode: {}",
            other as i32
        ))),
    };
    Ok(SamplerDesc {
        address_w: D3D11_TEXTURE_ADDRESS_CLAMP,
        mip_lod_bias: 0.0,
        max_anisotropy: 1,
        comparison_func: D3D11_COMPARISON_ALWAYS,
        min_lod: 0.0,
        max_lod: D3D11_FLOAT32_MAX,
        filter: match scale_mode {
            ScaleMode::Nearest => D3D11_FILTER_MIN_MAG_MIP_POINT,
            // (pixel art uses linear sampling)
            ScaleMode::PixelArt | ScaleMode::Linear => D3D11_FILTER_MIN_MAG_MIP_LINEAR,
        },
        address_u: address_mode(address_u)?,
        address_v: address_mode(address_v)?,
        ..Default::default()
    })
}

impl RenderBackend for D3d11Renderer {
    fn name(&self) -> &'static str {
        D3D11_RENDERER
    }

    fn output_size(&self, _textures: &TextureStore) -> Option<Result<(i32, i32)>> {
        None // (the window's size in pixels)
    }

    fn texture_formats(&self) -> Option<Vec<PixelFormat>> {
        Some(self.texture_formats.clone())
    }

    /// `SDL_PROP_RENDERER_MAX_TEXTURE_SIZE_NUMBER`, set in
    /// `D3D11_CreateDeviceResources()`.
    fn max_texture_size(&self) -> Option<i32> {
        Some(16384)
    }

    fn set_properties(&mut self, props: &Properties) {
        self.props = Some(props.clone());
        self.publish_properties();
    }

    /// Translation of `D3D11_SupportsBlendMode()`.
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

    /// Translation of `D3D11_CreateTexture()`.
    fn create_texture(
        &mut self,
        texture: &mut TextureData,
        _props: &TextureCreateProps,
    ) -> Result<()> {
        use PixelFormat as F;
        let texture_format =
            sdl_pixel_format_to_dxgi_texture_format(texture.format, self.output_colorspace);
        let device = self.device()?.clone();

        if texture_format == DXGI_FORMAT_UNKNOWN {
            return Err(Error::new(format!(
                "D3D11_CreateTexture, An unsupported SDL pixel format (0x{:x}) was specified",
                texture.format.0
            )));
        }

        let mut texture_data = D3d11Texture::default();

        let mut texture_desc = Texture2dDesc {
            width: texture.w as u32,
            height: texture.h as u32,
            mip_levels: 1,
            array_size: 1,
            format: texture_format,
            sample_desc: SampleDesc {
                count: 1,
                quality: 0,
            },
            misc_flags: 0,
            ..Default::default()
        };

        // NV12 textures must have even width and height
        if matches!(texture.format, F::NV12 | F::NV21 | F::P010) {
            texture_desc.width = (texture_desc.width + 1) & !1;
            texture_desc.height = (texture_desc.height + 1) & !1;
        }
        texture_data.w = texture_desc.width as i32;
        texture_data.h = texture_desc.height as i32;

        if texture.access == TextureAccess::Streaming {
            texture_desc.usage = D3D11_USAGE_DYNAMIC;
            texture_desc.cpu_access_flags = D3D11_CPU_ACCESS_WRITE;
        } else {
            texture_desc.usage = D3D11_USAGE_DEFAULT;
            texture_desc.cpu_access_flags = 0;
        }

        if texture.access == TextureAccess::Target {
            texture_desc.bind_flags = D3D11_BIND_SHADER_RESOURCE | D3D11_BIND_RENDER_TARGET;
        } else {
            texture_desc.bind_flags = D3D11_BIND_SHADER_RESOURCE;
        }

        let create = |desc: &Texture2dDesc| {
            device
                .create_texture_2d(desc)
                .map_err(|hr| hr_error("ID3D11Device1::CreateTexture2D", hr))
        };

        // (SDL_PROP_TEXTURE_CREATE_D3D11_TEXTURE_POINTER isn't taken)
        let main_texture = create(&texture_desc)?;
        let props = texture.props.get_or_insert_with(Properties::new).clone();
        let _ = props.set(
            PROP_TEXTURE_D3D11_TEXTURE_POINTER,
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

            let main_texture_u = create(&texture_desc)?;
            let _ = props.set(
                PROP_TEXTURE_D3D11_TEXTURE_U_POINTER,
                raw(&main_texture_u) as i64,
            );
            texture_data.main_texture_u = Some(main_texture_u);

            let main_texture_v = create(&texture_desc)?;
            let _ = props.set(
                PROP_TEXTURE_D3D11_TEXTURE_V_POINTER,
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
        let resource_view_desc = ShaderResourceViewDesc::texture_2d(
            sdl_pixel_format_to_dxgi_main_resource_view_format(
                texture.format,
                self.output_colorspace,
            ),
            0,
            texture_desc.mip_levels,
        );
        let create_view = |resource: &Option<Texture2d>, desc: &ShaderResourceViewDesc| {
            let resource = resource.as_ref().ok_or_else(not_available)?;
            device
                .create_shader_resource_view(resource, desc)
                .map_err(|hr| hr_error("ID3D11Device1::CreateShaderResourceView", hr))
        };
        texture_data.main_texture_resource_view = Some(create_view(
            &texture_data.main_texture,
            &resource_view_desc,
        )?);

        if texture_data.yuv {
            texture_data.main_texture_resource_view_u = Some(create_view(
                &texture_data.main_texture_u,
                &resource_view_desc,
            )?);
            texture_data.main_texture_resource_view_v = Some(create_view(
                &texture_data.main_texture_v,
                &resource_view_desc,
            )?);
        }

        if texture_data.nv12 {
            let mut nv_resource_view_desc = resource_view_desc;

            if texture.format == F::NV12 || texture.format == F::NV21 {
                nv_resource_view_desc.format = DXGI_FORMAT_R8G8_UNORM;
            } else if texture.format == F::P010 {
                nv_resource_view_desc.format = DXGI_FORMAT_R16G16_UNORM;
            }

            texture_data.main_texture_resource_view_nv = Some(create_view(
                &texture_data.main_texture,
                &nv_resource_view_desc,
            )?);
        }

        if texture.access == TextureAccess::Target {
            let render_target_view_desc = RenderTargetViewDesc::texture_2d(texture_desc.format, 0);

            let main_texture = texture_data
                .main_texture
                .as_ref()
                .ok_or_else(not_available)?;
            texture_data.main_texture_render_target_view = Some(
                device
                    .create_render_target_view(main_texture, Some(&render_target_view_desc))
                    .map_err(|hr| hr_error("ID3D11Device1::CreateRenderTargetView", hr))?,
            );
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

    /// Translation of `D3D11_QueueDrawPoints()`.
    fn queue_draw_points(&mut self, cmd: &mut DrawCmd, points: &[FPoint]) -> Result<()> {
        self.queue_points(cmd, points);
        Ok(())
    }

    /// `D3D11_QueueDrawPoints()`: lines and points queue vertices the same way.
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

    /// Translation of `D3D11_QueueGeometry()`.
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

    /// Translation of `D3D11_InvalidateCachedState()`.
    fn invalidate_cached_state(&mut self) {
        self.current_render_target_view = null_mut();
        self.current_rasterizer_state = null_mut();
        self.current_blend_state = null_mut();
        self.current_shader = None;
        self.num_current_shader_resources = 0;
        self.current_shader_resource = null_mut();
        self.num_current_shader_samplers = 0;
        self.current_shader_sampler = null_mut();
        self.cliprect_dirty = true;
        self.viewport_dirty = true;
    }

    /// Translation of `D3D11_RunCommandQueue()`.
    fn run_command_queue(
        &mut self,
        cmds: &[RenderCommand],
        textures: &mut TextureStore,
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
                    if self.current_cliprect_enabled != enabled {
                        self.current_cliprect_enabled = enabled;
                        self.cliprect_dirty = true;
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

                    if let (Some(context), Some(view)) =
                        (&self.d3d_context, self.get_current_render_target_view())
                    {
                        context
                            .clear_render_target_view(view, &[color.r, color.g, color.b, color.a]);
                    }
                }

                RenderCommand::Draw(DrawKind::Lines, d) if d.count > 0 => {
                    let mut count = d.count;
                    let start = d.first;

                    let _ = self.set_draw_state(&d, None, &[], &[], None);

                    // Add the final point in the line
                    let mut line_start = 0;
                    let mut line_end = line_start + count - 1;
                    let verts = &self.verts[start..];
                    if verts[line_start].pos != verts[line_end].pos {
                        self.draw_primitives(
                            D3D11_PRIMITIVE_TOPOLOGY_POINTLIST,
                            start + line_end,
                            1,
                        );
                    }

                    if count > 2 {
                        // joined lines cannot be grouped
                        self.draw_primitives(D3D11_PRIMITIVE_TOPOLOGY_LINESTRIP, start, count);
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
                                            self.draw_primitives(
                                                D3D11_PRIMITIVE_TOPOLOGY_POINTLIST,
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

                        self.draw_primitives(D3D11_PRIMITIVE_TOPOLOGY_LINELIST, start, count);
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

                    if d.texture.is_some() {
                        let _ = self.set_copy_state(&d, textures, None);
                    } else {
                        let _ = self.set_draw_state(&d, None, &[], &[], None);
                    }

                    if thiscmdtype == DrawKind::Geometry {
                        self.draw_primitives(D3D11_PRIMITIVE_TOPOLOGY_TRIANGLELIST, start, count);
                    } else {
                        self.draw_primitives(D3D11_PRIMITIVE_TOPOLOGY_POINTLIST, start, count);
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

    /// Translation of `D3D11_CreatePalette()`.
    fn create_palette(&mut self) -> Result<Box<dyn Any>> {
        let device = self.device()?;

        let texture_desc = Texture2dDesc {
            width: 256,
            height: 1,
            mip_levels: 1,
            array_size: 1,
            format: sdl_pixel_format_to_dxgi_texture_format(
                PixelFormat::RGBA32,
                self.output_colorspace,
            ),
            sample_desc: SampleDesc {
                count: 1,
                quality: 0,
            },
            misc_flags: 0,
            usage: D3D11_USAGE_DEFAULT,
            cpu_access_flags: 0,
            bind_flags: D3D11_BIND_SHADER_RESOURCE,
        };

        let texture = device
            .create_texture_2d(&texture_desc)
            .map_err(|hr| hr_error("ID3D11Device1::CreateTexture2D", hr))?;

        let resource_view_desc = ShaderResourceViewDesc::texture_2d(
            sdl_pixel_format_to_dxgi_main_resource_view_format(
                PixelFormat::RGBA32,
                self.output_colorspace,
            ),
            0,
            texture_desc.mip_levels,
        );
        let resource_view = device
            .create_shader_resource_view(&texture, &resource_view_desc)
            .map_err(|hr| hr_error("ID3D11Device1::CreateShaderResourceView", hr))?;

        let id = self.next_palette;
        self.next_palette += 1;
        self.palettes.insert(
            id,
            PaletteData {
                texture,
                resource_view,
            },
        );
        Ok(Box::new(PaletteRef(id)))
    }

    /// Translation of `D3D11_UpdatePalette()`.
    fn update_palette(&mut self, palette: &mut dyn Any, colors: &[Color]) -> Result<()> {
        let id = palette
            .downcast_ref::<PaletteRef>()
            .map(|p| p.0)
            .ok_or_else(|| Error::invalid_param("palette"))?;
        let palettedata = self
            .palettes
            .get(&id)
            .ok_or_else(|| Error::invalid_param("palette"))?;

        let bytes: Vec<u8> = colors.iter().flat_map(|c| [c.r, c.g, c.b, c.a]).collect();
        self.update_texture_internal(
            &palettedata.texture,
            4,
            (0, 0, colors.len() as i32, 1),
            &bytes,
            bytes.len() as i32,
        )
    }

    /// Translation of `D3D11_DestroyPalette()`.
    fn destroy_palette(&mut self, palette: Box<dyn Any>) {
        if let Ok(palette) = palette.downcast::<PaletteRef>() {
            self.palettes.remove(&palette.0);
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

    /// Translation of `D3D11_LockTexture()`.
    fn lock_texture(&mut self, texture: &mut TextureData, rect: &Rect) -> Result<(usize, i32)> {
        let id = Self::texture_id(texture)?;
        let texture_data = self.textures.get(&id).ok_or_else(not_available)?;
        let bpp = texture.format.bytes_per_pixel() as usize;

        if texture_data.yuv || texture_data.nv12 {
            // It's more efficient to upload directly...
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
        if texture_data.staging_texture.is_some() {
            return Err(Error::new("texture is already locked"));
        }
        let main_texture = texture_data
            .main_texture
            .as_ref()
            .ok_or_else(not_available)?;

        /* Create a 'staging' texture, which will be used to write to a portion
         * of the main texture.  This is necessary, as Direct3D 11.1 does not
         * have the ability to write a CPU-bound pixel buffer to a rectangular
         * subrect of a texture.  Direct3D 11.1 can, however, write a pixel
         * buffer to an entire texture, hence the use of a staging texture.
         */
        let mut staging_texture_desc = main_texture.desc();
        staging_texture_desc.width = rect.w as u32;
        staging_texture_desc.height = rect.h as u32;
        staging_texture_desc.bind_flags = 0;
        staging_texture_desc.misc_flags = 0;
        staging_texture_desc.cpu_access_flags = D3D11_CPU_ACCESS_WRITE;
        staging_texture_desc.usage = D3D11_USAGE_STAGING;
        let staging_texture = self
            .device()?
            .create_texture_2d(&staging_texture_desc)
            .map_err(|hr| {
                hr_error(
                    "ID3D11Device1::CreateTexture2D [create staging texture]",
                    hr,
                )
            })?;

        // Get a write-only pointer to data in the staging texture:
        let texture_memory = self
            .context()?
            .map(&staging_texture, D3D11_MAP_WRITE)
            .map_err(|hr| hr_error("ID3D11DeviceContext1::Map [map staging texture]", hr))?;

        /* Make note of where the staging texture will be written to
         * (on a call to SDL_UnlockTexture):
         */
        let texture_data = self.textures.get_mut(&id).ok_or_else(not_available)?;
        texture_data.staging_texture = Some(staging_texture);
        texture_data.locked_texture_position_x = rect.x;
        texture_data.locked_texture_position_y = rect.y;

        /* Make sure the caller has information on the texture's pixel buffer,
         * then return:
         */
        let row = rect.w.max(0) as usize * bpp;
        let len = match rect.h.max(0) as usize {
            0 => 0,
            h => (h - 1) * texture_memory.row_pitch as usize + row,
        };
        let staging = NonNull::new(texture_memory.data.cast::<u8>()).map(|p| (p, len));
        if let Some(tref) = texture_ref_mut(texture) {
            tref.staging = staging;
        }
        Ok((0, texture_memory.row_pitch as i32))
    }

    fn texture_pixels_mut<'t>(&mut self, texture: &'t mut TextureData) -> Option<&'t mut [u8]> {
        let tref = texture_ref_mut(texture)?;
        match tref.staging {
            // SAFETY: the mapped staging texture of the lock, which stays
            // mapped until the texture is unlocked or destroyed; both take
            // the texture mutably, which the returned borrow prevents.
            Some((ptr, len)) => Some(unsafe { std::slice::from_raw_parts_mut(ptr.as_ptr(), len) }),
            None => Some(&mut tref.pixels[..]),
        }
    }

    /// Translation of `D3D11_UnlockTexture()`.
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
        let Some(staging_texture) = texture_data.staging_texture.take() else {
            return;
        };
        let (Some(context), Some(main_texture)) = (&self.d3d_context, &texture_data.main_texture)
        else {
            return;
        };

        // Commit the pixel buffer's changes back to the staging texture:
        context.unmap(&staging_texture);

        // Copy the staging texture's contents back to the main texture:
        context.copy_subresource_region(
            main_texture,
            (
                texture_data.locked_texture_position_x as u32,
                texture_data.locked_texture_position_y as u32,
            ),
            &staging_texture,
            None,
        );
    }

    /// Translation of `D3D11_SetRenderTarget()`.
    fn set_render_target(
        &mut self,
        target: Option<Texture>,
        textures: &TextureStore,
    ) -> Result<()> {
        let Some(t) = target else {
            self.current_offscreen_render_target = None;
            self.target = None;
            self.current_colorspace = self.output_colorspace;
            return Ok(());
        };

        let texture = textures.get(t).ok_or_else(invalid_texture)?;
        // Note (upstream): a texture whose data went with a lost device is
        // dereferenced there.
        let id = Self::texture_id(texture)?;
        let texture_data = self.textures.get(&id).ok_or_else(not_available)?;

        if texture_data.main_texture_render_target_view.is_none() {
            return Err(Error::new("specified texture is not a render target"));
        }

        self.current_offscreen_render_target = Some(id);
        self.target = Some(t);
        self.current_colorspace = texture.colorspace;

        Ok(())
    }

    /// Translation of `D3D11_RenderReadPixels()`.
    fn read_pixels(
        &mut self,
        rect: &Rect,
        _textures: &mut TextureStore,
    ) -> Option<Result<Surface<'static>>> {
        Some(self.render_read_pixels(rect))
    }

    /// Translation of `D3D11_RenderPresent()`.
    fn present(&mut self) -> bool {
        // (SDL_RenderPresent() goes on without the error)
        self.render_present().is_ok()
    }

    /// Translation of `D3D11_DestroyTexture()`.
    fn destroy_texture(&mut self, texture: &mut TextureData) {
        let Some(internal) = texture.internal.take() else {
            return;
        };
        let Ok(tref) = internal.downcast::<TextureRef>() else {
            return;
        };
        self.destroy_texture_data(tref.id);
    }

    /// Translation of `D3D11_SetVSync()`.
    fn set_vsync(&mut self, vsync: i32) -> Option<Result<()>> {
        if vsync < 0 {
            return Some(Err(Error::unsupported()));
        }

        if vsync > 0 {
            self.sync_interval = vsync as u32;
        } else {
            self.sync_interval = 0;
        }
        self.update_present_flags();
        Some(Ok(()))
    }

    /// Translation of `D3D11_WindowEvent()`.
    fn window_event(&mut self, event_type: EventType) {
        if event_type == EventType::WINDOW_PIXEL_SIZE_CHANGED {
            self.pixel_size_changed = true;
        }
    }

    /// Translation of `D3D11_DestroyRenderer()`.
    fn destroy(&mut self) {
        // (the front end destroys the palettes first; any left go before
        // the libraries are unloaded)
        self.palettes.clear();
        self.release_all();
    }
}
