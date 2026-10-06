// Rust translation of src/render/vulkan/SDL_render_vulkan.c from Simple
// DirectMedia Layer, with the parts of src/render/SDL_d3dmath.h it uses.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! The Vulkan renderer ("vulkan"): draws through a Vulkan device and a
//! swapchain on the window's Vulkan surface. The Vulkan loader is the
//! video driver's (`SDL_Vulkan_LoadLibrary()`), and every Vulkan function
//! is looked up at run time through its `vkGetInstanceProcAddr`.
//!
//! What isn't translated:
//!
//! * the Android parts: textures of Android hardware buffers
//!   (`SDL_PIXELFORMAT_EXTERNAL_OES`) and the sampler YCbCr conversion
//!   pipelines they need. There is no Android platform layer.
//! * the creation options for an existing instance, surface, physical
//!   device, device or queue families (`SDL_PROP_RENDERER_CREATE_VULKAN_*`)
//!   and for existing images (`SDL_PROP_TEXTURE_CREATE_VULKAN_*`), which
//!   the front end doesn't take yet: the renderer makes and owns them all.
//!
//! The front end only makes renderers with sRGB output yet, so the linear
//! and HDR10 output paths here are kept for when it does.
//!
//! Upstream keeps the per-texture data in `texture->internal` and walks the
//! renderer's texture list to free it when the device is lost. Here the
//! renderer keeps its textures (and palettes) itself, and the front end's
//! texture refers to them by a key.

mod shaders;
#[cfg(test)]
mod tests;
mod vk;

use std::any::Any;
use std::collections::HashMap;
use std::ffi::{c_char, c_void, CStr, CString};
use std::ptr::{null, null_mut, NonNull};
use std::rc::Rc;

use shaders::Shader;
use vk::*;

use crate::error::{Error, Result};
use crate::events::window::WindowFlags;
use crate::events::{Event, EventType, RenderEvent};
use crate::hints;
use crate::log::Category;
use crate::properties::Properties;
use crate::render::sysrender::{
    invalid_texture, CopyEx, DrawCmd, DrawKind, Geometry, RenderBackend, RenderCommand,
    TextureCreateProps, TextureData, TextureStore,
};
use crate::render::{Texture, TextureAccess, TextureAddressMode};
use crate::video::blendmode::{BlendFactor, BlendOperation};
use crate::video::pixels::{
    convert_color_709_to_2020, pq_from_nits, srgb_to_linear, Color, ColorPrimaries, Colorspace,
    FColor, PixelFormat, TransferCharacteristics,
};
use crate::video::rect::{FPoint, FRect, Rect};
use crate::video::surface::{ScaleMode, Surface};
use crate::video::vulkan_utils::vulkan_get_result_string;
use crate::video::{BlendMode, Window};

/// The name of the Vulkan renderer (`VULKAN_RenderDriver.name`).
pub(crate) const VULKAN_RENDERER: &str = "vulkan";

/// The `VkInstance` of the renderer. Translation of
/// `SDL_PROP_RENDERER_VULKAN_INSTANCE_POINTER`.
pub const PROP_RENDERER_VULKAN_INSTANCE_POINTER: &str = "SDL.renderer.vulkan.instance";
/// The `VkSurfaceKHR` of the renderer. Translation of
/// `SDL_PROP_RENDERER_VULKAN_SURFACE_NUMBER`.
pub const PROP_RENDERER_VULKAN_SURFACE_NUMBER: &str = "SDL.renderer.vulkan.surface";
/// The `VkPhysicalDevice` of the renderer. Translation of
/// `SDL_PROP_RENDERER_VULKAN_PHYSICAL_DEVICE_POINTER`.
pub const PROP_RENDERER_VULKAN_PHYSICAL_DEVICE_POINTER: &str =
    "SDL.renderer.vulkan.physical_device";
/// The `VkDevice` of the renderer. Translation of
/// `SDL_PROP_RENDERER_VULKAN_DEVICE_POINTER`.
pub const PROP_RENDERER_VULKAN_DEVICE_POINTER: &str = "SDL.renderer.vulkan.device";
/// The queue family index used for rendering. Translation of
/// `SDL_PROP_RENDERER_VULKAN_GRAPHICS_QUEUE_FAMILY_INDEX_NUMBER`.
pub const PROP_RENDERER_VULKAN_GRAPHICS_QUEUE_FAMILY_INDEX_NUMBER: &str =
    "SDL.renderer.vulkan.graphics_queue_family_index";
/// The queue family index used for presentation. Translation of
/// `SDL_PROP_RENDERER_VULKAN_PRESENT_QUEUE_FAMILY_INDEX_NUMBER`.
pub const PROP_RENDERER_VULKAN_PRESENT_QUEUE_FAMILY_INDEX_NUMBER: &str =
    "SDL.renderer.vulkan.present_queue_family_index";
/// The number of swapchain images, the most frames in flight. Translation
/// of `SDL_PROP_RENDERER_VULKAN_SWAPCHAIN_IMAGE_COUNT_NUMBER`.
pub const PROP_RENDERER_VULKAN_SWAPCHAIN_IMAGE_COUNT_NUMBER: &str =
    "SDL.renderer.vulkan.swapchain_image_count";
/// The `VkImage` of a texture (its Y plane for YUV textures). Translation
/// of `SDL_PROP_TEXTURE_VULKAN_TEXTURE_NUMBER`.
pub const PROP_TEXTURE_VULKAN_TEXTURE_NUMBER: &str = "SDL.texture.vulkan.texture";
/// The `VkImage` of the U plane of a YUV texture. Translation of
/// `SDL_PROP_TEXTURE_VULKAN_TEXTURE_U_NUMBER`.
pub const PROP_TEXTURE_VULKAN_TEXTURE_U_NUMBER: &str = "SDL.texture.vulkan.texture_u";
/// The `VkImage` of the V plane of a YUV texture. Translation of
/// `SDL_PROP_TEXTURE_VULKAN_TEXTURE_V_NUMBER`.
pub const PROP_TEXTURE_VULKAN_TEXTURE_V_NUMBER: &str = "SDL.texture.vulkan.texture_v";

const SDL_VULKAN_FRAME_QUEUE_DEPTH: u32 = 2;
const SDL_VULKAN_NUM_VERTEX_BUFFERS: usize = 256;
const SDL_VULKAN_VERTEX_BUFFER_DEFAULT_SIZE: VkDeviceSize = 65536;
const SDL_VULKAN_CONSTANT_BUFFER_DEFAULT_SIZE: VkDeviceSize = 65536;
const SDL_VULKAN_NUM_UPLOAD_BUFFERS: usize = 32;
const SDL_VULKAN_MAX_DESCRIPTOR_SETS: u32 = 4096;
const SDL_VULKAN_NUM_TEXTURE_BINDINGS: usize = 3;

const SDL_VULKAN_VALIDATION_LAYER_NAME: &CStr = c"VK_LAYER_KHRONOS_validation";

const VK_KHR_SWAPCHAIN_EXTENSION_NAME: &CStr = c"VK_KHR_swapchain";
const VK_EXT_SWAPCHAIN_COLOR_SPACE_EXTENSION_NAME: &CStr = c"VK_EXT_swapchain_colorspace";
const VK_KHR_GET_PHYSICAL_DEVICE_PROPERTIES_2_EXTENSION_NAME: &CStr =
    c"VK_KHR_get_physical_device_properties2";
const VK_KHR_EXTERNAL_MEMORY_CAPABILITIES_EXTENSION_NAME: &CStr =
    c"VK_KHR_external_memory_capabilities";

/// The SDR white level of scRGB content (`SCRGB_NITS`).
const SCRGB_NITS: f32 = 80.0;

/// Translation of `SDL_clamp()` (`a` wins over `b` when they're the
/// wrong way around, where `Ord::clamp` would panic).
fn sdl_clamp(x: u32, a: u32, b: u32) -> u32 {
    if x < a {
        a
    } else if x > b {
        b
    } else {
        x
    }
}

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

/// Translation of `SET_ERROR_CODE()`: the error of a failed Vulkan call,
/// also logged (with a breakpoint) when [`hints::RENDER_VULKAN_DEBUG`] is set.
fn error_code(message: &str, rc: VkResult) -> Error {
    if hints::get_bool(hints::RENDER_VULKAN_DEBUG, false) {
        crate::log::error!(
            Category::Render,
            "{}: {}",
            message,
            vulkan_get_result_string(rc)
        );
        crate::assert::trigger_breakpoint();
    }
    Error::new(format!("{}: {}", message, vulkan_get_result_string(rc)))
}

/// Translation of `SET_ERROR_MESSAGE()`.
fn error_message(message: impl Into<String>) -> Error {
    let message = message.into();
    if hints::get_bool(hints::RENDER_VULKAN_DEBUG, false) {
        crate::log::error!(Category::Render, "{}", message);
        crate::assert::trigger_breakpoint();
    }
    Error::new(message)
}

/// `rc` as `Ok`, or the error of [`error_code`].
fn check(message: &str, rc: VkResult) -> Result<()> {
    if rc != VK_SUCCESS {
        return Err(error_code(message, rc));
    }
    Ok(())
}

/// A failed step: its `VkResult` (what the C function returns) and the
/// error it set.
type VkFailure = (VkResult, Error);

/// The outcome of a step whose `VkResult` the callers look at.
type VkResultOr<T> = std::result::Result<T, VkFailure>;

/// The failure of a Vulkan call (`SET_ERROR_CODE()` and its result).
fn failure(message: &str, rc: VkResult) -> VkFailure {
    (rc, error_code(message, rc))
}

/// Renderpass types. Translation of `VULKAN_RenderPass`.
#[derive(Clone, Copy)]
enum RenderPass {
    Load = 0,
    Clear = 1,
}

/// Translation of `VULKAN_RENDERPASS_COUNT`.
const VULKAN_RENDERPASS_COUNT: usize = 2;

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
/// `VULKAN_VertexShaderConstants`.
#[repr(C)]
#[derive(Clone, Copy, Default)]
struct VertexShaderConstants {
    model: Float4X4,
    projection_and_view: Float4X4,
}

// These should mirror the definitions in VULKAN_PixelShader_Common.hlsli
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

/// Pixel shader constants, common values. Translation of
/// `VULKAN_PixelShaderConstants`.
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

/// Per-vertex data. Translation of `VULKAN_VertexPositionColor`.
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

/// Vulkan Buffer. Translation of `VULKAN_Buffer`.
#[derive(Clone, Copy)]
struct Buffer {
    device_memory: VkDeviceMemory,
    buffer: VkBuffer,
    size: VkDeviceSize,
    mapped_buffer_ptr: *mut c_void,
}

impl Default for Buffer {
    fn default() -> Buffer {
        Buffer {
            device_memory: VK_NULL_HANDLE,
            buffer: VK_NULL_HANDLE,
            size: 0,
            mapped_buffer_ptr: null_mut(),
        }
    }
}

impl Buffer {
    /// The mapped memory of the buffer.
    fn mapped(&mut self) -> &mut [u8] {
        if self.mapped_buffer_ptr.is_null() {
            return &mut [];
        }
        // SAFETY: the buffer's memory is mapped from offset 0 for `size`
        // bytes (VULKAN_AllocateBuffer() maps it), until the buffer is
        // destroyed, and the borrow of the buffer keeps it alive.
        unsafe {
            std::slice::from_raw_parts_mut(self.mapped_buffer_ptr as *mut u8, self.size as usize)
        }
    }
}

/// Vulkan image. Translation of `VULKAN_Image`. (`allocatedImage` is always
/// true: the images of the creation options aren't taken.)
#[derive(Clone, Copy, Default)]
struct Image {
    image: VkImage,
    device_memory: VkDeviceMemory,
    image_layout: VkImageLayout,
    format: VkFormat,
}

/// Per-palette data. Translation of `VULKAN_PaletteData`.
#[derive(Default)]
struct PaletteData {
    image: Image,
    image_view: VkImageView,
}

/// The front end's handle of a palette: its key in the renderer.
struct PaletteRef(u32);

/// Per-texture data. Translation of `VULKAN_TextureData`.
#[derive(Default)]
struct VulkanTexture {
    num_images: usize,
    images: [Image; SDL_VULKAN_NUM_TEXTURE_BINDINGS],
    num_image_views: usize,
    image_views: [VkImageView; SDL_VULKAN_NUM_TEXTURE_BINDINGS],
    main_renderpasses: [VkRenderPass; VULKAN_RENDERPASS_COUNT],
    main_framebuffer: VkFramebuffer,
    staging_buffer: Buffer,
    width: i32,
    height: i32,
    ycbcr_matrix: Option<[f32; 16]>,

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
    /// The mapped staging buffer of a locked texture.
    staging: Option<(NonNull<u8>, usize)>,
}

fn texture_ref(texture: &TextureData) -> Option<&TextureRef> {
    texture.internal.as_ref()?.downcast_ref::<TextureRef>()
}

fn texture_ref_mut(texture: &mut TextureData) -> Option<&mut TextureRef> {
    texture.internal.as_mut()?.downcast_mut::<TextureRef>()
}

/// Pipeline State Object data. Translation of `VULKAN_PipelineState`.
struct PipelineState {
    shader: Shader,
    shader_constants: PixelShaderConstants,
    blend_mode: BlendMode,
    topology: VkPrimitiveTopology,
    format: VkFormat,
    pipeline_layout: VkPipelineLayout,
    descriptor_set_layout: VkDescriptorSetLayout,
    pipeline: VkPipeline,
}

/// Translation of `VULKAN_DrawStateCache`.
#[derive(Default)]
struct DrawStateCache {
    vertex_buffer: VkBuffer,
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

/// The renderer's data. Translation of `VULKAN_RenderData`, with the parts
/// of `SDL_Renderer` the backend reads (`window`, `output_colorspace`,
/// `current_colorspace`, the HDR headroom), its textures and palettes, and
/// its vertex storage.
pub(crate) struct VulkanRenderer {
    window: Window,
    output_colorspace: Colorspace,
    current_colorspace: Colorspace,
    /// `renderer->HDR_headroom`: 1 for the sRGB output the front end makes.
    hdr_headroom: f32,
    /// `renderer->target`, the front end's texture.
    target: Option<Texture>,

    vk_get_instance_proc_addr: Option<PfnVkGetInstanceProcAddr>,
    global: Option<Rc<GlobalFunctions>>,
    inst: Option<Rc<InstanceFunctions>>,
    dev: Option<Rc<DeviceFunctions>>,
    instance: VkInstance,
    surface: VkSurfaceKHR,
    physical_device: VkPhysicalDevice,
    physical_device_properties: Box<VkPhysicalDeviceProperties>,
    physical_device_memory_properties: Box<VkPhysicalDeviceMemoryProperties>,
    physical_device_features: VkPhysicalDeviceFeatures,
    graphics_queue: VkQueue,
    present_queue: VkQueue,
    device: VkDevice,
    graphics_queue_family_index: u32,
    present_queue_family_index: u32,
    swapchain: VkSwapchainKHR,
    command_pool: VkCommandPool,
    command_buffers: Vec<VkCommandBuffer>,
    current_command_buffer_index: u32,
    current_command_buffer: VkCommandBuffer,
    fences: Vec<VkFence>,
    surface_capabilities: VkSurfaceCapabilitiesKHR,
    surface_formats: Vec<VkSurfaceFormatKHR>,
    recreate_swapchain: bool,
    vsync: i32,

    framebuffers: Vec<VkFramebuffer>,
    render_passes: [VkRenderPass; VULKAN_RENDERPASS_COUNT],
    current_render_pass: VkRenderPass,

    vertex_shader_modules: [VkShaderModule; Shader::COUNT],
    fragment_shader_modules: [VkShaderModule; Shader::COUNT],
    descriptor_set_layout: VkDescriptorSetLayout,
    pipeline_layout: VkPipelineLayout,

    // Vertex buffer data
    vertex_buffers: Vec<Buffer>,
    vertex_shader_constants_data: VertexShaderConstants,

    // Data for staging/allocating textures
    upload_buffers: Vec<Vec<Buffer>>,
    current_upload_buffer: Vec<usize>,

    // Data for updating constants
    constant_buffers: Vec<Vec<Buffer>>,
    current_constant_buffer_index: usize,
    current_constant_buffer_offset: i32,

    samplers: [VkSampler; RENDER_SAMPLER_COUNT],
    descriptor_pools: Vec<Vec<VkDescriptorPool>>,
    current_descriptor_pool_index: usize,
    current_descriptor_set_index: u32,

    pipeline_states: Vec<PipelineState>,
    /// The index of the current pipeline state in `pipeline_states`.
    current_pipeline_state: Option<usize>,

    supports_ext_swapchain_colorspace: bool,
    supports_khr_get_physical_device_properties2: bool,
    supports_khr_sampler_ycbcr_conversion: bool,
    supports_khr_external_memory_capabilities: bool,
    swapchain_desired_image_count: u32,
    surface_format: VkSurfaceFormatKHR,
    swapchain_size: VkExtent2D,
    swap_chain_pre_transform: VkSurfaceTransformFlagBitsKHR,
    swapchain_image_count: u32,
    swapchain_images: Vec<VkImage>,
    swapchain_image_views: Vec<VkImageView>,
    swapchain_image_layouts: Vec<VkImageLayout>,
    image_available_semaphores: Vec<VkSemaphore>,
    rendering_finished_semaphores: Vec<VkSemaphore>,
    current_image_available_semaphore: VkSemaphore,
    current_swapchain_image_index: u32,

    wait_dest_stage_masks: Vec<VkPipelineStageFlags>,
    wait_render_semaphores: Vec<VkSemaphore>,
    signal_render_semaphores: Vec<VkSemaphore>,

    // Cached renderer properties
    /// The key of the target texture (`textureRenderTarget`).
    texture_render_target: Option<u32>,
    cliprect_dirty: bool,
    current_cliprect_enabled: bool,
    current_cliprect: Rect,
    current_viewport: Rect,
    current_viewport_rotation: VkSurfaceTransformFlagBitsKHR,
    viewport_dirty: bool,
    identity: Float4X4,
    identity_swizzle: VkComponentMapping,
    current_vertex_buffer: usize,
    issue_batch: bool,

    /// The textures, by the key their [`TextureRef`] holds.
    textures: HashMap<u32, VulkanTexture>,
    next_texture: u32,
    /// The palettes, by the key their [`PaletteRef`] holds.
    palettes: HashMap<u32, PaletteData>,
    next_palette: u32,
    /// The vertex data of the queued commands (`first` indexes it).
    verts: Vec<VertexPositionColor>,
    texture_formats: Vec<PixelFormat>,
    /// The renderer's properties, once the front end has made them.
    props: Option<Properties>,
}

// TODO: Sort this list based on what the Vulkan driver prefers?
/// Translation of `vk_format_map`: (SDL format, UNORM format, sRGB format).
///
/// FIXME (upstream): the `_EXT` formats are `VK_EXT_4444_formats`', which
/// the device is created without (the validation layers report every use).
const VK_FORMAT_MAP: &[(PixelFormat, VkFormat, VkFormat)] = &[
    (
        PixelFormat::BGRA32,
        VK_FORMAT_B8G8R8A8_UNORM,
        VK_FORMAT_B8G8R8A8_SRGB,
    ), // SDL_PIXELFORMAT_ARGB8888 on little endian systems
    (
        PixelFormat::RGBA32,
        VK_FORMAT_R8G8B8A8_UNORM,
        VK_FORMAT_R8G8B8A8_SRGB,
    ),
    #[cfg(target_endian = "big")]
    (
        PixelFormat::ABGR8888,
        VK_FORMAT_A8B8G8R8_UNORM_PACK32,
        VK_FORMAT_A8B8G8R8_SRGB_PACK32,
    ),
    (
        PixelFormat::ABGR2101010,
        VK_FORMAT_A2B10G10R10_UNORM_PACK32,
        VK_FORMAT_A2B10G10R10_UNORM_PACK32,
    ),
    (
        PixelFormat::RGBA64_FLOAT,
        VK_FORMAT_R16G16B16A16_SFLOAT,
        VK_FORMAT_R16G16B16A16_SFLOAT,
    ),
    (
        PixelFormat::RGB565,
        VK_FORMAT_R5G6B5_UNORM_PACK16,
        VK_FORMAT_R5G6B5_UNORM_PACK16,
    ),
    (
        PixelFormat::BGR565,
        VK_FORMAT_B5G6R5_UNORM_PACK16,
        VK_FORMAT_B5G6R5_UNORM_PACK16,
    ),
    (
        PixelFormat::RGBA5551,
        VK_FORMAT_R5G5B5A1_UNORM_PACK16,
        VK_FORMAT_R5G5B5A1_UNORM_PACK16,
    ),
    (
        PixelFormat::BGRA5551,
        VK_FORMAT_B5G5R5A1_UNORM_PACK16,
        VK_FORMAT_B5G5R5A1_UNORM_PACK16,
    ),
    (
        PixelFormat::ARGB1555,
        VK_FORMAT_A1R5G5B5_UNORM_PACK16,
        VK_FORMAT_A1R5G5B5_UNORM_PACK16,
    ),
    (
        PixelFormat::RGBA4444,
        VK_FORMAT_R4G4B4A4_UNORM_PACK16,
        VK_FORMAT_R4G4B4A4_UNORM_PACK16,
    ),
    (
        PixelFormat::BGRA4444,
        VK_FORMAT_B4G4R4A4_UNORM_PACK16,
        VK_FORMAT_B4G4R4A4_UNORM_PACK16,
    ),
    (
        PixelFormat::ARGB4444,
        VK_FORMAT_A4R4G4B4_UNORM_PACK16_EXT,
        VK_FORMAT_A4R4G4B4_UNORM_PACK16_EXT,
    ),
    (
        PixelFormat::ABGR4444,
        VK_FORMAT_A4B4G4R4_UNORM_PACK16_EXT,
        VK_FORMAT_A4B4G4R4_UNORM_PACK16_EXT,
    ),
];

/// Translation of `VULKAN_VkFormatToSDLPixelFormat()`.
fn vk_format_to_sdl_pixel_format(vk_format: VkFormat) -> PixelFormat {
    for &(sdl, unorm, srgb) in VK_FORMAT_MAP {
        if unorm == vk_format || srgb == vk_format {
            return sdl;
        }
    }
    PixelFormat::UNKNOWN
}

/// Translation of `VULKAN_GetFormatImageCount()`.
fn get_format_image_count(format: PixelFormat) -> usize {
    use PixelFormat as F;
    match format {
        F::YV12 | F::IYUV | F::I444 | F::I0FL | F::I4FL => 3,
        _ => 1,
    }
}

/// Translation of `VULKAN_GetFormatImageViewCount()`.
fn get_format_image_view_count(format: PixelFormat) -> usize {
    use PixelFormat as F;
    match format {
        F::NV12 | F::NV21 | F::P010 => 2,
        F::YV12 | F::IYUV | F::I444 | F::I0FL | F::I4FL => 3,
        _ => 1,
    }
}

/// Translation of `VULKAN_VkFormatGetNumPlanes()`.
fn vk_format_get_num_planes(vk_format: VkFormat) -> u32 {
    match vk_format {
        VK_FORMAT_G8_B8_R8_3PLANE_420_UNORM
        | VK_FORMAT_G8_B8_R8_3PLANE_444_UNORM
        | VK_FORMAT_G16_B16_R16_3PLANE_420_UNORM
        | VK_FORMAT_G16_B16_R16_3PLANE_444_UNORM => 3,
        VK_FORMAT_G8_B8R8_2PLANE_420_UNORM
        | VK_FORMAT_G10X6_B10X6R10X6_2PLANE_420_UNORM_3PACK16 => 2,
        _ => 1,
    }
}

/// Translation of `VULKAN_GetBytesPerPixel()`.
fn get_bytes_per_pixel(vk_format: VkFormat, plane: u32) -> VkDeviceSize {
    match vk_format {
        VK_FORMAT_R8_UNORM => 1,
        VK_FORMAT_R8G8_UNORM => 2,
        VK_FORMAT_R16G16_UNORM => 4,
        VK_FORMAT_G8_B8_R8_3PLANE_420_UNORM | VK_FORMAT_G8_B8_R8_3PLANE_444_UNORM => 1,
        VK_FORMAT_G16_B16_R16_3PLANE_420_UNORM | VK_FORMAT_G16_B16_R16_3PLANE_444_UNORM => 2,
        VK_FORMAT_G8_B8R8_2PLANE_420_UNORM => {
            if plane == 0 {
                1
            } else {
                2
            }
        }
        VK_FORMAT_G10X6_B10X6R10X6_2PLANE_420_UNORM_3PACK16 => {
            if plane == 0 {
                2
            } else {
                4
            }
        }
        _ => vk_format_to_sdl_pixel_format(vk_format).bytes_per_pixel() as VkDeviceSize,
    }
}

/// Translation of `VULKAN_GetVkImageFormat()`.
fn get_vk_image_format(format: PixelFormat, output_colorspace: Colorspace) -> VkFormat {
    use PixelFormat as F;
    match format {
        F::INDEX8 | F::YV12 | F::IYUV | F::I444 => VK_FORMAT_R8_UNORM,
        F::NV12 | F::NV21 => VK_FORMAT_G8_B8R8_2PLANE_420_UNORM,
        F::P010 => VK_FORMAT_G10X6_B10X6R10X6_2PLANE_420_UNORM_3PACK16,
        F::I0FL | F::I4FL => VK_FORMAT_R16_UNORM,
        _ => {
            for &(sdl, unorm, srgb) in VK_FORMAT_MAP {
                if sdl == format {
                    if output_colorspace == Colorspace::SRGB_LINEAR
                        || output_colorspace == Colorspace::HDR10
                    {
                        return srgb;
                    } else {
                        return unorm;
                    }
                }
            }
            VK_FORMAT_UNDEFINED
        }
    }
}

/// Translation of `VULKAN_GetVkImageViewFormat()`.
fn get_vk_image_view_format(
    format: PixelFormat,
    plane: usize,
    output_colorspace: Colorspace,
) -> VkFormat {
    use PixelFormat as F;
    match format {
        F::NV12 | F::NV21 => {
            if plane == 0 {
                VK_FORMAT_R8_UNORM
            } else {
                VK_FORMAT_R8G8_UNORM
            }
        }
        F::P010 => {
            if plane == 0 {
                VK_FORMAT_R16_UNORM
            } else {
                VK_FORMAT_R16G16_UNORM
            }
        }
        _ => get_vk_image_format(format, output_colorspace),
    }
}

/// Translation of `GetBlendFactor()`.
fn get_blend_factor(factor: Option<BlendFactor>) -> VkBlendFactor {
    match factor {
        Some(BlendFactor::Zero) => VK_BLEND_FACTOR_ZERO,
        Some(BlendFactor::One) => VK_BLEND_FACTOR_ONE,
        Some(BlendFactor::SrcColor) => VK_BLEND_FACTOR_SRC_COLOR,
        Some(BlendFactor::OneMinusSrcColor) => VK_BLEND_FACTOR_ONE_MINUS_SRC_COLOR,
        Some(BlendFactor::SrcAlpha) => VK_BLEND_FACTOR_SRC_ALPHA,
        Some(BlendFactor::OneMinusSrcAlpha) => VK_BLEND_FACTOR_ONE_MINUS_SRC_ALPHA,
        Some(BlendFactor::DstColor) => VK_BLEND_FACTOR_DST_COLOR,
        Some(BlendFactor::OneMinusDstColor) => VK_BLEND_FACTOR_ONE_MINUS_DST_COLOR,
        Some(BlendFactor::DstAlpha) => VK_BLEND_FACTOR_DST_ALPHA,
        Some(BlendFactor::OneMinusDstAlpha) => VK_BLEND_FACTOR_ONE_MINUS_DST_ALPHA,
        None => VK_BLEND_FACTOR_MAX_ENUM,
    }
}

/// Translation of `GetBlendOp()`.
fn get_blend_op(operation: Option<BlendOperation>) -> VkBlendOp {
    match operation {
        Some(BlendOperation::Add) => VK_BLEND_OP_ADD,
        Some(BlendOperation::Subtract) => VK_BLEND_OP_SUBTRACT,
        Some(BlendOperation::RevSubtract) => VK_BLEND_OP_REVERSE_SUBTRACT,
        Some(BlendOperation::Minimum) => VK_BLEND_OP_MIN,
        Some(BlendOperation::Maximum) => VK_BLEND_OP_MAX,
        None => VK_BLEND_OP_MAX_ENUM,
    }
}

/// Translation of `VULKAN_IsDisplayRotated90Degrees()`.
fn is_display_rotated_90_degrees(rotation: VkSurfaceTransformFlagBitsKHR) -> bool {
    matches!(
        rotation,
        VK_SURFACE_TRANSFORM_ROTATE_90_BIT_KHR | VK_SURFACE_TRANSFORM_ROTATE_270_BIT_KHR
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

/// The extension names of a `vkEnumerate*ExtensionProperties()` call:
/// the count with a null array, then the properties.
fn enumerate_extensions(
    mut enumerate: impl FnMut(*mut u32, *mut VkExtensionProperties) -> VkResult,
    what: &str,
) -> Result<Vec<VkExtensionProperties>> {
    let mut count = 0;
    check(what, enumerate(&mut count, null_mut()))?;
    let mut properties = vec![VkExtensionProperties::default(); count as usize];
    if count > 0 {
        check(what, enumerate(&mut count, properties.as_mut_ptr()))?;
    }
    properties.truncate(count as usize);
    Ok(properties)
}

/// Whether `properties` has the extension `name`.
fn has_extension(properties: &[VkExtensionProperties], name: &CStr) -> bool {
    properties.iter().any(|p| {
        // SAFETY: the names are NUL-terminated within their arrays (the
        // arrays started zeroed).
        (unsafe { CStr::from_ptr(p.extension_name.as_ptr()) }) == name
    })
}

impl VulkanRenderer {
    /// The instance functions (loaded once the instance exists).
    fn inst(&self) -> Rc<InstanceFunctions> {
        self.inst.clone().expect("Vulkan instance functions")
    }

    /// The device functions (loaded once the device exists).
    fn dev(&self) -> Rc<DeviceFunctions> {
        self.dev.clone().expect("Vulkan device functions")
    }

    /// The error of a renderer whose device was lost and not recovered.
    ///
    /// Note (upstream): only some functions check for it there; the others
    /// go on with the destroyed device (and its null arrays).
    fn check_device(&self) -> Result<()> {
        if self.device.is_null() || self.command_buffers.is_empty() {
            return Err(Error::new("Device lost and couldn't be recovered"));
        }
        Ok(())
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

    /// Translation of `VULKAN_DestroyAll()`.
    fn destroy_all(&mut self) {
        // Release all textures
        let ids: Vec<u32> = self.textures.keys().copied().collect();
        for id in ids {
            self.destroy_texture_data(id);
        }

        // (VULKAN_CleanupYUVPipeline(): the YUV pipelines are Android's)

        self.wait_dest_stage_masks = Vec::new();
        self.wait_render_semaphores = Vec::new();
        self.signal_render_semaphores = Vec::new();
        self.surface_formats = Vec::new();
        self.swapchain_images = Vec::new();

        let Some(dev) = self.dev.clone() else {
            // (the device, if it was created, has no device functions)
            self.destroy_device_without_functions();
            self.destroy_instance_objects();
            return;
        };
        let device = self.device;
        // SAFETY (for the rest of the function): each object destroyed was
        // created on this device, isn't used by pending GPU work (the
        // callers wait for the GPU first or never submitted) and is
        // forgotten as it's destroyed.
        unsafe {
            if self.swapchain != VK_NULL_HANDLE {
                (dev.destroy_swapchain_khr)(device, self.swapchain, null());
                self.swapchain = VK_NULL_HANDLE;
            }
            for fence in self.fences.drain(..) {
                if fence != VK_NULL_HANDLE {
                    (dev.destroy_fence)(device, fence, null());
                }
            }
            for view in self.swapchain_image_views.drain(..) {
                if view != VK_NULL_HANDLE {
                    (dev.destroy_image_view)(device, view, null());
                }
            }
            self.swapchain_image_layouts = Vec::new();
            for framebuffer in self.framebuffers.drain(..) {
                if framebuffer != VK_NULL_HANDLE {
                    (dev.destroy_framebuffer)(device, framebuffer, null());
                }
            }
            for sampler in &mut self.samplers {
                if *sampler != VK_NULL_HANDLE {
                    (dev.destroy_sampler)(device, *sampler, null());
                    *sampler = VK_NULL_HANDLE;
                }
            }
        }
        let mut vertex_buffers = std::mem::take(&mut self.vertex_buffers);
        for buffer in &mut vertex_buffers {
            self.destroy_buffer(buffer);
        }
        // SAFETY: as above.
        unsafe {
            for render_pass in &mut self.render_passes {
                if *render_pass != VK_NULL_HANDLE {
                    (dev.destroy_render_pass)(device, *render_pass, null());
                    *render_pass = VK_NULL_HANDLE;
                }
            }
            for semaphore in self.image_available_semaphores.drain(..) {
                if semaphore != VK_NULL_HANDLE {
                    (dev.destroy_semaphore)(device, semaphore, null());
                }
            }
            for semaphore in self.rendering_finished_semaphores.drain(..) {
                if semaphore != VK_NULL_HANDLE {
                    (dev.destroy_semaphore)(device, semaphore, null());
                }
            }
            if !self.command_buffers.is_empty() {
                (dev.free_command_buffers)(
                    device,
                    self.command_pool,
                    self.command_buffers.len() as u32,
                    self.command_buffers.as_ptr(),
                );
                self.command_buffers = Vec::new();
                self.current_command_buffer = null_mut();
                self.current_command_buffer_index = 0;
            }
            if self.command_pool != VK_NULL_HANDLE {
                (dev.destroy_command_pool)(device, self.command_pool, null());
                self.command_pool = VK_NULL_HANDLE;
            }
            for pools in self.descriptor_pools.drain(..) {
                for pool in pools {
                    if pool != VK_NULL_HANDLE {
                        (dev.destroy_descriptor_pool)(device, pool, null());
                    }
                }
            }
            for i in 0..Shader::COUNT {
                if self.vertex_shader_modules[i] != VK_NULL_HANDLE {
                    (dev.destroy_shader_module)(device, self.vertex_shader_modules[i], null());
                    self.vertex_shader_modules[i] = VK_NULL_HANDLE;
                }
                if self.fragment_shader_modules[i] != VK_NULL_HANDLE {
                    (dev.destroy_shader_module)(device, self.fragment_shader_modules[i], null());
                    self.fragment_shader_modules[i] = VK_NULL_HANDLE;
                }
            }
            if self.descriptor_set_layout != VK_NULL_HANDLE {
                (dev.destroy_descriptor_set_layout)(device, self.descriptor_set_layout, null());
                self.descriptor_set_layout = VK_NULL_HANDLE;
            }
            if self.pipeline_layout != VK_NULL_HANDLE {
                (dev.destroy_pipeline_layout)(device, self.pipeline_layout, null());
                self.pipeline_layout = VK_NULL_HANDLE;
            }
            for state in self.pipeline_states.drain(..) {
                (dev.destroy_pipeline)(device, state.pipeline, null());
            }
            self.current_pipeline_state = None;
        }

        let mut upload_buffers = std::mem::take(&mut self.upload_buffers);
        for (i, buffers) in upload_buffers.iter_mut().enumerate() {
            let used = self.current_upload_buffer.get(i).copied().unwrap_or(0);
            for buffer in &mut buffers[..used] {
                self.destroy_buffer(buffer);
            }
        }
        self.current_upload_buffer = Vec::new();

        let mut constant_buffers = std::mem::take(&mut self.constant_buffers);
        for buffers in &mut constant_buffers {
            for buffer in buffers {
                self.destroy_buffer(buffer);
            }
        }

        if !self.device.is_null() {
            // SAFETY: the device has no child objects left that the
            // renderer made (bar the palettes, see below).
            unsafe { (dev.destroy_device)(self.device, null()) };
            self.device = null_mut();
        }
        self.dev = None;
        // FIXME (upstream): the palettes' images and views aren't
        // released; after a device loss, they belong to the lost device.
        self.destroy_instance_objects();
    }

    /// The device, when it was made but its functions couldn't be looked
    /// up: destroyed through its own `vkDestroyDevice`.
    fn destroy_device_without_functions(&mut self) {
        if self.device.is_null() {
            return;
        }
        type PfnVkDestroyDevice = unsafe extern "system" fn(VkDevice, *const c_void);
        if let Some(inst) = &self.inst {
            // SAFETY: the instance's vkGetDeviceProcAddr; the function's
            // type.
            let f =
                unsafe { (inst.get_device_proc_addr)(self.device, c"vkDestroyDevice".as_ptr()) };
            if let Some(f) = f {
                // SAFETY: vkDestroyDevice has this type; the device has
                // no child objects (nothing was made without functions).
                unsafe {
                    let destroy: PfnVkDestroyDevice = std::mem::transmute(f);
                    destroy(self.device, null());
                }
            }
        }
        self.device = null_mut();
    }

    /// The end of `VULKAN_DestroyAll()`: the surface and the instance.
    /// (Without the instance functions, upstream calls a null
    /// `vkDestroyInstance`; here it's looked up on its own.)
    fn destroy_instance_objects(&mut self) {
        if self.inst.is_none() && !self.instance.is_null() {
            type PfnVkDestroyInstance = unsafe extern "system" fn(VkInstance, *const c_void);
            if let Some(f) = self.instance_lookup(self.instance)(c"vkDestroyInstance") {
                // SAFETY: vkDestroyInstance has this type; the instance
                // has no child objects (nothing was made without the
                // instance functions).
                unsafe {
                    let destroy: PfnVkDestroyInstance = std::mem::transmute(f);
                    destroy(self.instance, null());
                }
            }
            self.instance = null_mut();
        }
        if let Some(inst) = self.inst.clone() {
            if self.surface != VK_NULL_HANDLE {
                // SAFETY: the surface of this instance, with no swapchain left.
                unsafe { (inst.destroy_surface_khr)(self.instance, self.surface, null()) };
                self.surface = VK_NULL_HANDLE;
            }
            if !self.instance.is_null() {
                // SAFETY: the instance has no child objects left.
                unsafe { (inst.destroy_instance)(self.instance, null()) };
                self.instance = null_mut();
            }
        }
        self.inst = None;
    }

    /// Translation of `VULKAN_DestroyBuffer()`.
    fn destroy_buffer(&self, vulkan_buffer: &mut Buffer) {
        let Some(dev) = &self.dev else {
            *vulkan_buffer = Buffer::default();
            return;
        };
        // SAFETY: the buffer and its memory are this device's, and not in
        // use by pending GPU work.
        unsafe {
            if vulkan_buffer.buffer != VK_NULL_HANDLE {
                (dev.destroy_buffer)(self.device, vulkan_buffer.buffer, null());
            }
            if vulkan_buffer.device_memory != VK_NULL_HANDLE {
                (dev.free_memory)(self.device, vulkan_buffer.device_memory, null());
            }
        }
        *vulkan_buffer = Buffer::default();
    }

    /// Translation of `VULKAN_AllocateBuffer()`.
    fn allocate_buffer(
        &mut self,
        size: VkDeviceSize,
        usage: VkBufferUsageFlags,
        required_memory_props: VkMemoryPropertyFlags,
        desired_memory_props: VkMemoryPropertyFlags,
    ) -> VkResultOr<Buffer> {
        let dev = self.dev();
        let mut buffer_out = Buffer::default();
        let buffer_create_info = VkBufferCreateInfo {
            s_type: VK_STRUCTURE_TYPE_BUFFER_CREATE_INFO,
            size,
            usage,
            ..Default::default()
        };
        // SAFETY: a valid create info.
        let result = unsafe {
            (dev.create_buffer)(
                self.device,
                &buffer_create_info,
                null(),
                &mut buffer_out.buffer,
            )
        };
        if result != VK_SUCCESS {
            return Err(failure("vkCreateBuffer()", result));
        }

        let mut memory_requirements = VkMemoryRequirements::default();
        // SAFETY: the buffer just made.
        unsafe {
            (dev.get_buffer_memory_requirements)(
                self.device,
                buffer_out.buffer,
                &mut memory_requirements,
            )
        };
        // (upstream checks the stale result of vkCreateBuffer() here)

        let memory_type_index = match self.find_memory_type_index(
            memory_requirements.memory_type_bits,
            required_memory_props,
            desired_memory_props,
        ) {
            Ok(index) => index,
            Err(e) => {
                self.destroy_buffer(&mut buffer_out);
                return Err((VK_ERROR_UNKNOWN, e));
            }
        };

        let memory_allocate_info = VkMemoryAllocateInfo {
            s_type: VK_STRUCTURE_TYPE_MEMORY_ALLOCATE_INFO,
            allocation_size: memory_requirements.size,
            memory_type_index,
            ..Default::default()
        };
        // SAFETY: a valid allocate info.
        let result = unsafe {
            (dev.allocate_memory)(
                self.device,
                &memory_allocate_info,
                null(),
                &mut buffer_out.device_memory,
            )
        };
        if result != VK_SUCCESS {
            self.destroy_buffer(&mut buffer_out);
            return Err(failure("vkAllocateMemory()", result));
        }
        // SAFETY: the buffer and memory just made.
        let result = unsafe {
            (dev.bind_buffer_memory)(self.device, buffer_out.buffer, buffer_out.device_memory, 0)
        };
        if result != VK_SUCCESS {
            self.destroy_buffer(&mut buffer_out);
            return Err(failure("vkBindBufferMemory()", result));
        }

        // SAFETY: host visible memory (the required properties of every
        // caller), mapped once.
        let result = unsafe {
            (dev.map_memory)(
                self.device,
                buffer_out.device_memory,
                0,
                size,
                0,
                &mut buffer_out.mapped_buffer_ptr,
            )
        };
        if result != VK_SUCCESS {
            self.destroy_buffer(&mut buffer_out);
            return Err(failure("vkMapMemory()", result));
        }
        buffer_out.size = size;
        Ok(buffer_out)
    }

    /// Translation of `VULKAN_DestroyImage()`.
    fn destroy_image(&self, vulkan_image: &mut Image) {
        let dev = self.dev();
        // SAFETY: the image and its memory are this device's, and not in
        // use by pending GPU work.
        unsafe {
            if vulkan_image.image != VK_NULL_HANDLE {
                (dev.destroy_image)(self.device, vulkan_image.image, null());
                vulkan_image.image = VK_NULL_HANDLE;
            }

            if vulkan_image.device_memory != VK_NULL_HANDLE {
                (dev.free_memory)(self.device, vulkan_image.device_memory, null());
                vulkan_image.device_memory = VK_NULL_HANDLE;
            }
        }
    }

    /// Translation of `VULKAN_AllocateImage()` (without the image of a
    /// creation property, nor an external buffer).
    fn allocate_image(
        &mut self,
        width: u32,
        height: u32,
        format: VkFormat,
        image_usage: VkImageUsageFlags,
    ) -> VkResultOr<Image> {
        let dev = self.dev();
        let mut image_out = Image {
            format,
            ..Image::default()
        };

        let mut image_create_info = VkImageCreateInfo {
            s_type: VK_STRUCTURE_TYPE_IMAGE_CREATE_INFO,
            flags: 0,
            image_type: VK_IMAGE_TYPE_2D,
            format,
            extent: VkExtent3D {
                width,
                height,
                depth: 1,
            },
            mip_levels: 1,
            array_layers: 1,
            samples: VK_SAMPLE_COUNT_1_BIT,
            tiling: VK_IMAGE_TILING_OPTIMAL,
            usage: image_usage,
            sharing_mode: VK_SHARING_MODE_EXCLUSIVE,
            queue_family_index_count: 0,
            initial_layout: VK_IMAGE_LAYOUT_UNDEFINED,
            ..Default::default()
        };

        // FIXME (upstream): always true (a format has at least one plane),
        // so every image is created with VK_IMAGE_CREATE_MUTABLE_FORMAT_BIT.
        if vk_format_get_num_planes(format) > 0 {
            // We'll take image views with a different format
            image_create_info.flags |= VK_IMAGE_CREATE_MUTABLE_FORMAT_BIT;
        }

        // SAFETY: a valid create info.
        let result = unsafe {
            (dev.create_image)(
                self.device,
                &image_create_info,
                null(),
                &mut image_out.image,
            )
        };
        if result != VK_SUCCESS {
            self.destroy_image(&mut image_out);
            return Err(failure("vkCreateImage()", result));
        }

        let mut memory_requirements = VkMemoryRequirements::default();
        // SAFETY: the image just made.
        unsafe {
            (dev.get_image_memory_requirements)(
                self.device,
                image_out.image,
                &mut memory_requirements,
            )
        };
        // (upstream checks the stale result of vkCreateImage() here)

        let memory_type_index = match self.find_memory_type_index(
            memory_requirements.memory_type_bits,
            0,
            VK_MEMORY_PROPERTY_DEVICE_LOCAL_BIT,
        ) {
            Ok(index) => index,
            Err(e) => {
                self.destroy_image(&mut image_out);
                return Err((VK_ERROR_UNKNOWN, e));
            }
        };

        let memory_allocate_info = VkMemoryAllocateInfo {
            s_type: VK_STRUCTURE_TYPE_MEMORY_ALLOCATE_INFO,
            allocation_size: memory_requirements.size,
            memory_type_index,
            ..Default::default()
        };
        // SAFETY: a valid allocate info.
        let result = unsafe {
            (dev.allocate_memory)(
                self.device,
                &memory_allocate_info,
                null(),
                &mut image_out.device_memory,
            )
        };
        if result != VK_SUCCESS {
            self.destroy_image(&mut image_out);
            return Err(failure("vkAllocateMemory()", result));
        }
        // SAFETY: the image and memory just made.
        let result = unsafe {
            (dev.bind_image_memory)(self.device, image_out.image, image_out.device_memory, 0)
        };
        if result != VK_SUCCESS {
            self.destroy_image(&mut image_out);
            return Err(failure("vkBindImageMemory()", result));
        }

        if image_out.image_layout != VK_IMAGE_LAYOUT_SHADER_READ_ONLY_OPTIMAL {
            self.ensure_command_buffer();
            image_out.image_layout = self.record_pipeline_image_barrier(
                VK_ACCESS_NONE,
                VK_ACCESS_SHADER_READ_BIT,
                VK_PIPELINE_STAGE_FRAGMENT_SHADER_BIT,
                VK_PIPELINE_STAGE_FRAGMENT_SHADER_BIT,
                VK_IMAGE_LAYOUT_SHADER_READ_ONLY_OPTIMAL,
                image_out.image,
                image_out.image_layout,
            );
        }

        Ok(image_out)
    }

    /// Translation of `VULKAN_AllocateImageView()` (without a YCbCr
    /// conversion: those are Android's). `plane` is `None` for the color
    /// aspect.
    fn allocate_image_view(
        &self,
        image: VkImage,
        plane: Option<usize>,
        format: VkFormat,
        image_usage: VkImageUsageFlags,
        swizzle: VkComponentMapping,
    ) -> VkResultOr<VkImageView> {
        let aspect_mask = match plane {
            None => VK_IMAGE_ASPECT_COLOR_BIT,
            Some(plane) => VK_IMAGE_ASPECT_PLANE_0_BIT << plane,
        };

        let image_view_usage_create_info = VkImageViewUsageCreateInfo {
            s_type: VK_STRUCTURE_TYPE_IMAGE_VIEW_USAGE_CREATE_INFO,
            usage: image_usage,
            ..Default::default()
        };
        let image_view_create_info = VkImageViewCreateInfo {
            s_type: VK_STRUCTURE_TYPE_IMAGE_VIEW_CREATE_INFO,
            p_next: &image_view_usage_create_info as *const _ as *const c_void,
            image,
            view_type: VK_IMAGE_VIEW_TYPE_2D,
            format,
            components: swizzle,
            subresource_range: VkImageSubresourceRange {
                aspect_mask,
                base_mip_level: 0,
                level_count: 1,
                base_array_layer: 0,
                layer_count: 1,
            },
            ..Default::default()
        };

        let mut image_view_out = VK_NULL_HANDLE;
        // SAFETY: a valid create info, its chain outliving the call.
        let result = unsafe {
            (self.dev().create_image_view)(
                self.device,
                &image_view_create_info,
                null(),
                &mut image_view_out,
            )
        };
        if result != VK_SUCCESS {
            return Err(failure("vkCreateImageView()", result));
        }
        Ok(image_view_out)
    }

    /// End the render pass, if one is open (the start of the barrier
    /// functions).
    fn end_render_pass(&mut self) {
        // Stop any outstanding renderpass if open
        if self.current_render_pass != VK_NULL_HANDLE {
            // SAFETY: the command buffer is recording, in a render pass.
            unsafe { (self.dev().cmd_end_render_pass)(self.current_command_buffer) };
            self.current_render_pass = VK_NULL_HANDLE;
        }
    }

    /// Translation of `VULKAN_RecordPipelineImageBarrier()`: the image goes
    /// from `image_layout` to `dest_layout`, which is returned.
    #[allow(clippy::too_many_arguments)]
    fn record_pipeline_image_barrier(
        &mut self,
        source_access_mask: VkAccessFlags,
        dest_access_mask: VkAccessFlags,
        src_stage_flags: VkPipelineStageFlags,
        dst_stage_flags: VkPipelineStageFlags,
        dest_layout: VkImageLayout,
        image: VkImage,
        image_layout: VkImageLayout,
    ) -> VkImageLayout {
        self.end_render_pass();

        let barrier = VkImageMemoryBarrier {
            s_type: VK_STRUCTURE_TYPE_IMAGE_MEMORY_BARRIER,
            src_access_mask: source_access_mask,
            dst_access_mask: dest_access_mask,
            old_layout: image_layout,
            new_layout: dest_layout,
            src_queue_family_index: VK_QUEUE_FAMILY_IGNORED,
            dst_queue_family_index: VK_QUEUE_FAMILY_IGNORED,
            image,
            subresource_range: VkImageSubresourceRange {
                aspect_mask: VK_IMAGE_ASPECT_COLOR_BIT,
                base_mip_level: 0,
                level_count: 1,
                base_array_layer: 0,
                layer_count: 1,
            },
            ..Default::default()
        };
        // SAFETY: the command buffer is recording, outside a render pass.
        unsafe {
            (self.dev().cmd_pipeline_barrier)(
                self.current_command_buffer,
                src_stage_flags,
                dst_stage_flags,
                0,
                0,
                null(),
                0,
                null(),
                1,
                &barrier,
            )
        };
        dest_layout
    }

    /// The barrier of `VULKAN_RecordExternalImageBarrier()` is only for
    /// external (Android) images.
    ///
    /// Translation of `VULKAN_AcquireNextSwapchainImage()`.
    fn acquire_next_swapchain_image(&mut self) -> VkResult {
        self.current_image_available_semaphore = VK_NULL_HANDLE;
        let index = self.current_command_buffer_index as usize;
        // SAFETY: the swapchain and semaphore are this device's.
        let mut result = unsafe {
            (self.dev().acquire_next_image_khr)(
                self.device,
                self.swapchain,
                u64::MAX,
                self.image_available_semaphores[index],
                VK_NULL_HANDLE,
                &mut self.current_swapchain_image_index,
            )
        };
        if result == VK_ERROR_OUT_OF_DATE_KHR || result == VK_ERROR_SURFACE_LOST_KHR {
            if !self
                .window
                .flags()
                .unwrap_or_default()
                .contains(WindowFlags::MINIMIZED)
            {
                result = match self.create_window_size_dependent_resources() {
                    Ok(()) => VK_SUCCESS,
                    Err((rc, _)) => rc,
                };
            }
            return result;
        } else if result == VK_SUBOPTIMAL_KHR {
            // Suboptimal, but we can continue
        } else if result != VK_SUCCESS {
            let _ = error_code("vkAcquireNextImageKHR()", result);
            return result;
        }
        self.current_image_available_semaphore = self.image_available_semaphores[index];
        result
    }

    /// The width, height, render passes and framebuffer drawn into: the
    /// target texture's or the swapchain's.
    fn render_target_size(&self) -> (u32, u32) {
        match self
            .texture_render_target
            .and_then(|id| self.textures.get(&id))
        {
            Some(t) => (t.width as u32, t.height as u32),
            None => (self.swapchain_size.width, self.swapchain_size.height),
        }
    }

    /// Translation of `VULKAN_BeginRenderPass()`.
    fn begin_render_pass(&mut self, load_op: VkAttachmentLoadOp, clear_color: Option<[f32; 4]>) {
        let (width, height) = self.render_target_size();
        let target = self
            .texture_render_target
            .and_then(|id| self.textures.get(&id));

        let pass = match load_op {
            VK_ATTACHMENT_LOAD_OP_CLEAR => RenderPass::Clear,
            _ => RenderPass::Load,
        };
        self.current_render_pass = match target {
            Some(t) => t.main_renderpasses[pass as usize],
            None => self.render_passes[pass as usize],
        };

        let framebuffer = match target {
            Some(t) => t.main_framebuffer,
            None => self.framebuffers[self.current_swapchain_image_index as usize],
        };

        let clear_value = VkClearValue {
            float32: clear_color.unwrap_or_default(),
        };
        let render_pass_begin_info = VkRenderPassBeginInfo {
            s_type: VK_STRUCTURE_TYPE_RENDER_PASS_BEGIN_INFO,
            p_next: null(),
            render_pass: self.current_render_pass,
            framebuffer,
            render_area: VkRect2D {
                offset: VkOffset2D { x: 0, y: 0 },
                extent: VkExtent2D { width, height },
            },
            clear_value_count: clear_color.is_some() as u32,
            p_clear_values: if clear_color.is_some() {
                &clear_value
            } else {
                null()
            },
        };
        // SAFETY: the command buffer is recording, outside a render pass;
        // the begin info is valid.
        unsafe {
            (self.dev().cmd_begin_render_pass)(
                self.current_command_buffer,
                &render_pass_begin_info,
                VK_SUBPASS_CONTENTS_INLINE,
            )
        };
    }

    /// Translation of `VULKAN_EnsureCommandBuffer()`.
    fn ensure_command_buffer(&mut self) {
        if self.current_command_buffer.is_null() {
            self.current_command_buffer =
                self.command_buffers[self.current_command_buffer_index as usize];
            self.reset_command_list();

            // Ensure the swapchain is in the correct layout
            let image_index = self.current_swapchain_image_index as usize;
            if self.swapchain_image_layouts[image_index] == VK_IMAGE_LAYOUT_UNDEFINED {
                self.swapchain_image_layouts[image_index] = self.record_pipeline_image_barrier(
                    0,
                    VK_ACCESS_COLOR_ATTACHMENT_READ_BIT | VK_ACCESS_COLOR_ATTACHMENT_WRITE_BIT,
                    VK_PIPELINE_STAGE_TOP_OF_PIPE_BIT,
                    VK_PIPELINE_STAGE_COLOR_ATTACHMENT_OUTPUT_BIT,
                    VK_IMAGE_LAYOUT_COLOR_ATTACHMENT_OPTIMAL,
                    self.swapchain_images[image_index],
                    self.swapchain_image_layouts[image_index],
                );
            }
            // FIXME (upstream): this looks at the layout of the swapchain
            // image of the command buffer's index, not of the current
            // swapchain image (which it then transitions).
            else if self.swapchain_image_layouts[self.current_command_buffer_index as usize]
                != VK_IMAGE_LAYOUT_COLOR_ATTACHMENT_OPTIMAL
            {
                self.swapchain_image_layouts[image_index] = self.record_pipeline_image_barrier(
                    VK_ACCESS_COLOR_ATTACHMENT_READ_BIT,
                    VK_ACCESS_COLOR_ATTACHMENT_READ_BIT | VK_ACCESS_COLOR_ATTACHMENT_WRITE_BIT,
                    VK_PIPELINE_STAGE_COLOR_ATTACHMENT_OUTPUT_BIT,
                    VK_PIPELINE_STAGE_COLOR_ATTACHMENT_OUTPUT_BIT,
                    VK_IMAGE_LAYOUT_COLOR_ATTACHMENT_OPTIMAL,
                    self.swapchain_images[image_index],
                    self.swapchain_image_layouts[image_index],
                );
            }
        }
    }

    /// Translation of `VULKAN_ActivateCommandBuffer()`.
    fn activate_command_buffer(
        &mut self,
        load_op: VkAttachmentLoadOp,
        clear_color: Option<[f32; 4]>,
        state_cache: &DrawStateCache,
    ) {
        self.ensure_command_buffer();

        if self.current_render_pass == VK_NULL_HANDLE || load_op == VK_ATTACHMENT_LOAD_OP_CLEAR {
            self.end_render_pass();
            self.begin_render_pass(load_op, clear_color);
        }

        // Bind cached VB now
        if state_cache.vertex_buffer != VK_NULL_HANDLE {
            let offset: VkDeviceSize = 0;
            // SAFETY: the command buffer is recording; the buffer is a
            // vertex buffer of this device.
            unsafe {
                (self.dev().cmd_bind_vertex_buffers)(
                    self.current_command_buffer,
                    0,
                    1,
                    &state_cache.vertex_buffer,
                    &offset,
                )
            };
        }
    }

    /// Translation of `VULKAN_WaitForGPU()`.
    fn wait_for_gpu(&self) {
        if let Some(inst) = &self.inst {
            if !self.graphics_queue.is_null() {
                // SAFETY: the device's graphics queue.
                unsafe { (inst.queue_wait_idle)(self.graphics_queue) };
            }
        }
    }

    /// Translation of `VULKAN_ResetCommandList()`.
    fn reset_command_list(&mut self) {
        let dev = self.dev();
        let index = self.current_command_buffer_index as usize;
        // SAFETY: the command buffer and pools are this device's, and the
        // GPU is done with them (the callers waited, or the fence of the
        // frame was waited for).
        unsafe {
            (dev.reset_command_buffer)(self.current_command_buffer, 0);
            for &pool in &self.descriptor_pools[index] {
                (dev.reset_descriptor_pool)(self.device, pool, 0);
            }

            let begin_info = VkCommandBufferBeginInfo {
                s_type: VK_STRUCTURE_TYPE_COMMAND_BUFFER_BEGIN_INFO,
                flags: 0,
                ..Default::default()
            };
            (dev.begin_command_buffer)(self.current_command_buffer, &begin_info);
        }

        self.current_pipeline_state = None;
        self.current_vertex_buffer = 0;
        self.issue_batch = false;
        self.cliprect_dirty = true;
        self.current_descriptor_set_index = 0;
        self.current_descriptor_pool_index = 0;
        self.current_constant_buffer_offset = -1;
        self.current_constant_buffer_index = 0;

        // Release any upload buffers that were inflight
        let used = self.current_upload_buffer[index];
        let mut buffers = std::mem::take(&mut self.upload_buffers[index]);
        for buffer in &mut buffers[..used] {
            self.destroy_buffer(buffer);
        }
        self.upload_buffers[index] = buffers;
        self.current_upload_buffer[index] = 0;
    }

    /// The wait semaphores of a submission: the ones added for the frame
    /// and the image available semaphore (the `waitRenderSemaphores` and
    /// `waitDestStageMasks` arrays with their extra slot at the end).
    fn submit_waits(&mut self) -> (Vec<VkSemaphore>, Vec<VkPipelineStageFlags>) {
        let mut semaphores = std::mem::take(&mut self.wait_render_semaphores);
        let mut masks = std::mem::take(&mut self.wait_dest_stage_masks);
        if self.current_image_available_semaphore != VK_NULL_HANDLE {
            semaphores.push(self.current_image_available_semaphore);
            masks.push(VK_PIPELINE_STAGE_ALL_COMMANDS_BIT);
        }
        (semaphores, masks)
    }

    /// Translation of `VULKAN_IssueBatch()`.
    fn issue_batch(&mut self) -> VkResult {
        if self.current_command_buffer.is_null() {
            return VK_SUCCESS;
        }

        self.end_render_pass();

        self.current_pipeline_state = None;
        self.viewport_dirty = true;

        let dev = self.dev();
        // SAFETY: the command buffer is recording, outside a render pass.
        unsafe { (dev.end_command_buffer)(self.current_command_buffer) };

        let (wait_semaphores, wait_dest_stage_masks) = self.submit_waits();
        let submit_info = VkSubmitInfo {
            s_type: VK_STRUCTURE_TYPE_SUBMIT_INFO,
            command_buffer_count: 1,
            p_command_buffers: &self.current_command_buffer,
            wait_semaphore_count: wait_semaphores.len() as u32,
            p_wait_semaphores: wait_semaphores.as_ptr(),
            p_wait_dst_stage_mask: wait_dest_stage_masks.as_ptr(),
            ..Default::default()
        };

        // SAFETY: the command buffer is complete; the semaphores and masks
        // outlive the call.
        let result =
            unsafe { (dev.queue_submit)(self.graphics_queue, 1, &submit_info, VK_NULL_HANDLE) };
        self.current_image_available_semaphore = VK_NULL_HANDLE;

        self.wait_for_gpu();

        self.reset_command_list();

        result
    }

    /// Translation of `VULKAN_CreatePipelineState()`: the index of the new
    /// state.
    fn create_pipeline_state(
        &mut self,
        shader: Shader,
        pipeline_layout: VkPipelineLayout,
        descriptor_set_layout: VkDescriptorSetLayout,
        blend_mode: BlendMode,
        topology: VkPrimitiveTopology,
        format: VkFormat,
    ) -> Result<usize> {
        let mut pipeline = VK_NULL_HANDLE;

        // Shaders
        let name = c"main";
        let shader_stage_create_info: [VkPipelineShaderStageCreateInfo; 2] =
            std::array::from_fn(|i| VkPipelineShaderStageCreateInfo {
                s_type: VK_STRUCTURE_TYPE_PIPELINE_SHADER_STAGE_CREATE_INFO,
                module: if i == 0 {
                    self.vertex_shader_modules[shader as usize]
                } else {
                    self.fragment_shader_modules[shader as usize]
                },
                stage: if i == 0 {
                    VK_SHADER_STAGE_VERTEX_BIT
                } else {
                    VK_SHADER_STAGE_FRAGMENT_BIT
                },
                p_name: name.as_ptr(),
                ..Default::default()
            });

        // Vertex input
        let attribute_descriptions = [
            VkVertexInputAttributeDescription {
                binding: 0,
                format: VK_FORMAT_R32G32_SFLOAT,
                location: 0,
                offset: 0,
            },
            VkVertexInputAttributeDescription {
                binding: 0,
                format: VK_FORMAT_R32G32_SFLOAT,
                location: 1,
                offset: 8,
            },
            VkVertexInputAttributeDescription {
                binding: 0,
                format: VK_FORMAT_R32G32B32A32_SFLOAT,
                location: 2,
                offset: 16,
            },
        ];
        let binding_descriptions = [VkVertexInputBindingDescription {
            binding: 0,
            input_rate: VK_VERTEX_INPUT_RATE_VERTEX,
            stride: 32,
        }];
        let vertex_input_create_info = VkPipelineVertexInputStateCreateInfo {
            s_type: VK_STRUCTURE_TYPE_PIPELINE_VERTEX_INPUT_STATE_CREATE_INFO,
            vertex_attribute_description_count: 3,
            p_vertex_attribute_descriptions: attribute_descriptions.as_ptr(),
            vertex_binding_description_count: 1,
            p_vertex_binding_descriptions: binding_descriptions.as_ptr(),
            ..Default::default()
        };

        // Input assembly
        let input_assembly_state_create_info = VkPipelineInputAssemblyStateCreateInfo {
            s_type: VK_STRUCTURE_TYPE_PIPELINE_INPUT_ASSEMBLY_STATE_CREATE_INFO,
            topology,
            primitive_restart_enable: VK_FALSE,
            ..Default::default()
        };

        let viewport_state_create_info = VkPipelineViewportStateCreateInfo {
            s_type: VK_STRUCTURE_TYPE_PIPELINE_VIEWPORT_STATE_CREATE_INFO,
            scissor_count: 1,
            viewport_count: 1,
            ..Default::default()
        };

        // Dynamic states
        let dynamic_states = [VK_DYNAMIC_STATE_VIEWPORT, VK_DYNAMIC_STATE_SCISSOR];
        let dynamic_state_create_info = VkPipelineDynamicStateCreateInfo {
            s_type: VK_STRUCTURE_TYPE_PIPELINE_DYNAMIC_STATE_CREATE_INFO,
            dynamic_state_count: dynamic_states.len() as u32,
            p_dynamic_states: dynamic_states.as_ptr(),
            ..Default::default()
        };

        // Rasterization state
        let rasterization_state_create_info = VkPipelineRasterizationStateCreateInfo {
            s_type: VK_STRUCTURE_TYPE_PIPELINE_RASTERIZATION_STATE_CREATE_INFO,
            depth_clamp_enable: VK_FALSE,
            rasterizer_discard_enable: VK_FALSE,
            cull_mode: VK_CULL_MODE_NONE,
            polygon_mode: VK_POLYGON_MODE_FILL,
            front_face: VK_FRONT_FACE_COUNTER_CLOCKWISE,
            depth_bias_enable: VK_FALSE,
            depth_bias_constant_factor: 0.0,
            depth_bias_clamp: 0.0,
            depth_bias_slope_factor: 0.0,
            line_width: 1.0,
            ..Default::default()
        };

        // MSAA state
        let multi_sample_mask: u32 = 0xFFFFFFFF;
        let multisample_state_create_info = VkPipelineMultisampleStateCreateInfo {
            s_type: VK_STRUCTURE_TYPE_PIPELINE_MULTISAMPLE_STATE_CREATE_INFO,
            p_sample_mask: &multi_sample_mask,
            rasterization_samples: VK_SAMPLE_COUNT_1_BIT,
            ..Default::default()
        };

        // Depth Stencil
        let depth_stencil_state_create_info = VkPipelineDepthStencilStateCreateInfo {
            s_type: VK_STRUCTURE_TYPE_PIPELINE_DEPTH_STENCIL_STATE_CREATE_INFO,
            ..Default::default()
        };

        // Color blend
        let color_blend_attachment = VkPipelineColorBlendAttachmentState {
            blend_enable: VK_TRUE,
            src_color_blend_factor: get_blend_factor(blend_mode.src_color_factor()),
            src_alpha_blend_factor: get_blend_factor(blend_mode.src_alpha_factor()),
            color_blend_op: get_blend_op(blend_mode.color_operation()),
            dst_color_blend_factor: get_blend_factor(blend_mode.dst_color_factor()),
            dst_alpha_blend_factor: get_blend_factor(blend_mode.dst_alpha_factor()),
            alpha_blend_op: get_blend_op(blend_mode.alpha_operation()),
            color_write_mask: VK_COLOR_COMPONENT_R_BIT
                | VK_COLOR_COMPONENT_G_BIT
                | VK_COLOR_COMPONENT_B_BIT
                | VK_COLOR_COMPONENT_A_BIT,
        };
        let color_blend_state_create_info = VkPipelineColorBlendStateCreateInfo {
            s_type: VK_STRUCTURE_TYPE_PIPELINE_COLOR_BLEND_STATE_CREATE_INFO,
            attachment_count: 1,
            p_attachments: &color_blend_attachment,
            ..Default::default()
        };

        let pipeline_create_info = VkGraphicsPipelineCreateInfo {
            s_type: VK_STRUCTURE_TYPE_GRAPHICS_PIPELINE_CREATE_INFO,
            flags: 0,
            stage_count: 2,
            p_stages: shader_stage_create_info.as_ptr(),
            p_vertex_input_state: &vertex_input_create_info,
            p_input_assembly_state: &input_assembly_state_create_info,
            p_viewport_state: &viewport_state_create_info,
            p_rasterization_state: &rasterization_state_create_info,
            p_multisample_state: &multisample_state_create_info,
            p_depth_stencil_state: &depth_stencil_state_create_info,
            p_color_blend_state: &color_blend_state_create_info,
            p_dynamic_state: &dynamic_state_create_info,
            // Renderpass / layout
            render_pass: self.current_render_pass,
            subpass: 0,
            layout: pipeline_layout,
            ..Default::default()
        };

        // SAFETY: the create info and everything it points to outlive the
        // call.
        let result = unsafe {
            (self.dev().create_graphics_pipelines)(
                self.device,
                VK_NULL_HANDLE,
                1,
                &pipeline_create_info,
                null(),
                &mut pipeline,
            )
        };
        check("vkCreateGraphicsPipelines()", result)?;

        self.pipeline_states.push(PipelineState {
            shader,
            shader_constants: PixelShaderConstants::default(),
            blend_mode,
            topology,
            format,
            pipeline,
            descriptor_set_layout,
            pipeline_layout: pipeline_create_info.layout,
        });

        Ok(self.pipeline_states.len() - 1)
    }

    /// Translation of `VULKAN_FindMemoryTypeIndex()`.
    fn find_memory_type_index(
        &self,
        type_bits: u32,
        required_flags: VkMemoryPropertyFlags,
        mut desired_flags: VkMemoryPropertyFlags,
    ) -> Result<u32> {
        let props = &self.physical_device_memory_properties;
        let count = props.memory_type_count;
        let mut memory_type_index = 0;
        let mut found_exact_match = false;

        // Desired flags must be a superset of required flags.
        desired_flags |= required_flags;

        while memory_type_index < count {
            if type_bits & (1 << memory_type_index) != 0
                && props.memory_types[memory_type_index as usize].property_flags == desired_flags
            {
                found_exact_match = true;
                break;
            }
            memory_type_index += 1;
        }
        if !found_exact_match {
            memory_type_index = 0;
            while memory_type_index < count {
                if type_bits & (1 << memory_type_index) != 0
                    && (props.memory_types[memory_type_index as usize].property_flags
                        & required_flags)
                        == required_flags
                {
                    break;
                }
                memory_type_index += 1;
            }
        }

        if memory_type_index >= count {
            return Err(error_message("Unable to find memory type for allocation"));
        }
        Ok(memory_type_index)
    }

    /// Translation of `VULKAN_CreateVertexBuffer()`.
    fn create_vertex_buffer(&mut self, vbidx: usize, size: VkDeviceSize) -> Result<()> {
        let mut old = std::mem::take(&mut self.vertex_buffers[vbidx]);
        self.destroy_buffer(&mut old);

        self.vertex_buffers[vbidx] = self
            .allocate_buffer(
                size,
                VK_BUFFER_USAGE_VERTEX_BUFFER_BIT,
                VK_MEMORY_PROPERTY_HOST_COHERENT_BIT | VK_MEMORY_PROPERTY_HOST_VISIBLE_BIT,
                VK_MEMORY_PROPERTY_DEVICE_LOCAL_BIT,
            )
            .map_err(|(_, e)| e)?;
        Ok(())
    }

    /// The loader's `vkGetInstanceProcAddr` as a lookup function for the
    /// function tables.
    fn instance_lookup(
        &self,
        instance: VkInstance,
    ) -> impl FnMut(&CStr) -> Option<PfnVkVoidFunction> {
        let get = self.vk_get_instance_proc_addr;
        move |name: &CStr| {
            let get = get?;
            // SAFETY: the loader's vkGetInstanceProcAddr, with a null or
            // live instance and a NUL-terminated name.
            unsafe { get(instance, name.as_ptr()) }
        }
    }

    /// Translation of `VULKAN_LoadGlobalFunctions()`.
    fn load_global_functions(&mut self) -> Result<()> {
        // SAFETY: vkGetInstanceProcAddr returns the functions of the names.
        match unsafe { GlobalFunctions::load(self.instance_lookup(null_mut())) } {
            Ok(functions) => {
                self.global = Some(Rc::new(functions));
                Ok(())
            }
            Err(name) => Err(error_message(format!(
                "vkGetInstanceProcAddr(VK_NULL_HANDLE, \"{name}\") failed"
            ))),
        }
    }

    /// Translation of `VULKAN_LoadInstanceFunctions()`.
    fn load_instance_functions(&mut self) -> Result<()> {
        // SAFETY: vkGetInstanceProcAddr returns the functions of the names.
        match unsafe { InstanceFunctions::load(self.instance_lookup(self.instance)) } {
            Ok(functions) => {
                self.inst = Some(Rc::new(functions));
                Ok(())
            }
            Err(name) => Err(error_message(format!(
                "vkGetInstanceProcAddr(instance, \"{name}\") failed"
            ))),
        }
    }

    /// Translation of `VULKAN_LoadDeviceFunctions()`.
    fn load_device_functions(&mut self) -> Result<()> {
        let inst = self.inst();
        let device = self.device;
        let lookup = |name: &CStr| {
            // SAFETY: the instance's vkGetDeviceProcAddr, with the live
            // device and a NUL-terminated name.
            unsafe { (inst.get_device_proc_addr)(device, name.as_ptr()) }
        };
        // SAFETY: vkGetDeviceProcAddr returns the functions of the names.
        match unsafe { DeviceFunctions::load(lookup) } {
            Ok(functions) => {
                self.dev = Some(Rc::new(functions));
                Ok(())
            }
            Err(name) => Err(error_message(format!(
                "vkGetDeviceProcAddr(device, \"{name}\") failed"
            ))),
        }
    }

    /// Translation of `VULKAN_FindPhysicalDevice()`.
    fn find_physical_device(&mut self) -> Result<()> {
        let inst = self.inst();
        let mut physical_device_count: u32 = 0;

        // SAFETY: a null array queries the count.
        let result = unsafe {
            (inst.enumerate_physical_devices)(self.instance, &mut physical_device_count, null_mut())
        };
        check("vkEnumeratePhysicalDevices()", result)?;
        if physical_device_count == 0 {
            return Err(error_message(
                "vkEnumeratePhysicalDevices(): no physical devices",
            ));
        }
        let mut physical_devices = vec![null_mut(); physical_device_count as usize];
        // SAFETY: the array has room for the count.
        let result = unsafe {
            (inst.enumerate_physical_devices)(
                self.instance,
                &mut physical_device_count,
                physical_devices.as_mut_ptr(),
            )
        };
        check("vkEnumeratePhysicalDevices()", result)?;
        physical_devices.truncate(physical_device_count as usize);

        self.physical_device = null_mut();
        for &physical_device in &physical_devices {
            let mut queue_families_count: u32 = 0;
            let mut has_swapchain_extension = false;

            // SAFETY (for the queries below): a physical device of the
            // instance; the outputs have room for what they're asked for.
            unsafe {
                (inst.get_physical_device_properties)(
                    physical_device,
                    &mut *self.physical_device_properties,
                )
            };
            if vk_version_major(self.physical_device_properties.api_version) < 1 {
                continue;
            }
            // SAFETY: as above.
            unsafe {
                (inst.get_physical_device_memory_properties)(
                    physical_device,
                    &mut *self.physical_device_memory_properties,
                );
                (inst.get_physical_device_features)(
                    physical_device,
                    &mut self.physical_device_features,
                );
                (inst.get_physical_device_queue_family_properties)(
                    physical_device,
                    &mut queue_families_count,
                    null_mut(),
                );
            }
            if queue_families_count == 0 {
                continue;
            }
            let mut queue_families_properties =
                vec![VkQueueFamilyProperties::default(); queue_families_count as usize];
            // SAFETY: as above.
            unsafe {
                (inst.get_physical_device_queue_family_properties)(
                    physical_device,
                    &mut queue_families_count,
                    queue_families_properties.as_mut_ptr(),
                )
            };
            self.graphics_queue_family_index = queue_families_count;
            self.present_queue_family_index = queue_families_count;
            for queue_family_index in 0..queue_families_count {
                let mut supported: VkBool32 = 0;
                let family = &queue_families_properties[queue_family_index as usize];

                if family.queue_count == 0 {
                    continue;
                }

                if family.queue_flags & VK_QUEUE_GRAPHICS_BIT != 0 {
                    self.graphics_queue_family_index = queue_family_index;
                }

                // SAFETY: as above, with the instance's surface.
                let result = unsafe {
                    (inst.get_physical_device_surface_support_khr)(
                        physical_device,
                        queue_family_index,
                        self.surface,
                        &mut supported,
                    )
                };
                if result != VK_SUCCESS {
                    return Err(error_code("vkGetPhysicalDeviceSurfaceSupportKHR()", result));
                }
                if supported != 0 {
                    self.present_queue_family_index = queue_family_index;
                    if family.queue_flags & VK_QUEUE_GRAPHICS_BIT != 0 {
                        break; // use this queue because it can present and do graphics
                    }
                }
            }

            if self.graphics_queue_family_index == queue_families_count {
                // no good queues found
                continue;
            }
            if self.present_queue_family_index == queue_families_count {
                // no good queues found
                continue;
            }
            let device_extensions = enumerate_extensions(
                |count, properties| {
                    // SAFETY: as above.
                    unsafe {
                        (inst.enumerate_device_extension_properties)(
                            physical_device,
                            null(),
                            count,
                            properties,
                        )
                    }
                },
                "vkEnumerateDeviceExtensionProperties()",
            )?;
            if device_extensions.is_empty() {
                continue;
            }
            if has_extension(&device_extensions, VK_KHR_SWAPCHAIN_EXTENSION_NAME) {
                has_swapchain_extension = true;
            }
            if !has_swapchain_extension {
                continue;
            }
            self.physical_device = physical_device;
            break;
        }
        if self.physical_device.is_null() {
            return Err(error_message("No viable physical devices found"));
        }
        Ok(())
    }

    /// Translation of `VULKAN_GetSurfaceFormats()`.
    fn get_surface_formats(&mut self) -> Result<()> {
        let inst = self.inst();
        let mut count = 0;
        // SAFETY: a null array queries the count.
        let result = unsafe {
            (inst.get_physical_device_surface_formats_khr)(
                self.physical_device,
                self.surface,
                &mut count,
                null_mut(),
            )
        };
        if result != VK_SUCCESS {
            self.surface_formats.clear();
            return Err(error_code("vkGetPhysicalDeviceSurfaceFormatsKHR()", result));
        }
        let mut formats = vec![VkSurfaceFormatKHR::default(); count as usize];
        // SAFETY: the array has room for the count.
        let result = unsafe {
            (inst.get_physical_device_surface_formats_khr)(
                self.physical_device,
                self.surface,
                &mut count,
                formats.as_mut_ptr(),
            )
        };
        if result != VK_SUCCESS {
            self.surface_formats.clear();
            return Err(error_code("vkGetPhysicalDeviceSurfaceFormatsKHR()", result));
        }
        formats.truncate(count as usize);
        self.surface_formats = formats;

        Ok(())
    }

    /// Translation of `VULKAN_CreateSemaphore()`.
    fn create_semaphore(&self) -> Result<VkSemaphore> {
        let mut semaphore = VK_NULL_HANDLE;

        let semaphore_create_info = VkSemaphoreCreateInfo {
            s_type: VK_STRUCTURE_TYPE_SEMAPHORE_CREATE_INFO,
            ..Default::default()
        };
        // SAFETY: a valid create info.
        let result = unsafe {
            (self.dev().create_semaphore)(
                self.device,
                &semaphore_create_info,
                null(),
                &mut semaphore,
            )
        };
        check("vkCreateSemaphore()", result)?;
        Ok(semaphore)
    }

    /// Translation of `VULKAN_DeviceExtensionsFound()`.
    fn device_extensions_found(&self, ext_names: &[&CStr]) -> Result<bool> {
        let inst = self.inst();
        let physical_device = self.physical_device;
        let mut found_extensions = true;
        let extension_properties = enumerate_extensions(
            |count, properties| {
                // SAFETY: the renderer's physical device; the array has
                // room for the count.
                unsafe {
                    (inst.enumerate_device_extension_properties)(
                        physical_device,
                        null(),
                        count,
                        properties,
                    )
                }
            },
            "vkEnumerateDeviceExtensionProperties()",
        )?;
        if !extension_properties.is_empty() {
            for name in ext_names {
                if !found_extensions {
                    break;
                }
                found_extensions &= has_extension(&extension_properties, name);
            }
        }

        Ok(found_extensions)
    }

    /// Translation of `VULKAN_InstanceExtensionFound()`.
    fn instance_extension_found(&self, ext_name: &CStr) -> Result<bool> {
        let global = self.global.clone().expect("Vulkan global functions");
        let extension_properties = enumerate_extensions(
            |count, properties| {
                // SAFETY: the array has room for the count.
                unsafe {
                    (global.enumerate_instance_extension_properties)(null(), count, properties)
                }
            },
            "vkEnumerateInstanceExtensionProperties()",
        )?;
        Ok(has_extension(&extension_properties, ext_name))
    }

    /// Translation of `VULKAN_ValidationLayersFound()`.
    fn validation_layers_found(&self) -> bool {
        let global = self.global.clone().expect("Vulkan global functions");
        let mut instance_layer_count: u32 = 0;
        let mut found_validation = false;

        // SAFETY: a null array queries the count.
        unsafe {
            (global.enumerate_instance_layer_properties)(&mut instance_layer_count, null_mut())
        };
        if instance_layer_count > 0 {
            let mut instance_layers =
                vec![VkLayerProperties::default(); instance_layer_count as usize];
            // SAFETY: the array has room for the count.
            unsafe {
                (global.enumerate_instance_layer_properties)(
                    &mut instance_layer_count,
                    instance_layers.as_mut_ptr(),
                )
            };
            for layer in instance_layers.iter().take(instance_layer_count as usize) {
                // SAFETY: the names are NUL-terminated (the arrays started
                // zeroed).
                if unsafe { CStr::from_ptr(layer.layer_name.as_ptr()) }
                    == SDL_VULKAN_VALIDATION_LAYER_NAME
                {
                    found_validation = true;
                    break;
                }
            }
        }

        found_validation
    }

    /// Create resources that depend on the device. Translation of
    /// `VULKAN_CreateDeviceResources()` (the instance, surface, physical
    /// device, device and queue families are always made here).
    fn create_device_resources(&mut self) -> Result<()> {
        /* The list of extension names, in dependency order.
         * If there are extension blocks that are not dependent upon each other, you'll need
         * to dynamically build the list of extension names instead of using this array.
         */
        const DEVICE_EXTENSION_NAMES: [&CStr; 6] = [
            VK_KHR_SWAPCHAIN_EXTENSION_NAME,
            // VK_KHR_sampler_ycbcr_conversion + dependent extensions
            c"VK_KHR_sampler_ycbcr_conversion",
            c"VK_KHR_maintenance1",
            c"VK_KHR_maintenance2",
            c"VK_KHR_bind_memory2",
            c"VK_KHR_get_memory_requirements2",
        ];
        const YCBCR_EXTENSION_OFFSET: usize = 1;
        const YCBCR_EXTENSION_COUNT: usize = 5;
        // (the Android extensions come after these upstream)
        const _: () =
            assert!(DEVICE_EXTENSION_NAMES.len() == YCBCR_EXTENSION_OFFSET + YCBCR_EXTENSION_COUNT);

        let create_debug = hints::get_bool(hints::RENDER_VULKAN_DEBUG, false);
        let validation_layer_name = [SDL_VULKAN_VALIDATION_LAYER_NAME.as_ptr()];

        // FIXME (upstream): this reference to the loader is never released
        // (and a device reset takes another one).
        if let Err(e) = crate::video::vulkan::vulkan_load_library(None) {
            crate::log::debug!(Category::Render, "SDL_Vulkan_LoadLibrary failed");
            return Err(e);
        }
        let vk_get_instance_proc_addr =
            match crate::video::vulkan::vulkan_get_vk_get_instance_proc_addr() {
                Ok(f) if f != 0 => f,
                other => {
                    crate::log::debug!(Category::Render, "vkGetInstanceProcAddr is NULL");
                    return Err(other
                        .err()
                        .unwrap_or_else(|| Error::new("vkGetInstanceProcAddr is NULL")));
                }
            };

        // Load global Vulkan functions
        // SAFETY: the video driver's vkGetInstanceProcAddr, which stays
        // loaded while the renderer holds its reference to the loader.
        self.vk_get_instance_proc_addr = Some(unsafe {
            std::mem::transmute::<usize, PfnVkGetInstanceProcAddr>(vk_get_instance_proc_addr)
        });
        self.load_global_functions()?;
        let global = self.global.clone().expect("Vulkan global functions");

        // Check for colorspace extension
        self.supports_ext_swapchain_colorspace = false;
        if self.output_colorspace == Colorspace::SRGB_LINEAR
            || self.output_colorspace == Colorspace::HDR10
        {
            self.supports_ext_swapchain_colorspace =
                self.instance_extension_found(VK_EXT_SWAPCHAIN_COLOR_SPACE_EXTENSION_NAME)?;
            if !self.supports_ext_swapchain_colorspace {
                return Err(Error::new(format!(
                    "Using HDR output but {} not supported",
                    VK_EXT_SWAPCHAIN_COLOR_SPACE_EXTENSION_NAME.to_string_lossy()
                )));
            }
        }

        // Check for VK_KHR_get_physical_device_properties2
        self.supports_khr_get_physical_device_properties2 = self
            .instance_extension_found(VK_KHR_GET_PHYSICAL_DEVICE_PROPERTIES_2_EXTENSION_NAME)
            .unwrap_or(false);

        // Check for VK_KHR_external_memory_capabilities, which we need to import hardware buffers
        self.supports_khr_external_memory_capabilities = self
            .instance_extension_found(VK_KHR_EXTERNAL_MEMORY_CAPABILITIES_EXTENSION_NAME)
            .unwrap_or(false);

        // Create VkInstance
        {
            let app_info = VkApplicationInfo {
                s_type: VK_STRUCTURE_TYPE_APPLICATION_INFO,
                api_version: VK_API_VERSION_1_0,
                ..Default::default()
            };
            let instance_extensions =
                crate::video::vulkan::vulkan_instance_extensions().unwrap_or_default();
            let mut names: Vec<CString> = instance_extensions
                .iter()
                .filter_map(|e| CString::new(*e).ok())
                .collect();
            if self.supports_ext_swapchain_colorspace {
                names.push(VK_EXT_SWAPCHAIN_COLOR_SPACE_EXTENSION_NAME.to_owned());
            }
            if self.supports_khr_get_physical_device_properties2 {
                names.push(VK_KHR_GET_PHYSICAL_DEVICE_PROPERTIES_2_EXTENSION_NAME.to_owned());
            }
            if self.supports_khr_external_memory_capabilities {
                names.push(VK_KHR_EXTERNAL_MEMORY_CAPABILITIES_EXTENSION_NAME.to_owned());
            }
            let instance_extensions_copy: Vec<*const c_char> =
                names.iter().map(|n| n.as_ptr()).collect();
            let mut instance_create_info = VkInstanceCreateInfo {
                s_type: VK_STRUCTURE_TYPE_INSTANCE_CREATE_INFO,
                p_application_info: &app_info,
                enabled_extension_count: instance_extensions_copy.len() as u32,
                pp_enabled_extension_names: instance_extensions_copy.as_ptr(),
                ..Default::default()
            };
            if create_debug && self.validation_layers_found() {
                instance_create_info.pp_enabled_layer_names = validation_layer_name.as_ptr();
                instance_create_info.enabled_layer_count = 1;
            }
            // SAFETY: the create info and the names outlive the call.
            let result = unsafe {
                (global.create_instance)(&instance_create_info, null(), &mut self.instance)
            };
            check("vkCreateInstance()", result)?;
        }

        // Load instance Vulkan functions
        if let Err(e) = self.load_instance_functions() {
            self.destroy_all();
            return Err(e);
        }
        let inst = self.inst();

        // Create Vulkan surface
        match crate::video::vulkan::vulkan_create_surface(&self.window, self.instance as usize, 0) {
            Ok(surface) => self.surface = surface,
            Err(e) => {
                self.destroy_all();
                return Err(e);
            }
        }

        // Choose Vulkan physical device
        if let Err(e) = self.find_physical_device() {
            self.destroy_all();
            return Err(e);
        }

        let found = self.supports_khr_get_physical_device_properties2
            && self
                .device_extensions_found(
                    &DEVICE_EXTENSION_NAMES[YCBCR_EXTENSION_OFFSET..][..YCBCR_EXTENSION_COUNT],
                )
                .unwrap_or(false);
        if found {
            self.supports_khr_sampler_ycbcr_conversion = true;
        }

        // Create Vulkan device
        {
            let queue_priority = [1.0f32];
            let mut device_queue_create_info = [VkDeviceQueueCreateInfo::default(); 2];
            let mut device_sampler_ycbcr_conversion_features =
                VkPhysicalDeviceSamplerYcbcrConversionFeatures::default();
            let device_extension_names: Vec<*const c_char> =
                DEVICE_EXTENSION_NAMES.iter().map(|n| n.as_ptr()).collect();

            let mut queue_create_info_count = 0;
            device_queue_create_info[0] = VkDeviceQueueCreateInfo {
                s_type: VK_STRUCTURE_TYPE_DEVICE_QUEUE_CREATE_INFO,
                queue_family_index: self.graphics_queue_family_index,
                queue_count: 1,
                p_queue_priorities: queue_priority.as_ptr(),
                ..Default::default()
            };
            queue_create_info_count += 1;

            if self.present_queue_family_index != self.graphics_queue_family_index {
                device_queue_create_info[1] = VkDeviceQueueCreateInfo {
                    s_type: VK_STRUCTURE_TYPE_DEVICE_QUEUE_CREATE_INFO,
                    queue_family_index: self.present_queue_family_index,
                    queue_count: 1,
                    p_queue_priorities: queue_priority.as_ptr(),
                    ..Default::default()
                };
                queue_create_info_count += 1;
            }

            let mut device_create_info = VkDeviceCreateInfo {
                s_type: VK_STRUCTURE_TYPE_DEVICE_CREATE_INFO,
                queue_create_info_count,
                p_queue_create_infos: device_queue_create_info.as_ptr(),
                p_enabled_features: null(),
                enabled_extension_count: 1,
                ..Default::default()
            };
            if self.supports_khr_sampler_ycbcr_conversion {
                device_create_info.enabled_extension_count += YCBCR_EXTENSION_COUNT as u32;
            }
            device_create_info.pp_enabled_extension_names = device_extension_names.as_ptr();

            if self.supports_khr_sampler_ycbcr_conversion {
                device_sampler_ycbcr_conversion_features.s_type =
                    VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_SAMPLER_YCBCR_CONVERSION_FEATURES;
                device_sampler_ycbcr_conversion_features.sampler_ycbcr_conversion = VK_TRUE;
                device_sampler_ycbcr_conversion_features.p_next =
                    device_create_info.p_next as *mut c_void;
                device_create_info.p_next =
                    &device_sampler_ycbcr_conversion_features as *const _ as *const c_void;
            }

            // SAFETY: the create info and everything it points to outlive
            // the call.
            let result = unsafe {
                (inst.create_device)(
                    self.physical_device,
                    &device_create_info,
                    null(),
                    &mut self.device,
                )
            };
            if result != VK_SUCCESS {
                let e = error_code("vkCreateDevice()", result);
                self.destroy_all();
                return Err(e);
            }
        }

        if let Err(e) = self.load_device_functions() {
            self.destroy_all();
            return Err(e);
        }
        let dev = self.dev();

        // Get graphics/present queues
        // SAFETY: the queues the device was created with.
        unsafe {
            (dev.get_device_queue)(
                self.device,
                self.graphics_queue_family_index,
                0,
                &mut self.graphics_queue,
            );
            if self.graphics_queue_family_index != self.present_queue_family_index {
                (dev.get_device_queue)(
                    self.device,
                    self.present_queue_family_index,
                    0,
                    &mut self.present_queue,
                );
            } else {
                self.present_queue = self.graphics_queue;
            }
        }

        // Create command pool/command buffers
        let command_pool_create_info = VkCommandPoolCreateInfo {
            s_type: VK_STRUCTURE_TYPE_COMMAND_POOL_CREATE_INFO,
            flags: VK_COMMAND_POOL_CREATE_RESET_COMMAND_BUFFER_BIT,
            queue_family_index: self.graphics_queue_family_index,
            ..Default::default()
        };
        // SAFETY: a valid create info.
        let result = unsafe {
            (dev.create_command_pool)(
                self.device,
                &command_pool_create_info,
                null(),
                &mut self.command_pool,
            )
        };
        if result != VK_SUCCESS {
            self.destroy_all();
            return Err(error_code("vkCreateCommandPool()", result));
        }

        // FIXME (upstream): on failure, upstream returns the stale result of
        // vkCreateCommandPool() (VK_SUCCESS), so the creation goes on with
        // everything destroyed; here the failure is returned.
        if let Err(e) = self.get_surface_formats() {
            self.destroy_all();
            return Err(e);
        }

        // Create shaders / layouts
        for shader in Shader::ALL {
            let i = shader as usize;
            let code = shader.vertex_shader();
            let mut shader_module_create_info = VkShaderModuleCreateInfo {
                s_type: VK_STRUCTURE_TYPE_SHADER_MODULE_CREATE_INFO,
                p_code: code.as_ptr(),
                code_size: size_of_val(code),
                ..Default::default()
            };
            // SAFETY: the SPIR-V outlives the call.
            let result = unsafe {
                (dev.create_shader_module)(
                    self.device,
                    &shader_module_create_info,
                    null(),
                    &mut self.vertex_shader_modules[i],
                )
            };
            if result != VK_SUCCESS {
                self.destroy_all();
                return Err(error_code("vkCreateShaderModule()", result));
            }
            let code = shader.pixel_shader();
            shader_module_create_info.p_code = code.as_ptr();
            shader_module_create_info.code_size = size_of_val(code);
            // SAFETY: as above.
            let result = unsafe {
                (dev.create_shader_module)(
                    self.device,
                    &shader_module_create_info,
                    null(),
                    &mut self.fragment_shader_modules[i],
                )
            };
            if result != VK_SUCCESS {
                self.destroy_all();
                return Err(error_code("vkCreateShaderModule()", result));
            }
        }

        // Descriptor set layout / pipeline layout
        match self.create_descriptor_set_and_pipeline_layout(VK_NULL_HANDLE) {
            Ok((descriptor_set_layout, pipeline_layout)) => {
                self.descriptor_set_layout = descriptor_set_layout;
                self.pipeline_layout = pipeline_layout;
            }
            Err(e) => {
                self.destroy_all();
                return Err(e);
            }
        }

        // Create default vertex buffers
        self.vertex_buffers = vec![Buffer::default(); SDL_VULKAN_NUM_VERTEX_BUFFERS];
        for i in 0..SDL_VULKAN_NUM_VERTEX_BUFFERS {
            let _ = self.create_vertex_buffer(i, SDL_VULKAN_VERTEX_BUFFER_DEFAULT_SIZE);
        }

        self.publish_device_properties();

        Ok(())
    }

    /// The `SDL_PROP_RENDERER_VULKAN_*` properties of the device, once the
    /// front end has made the properties.
    fn publish_device_properties(&self) {
        let Some(props) = &self.props else {
            return;
        };
        let _ = props.set(
            PROP_RENDERER_VULKAN_INSTANCE_POINTER,
            self.instance as usize as i64,
        );
        let _ = props.set(PROP_RENDERER_VULKAN_SURFACE_NUMBER, self.surface as i64);
        let _ = props.set(
            PROP_RENDERER_VULKAN_PHYSICAL_DEVICE_POINTER,
            self.physical_device as usize as i64,
        );
        let _ = props.set(
            PROP_RENDERER_VULKAN_DEVICE_POINTER,
            self.device as usize as i64,
        );
        let _ = props.set(
            PROP_RENDERER_VULKAN_GRAPHICS_QUEUE_FAMILY_INDEX_NUMBER,
            self.graphics_queue_family_index as i64,
        );
        let _ = props.set(
            PROP_RENDERER_VULKAN_PRESENT_QUEUE_FAMILY_INDEX_NUMBER,
            self.present_queue_family_index as i64,
        );
    }

    /// `SDL_PROP_RENDERER_VULKAN_SWAPCHAIN_IMAGE_COUNT_NUMBER`.
    fn publish_swapchain_properties(&self) {
        if let Some(props) = &self.props {
            let _ = props.set(
                PROP_RENDERER_VULKAN_SWAPCHAIN_IMAGE_COUNT_NUMBER,
                self.swapchain_image_count as i64,
            );
        }
    }

    /// Translation of `VULKAN_CreateFramebuffersAndRenderPasses()`: a
    /// framebuffer for each image view, and the render passes.
    fn create_framebuffers_and_render_passes(
        &self,
        w: i32,
        h: i32,
        format: VkFormat,
        image_views: &[VkImageView],
        framebuffers: &mut Vec<VkFramebuffer>,
        render_passes: &mut [VkRenderPass; VULKAN_RENDERPASS_COUNT],
    ) -> VkResultOr<()> {
        let dev = self.dev();

        let mut attachment_description = VkAttachmentDescription {
            format,
            load_op: VK_ATTACHMENT_LOAD_OP_LOAD,
            store_op: VK_ATTACHMENT_STORE_OP_STORE,
            stencil_load_op: VK_ATTACHMENT_LOAD_OP_DONT_CARE,
            stencil_store_op: VK_ATTACHMENT_STORE_OP_DONT_CARE,
            initial_layout: VK_IMAGE_LAYOUT_COLOR_ATTACHMENT_OPTIMAL,
            final_layout: VK_IMAGE_LAYOUT_COLOR_ATTACHMENT_OPTIMAL,
            samples: VK_SAMPLE_COUNT_1_BIT,
            flags: 0,
        };

        let color_attachment_reference = VkAttachmentReference {
            attachment: 0,
            layout: VK_IMAGE_LAYOUT_COLOR_ATTACHMENT_OPTIMAL,
        };

        let subpass_description = VkSubpassDescription {
            pipeline_bind_point: VK_PIPELINE_BIND_POINT_GRAPHICS,
            flags: 0,
            input_attachment_count: 0,
            p_input_attachments: null(),
            color_attachment_count: 1,
            p_color_attachments: &color_attachment_reference,
            p_resolve_attachments: null(),
            p_depth_stencil_attachment: null(),
            preserve_attachment_count: 0,
            p_preserve_attachments: null(),
        };

        let sub_pass_dependency = VkSubpassDependency {
            src_subpass: VK_SUBPASS_EXTERNAL,
            dst_subpass: 0,
            src_stage_mask: VK_PIPELINE_STAGE_COLOR_ATTACHMENT_OUTPUT_BIT,
            dst_stage_mask: VK_PIPELINE_STAGE_COLOR_ATTACHMENT_OUTPUT_BIT,
            src_access_mask: VK_ACCESS_COLOR_ATTACHMENT_WRITE_BIT,
            dst_access_mask: VK_ACCESS_COLOR_ATTACHMENT_WRITE_BIT
                | VK_ACCESS_COLOR_ATTACHMENT_READ_BIT,
            dependency_flags: VK_DEPENDENCY_BY_REGION_BIT,
        };

        let mut render_pass_create_info = VkRenderPassCreateInfo {
            s_type: VK_STRUCTURE_TYPE_RENDER_PASS_CREATE_INFO,
            flags: 0,
            attachment_count: 1,
            p_attachments: &attachment_description,
            subpass_count: 1,
            p_subpasses: &subpass_description,
            dependency_count: 1,
            p_dependencies: &sub_pass_dependency,
            ..Default::default()
        };

        // SAFETY: the create info and everything it points to outlive the
        // call.
        let result = unsafe {
            (dev.create_render_pass)(
                self.device,
                &render_pass_create_info,
                null(),
                &mut render_passes[RenderPass::Load as usize],
            )
        };
        if result != VK_SUCCESS {
            return Err(failure("vkCreateRenderPass()", result));
        }

        attachment_description.load_op = VK_ATTACHMENT_LOAD_OP_CLEAR;
        render_pass_create_info.p_attachments = &attachment_description;
        // SAFETY: as above.
        let result = unsafe {
            (dev.create_render_pass)(
                self.device,
                &render_pass_create_info,
                null(),
                &mut render_passes[RenderPass::Clear as usize],
            )
        };
        if result != VK_SUCCESS {
            return Err(failure("vkCreateRenderPass()", result));
        }

        let mut framebuffer_create_info = VkFramebufferCreateInfo {
            s_type: VK_STRUCTURE_TYPE_FRAMEBUFFER_CREATE_INFO,
            p_next: null(),
            render_pass: render_passes[RenderPass::Load as usize],
            attachment_count: 1,
            width: w as u32,
            height: h as u32,
            layers: 1,
            ..Default::default()
        };

        for image_view in image_views {
            framebuffer_create_info.p_attachments = image_view;
            let mut framebuffer = VK_NULL_HANDLE;
            // SAFETY: as above.
            let result = unsafe {
                (dev.create_framebuffer)(
                    self.device,
                    &framebuffer_create_info,
                    null(),
                    &mut framebuffer,
                )
            };
            if result != VK_SUCCESS {
                return Err(failure("vkCreateFramebuffer()", result));
            }
            framebuffers.push(framebuffer);
        }

        Ok(())
    }

    /// Translation of `VULKAN_CreateSwapChain()`; the `VkResult` comes with
    /// the error.
    fn create_swap_chain(&mut self, w: i32, h: i32) -> VkResultOr<()> {
        let inst = self.inst();
        let dev = self.dev();
        // SAFETY: the renderer's physical device and surface.
        let result = unsafe {
            (inst.get_physical_device_surface_capabilities_khr)(
                self.physical_device,
                self.surface,
                &mut self.surface_capabilities,
            )
        };
        if result != VK_SUCCESS {
            return Err(failure(
                "vkGetPhysicalDeviceSurfaceCapabilitiesKHR()",
                result,
            ));
        }

        // clean up previous swapchain resources
        // SAFETY (for the destruction): the objects are this device's and
        // the GPU is idle (the callers waited for it).
        unsafe {
            for view in self.swapchain_image_views.drain(..) {
                (dev.destroy_image_view)(self.device, view, null());
            }
            for fence in self.fences.drain(..) {
                if fence != VK_NULL_HANDLE {
                    (dev.destroy_fence)(self.device, fence, null());
                }
            }
            if !self.command_buffers.is_empty() {
                (dev.free_command_buffers)(
                    self.device,
                    self.command_pool,
                    self.command_buffers.len() as u32,
                    self.command_buffers.as_ptr(),
                );
                self.command_buffers = Vec::new();
                self.current_command_buffer = null_mut();
                self.current_command_buffer_index = 0;
            }
            for framebuffer in self.framebuffers.drain(..) {
                if framebuffer != VK_NULL_HANDLE {
                    (dev.destroy_framebuffer)(self.device, framebuffer, null());
                }
            }
            for pools in self.descriptor_pools.drain(..) {
                for pool in pools {
                    if pool != VK_NULL_HANDLE {
                        (dev.destroy_descriptor_pool)(self.device, pool, null());
                    }
                }
            }
            for semaphore in self.image_available_semaphores.drain(..) {
                if semaphore != VK_NULL_HANDLE {
                    (dev.destroy_semaphore)(self.device, semaphore, null());
                }
            }
            for semaphore in self.rendering_finished_semaphores.drain(..) {
                if semaphore != VK_NULL_HANDLE {
                    (dev.destroy_semaphore)(self.device, semaphore, null());
                }
            }
        }
        let mut upload_buffers = std::mem::take(&mut self.upload_buffers);
        for buffers in &mut upload_buffers {
            for buffer in buffers {
                self.destroy_buffer(buffer);
            }
        }
        let mut constant_buffers = std::mem::take(&mut self.constant_buffers);
        for buffers in &mut constant_buffers {
            for buffer in buffers {
                self.destroy_buffer(buffer);
            }
        }

        // pick an image count
        let caps = self.surface_capabilities;
        self.swapchain_desired_image_count = caps.min_image_count + SDL_VULKAN_FRAME_QUEUE_DEPTH;
        if self.swapchain_desired_image_count > caps.max_image_count && caps.max_image_count > 0 {
            self.swapchain_desired_image_count = caps.max_image_count;
        }

        let mut desired_format = VK_FORMAT_B8G8R8A8_UNORM;
        let mut desired_color_space = VK_COLOR_SPACE_SRGB_NONLINEAR_KHR;
        if self.output_colorspace == Colorspace::SRGB_LINEAR {
            desired_format = VK_FORMAT_R16G16B16A16_SFLOAT;
            desired_color_space = VK_COLOR_SPACE_EXTENDED_SRGB_LINEAR_EXT;
        } else if self.output_colorspace == Colorspace::HDR10 {
            desired_format = VK_FORMAT_A2B10G10R10_UNORM_PACK32;
            desired_color_space = VK_COLOR_SPACE_HDR10_ST2084_EXT;
        }

        if self.surface_formats.len() == 1 && self.surface_formats[0].format == VK_FORMAT_UNDEFINED
        {
            // aren't any preferred formats, so we pick
            self.surface_format.color_space = VK_COLOR_SPACE_SRGB_NONLINEAR_KHR;
            self.surface_format.format = desired_format;
        } else if let Some(&first) = self.surface_formats.first() {
            self.surface_format = first;
            for format in &self.surface_formats {
                if format.format == desired_format && format.color_space == desired_color_space {
                    self.surface_format = *format;
                    break;
                }
            }
        }
        // (with no surface formats at all, upstream reads past its array;
        // the previous format stays here)

        self.swapchain_size.width = sdl_clamp(
            w as u32,
            caps.min_image_extent.width,
            caps.max_image_extent.width,
        );

        self.swapchain_size.height = sdl_clamp(
            h as u32,
            caps.min_image_extent.height,
            caps.max_image_extent.height,
        );

        // Handle rotation
        self.swap_chain_pre_transform = caps.current_transform;
        if self.swap_chain_pre_transform == VK_SURFACE_TRANSFORM_ROTATE_90_BIT_KHR
            || self.swap_chain_pre_transform == VK_SURFACE_TRANSFORM_ROTATE_270_BIT_KHR
        {
            std::mem::swap(
                &mut self.swapchain_size.width,
                &mut self.swapchain_size.height,
            );
        }

        if self.swapchain_size.width == 0 && self.swapchain_size.height == 0 {
            // Don't recreate the swapchain if size is (0,0), just fail and continue attempting creation
            // (upstream sets no error message here)
            return Err((
                VK_ERROR_OUT_OF_DATE_KHR,
                Error::new(vulkan_get_result_string(VK_ERROR_OUT_OF_DATE_KHR)),
            ));
        }

        // Choose a present mode. If vsync is requested, then use VK_PRESENT_MODE_FIFO_KHR which is guaranteed to be supported
        let mut present_mode = VK_PRESENT_MODE_FIFO_KHR;
        if self.vsync <= 0 {
            let mut present_mode_count: u32 = 0;
            // SAFETY: a null array queries the count.
            let result = unsafe {
                (inst.get_physical_device_surface_present_modes_khr)(
                    self.physical_device,
                    self.surface,
                    &mut present_mode_count,
                    null_mut(),
                )
            };
            if result != VK_SUCCESS {
                return Err(failure(
                    "vkGetPhysicalDeviceSurfacePresentModesKHR()",
                    result,
                ));
            }
            if present_mode_count > 0 {
                let mut present_modes = vec![0; present_mode_count as usize];
                // SAFETY: the array has room for the count.
                let result = unsafe {
                    (inst.get_physical_device_surface_present_modes_khr)(
                        self.physical_device,
                        self.surface,
                        &mut present_mode_count,
                        present_modes.as_mut_ptr(),
                    )
                };
                if result != VK_SUCCESS {
                    return Err(failure(
                        "vkGetPhysicalDeviceSurfacePresentModesKHR()",
                        result,
                    ));
                }
                present_modes.truncate(present_mode_count as usize);

                if self.vsync == 0 {
                    /* If vsync is not requested, in favor these options in order:
                    VK_PRESENT_MODE_IMMEDIATE_KHR    - no v-sync with tearing
                    VK_PRESENT_MODE_MAILBOX_KHR      - no v-sync without tearing
                    VK_PRESENT_MODE_FIFO_RELAXED_KHR - no v-sync, may tear */
                    for &mode in &present_modes {
                        if mode == VK_PRESENT_MODE_IMMEDIATE_KHR {
                            present_mode = VK_PRESENT_MODE_IMMEDIATE_KHR;
                            break;
                        } else if mode == VK_PRESENT_MODE_MAILBOX_KHR {
                            present_mode = VK_PRESENT_MODE_MAILBOX_KHR;
                        } else if present_mode != VK_PRESENT_MODE_MAILBOX_KHR
                            && mode == VK_PRESENT_MODE_FIFO_RELAXED_KHR
                        {
                            present_mode = VK_PRESENT_MODE_FIFO_RELAXED_KHR;
                        }
                    }
                } else if self.vsync == -1 {
                    for &mode in &present_modes {
                        if mode == VK_PRESENT_MODE_FIFO_RELAXED_KHR {
                            present_mode = VK_PRESENT_MODE_FIFO_RELAXED_KHR;
                            break;
                        }
                    }
                }
            }
        }

        let transparent = self
            .window
            .flags()
            .unwrap_or_default()
            .contains(WindowFlags::TRANSPARENT);
        let mut swapchain_create_info = VkSwapchainCreateInfoKHR {
            s_type: VK_STRUCTURE_TYPE_SWAPCHAIN_CREATE_INFO_KHR,
            surface: self.surface,
            min_image_count: self.swapchain_desired_image_count,
            image_format: self.surface_format.format,
            image_color_space: self.surface_format.color_space,
            image_extent: self.swapchain_size,
            image_array_layers: 1,
            image_usage: VK_IMAGE_USAGE_COLOR_ATTACHMENT_BIT | VK_IMAGE_USAGE_TRANSFER_DST_BIT,
            image_sharing_mode: VK_SHARING_MODE_EXCLUSIVE,
            pre_transform: self.swap_chain_pre_transform,
            composite_alpha: if transparent {
                0
            } else {
                VK_COMPOSITE_ALPHA_OPAQUE_BIT_KHR
            },
            present_mode,
            clipped: VK_TRUE,
            old_swapchain: self.swapchain,
            ..Default::default()
        };
        // SDL_RenderReadPixels() copies from the swapchain image, which requires TRANSFER_SRC
        if caps.supported_usage_flags & VK_IMAGE_USAGE_TRANSFER_SRC_BIT != 0 {
            swapchain_create_info.image_usage |= VK_IMAGE_USAGE_TRANSFER_SRC_BIT;
        }
        if !transparent {
            // Set the first supported swap chain composite mode
            // Android doesn't support VK_COMPOSITE_ALPHA_OPAQUE_BIT_KHR, for example
            let composite_alpha_flags = [
                VK_COMPOSITE_ALPHA_OPAQUE_BIT_KHR,
                VK_COMPOSITE_ALPHA_INHERIT_BIT_KHR,
            ];
            for flag in composite_alpha_flags {
                if caps.supported_composite_alpha & flag != 0 {
                    swapchain_create_info.composite_alpha = flag;
                    break;
                }
            }
            if swapchain_create_info.composite_alpha == 0 {
                // Fall back to VK_COMPOSITE_ALPHA_OPAQUE_BIT_KHR, unsupported but better than nothing
                swapchain_create_info.composite_alpha = VK_COMPOSITE_ALPHA_OPAQUE_BIT_KHR;
            }
        }
        // SAFETY: a valid create info; the old swapchain is idle.
        let result = unsafe {
            (dev.create_swapchain_khr)(
                self.device,
                &swapchain_create_info,
                null(),
                &mut self.swapchain,
            )
        };

        if swapchain_create_info.old_swapchain != VK_NULL_HANDLE {
            // SAFETY: the old swapchain, retired by the creation.
            unsafe {
                (dev.destroy_swapchain_khr)(
                    self.device,
                    swapchain_create_info.old_swapchain,
                    null(),
                )
            };
        }

        if result != VK_SUCCESS {
            self.swapchain = VK_NULL_HANDLE;
            return Err(failure("vkCreateSwapchainKHR()", result));
        }

        self.swapchain_images = Vec::new();
        // SAFETY: a null array queries the count.
        let result = unsafe {
            (dev.get_swapchain_images_khr)(
                self.device,
                self.swapchain,
                &mut self.swapchain_image_count,
                null_mut(),
            )
        };
        if result != VK_SUCCESS {
            self.swapchain_image_count = 0;
            return Err(failure("vkGetSwapchainImagesKHR()", result));
        }

        let mut swapchain_images = vec![VK_NULL_HANDLE; self.swapchain_image_count as usize];
        // SAFETY: the array has room for the count.
        let result = unsafe {
            (dev.get_swapchain_images_khr)(
                self.device,
                self.swapchain,
                &mut self.swapchain_image_count,
                swapchain_images.as_mut_ptr(),
            )
        };
        if result != VK_SUCCESS {
            self.swapchain_image_count = 0;
            return Err(failure("vkGetSwapchainImagesKHR()", result));
        }
        swapchain_images.truncate(self.swapchain_image_count as usize);
        self.swapchain_images = swapchain_images;
        let image_count = self.swapchain_image_count as usize;

        // Create VkImageView's for swapchain images
        {
            let mut image_view_create_info = VkImageViewCreateInfo {
                s_type: VK_STRUCTURE_TYPE_IMAGE_VIEW_CREATE_INFO,
                flags: 0,
                format: self.surface_format.format,
                components: VkComponentMapping {
                    r: VK_COMPONENT_SWIZZLE_IDENTITY,
                    g: VK_COMPONENT_SWIZZLE_IDENTITY,
                    b: VK_COMPONENT_SWIZZLE_IDENTITY,
                    a: VK_COMPONENT_SWIZZLE_IDENTITY,
                },
                subresource_range: VkImageSubresourceRange {
                    aspect_mask: VK_IMAGE_ASPECT_COLOR_BIT,
                    base_array_layer: 0,
                    base_mip_level: 0,
                    layer_count: 1,
                    level_count: 1,
                },
                view_type: VK_IMAGE_VIEW_TYPE_2D,
                ..Default::default()
            };
            self.swapchain_image_layouts = vec![VK_IMAGE_LAYOUT_UNDEFINED; image_count];
            for i in 0..image_count {
                image_view_create_info.image = self.swapchain_images[i];
                let mut view = VK_NULL_HANDLE;
                // SAFETY: a valid create info for an image of the swapchain.
                let result = unsafe {
                    (dev.create_image_view)(self.device, &image_view_create_info, null(), &mut view)
                };
                if result != VK_SUCCESS {
                    self.destroy_all();
                    return Err(failure("vkCreateImageView()", result));
                }
                self.swapchain_image_views.push(view);
                self.swapchain_image_layouts[i] = VK_IMAGE_LAYOUT_UNDEFINED;
            }
        }

        let command_buffer_allocate_info = VkCommandBufferAllocateInfo {
            s_type: VK_STRUCTURE_TYPE_COMMAND_BUFFER_ALLOCATE_INFO,
            command_pool: self.command_pool,
            level: VK_COMMAND_BUFFER_LEVEL_PRIMARY,
            command_buffer_count: self.swapchain_image_count,
            ..Default::default()
        };
        let mut command_buffers = vec![null_mut(); image_count];
        // SAFETY: the array has room for the count.
        let result = unsafe {
            (dev.allocate_command_buffers)(
                self.device,
                &command_buffer_allocate_info,
                command_buffers.as_mut_ptr(),
            )
        };
        if result != VK_SUCCESS {
            self.destroy_all();
            return Err(failure("vkAllocateCommandBuffers()", result));
        }
        self.command_buffers = command_buffers;

        // Create fences
        for _ in 0..image_count {
            let fence_create_info = VkFenceCreateInfo {
                s_type: VK_STRUCTURE_TYPE_FENCE_CREATE_INFO,
                flags: VK_FENCE_CREATE_SIGNALED_BIT,
                ..Default::default()
            };
            let mut fence = VK_NULL_HANDLE;
            // SAFETY: a valid create info.
            let result =
                unsafe { (dev.create_fence)(self.device, &fence_create_info, null(), &mut fence) };
            if result != VK_SUCCESS {
                self.destroy_all();
                return Err(failure("vkCreateFence()", result));
            }
            self.fences.push(fence);
        }

        // Create renderpasses and framebuffer
        for render_pass in &mut self.render_passes {
            if *render_pass != VK_NULL_HANDLE {
                // SAFETY: the render pass is this device's, and idle.
                unsafe { (dev.destroy_render_pass)(self.device, *render_pass, null()) };
                *render_pass = VK_NULL_HANDLE;
            }
        }
        let mut framebuffers = Vec::with_capacity(image_count);
        let mut render_passes = [VK_NULL_HANDLE; VULKAN_RENDERPASS_COUNT];
        let views = self.swapchain_image_views.clone();
        let result = self.create_framebuffers_and_render_passes(
            self.swapchain_size.width as i32,
            self.swapchain_size.height as i32,
            self.surface_format.format,
            &views,
            &mut framebuffers,
            &mut render_passes,
        );
        self.framebuffers = framebuffers;
        self.render_passes = render_passes;
        if let Err((rc, _)) = result {
            self.destroy_all();
            return Err(failure("VULKAN_CreateFramebuffersAndRenderPasses()", rc));
        }

        // Create descriptor pools - start by allocating one per swapchain image, let it grow if more are needed
        // FIXME (upstream): a failed pool allocation isn't noticed (the
        // stale result is checked instead), leaving a null pool.
        for _ in 0..image_count {
            // Start by just allocating one pool, it will grow if needed
            let pool = self.allocate_descriptor_pool().unwrap_or(VK_NULL_HANDLE);
            self.descriptor_pools.push(vec![pool]);
        }

        // Create semaphores
        for _ in 0..image_count {
            match self.create_semaphore() {
                Ok(s) => self.image_available_semaphores.push(s),
                Err(e) => {
                    self.destroy_all();
                    return Err((VK_ERROR_UNKNOWN, e));
                }
            }
            match self.create_semaphore() {
                Ok(s) => self.rendering_finished_semaphores.push(s),
                Err(e) => {
                    self.destroy_all();
                    return Err((VK_ERROR_UNKNOWN, e));
                }
            }
        }

        // Upload buffers
        self.upload_buffers = (0..image_count)
            .map(|_| vec![Buffer::default(); SDL_VULKAN_NUM_UPLOAD_BUFFERS])
            .collect();
        self.current_upload_buffer = vec![0; image_count];

        // Constant buffers
        for _ in 0..image_count {
            // Start with just allocating one, will grow if needed
            match self.allocate_buffer(
                SDL_VULKAN_CONSTANT_BUFFER_DEFAULT_SIZE,
                VK_BUFFER_USAGE_UNIFORM_BUFFER_BIT,
                VK_MEMORY_PROPERTY_HOST_VISIBLE_BIT | VK_MEMORY_PROPERTY_HOST_COHERENT_BIT,
                VK_MEMORY_PROPERTY_DEVICE_LOCAL_BIT,
            ) {
                Ok(buffer) => self.constant_buffers.push(vec![buffer]),
                Err(failed) => {
                    self.destroy_all();
                    return Err(failed);
                }
            }
        }
        self.current_constant_buffer_offset = -1;
        self.current_constant_buffer_index = 0;

        // (its result isn't looked at)
        self.acquire_next_swapchain_image();

        self.publish_swapchain_properties();

        Ok(())
    }

    /// Initialize all resources that change when the window's size
    /// changes. Translation of `VULKAN_CreateWindowSizeDependentResources()`.
    fn create_window_size_dependent_resources(&mut self) -> VkResultOr<()> {
        // Release resources in the current command list
        self.issue_batch();
        self.wait_for_gpu();

        /* The width and height of the swap chain must be based on the display's
         * non-rotated size.
         */
        let (w, h) = self.window.size_in_pixels().unwrap_or((0, 0));

        let result = self.create_swap_chain(w, h);
        if result.is_err() {
            self.recreate_swapchain = true;
        }

        self.viewport_dirty = true;

        result
    }

    /// Translation of `VULKAN_HandleDeviceLost()`.
    fn handle_device_lost(&mut self) -> bool {
        let mut recovered = false;

        self.destroy_all();

        let result = self.create_device_resources().and_then(|()| {
            self.create_window_size_dependent_resources()
                .map_err(|(_, e)| e)
        });
        match result {
            Ok(()) => recovered = true,
            Err(e) => {
                crate::log::error!(
                    Category::Render,
                    "Renderer couldn't recover from device lost: {}",
                    e
                );
                self.destroy_all();
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
    /// `VULKAN_UpdateForWindowSizeChange()`.
    fn update_for_window_size_change(&mut self) -> VkResultOr<()> {
        // If the GPU has previous work, wait for it to be done first
        self.wait_for_gpu();

        self.create_window_size_dependent_resources()
    }

    /// Translation of `VULKAN_DestroyTexture()`, for the renderer's data of
    /// a texture.
    fn destroy_texture_data(&mut self, id: u32) {
        let Some(mut texture_data) = self.textures.remove(&id) else {
            return;
        };
        if self.texture_render_target == Some(id) {
            self.texture_render_target = None;
        }
        if self.dev.is_none() {
            return;
        }

        /* Because SDL_DestroyTexture might be called while the data is in-flight, we need to issue the batch first
        Unfortunately, this means that deleting a lot of textures mid-frame will have poor performance. */
        self.issue_batch();
        self.wait_for_gpu();

        let dev = self.dev();
        for view in &mut texture_data.image_views[..texture_data.num_image_views] {
            if *view != VK_NULL_HANDLE {
                // SAFETY: the view is this device's, and idle.
                unsafe { (dev.destroy_image_view)(self.device, *view, null()) };
                *view = VK_NULL_HANDLE;
            }
        }
        for i in 0..texture_data.num_images {
            self.destroy_image(&mut texture_data.images[i]);
        }

        self.destroy_buffer(&mut texture_data.staging_buffer);
        // SAFETY: the framebuffer and render passes are this device's, and
        // idle.
        unsafe {
            if texture_data.main_framebuffer != VK_NULL_HANDLE {
                (dev.destroy_framebuffer)(self.device, texture_data.main_framebuffer, null());
                texture_data.main_framebuffer = VK_NULL_HANDLE;
            }
            for render_pass in &mut texture_data.main_renderpasses {
                if *render_pass != VK_NULL_HANDLE {
                    (dev.destroy_render_pass)(self.device, *render_pass, null());
                    *render_pass = VK_NULL_HANDLE;
                }
            }
        }
    }

    /// Translation of `VULKAN_UpdateTextureInternal()`: the image's new
    /// layout.
    #[allow(clippy::too_many_arguments)]
    fn update_texture_internal(
        &mut self,
        image: VkImage,
        format: VkFormat,
        plane: u32,
        (x, y, w, h): (i32, i32, i32, i32),
        pixels: &[u8],
        pitch: usize,
        image_layout: VkImageLayout,
    ) -> Result<VkImageLayout> {
        let pixel_size = get_bytes_per_pixel(format, plane);
        let mut length = w as VkDeviceSize * pixel_size;
        let upload_buffer_size = length * h as VkDeviceSize;
        let plane_count = vk_format_get_num_planes(format);
        self.check_device()?;

        // (the C code reads the rows without knowing the buffer's size)
        let rows = h.max(0) as usize;
        let row_length = (length as usize).min(pitch);
        let needed = if length as usize == pitch {
            length as usize * rows
        } else if rows == 0 {
            0
        } else {
            (rows - 1) * pitch + row_length
        };
        if pixels.len() < needed {
            return Err(Error::invalid_param("pixels"));
        }

        self.ensure_command_buffer();

        let index = self.current_command_buffer_index as usize;
        let current_upload_buffer_index = self.current_upload_buffer[index];

        let mut upload_buffer = self
            .allocate_buffer(
                upload_buffer_size,
                VK_BUFFER_USAGE_TRANSFER_SRC_BIT,
                VK_MEMORY_PROPERTY_HOST_VISIBLE_BIT | VK_MEMORY_PROPERTY_HOST_COHERENT_BIT,
                VK_MEMORY_PROPERTY_DEVICE_LOCAL_BIT,
            )
            .map_err(|(_, e)| e)?;

        let dst = upload_buffer.mapped();
        if length as usize == pitch {
            dst[..needed].copy_from_slice(&pixels[..needed]);
        } else {
            if length > pitch as VkDeviceSize {
                length = pitch as VkDeviceSize;
            }
            let length = length as usize;
            for row in 0..rows {
                dst[row * length..][..length].copy_from_slice(&pixels[row * pitch..][..length]);
            }
        }
        self.upload_buffers[index][current_upload_buffer_index] = upload_buffer;

        // Make sure the destination is in the correct resource state
        let mut image_layout = self.record_pipeline_image_barrier(
            VK_ACCESS_SHADER_READ_BIT
                | VK_ACCESS_COLOR_ATTACHMENT_WRITE_BIT
                | VK_ACCESS_COLOR_ATTACHMENT_READ_BIT
                | VK_ACCESS_TRANSFER_READ_BIT
                | VK_ACCESS_TRANSFER_WRITE_BIT,
            VK_ACCESS_TRANSFER_WRITE_BIT,
            VK_PIPELINE_STAGE_TRANSFER_BIT
                | VK_PIPELINE_STAGE_COLOR_ATTACHMENT_OUTPUT_BIT
                | VK_PIPELINE_STAGE_FRAGMENT_SHADER_BIT,
            VK_PIPELINE_STAGE_TRANSFER_BIT,
            VK_IMAGE_LAYOUT_TRANSFER_DST_OPTIMAL,
            image,
            image_layout,
        );

        let region = VkBufferImageCopy {
            buffer_offset: 0,
            buffer_row_length: 0,
            buffer_image_height: 0,
            image_subresource: VkImageSubresourceLayers {
                base_array_layer: 0,
                layer_count: 1,
                mip_level: 0,
                aspect_mask: if plane_count <= 1 {
                    VK_IMAGE_ASPECT_COLOR_BIT
                } else {
                    VK_IMAGE_ASPECT_PLANE_0_BIT << plane
                },
            },
            image_offset: VkOffset3D { x, y, z: 0 },
            image_extent: VkExtent3D {
                width: w as u32,
                height: h as u32,
                depth: 1,
            },
        };

        // SAFETY: the command buffer is recording, outside a render pass;
        // the upload buffer holds the region's pixels.
        unsafe {
            (self.dev().cmd_copy_buffer_to_image)(
                self.current_command_buffer,
                upload_buffer.buffer,
                image,
                image_layout,
                1,
                &region,
            )
        };

        // Transition the texture to be shader accessible
        image_layout = self.record_pipeline_image_barrier(
            VK_ACCESS_TRANSFER_WRITE_BIT,
            VK_ACCESS_SHADER_READ_BIT,
            VK_PIPELINE_STAGE_TRANSFER_BIT,
            VK_PIPELINE_STAGE_FRAGMENT_SHADER_BIT,
            VK_IMAGE_LAYOUT_SHADER_READ_ONLY_OPTIMAL,
            image,
            image_layout,
        );

        self.current_upload_buffer[index] += 1;

        // If we've used up all the upload buffers, we need to issue the batch
        if self.current_upload_buffer[index] == SDL_VULKAN_NUM_UPLOAD_BUFFERS {
            self.issue_batch();
        }

        Ok(image_layout)
    }

    /// `VULKAN_UpdateTextureInternal()` of image `i` of a texture, which
    /// takes the new layout.
    fn update_texture_image(
        &mut self,
        id: u32,
        i: usize,
        plane: u32,
        rect: (i32, i32, i32, i32),
        pixels: &[u8],
        pitch: usize,
    ) -> Result<()> {
        let image = self
            .textures
            .get(&id)
            .map(|t| t.images[i])
            .ok_or_else(not_available)?;
        let layout = self.update_texture_internal(
            image.image,
            image.format,
            plane,
            rect,
            pixels,
            pitch,
            image.image_layout,
        )?;
        if let Some(t) = self.textures.get_mut(&id) {
            t.images[i].image_layout = layout;
        }
        Ok(())
    }

    /// The key of the renderer's data of a texture.
    fn texture_id(texture: &TextureData) -> Result<u32> {
        texture_ref(texture).map(|r| r.id).ok_or_else(not_available)
    }

    /// Translation of `VULKAN_UpdateTexture()`.
    fn update_texture_pixels(
        &mut self,
        texture: &TextureData,
        rect: &Rect,
        src_pixels: &[u8],
        src_pitch: usize,
    ) -> Result<()> {
        let id = Self::texture_id(texture)?;
        let texture_data = self.textures.get(&id).ok_or_else(not_available)?;

        if texture.format == PixelFormat::EXTERNAL_OES {
            return Err(Error::unsupported());
        }

        let rows = rect.h.max(0) as usize;
        if texture_data.num_image_views == 2 {
            // NV12/NV21 data
            let uv_bpp = get_bytes_per_pixel(texture_data.images[0].format, 1) as usize;
            let y_pitch = src_pitch;
            let uv_pitch = (src_pitch + (uv_bpp - 1)) & !(uv_bpp - 1);
            let plane0 = src_pixels;
            let plane1 = plane(src_pixels, rows * src_pitch)?;

            return self.update_texture_nv_planes(
                texture,
                rect,
                (plane0, y_pitch),
                (plane1, uv_pitch),
            );
        } else if texture_data.num_image_views == 3 {
            // YUV data
            if texture.format == PixelFormat::I444 || texture.format == PixelFormat::I4FL {
                let plane0 = src_pixels;
                let plane1 = plane(plane0, rows * src_pitch)?;
                let plane2 = plane(plane1, rows * src_pitch)?;

                return self.update_texture_yuv_planes(
                    texture,
                    rect,
                    (plane0, src_pitch),
                    (plane1, src_pitch),
                    (plane2, src_pitch),
                );
            } else {
                let y_pitch = src_pitch;
                let uv_pitch = (y_pitch + texture.format.bytes_per_pixel() as usize) / 2;
                let plane0 = src_pixels;
                let plane1 = plane(plane0, rows * y_pitch)?;
                let plane2 = plane(plane1, rows.div_ceil(2) * uv_pitch)?;

                if texture.format == PixelFormat::YV12 {
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
        self.update_texture_image(
            id,
            0,
            0,
            (rect.x, rect.y, rect.w, rect.h),
            src_pixels,
            src_pitch,
        )
    }

    /// Translation of `VULKAN_UpdateTextureYUV()`.
    fn update_texture_yuv_planes(
        &mut self,
        texture: &TextureData,
        rect: &Rect,
        (y_plane, y_pitch): (&[u8], usize),
        (u_plane, u_pitch): (&[u8], usize),
        (v_plane, v_pitch): (&[u8], usize),
    ) -> Result<()> {
        let id = Self::texture_id(texture)?;
        if !self.textures.contains_key(&id) {
            return Err(not_available());
        }

        if texture.format == PixelFormat::EXTERNAL_OES {
            return Err(Error::unsupported());
        }

        let full = (rect.x, rect.y, rect.w, rect.h);
        let half = (rect.x / 2, rect.y / 2, (rect.w + 1) / 2, (rect.h + 1) / 2);
        self.update_texture_image(id, 0, 0, full, y_plane, y_pitch)?;
        if texture.format == PixelFormat::I444 || texture.format == PixelFormat::I4FL {
            self.update_texture_image(id, 1, 0, full, u_plane, u_pitch)?;
            self.update_texture_image(id, 2, 0, full, v_plane, v_pitch)?;
        } else if texture.format == PixelFormat::YV12 {
            self.update_texture_image(id, 2, 1, half, v_plane, v_pitch)?;
            self.update_texture_image(id, 1, 2, half, u_plane, u_pitch)?;
        } else {
            self.update_texture_image(id, 1, 1, half, u_plane, u_pitch)?;
            self.update_texture_image(id, 2, 2, half, v_plane, v_pitch)?;
        }
        Ok(())
    }

    /// Translation of `VULKAN_UpdateTextureNV()`.
    fn update_texture_nv_planes(
        &mut self,
        texture: &TextureData,
        rect: &Rect,
        (y_plane, y_pitch): (&[u8], usize),
        (uv_plane, uv_pitch): (&[u8], usize),
    ) -> Result<()> {
        let id = Self::texture_id(texture)?;
        if !self.textures.contains_key(&id) {
            return Err(not_available());
        }

        if texture.format == PixelFormat::EXTERNAL_OES {
            return Err(Error::unsupported());
        }

        self.update_texture_image(id, 0, 0, (rect.x, rect.y, rect.w, rect.h), y_plane, y_pitch)?;

        self.update_texture_image(
            id,
            0,
            1,
            (rect.x / 2, rect.y / 2, (rect.w + 1) / 2, (rect.h + 1) / 2),
            uv_plane,
            uv_pitch,
        )
    }

    /// Translation of `VULKAN_GetRotationForCurrentRenderTarget()`.
    fn get_rotation_for_current_render_target(&self) -> VkSurfaceTransformFlagBitsKHR {
        if self.texture_render_target.is_some() {
            VK_SURFACE_TRANSFORM_IDENTITY_BIT_KHR
        } else {
            self.swap_chain_pre_transform
        }
    }

    /// Translation of `VULKAN_UpdateViewport()`.
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

        let projection = match rotation {
            VK_SURFACE_TRANSFORM_ROTATE_270_BIT_KHR => {
                Float4X4::rotation_z(std::f32::consts::PI * 0.5)
            }
            VK_SURFACE_TRANSFORM_ROTATE_180_BIT_KHR => Float4X4::rotation_z(std::f32::consts::PI),
            VK_SURFACE_TRANSFORM_ROTATE_90_BIT_KHR => {
                Float4X4::rotation_z(-std::f32::consts::PI * 0.5)
            }
            _ => Float4X4::identity(),
        };

        // Update the view matrix
        let mut view = Float4X4::default();
        view.m[0][0] = 2.0 / viewport.w as f32;
        view.m[1][1] = -2.0 / viewport.h as f32;
        view.m[2][2] = 1.0;
        view.m[3][0] = -1.0;
        view.m[3][1] = 1.0;
        view.m[3][3] = 1.0;

        self.vertex_shader_constants_data.projection_and_view =
            Float4X4::multiply(&view, &projection);

        let swap_dimensions = is_display_rotated_90_degrees(rotation);
        let vk_viewport = if swap_dimensions {
            VkViewport {
                x: viewport.y as f32,
                y: viewport.x as f32,
                width: viewport.h as f32,
                height: viewport.w as f32,
                min_depth: 0.0,
                max_depth: 1.0,
            }
        } else {
            VkViewport {
                x: viewport.x as f32,
                y: viewport.y as f32,
                width: viewport.w as f32,
                height: viewport.h as f32,
                min_depth: 0.0,
                max_depth: 1.0,
            }
        };
        // SAFETY: the command buffer is recording.
        unsafe { (self.dev().cmd_set_viewport)(self.current_command_buffer, 0, 1, &vk_viewport) };

        self.viewport_dirty = false;
        true
    }

    /// Translation of `VULKAN_UpdateClipRect()`.
    fn update_clip_rect(&mut self) {
        let viewport = self.current_viewport;
        let rotation = self.get_rotation_for_current_render_target();
        let swap_dimensions = is_display_rotated_90_degrees(rotation);

        let mut scissor = if self.current_cliprect_enabled {
            VkRect2D {
                offset: VkOffset2D {
                    x: viewport.x + self.current_cliprect.x,
                    y: viewport.y + self.current_cliprect.y,
                },
                extent: VkExtent2D {
                    width: self.current_cliprect.w as u32,
                    height: self.current_cliprect.h as u32,
                },
            }
        } else {
            VkRect2D {
                offset: VkOffset2D {
                    x: viewport.x,
                    y: viewport.y,
                },
                extent: VkExtent2D {
                    width: viewport.w as u32,
                    height: viewport.h as u32,
                },
            }
        };
        if swap_dimensions {
            let scissor_temp = scissor;
            scissor.offset.x = scissor_temp.offset.y;
            scissor.offset.y = scissor_temp.offset.x;
            scissor.extent.width = scissor_temp.extent.height;
            scissor.extent.height = scissor_temp.extent.width;
        }
        // SAFETY: the command buffer is recording.
        unsafe { (self.dev().cmd_set_scissor)(self.current_command_buffer, 0, 1, &scissor) };

        self.cliprect_dirty = false;
    }

    /// The HDR headroom of the output (`renderer->target->HDR_headroom` or
    /// `renderer->HDR_headroom`).
    fn output_headroom(&self, textures: &TextureStore) -> f32 {
        match self.target.and_then(|t| textures.get(t)) {
            Some(target) => target.hdr_headroom,
            None => self.hdr_headroom,
        }
    }

    /// Translation of `VULKAN_SetupShaderConstants()`.
    fn setup_shader_constants(
        &self,
        cmd: &DrawCmd,
        texture: Option<(&SourceTexture, &VulkanTexture)>,
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
                    } else if texture.colorspace.primaries() == ColorPrimaries::Bt2020
                        && texture.colorspace.transfer() == TransferCharacteristics::Pq
                    {
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
        match shader_constants {
            Some(shader_constants) => {
                if self.current_colorspace == Colorspace::HDR10 {
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
                if self.current_colorspace == Colorspace::HDR10 {
                    return Shader::SolidPq;
                }

                Shader::Solid
            }
        }
    }

    /// Translation of `VULKAN_AllocateDescriptorPool()`.
    fn allocate_descriptor_pool(&self) -> Result<VkDescriptorPool> {
        let mut descriptor_pool = VK_NULL_HANDLE;
        let descriptor_pool_sizes = [
            VkDescriptorPoolSize {
                descriptor_count: SDL_VULKAN_MAX_DESCRIPTOR_SETS,
                ty: VK_DESCRIPTOR_TYPE_SAMPLER,
            },
            VkDescriptorPoolSize {
                descriptor_count: SDL_VULKAN_NUM_TEXTURE_BINDINGS as u32
                    * SDL_VULKAN_MAX_DESCRIPTOR_SETS,
                ty: VK_DESCRIPTOR_TYPE_COMBINED_IMAGE_SAMPLER,
            },
            VkDescriptorPoolSize {
                descriptor_count: SDL_VULKAN_MAX_DESCRIPTOR_SETS,
                ty: VK_DESCRIPTOR_TYPE_UNIFORM_BUFFER,
            },
        ];

        let descriptor_pool_create_info = VkDescriptorPoolCreateInfo {
            s_type: VK_STRUCTURE_TYPE_DESCRIPTOR_POOL_CREATE_INFO,
            pool_size_count: descriptor_pool_sizes.len() as u32,
            p_pool_sizes: descriptor_pool_sizes.as_ptr(),
            max_sets: SDL_VULKAN_MAX_DESCRIPTOR_SETS,
            ..Default::default()
        };
        // SAFETY: the create info and the sizes outlive the call.
        let result = unsafe {
            (self.dev().create_descriptor_pool)(
                self.device,
                &descriptor_pool_create_info,
                null(),
                &mut descriptor_pool,
            )
        };
        check("vkCreateDescrptorPool()", result)?;

        Ok(descriptor_pool)
    }

    /// Translation of `VULKAN_CreateDescriptorSetAndPipelineLayout()`.
    fn create_descriptor_set_and_pipeline_layout(
        &self,
        sampler_ycbcr: VkSampler,
    ) -> Result<(VkDescriptorSetLayout, VkPipelineLayout)> {
        let dev = self.dev();
        let mut descriptor_set_layout_out = VK_NULL_HANDLE;
        let mut pipeline_layout_out = VK_NULL_HANDLE;

        // Descriptor set layout
        let mut layout_bindings =
            [VkDescriptorSetLayoutBinding::default(); 1 + SDL_VULKAN_NUM_TEXTURE_BINDINGS];
        // PixelShaderConstants
        layout_bindings[0] = VkDescriptorSetLayoutBinding {
            binding: 0,
            descriptor_count: 1,
            descriptor_type: VK_DESCRIPTOR_TYPE_UNIFORM_BUFFER,
            stage_flags: VK_SHADER_STAGE_FRAGMENT_BIT,
            p_immutable_samplers: null(),
        };

        for (i, binding) in layout_bindings.iter_mut().enumerate().skip(1) {
            *binding = VkDescriptorSetLayoutBinding {
                binding: i as u32,
                descriptor_count: 1,
                descriptor_type: VK_DESCRIPTOR_TYPE_COMBINED_IMAGE_SAMPLER,
                stage_flags: VK_SHADER_STAGE_FRAGMENT_BIT,
                p_immutable_samplers: if sampler_ycbcr != VK_NULL_HANDLE {
                    &sampler_ycbcr
                } else {
                    null()
                },
            };
        }

        let descriptor_set_layout_create_info = VkDescriptorSetLayoutCreateInfo {
            s_type: VK_STRUCTURE_TYPE_DESCRIPTOR_SET_LAYOUT_CREATE_INFO,
            flags: 0,
            binding_count: layout_bindings.len() as u32,
            p_bindings: layout_bindings.as_ptr(),
            ..Default::default()
        };
        // SAFETY: the create info and the bindings outlive the call.
        let result = unsafe {
            (dev.create_descriptor_set_layout)(
                self.device,
                &descriptor_set_layout_create_info,
                null(),
                &mut descriptor_set_layout_out,
            )
        };
        check("vkCreateDescriptorSetLayout()", result)?;

        // Pipeline layout
        let push_constant_range = VkPushConstantRange {
            size: size_of::<VertexShaderConstants>() as u32,
            offset: 0,
            stage_flags: VK_SHADER_STAGE_VERTEX_BIT,
        };
        let pipeline_layout_create_info = VkPipelineLayoutCreateInfo {
            s_type: VK_STRUCTURE_TYPE_PIPELINE_LAYOUT_CREATE_INFO,
            set_layout_count: 1,
            p_set_layouts: &descriptor_set_layout_out,
            push_constant_range_count: 1,
            p_push_constant_ranges: &push_constant_range,
            ..Default::default()
        };
        // SAFETY: as above.
        let result = unsafe {
            (dev.create_pipeline_layout)(
                self.device,
                &pipeline_layout_create_info,
                null(),
                &mut pipeline_layout_out,
            )
        };
        if result != VK_SUCCESS {
            // (upstream leaves the descriptor set layout to the caller,
            // which forgets it)
            // SAFETY: the layout just made, unused.
            unsafe {
                (dev.destroy_descriptor_set_layout)(self.device, descriptor_set_layout_out, null())
            };
            return Err(error_code("vkCreatePipelineLayout()", result));
        }

        Ok((descriptor_set_layout_out, pipeline_layout_out))
    }

    /// Translation of `VULKAN_AllocateDescriptorSet()`.
    fn allocate_descriptor_set(
        &mut self,
        descriptor_set_layout: VkDescriptorSetLayout,
        constant_buffer: VkBuffer,
        constant_buffer_offset: VkDeviceSize,
        image_views: &[VkImageView],
        samplers: &[VkSampler],
    ) -> Result<VkDescriptorSet> {
        let dev = self.dev();
        let index = self.current_command_buffer_index as usize;
        let mut current_descriptor_pool_index = self.current_descriptor_pool_index;
        let mut descriptor_pool = self.descriptor_pools[index][current_descriptor_pool_index];

        let mut descriptor_set_allocate_info = VkDescriptorSetAllocateInfo {
            s_type: VK_STRUCTURE_TYPE_DESCRIPTOR_SET_ALLOCATE_INFO,
            descriptor_set_count: 1,
            descriptor_pool,
            p_set_layouts: &descriptor_set_layout,
            ..Default::default()
        };

        let mut descriptor_set = VK_NULL_HANDLE;
        let mut result = if self.current_descriptor_set_index >= SDL_VULKAN_MAX_DESCRIPTOR_SETS {
            VK_ERROR_OUT_OF_DEVICE_MEMORY
        } else {
            VK_SUCCESS
        };
        if result == VK_SUCCESS {
            // SAFETY: a valid allocate info.
            result = unsafe {
                (dev.allocate_descriptor_sets)(
                    self.device,
                    &descriptor_set_allocate_info,
                    &mut descriptor_set,
                )
            };
        }
        if result != VK_SUCCESS {
            // Out of descriptor sets in this pool - see if we have more pools allocated
            current_descriptor_pool_index += 1;
            if current_descriptor_pool_index < self.descriptor_pools[index].len() {
                descriptor_pool = self.descriptor_pools[index][current_descriptor_pool_index];
                descriptor_set_allocate_info.descriptor_pool = descriptor_pool;
                // SAFETY: as above.
                result = unsafe {
                    (dev.allocate_descriptor_sets)(
                        self.device,
                        &descriptor_set_allocate_info,
                        &mut descriptor_set,
                    )
                };
                if result != VK_SUCCESS {
                    // This should not fail - we are allocating from the front of the descriptor set
                    return Err(Error::new("Unable to allocate descriptor set"));
                }
                self.current_descriptor_pool_index = current_descriptor_pool_index;
                self.current_descriptor_set_index = 0;
            }
            // We are out of pools, create a new one
            else {
                // (the error comes from VULKAN_AllocateDescriptorPool() if we failed to allocate a new pool)
                descriptor_pool = self.allocate_descriptor_pool()?;
                self.descriptor_pools[index].push(descriptor_pool);
                self.current_descriptor_pool_index = current_descriptor_pool_index;
                self.current_descriptor_set_index = 0;

                // Call recursively to allocate from the new pool
                return self.allocate_descriptor_set(
                    descriptor_set_layout,
                    constant_buffer,
                    constant_buffer_offset,
                    image_views,
                    samplers,
                );
            }
        }
        self.current_descriptor_set_index += 1;
        let mut combined_image_sampler_descriptor =
            [VkDescriptorImageInfo::default(); SDL_VULKAN_NUM_TEXTURE_BINDINGS];
        let buffer_descriptor = VkDescriptorBufferInfo {
            buffer: constant_buffer,
            offset: constant_buffer_offset,
            range: size_of::<PixelShaderConstants>() as VkDeviceSize,
        };

        let mut descriptor_writes =
            [VkWriteDescriptorSet::default(); 1 + SDL_VULKAN_NUM_TEXTURE_BINDINGS];
        let mut descriptor_count = 1; // Always have the uniform buffer

        descriptor_writes[0] = VkWriteDescriptorSet {
            s_type: VK_STRUCTURE_TYPE_WRITE_DESCRIPTOR_SET,
            dst_set: descriptor_set,
            dst_binding: 0,
            dst_array_element: 0,
            descriptor_count: 1,
            descriptor_type: VK_DESCRIPTOR_TYPE_UNIFORM_BUFFER,
            p_buffer_info: &buffer_descriptor,
            ..Default::default()
        };

        crate::sdl_assert!(samplers.len() == image_views.len());
        for (i, &image_view) in image_views.iter().enumerate() {
            crate::sdl_assert!(i < combined_image_sampler_descriptor.len());
            let p_image_info = &mut combined_image_sampler_descriptor[i];
            *p_image_info = VkDescriptorImageInfo::default();
            if descriptor_set_layout == self.descriptor_set_layout {
                p_image_info.sampler = samplers[i];
            } else {
                // Ignore the sampler if we're using YcBcCr data since it will be baked in the descriptor set layout
            }
            p_image_info.image_view = image_view;
            p_image_info.image_layout = VK_IMAGE_LAYOUT_SHADER_READ_ONLY_OPTIMAL;
        }
        for (i, image_info) in combined_image_sampler_descriptor
            .iter()
            .enumerate()
            .take(image_views.len())
        {
            crate::sdl_assert!(descriptor_count < descriptor_writes.len());
            descriptor_writes[descriptor_count] = VkWriteDescriptorSet {
                s_type: VK_STRUCTURE_TYPE_WRITE_DESCRIPTOR_SET,
                dst_set: descriptor_set,
                dst_binding: 1 + i as u32,
                dst_array_element: 0,
                descriptor_count: 1,
                descriptor_type: VK_DESCRIPTOR_TYPE_COMBINED_IMAGE_SAMPLER,
                p_image_info: image_info,
                ..Default::default()
            };
            descriptor_count += 1;
        }

        // SAFETY: the writes and the infos they point to outlive the call.
        unsafe {
            (dev.update_descriptor_sets)(
                self.device,
                descriptor_count as u32,
                descriptor_writes.as_ptr(),
                0,
                null(),
            )
        };

        Ok(descriptor_set)
    }

    /// Translation of `VULKAN_SetDrawState()`.
    #[allow(clippy::too_many_arguments)]
    fn set_draw_state(
        &mut self,
        cmd: &DrawCmd,
        pipeline_layout: VkPipelineLayout,
        descriptor_set_layout: VkDescriptorSetLayout,
        shader_constants: Option<&PixelShaderConstants>,
        topology: VkPrimitiveTopology,
        image_views: &[VkImageView],
        samplers: &[VkSampler],
        matrix: Option<&Float4X4>,
        state_cache: &DrawStateCache,
    ) -> Result<()> {
        let blend_mode = cmd.blend;
        let newmatrix = *matrix.unwrap_or(&self.identity);
        let mut update_constants = false;
        let shader = self.select_shader(shader_constants);

        self.activate_command_buffer(VK_ATTACHMENT_LOAD_OP_LOAD, None, state_cache);

        let format = match self
            .texture_render_target
            .and_then(|id| self.textures.get(&id))
        {
            Some(target) => target.images[0].format,
            None => self.surface_format.format,
        };

        // See if we need to change the pipeline state
        let matches = |state: &PipelineState| {
            state.shader == shader
                && state.blend_mode == blend_mode
                && state.topology == topology
                && state.format == format
                && state.pipeline_layout == pipeline_layout
                && state.descriptor_set_layout == descriptor_set_layout
        };
        if !self
            .current_pipeline_state
            .is_some_and(|i| matches(&self.pipeline_states[i]))
        {
            self.current_pipeline_state = self.pipeline_states.iter().position(matches);

            // If we didn't find a match, create a new one -- it must mean the blend mode is non-standard
            if self.current_pipeline_state.is_none() {
                self.current_pipeline_state = self
                    .create_pipeline_state(
                        shader,
                        pipeline_layout,
                        descriptor_set_layout,
                        blend_mode,
                        topology,
                        format,
                    )
                    .ok();
            }

            let Some(current) = self.current_pipeline_state else {
                return Err(Error::new("Unable to create required pipeline state"));
            };

            // SAFETY: the command buffer is recording; the pipeline is this
            // device's.
            unsafe {
                (self.dev().cmd_bind_pipeline)(
                    self.current_command_buffer,
                    VK_PIPELINE_BIND_POINT_GRAPHICS,
                    self.pipeline_states[current].pipeline,
                )
            };
            update_constants = true;
        }
        let current = self.current_pipeline_state.expect("a pipeline state");

        if self.viewport_dirty && self.update_viewport() {
            // vertexShaderConstantsData.projectionAndView has changed
            update_constants = true;
        }

        if self.cliprect_dirty {
            self.update_clip_rect();
        }

        if update_constants
            || !self
                .vertex_shader_constants_data
                .model
                .same_bits(&newmatrix)
        {
            self.vertex_shader_constants_data.model = newmatrix;
            // SAFETY: the command buffer is recording; the constants are
            // the size of the push constant range.
            unsafe {
                (self.dev().cmd_push_constants)(
                    self.current_command_buffer,
                    self.pipeline_states[current].pipeline_layout,
                    VK_SHADER_STAGE_VERTEX_BIT,
                    0,
                    size_of::<VertexShaderConstants>() as u32,
                    &self.vertex_shader_constants_data as *const _ as *const c_void,
                )
            };
        }

        let solid_constants;
        let shader_constants = match shader_constants {
            Some(constants) => constants,
            None => {
                solid_constants = self.setup_shader_constants(cmd, None, 0.0);
                &solid_constants
            }
        };

        let index = self.current_command_buffer_index as usize;
        let mut constant_buffer =
            self.constant_buffers[index][self.current_constant_buffer_index].buffer;
        let mut constant_buffer_offset = self.current_constant_buffer_offset.max(0) as VkDeviceSize;
        if update_constants
            || !shader_constants.same_bits(&self.pipeline_states[current].shader_constants)
        {
            if self.current_constant_buffer_offset == -1 {
                // First time, grab offset 0
                self.current_constant_buffer_offset = 0;
                constant_buffer_offset = 0;
            } else {
                // Align the next address to the minUniformBufferOffsetAlignment
                let alignment = self
                    .physical_device_properties
                    .limits
                    .min_uniform_buffer_offset_alignment;
                crate::sdl_assert!(self.current_constant_buffer_offset >= 0);
                self.current_constant_buffer_offset +=
                    ((size_of::<PixelShaderConstants>() as VkDeviceSize + alignment - 1)
                        & !(alignment - 1)) as i32;
                constant_buffer_offset = self.current_constant_buffer_offset as VkDeviceSize;
            }

            // If we have run out of size in this constant buffer, create another if needed
            // FIXME (upstream): only the offset is checked, so with an
            // aligned size that doesn't divide the buffer's (a
            // minUniformBufferOffsetAlignment under 32) the last constants
            // of a buffer go past its end (here only what fits is written).
            if self.current_constant_buffer_offset as VkDeviceSize
                >= SDL_VULKAN_CONSTANT_BUFFER_DEFAULT_SIZE
            {
                let new_constant_buffer_index = self.current_constant_buffer_index + 1;
                // We need a new constant buffer
                if new_constant_buffer_index >= self.constant_buffers[index].len() {
                    let new_constant_buffer = self
                        .allocate_buffer(
                            SDL_VULKAN_CONSTANT_BUFFER_DEFAULT_SIZE,
                            VK_BUFFER_USAGE_UNIFORM_BUFFER_BIT,
                            VK_MEMORY_PROPERTY_HOST_VISIBLE_BIT
                                | VK_MEMORY_PROPERTY_HOST_COHERENT_BIT,
                            VK_MEMORY_PROPERTY_DEVICE_LOCAL_BIT,
                        )
                        .map_err(|(_, e)| e)?;
                    self.constant_buffers[index].push(new_constant_buffer);
                }
                self.current_constant_buffer_index = new_constant_buffer_index;
                self.current_constant_buffer_offset = 0;
                constant_buffer_offset = 0;
                constant_buffer =
                    self.constant_buffers[index][self.current_constant_buffer_index].buffer;
            }

            self.pipeline_states[current].shader_constants = *shader_constants;

            // Upload constants to persistently mapped buffer
            let words = shader_constants.words();
            let buffer = &mut self.constant_buffers[index][self.current_constant_buffer_index];
            let dst = &mut buffer.mapped()[constant_buffer_offset as usize..];
            for (d, w) in dst.chunks_exact_mut(4).zip(words) {
                d.copy_from_slice(&w.to_ne_bytes());
            }
        }

        // Allocate/update descriptor set with the bindings
        let descriptor_set = self.allocate_descriptor_set(
            descriptor_set_layout,
            constant_buffer,
            constant_buffer_offset,
            image_views,
            samplers,
        )?;

        // Bind the descriptor set with the sampler/UBO/image views
        // SAFETY: the command buffer is recording; the set and layout are
        // this device's.
        unsafe {
            (self.dev().cmd_bind_descriptor_sets)(
                self.current_command_buffer,
                VK_PIPELINE_BIND_POINT_GRAPHICS,
                self.pipeline_states[current].pipeline_layout,
                0,
                1,
                &descriptor_set,
                0,
                null(),
            )
        };

        Ok(())
    }

    /// `VULKAN_SetDrawState()` for an untextured draw.
    fn set_solid_draw_state(
        &mut self,
        cmd: &DrawCmd,
        topology: VkPrimitiveTopology,
        state_cache: &DrawStateCache,
    ) -> Result<()> {
        self.set_draw_state(
            cmd,
            self.pipeline_layout,
            self.descriptor_set_layout,
            None,
            topology,
            &[],
            &[],
            None,
            state_cache,
        )
    }

    /// Translation of `VULKAN_GetSampler()`.
    fn get_sampler(
        &mut self,
        format: PixelFormat,
        mut scale_mode: ScaleMode,
        address_u: TextureAddressMode,
        address_v: TextureAddressMode,
    ) -> Result<VkSampler> {
        if format == PixelFormat::INDEX8 {
            // We'll do linear sampling in the shader if needed
            scale_mode = ScaleMode::Nearest;
        }

        let key = render_sampler_hashkey(scale_mode, address_u, address_v);
        crate::sdl_assert!(key < self.samplers.len());
        if self.samplers[key] == VK_NULL_HANDLE {
            let mut sampler_create_info = VkSamplerCreateInfo {
                s_type: VK_STRUCTURE_TYPE_SAMPLER_CREATE_INFO,
                mipmap_mode: VK_SAMPLER_MIPMAP_MODE_NEAREST,
                address_mode_w: VK_SAMPLER_ADDRESS_MODE_CLAMP_TO_EDGE,
                mip_lod_bias: 0.0,
                anisotropy_enable: VK_FALSE,
                max_anisotropy: 1.0,
                min_lod: 0.0,
                max_lod: 1000.0,
                ..Default::default()
            };
            match scale_mode {
                ScaleMode::Nearest => {
                    sampler_create_info.mag_filter = VK_FILTER_NEAREST;
                    sampler_create_info.min_filter = VK_FILTER_NEAREST;
                }
                ScaleMode::PixelArt | ScaleMode::Linear => {
                    // (pixel art uses linear sampling)
                    sampler_create_info.mag_filter = VK_FILTER_LINEAR;
                    sampler_create_info.min_filter = VK_FILTER_LINEAR;
                }
            }
            sampler_create_info.address_mode_u = match address_u {
                TextureAddressMode::Clamp => VK_SAMPLER_ADDRESS_MODE_CLAMP_TO_EDGE,
                TextureAddressMode::Wrap => VK_SAMPLER_ADDRESS_MODE_REPEAT,
                other => {
                    return Err(Error::new(format!(
                        "Unknown texture address mode: {}",
                        other as i32
                    )))
                }
            };
            sampler_create_info.address_mode_v = match address_v {
                TextureAddressMode::Clamp => VK_SAMPLER_ADDRESS_MODE_CLAMP_TO_EDGE,
                TextureAddressMode::Wrap => VK_SAMPLER_ADDRESS_MODE_REPEAT,
                other => {
                    return Err(Error::new(format!(
                        "Unknown texture address mode: {}",
                        other as i32
                    )))
                }
            };
            // SAFETY: a valid create info.
            let result = unsafe {
                (self.dev().create_sampler)(
                    self.device,
                    &sampler_create_info,
                    null(),
                    &mut self.samplers[key],
                )
            };
            check("vkCreateSampler()", result)?;
        }
        Ok(self.samplers[key])
    }

    /// The barrier that makes a sampled image readable by the shaders, with
    /// the render pass stopped and restarted around it (in
    /// `VULKAN_SetCopyState()`).
    fn make_shader_readable(
        &mut self,
        image: VkImage,
        image_layout: VkImageLayout,
    ) -> VkImageLayout {
        let mut stopped_render_pass = false;
        if self.current_render_pass != VK_NULL_HANDLE {
            self.end_render_pass();
            stopped_render_pass = true;
        }
        let layout = self.record_pipeline_image_barrier(
            VK_ACCESS_TRANSFER_WRITE_BIT
                | VK_ACCESS_SHADER_READ_BIT
                | VK_ACCESS_COLOR_ATTACHMENT_READ_BIT
                | VK_ACCESS_COLOR_ATTACHMENT_WRITE_BIT,
            VK_ACCESS_SHADER_READ_BIT,
            VK_PIPELINE_STAGE_TRANSFER_BIT
                | VK_PIPELINE_STAGE_FRAGMENT_SHADER_BIT
                | VK_PIPELINE_STAGE_COLOR_ATTACHMENT_OUTPUT_BIT,
            VK_PIPELINE_STAGE_FRAGMENT_SHADER_BIT,
            VK_IMAGE_LAYOUT_SHADER_READ_ONLY_OPTIMAL,
            image,
            image_layout,
        );
        if stopped_render_pass {
            self.begin_render_pass(VK_ATTACHMENT_LOAD_OP_LOAD, None);
        }
        layout
    }

    /// Translation of `VULKAN_SetCopyState()`.
    fn set_copy_state(
        &mut self,
        cmd: &DrawCmd,
        textures: &TextureStore,
        state_cache: &DrawStateCache,
    ) -> Result<()> {
        let handle = cmd.texture.ok_or_else(invalid_texture)?;
        let texture = textures.get(handle).ok_or_else(invalid_texture)?;
        let source = SourceTexture::of(texture);
        let id = Self::texture_id(texture)?;
        let output_headroom = self.output_headroom(textures);
        let texture_data = self.textures.get(&id).ok_or_else(not_available)?;
        let mut image_views = Vec::with_capacity(SDL_VULKAN_NUM_TEXTURE_BINDINGS);
        let mut samplers = Vec::with_capacity(SDL_VULKAN_NUM_TEXTURE_BINDINGS);

        let constants =
            self.setup_shader_constants(cmd, Some((&source, texture_data)), output_headroom);

        for i in 0..texture_data.num_images {
            let image = self.textures[&id].images[i];
            if image.image_layout != VK_IMAGE_LAYOUT_SHADER_READ_ONLY_OPTIMAL {
                let layout = self.make_shader_readable(image.image, image.image_layout);
                if let Some(t) = self.textures.get_mut(&id) {
                    t.images[i].image_layout = layout;
                }
            }
        }
        let texture_data = &self.textures[&id];
        image_views.extend_from_slice(&texture_data.image_views[..texture_data.num_image_views]);
        let palette = texture_data.palette;

        samplers.push(self.get_sampler(
            source.format,
            cmd.texture_scale_mode,
            cmd.texture_address_mode_u,
            cmd.texture_address_mode_v,
        )?);

        if let Some(palette) = palette
            .filter(|_| source.has_palette)
            .filter(|p| self.palettes.contains_key(p))
        {
            let image = self.palettes[&palette].image;
            if image.image_layout != VK_IMAGE_LAYOUT_SHADER_READ_ONLY_OPTIMAL {
                let layout = self.make_shader_readable(image.image, image.image_layout);
                if let Some(p) = self.palettes.get_mut(&palette) {
                    p.image.image_layout = layout;
                }
            }
            image_views.push(self.palettes[&palette].image_view);

            samplers.push(self.get_sampler(
                PixelFormat::UNKNOWN,
                ScaleMode::Nearest,
                TextureAddressMode::Clamp,
                TextureAddressMode::Clamp,
            )?);
        }

        // Fill out the rest of the image views and samplers
        while image_views.len() < SDL_VULKAN_NUM_TEXTURE_BINDINGS {
            image_views.push(image_views[0]);
        }
        while samplers.len() < SDL_VULKAN_NUM_TEXTURE_BINDINGS {
            samplers.push(samplers[0]);
        }

        // (a YUV pipeline's layouts are Android's)
        let descriptor_set_layout = self.descriptor_set_layout;
        let pipeline_layout = self.pipeline_layout;
        self.set_draw_state(
            cmd,
            pipeline_layout,
            descriptor_set_layout,
            Some(&constants),
            VK_PRIMITIVE_TOPOLOGY_TRIANGLE_LIST,
            &image_views,
            &samplers,
            None,
            state_cache,
        )
    }

    /// Translation of `VULKAN_DrawPrimitives()`.
    fn draw_primitives(&self, vertex_start: usize, vertex_count: usize) {
        // SAFETY: the command buffer is recording in a render pass, with a
        // pipeline and the vertex buffer bound.
        unsafe {
            (self.dev().cmd_draw)(
                self.current_command_buffer,
                vertex_count as u32,
                1,
                vertex_start as u32,
                0,
            )
        };
    }

    /// Translation of `VULKAN_UpdateVertexBuffer()`.
    fn update_vertex_buffer(&mut self, state_cache: &mut DrawStateCache) -> Result<()> {
        let vbidx = self.current_vertex_buffer;
        let data_size_in_bytes = size_of_val(&self.verts[..]) as VkDeviceSize;

        if data_size_in_bytes == 0 {
            return Ok(()); // nothing to do.
        }

        if self.issue_batch && self.issue_batch() != VK_SUCCESS {
            return Err(Error::new("Failed to issue intermediate batch"));
        }
        // If the existing vertex buffer isn't big enough, we need to recreate a big enough one
        if data_size_in_bytes > self.vertex_buffers[vbidx].size {
            self.issue_batch();
            self.wait_for_gpu();
            let _ = self.create_vertex_buffer(vbidx, data_size_in_bytes);
        }

        let verts = std::mem::take(&mut self.verts);
        let vertex_buffer = &mut self.vertex_buffers[vbidx];
        let dst = vertex_buffer.mapped();
        for (d, v) in dst
            .chunks_exact_mut(size_of::<VertexPositionColor>())
            .zip(&verts)
        {
            let words = [
                v.pos[0], v.pos[1], v.tex[0], v.tex[1], v.color[0], v.color[1], v.color[2],
                v.color[3],
            ];
            for (b, w) in d.chunks_exact_mut(4).zip(words) {
                b.copy_from_slice(&w.to_ne_bytes());
            }
        }
        self.verts = verts;

        state_cache.vertex_buffer = self.vertex_buffers[vbidx].buffer;

        self.current_vertex_buffer = vbidx + 1;
        if self.current_vertex_buffer >= SDL_VULKAN_NUM_VERTEX_BUFFERS {
            self.current_vertex_buffer = 0;
            self.issue_batch = true;
        }

        Ok(())
    }

    /// Translation of `VULKAN_QueueDrawPoints()` (lines and points queue
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

    /// Translation of `VULKAN_CreateRenderer()` for a window.
    pub(crate) fn for_window(
        window: Window,
        output_colorspace: Colorspace,
    ) -> Result<VulkanRenderer> {
        // Clear any OpenGL properties on the window to avoid potential driver conflicts.
        let mut flags = window.flags()?;
        if flags.contains(WindowFlags::OPENGL) {
            flags &= !WindowFlags::OPENGL;
            let _ = window.reconfigure(flags);
        }

        // (SDL_SetupRendererColorspace())
        if output_colorspace != Colorspace::SRGB
            && output_colorspace != Colorspace::SRGB_LINEAR
            && output_colorspace != Colorspace::HDR10
        {
            return Err(Error::new("Unsupported output colorspace"));
        }

        let mut renderer_data = VulkanRenderer::new(window, output_colorspace);

        // (renderer->window is the window from the start)

        // Initialize Vulkan resources
        let result = renderer_data.create_device_resources().and_then(|()| {
            renderer_data
                .create_window_size_dependent_resources()
                .map_err(|(_, e)| e)
        });
        if let Err(e) = result {
            // (SDL_CreateRenderer() destroys the renderer that failed)
            renderer_data.destroy();
            return Err(e);
        }

        let inst = renderer_data.inst();
        let physical_device = renderer_data.physical_device;
        let texture_format_supported = |format: VkFormat| {
            let mut properties = VkImageFormatProperties::default();
            // SAFETY: the renderer's physical device; a valid output.
            unsafe {
                (inst.get_physical_device_image_format_properties)(
                    physical_device,
                    format,
                    VK_IMAGE_TYPE_2D,
                    VK_IMAGE_TILING_OPTIMAL,
                    VK_IMAGE_USAGE_SAMPLED_BIT | VK_IMAGE_USAGE_TRANSFER_DST_BIT,
                    0,
                    &mut properties,
                ) == VK_SUCCESS
            }
        };

        let mut formats = Vec::new();
        for &(sdl, unorm, srgb) in VK_FORMAT_MAP {
            if !texture_format_supported(unorm) {
                continue;
            }
            if !texture_format_supported(srgb) {
                continue;
            }
            formats.push(sdl);
        }

        if texture_format_supported(get_vk_image_format(PixelFormat::INDEX8, output_colorspace))
            && texture_format_supported(get_vk_image_format(PixelFormat::RGBA32, output_colorspace))
        {
            formats.push(PixelFormat::INDEX8);
        }

        const YUV_FORMATS: [PixelFormat; 8] = [
            PixelFormat::YV12,
            PixelFormat::IYUV,
            PixelFormat::I444,
            PixelFormat::NV12,
            PixelFormat::NV21,
            PixelFormat::P010,
            PixelFormat::I0FL,
            PixelFormat::I4FL,
        ];
        for format in YUV_FORMATS {
            let vk_format = get_vk_image_format(format, output_colorspace);
            if texture_format_supported(vk_format) {
                formats.push(format);
            }
        }
        renderer_data.texture_formats = formats;

        // (SDL_PIXELFORMAT_EXTERNAL_OES is Android's)

        Ok(renderer_data)
    }

    /// The renderer's data before the device exists (the `SDL_calloc()`
    /// and the setup of `VULKAN_CreateRenderer()`).
    fn new(window: Window, output_colorspace: Colorspace) -> VulkanRenderer {
        let mut data = VulkanRenderer {
            window,
            output_colorspace,
            current_colorspace: output_colorspace,
            hdr_headroom: 1.0,
            target: None,
            vk_get_instance_proc_addr: None,
            global: None,
            inst: None,
            dev: None,
            instance: null_mut(),
            surface: VK_NULL_HANDLE,
            physical_device: null_mut(),
            physical_device_properties: Box::default(),
            physical_device_memory_properties: Box::default(),
            physical_device_features: VkPhysicalDeviceFeatures::default(),
            graphics_queue: null_mut(),
            present_queue: null_mut(),
            device: null_mut(),
            graphics_queue_family_index: 0,
            present_queue_family_index: 0,
            swapchain: VK_NULL_HANDLE,
            command_pool: VK_NULL_HANDLE,
            command_buffers: Vec::new(),
            current_command_buffer_index: 0,
            current_command_buffer: null_mut(),
            fences: Vec::new(),
            surface_capabilities: VkSurfaceCapabilitiesKHR::default(),
            surface_formats: Vec::new(),
            recreate_swapchain: false,
            vsync: 0,
            framebuffers: Vec::new(),
            render_passes: [VK_NULL_HANDLE; VULKAN_RENDERPASS_COUNT],
            current_render_pass: VK_NULL_HANDLE,
            vertex_shader_modules: [VK_NULL_HANDLE; Shader::COUNT],
            fragment_shader_modules: [VK_NULL_HANDLE; Shader::COUNT],
            descriptor_set_layout: VK_NULL_HANDLE,
            pipeline_layout: VK_NULL_HANDLE,
            vertex_buffers: Vec::new(),
            vertex_shader_constants_data: VertexShaderConstants::default(),
            upload_buffers: Vec::new(),
            current_upload_buffer: Vec::new(),
            constant_buffers: Vec::new(),
            current_constant_buffer_index: 0,
            current_constant_buffer_offset: 0,
            samplers: [VK_NULL_HANDLE; RENDER_SAMPLER_COUNT],
            descriptor_pools: Vec::new(),
            current_descriptor_pool_index: 0,
            current_descriptor_set_index: 0,
            pipeline_states: Vec::new(),
            current_pipeline_state: None,
            supports_ext_swapchain_colorspace: false,
            supports_khr_get_physical_device_properties2: false,
            supports_khr_sampler_ycbcr_conversion: false,
            supports_khr_external_memory_capabilities: false,
            swapchain_desired_image_count: 0,
            surface_format: VkSurfaceFormatKHR::default(),
            swapchain_size: VkExtent2D::default(),
            swap_chain_pre_transform: 0,
            swapchain_image_count: 0,
            swapchain_images: Vec::new(),
            swapchain_image_views: Vec::new(),
            swapchain_image_layouts: Vec::new(),
            image_available_semaphores: Vec::new(),
            rendering_finished_semaphores: Vec::new(),
            current_image_available_semaphore: VK_NULL_HANDLE,
            current_swapchain_image_index: 0,
            wait_dest_stage_masks: Vec::new(),
            wait_render_semaphores: Vec::new(),
            signal_render_semaphores: Vec::new(),
            texture_render_target: None,
            cliprect_dirty: false,
            current_cliprect_enabled: false,
            current_cliprect: Rect::default(),
            current_viewport: Rect::default(),
            current_viewport_rotation: 0,
            viewport_dirty: false,
            identity: Float4X4::identity(),
            identity_swizzle: VkComponentMapping {
                r: VK_COMPONENT_SWIZZLE_IDENTITY,
                g: VK_COMPONENT_SWIZZLE_IDENTITY,
                b: VK_COMPONENT_SWIZZLE_IDENTITY,
                a: VK_COMPONENT_SWIZZLE_IDENTITY,
            },
            current_vertex_buffer: 0,
            issue_batch: false,
            textures: HashMap::new(),
            next_texture: 1,
            palettes: HashMap::new(),
            next_palette: 1,
            verts: Vec::new(),
            texture_formats: Vec::new(),
            props: None,
        };
        data.invalidate_cached_state();
        data
    }

    /// Translation of `VULKAN_RenderPresent()` (the error with it).
    fn render_present(&mut self) -> Result<()> {
        if self.device.is_null() {
            return Err(Error::new("Device lost and couldn't be recovered"));
        }

        if !self.current_command_buffer.is_null() {
            self.current_pipeline_state = None;
            self.viewport_dirty = true;

            let image_index = self.current_swapchain_image_index as usize;
            self.swapchain_image_layouts[image_index] = self.record_pipeline_image_barrier(
                VK_ACCESS_COLOR_ATTACHMENT_WRITE_BIT,
                VK_ACCESS_COLOR_ATTACHMENT_READ_BIT | VK_ACCESS_COLOR_ATTACHMENT_WRITE_BIT,
                VK_PIPELINE_STAGE_COLOR_ATTACHMENT_OUTPUT_BIT,
                VK_PIPELINE_STAGE_COLOR_ATTACHMENT_OUTPUT_BIT,
                VK_IMAGE_LAYOUT_PRESENT_SRC_KHR,
                self.swapchain_images[image_index],
                self.swapchain_image_layouts[image_index],
            );

            let dev = self.dev();
            let index = self.current_command_buffer_index as usize;
            // SAFETY: the command buffer is recording, outside a render pass.
            unsafe { (dev.end_command_buffer)(self.current_command_buffer) };

            // SAFETY: the frame's fence, signaled (waited for before).
            let result = unsafe { (dev.reset_fences)(self.device, 1, &self.fences[index]) };
            check("vkResetFences()", result)?;

            let (wait_semaphores, wait_dest_stage_masks) = self.submit_waits();
            let mut signal_semaphores = std::mem::take(&mut self.signal_render_semaphores);
            signal_semaphores.push(self.rendering_finished_semaphores[index]);
            let submit_info = VkSubmitInfo {
                s_type: VK_STRUCTURE_TYPE_SUBMIT_INFO,
                wait_semaphore_count: wait_semaphores.len() as u32,
                p_wait_semaphores: wait_semaphores.as_ptr(),
                p_wait_dst_stage_mask: wait_dest_stage_masks.as_ptr(),
                command_buffer_count: 1,
                p_command_buffers: &self.current_command_buffer,
                signal_semaphore_count: signal_semaphores.len() as u32,
                p_signal_semaphores: signal_semaphores.as_ptr(),
                ..Default::default()
            };
            // SAFETY: the command buffer is complete; the semaphores, masks
            // and fence outlive the call.
            let result = unsafe {
                (dev.queue_submit)(self.graphics_queue, 1, &submit_info, self.fences[index])
            };
            if result != VK_SUCCESS {
                if result == VK_ERROR_DEVICE_LOST {
                    if self.handle_device_lost() {
                        return Err(Error::new("Present failed, device lost"));
                    } else {
                        // Recovering from device lost failed, error is already set
                        return Err(Error::new("Device lost and couldn't be recovered"));
                    }
                }
                return Err(error_code("vkQueueSubmit()", result));
            }
            self.current_command_buffer = null_mut();
            self.current_image_available_semaphore = VK_NULL_HANDLE;

            let present_info = VkPresentInfoKHR {
                s_type: VK_STRUCTURE_TYPE_PRESENT_INFO_KHR,
                wait_semaphore_count: 1,
                p_wait_semaphores: &self.rendering_finished_semaphores[index],
                swapchain_count: 1,
                p_swapchains: &self.swapchain,
                p_image_indices: &self.current_swapchain_image_index,
                ..Default::default()
            };
            // SAFETY: the present info and what it points to outlive the call.
            let result = unsafe { (dev.queue_present_khr)(self.present_queue, &present_info) };
            if result != VK_SUCCESS
                && result != VK_ERROR_OUT_OF_DATE_KHR
                && result != VK_ERROR_SURFACE_LOST_KHR
                && result != VK_SUBOPTIMAL_KHR
            {
                return Err(error_code("vkQueuePresentKHR()", result));
            }

            self.current_command_buffer_index =
                (self.current_command_buffer_index + 1) % self.swapchain_image_count;

            // Wait for previous time this command buffer was submitted, will be N frames ago
            let index = self.current_command_buffer_index as usize;
            // SAFETY: the frame's fence.
            let result = unsafe {
                (dev.wait_for_fences)(self.device, 1, &self.fences[index], VK_TRUE, u64::MAX)
            };
            if result != VK_SUCCESS {
                if result == VK_ERROR_DEVICE_LOST {
                    if self.handle_device_lost() {
                        return Err(Error::new("Present failed, device lost"));
                    } else {
                        // Recovering from device lost failed, error is already set
                        return Err(Error::new("Device lost and couldn't be recovered"));
                    }
                }
                return Err(error_code("vkWaitForFences()", result));
            }

            self.acquire_next_swapchain_image();
        }

        Ok(())
    }

    /// Translation of `VULKAN_AddVulkanRenderSemaphores()`.
    fn add_render_semaphores(
        &mut self,
        wait_stage_mask: u32,
        wait_semaphore: i64,
        signal_semaphore: i64,
    ) {
        if wait_semaphore != 0 {
            self.wait_dest_stage_masks.push(wait_stage_mask);
            self.wait_render_semaphores
                .push(wait_semaphore as VkSemaphore);
        }

        if signal_semaphore != 0 {
            self.signal_render_semaphores
                .push(signal_semaphore as VkSemaphore);
        }
    }

    /// Translation of `VULKAN_ReadPixels()` (the error with it).
    fn render_read_pixels(&mut self, rect: &Rect) -> Result<Surface<'static>> {
        self.check_device()?;
        self.ensure_command_buffer();

        // Stop any outstanding renderpass if open
        self.end_render_pass();

        let target = self
            .texture_render_target
            .and_then(|id| self.textures.get(&id).map(|t| (id, t.images[0])));
        let (back_buffer, image_layout, vk_format) = match target {
            Some((_, image)) => (image.image, image.image_layout, image.format),
            None => {
                let index = self.current_swapchain_image_index as usize;
                (
                    self.swapchain_images[index],
                    self.swapchain_image_layouts[index],
                    self.surface_format.format,
                )
            }
        };

        let pixel_size = get_bytes_per_pixel(vk_format, 0);
        let length = rect.w as VkDeviceSize * pixel_size;
        let readback_buffer_size = length * rect.h as VkDeviceSize;
        let mut readback_buffer = self
            .allocate_buffer(
                readback_buffer_size,
                VK_BUFFER_USAGE_TRANSFER_DST_BIT,
                VK_MEMORY_PROPERTY_HOST_COHERENT_BIT | VK_MEMORY_PROPERTY_HOST_VISIBLE_BIT,
                VK_MEMORY_PROPERTY_DEVICE_LOCAL_BIT,
            )
            .map_err(|(_, e)| e)?;

        // Make sure the source is in the correct resource state
        let mut image_layout = self.record_pipeline_image_barrier(
            VK_ACCESS_SHADER_READ_BIT
                | VK_ACCESS_COLOR_ATTACHMENT_WRITE_BIT
                | VK_ACCESS_COLOR_ATTACHMENT_READ_BIT
                | VK_ACCESS_TRANSFER_READ_BIT
                | VK_ACCESS_TRANSFER_WRITE_BIT,
            VK_ACCESS_TRANSFER_READ_BIT,
            VK_PIPELINE_STAGE_TRANSFER_BIT
                | VK_PIPELINE_STAGE_COLOR_ATTACHMENT_OUTPUT_BIT
                | VK_PIPELINE_STAGE_FRAGMENT_SHADER_BIT,
            VK_PIPELINE_STAGE_TRANSFER_BIT,
            VK_IMAGE_LAYOUT_TRANSFER_SRC_OPTIMAL,
            back_buffer,
            image_layout,
        );

        // Copy the image to the readback buffer
        let region = VkBufferImageCopy {
            buffer_offset: 0,
            buffer_row_length: 0,
            buffer_image_height: 0,
            image_subresource: VkImageSubresourceLayers {
                base_array_layer: 0,
                layer_count: 1,
                mip_level: 0,
                aspect_mask: VK_IMAGE_ASPECT_COLOR_BIT,
            },
            image_offset: VkOffset3D {
                x: rect.x,
                y: rect.y,
                z: 0,
            },
            image_extent: VkExtent3D {
                width: rect.w as u32,
                height: rect.h as u32,
                depth: 1,
            },
        };
        // SAFETY: the command buffer is recording, outside a render pass;
        // the buffer has room for the region.
        unsafe {
            (self.dev().cmd_copy_image_to_buffer)(
                self.current_command_buffer,
                back_buffer,
                image_layout,
                readback_buffer.buffer,
                1,
                &region,
            )
        };

        // We need to issue the command list for the copy to finish
        self.issue_batch();

        // Transition the render target back to a render target
        image_layout = self.record_pipeline_image_barrier(
            VK_ACCESS_TRANSFER_WRITE_BIT,
            VK_ACCESS_SHADER_READ_BIT
                | VK_ACCESS_COLOR_ATTACHMENT_WRITE_BIT
                | VK_ACCESS_COLOR_ATTACHMENT_READ_BIT
                | VK_ACCESS_TRANSFER_READ_BIT
                | VK_ACCESS_TRANSFER_WRITE_BIT,
            VK_PIPELINE_STAGE_TRANSFER_BIT,
            VK_PIPELINE_STAGE_TRANSFER_BIT
                | VK_PIPELINE_STAGE_COLOR_ATTACHMENT_OUTPUT_BIT
                | VK_PIPELINE_STAGE_FRAGMENT_SHADER_BIT,
            VK_IMAGE_LAYOUT_COLOR_ATTACHMENT_OPTIMAL,
            back_buffer,
            image_layout,
        );
        match target {
            Some((id, _)) => {
                if let Some(t) = self.textures.get_mut(&id) {
                    t.images[0].image_layout = image_layout;
                }
            }
            None => {
                self.swapchain_image_layouts[self.current_swapchain_image_index as usize] =
                    image_layout;
            }
        }

        let output = crate::video::surface::duplicate_pixels(
            rect.w,
            rect.h,
            vk_format_to_sdl_pixel_format(vk_format),
            self.current_colorspace,
            Some(readback_buffer.mapped()),
            length as i32,
        );

        self.destroy_buffer(&mut readback_buffer);

        output
    }
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

impl RenderBackend for VulkanRenderer {
    fn name(&self) -> &'static str {
        VULKAN_RENDERER
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
        self.props = Some(props.clone());
        self.publish_device_properties();
        self.publish_swapchain_properties();
    }

    /// Translation of `VULKAN_SupportsBlendMode()`.
    fn supports_blend_mode(&self, blend_mode: BlendMode) -> bool {
        let src_color_factor = blend_mode.src_color_factor();
        let src_alpha_factor = blend_mode.src_alpha_factor();
        let color_operation = blend_mode.color_operation();
        let dst_color_factor = blend_mode.dst_color_factor();
        let dst_alpha_factor = blend_mode.dst_alpha_factor();
        let alpha_operation = blend_mode.alpha_operation();

        !(get_blend_factor(src_color_factor) == VK_BLEND_FACTOR_MAX_ENUM
            || get_blend_factor(src_alpha_factor) == VK_BLEND_FACTOR_MAX_ENUM
            || get_blend_op(color_operation) == VK_BLEND_OP_MAX_ENUM
            || get_blend_factor(dst_color_factor) == VK_BLEND_FACTOR_MAX_ENUM
            || get_blend_factor(dst_alpha_factor) == VK_BLEND_FACTOR_MAX_ENUM
            || get_blend_op(alpha_operation) == VK_BLEND_OP_MAX_ENUM)
    }

    /// Translation of `VULKAN_CreateTexture()`.
    fn create_texture(
        &mut self,
        texture: &mut TextureData,
        _props: &TextureCreateProps,
    ) -> Result<()> {
        use PixelFormat as F;
        let num_images = get_format_image_count(texture.format);
        let num_image_views = get_format_image_view_count(texture.format);
        let texture_format = get_vk_image_format(texture.format, self.output_colorspace);
        let mut width = texture.w as u32;
        let mut height = texture.h as u32;
        let mut chroma_width = width;
        let mut chroma_height = height;
        let image_view_swizzle = self.identity_swizzle;

        if self.device.is_null() {
            return Err(Error::new("Device lost and couldn't be recovered"));
        }

        let mut texture_data = VulkanTexture::default();

        if texture.format == F::EXTERNAL_OES {
            // (Android hardware buffers)
            return Err(Error::new(
                "SDL_PIXELFORMAT_EXTERNAL_OES texture requires a hardware buffer",
            ));
        } else if texture.format.is_fourcc() {
            if matches!(
                texture.format,
                F::YV12 | F::IYUV | F::NV12 | F::NV21 | F::P010 | F::I0FL
            ) {
                // Pad width/height to multiple of 2
                width = (width + 1) & !1;
                height = (height + 1) & !1;
            }
            if matches!(texture.format, F::YV12 | F::IYUV | F::I0FL) {
                // YUV 4:2:0 formats
                chroma_width = width / 2;
                chroma_height = height / 2;
            }

            let bits_per_pixel = match texture.format {
                F::I0FL | F::I4FL => 16,
                F::P010 => 10,
                _ => 8,
            };
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
            texture_data.ycbcr_matrix = Some(ycbcr);
        }
        texture_data.width = width as i32;
        texture_data.height = height as i32;

        let mut usage = VK_IMAGE_USAGE_SAMPLED_BIT | VK_IMAGE_USAGE_TRANSFER_DST_BIT;
        if texture.access == TextureAccess::Target {
            usage |= VK_IMAGE_USAGE_COLOR_ATTACHMENT_BIT | VK_IMAGE_USAGE_TRANSFER_SRC_BIT;
        }
        // (SDL_PROP_TEXTURE_CREATE_VULKAN_USAGE_NUMBER isn't taken)

        // The texture's data goes in the renderer now, so that a failure
        // below releases what was made (upstream leaves that to
        // SDL_DestroyTexture()).
        let id = self.next_texture;
        self.next_texture += 1;
        self.textures.insert(id, texture_data);
        let fail = |this: &mut VulkanRenderer, e: Error| {
            this.destroy_texture_data(id);
            Err(e)
        };

        for i in 0..num_images {
            let image_width = if i == 0 { width } else { chroma_width };
            let image_height = if i == 0 { height } else { chroma_height };
            match self.allocate_image(image_width, image_height, texture_format, usage) {
                Ok(image) => {
                    let t = self.textures.get_mut(&id).expect("the new texture");
                    t.images[i] = image;
                    t.num_images += 1;
                }
                Err((rc, _)) => {
                    let e = error_code("VULKAN_AllocateImage()", rc);
                    return fail(self, e);
                }
            }
        }

        let use_plane_views = num_image_views > num_images;
        let mut image_index = 0;
        for i in 0..num_image_views {
            let image_view_format =
                get_vk_image_view_format(texture.format, i, self.output_colorspace);
            let image_plane = if use_plane_views { Some(i) } else { None };
            let image = self.textures[&id].images[image_index].image;
            match self.allocate_image_view(
                image,
                image_plane,
                image_view_format,
                usage,
                image_view_swizzle,
            ) {
                Ok(view) => {
                    let t = self.textures.get_mut(&id).expect("the new texture");
                    t.image_views[i] = view;
                    t.num_image_views += 1;
                }
                Err((rc, _)) => {
                    let e = error_code("VULKAN_AllocateImageView()", rc);
                    return fail(self, e);
                }
            }

            if image_index < num_images - 1 {
                image_index += 1;
            }
        }

        if texture.access == TextureAccess::Target {
            let mut framebuffers = Vec::new();
            let mut render_passes = [VK_NULL_HANDLE; VULKAN_RENDERPASS_COUNT];
            let t = &self.textures[&id];
            let views = t.image_views[..t.num_image_views].to_vec();
            let result = self.create_framebuffers_and_render_passes(
                texture.w,
                texture.h,
                texture_format,
                &views,
                &mut framebuffers,
                &mut render_passes,
            );
            let t = self.textures.get_mut(&id).expect("the new texture");
            t.main_renderpasses = render_passes;
            // (a target has one view: the front end makes no FOURCC targets)
            t.main_framebuffer = framebuffers.first().copied().unwrap_or(VK_NULL_HANDLE);
            if let Err((rc, _)) = result {
                let e = error_code("VULKAN_CreateFramebuffersAndRenderPasses()", rc);
                return fail(self, e);
            }
        }

        const IMAGE_PROPERTIES: [&str; SDL_VULKAN_NUM_TEXTURE_BINDINGS] = [
            PROP_TEXTURE_VULKAN_TEXTURE_NUMBER,
            PROP_TEXTURE_VULKAN_TEXTURE_U_NUMBER,
            PROP_TEXTURE_VULKAN_TEXTURE_V_NUMBER,
        ];

        let props = texture.props.get_or_insert_with(Properties::new).clone();
        for (property, image) in IMAGE_PROPERTIES
            .iter()
            .zip(&self.textures[&id].images[..num_images])
        {
            let _ = props.set(property, image.image as i64);
        }

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

    /// Translation of `VULKAN_QueueDrawPoints()`.
    fn queue_draw_points(&mut self, cmd: &mut DrawCmd, points: &[FPoint]) -> Result<()> {
        self.queue_points(cmd, points);
        Ok(())
    }

    /// `VULKAN_QueueDrawPoints()`: lines and points queue vertices the same way.
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

    /// Translation of `VULKAN_QueueGeometry()`.
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
            (Some(t), Some(d)) => (t.w as f32 / d.width as f32, t.h as f32 / d.height as f32),
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

    /// Translation of `VULKAN_InvalidateCachedState()`.
    fn invalidate_cached_state(&mut self) {
        self.current_pipeline_state = None;
        self.cliprect_dirty = true;

        // Make sure pending drawing is submitted to the GPU
        self.issue_batch();
    }

    /// Translation of `VULKAN_RunCommandQueue()`.
    fn run_command_queue(
        &mut self,
        cmds: &[RenderCommand],
        textures: &mut TextureStore,
        _gpu_render_states: &crate::render::sysrender::GpuRenderStates,
    ) -> Result<()> {
        let current_rotation = self.get_rotation_for_current_render_target();
        let mut state_cache = DrawStateCache::default();

        if self.device.is_null() {
            return Err(Error::new("Device lost and couldn't be recovered"));
        }

        if self.current_viewport_rotation != current_rotation {
            self.current_viewport_rotation = current_rotation;
            self.viewport_dirty = true;
            self.cliprect_dirty = true;
        }

        if self.recreate_swapchain {
            self.update_for_window_size_change().map_err(|(_, e)| e)?;
            self.recreate_swapchain = false;
        }

        self.update_vertex_buffer(&mut state_cache)?;

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

                    let clear_color = [color.r, color.g, color.b, color.a];
                    self.activate_command_buffer(
                        VK_ATTACHMENT_LOAD_OP_CLEAR,
                        Some(clear_color),
                        &state_cache,
                    );
                }

                RenderCommand::Draw(DrawKind::Lines, d) if d.count > 0 => {
                    let mut count = d.count;
                    let start = d.first;
                    let verts = &self.verts[start..];
                    let mut have_point_draw_state = false;

                    // Add the final point in the line
                    let mut line_start = 0;
                    let mut line_end = line_start + count - 1;
                    if verts[line_start].pos != verts[line_end].pos
                        && self
                            .set_solid_draw_state(
                                &d,
                                VK_PRIMITIVE_TOPOLOGY_POINT_LIST,
                                &state_cache,
                            )
                            .is_ok()
                    {
                        self.draw_primitives(start + line_end, 1);
                        have_point_draw_state = true;
                    }

                    if count > 2 {
                        // joined lines cannot be grouped
                        if self
                            .set_solid_draw_state(
                                &d,
                                VK_PRIMITIVE_TOPOLOGY_LINE_STRIP,
                                &state_cache,
                            )
                            .is_ok()
                        {
                            self.draw_primitives(start, count);
                        }
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
                                                have_point_draw_state = self
                                                    .set_solid_draw_state(
                                                        &d,
                                                        VK_PRIMITIVE_TOPOLOGY_POINT_LIST,
                                                        &state_cache,
                                                    )
                                                    .is_ok();
                                            }
                                            if have_point_draw_state {
                                                self.draw_primitives(start + line_end, 1);
                                            }
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

                        if self
                            .set_solid_draw_state(&d, VK_PRIMITIVE_TOPOLOGY_LINE_LIST, &state_cache)
                            .is_ok()
                        {
                            self.draw_primitives(start, count);
                        }

                        i = finalcmd; // skip any copy commands we just combined in here.
                    }
                }

                RenderCommand::Draw(DrawKind::Lines, _) => {
                    // (no vertices: upstream reads the one before its first)
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
                        let set_state = if d.texture.is_some() {
                            self.set_copy_state(&d, textures, &state_cache)
                        } else {
                            self.set_solid_draw_state(
                                &d,
                                VK_PRIMITIVE_TOPOLOGY_TRIANGLE_LIST,
                                &state_cache,
                            )
                        };

                        if set_state.is_ok() {
                            self.draw_primitives(start, count);
                        }
                    } else if self
                        .set_solid_draw_state(&d, VK_PRIMITIVE_TOPOLOGY_POINT_LIST, &state_cache)
                        .is_ok()
                    {
                        self.draw_primitives(start, count);
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

    /// Translation of `VULKAN_CreatePalette()`.
    fn create_palette(&mut self) -> Result<Box<dyn Any>> {
        self.check_device()?;
        let format = get_vk_image_format(PixelFormat::RGBA32, self.output_colorspace);
        let usage = VK_IMAGE_USAGE_SAMPLED_BIT | VK_IMAGE_USAGE_TRANSFER_DST_BIT;
        let image_view_swizzle = self.identity_swizzle;
        let mut palettedata = PaletteData {
            image: self
                .allocate_image(256, 1, format, usage)
                .map_err(|(rc, _)| error_code("VULKAN_AllocateImage()", rc))?,
            image_view: VK_NULL_HANDLE,
        };
        match self.allocate_image_view(
            palettedata.image.image,
            None,
            format,
            usage,
            image_view_swizzle,
        ) {
            Ok(view) => palettedata.image_view = view,
            Err((rc, _)) => {
                // (upstream leaves the image to VULKAN_DestroyPalette())
                self.issue_batch();
                self.wait_for_gpu();
                self.destroy_image(&mut palettedata.image);
                return Err(error_code("VULKAN_AllocateImage()", rc));
            }
        }
        let id = self.next_palette;
        self.next_palette += 1;
        self.palettes.insert(id, palettedata);
        Ok(Box::new(PaletteRef(id)))
    }

    /// Translation of `VULKAN_UpdatePalette()`.
    fn update_palette(&mut self, palette: &mut dyn Any, colors: &[Color]) -> Result<()> {
        let id = palette
            .downcast_ref::<PaletteRef>()
            .map(|p| p.0)
            .ok_or_else(|| Error::invalid_param("palette"))?;
        let image = self
            .palettes
            .get(&id)
            .map(|p| p.image)
            .ok_or_else(|| Error::invalid_param("palette"))?;

        let bytes: Vec<u8> = colors.iter().flat_map(|c| [c.r, c.g, c.b, c.a]).collect();
        let layout = self.update_texture_internal(
            image.image,
            image.format,
            0,
            (0, 0, colors.len() as i32, 1),
            &bytes,
            bytes.len(),
            image.image_layout,
        )?;
        if let Some(p) = self.palettes.get_mut(&id) {
            p.image.image_layout = layout;
        }
        Ok(())
    }

    /// Translation of `VULKAN_DestroyPalette()`.
    fn destroy_palette(&mut self, palette: Box<dyn Any>) {
        let Ok(palette) = palette.downcast::<PaletteRef>() else {
            return;
        };
        let Some(mut palettedata) = self.palettes.remove(&palette.0) else {
            return;
        };
        if self.dev.is_none() {
            return;
        }

        /* Because VULKAN_DestroyPalette might be called while the data is in-flight, we need to issue the batch first
        Unfortunately, this means that deleting a lot of palettes mid-frame will have poor performance. */
        self.issue_batch();
        self.wait_for_gpu();

        self.destroy_image(&mut palettedata.image);

        if palettedata.image_view != VK_NULL_HANDLE {
            // SAFETY: the view is this device's, and idle.
            unsafe { (self.dev().destroy_image_view)(self.device, palettedata.image_view, null()) };
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
        self.update_texture_pixels(texture, rect, pixels, pitch)
    }

    fn update_texture_yuv(
        &mut self,
        texture: &mut TextureData,
        rect: &Rect,
        y: (&[u8], usize),
        u: (&[u8], usize),
        v: (&[u8], usize),
    ) -> Option<Result<()>> {
        Some(self.update_texture_yuv_planes(texture, rect, y, u, v))
    }

    fn update_texture_nv(
        &mut self,
        texture: &mut TextureData,
        rect: &Rect,
        y: (&[u8], usize),
        uv: (&[u8], usize),
    ) -> Option<Result<()>> {
        Some(self.update_texture_nv_planes(texture, rect, y, uv))
    }

    /// Translation of `VULKAN_LockTexture()`.
    fn lock_texture(&mut self, texture: &mut TextureData, rect: &Rect) -> Result<(usize, i32)> {
        let id = Self::texture_id(texture)?;
        let texture_data = self.textures.get(&id).ok_or_else(not_available)?;

        if texture_data.staging_buffer.buffer != VK_NULL_HANDLE {
            return Err(Error::new("texture is already locked"));
        }

        if texture.format == PixelFormat::EXTERNAL_OES {
            return Err(Error::unsupported());
        }

        let bpp = texture.format.bytes_per_pixel() as usize;
        if texture_data.num_image_views > 1 {
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

        let pixel_size = get_bytes_per_pixel(texture_data.images[0].format, 0);
        let length = rect.w as VkDeviceSize * pixel_size;
        let staging_buffer_size = length * rect.h as VkDeviceSize;
        let mut staging_buffer = self
            .allocate_buffer(
                staging_buffer_size,
                VK_BUFFER_USAGE_TRANSFER_SRC_BIT,
                VK_MEMORY_PROPERTY_HOST_COHERENT_BIT | VK_MEMORY_PROPERTY_HOST_VISIBLE_BIT,
                VK_MEMORY_PROPERTY_DEVICE_LOCAL_BIT,
            )
            .map_err(|(_, e)| e)?;

        /* Make note of where the staging texture will be written to
         * (on a call to SDL_UnlockTexture):
         */
        let mapped = staging_buffer.mapped();
        let staging = NonNull::new(mapped.as_mut_ptr()).map(|p| (p, mapped.len()));
        let texture_data = self.textures.get_mut(&id).ok_or_else(not_available)?;
        texture_data.locked_rect = *rect;
        texture_data.staging_buffer = staging_buffer;

        /* Make sure the caller has information on the texture's pixel buffer,
         * then return:
         */
        if let Some(tref) = texture_ref_mut(texture) {
            tref.staging = staging;
        }
        Ok((0, length as i32))
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

    /// Translation of `VULKAN_UnlockTexture()`.
    fn unlock_texture(&mut self, texture: &mut TextureData) {
        let Ok(id) = Self::texture_id(texture) else {
            return;
        };
        let Some(texture_data) = self.textures.get(&id) else {
            return;
        };

        if texture.format == PixelFormat::EXTERNAL_OES {
            return;
        }

        if texture_data.num_image_views > 1 {
            let rect = texture_data.locked_rect;
            let pitch = texture_data.pitch as usize;
            let offset = rect.y as usize * pitch
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

        self.ensure_command_buffer();

        let texture_data = &self.textures[&id];
        let image = texture_data.images[0];
        let locked_rect = texture_data.locked_rect;
        let staging = texture_data.staging_buffer.buffer;

        // Make sure the destination is in the correct resource state
        let mut layout = self.record_pipeline_image_barrier(
            VK_ACCESS_SHADER_READ_BIT
                | VK_ACCESS_COLOR_ATTACHMENT_WRITE_BIT
                | VK_ACCESS_COLOR_ATTACHMENT_READ_BIT
                | VK_ACCESS_TRANSFER_READ_BIT
                | VK_ACCESS_TRANSFER_WRITE_BIT,
            VK_ACCESS_TRANSFER_WRITE_BIT,
            VK_PIPELINE_STAGE_TRANSFER_BIT
                | VK_PIPELINE_STAGE_COLOR_ATTACHMENT_OUTPUT_BIT
                | VK_PIPELINE_STAGE_FRAGMENT_SHADER_BIT,
            VK_PIPELINE_STAGE_TRANSFER_BIT,
            VK_IMAGE_LAYOUT_TRANSFER_DST_OPTIMAL,
            image.image,
            image.image_layout,
        );

        let region = VkBufferImageCopy {
            buffer_offset: 0,
            buffer_row_length: 0,
            buffer_image_height: 0,
            image_subresource: VkImageSubresourceLayers {
                base_array_layer: 0,
                layer_count: 1,
                mip_level: 0,
                aspect_mask: VK_IMAGE_ASPECT_COLOR_BIT,
            },
            image_offset: VkOffset3D {
                x: locked_rect.x,
                y: locked_rect.y,
                z: 0,
            },
            image_extent: VkExtent3D {
                width: locked_rect.w as u32,
                height: locked_rect.h as u32,
                depth: 1,
            },
        };
        // SAFETY: the command buffer is recording, outside a render pass;
        // the staging buffer holds the locked rectangle's pixels.
        unsafe {
            (self.dev().cmd_copy_buffer_to_image)(
                self.current_command_buffer,
                staging,
                image.image,
                layout,
                1,
                &region,
            )
        };

        // Transition the texture to be shader accessible
        layout = self.record_pipeline_image_barrier(
            VK_ACCESS_TRANSFER_WRITE_BIT,
            VK_ACCESS_SHADER_READ_BIT,
            VK_PIPELINE_STAGE_TRANSFER_BIT,
            VK_PIPELINE_STAGE_FRAGMENT_SHADER_BIT,
            VK_IMAGE_LAYOUT_SHADER_READ_ONLY_OPTIMAL,
            image.image,
            layout,
        );
        if let Some(t) = self.textures.get_mut(&id) {
            t.images[0].image_layout = layout;
        }

        // Execute the command list before releasing the staging buffer
        self.issue_batch();

        if let Some(t) = self.textures.get_mut(&id) {
            let mut staging_buffer = std::mem::take(&mut t.staging_buffer);
            self.destroy_buffer(&mut staging_buffer);
        }
    }

    /// Translation of `VULKAN_SetRenderTarget()`.
    fn set_render_target(
        &mut self,
        target: Option<Texture>,
        textures: &TextureStore,
    ) -> Result<()> {
        self.check_device()?;
        self.ensure_command_buffer();

        match target {
            Some(t) => {
                let texture = textures.get(t).ok_or_else(invalid_texture)?;
                let id = Self::texture_id(texture)?;
                let texture_data = self.textures.get(&id).ok_or_else(not_available)?;

                if texture_data.image_views[0] == VK_NULL_HANDLE {
                    return Err(Error::new("specified texture is not a render target"));
                }

                self.texture_render_target = Some(id);
                self.target = Some(t);
                self.current_colorspace = texture.colorspace;
                let image = texture_data.images[0];
                let all_access = VK_ACCESS_TRANSFER_WRITE_BIT
                    | VK_ACCESS_SHADER_READ_BIT
                    | VK_ACCESS_COLOR_ATTACHMENT_READ_BIT
                    | VK_ACCESS_COLOR_ATTACHMENT_WRITE_BIT;
                let all_stages = VK_PIPELINE_STAGE_TRANSFER_BIT
                    | VK_PIPELINE_STAGE_FRAGMENT_SHADER_BIT
                    | VK_PIPELINE_STAGE_COLOR_ATTACHMENT_OUTPUT_BIT;
                let layout = self.record_pipeline_image_barrier(
                    all_access,
                    all_access,
                    all_stages,
                    all_stages,
                    VK_IMAGE_LAYOUT_COLOR_ATTACHMENT_OPTIMAL,
                    image.image,
                    image.image_layout,
                );
                if let Some(t) = self.textures.get_mut(&id) {
                    t.images[0].image_layout = layout;
                }
            }
            None => {
                if let Some(id) = self.texture_render_target {
                    if let Some(image) = self.textures.get(&id).map(|t| t.images[0]) {
                        let layout = self.record_pipeline_image_barrier(
                            VK_ACCESS_TRANSFER_WRITE_BIT
                                | VK_ACCESS_SHADER_READ_BIT
                                | VK_ACCESS_COLOR_ATTACHMENT_READ_BIT
                                | VK_ACCESS_COLOR_ATTACHMENT_WRITE_BIT,
                            VK_ACCESS_SHADER_READ_BIT,
                            VK_PIPELINE_STAGE_TRANSFER_BIT
                                | VK_PIPELINE_STAGE_FRAGMENT_SHADER_BIT
                                | VK_PIPELINE_STAGE_COLOR_ATTACHMENT_OUTPUT_BIT,
                            VK_PIPELINE_STAGE_FRAGMENT_SHADER_BIT,
                            VK_IMAGE_LAYOUT_SHADER_READ_ONLY_OPTIMAL,
                            image.image,
                            image.image_layout,
                        );
                        if let Some(t) = self.textures.get_mut(&id) {
                            t.images[0].image_layout = layout;
                        }
                    }
                }
                self.texture_render_target = None;
                self.target = None;
                self.current_colorspace = self.output_colorspace;
            }
        }

        self.issue_batch();

        Ok(())
    }

    /// Translation of `VULKAN_RenderReadPixels()`.
    fn read_pixels(
        &mut self,
        rect: &Rect,
        _textures: &mut TextureStore,
    ) -> Option<Result<Surface<'static>>> {
        Some(self.render_read_pixels(rect))
    }

    /// Translation of `VULKAN_RenderPresent()`.
    fn present(&mut self) -> bool {
        // (SDL_RenderPresent() goes on without the error)
        self.render_present().is_ok()
    }

    /// Translation of `VULKAN_DestroyTexture()`.
    fn destroy_texture(&mut self, texture: &mut TextureData) {
        let Some(internal) = texture.internal.take() else {
            return;
        };
        let Ok(tref) = internal.downcast::<TextureRef>() else {
            return;
        };
        self.destroy_texture_data(tref.id);
    }

    /// Translation of `VULKAN_SetVSync()`.
    fn set_vsync(&mut self, vsync: i32) -> Option<Result<()>> {
        match vsync {
            -1..=1 => {
                // Supported
            }
            _ => return Some(Err(Error::unsupported())),
        }
        if vsync != self.vsync {
            self.vsync = vsync;
            self.recreate_swapchain = true;
        }
        Some(Ok(()))
    }

    /// Translation of `VULKAN_AddVulkanRenderSemaphores()`.
    fn add_vulkan_render_semaphores(
        &mut self,
        wait_stage_mask: u32,
        wait_semaphore: i64,
        signal_semaphore: i64,
    ) -> Option<Result<()>> {
        self.add_render_semaphores(wait_stage_mask, wait_semaphore, signal_semaphore);
        Some(Ok(()))
    }

    /// Translation of `VULKAN_WindowEvent()` (the Android parts aside).
    fn window_event(&mut self, event_type: EventType) {
        if event_type == EventType::WINDOW_PIXEL_SIZE_CHANGED {
            self.recreate_swapchain = true;
        }
    }

    /// Translation of `VULKAN_DestroyRenderer()`.
    fn destroy(&mut self) {
        if !self.device.is_null() {
            if let Some(dev) = &self.dev {
                // SAFETY: the renderer's device.
                unsafe { (dev.device_wait_idle)(self.device) };
            }
            self.destroy_all();
        } else {
            // (the instance and surface of a creation that failed before
            // the device)
            self.destroy_all();
        }
    }
}
