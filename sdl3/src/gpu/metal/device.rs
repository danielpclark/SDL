// Rust translation of the device creation, blit resource and format query
// parts of src/gpu/metal/SDL_gpu_metal.m from Simple DirectMedia Layer.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see
// LICENSE.txt.

//! `PrepareDriver`, device creation (with the low-power device choice, the
//! hardware requirements and `MTL_DEBUG_LAYER` in debug mode), the
//! device's destruction, the blit shaders and samplers, and
//! `SupportsTextureFormat`.

use std::sync::atomic::AtomicU32;
use std::sync::Mutex;

use super::commands::AvailableCommandBuffers;
use super::shaders::*;
use super::sys::*;
use super::{lock, BlitResources, MetalRenderer};
#[cfg(target_os = "macos")]
use crate::core::apple::foundation::ns_array_items;
use crate::core::apple::foundation::{available, string_from_ns};
use crate::core::apple::objc::{msg_send, Bool, Id, StrongPtr};
use crate::error::{Error, Result};
use crate::gpu::sysgpu::{BackendObject, BlitPipelineCache, UNIFORM_BUFFER_SIZE};
use crate::gpu::{
    CompareOp, Filter, Sampler, SamplerAddressMode, SamplerCreateInfo, SamplerMipmapMode, Shader,
    ShaderCreateInfo, ShaderFormat, ShaderStage, TextureFormat, TextureType, TextureUsageFlags,
    PROP_GPU_DEVICE_CREATE_SHADERS_METALLIB_BOOLEAN, PROP_GPU_DEVICE_CREATE_SHADERS_MSL_BOOLEAN,
    PROP_GPU_DEVICE_CREATE_VERBOSE_BOOLEAN, PROP_GPU_DEVICE_CREATE_XR_ENABLE_BOOLEAN,
    PROP_GPU_DEVICE_NAME_STRING,
};
use crate::log::Category;
use crate::properties::Properties;
use crate::video::sysvideo::VideoDriver;

/// Translation of `METAL_PrepareDriver()`.
pub(super) fn prepare_driver(video: &dyn VideoDriver, props: &Properties) -> bool {
    if props
        .get_bool(PROP_GPU_DEVICE_CREATE_XR_ENABLE_BOOLEAN)
        .unwrap_or(false)
    {
        return false;
    }

    if !props
        .get_bool(PROP_GPU_DEVICE_CREATE_SHADERS_MSL_BOOLEAN)
        .unwrap_or(false)
        && !props
            .get_bool(PROP_GPU_DEVICE_CREATE_SHADERS_METALLIB_BOOLEAN)
            .unwrap_or(false)
    {
        return false;
    }

    if available!(macos(10, 14), ios(13, 0), tvos(13, 0)) {
        return video.implements_metal_views();
    }
    false
}

/// `[device supportsFamily:family]`.
///
/// # Safety
///
/// `device` must be a Metal device.
pub(super) unsafe fn supports_family(device: Id, family: isize) -> bool {
    // SAFETY: the caller's contract.
    let r: Bool = unsafe { msg_send![device, supportsFamily: family] };
    r.is_true()
}

/// Translation of `METAL_CreateDevice()` (the renderer; the bootstrap
/// wraps it as the device's driver).
pub(super) fn create_device(
    debug_mode: bool,
    prefer_low_power: bool,
    props: &Properties,
) -> Result<MetalRenderer> {
    let verbose_logs = props
        .get_bool(PROP_GPU_DEVICE_CREATE_VERBOSE_BOOLEAN)
        .unwrap_or(true);

    if debug_mode {
        /* Due to a Metal driver quirk, once a MTLDevice has been created
         * with this environment variable set, the Metal validation layers
         * will remain enabled for the rest of the application's lifespan,
         * even if the device is destroyed and recreated.
         */
        let _ = crate::stdlib::setenv_unsafe("MTL_DEBUG_LAYER", "1", false);
    }

    // Create the Metal device and command queue
    let mut device = StrongPtr::nil();
    #[cfg(target_os = "macos")]
    if prefer_low_power {
        // SAFETY: a +1 array of devices, and their isLowPower property.
        unsafe {
            let devices = StrongPtr::from_owned(MTLCopyAllDevices());
            for candidate in ns_array_items(devices.get()) {
                let is_low_power: Bool = msg_send![candidate, isLowPower];
                if is_low_power.is_true() {
                    device = StrongPtr::retain(candidate);
                    break;
                }
            }
        }
    }
    #[cfg(not(target_os = "macos"))]
    let _ = prefer_low_power;
    if device.is_nil() {
        // SAFETY: returns a +1 device, or nil.
        device = unsafe { StrongPtr::from_owned(MTLCreateSystemDefaultDevice()) };
        if device.is_nil() {
            return Err(Error::new("Failed to create Metal device"));
        }
    }

    #[cfg(target_os = "macos")]
    let has_hardware_support = {
        let mut has_hardware_support = true;
        let allow_mac_family1 = props
            .get_bool(crate::gpu::PROP_GPU_DEVICE_CREATE_METAL_ALLOW_MACFAMILY1_BOOLEAN)
            .unwrap_or(false);
        // SAFETY: the device's feature queries.
        unsafe {
            if available!(macos(10, 15)) {
                has_hardware_support = if allow_mac_family1 {
                    supports_family(device.get(), MTL_GPU_FAMILY_MAC1)
                } else {
                    supports_family(device.get(), MTL_GPU_FAMILY_MAC2)
                };
            } else if available!(macos(10, 14)) {
                let feature_set = if allow_mac_family1 {
                    MTL_FEATURE_SET_MACOS_GPU_FAMILY1_V4
                } else {
                    MTL_FEATURE_SET_MACOS_GPU_FAMILY2_V1
                };
                let r: Bool = msg_send![device, supportsFeatureSet: feature_set];
                has_hardware_support = r.is_true();
            }
        }
        has_hardware_support
    };
    #[cfg(target_os = "visionos")]
    let has_hardware_support = true;
    // (the other UIKit platforms)
    #[cfg(not(any(target_os = "macos", target_os = "visionos")))]
    // SAFETY: the device's feature query.
    let has_hardware_support = available!(ios(13, 0), tvos(13, 0))
        && unsafe { supports_family(device.get(), MTL_GPU_FAMILY_APPLE3) };

    if !has_hardware_support {
        return Err(Error::new(
            "Device does not meet the hardware requirements for SDL_GPU Metal",
        ));
    }

    // SAFETY: a +1 command queue of the device.
    let queue = unsafe { StrongPtr::from_owned(msg_send![device, newCommandQueue]) };

    let renderer_props = Properties::new();
    if verbose_logs {
        crate::log::info!(Category::Gpu, "SDL_GPU Driver: Metal");
    }

    // Record device name
    // SAFETY: the device's name (an NSString).
    let device_name = unsafe {
        let name: Id = msg_send![device, name];
        string_from_ns(name).unwrap_or_default()
    };
    let _ = renderer_props.set(PROP_GPU_DEVICE_NAME_STRING, device_name.as_str());
    if verbose_logs {
        crate::log::info!(Category::Gpu, "Metal Device: {}", device_name);
    }

    // Create command buffer pool
    let mut available_command_buffers = AvailableCommandBuffers::default();
    MetalRenderer::allocate_command_buffers(&mut available_command_buffers, 2);

    let renderer = MetalRenderer {
        device,
        queue,
        // Remember debug mode
        debug_mode,
        props: renderer_props,
        allowed_frames_in_flight: AtomicU32::new(2),
        claimed_windows: Mutex::new(Vec::with_capacity(1)),
        available_command_buffers: Mutex::new(available_command_buffers),
        submitted_command_buffers: Mutex::new(Vec::new()),
        // Create fence pool
        available_fences: Mutex::new(Vec::with_capacity(2)),
        uniform_buffer_pool: Mutex::new(Vec::with_capacity(32)),
        // Create deferred destroy arrays
        dispose: Mutex::new(Default::default()),
        blit: Mutex::new(BlitResources::default()),
    };

    // Create uniform buffer pool
    {
        let mut pool = lock(&renderer.uniform_buffer_pool);
        for _ in 0..32 {
            // Note (upstream): C keeps a NULL buffer in the pool when the
            // creation fails; it is left out here.
            if let Some(uniform_buffer) = renderer.create_uniform_buffer(UNIFORM_BUFFER_SIZE) {
                pool.push(uniform_buffer);
            }
        }
    }

    // Initialize blit resources
    let blit = renderer.init_blit_resources();
    *lock(&renderer.blit) = blit;

    Ok(renderer)
}

impl MetalRenderer {
    /// Translation of `METAL_INTERNAL_InitBlitResources()`.
    fn init_blit_resources(&self) -> BlitResources {
        let shader =
            |code: &[u8], stage: ShaderStage, entrypoint: &str, uniforms: u32, error: &str| {
                let shader_module_create_info = ShaderCreateInfo {
                    code,
                    entrypoint,
                    format: ShaderFormat::METALLIB,
                    stage,
                    num_samplers: uniforms,
                    num_storage_textures: 0,
                    num_storage_buffers: 0,
                    num_uniform_buffers: uniforms,
                    props: None,
                };
                match self.create_shader_internal(&shader_module_create_info) {
                    Ok(shader) => Some(Shader::from_backend(BackendObject::new(shader))),
                    Err(_) => {
                        crate::log::error!(Category::Gpu, "{}", error);
                        None
                    }
                }
            };

        // Fullscreen vertex shader
        let vertex_shader = shader(
            &FULLSCREEN_VERT,
            ShaderStage::Vertex,
            "FullscreenVert",
            0,
            "Failed to compile vertex shader for blit!",
        );

        // BlitFrom2D fragment shader
        let from_2d_shader = shader(
            &BLIT_FROM_2D,
            ShaderStage::Fragment,
            "BlitFrom2D",
            1,
            "Failed to compile BlitFrom2D fragment shader!",
        );

        // BlitFrom2DArray fragment shader
        let from_2d_array_shader = shader(
            &BLIT_FROM_2D_ARRAY,
            ShaderStage::Fragment,
            "BlitFrom2DArray",
            1,
            "Failed to compile BlitFrom2DArray fragment shader!",
        );

        // BlitFrom3D fragment shader
        let from_3d_shader = shader(
            &BLIT_FROM_3D,
            ShaderStage::Fragment,
            "BlitFrom3D",
            1,
            "Failed to compile BlitFrom3D fragment shader!",
        );

        // BlitFromCube fragment shader
        let from_cube_shader = shader(
            &BLIT_FROM_CUBE,
            ShaderStage::Fragment,
            "BlitFromCube",
            1,
            "Failed to compile BlitFromCube fragment shader!",
        );

        // BlitFromCubeArray fragment shader
        let from_cube_array_shader = shader(
            &BLIT_FROM_CUBE_ARRAY,
            ShaderStage::Fragment,
            "BlitFromCubeArray",
            1,
            "Failed to compile BlitFromCubeArray fragment shader!",
        );

        // Create samplers
        let mut createinfo = SamplerCreateInfo {
            address_mode_u: SamplerAddressMode::ClampToEdge,
            address_mode_v: SamplerAddressMode::ClampToEdge,
            address_mode_w: SamplerAddressMode::ClampToEdge,
            enable_anisotropy: false,
            enable_compare: false,
            mag_filter: Filter::Nearest,
            min_filter: Filter::Nearest,
            mipmap_mode: SamplerMipmapMode::Nearest,
            mip_lod_bias: 0.0,
            min_lod: 0.0,
            max_lod: 1000.0,
            max_anisotropy: 1.0,
            compare_op: CompareOp::Always,
            props: None,
        };
        let sampler =
            |info: &SamplerCreateInfo, error: &str| match self.create_sampler_internal(info) {
                Ok(sampler) => Some(Sampler::from_backend(BackendObject::new(sampler))),
                Err(_) => {
                    crate::log::error!(Category::Gpu, "{}", error);
                    None
                }
            };

        let nearest_sampler = sampler(&createinfo, "Failed to create blit nearest sampler!");

        createinfo.mag_filter = Filter::Linear;
        createinfo.min_filter = Filter::Linear;
        createinfo.mipmap_mode = SamplerMipmapMode::Linear;

        let linear_sampler = sampler(&createinfo, "Failed to create blit linear sampler!");

        BlitResources {
            vertex_shader,
            from_2d_shader,
            from_2d_array_shader,
            from_3d_shader,
            from_cube_shader,
            from_cube_array_shader,
            nearest_sampler,
            linear_sampler,
            // Allocate the dynamic blit pipeline list
            pipelines: BlitPipelineCache::PerFormat(Vec::with_capacity(2)),
        }
    }

    /// Translation of `METAL_DestroyDevice()` (the rest is dropping the
    /// renderer: the pools, the queue and the device).
    pub(super) fn destroy_device(&mut self) {
        // Flush any remaining GPU work...
        let _ = self.wait_internal();

        // Release the window data
        self.release_claimed_windows();

        // Release the blit resources
        // (METAL_INTERNAL_DestroyBlitResources(): the samplers, the shaders
        // and the pipelines are the backend's front-end handles)
        *lock(&self.blit) = BlitResources::default();
    }

    /// Translation of `METAL_SupportsTextureFormat()`.
    // FIXME: Check simultaneous read-write support
    pub(super) fn supports_texture_format_internal(
        &self,
        format: TextureFormat,
        texture_type: TextureType,
        usage: TextureUsageFlags,
    ) -> bool {
        use TextureFormat as F;

        // Only depth textures can be used as... depth textures
        if usage.contains(TextureUsageFlags::DEPTH_STENCIL_TARGET) && !format.is_depth_format() {
            return false;
        }

        let device = self.device.get();

        // Cube arrays are not supported on older iOS devices
        if texture_type == TextureType::CubeArray {
            if cfg!(target_os = "macos") {
                return true;
            }
            // SAFETY: the device's feature queries.
            if !(available!(ios(13, 0), tvos(13, 0))
                && unsafe {
                    supports_family(device, MTL_GPU_FAMILY_COMMON2)
                        || supports_family(device, MTL_GPU_FAMILY_APPLE4)
                })
            {
                return false;
            }
        }

        // SAFETY: the device's feature queries.
        unsafe {
            match format {
                // Apple GPU exclusive
                F::B5G6R5_UNORM | F::B5G5R5A1_UNORM | F::B4G4R4A4_UNORM => {
                    if available!(macos(10, 15), ios(13, 0), tvos(13, 0)) {
                        supports_family(device, MTL_GPU_FAMILY_APPLE1)
                    } else {
                        false
                    }
                }

                // Requires BC compression support
                F::BC1_RGBA_UNORM
                | F::BC2_RGBA_UNORM
                | F::BC3_RGBA_UNORM
                | F::BC4_R_UNORM
                | F::BC5_RG_UNORM
                | F::BC7_RGBA_UNORM
                | F::BC6H_RGB_FLOAT
                | F::BC6H_RGB_UFLOAT
                | F::BC1_RGBA_UNORM_SRGB
                | F::BC2_RGBA_UNORM_SRGB
                | F::BC3_RGBA_UNORM_SRGB
                | F::BC7_RGBA_UNORM_SRGB => {
                    if available!(ios(16, 4), tvos(16, 4)) {
                        if usage.contains(TextureUsageFlags::COLOR_TARGET) {
                            return false;
                        }
                        if available!(macos(11, 0)) {
                            let r: Bool = msg_send![device, supportsBCTextureCompression];
                            r.is_true()
                        } else {
                            true
                        }
                    } else {
                        false
                    }
                }

                // Requires D24S8 support
                F::D24_UNORM | F::D24_UNORM_S8_UINT => {
                    if cfg!(target_os = "macos") {
                        let r: Bool = msg_send![device, isDepth24Stencil8PixelFormatSupported];
                        r.is_true()
                    } else {
                        false
                    }
                }

                F::D16_UNORM => available!(macos(10, 12), ios(13, 0), tvos(13, 0)),

                F::ASTC_4x4_UNORM
                | F::ASTC_5x4_UNORM
                | F::ASTC_5x5_UNORM
                | F::ASTC_6x5_UNORM
                | F::ASTC_6x6_UNORM
                | F::ASTC_8x5_UNORM
                | F::ASTC_8x6_UNORM
                | F::ASTC_8x8_UNORM
                | F::ASTC_10x5_UNORM
                | F::ASTC_10x6_UNORM
                | F::ASTC_10x8_UNORM
                | F::ASTC_10x10_UNORM
                | F::ASTC_12x10_UNORM
                | F::ASTC_12x12_UNORM
                | F::ASTC_4x4_UNORM_SRGB
                | F::ASTC_5x4_UNORM_SRGB
                | F::ASTC_5x5_UNORM_SRGB
                | F::ASTC_6x5_UNORM_SRGB
                | F::ASTC_6x6_UNORM_SRGB
                | F::ASTC_8x5_UNORM_SRGB
                | F::ASTC_8x6_UNORM_SRGB
                | F::ASTC_8x8_UNORM_SRGB
                | F::ASTC_10x5_UNORM_SRGB
                | F::ASTC_10x6_UNORM_SRGB
                | F::ASTC_10x8_UNORM_SRGB
                | F::ASTC_10x10_UNORM_SRGB
                | F::ASTC_12x10_UNORM_SRGB
                | F::ASTC_12x12_UNORM_SRGB => {
                    if cfg!(target_os = "macos") {
                        if available!(macos(11, 0)) {
                            supports_family(device, MTL_GPU_FAMILY_APPLE7)
                        } else {
                            false
                        }
                    } else {
                        true
                    }
                }
                F::ASTC_4x4_FLOAT
                | F::ASTC_5x4_FLOAT
                | F::ASTC_5x5_FLOAT
                | F::ASTC_6x5_FLOAT
                | F::ASTC_6x6_FLOAT
                | F::ASTC_8x5_FLOAT
                | F::ASTC_8x6_FLOAT
                | F::ASTC_8x8_FLOAT
                | F::ASTC_10x5_FLOAT
                | F::ASTC_10x6_FLOAT
                | F::ASTC_10x8_FLOAT
                | F::ASTC_10x10_FLOAT
                | F::ASTC_12x10_FLOAT
                | F::ASTC_12x12_FLOAT => {
                    if cfg!(target_os = "macos") {
                        if available!(macos(11, 0)) {
                            supports_family(device, MTL_GPU_FAMILY_APPLE7)
                        } else {
                            false
                        }
                    } else if available!(ios(13, 0), tvos(13, 0)) {
                        supports_family(device, MTL_GPU_FAMILY_APPLE6)
                    } else {
                        false
                    }
                }
                _ => true,
            }
        }
    }
}
