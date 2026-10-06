// Rust translation of src/render/gpu/SDL_render_gpu.c, SDL_gpu_util.h and the
// device parts of SDL_shaders_gpu.c from Simple DirectMedia Layer, with the
// parts of src/render/SDL_d3dmath.h it uses.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! The GPU renderer ("gpu"): draws through the GPU API ([`crate::gpu`]),
//! into a backbuffer texture that presenting blits to the window's
//! swapchain. It runs on the GPU API's Vulkan backend with its SPIR-V
//! shaders, and on Windows on the Direct3D 12 backend with its DXIL
//! shaders (shader model 6.0: a Direct3D 12 device without it can't take
//! them, and then the Vulkan backend is tried).
//!
//! What isn't translated:
//!
//! * render states with custom fragment shaders (`SDL_GPURenderState`,
//!   `FRAG_SHADER_TEXTURE_CUSTOM`), which the front end doesn't take yet;
//! * the creation options for an existing GPU device or the application's
//!   shader formats (`SDL_PROP_RENDERER_CREATE_GPU_*`) and for existing
//!   textures (`SDL_PROP_TEXTURE_CREATE_GPU_TEXTURE_*`), which the front
//!   end doesn't take yet either: the renderer makes and owns them all;
//! * the GDK suspend and resume functions (there is no GDK platform layer).
//!
//! The front end only makes renderers with sRGB output yet, so the linear
//! output path here is kept for when it does.
//!
//! Upstream keeps a texture's data in `texture->internal` and frees it in
//! `GPU_DestroyTexture()`; here it's a [`GpuTextureData`] in the front
//! end's texture, whose GPU textures are released when it is dropped. The
//! GPU textures are shared (`Arc`) with the texture's properties.

mod pipeline;
mod shaders;
#[cfg(test)]
mod tests;

use std::any::Any;
use std::sync::Arc;

use pipeline::{PipelineCache, PipelineParameters};
use shaders::{FragmentShaderId, ShaderSources, VertexShaderId};

use crate::error::{Error, Result};
use crate::events::window::WindowFlags;
use crate::events::EventType;
use crate::gpu::{self, LoadOp, PrimitiveType, ShaderFormat, TextureFormat, TextureUsageFlags};
use crate::hints;
use crate::log::Category;
use crate::properties::{Properties, Value};
use crate::render::sysrender::{
    CopyEx, DrawCmd, DrawKind, Geometry, RenderBackend, RenderCommand, TextureCreateProps,
    TextureData, TextureStore,
};
use crate::render::{Texture, TextureAccess, TextureAddressMode, GPU_RENDERER};
use crate::video::blendmode::{BlendFactor, BlendOperation};
use crate::video::pixels::{
    convert_color_709_to_2020, srgb_to_linear, Color, Colorspace, FColor, PixelFormat,
    TransferCharacteristics,
};
use crate::video::rect::{FPoint, FRect, Rect};
use crate::video::surface::{ScaleMode, Surface};
use crate::video::{BlendMode, FlipMode, Window};

/// The GPU device of the renderer (a [`gpu::Device`]). Translation of
/// `SDL_PROP_RENDERER_GPU_DEVICE_POINTER`.
pub const PROP_RENDERER_GPU_DEVICE_POINTER: &str = "SDL.renderer.gpu.device";
/// The GPU texture of a texture (a [`gpu::Texture`]). Translation of
/// `SDL_PROP_TEXTURE_GPU_TEXTURE_POINTER`.
pub const PROP_TEXTURE_GPU_TEXTURE_POINTER: &str = "SDL.texture.gpu.texture";
/// The GPU texture of an NV12-style texture's UV plane. Translation of
/// `SDL_PROP_TEXTURE_GPU_TEXTURE_UV_POINTER`.
pub const PROP_TEXTURE_GPU_TEXTURE_UV_POINTER: &str = "SDL.texture.gpu.texture_uv";
/// The GPU texture of a YUV texture's U plane. Translation of
/// `SDL_PROP_TEXTURE_GPU_TEXTURE_U_POINTER`.
pub const PROP_TEXTURE_GPU_TEXTURE_U_POINTER: &str = "SDL.texture.gpu.texture_u";
/// The GPU texture of a YUV texture's V plane. Translation of
/// `SDL_PROP_TEXTURE_GPU_TEXTURE_V_POINTER`.
pub const PROP_TEXTURE_GPU_TEXTURE_V_POINTER: &str = "SDL.texture.gpu.texture_v";

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

// SDL_gpu_util.h

/// Translation of `GPU_ConvertBlendFactor()`: `None` (upstream's
/// `SDL_GPU_BLENDFACTOR_INVALID`) for a factor the GPU API doesn't have.
fn convert_blend_factor(factor: Option<BlendFactor>) -> Option<gpu::BlendFactor> {
    use gpu::BlendFactor as G;
    Some(match factor? {
        BlendFactor::Zero => G::Zero,
        BlendFactor::One => G::One,
        BlendFactor::SrcColor => G::SrcColor,
        BlendFactor::OneMinusSrcColor => G::OneMinusSrcColor,
        BlendFactor::SrcAlpha => G::SrcAlpha,
        BlendFactor::OneMinusSrcAlpha => G::OneMinusSrcAlpha,
        BlendFactor::DstColor => G::DstColor,
        BlendFactor::OneMinusDstColor => G::OneMinusDstColor,
        BlendFactor::DstAlpha => G::DstAlpha,
        BlendFactor::OneMinusDstAlpha => G::OneMinusDstAlpha,
    })
}

/// Translation of `GPU_ConvertBlendOperation()`: `None` (upstream's
/// `SDL_GPU_BLENDOP_INVALID`) for an operation the GPU API doesn't have.
fn convert_blend_operation(operation: Option<BlendOperation>) -> Option<gpu::BlendOp> {
    use gpu::BlendOp as G;
    Some(match operation? {
        BlendOperation::Add => G::Add,
        BlendOperation::Subtract => G::Subtract,
        BlendOperation::RevSubtract => G::ReverseSubtract,
        BlendOperation::Minimum => G::Min,
        BlendOperation::Maximum => G::Max,
    })
}

// SDL_render_gpu.c

/// A 4x4 matrix of floats. Translation of `Float4X4` (from `SDL_d3dmath.h`).
#[derive(Clone, Copy, Debug, Default)]
struct Float4X4 {
    m: [[f32; 4]; 4],
}

/// Translation of `GPU_VertexShaderUniformData`.
#[derive(Clone, Copy, Debug, Default)]
struct VertexShaderUniformData {
    mvp: Float4X4,
}

impl VertexShaderUniformData {
    /// The uniform buffer's bytes, as the C structure lays them out.
    fn bytes(&self) -> Vec<u8> {
        float_bytes(self.mvp.m.as_flattened())
    }
}

/// Translation of `GPU_SimpleFragmentShaderUniformData`.
#[derive(Clone, Copy, Debug, Default)]
struct SimpleFragmentShaderUniformData {
    color_scale: f32,
}

impl SimpleFragmentShaderUniformData {
    fn bytes(&self) -> Vec<u8> {
        float_bytes(&[self.color_scale])
    }
}

/// Translation of `GPU_AdvancedFragmentShaderUniformData`.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
struct AdvancedFragmentShaderUniformData {
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

impl AdvancedFragmentShaderUniformData {
    /// The uniform buffer's bytes, as the C structure lays them out (the
    /// `cbuffer Constants` of `texture_advanced.frag.hlsl`).
    fn bytes(&self) -> Vec<u8> {
        let mut floats = vec![
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
        floats.extend_from_slice(&self.ycbcr_matrix);
        float_bytes(&floats)
    }
}

/// Floats as the bytes of a uniform buffer.
fn float_bytes(floats: &[f32]) -> Vec<u8> {
    floats.iter().flat_map(|f| f.to_ne_bytes()).collect()
}

// These should mirror the definitions in shaders/texture_advanced.frag.hlsl
const TONEMAP_NONE: f32 = 0.0;
//const TONEMAP_LINEAR: f32 = 1.0;
const TONEMAP_CHROME: f32 = 2.0;

//const TEXTURETYPE_NONE: f32 = 0.0;
const TEXTURETYPE_RGB: f32 = 1.0;
const TEXTURETYPE_RGB_PIXELART: f32 = 2.0;
const TEXTURETYPE_RGBA: f32 = 3.0;
const TEXTURETYPE_RGBA_PIXELART: f32 = 4.0;
const TEXTURETYPE_PALETTE_NEAREST: f32 = 5.0;
const TEXTURETYPE_PALETTE_LINEAR: f32 = 6.0;
const TEXTURETYPE_PALETTE_PIXELART: f32 = 7.0;
const TEXTURETYPE_NV12: f32 = 8.0;
const TEXTURETYPE_NV21: f32 = 9.0;
const TEXTURETYPE_YUV: f32 = 10.0;

const INPUTTYPE_UNSPECIFIED: f32 = 0.0;
const INPUTTYPE_SRGB: f32 = 1.0;
const INPUTTYPE_SCRGB: f32 = 2.0;
const INPUTTYPE_HDR10: f32 = 3.0;

/// The texture the renderer draws into when there is no target
/// (`GPU_RenderData::backbuffer`).
struct Backbuffer {
    texture: Arc<gpu::Texture>,
    format: TextureFormat,
    width: u32,
    height: u32,
}

/// `GPU_RenderData::swapchain`.
#[derive(Clone, Copy, Debug)]
struct SwapchainState {
    composition: gpu::SwapchainComposition,
    present_mode: gpu::PresentMode,
}

/// `GPU_RenderData::vertices`.
struct VertexBuffer {
    transfer_buf: gpu::TransferBuffer,
    buffer: gpu::Buffer,
    buffer_size: u32,
}

/// `GPU_RenderData::state`. The render pass and the color attachment's
/// texture live in [`GpuRenderer::run_command_queue`], which has the
/// command buffer while it runs: a render pass borrows its command buffer.
#[derive(Debug)]
struct RenderState {
    /// The front end's render target (`render_target`).
    render_target: Option<Texture>,
    command_buffer: Option<gpu::CommandBuffer>,
    /// `color_attachment.clear_color`
    clear_color: FColor,
    /// `color_attachment.load_op`
    load_op: LoadOp,
    viewport: gpu::Viewport,
    scissor: Rect,
    scissor_enabled: bool,
    scissor_was_enabled: bool,
}

/// Per-palette data. Translation of `GPU_PaletteData`.
struct PaletteData {
    texture: Arc<gpu::Texture>,
}

/// The extra planes of a YUV texture.
enum Planes {
    /// An RGB or indexed texture: one plane.
    Single,
    /// YV12 texture support (`yuv`, `textureU`, `textureV`)
    Yuv {
        u: Arc<gpu::Texture>,
        v: Arc<gpu::Texture>,
    },
    /// NV12 texture support (`nv12`, `textureNV`)
    Nv(Arc<gpu::Texture>),
}

/// Per-texture data. Translation of `GPU_TextureData`.
struct GpuTextureData {
    texture: Arc<gpu::Texture>,
    format: TextureFormat,
    /// The pixels of a streaming texture.
    pixels: Vec<u8>,
    pitch: i32,
    locked_rect: Rect,
    ycbcr_matrix: Option<[f32; 16]>,
    planes: Planes,
    /// The texture of the texture's palette (`texture->palette->internal`),
    /// as the front end last changed it.
    palette: Option<Arc<gpu::Texture>>,
}

fn texture_data(texture: &TextureData) -> Result<&GpuTextureData> {
    texture
        .internal
        .as_ref()
        .and_then(|d| d.downcast_ref::<GpuTextureData>())
        .ok_or_else(not_available)
}

fn texture_data_mut(texture: &mut TextureData) -> Result<&mut GpuTextureData> {
    texture
        .internal
        .as_mut()
        .and_then(|d| d.downcast_mut::<GpuTextureData>())
        .ok_or_else(not_available)
}

/// The error of a texture without renderer data.
fn not_available() -> Error {
    Error::new("Texture is not currently available")
}

/// The error of a call while the renderer has no command buffer (one
/// couldn't be acquired after the last submission).
fn no_command_buffer() -> Error {
    Error::invalid_param("command_buffer")
}

/// The bytes of `pixels` from `offset` on (an error past the end: the C
/// code trusts the caller to pass all the planes).
fn plane(pixels: &[u8], offset: usize) -> Result<&[u8]> {
    pixels
        .get(offset..)
        .ok_or_else(|| Error::invalid_param("pixels"))
}

// TODO: Sort this list based on what the GPU driver prefers?
/// Translation of `supported_formats`.
const SUPPORTED_FORMATS: [PixelFormat; 9] = [
    PixelFormat::BGRA32, // SDL_PIXELFORMAT_ARGB8888 on little endian systems
    PixelFormat::RGBA32,
    PixelFormat::BGRX32,
    PixelFormat::RGBX32,
    PixelFormat::ABGR2101010,
    PixelFormat::RGBA64_FLOAT,
    PixelFormat::RGB565,
    PixelFormat::ARGB1555,
    PixelFormat::ARGB4444,
];

// SDL_shaders_gpu.c (the device parts; the tables are in shaders.rs)

/// Whether the renderer has shaders for each format (upstream's
/// `HAVE_*_SHADERS`, for the GPU backends compiled in: Vulkan, and
/// Direct3D 12 on Windows).
const HAVE_PRIVATE_SHADERS: bool = false;
const HAVE_SPIRV_SHADERS: bool = true;
const HAVE_DXIL60_SHADERS: bool = cfg!(windows);
const HAVE_METAL_SHADERS: bool = false;

/// The renderer's shaders. Translation of `GPU_Shaders`; dropping it is
/// `GPU_ReleaseShaders()`.
struct Shaders {
    vert_shaders: Vec<gpu::Shader>,
    frag_shaders: Vec<gpu::Shader>,
}

/// Translation of `CompileShader()`.
fn compile_shader(
    sources: &ShaderSources,
    device: &gpu::Device,
    stage: gpu::ShaderStage,
) -> Result<gpu::Shader> {
    let formats = device.shader_formats();

    // (SDL_GetGPUShaderFormats() can't fail: the device exists. There are
    // no private or MSL shaders.)
    let (code, format) = if HAVE_SPIRV_SHADERS && formats.contains(ShaderFormat::SPIRV) {
        (sources.spirv, ShaderFormat::SPIRV)
    } else if HAVE_DXIL60_SHADERS && formats.contains(ShaderFormat::DXIL) {
        (dxil60(sources), ShaderFormat::DXIL)
    } else {
        return Err(Error::new("Unsupported GPU backend"));
    };

    let sci = gpu::ShaderCreateInfo {
        code,
        format,
        // FIXME not sure if this is correct
        // (the MSL shaders' entry point would be "main0")
        entrypoint: "main",
        num_samplers: sources.num_samplers,
        num_uniform_buffers: sources.num_uniform_buffers,
        stage,
        ..Default::default()
    };

    device.create_shader(&sci)
}

/// The DXIL of `sources` (`sources->dxil60`; off Windows there is no
/// Direct3D 12 backend, and no DXIL).
#[cfg(windows)]
fn dxil60(sources: &ShaderSources) -> &'static [u8] {
    sources.dxil60
}

#[cfg(not(windows))]
fn dxil60(_sources: &ShaderSources) -> &'static [u8] {
    &[]
}

impl Shaders {
    /// Translation of `GPU_InitShaders()`.
    fn new(device: &gpu::Device) -> Result<Shaders> {
        let vert_shaders = VertexShaderId::ALL
            .iter()
            .map(|id| compile_shader(id.sources(), device, gpu::ShaderStage::Vertex))
            .collect::<Result<Vec<_>>>()?;
        // (FRAG_SHADER_TEXTURE_CUSTOM is not one of them)
        let frag_shaders = FragmentShaderId::ALL
            .iter()
            .map(|id| compile_shader(id.sources(), device, gpu::ShaderStage::Fragment))
            .collect::<Result<Vec<_>>>()?;
        Ok(Shaders {
            vert_shaders,
            frag_shaders,
        })
    }

    /// Translation of `GPU_GetVertexShader()`.
    fn vertex_shader(&self, id: VertexShaderId) -> &gpu::Shader {
        &self.vert_shaders[id as usize]
    }

    /// Translation of `GPU_GetFragmentShader()`.
    fn fragment_shader(&self, id: FragmentShaderId) -> &gpu::Shader {
        &self.frag_shaders[id as usize]
    }
}

/// Translation of `GPU_FillSupportedShaderFormats()`. The front end doesn't
/// take the application's shader formats
/// (`SDL_PROP_RENDERER_CREATE_GPU_SHADERS_*_BOOLEAN`) yet, so there are no
/// custom shaders: the device is asked for the formats the renderer has.
fn fill_supported_shader_formats(props: &Properties) -> Result<()> {
    props.set(
        gpu::PROP_GPU_DEVICE_CREATE_SHADERS_PRIVATE_BOOLEAN,
        HAVE_PRIVATE_SHADERS,
    )?;
    props.set(
        gpu::PROP_GPU_DEVICE_CREATE_SHADERS_SPIRV_BOOLEAN,
        HAVE_SPIRV_SHADERS,
    )?;
    props.set(
        gpu::PROP_GPU_DEVICE_CREATE_SHADERS_DXIL_BOOLEAN,
        HAVE_DXIL60_SHADERS,
    )?;
    props.set(
        gpu::PROP_GPU_DEVICE_CREATE_SHADERS_MSL_BOOLEAN,
        HAVE_METAL_SHADERS,
    )?;
    Ok(())
}

/// Translation of `ChoosePresentMode()`.
fn choose_present_mode(
    device: &gpu::Device,
    window: &Window,
    vsync: i32,
) -> Result<gpu::PresentMode> {
    use gpu::PresentMode as P;
    match vsync {
        0 => {
            let mut mode = P::Mailbox;

            if !device.window_supports_present_mode(window, mode) {
                mode = P::Immediate;

                if !device.window_supports_present_mode(window, mode) {
                    mode = P::Vsync;
                }
            }

            // FIXME should we return an error if both mailbox and immediate fail?
            Ok(mode)
        }
        1 => Ok(P::Vsync),
        _ => Err(Error::unsupported()),
    }
}

/// Translation of `GPU_UpdateTextureInternal()`: upload a `w` x `h`
/// rectangle of `pixels` (rows `pitch` bytes apart) at (`x`, `y`).
#[allow(clippy::too_many_arguments)]
fn update_texture_internal(
    device: &gpu::Device,
    cpass: &mut gpu::CopyPass<'_>,
    texture: &gpu::Texture,
    bpp: usize,
    (x, y, w, h): (i32, i32, i32, i32),
    pixels: &[u8],
    pitch: usize,
) -> Result<()> {
    let overflow = || Error::new("update size overflow");
    let (w, h) = (w.max(0) as usize, h.max(0) as usize);
    let row_size = w.checked_mul(bpp).ok_or_else(overflow)?;
    let data_size = h.checked_mul(row_size).ok_or_else(overflow)?;
    let size = u32::try_from(data_size).map_err(|_| overflow())?;

    // (C trusts the caller to pass `h` rows `pitch` bytes apart)
    if h > 0 && pixels.len() < (h - 1) * pitch + row_size {
        return Err(Error::invalid_param("pixels"));
    }

    let mut tbuf = device.create_transfer_buffer(&gpu::TransferBufferCreateInfo {
        usage: gpu::TransferBufferUsage::Upload,
        size,
        props: None,
    })?;

    {
        let mut output = tbuf.map(false)?;
        if pitch == row_size {
            output[..data_size].copy_from_slice(&pixels[..data_size]);
        } else {
            for (row, out) in output[..data_size].chunks_exact_mut(row_size).enumerate() {
                out.copy_from_slice(&pixels[row * pitch..][..row_size]);
            }
        }
    }

    cpass.upload_to_texture(
        &gpu::TextureTransferInfo {
            transfer_buffer: &tbuf,
            offset: 0,
            pixels_per_row: w as u32,
            rows_per_layer: h as u32,
        },
        &gpu::TextureRegion {
            texture,
            mip_level: 0,
            layer: 0,
            x: x as u32,
            y: y as u32,
            z: 0,
            w: w as u32,
            h: h as u32,
            d: 1,
        },
        false,
    );
    // (SDL_ReleaseGPUTransferBuffer(): the GPU API keeps it until the
    // upload is done)
    drop(tbuf);

    Ok(())
}

/// The rectangle of a 4:2:0 chroma plane covering `rect`.
fn chroma_rect(rect: &Rect) -> (i32, i32, i32, i32) {
    (rect.x / 2, rect.y / 2, (rect.w + 1) / 2, (rect.h + 1) / 2)
}

/// Whether a render command draws (`Draw()` in `GPU_RunCommandQueue()`).
fn draws(cmd: &RenderCommand) -> bool {
    matches!(
        cmd,
        RenderCommand::Draw(DrawKind::Lines | DrawKind::Points | DrawKind::Geometry, _)
    )
}

/// The renderer's data. Translation of `GPU_RenderData`, with the parts of
/// `SDL_Renderer` the backend reads (`window`, `output_colorspace`, the
/// colorspace and HDR headroom of the output), and its vertex storage.
pub(crate) struct GpuRenderer {
    window: Window,
    output_colorspace: Colorspace,
    /// `renderer->target`'s colorspace, or the output colorspace.
    current_colorspace: Colorspace,
    /// `renderer->HDR_headroom`: 1 for the sRGB output the front end makes.
    hdr_headroom: f32,

    device: gpu::Device,
    shaders: Option<Shaders>,
    pipeline_cache: PipelineCache,
    backbuffer: Option<Backbuffer>,
    swapchain: SwapchainState,
    vertices: Option<VertexBuffer>,
    state: RenderState,
    samplers: [Option<gpu::Sampler>; RENDER_SAMPLER_COUNT],

    /// The vertex data of the queued commands (`first` is a byte offset in
    /// it, as upstream's vertex data is bytes).
    verts: Vec<f32>,
    texture_formats: Vec<PixelFormat>,
    /// The window was released (`GPU_DestroyRenderer()` ran).
    destroyed: bool,
}

impl GpuRenderer {
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

    /// The vertex data's byte offset for the next command.
    fn vertex_offset(&self) -> usize {
        self.verts.len() * size_of::<f32>()
    }

    /// Translation of `GPU_QueueDrawPoints()` (lines and points queue
    /// vertices the same way).
    fn queue_points(&mut self, cmd: &mut DrawCmd, points: &[FPoint]) {
        let mut color = cmd.color;
        let convert_color = self.rendering_linear_space();

        cmd.first = self.vertex_offset();

        if convert_color {
            self.convert_to_linear(&mut color);
        }

        cmd.count = points.len();
        for p in points {
            self.verts.extend_from_slice(&[
                0.5 + p.x,
                0.5 + p.y,
                color.r,
                color.g,
                color.b,
                color.a,
            ]);
        }
    }

    /// The GPU texture the renderer draws into: the target's, or the
    /// backbuffer (`renderer->target`, `data->backbuffer.texture`), and its
    /// format.
    fn target_texture(
        &self,
        textures: &TextureStore,
    ) -> Option<(Arc<gpu::Texture>, TextureFormat)> {
        match self.state.render_target.and_then(|t| textures.get(t)) {
            Some(target) => {
                let tdata = texture_data(target).ok()?;
                Some((tdata.texture.clone(), tdata.format))
            }
            // (a destroyed target is no target, as GPU_DestroyTexture()
            // makes it)
            None => self
                .backbuffer
                .as_ref()
                .map(|b| (b.texture.clone(), b.format)),
        }
    }

    /// The HDR headroom of the output (`renderer->target->HDR_headroom` or
    /// `renderer->HDR_headroom`).
    fn output_headroom(&self, textures: &TextureStore) -> f32 {
        match self.state.render_target.and_then(|t| textures.get(t)) {
            Some(target) => target.hdr_headroom,
            None => self.hdr_headroom,
        }
    }

    /// Translation of `RestartRenderPass()`, beginning the pass on
    /// `command_buffer` (the previous one has ended: it borrowed the
    /// command buffer).
    fn restart_render_pass<'c>(
        &mut self,
        command_buffer: &'c mut gpu::CommandBuffer,
        target: &gpu::Texture,
    ) -> Result<gpu::RenderPass<'c>> {
        let mut color_attachment = gpu::ColorTargetInfo::new(target);
        color_attachment.clear_color = self.state.clear_color;
        color_attachment.load_op = self.state.load_op;
        let render_pass = command_buffer.begin_render_pass(&[color_attachment], None);

        // *** FIXME ***
        // This is busted. We should be able to know which load op to use.
        // LOAD is incorrect behavior most of the time, unless we had to break a render pass.
        // -cosmonaut
        self.state.load_op = LoadOp::Load;
        self.state.scissor_was_enabled = false;

        render_pass
    }

    /// Translation of `PushVertexUniforms()`.
    fn push_vertex_uniforms(&self, pass: &mut gpu::RenderPass<'_>) {
        let viewport = &self.state.viewport;
        let mut uniforms = VertexShaderUniformData::default();
        uniforms.mvp.m[0][0] = 2.0 / viewport.w;
        uniforms.mvp.m[1][1] = -2.0 / viewport.h;
        uniforms.mvp.m[2][2] = 1.0;
        uniforms.mvp.m[3][0] = -1.0;
        uniforms.mvp.m[3][1] = 1.0;
        uniforms.mvp.m[3][3] = 1.0;

        let _ = pass
            .command_buffer()
            .push_vertex_uniform_data(0, &uniforms.bytes());
    }

    /// Translation of `SetViewportAndScissor()`.
    fn set_viewport_and_scissor(&mut self, pass: &mut gpu::RenderPass<'_>) {
        let _ = pass.set_viewport(&self.state.viewport);

        if self.state.scissor_enabled {
            let _ = pass.set_scissor(&self.state.scissor);
            self.state.scissor_was_enabled = true;
        } else if self.state.scissor_was_enabled {
            let viewport = &self.state.viewport;
            let r = Rect::new(
                viewport.x as i32,
                viewport.y as i32,
                viewport.w as i32,
                viewport.h as i32,
            );
            let _ = pass.set_scissor(&r);
            self.state.scissor_was_enabled = false;
        }
    }

    /// Translation of `GetSampler()`: the key of the sampler in
    /// `samplers`, created on first use.
    fn get_sampler(
        &mut self,
        format: PixelFormat,
        mut scale_mode: ScaleMode,
        address_u: TextureAddressMode,
        address_v: TextureAddressMode,
    ) -> Result<usize> {
        if format == PixelFormat::INDEX8 {
            // We'll do linear sampling in the shader if needed
            scale_mode = ScaleMode::Nearest;
        }

        let key = render_sampler_hashkey(scale_mode, address_u, address_v);
        crate::sdl_assert!(key < self.samplers.len());
        if self.samplers[key].is_none() {
            let mut sci = gpu::SamplerCreateInfo::default();
            match scale_mode {
                ScaleMode::Nearest => {
                    sci.min_filter = gpu::Filter::Nearest;
                    sci.mag_filter = gpu::Filter::Nearest;
                    sci.mipmap_mode = gpu::SamplerMipmapMode::Nearest;
                }
                // (pixel art uses linear sampling)
                ScaleMode::PixelArt | ScaleMode::Linear => {
                    sci.min_filter = gpu::Filter::Linear;
                    sci.mag_filter = gpu::Filter::Linear;
                    sci.mipmap_mode = gpu::SamplerMipmapMode::Linear;
                }
            }
            let address_mode = |mode: TextureAddressMode| match mode {
                TextureAddressMode::Clamp => Ok(gpu::SamplerAddressMode::ClampToEdge),
                TextureAddressMode::Wrap => Ok(gpu::SamplerAddressMode::Repeat),
                other => Err(Error::new(format!(
                    "Unknown texture address mode: {}",
                    other as i32
                ))),
            };
            sci.address_mode_u = address_mode(address_u)?;
            sci.address_mode_v = address_mode(address_v)?;
            sci.address_mode_w = gpu::SamplerAddressMode::ClampToEdge;

            self.samplers[key] = Some(self.device.create_sampler(&sci)?);
        }
        Ok(key)
    }

    /// Translation of `CalculateAdvancedShaderConstants()`.
    fn calculate_advanced_shader_constants(
        &self,
        cmd: &DrawCmd,
        texture: &TextureData,
        tdata: &GpuTextureData,
        output_headroom: f32,
    ) -> AdvancedFragmentShaderUniformData {
        use PixelFormat as F;
        let mut constants = AdvancedFragmentShaderUniformData {
            sc_rgb_output: self.rendering_linear_space() as i32 as f32,
            color_scale: cmd.color_scale,
            ..Default::default()
        };

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
                let pixel_art = cmd.texture_scale_mode == ScaleMode::PixelArt;
                constants.texture_type = match texture.format {
                    F::BGRX32 | F::RGBX32 if pixel_art => TEXTURETYPE_RGB_PIXELART,
                    F::BGRX32 | F::RGBX32 => TEXTURETYPE_RGB,
                    _ if pixel_art => TEXTURETYPE_RGBA_PIXELART,
                    _ => TEXTURETYPE_RGBA,
                };
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
            || constants.texture_type == TEXTURETYPE_RGBA_PIXELART
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

        if let Some(matrix) = &tdata.ycbcr_matrix {
            constants.ycbcr_matrix = *matrix;
        }
        constants
    }

    /// Translation of `Draw()`, in the render pass that
    /// [`GpuRenderer::run_command_queue`] restarts first when needed.
    fn draw(
        &mut self,
        pass: &mut gpu::RenderPass<'_>,
        cmd: &DrawCmd,
        num_verts: u32,
        offset: u32,
        prim: PrimitiveType,
        textures: &TextureStore,
    ) {
        let simple_constants = SimpleFragmentShaderUniformData {
            color_scale: cmd.color_scale,
        };
        let mut advanced_constants = AdvancedFragmentShaderUniformData::default();

        // (a texture without renderer data is drawn without it)
        let texture = cmd
            .texture
            .and_then(|t| textures.get(t))
            .and_then(|t| Some((t, texture_data(t).ok()?)));

        let (v_shader, f_shader) = if prim == PrimitiveType::TriangleList {
            if let Some((texture, tdata)) = texture {
                advanced_constants = self.calculate_advanced_shader_constants(
                    cmd,
                    texture,
                    tdata,
                    self.output_headroom(textures),
                );
                // FIXME (upstream): the textures with an alpha channel
                // other than RGBA32 and BGRA32 (ARGB4444, ARGB1555,
                // ABGR2101010, RGBA64_FLOAT) get the RGB shader, which
                // drops their alpha.
                let f_shader = if (advanced_constants.texture_type == TEXTURETYPE_RGB
                    || advanced_constants.texture_type == TEXTURETYPE_RGBA)
                    && advanced_constants.input_type == INPUTTYPE_UNSPECIFIED
                    && advanced_constants.tonemap_method == TONEMAP_NONE
                {
                    if texture.format == PixelFormat::RGBA32
                        || texture.format == PixelFormat::BGRA32
                    {
                        FragmentShaderId::TextureRgba
                    } else {
                        FragmentShaderId::TextureRgb
                    }
                } else {
                    FragmentShaderId::TextureAdvanced
                };
                (VertexShaderId::TriTexture, f_shader)
            } else {
                (VertexShaderId::TriColor, FragmentShaderId::Color)
            }
        } else {
            (VertexShaderId::Linepoint, FragmentShaderId::Color)
        };

        let Some((_, attachment_format)) = self.target_texture(textures) else {
            return;
        };
        let pipe_params = PipelineParameters {
            blend_mode: cmd.blend,
            vert_shader: v_shader,
            frag_shader: f_shader,
            primitive_type: prim,
            attachment_format,
        };

        let Some(shaders) = &self.shaders else {
            return;
        };
        let Ok(pipe) = self
            .pipeline_cache
            .get_pipeline(shaders, &self.device, &pipe_params)
        else {
            return;
        };

        pass.bind_graphics_pipeline(pipe);

        if let Some((texture, tdata)) = texture {
            // (upstream binds a NULL sampler it couldn't create)
            let Ok(sampler) = self.get_sampler(
                texture.format,
                cmd.texture_scale_mode,
                cmd.texture_address_mode_u,
                cmd.texture_address_mode_v,
            ) else {
                return;
            };
            let palette = match &tdata.palette {
                Some(palette)
                    if f_shader == FragmentShaderId::TextureAdvanced
                        && texture.palette.is_some() =>
                {
                    let Ok(key) = self.get_sampler(
                        PixelFormat::UNKNOWN,
                        ScaleMode::Nearest,
                        TextureAddressMode::Clamp,
                        TextureAddressMode::Clamp,
                    ) else {
                        return;
                    };
                    Some((palette, key))
                }
                _ => None,
            };

            let Some(sampler) = self.samplers[sampler].as_ref() else {
                return;
            };
            let mut sampler_binds = vec![gpu::TextureSamplerBinding {
                texture: &tdata.texture,
                sampler,
            }];

            if f_shader == FragmentShaderId::TextureAdvanced {
                if let Some((palette, key)) = palette {
                    if let Some(sampler) = self.samplers[key].as_ref() {
                        sampler_binds.push(gpu::TextureSamplerBinding {
                            texture: palette,
                            sampler,
                        });
                    }
                } else {
                    match &tdata.planes {
                        Planes::Yuv { u, v } => {
                            sampler_binds.push(gpu::TextureSamplerBinding {
                                texture: u,
                                sampler,
                            });
                            sampler_binds.push(gpu::TextureSamplerBinding {
                                texture: v,
                                sampler,
                            });
                        }
                        Planes::Nv(nv) => {
                            sampler_binds.push(gpu::TextureSamplerBinding {
                                texture: nv,
                                sampler,
                            });
                        }
                        Planes::Single => {}
                    }
                }

                // We need to fill 3 sampler slots for the advanced shader
                while sampler_binds.len() < 3 {
                    let last = sampler_binds[sampler_binds.len() - 1];
                    sampler_binds.push(last);
                }
            }
            let _ = pass.bind_fragment_samplers(0, &sampler_binds);
        }

        // (no render states: the uniforms are the renderer's)
        let uniforms = if f_shader == FragmentShaderId::TextureAdvanced {
            advanced_constants.bytes()
        } else {
            simple_constants.bytes()
        };
        let _ = pass
            .command_buffer()
            .push_fragment_uniform_data(0, &uniforms);

        let Some(vertices) = &self.vertices else {
            return;
        };
        pass.bind_vertex_buffers(
            0,
            &[gpu::BufferBinding {
                buffer: &vertices.buffer,
                offset,
            }],
        );
        self.push_vertex_uniforms(pass);

        self.set_viewport_and_scissor(pass);

        let _ = pass.draw_primitives(num_verts, 1, 0, 0);
    }

    /// Translation of `InitVertexBuffer()` (`ReleaseVertexBuffer()` is
    /// dropping it).
    fn init_vertex_buffer(&mut self, size: u32) -> Result<()> {
        let buffer = self.device.create_buffer(&gpu::BufferCreateInfo {
            usage: gpu::BufferUsageFlags::VERTEX,
            size,
            props: None,
        })?;

        let transfer_buf = self
            .device
            .create_transfer_buffer(&gpu::TransferBufferCreateInfo {
                usage: gpu::TransferBufferUsage::Upload,
                size,
                props: None,
            })?;

        self.vertices = Some(VertexBuffer {
            transfer_buf,
            buffer,
            buffer_size: size,
        });

        Ok(())
    }

    /// Translation of `UploadVertices()`.
    fn upload_vertices(&mut self, command_buffer: &mut gpu::CommandBuffer) -> Result<()> {
        let vertsize = self.vertex_offset();
        if vertsize == 0 {
            return Ok(());
        }
        // (C casts the size to Uint32)
        let size = u32::try_from(vertsize).map_err(|_| Error::new("vertex data too large"))?;

        if self.vertices.as_ref().is_none_or(|v| size > v.buffer_size) {
            // ReleaseVertexBuffer()
            self.vertices = None;
            self.init_vertex_buffer(size)?;
        }
        let Some(vertices) = &mut self.vertices else {
            return Ok(());
        };

        {
            let mut staging_buf = vertices.transfer_buf.map(true)?;
            for (out, v) in staging_buf[..vertsize].chunks_exact_mut(4).zip(&self.verts) {
                out.copy_from_slice(&v.to_ne_bytes());
            }
        }

        let mut pass = command_buffer.begin_copy_pass()?;

        pass.upload_to_buffer(
            &gpu::TransferBufferLocation {
                transfer_buffer: &vertices.transfer_buf,
                offset: 0,
            },
            &gpu::BufferRegion {
                buffer: &vertices.buffer,
                offset: 0,
                size,
            },
            true,
        );
        pass.end();

        Ok(())
    }

    /// `GPU_RunCommandQueue()` with the command buffer taken out of the
    /// renderer: the render passes borrow it.
    fn run_commands(
        &mut self,
        command_buffer: &mut gpu::CommandBuffer,
        cmds: &[RenderCommand],
        textures: &TextureStore,
    ) -> Result<()> {
        self.upload_vertices(command_buffer)?;

        self.state.load_op = LoadOp::Load;

        let Some((target, _)) = self.target_texture(textures) else {
            return Err(Error::new("Render target texture is NULL"));
        };

        // Each draw happens in a render pass: Draw() restarts the pass when
        // there is none yet or a clear is pending. Between the draws, the
        // pass stays open.
        let mut i = 0;
        while i < cmds.len() {
            if draws(&cmds[i]) {
                let mut pass = self.restart_render_pass(command_buffer, &target)?;
                loop {
                    i = self.run_command(cmds, i, Some(&mut pass), textures);
                    if i >= cmds.len() || (draws(&cmds[i]) && self.state.load_op == LoadOp::Clear) {
                        break;
                    }
                }
            } else {
                i = self.run_command(cmds, i, None, textures);
            }
        }

        if self.state.load_op == LoadOp::Clear {
            drop(self.restart_render_pass(command_buffer, &target)?);
        }

        // (the last render pass has ended)
        Ok(())
    }

    /// Run `cmds[i]` (and the commands it's combined with) as
    /// `GPU_RunCommandQueue()`'s loop does; the index of the next command.
    /// `pass` is the render pass when `cmds[i]` draws.
    fn run_command(
        &mut self,
        cmds: &[RenderCommand],
        i: usize,
        pass: Option<&mut gpu::RenderPass<'_>>,
        textures: &TextureStore,
    ) -> usize {
        let mut finalcmd = i;
        match cmds[i] {
            RenderCommand::SetDrawColor { .. } => {
                // this isn't currently used in this render backend.
            }

            RenderCommand::SetViewport { rect: viewport, .. } => {
                self.state.viewport.x = viewport.x as f32;
                self.state.viewport.y = viewport.y as f32;
                self.state.viewport.w = viewport.w as f32;
                self.state.viewport.h = viewport.h as f32;
            }

            RenderCommand::SetClipRect { enabled, rect } => {
                self.state.scissor.x = self.state.viewport.x as i32 + rect.x;
                self.state.scissor.y = self.state.viewport.y as i32 + rect.y;
                self.state.scissor.w = rect.w;
                self.state.scissor.h = rect.h;
                self.state.scissor_enabled = enabled;
            }

            RenderCommand::Clear {
                mut color,
                color_scale,
                ..
            } => {
                let convert_color = self.rendering_linear_space();
                if convert_color {
                    self.convert_to_linear(&mut color);
                }
                color.r *= color_scale;
                color.g *= color_scale;
                color.b *= color_scale;
                self.state.clear_color = color;
                self.state.load_op = LoadOp::Clear;
            }

            RenderCommand::Draw(DrawKind::FillRects | DrawKind::Copy | DrawKind::CopyEx, _) => {
                // unused
            }

            RenderCommand::Draw(DrawKind::Lines, d) => {
                let Some(pass) = pass else {
                    return i + 1;
                };
                let mut count = d.count as u32;
                let offset = d.first as u32;

                if count > 2 {
                    // joined lines cannot be grouped
                    self.draw(pass, &d, count, offset, PrimitiveType::LineStrip, textures);
                } else {
                    // let's group non joined lines
                    let thiscolorscale = d.color_scale;
                    let thisblend = d.blend;

                    for (j, next) in cmds.iter().enumerate().skip(i + 1) {
                        match next {
                            RenderCommand::Draw(DrawKind::Lines, n) => {
                                if n.count != 2 {
                                    break; // can't go any further on this draw call, those are joined lines
                                } else if n.blend != thisblend || n.color_scale != thiscolorscale {
                                    break; // can't go any further on this draw call, different blendmode copy up next.
                                } else {
                                    finalcmd = j; // we can combine copy operations here. Mark this one as the furthest okay command.
                                    count += n.count as u32;
                                }
                            }
                            RenderCommand::SetDrawColor { .. } => {
                                // The vertex data has the draw color built in, ignore this
                                continue;
                            }
                            _ => break, // can't go any further on this draw call, different render command up next.
                        }
                    }

                    self.draw(pass, &d, count, offset, PrimitiveType::LineList, textures);
                }
            }

            RenderCommand::Draw(thiscmdtype @ (DrawKind::Points | DrawKind::Geometry), d) => {
                let Some(pass) = pass else {
                    return i + 1;
                };
                /* as long as we have the same copy command in a row, with the
                same texture, we can combine them all into a single draw call. */
                let mut count = d.count as u32;
                let offset = d.first as u32;

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
                                count += n.count as u32;
                            }
                        }
                        RenderCommand::SetDrawColor { .. } => {
                            // The vertex data has the draw color built in, ignore this
                            continue;
                        }
                        _ => break, // can't go any further on this draw call, different render command up next.
                    }
                }

                let prim = if thiscmdtype == DrawKind::Geometry {
                    PrimitiveType::TriangleList
                } else {
                    PrimitiveType::PointList
                };
                self.draw(pass, &d, count, offset, prim, textures);
            }

            RenderCommand::NoOp => {}
        }

        finalcmd + 1 // skip any copy commands we just combined in here.
    }

    /// Translation of `CreateBackbuffer()`.
    fn create_backbuffer(&mut self, w: u32, h: u32, fmt: TextureFormat) -> Result<()> {
        let tci = gpu::TextureCreateInfo {
            width: w,
            height: h,
            format: fmt,
            layer_count_or_depth: 1,
            num_levels: 1,
            sample_count: gpu::SampleCount::One,
            usage: TextureUsageFlags::COLOR_TARGET | TextureUsageFlags::SAMPLER,
            ..Default::default()
        };

        // (SDL_ReleaseGPUTexture())
        self.backbuffer = None;

        let texture = self.device.create_texture(&tci)?;
        self.backbuffer = Some(Backbuffer {
            texture: Arc::new(texture),
            format: fmt,
            width: w,
            height: h,
        });

        Ok(())
    }

    /// The command buffer for the next commands (`data->state.command_buffer
    /// = SDL_AcquireGPUCommandBuffer()`).
    fn acquire_command_buffer(&mut self) {
        self.state.command_buffer = self.device.acquire_command_buffer().ok();
    }

    /// Translation of `GPU_RenderReadPixels()`.
    fn render_read_pixels(
        &mut self,
        rect: &Rect,
        textures: &TextureStore,
    ) -> Result<Surface<'static>> {
        let (gpu_tex, pixfmt) = match self.state.render_target.and_then(|t| textures.get(t)) {
            Some(texture) => (texture_data(texture)?.texture.clone(), texture.format),
            None => {
                let backbuffer = self.backbuffer.as_ref().ok_or_else(not_available)?;
                let Some(pixfmt) = backbuffer.format.pixel_format() else {
                    return Err(Error::new("Unsupported backbuffer format"));
                };
                (backbuffer.texture.clone(), pixfmt)
            }
        };

        let bpp = pixfmt.bytes_per_pixel() as usize;
        let overflow = || Error::new("read size overflow");
        let (w, h) = (rect.w.max(0) as usize, rect.h.max(0) as usize);
        let row_size = w.checked_mul(bpp).ok_or_else(overflow)?;
        let image_size = h.checked_mul(row_size).ok_or_else(overflow)?;
        let size = u32::try_from(image_size).map_err(|_| overflow())?;

        let mut surface = Surface::new_uninitialized(rect.w, rect.h, pixfmt)?;

        let mut tbuf = self
            .device
            .create_transfer_buffer(&gpu::TransferBufferCreateInfo {
                usage: gpu::TransferBufferUsage::Download,
                size,
                props: None,
            })?;

        let mut command_buffer = self
            .state
            .command_buffer
            .take()
            .ok_or_else(no_command_buffer)?;
        let downloaded = (|| {
            let mut pass = command_buffer.begin_copy_pass()?;
            pass.download_from_texture(
                &gpu::TextureRegion {
                    texture: &gpu_tex,
                    mip_level: 0,
                    layer: 0,
                    x: rect.x as u32,
                    y: rect.y as u32,
                    z: 0,
                    w: w as u32,
                    h: h as u32,
                    d: 1,
                },
                &gpu::TextureTransferInfo {
                    transfer_buffer: &tbuf,
                    offset: 0,
                    pixels_per_row: w as u32,
                    rows_per_layer: h as u32,
                },
            );
            pass.end();
            Ok(())
        })();
        let submitted = downloaded.and_then(|()| {
            let fence = command_buffer.submit_and_acquire_fence()?;
            self.device.wait_for_fences(true, &[&fence])
            // (SDL_ReleaseGPUFence(): dropping it)
        });
        self.acquire_command_buffer();
        submitted?;

        let mapped_tbuf = tbuf.map(false)?;
        let pitch = surface.pitch() as usize;
        let pixels = surface
            .pixels_mut()
            .ok_or_else(|| Error::new("surface has no pixels"))?;
        if pitch == row_size {
            pixels[..image_size].copy_from_slice(&mapped_tbuf[..image_size]);
        } else {
            for (row, input) in mapped_tbuf[..image_size].chunks_exact(row_size).enumerate() {
                pixels[row * pitch..][..row_size].copy_from_slice(input);
            }
        }

        Ok(surface)
    }

    /// Translation of `GPU_RenderPresent()`.
    fn render_present(&mut self) {
        let Some(mut command_buffer) = self.state.command_buffer.take() else {
            self.acquire_command_buffer();
            return;
        };

        let swapchain = match command_buffer.wait_and_acquire_swapchain_texture(&self.window) {
            Ok(swapchain) => swapchain,
            Err(e) => {
                crate::log::error!(
                    Category::Render,
                    "Failed to acquire swapchain texture: {}",
                    e
                );
                None
            }
        };

        let backbuffer = self
            .backbuffer
            .as_ref()
            .map(|b| (b.texture.clone(), b.width, b.height));
        match (swapchain, backbuffer) {
            (Some(swapchain), Some((backbuffer, width, height))) => {
                let blit_info = gpu::BlitInfo {
                    source: gpu::BlitRegion {
                        texture: &backbuffer,
                        mip_level: 0,
                        layer_or_depth_plane: 0,
                        x: 0,
                        y: 0,
                        w: width,
                        h: height,
                    },
                    destination: gpu::BlitRegion {
                        texture: &swapchain.texture,
                        mip_level: 0,
                        layer_or_depth_plane: 0,
                        x: 0,
                        y: 0,
                        w: swapchain.width,
                        h: swapchain.height,
                    },
                    load_op: LoadOp::DontCare,
                    clear_color: FColor::default(),
                    flip_mode: FlipMode::None,
                    filter: gpu::Filter::Linear,
                    cycle: false,
                };

                let _ = command_buffer.blit_texture(&blit_info);

                let _ = command_buffer.submit();

                if swapchain.width != width || swapchain.height != height {
                    if let Ok(format) = self.device.swapchain_texture_format(&self.window) {
                        let _ = self.create_backbuffer(swapchain.width, swapchain.height, format);
                    }

                    // Notify the application that it needs to redraw this frame
                    crate::events::window::send_window_event(
                        self.window.id(),
                        EventType::WINDOW_EXPOSED,
                        0,
                        0,
                    );
                }
            }
            _ => {
                let _ = command_buffer.submit();
            }
        }

        self.acquire_command_buffer();
    }

    /// Translation of `GPU_CreateRenderer()` for a window.
    pub(crate) fn for_window(
        window: Window,
        output_colorspace: Colorspace,
        vsync: i32,
    ) -> Result<GpuRenderer> {
        // Clear any OpenGL properties on the window to avoid potential driver conflicts.
        let mut flags = window.flags()?;
        if flags.contains(WindowFlags::OPENGL) {
            flags &= !WindowFlags::OPENGL;
            let _ = window.reconfigure(flags);
        }

        // (SDL_SetupRendererColorspace())
        if output_colorspace != Colorspace::SRGB && output_colorspace != Colorspace::SRGB_LINEAR
        /*&& output_colorspace != Colorspace::HDR10*/
        {
            return Err(Error::new("Unsupported output colorspace"));
        }

        // (renderer->window is the window from the start; the renderer's
        // functions are the RenderBackend implementation)

        // (there is no SDL_PROP_RENDERER_CREATE_GPU_DEVICE_POINTER to take:
        // the renderer makes its device)
        let create_props = Properties::new();
        {
            // Prefer environment variables/hints if they exist, otherwise defer to properties
            let debug = hints::get_bool(hints::RENDER_GPU_DEBUG, false);
            let lowpower = hints::get_bool(hints::RENDER_GPU_LOW_POWER, false);

            create_props.set(gpu::PROP_GPU_DEVICE_CREATE_DEBUGMODE_BOOLEAN, debug)?;
            create_props.set(gpu::PROP_GPU_DEVICE_CREATE_PREFERLOWPOWER_BOOLEAN, lowpower)?;

            // Vulkan windows get the Vulkan GPU backend by default
            if window.flags()?.contains(WindowFlags::VULKAN) {
                create_props.set(gpu::PROP_GPU_DEVICE_CREATE_NAME_STRING, "vulkan")?;
            }

            // Set hints for the greatest hardware compatibility
            // This property allows using the renderer on Intel Haswell and Broadwell GPUs.
            create_props.set(
                gpu::PROP_GPU_DEVICE_CREATE_D3D12_ALLOW_FEWER_RESOURCE_SLOTS_BOOLEAN,
                true,
            )?;
            // These properties allow using the renderer on more Android devices.
            create_props.set(
                gpu::PROP_GPU_DEVICE_CREATE_FEATURE_CLIP_DISTANCE_BOOLEAN,
                false,
            )?;
            create_props.set(
                gpu::PROP_GPU_DEVICE_CREATE_FEATURE_DEPTH_CLAMPING_BOOLEAN,
                false,
            )?;
            create_props.set(
                gpu::PROP_GPU_DEVICE_CREATE_FEATURE_INDIRECT_DRAW_FIRST_INSTANCE_BOOLEAN,
                false,
            )?;
            create_props.set(
                gpu::PROP_GPU_DEVICE_CREATE_FEATURE_ANISOTROPY_BOOLEAN,
                false,
            )?;
            // These properties allow using the renderer on more macOS devices.
            create_props.set(
                gpu::PROP_GPU_DEVICE_CREATE_METAL_ALLOW_MACFAMILY1_BOOLEAN,
                false,
            )?;

            fill_supported_shader_formats(&create_props)?;
        }
        let device = gpu::Device::with_properties(&create_props)?;

        let shaders = Shaders::new(&device)?;

        let mut data = GpuRenderer {
            window,
            output_colorspace,
            current_colorspace: output_colorspace,
            hdr_headroom: 1.0,
            device,
            shaders: Some(shaders),
            // (GPU_InitPipelineCache())
            pipeline_cache: PipelineCache::default(),
            backbuffer: None,
            swapchain: SwapchainState {
                composition: gpu::SwapchainComposition::Sdr,
                present_mode: gpu::PresentMode::Vsync,
            },
            vertices: None,
            state: RenderState {
                render_target: None,
                command_buffer: None,
                clear_color: FColor::default(),
                load_op: LoadOp::Load,
                viewport: gpu::Viewport::default(),
                scissor: Rect::default(),
                scissor_enabled: false,
                scissor_was_enabled: false,
            },
            samplers: Default::default(),
            verts: Vec::new(),
            texture_formats: Vec::new(),
            destroyed: false,
        };

        // FIXME: What's a good initial size?
        data.init_vertex_buffer(1 << 16)?;

        data.device.claim_window(&window)?;
        // (SDL_CreateRenderer() destroys the renderer that failed, which
        // releases the window)
        if let Err(e) = data.setup_swapchain(vsync) {
            data.destroy();
            return Err(e);
        }

        for format in SUPPORTED_FORMATS {
            if TextureFormat::from_pixel_format(format).is_some_and(|f| {
                data.device.texture_supports_format(
                    f,
                    gpu::TextureType::Texture2D,
                    TextureUsageFlags::SAMPLER,
                )
            }) {
                data.texture_formats.push(format);
            }
        }
        data.texture_formats.extend_from_slice(&[
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

        // (SDL_PROP_RENDERER_MAX_TEXTURE_SIZE_NUMBER: max_texture_size())

        data.state.viewport.min_depth = 0.0;
        data.state.viewport.max_depth = 1.0;
        data.acquire_command_buffer();

        // (SDL_PROP_RENDERER_GPU_DEVICE_POINTER: set_properties())

        Ok(data)
    }

    /// The swapchain setup and the backbuffer of `GPU_CreateRenderer()`,
    /// once the window is claimed.
    fn setup_swapchain(&mut self, vsync: i32) -> Result<()> {
        let window = self.window;
        self.swapchain.composition = match self.output_colorspace {
            Colorspace::SRGB_LINEAR => gpu::SwapchainComposition::HdrExtendedLinear,
            Colorspace::HDR10 => gpu::SwapchainComposition::Hdr10St2084,
            _ => gpu::SwapchainComposition::Sdr,
        };
        self.swapchain.present_mode = gpu::PresentMode::Vsync;

        if let Ok(mode) = choose_present_mode(&self.device, &window, vsync) {
            self.swapchain.present_mode = mode;
        }

        let _ = self.device.set_swapchain_parameters(
            &window,
            self.swapchain.composition,
            self.swapchain.present_mode,
        );

        let _ = self.device.set_allowed_frames_in_flight(1);

        let (w, h) = window.size_in_pixels()?;

        let format = self.device.swapchain_texture_format(&window)?;
        self.create_backbuffer(w as u32, h as u32, format)
    }

    /// `GPU_UpdateTexture()` for pixels the front end or a lock gives.
    fn update_texture_pixels(
        &mut self,
        texture: &TextureData,
        rect: &Rect,
        pixels: &[u8],
        pitch: usize,
    ) -> Result<()> {
        let data = texture_data(texture)?;
        let command_buffer = self
            .state
            .command_buffer
            .as_mut()
            .ok_or_else(no_command_buffer)?;
        let device = &self.device;
        let mut cpass = command_buffer.begin_copy_pass()?;
        let mut bpp = texture.format.bytes_per_pixel() as usize;
        let full = (rect.x, rect.y, rect.w, rect.h);
        let plane_offset = |rows: i32, pitch: usize| rows.max(0) as usize * pitch;

        let mut retval =
            update_texture_internal(device, &mut cpass, &data.texture, bpp, full, pixels, pitch);

        match &data.planes {
            Planes::Nv(texture_nv) => {
                let uv_plane = plane(pixels, plane_offset(rect.h, pitch));

                bpp *= 2;
                let uv_pitch = if texture.format == PixelFormat::P010 {
                    (pitch + 3) & !3
                } else {
                    (pitch + 1) & !1
                };
                retval = retval.and(uv_plane.and_then(|uv_plane| {
                    update_texture_internal(
                        device,
                        &mut cpass,
                        texture_nv,
                        bpp,
                        chroma_rect(rect),
                        uv_plane,
                        uv_pitch,
                    )
                }));
            }
            Planes::Yuv { u, v } => {
                if texture.format == PixelFormat::I444 || texture.format == PixelFormat::I4FL {
                    let u_offset = plane_offset(rect.h, pitch);
                    let v_offset = u_offset + plane_offset(rect.h, pitch);

                    retval = retval.and(plane(pixels, u_offset).and_then(|u_plane| {
                        update_texture_internal(device, &mut cpass, u, bpp, full, u_plane, pitch)
                    }));
                    retval = retval.and(plane(pixels, v_offset).and_then(|v_plane| {
                        update_texture_internal(device, &mut cpass, v, bpp, full, v_plane, pitch)
                    }));
                } else {
                    let y_pitch = pitch;
                    let uv_pitch = (y_pitch / bpp).div_ceil(2) * bpp;
                    let u_offset = plane_offset(rect.h, y_pitch);
                    let v_offset = u_offset + plane_offset((rect.h + 1) / 2, uv_pitch);

                    // (YV12 has the V plane first)
                    let (first, second) = if texture.format == PixelFormat::YV12 {
                        (v, u)
                    } else {
                        (u, v)
                    };
                    retval = retval.and(plane(pixels, u_offset).and_then(|u_plane| {
                        update_texture_internal(
                            device,
                            &mut cpass,
                            first,
                            bpp,
                            chroma_rect(rect),
                            u_plane,
                            uv_pitch,
                        )
                    }));
                    retval = retval.and(plane(pixels, v_offset).and_then(|v_plane| {
                        update_texture_internal(
                            device,
                            &mut cpass,
                            second,
                            bpp,
                            chroma_rect(rect),
                            v_plane,
                            uv_pitch,
                        )
                    }));
                }
            }
            Planes::Single => {}
        }

        cpass.end();
        retval
    }
}

impl RenderBackend for GpuRenderer {
    fn name(&self) -> &'static str {
        GPU_RENDERER
    }

    fn output_size(&self, _textures: &TextureStore) -> Option<Result<(i32, i32)>> {
        None // (the window's size in pixels)
    }

    fn texture_formats(&self) -> Option<Vec<PixelFormat>> {
        Some(self.texture_formats.clone())
    }

    fn max_texture_size(&self) -> Option<i32> {
        Some(16384)
    }

    fn set_properties(&mut self, props: &Properties) {
        let _ = props.set_any(PROP_RENDERER_GPU_DEVICE_POINTER, self.device.clone());
    }

    /// Translation of `GPU_SupportsBlendMode()`.
    fn supports_blend_mode(&self, blend_mode: BlendMode) -> bool {
        let src_color_factor = blend_mode.src_color_factor();
        let src_alpha_factor = blend_mode.src_alpha_factor();
        let color_operation = blend_mode.color_operation();
        let dst_color_factor = blend_mode.dst_color_factor();
        let dst_alpha_factor = blend_mode.dst_alpha_factor();
        let alpha_operation = blend_mode.alpha_operation();

        !(convert_blend_factor(src_color_factor).is_none()
            || convert_blend_factor(src_alpha_factor).is_none()
            || convert_blend_operation(color_operation).is_none()
            || convert_blend_factor(dst_color_factor).is_none()
            || convert_blend_factor(dst_alpha_factor).is_none()
            || convert_blend_operation(alpha_operation).is_none())
    }

    /// Translation of `GPU_CreateTexture()`.
    fn create_texture(
        &mut self,
        texture: &mut TextureData,
        _props: &TextureCreateProps,
    ) -> Result<()> {
        use PixelFormat as F;
        let mut usage = TextureUsageFlags::SAMPLER;

        let mut format = match texture.format {
            F::INDEX8 | F::YV12 | F::IYUV | F::I444 | F::NV12 | F::NV21 => {
                Some(TextureFormat::R8_UNORM)
            }
            F::P010 | F::I0FL | F::I4FL => Some(TextureFormat::R16_UNORM),
            other => TextureFormat::from_pixel_format(other),
        };
        if self.output_colorspace == Colorspace::SRGB_LINEAR {
            format = match format {
                Some(TextureFormat::R8G8B8A8_UNORM) => Some(TextureFormat::R8G8B8A8_UNORM_SRGB),
                Some(TextureFormat::B8G8R8A8_UNORM) => Some(TextureFormat::B8G8R8A8_UNORM_SRGB),
                other => other,
            };
        }
        let Some(format) = format else {
            return Err(Error::new(format!(
                "Texture format {} not supported by SDL_GPU",
                texture.format.name()
            )));
        };

        let mut pixels = Vec::new();
        let mut pitch = 0;
        if texture.access == TextureAccess::Streaming {
            let size;
            if texture.format.is_fourcc() {
                let (yuv_size, yuv_pitch) = crate::video::surface::calculate_yuv_size(
                    texture.format,
                    texture.w,
                    texture.h,
                )?;
                size = yuv_size;
                pitch = yuv_pitch as i32;
            } else {
                pitch = texture.w * texture.format.bytes_per_pixel() as i32;
                size = texture.h.max(0) as usize * pitch.max(0) as usize;
            }
            pixels = vec![0; size];

            // TODO allocate a persistent transfer buffer
        }

        if texture.access == TextureAccess::Target {
            usage |= TextureUsageFlags::COLOR_TARGET;
        }

        let mut tci = gpu::TextureCreateInfo {
            format,
            layer_count_or_depth: 1,
            num_levels: 1,
            usage,
            width: texture.w as u32,
            height: texture.h as u32,
            sample_count: gpu::SampleCount::One,
            ..Default::default()
        };

        // (no SDL_PROP_TEXTURE_CREATE_GPU_TEXTURE_POINTER to take)
        let main = Arc::new(self.device.create_texture(&tci)?);

        let props = texture.props.get_or_insert_with(Properties::new).clone();
        let publish = |name: &str, t: &Arc<gpu::Texture>| {
            let _ = props.set(name, Value::Any(t.clone()));
        };
        publish(PROP_TEXTURE_GPU_TEXTURE_POINTER, &main);

        // FIXME (upstream): the default colorspace of P010, I0FL and I4FL
        // textures (HDR10) is an RGB one, which has no YCbCr matrix: they
        // need a YCbCr colorspace.
        let ycbcr_matrix = |bits_per_pixel: u32| -> Result<[f32; 16]> {
            let Some(matrix) = texture
                .colorspace
                .ycbcr_to_rgb_matrix(texture.h.max(0) as u32, bits_per_pixel)
            else {
                return Err(Error::new("Unsupported YUV colorspace"));
            };
            let mut ycbcr = [0.0; 16];
            ycbcr[..3].copy_from_slice(&matrix.offset);
            for (row, coeff) in matrix.coeff.iter().enumerate() {
                ycbcr[4 + 4 * row..7 + 4 * row].copy_from_slice(coeff);
            }
            Ok(ycbcr)
        };

        let mut planes = Planes::Single;
        let mut matrix = None;
        if matches!(texture.format, F::YV12 | F::IYUV | F::I0FL) {
            tci.width = tci.width.div_ceil(2);
            tci.height = tci.height.div_ceil(2);

            let u = Arc::new(self.device.create_texture(&tci)?);
            publish(PROP_TEXTURE_GPU_TEXTURE_U_POINTER, &u);

            let v = Arc::new(self.device.create_texture(&tci)?);
            publish(PROP_TEXTURE_GPU_TEXTURE_V_POINTER, &v);

            planes = Planes::Yuv { u, v };
            let bits_per_pixel = if texture.format == F::I0FL { 16 } else { 8 };
            matrix = Some(ycbcr_matrix(bits_per_pixel)?);
        }
        if matches!(texture.format, F::I444 | F::I4FL) {
            let u = Arc::new(self.device.create_texture(&tci)?);
            publish(PROP_TEXTURE_GPU_TEXTURE_U_POINTER, &u);

            let v = Arc::new(self.device.create_texture(&tci)?);
            // FIXME (upstream): the V property is set to the U texture.
            publish(PROP_TEXTURE_GPU_TEXTURE_V_POINTER, &u);

            planes = Planes::Yuv { u, v };
            let bits_per_pixel = if texture.format == F::I4FL { 16 } else { 8 };
            matrix = Some(ycbcr_matrix(bits_per_pixel)?);
        }
        if matches!(texture.format, F::NV12 | F::NV21 | F::P010) {
            tci.width = tci.width.div_ceil(2);
            tci.height = tci.height.div_ceil(2);
            tci.format = if texture.format == F::P010 {
                TextureFormat::R16G16_UNORM
            } else {
                TextureFormat::R8G8_UNORM
            };

            let nv = Arc::new(self.device.create_texture(&tci)?);
            publish(PROP_TEXTURE_GPU_TEXTURE_UV_POINTER, &nv);

            planes = Planes::Nv(nv);
            let bits_per_pixel = if texture.format == F::P010 { 10 } else { 8 };
            matrix = Some(ycbcr_matrix(bits_per_pixel)?);
        }

        texture.internal = Some(Box::new(GpuTextureData {
            texture: main,
            format,
            pixels,
            pitch,
            locked_rect: Rect::default(),
            ycbcr_matrix: matrix,
            planes,
            palette: None,
        }));
        Ok(())
    }

    fn queue_set_viewport(&mut self, _cmd: &mut RenderCommand) -> Result<()> {
        Ok(()) // nothing to do in this backend.
    }

    fn queue_set_draw_color(&mut self, _cmd: &mut RenderCommand) -> Result<()> {
        Ok(()) // nothing to do in this backend.
    }

    /// Translation of `GPU_QueueDrawPoints()`.
    fn queue_draw_points(&mut self, cmd: &mut DrawCmd, points: &[FPoint]) -> Result<()> {
        self.queue_points(cmd, points);
        Ok(())
    }

    /// `GPU_QueueDrawPoints()`: lines and points queue vertices the same way.
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

    /// Translation of `GPU_QueueGeometry()`.
    fn queue_geometry(
        &mut self,
        cmd: &mut DrawCmd,
        _texture: Option<&TextureData>,
        geometry: &Geometry<'_>,
        scale_x: f32,
        scale_y: f32,
    ) -> Result<()> {
        let count = geometry.count();
        let convert_color = self.rendering_linear_space();

        cmd.first = self.vertex_offset();
        cmd.count = count;

        for i in 0..count {
            let j = geometry.vertex(i);
            let (x, y) = geometry.xy(j);

            let mut col = geometry.color(j);
            if convert_color {
                self.convert_to_linear(&mut col);
            }

            let (u, v) = if !geometry.uv.is_empty() {
                geometry.uv(j)
            } else {
                (0.0, 0.0)
            };

            self.verts.extend_from_slice(&[
                x * scale_x,
                y * scale_y,
                col.r,
                col.g,
                col.b,
                col.a,
                u,
                v,
            ]);
        }
        Ok(())
    }

    /// Translation of `GPU_InvalidateCachedState()`.
    fn invalidate_cached_state(&mut self) {
        self.state.scissor_enabled = false;
    }

    // *** FIXME ***
    // We might be able to run these data uploads on a separate command buffer
    // which would allow us to avoid breaking render passes.
    // Honestly I'm a little skeptical of this entire approach,
    // we already have a command buffer structure
    // so it feels weird to be deferring the operations manually.
    // We could also fairly easily run the geometry transformations
    // on compute shaders instead of the CPU, which would be a HUGE performance win.
    // -cosmonaut
    /// Translation of `GPU_RunCommandQueue()`.
    fn run_command_queue(
        &mut self,
        cmds: &[RenderCommand],
        textures: &mut TextureStore,
    ) -> Result<()> {
        let mut command_buffer = self
            .state
            .command_buffer
            .take()
            .ok_or_else(no_command_buffer)?;
        let result = self.run_commands(&mut command_buffer, cmds, textures);
        self.state.command_buffer = Some(command_buffer);
        result
    }

    fn reset_vertices(&mut self) {
        self.verts.clear();
    }

    /// Translation of `GPU_CreatePalette()`.
    fn create_palette(&mut self) -> Result<Box<dyn Any>> {
        let tci = gpu::TextureCreateInfo {
            format: TextureFormat::from_pixel_format(PixelFormat::RGBA32)
                .unwrap_or(TextureFormat::INVALID),
            layer_count_or_depth: 1,
            num_levels: 1,
            usage: TextureUsageFlags::SAMPLER,
            width: 256,
            height: 1,
            sample_count: gpu::SampleCount::One,
            ..Default::default()
        };

        let texture = self.device.create_texture(&tci)?;
        Ok(Box::new(PaletteData {
            texture: Arc::new(texture),
        }))
    }

    /// Translation of `GPU_UpdatePalette()`.
    fn update_palette(&mut self, palette: &mut dyn Any, colors: &[Color]) -> Result<()> {
        let palettedata = palette
            .downcast_ref::<PaletteData>()
            .ok_or_else(|| Error::invalid_param("palette"))?;
        let ncolors = colors.len() as u32;
        let data_size = ncolors * 4;

        let mut tbuf = self
            .device
            .create_transfer_buffer(&gpu::TransferBufferCreateInfo {
                size: data_size,
                usage: gpu::TransferBufferUsage::Upload,
                props: None,
            })?;

        {
            let mut output = tbuf.map(false)?;
            for (out, c) in output.chunks_exact_mut(4).zip(colors) {
                out.copy_from_slice(&[c.r, c.g, c.b, c.a]);
            }
        }

        let cbuf = self
            .state
            .command_buffer
            .as_mut()
            .ok_or_else(no_command_buffer)?;
        let mut cpass = cbuf.begin_copy_pass()?;

        cpass.upload_to_texture(
            &gpu::TextureTransferInfo {
                transfer_buffer: &tbuf,
                offset: 0,
                rows_per_layer: 1,
                pixels_per_row: ncolors,
            },
            &gpu::TextureRegion {
                texture: &palettedata.texture,
                mip_level: 0,
                layer: 0,
                x: 0,
                y: 0,
                z: 0,
                w: ncolors,
                h: 1,
                d: 1,
            },
            false,
        );
        cpass.end();
        // (SDL_ReleaseGPUTransferBuffer())
        drop(tbuf);

        Ok(())
    }

    /// Translation of `GPU_DestroyPalette()`: dropping it releases its
    /// texture.
    fn destroy_palette(&mut self, palette: Box<dyn Any>) {
        drop(palette);
    }

    /// Record the texture's palette, which the draws bind
    /// (`texture->palette->internal`).
    fn change_texture_palette(
        &mut self,
        texture: &mut TextureData,
        palette: Option<&dyn Any>,
    ) -> Option<Result<()>> {
        let palette = palette
            .and_then(|p| p.downcast_ref::<PaletteData>())
            .map(|p| p.texture.clone());
        if let Ok(data) = texture_data_mut(texture) {
            data.palette = palette;
        }
        Some(Ok(()))
    }

    /// Translation of `GPU_UpdateTexture()`.
    fn update_texture(
        &mut self,
        texture: &mut TextureData,
        rect: &Rect,
        pixels: &[u8],
        pitch: usize,
    ) -> Result<()> {
        self.update_texture_pixels(texture, rect, pixels, pitch)
    }

    /// Translation of `GPU_UpdateTextureYUV()`.
    fn update_texture_yuv(
        &mut self,
        texture: &mut TextureData,
        rect: &Rect,
        (y_plane, y_pitch): (&[u8], usize),
        (u_plane, u_pitch): (&[u8], usize),
        (v_plane, v_pitch): (&[u8], usize),
    ) -> Option<Result<()>> {
        let data = match texture_data(texture) {
            Ok(data) => data,
            Err(e) => return Some(Err(e)),
        };
        let Planes::Yuv { u, v } = &data.planes else {
            return Some(Err(Error::invalid_param("texture")));
        };
        let device = &self.device;
        let Some(cbuf) = self.state.command_buffer.as_mut() else {
            return Some(Err(no_command_buffer()));
        };
        let bpp = texture.format.bytes_per_pixel() as usize;
        let full = (rect.x, rect.y, rect.w, rect.h);

        let mut cpass = match cbuf.begin_copy_pass() {
            Ok(cpass) => cpass,
            Err(e) => return Some(Err(e)),
        };
        let mut retval = update_texture_internal(
            device,
            &mut cpass,
            &data.texture,
            bpp,
            full,
            y_plane,
            y_pitch,
        );
        let chroma = if texture.format == PixelFormat::I444 || texture.format == PixelFormat::I4FL {
            full
        } else {
            chroma_rect(rect)
        };
        retval = retval.and(update_texture_internal(
            device, &mut cpass, u, bpp, chroma, u_plane, u_pitch,
        ));
        retval = retval.and(update_texture_internal(
            device, &mut cpass, v, bpp, chroma, v_plane, v_pitch,
        ));
        cpass.end();
        Some(retval)
    }

    /// Translation of `GPU_UpdateTextureNV()`.
    fn update_texture_nv(
        &mut self,
        texture: &mut TextureData,
        rect: &Rect,
        (y_plane, y_pitch): (&[u8], usize),
        (uv_plane, uv_pitch): (&[u8], usize),
    ) -> Option<Result<()>> {
        let data = match texture_data(texture) {
            Ok(data) => data,
            Err(e) => return Some(Err(e)),
        };
        let Planes::Nv(nv) = &data.planes else {
            return Some(Err(Error::invalid_param("texture")));
        };
        let device = &self.device;
        let Some(cbuf) = self.state.command_buffer.as_mut() else {
            return Some(Err(no_command_buffer()));
        };
        let mut bpp = texture.format.bytes_per_pixel() as usize;
        let full = (rect.x, rect.y, rect.w, rect.h);

        let mut cpass = match cbuf.begin_copy_pass() {
            Ok(cpass) => cpass,
            Err(e) => return Some(Err(e)),
        };
        let mut retval = update_texture_internal(
            device,
            &mut cpass,
            &data.texture,
            bpp,
            full,
            y_plane,
            y_pitch,
        );
        bpp *= 2;
        retval = retval.and(update_texture_internal(
            device,
            &mut cpass,
            nv,
            bpp,
            chroma_rect(rect),
            uv_plane,
            uv_pitch,
        ));
        cpass.end();
        Some(retval)
    }

    /// Translation of `GPU_LockTexture()`.
    fn lock_texture(&mut self, texture: &mut TextureData, rect: &Rect) -> Result<(usize, i32)> {
        let bpp = texture.format.bytes_per_pixel() as usize;
        let data = texture_data_mut(texture)?;

        data.locked_rect = *rect;
        let offset = rect.y as usize * data.pitch as usize + rect.x as usize * bpp;
        Ok((offset, data.pitch))
    }

    fn texture_pixels_mut<'t>(&mut self, texture: &'t mut TextureData) -> Option<&'t mut [u8]> {
        Some(&mut texture_data_mut(texture).ok()?.pixels[..])
    }

    /// Translation of `GPU_UnlockTexture()`.
    fn unlock_texture(&mut self, texture: &mut TextureData) {
        let bpp = texture.format.bytes_per_pixel() as usize;
        let Ok(data) = texture_data(texture) else {
            return;
        };

        let rect = data.locked_rect;
        let pitch = data.pitch as usize;
        let offset = rect.y as usize * pitch + rect.x as usize * bpp;
        if let Some(pixels) = data.pixels.get(offset..) {
            let _ = self.update_texture_pixels(texture, &rect, pixels, pitch);
        }
    }

    /// Translation of `GPU_SetRenderTarget()`.
    fn set_render_target(
        &mut self,
        target: Option<Texture>,
        textures: &TextureStore,
    ) -> Result<()> {
        self.state.render_target = target;
        // (renderer->target's colorspace, for SDL_RenderingLinearSpace())
        self.current_colorspace = target
            .and_then(|t| textures.get(t))
            .map_or(self.output_colorspace, |t| t.colorspace);

        Ok(())
    }

    /// Translation of `GPU_RenderReadPixels()`.
    fn read_pixels(
        &mut self,
        rect: &Rect,
        textures: &mut TextureStore,
    ) -> Option<Result<Surface<'static>>> {
        Some(self.render_read_pixels(rect, textures))
    }

    /// Translation of `GPU_RenderPresent()`.
    fn present(&mut self) -> bool {
        self.render_present();
        true
    }

    /// Translation of `GPU_DestroyTexture()`: dropping the data releases the
    /// GPU textures. (The front end unsets a target before destroying it,
    /// and a destroyed target's handle finds no texture.)
    fn destroy_texture(&mut self, texture: &mut TextureData) {
        texture.internal = None;
    }

    /// Translation of `GPU_SetVSync()`.
    fn set_vsync(&mut self, vsync: i32) -> Option<Result<()>> {
        // (there is always a window)
        let mode = match choose_present_mode(&self.device, &self.window, vsync) {
            Ok(mode) => mode,
            Err(e) => return Some(Err(e)),
        };

        if mode != self.swapchain.present_mode {
            // XXX returns bool instead of SDL-style error code
            if let Err(e) =
                self.device
                    .set_swapchain_parameters(&self.window, self.swapchain.composition, mode)
            {
                return Some(Err(e));
            }
            self.swapchain.present_mode = mode;
        }

        Some(Ok(()))
    }

    /// Translation of `GPU_DestroyRenderer()`. The device is destroyed once
    /// the renderer and everything made from it are gone.
    fn destroy(&mut self) {
        if self.destroyed {
            return;
        }
        self.destroyed = true;

        if let Some(command_buffer) = self.state.command_buffer.take() {
            let _ = command_buffer.cancel();
        }

        self.samplers = Default::default();

        self.backbuffer = None;

        self.device.release_window(&self.window);

        // ReleaseVertexBuffer()
        self.vertices = None;
        // GPU_DestroyPipelineCache()
        self.pipeline_cache = PipelineCache::default();

        // GPU_ReleaseShaders()
        self.shaders = None;
    }
}
