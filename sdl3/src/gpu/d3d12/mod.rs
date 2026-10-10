// Rust translation of src/gpu/d3d12/SDL_gpu_d3d12.c from Simple
// DirectMedia Layer.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! The Direct3D 12 GPU backend ("direct3d12", tried before Vulkan on
//! Windows): the GPU API on a Direct3D 12 device. `d3d12.dll`, `dxgi.dll`
//! (and, in debug mode, `dxgidebug.dll`, and the PIX runtime when it is
//! there) are loaded at run time, as upstream does, and the COM interfaces
//! are declared in [`d3d`] (most DXGI ones are the Direct3D 11 renderer's).
//! It takes DXBC shaders, and DXIL ones on devices with shader model 6.
//!
//! * `PrepareDriver`, adapter selection (by GPU preference), the vendored
//!   runtime of the Agility SDK, the DXGI and Direct3D 12 debug layers
//!   with their info queue logging, the device, command queue and
//!   indirect command signatures, and the device properties ([`device`]);
//! * the conversion and format tables, the depth-stencil and typeless
//!   formats among them ([`tables`]);
//! * descriptor heaps, the staging descriptor pools and the pools of
//!   shader-visible heaps ([`descriptors`]);
//! * buffers, transfer buffers (mapping, cycling), uniform buffers and
//!   their pool, textures (with their views and subresources) and their
//!   cycling, samplers and DXBC/DXIL shaders, their release and the
//!   deferred destruction (`PerformPendingDestroys`), and the debug names
//!   ([`resources`]);
//! * root signatures, graphics and compute pipelines and the state
//!   conversions ([`pipelines`]);
//! * command buffers and their allocators, resource barriers and tracking,
//!   fences, uniform data, the shader-visible descriptor heaps of the
//!   command buffers, submission (with presentation and the cleanup of
//!   finished command buffers), cancelling, `Wait` and `WaitForFences`,
//!   and the debug labels (through the PIX runtime, a no-op without it)
//!   ([`commands`]);
//! * render passes (targets, dynamic state, bindings and their root
//!   parameters, draws, indirect ones too), compute passes and
//!   dispatches, copy passes (uploads and downloads with the texture pitch
//!   workaround, copies), and blits and mipmap generation through the
//!   front end's blit pipelines with the blit shaders (DXBC, [`shaders`],
//!   made by `tools/gen_d3d12_shaders.py`) ([`passes`]);
//! * claimed windows and their DXGI flip model swapchains, present modes,
//!   compositions (HDR too), frames in flight, swapchain texture
//!   acquisition and resizing ([`swapchain`]);
//! * `SupportsTextureFormat` and `SupportsSampleCount`.
//!
//! The OpenXR parts (`HAVE_GPU_OPENXR`) and the Xbox (GDK) code are not
//! translated.

mod commands;
pub(crate) mod d3d;
mod descriptors;
mod device;
mod passes;
mod pipelines;
mod resources;
mod shaders;
mod swapchain;
mod tables;
#[cfg(test)]
mod test_dxbc;
#[cfg(test)]
mod test_dxil;
#[cfg(test)]
mod tests;

use std::ffi::CString;
use std::ptr::NonNull;
use std::sync::atomic::AtomicU32;
use std::sync::{Arc, Mutex, MutexGuard};

use windows_sys::core::HRESULT;
use windows_sys::Win32::System::Diagnostics::Debug::{
    FormatMessageA, FORMAT_MESSAGE_FROM_SYSTEM, FORMAT_MESSAGE_IGNORE_INSERTS,
};

use commands::{D3D12CommandBuffer, D3D12Fence, UniformStage};
use d3d::*;
use descriptors::{GpuDescriptorHeapPool, StagingDescriptorPool};
use passes::GraphicsStage;
use pipelines::{D3D12ComputePipeline, D3D12GraphicsPipeline};
use resources::{
    BufferContainer, D3D12BufferType, D3D12Sampler, D3D12Shader, PendingDestroys, TextureContainer,
    UniformBuffer,
};
use tables::{sample_count, sdl_to_d3d12_depth_format, sdl_to_d3d12_texture_format};

use super::sysgpu::{
    BackendCommandBuffer, BackendDevice, BackendObject, BackendSwapchainTexture, BlitPipelineCache,
    ComputePipelineHeader, GpuBootstrap, GpuDriver, GraphicsPipelineHeader,
};
use super::{
    BlitInfo, BufferBinding, BufferLocation, BufferRegion, BufferUsageFlags, ColorTargetInfo,
    CommandBuffer, ComputePipelineCreateInfo, DepthStencilTargetInfo, GraphicsPipelineCreateInfo,
    IndexElementSize, PresentMode, SampleCount, Sampler, SamplerCreateInfo, Shader,
    ShaderCreateInfo, StorageBufferReadWriteBinding, StorageTextureReadWriteBinding,
    SwapchainComposition, Texture, TextureCreateInfo, TextureFormat, TextureLocation,
    TextureRegion, TextureSamplerBinding, TextureTransferInfo, TextureType, TextureUsageFlags,
    TransferBufferLocation, TransferBufferUsage, Viewport,
};
use crate::error::{Error, Result};
use crate::loadso::SharedObject;
use crate::log::Category;
use crate::properties::Properties;
use crate::render::direct3d11::d3d::{
    DxgiAdapter1, DxgiDebug, DxgiFactory4, DxgiInfoQueue, D3D_FEATURE_LEVEL_11_0, DXGI_DEBUG_ALL,
    DXGI_DEBUG_RLO_DETAIL, DXGI_DEBUG_RLO_SUMMARY, DXGI_ERROR_DEVICE_REMOVED,
};
use crate::video::{FColor, Rect, Window};

// Defines

const D3D12_DLL: &str = "d3d12.dll";
const WINPIXEVENTRUNTIME_DLL: &str = "WinPixEventRuntime.dll";
const DXGI_DLL: &str = "dxgi.dll";
const DXGIDEBUG_DLL: &str = "dxgidebug.dll";

/// `D3D_FEATURE_LEVEL_CHOICE`
const D3D_FEATURE_LEVEL_CHOICE: u32 = D3D_FEATURE_LEVEL_11_0;
/// `D3D_FEATURE_LEVEL_CHOICE_STR`
const D3D_FEATURE_LEVEL_CHOICE_STR: &str = "11_0";
// TODO: do these need to be tuned?
const VIEW_GPU_DESCRIPTOR_COUNT: u32 = 65536;
const SAMPLER_GPU_DESCRIPTOR_COUNT: u32 = 2048;
const STAGING_HEAP_DESCRIPTOR_COUNT: u32 = 1024;

/// The Direct3D 12 backend. Translation of `D3D12Driver`.
pub(crate) static D3D12_DRIVER: GpuBootstrap = GpuBootstrap {
    name: "direct3d12",
    prepare_driver: device::prepare_driver,
    create_device: d3d12_create_device,
};

/// `D3D12_CreateDevice()` for the bootstrap: the renderer as a driver.
fn d3d12_create_device(
    debug_mode: bool,
    prefer_low_power: bool,
    props: &Properties,
) -> Result<BackendDevice> {
    let (renderer, shader_formats) = device::create_device(debug_mode, prefer_low_power, props)?;
    Ok(BackendDevice {
        driver: Box::new(renderer),
        shader_formats,
    })
}

/// Lock a mutex, going on with the data of a holder that panicked.
fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

/// The error of a failed call, logged in debug mode. Translation of
/// `SET_STRING_ERROR_AND_RETURN`.
fn set_string_error(debug_mode: bool, msg: &str) -> Error {
    if debug_mode {
        crate::log::error!(Category::Gpu, "{}", msg);
    }
    Error::new(msg.to_owned())
}

/// The error of a failed Direct3D call: `msg`, the system's message for
/// the `HRESULT` (the device's removal reason for a removed device) and
/// the code, logged in debug mode. Translation of
/// `D3D12_INTERNAL_SetError()`.
fn d3d12_error(debug_mode: bool, device: Option<&D3d12Device>, msg: &str, res: HRESULT) -> Error {
    const MAX_ERROR_LEN: usize = 1024; // FIXME: Arbitrary!

    let mut res = res;
    if res == DXGI_ERROR_DEVICE_REMOVED {
        if let Some(device) = device {
            res = device.device_removed_reason();
        }
    }

    // Buffer for text, ensure space for \0 terminator after buffer
    let mut wsz_msg_buff = [0u8; MAX_ERROR_LEN + 1];

    // Try to get the message from the system errors.
    // SAFETY: the buffer holds MAX_ERROR_LEN + 1 bytes; no source or
    // arguments are used with these flags.
    let dw_chars = unsafe {
        FormatMessageA(
            FORMAT_MESSAGE_FROM_SYSTEM | FORMAT_MESSAGE_IGNORE_INSERTS,
            std::ptr::null(),
            res as u32,
            0,
            wsz_msg_buff.as_mut_ptr(),
            MAX_ERROR_LEN as u32,
            std::ptr::null(),
        )
    } as usize;

    // No message? Screw it, just post the code.
    let message = if dw_chars == 0 {
        format!("{msg}! Error Code: (0x{:08X})", res as u32)
    } else {
        // Ensure valid range
        let mut dw_chars = dw_chars.min(MAX_ERROR_LEN);

        // Trim whitespace from tail of message
        while dw_chars > 0 && wsz_msg_buff[dw_chars - 1] <= b' ' {
            dw_chars -= 1;
        }

        format!(
            "{msg}! Error Code: {} (0x{:08X})",
            String::from_utf8_lossy(&wsz_msg_buff[..dw_chars]),
            res as u32
        )
    };
    if debug_mode {
        crate::log::error!(Category::Gpu, "{}", message);
    }
    Error::new(message)
}

/// A backend object of this backend.
fn object<T: std::any::Any + Send + Sync>(object: &BackendObject) -> Arc<T> {
    object
        .downcast::<T>()
        .expect("a GPU object of another backend")
}

/// The functions of the PIX runtime for the debug labels. Translation of
/// `WinPixEventRuntimeFns`.
#[derive(Clone, Copy, Debug, Default)]
struct WinPixEventRuntimeFns {
    begin_event_on_command_list: Option<PfnBeginEventOnCommandList>,
    end_event_on_command_list: Option<PfnEndEventOnCommandList>,
    set_marker_on_command_list: Option<PfnBeginEventOnCommandList>,
}

/// The blit shaders, samplers and pipelines (`blitVertexShader`...,
/// `blitPipelines`): front-end handles the backend owns (and releases).
/// A shader or sampler that failed to be made is `None`, as upstream's is
/// NULL.
#[derive(Debug)]
struct BlitResources {
    vertex_shader: Option<Shader>,
    from_2d_shader: Option<Shader>,
    from_2d_array_shader: Option<Shader>,
    from_3d_shader: Option<Shader>,
    from_cube_shader: Option<Shader>,
    from_cube_array_shader: Option<Shader>,

    nearest_sampler: Option<Sampler>,
    linear_sampler: Option<Sampler>,

    pipelines: BlitPipelineCache,
}

impl Default for BlitResources {
    fn default() -> BlitResources {
        BlitResources {
            vertex_shader: None,
            from_2d_shader: None,
            from_2d_array_shader: None,
            from_3d_shader: None,
            from_cube_shader: None,
            from_cube_array_shader: None,
            nearest_sampler: None,
            linear_sampler: None,
            pipelines: BlitPipelineCache::PerFormat(Vec::new()),
        }
    }
}

/// The DXGI debug interface, which reports the live objects when it is
/// released (after the renderer's other objects are).
#[derive(Debug)]
struct LiveObjectReporter(DxgiDebug);

impl Drop for LiveObjectReporter {
    fn drop(&mut self) {
        self.0.report_live_objects(
            DXGI_DEBUG_ALL,
            DXGI_DEBUG_RLO_SUMMARY | DXGI_DEBUG_RLO_DETAIL,
        );
    }
}

/// The Direct3D 12 device. Translation of `D3D12Renderer`.
///
/// Upstream's locks guard the same state here: `acquireUniformBufferLock`
/// the uniform buffer pool, `disposeLock` the pending destroys, each pool
/// its descriptor heaps, `acquireCommandBufferLock` the available command
/// buffers, `submitLock` the submitted ones, `fenceLock` the fence pool
/// and `windowLock` the claimed windows.
///
/// The fields are released in their order (upstream's
/// `D3D12_INTERNAL_DestroyRenderer()`): the pools and resources, then the
/// Direct3D objects, then the libraries.
struct D3D12Renderer {
    /// `claimedWindows`, with `windowLock`.
    claimed_windows: Mutex<Vec<Arc<swapchain::WindowEntry>>>,

    /// `submittedCommandBuffers`, with `submitLock`.
    submit_lock: Mutex<Vec<D3D12CommandBuffer>>,
    /// `availableCommandBuffers`, with `acquireCommandBufferLock`.
    command_buffer_pool: Mutex<Vec<D3D12CommandBuffer>>,
    /// `availableFences`, with `fenceLock`.
    fence_pool: Mutex<Vec<Arc<D3D12Fence>>>,

    // Resources
    /// `uniformBufferPool`, with `acquireUniformBufferLock`.
    uniform_buffer_pool: Mutex<Vec<UniformBuffer>>,

    // Deferred resource releasing
    /// The `*ToDestroy` arrays, with `disposeLock`.
    dispose: Mutex<PendingDestroys>,

    // Blit
    blit: Mutex<BlitResources>,

    /// `stagingDescriptorPools` (one per heap type).
    staging_descriptor_pools:
        [Arc<StagingDescriptorPool>; D3D12_DESCRIPTOR_HEAP_TYPE_NUM_TYPES as usize],
    /// `gpuDescriptorHeapPools` (CBV/SRV/UAV, sampler).
    gpu_descriptor_heap_pools: [GpuDescriptorHeapPool; 2],

    allowed_frames_in_flight: AtomicU32,
    props: Properties,
    semantic: CString,

    debug_mode: bool,
    gpu_upload_heap_supported: bool,
    unrestricted_buffer_texture_copy_pitch_supported: bool,
    supports_tearing: bool,
    info_queue_message_callback_supported: bool,
    winpixeventruntime_fns: WinPixEventRuntimeFns,
    serialize_root_signature: PfnD3d12SerializeRootSignature,

    // Indirect command signatures
    indirect_draw_command_signature: D3d12CommandSignature,
    indirect_indexed_draw_command_signature: D3d12CommandSignature,
    indirect_dispatch_command_signature: D3d12CommandSignature,

    command_queue: D3d12CommandQueue,
    debug_info_queue: Option<D3d12InfoQueue>,
    device: D3d12Device,
    /// Note (upstream): C never releases the debug interface; it is
    /// released here, after the device.
    #[allow(dead_code)] // (kept alive, as upstream does)
    d3d12_debug: Option<D3d12Debug>,
    #[allow(dead_code)] // (kept as upstream does)
    adapter: DxgiAdapter1,
    factory: DxgiFactory4,
    /// Note (upstream): C never releases the DXGI info queue; it is
    /// released here.
    #[allow(dead_code)] // (kept alive, as upstream does)
    dxgi_info_queue: Option<DxgiInfoQueue>,
    #[allow(dead_code)] // (reports when dropped)
    dxgi_debug: Option<LiveObjectReporter>,

    #[allow(dead_code)] // (kept loaded)
    d3d12_dll: SharedObject,
    #[allow(dead_code)] // (kept loaded)
    dxgi_dll: SharedObject,
    #[allow(dead_code)] // (kept loaded)
    dxgidebug_dll: Option<SharedObject>,
    #[allow(dead_code)] // (kept loaded)
    winpixeventruntime_dll: Option<SharedObject>,
}

// SAFETY: Direct3D 12 devices, queues, heaps and resources, and DXGI
// factories and adapters, are free-threaded; the renderer's mutable state
// is behind its locks, as upstream's is.
unsafe impl Send for D3D12Renderer {}
// SAFETY: as for Send.
unsafe impl Sync for D3D12Renderer {}

impl D3D12Renderer {
    /// `SET_STRING_ERROR_AND_RETURN` with this renderer's debug mode.
    fn set_string_error(&self, msg: &str) -> Error {
        set_string_error(self.debug_mode, msg)
    }

    /// `D3D12_INTERNAL_SetError()` with this renderer.
    fn set_error(&self, msg: &str, res: HRESULT) -> Error {
        d3d12_error(self.debug_mode, Some(&self.device), msg, res)
    }

    /// Translation of `D3D12_DestroyDevice()` (the rest of
    /// `D3D12_INTERNAL_DestroyRenderer()` is dropping the renderer).
    fn destroy_device(&mut self) {
        // Release blit pipeline structures
        self.release_blit_pipelines();

        // Flush any remaining GPU work...
        let _ = self.wait_internal();

        // Release window data
        self.release_claimed_windows();
    }

    /// Translation of `D3D12_SupportsTextureFormat()`.
    fn supports_texture_format_internal(
        &self,
        format: TextureFormat,
        texture_type: TextureType,
        usage: TextureUsageFlags,
    ) -> bool {
        let dxgi_format = sdl_to_d3d12_texture_format(format);
        let mut format_support = FeatureDataFormatSupport {
            format: dxgi_format,
            support1: D3D12_FORMAT_SUPPORT1_NONE,
            support2: D3D12_FORMAT_SUPPORT2_NONE,
        };

        let res = self
            .device
            .check_feature_support(D3D12_FEATURE_FORMAT_SUPPORT, &mut format_support);
        if res < 0 {
            // Format is apparently unknown
            return false;
        }

        let support1 = format_support.support1;
        let support2 = format_support.support2;

        // Is the texture type supported?
        let type_bit = match texture_type {
            TextureType::Texture2D | TextureType::Texture2DArray => D3D12_FORMAT_SUPPORT1_TEXTURE2D,
            TextureType::Texture3D => D3D12_FORMAT_SUPPORT1_TEXTURE3D,
            TextureType::Cube | TextureType::CubeArray => D3D12_FORMAT_SUPPORT1_TEXTURECUBE,
        };
        if support1 & type_bit == 0 {
            return false;
        }

        // Are the usage flags supported?
        if usage.contains(TextureUsageFlags::SAMPLER)
            && support1 & D3D12_FORMAT_SUPPORT1_SHADER_SAMPLE == 0
        {
            return false;
        }
        if usage.intersects(
            TextureUsageFlags::GRAPHICS_STORAGE_READ | TextureUsageFlags::COMPUTE_STORAGE_READ,
        ) && support1 & D3D12_FORMAT_SUPPORT1_SHADER_LOAD == 0
        {
            return false;
        }
        if usage.contains(TextureUsageFlags::COMPUTE_STORAGE_WRITE)
            && support2 & D3D12_FORMAT_SUPPORT2_UAV_TYPED_STORE == 0
        {
            return false;
        }
        if usage.contains(TextureUsageFlags::COMPUTE_STORAGE_SIMULTANEOUS_READ_WRITE)
            && support2 & D3D12_FORMAT_SUPPORT2_UAV_TYPED_LOAD == 0
        {
            return false;
        }
        if usage.contains(TextureUsageFlags::COLOR_TARGET)
            && support1 & D3D12_FORMAT_SUPPORT1_RENDER_TARGET == 0
        {
            return false;
        }

        // Special case check for depth, because D3D12 is great.
        format_support.format = sdl_to_d3d12_depth_format(format);
        format_support.support1 = D3D12_FORMAT_SUPPORT1_NONE;
        format_support.support2 = D3D12_FORMAT_SUPPORT2_NONE;

        let res = self
            .device
            .check_feature_support(D3D12_FEATURE_FORMAT_SUPPORT, &mut format_support);
        if res < 0 {
            // Format is apparently unknown
            return false;
        }

        if usage.contains(TextureUsageFlags::DEPTH_STENCIL_TARGET)
            && format_support.support1 & D3D12_FORMAT_SUPPORT1_DEPTH_STENCIL == 0
        {
            return false;
        }

        true
    }

    /// Translation of `D3D12_SupportsSampleCount()`.
    fn supports_sample_count_internal(
        &self,
        format: TextureFormat,
        sample_count_: SampleCount,
    ) -> bool {
        let mut feature_data = FeatureDataMultisampleQualityLevels {
            flags: 0,
            format: if format.is_depth_format() {
                sdl_to_d3d12_depth_format(format)
            } else {
                sdl_to_d3d12_texture_format(format)
            },
            sample_count: sample_count(sample_count_),
            num_quality_levels: 0,
        };

        let res = self
            .device
            .check_feature_support(D3D12_FEATURE_MULTISAMPLE_QUALITY_LEVELS, &mut feature_data);

        res >= 0 && feature_data.num_quality_levels > 0
    }
}

impl GpuDriver for D3D12Renderer {
    // Device

    fn destroy(&mut self) {
        self.destroy_device();
    }

    /// Translation of `D3D12_GetDeviceProperties()`.
    fn properties(&self) -> Properties {
        self.props.clone()
    }

    // State Creation

    fn create_compute_pipeline(
        &self,
        createinfo: &ComputePipelineCreateInfo<'_>,
    ) -> Result<(BackendObject, ComputePipelineHeader)> {
        let pipeline = self.create_compute_pipeline_internal(createinfo)?;
        let header = pipeline.header;
        Ok((BackendObject(pipeline), header))
    }

    fn create_graphics_pipeline(
        &self,
        createinfo: &GraphicsPipelineCreateInfo<'_>,
    ) -> Result<(BackendObject, GraphicsPipelineHeader)> {
        let vert_shader = object::<D3D12Shader>(&createinfo.vertex_shader.raw);
        let frag_shader = object::<D3D12Shader>(&createinfo.fragment_shader.raw);
        let pipeline =
            self.create_graphics_pipeline_internal(createinfo, &vert_shader, &frag_shader)?;
        let header = pipeline.header;
        Ok((BackendObject(pipeline), header))
    }

    fn create_sampler(&self, createinfo: &SamplerCreateInfo) -> Result<BackendObject> {
        Ok(BackendObject(self.create_sampler_internal(createinfo)?))
    }

    fn create_shader(&self, createinfo: &ShaderCreateInfo<'_>) -> Result<BackendObject> {
        Ok(BackendObject::new(self.create_shader_internal(createinfo)?))
    }

    fn create_texture(&self, createinfo: &TextureCreateInfo) -> Result<BackendObject> {
        Ok(BackendObject(self.create_texture_container(createinfo)?))
    }

    /// Translation of `D3D12_CreateBuffer()`.
    fn create_buffer(
        &self,
        usage_flags: BufferUsageFlags,
        size: u32,
        debug_name: Option<&str>,
    ) -> Result<BackendObject> {
        Ok(BackendObject(self.create_buffer_container(
            usage_flags,
            size,
            D3D12BufferType::Gpu,
            debug_name,
        )?))
    }

    /// Translation of `D3D12_CreateTransferBuffer()`.
    fn create_transfer_buffer(
        &self,
        usage: TransferBufferUsage,
        size: u32,
        debug_name: Option<&str>,
    ) -> Result<BackendObject> {
        Ok(BackendObject(self.create_buffer_container(
            BufferUsageFlags::default(),
            size,
            if usage == TransferBufferUsage::Upload {
                D3D12BufferType::Upload
            } else {
                D3D12BufferType::Download
            },
            debug_name,
        )?))
    }

    // Debug Naming

    fn set_buffer_name(&self, buffer: &BackendObject, text: &str) {
        self.set_buffer_name_internal(&object::<BufferContainer>(buffer), text);
    }

    fn set_texture_name(&self, texture: &BackendObject, text: &str) {
        self.set_texture_name_internal(&object::<TextureContainer>(texture), text);
    }

    fn insert_debug_label(&self, command_buffer: &mut BackendCommandBuffer, text: &str) {
        self.insert_debug_label_internal(Self::d3d12_command_buffer(command_buffer), text);
    }

    fn push_debug_group(&self, command_buffer: &mut BackendCommandBuffer, name: &str) {
        self.push_debug_group_internal(Self::d3d12_command_buffer(command_buffer), name);
    }

    fn pop_debug_group(&self, command_buffer: &mut BackendCommandBuffer) {
        self.pop_debug_group_internal(Self::d3d12_command_buffer(command_buffer));
    }

    // Disposal

    /// Translation of `D3D12_ReleaseTexture()`.
    fn release_texture(&self, texture: &BackendObject) {
        self.release_texture_container(&object::<TextureContainer>(texture));
    }

    fn release_sampler(&self, sampler: &BackendObject) {
        self.release_sampler_internal(object::<D3D12Sampler>(sampler));
    }

    /// Translation of `D3D12_ReleaseBuffer()`.
    fn release_buffer(&self, buffer: &BackendObject) {
        self.release_buffer_container(&object::<BufferContainer>(buffer));
    }

    /// Translation of `D3D12_ReleaseTransferBuffer()`.
    fn release_transfer_buffer(&self, transfer_buffer: &BackendObject) {
        self.release_buffer_container(&object::<BufferContainer>(transfer_buffer));
    }

    /// Translation of `D3D12_ReleaseShader()`: the shader (its copy of the
    /// bytecode) is freed when the front end drops its handle.
    fn release_shader(&self, _shader: &BackendObject) {}

    fn release_compute_pipeline(&self, compute_pipeline: &BackendObject) {
        self.release_compute_pipeline_internal(object::<D3D12ComputePipeline>(compute_pipeline));
    }

    fn release_graphics_pipeline(&self, graphics_pipeline: &BackendObject) {
        self.release_graphics_pipeline_internal(object::<D3D12GraphicsPipeline>(graphics_pipeline));
    }

    // Render Pass

    fn begin_render_pass(
        &self,
        command_buffer: &mut BackendCommandBuffer,
        color_target_infos: &[ColorTargetInfo<'_>],
        depth_stencil_target_info: Option<&DepthStencilTargetInfo<'_>>,
    ) {
        self.begin_render_pass_internal(
            Self::d3d12_command_buffer(command_buffer),
            color_target_infos,
            depth_stencil_target_info,
        );
    }

    fn bind_graphics_pipeline(
        &self,
        command_buffer: &mut BackendCommandBuffer,
        graphics_pipeline: &BackendObject,
    ) {
        self.bind_graphics_pipeline_internal(
            Self::d3d12_command_buffer(command_buffer),
            &object::<D3D12GraphicsPipeline>(graphics_pipeline),
        );
    }

    fn set_viewport(&self, command_buffer: &mut BackendCommandBuffer, viewport: &Viewport) {
        Self::set_viewport_internal(Self::d3d12_command_buffer(command_buffer), viewport);
    }

    fn set_scissor(&self, command_buffer: &mut BackendCommandBuffer, scissor: &Rect) {
        Self::set_scissor_internal(Self::d3d12_command_buffer(command_buffer), scissor);
    }

    fn set_blend_constants(
        &self,
        command_buffer: &mut BackendCommandBuffer,
        blend_constants: FColor,
    ) {
        Self::set_blend_constants_internal(
            Self::d3d12_command_buffer(command_buffer),
            blend_constants,
        );
    }

    fn set_stencil_reference(&self, command_buffer: &mut BackendCommandBuffer, reference: u8) {
        Self::set_stencil_reference_internal(Self::d3d12_command_buffer(command_buffer), reference);
    }

    fn bind_vertex_buffers(
        &self,
        command_buffer: &mut BackendCommandBuffer,
        first_slot: u32,
        bindings: &[BufferBinding<'_>],
    ) {
        Self::bind_vertex_buffers_internal(
            Self::d3d12_command_buffer(command_buffer),
            first_slot,
            bindings,
        );
    }

    fn bind_index_buffer(
        &self,
        command_buffer: &mut BackendCommandBuffer,
        binding: &BufferBinding<'_>,
        index_element_size: IndexElementSize,
    ) {
        Self::bind_index_buffer_internal(
            Self::d3d12_command_buffer(command_buffer),
            binding,
            index_element_size,
        );
    }

    fn bind_vertex_samplers(
        &self,
        command_buffer: &mut BackendCommandBuffer,
        first_slot: u32,
        texture_sampler_bindings: &[TextureSamplerBinding<'_>],
    ) {
        Self::bind_graphics_samplers(
            Self::d3d12_command_buffer(command_buffer),
            GraphicsStage::Vertex,
            first_slot,
            texture_sampler_bindings,
        );
    }

    fn bind_vertex_storage_textures(
        &self,
        command_buffer: &mut BackendCommandBuffer,
        first_slot: u32,
        storage_textures: &[&Texture],
    ) {
        Self::bind_graphics_storage_textures(
            Self::d3d12_command_buffer(command_buffer),
            GraphicsStage::Vertex,
            first_slot,
            storage_textures,
        );
    }

    fn bind_vertex_storage_buffers(
        &self,
        command_buffer: &mut BackendCommandBuffer,
        first_slot: u32,
        storage_buffers: &[&super::Buffer],
    ) {
        Self::bind_graphics_storage_buffers(
            Self::d3d12_command_buffer(command_buffer),
            GraphicsStage::Vertex,
            first_slot,
            storage_buffers,
        );
    }

    fn bind_fragment_samplers(
        &self,
        command_buffer: &mut BackendCommandBuffer,
        first_slot: u32,
        texture_sampler_bindings: &[TextureSamplerBinding<'_>],
    ) {
        Self::bind_graphics_samplers(
            Self::d3d12_command_buffer(command_buffer),
            GraphicsStage::Fragment,
            first_slot,
            texture_sampler_bindings,
        );
    }

    fn bind_fragment_storage_textures(
        &self,
        command_buffer: &mut BackendCommandBuffer,
        first_slot: u32,
        storage_textures: &[&Texture],
    ) {
        Self::bind_graphics_storage_textures(
            Self::d3d12_command_buffer(command_buffer),
            GraphicsStage::Fragment,
            first_slot,
            storage_textures,
        );
    }

    fn bind_fragment_storage_buffers(
        &self,
        command_buffer: &mut BackendCommandBuffer,
        first_slot: u32,
        storage_buffers: &[&super::Buffer],
    ) {
        Self::bind_graphics_storage_buffers(
            Self::d3d12_command_buffer(command_buffer),
            GraphicsStage::Fragment,
            first_slot,
            storage_buffers,
        );
    }

    /// Translation of `D3D12_PushVertexUniformData()`.
    fn push_vertex_uniform_data(
        &self,
        command_buffer: &mut BackendCommandBuffer,
        slot_index: u32,
        data: &[u8],
    ) {
        self.push_uniform_data(
            Self::d3d12_command_buffer(command_buffer),
            UniformStage::Vertex,
            slot_index,
            data,
        );
    }

    /// Translation of `D3D12_PushFragmentUniformData()`.
    fn push_fragment_uniform_data(
        &self,
        command_buffer: &mut BackendCommandBuffer,
        slot_index: u32,
        data: &[u8],
    ) {
        self.push_uniform_data(
            Self::d3d12_command_buffer(command_buffer),
            UniformStage::Fragment,
            slot_index,
            data,
        );
    }

    fn draw_indexed_primitives(
        &self,
        command_buffer: &mut BackendCommandBuffer,
        num_indices: u32,
        num_instances: u32,
        first_index: u32,
        vertex_offset: i32,
        first_instance: u32,
    ) {
        self.draw_indexed_primitives_internal(
            Self::d3d12_command_buffer(command_buffer),
            num_indices,
            num_instances,
            first_index,
            vertex_offset,
            first_instance,
        );
    }

    fn draw_primitives(
        &self,
        command_buffer: &mut BackendCommandBuffer,
        num_vertices: u32,
        num_instances: u32,
        first_vertex: u32,
        first_instance: u32,
    ) {
        self.draw_primitives_internal(
            Self::d3d12_command_buffer(command_buffer),
            num_vertices,
            num_instances,
            first_vertex,
            first_instance,
        );
    }

    fn draw_primitives_indirect(
        &self,
        command_buffer: &mut BackendCommandBuffer,
        buffer: &BackendObject,
        offset: u32,
        draw_count: u32,
    ) {
        self.draw_primitives_indirect_internal(
            Self::d3d12_command_buffer(command_buffer),
            &object::<BufferContainer>(buffer),
            offset,
            draw_count,
            false,
        );
    }

    fn draw_indexed_primitives_indirect(
        &self,
        command_buffer: &mut BackendCommandBuffer,
        buffer: &BackendObject,
        offset: u32,
        draw_count: u32,
    ) {
        self.draw_primitives_indirect_internal(
            Self::d3d12_command_buffer(command_buffer),
            &object::<BufferContainer>(buffer),
            offset,
            draw_count,
            true,
        );
    }

    fn end_render_pass(&self, command_buffer: &mut BackendCommandBuffer) {
        Self::end_render_pass_internal(Self::d3d12_command_buffer(command_buffer));
    }

    // Compute Pass

    fn begin_compute_pass(
        &self,
        command_buffer: &mut BackendCommandBuffer,
        storage_texture_bindings: &[StorageTextureReadWriteBinding<'_>],
        storage_buffer_bindings: &[StorageBufferReadWriteBinding<'_>],
    ) {
        self.begin_compute_pass_internal(
            Self::d3d12_command_buffer(command_buffer),
            storage_texture_bindings,
            storage_buffer_bindings,
        );
    }

    fn bind_compute_pipeline(
        &self,
        command_buffer: &mut BackendCommandBuffer,
        compute_pipeline: &BackendObject,
    ) {
        self.bind_compute_pipeline_internal(
            Self::d3d12_command_buffer(command_buffer),
            &object::<D3D12ComputePipeline>(compute_pipeline),
        );
    }

    fn bind_compute_samplers(
        &self,
        command_buffer: &mut BackendCommandBuffer,
        first_slot: u32,
        texture_sampler_bindings: &[TextureSamplerBinding<'_>],
    ) {
        Self::bind_compute_samplers_internal(
            Self::d3d12_command_buffer(command_buffer),
            first_slot,
            texture_sampler_bindings,
        );
    }

    fn bind_compute_storage_textures(
        &self,
        command_buffer: &mut BackendCommandBuffer,
        first_slot: u32,
        storage_textures: &[&Texture],
    ) {
        Self::bind_compute_storage_textures_internal(
            Self::d3d12_command_buffer(command_buffer),
            first_slot,
            storage_textures,
        );
    }

    fn bind_compute_storage_buffers(
        &self,
        command_buffer: &mut BackendCommandBuffer,
        first_slot: u32,
        storage_buffers: &[&super::Buffer],
    ) {
        Self::bind_compute_storage_buffers_internal(
            Self::d3d12_command_buffer(command_buffer),
            first_slot,
            storage_buffers,
        );
    }

    /// Translation of `D3D12_PushComputeUniformData()`.
    fn push_compute_uniform_data(
        &self,
        command_buffer: &mut BackendCommandBuffer,
        slot_index: u32,
        data: &[u8],
    ) {
        self.push_uniform_data(
            Self::d3d12_command_buffer(command_buffer),
            UniformStage::Compute,
            slot_index,
            data,
        );
    }

    fn dispatch_compute(
        &self,
        command_buffer: &mut BackendCommandBuffer,
        groupcount_x: u32,
        groupcount_y: u32,
        groupcount_z: u32,
    ) {
        self.dispatch_compute_internal(
            Self::d3d12_command_buffer(command_buffer),
            groupcount_x,
            groupcount_y,
            groupcount_z,
        );
    }

    fn dispatch_compute_indirect(
        &self,
        command_buffer: &mut BackendCommandBuffer,
        buffer: &BackendObject,
        offset: u32,
    ) {
        self.dispatch_compute_indirect_internal(
            Self::d3d12_command_buffer(command_buffer),
            &object::<BufferContainer>(buffer),
            offset,
        );
    }

    fn end_compute_pass(&self, command_buffer: &mut BackendCommandBuffer) {
        Self::end_compute_pass_internal(Self::d3d12_command_buffer(command_buffer));
    }

    // TransferBuffer Data

    fn map_transfer_buffer(
        &self,
        transfer_buffer: &BackendObject,
        cycle: bool,
    ) -> Result<NonNull<u8>> {
        self.map_transfer_buffer_internal(&object::<BufferContainer>(transfer_buffer), cycle)
    }

    fn unmap_transfer_buffer(&self, transfer_buffer: &BackendObject) {
        self.unmap_transfer_buffer_internal(&object::<BufferContainer>(transfer_buffer));
    }

    // Copy Pass

    /// Translation of `D3D12_BeginCopyPass()`.
    fn begin_copy_pass(&self, _command_buffer: &mut BackendCommandBuffer) {
        // no-op
    }

    fn upload_to_texture(
        &self,
        command_buffer: &mut BackendCommandBuffer,
        source: &TextureTransferInfo<'_>,
        destination: &TextureRegion<'_>,
        cycle: bool,
    ) {
        self.upload_to_texture_internal(
            Self::d3d12_command_buffer(command_buffer),
            source,
            destination,
            cycle,
        );
    }

    fn upload_to_buffer(
        &self,
        command_buffer: &mut BackendCommandBuffer,
        source: &TransferBufferLocation<'_>,
        destination: &BufferRegion<'_>,
        cycle: bool,
    ) {
        self.upload_to_buffer_internal(
            Self::d3d12_command_buffer(command_buffer),
            source,
            destination,
            cycle,
        );
    }

    fn copy_texture_to_texture(
        &self,
        command_buffer: &mut BackendCommandBuffer,
        source: &TextureLocation<'_>,
        destination: &TextureLocation<'_>,
        w: u32,
        h: u32,
        d: u32,
        cycle: bool,
    ) {
        self.copy_texture_to_texture_internal(
            Self::d3d12_command_buffer(command_buffer),
            source,
            destination,
            w,
            h,
            d,
            cycle,
        );
    }

    fn copy_buffer_to_buffer(
        &self,
        command_buffer: &mut BackendCommandBuffer,
        source: &BufferLocation<'_>,
        destination: &BufferLocation<'_>,
        size: u32,
        cycle: bool,
    ) {
        self.copy_buffer_to_buffer_internal(
            Self::d3d12_command_buffer(command_buffer),
            source,
            destination,
            size,
            cycle,
        );
    }

    fn generate_mipmaps(&self, command_buffer: &mut CommandBuffer, texture: &Texture) {
        self.generate_mipmaps_internal(command_buffer, texture);
    }

    fn download_from_texture(
        &self,
        command_buffer: &mut BackendCommandBuffer,
        source: &TextureRegion<'_>,
        destination: &TextureTransferInfo<'_>,
    ) {
        self.download_from_texture_internal(
            Self::d3d12_command_buffer(command_buffer),
            source,
            destination,
        );
    }

    fn download_from_buffer(
        &self,
        command_buffer: &mut BackendCommandBuffer,
        source: &BufferRegion<'_>,
        destination: &TransferBufferLocation<'_>,
    ) {
        self.download_from_buffer_internal(
            Self::d3d12_command_buffer(command_buffer),
            source,
            destination,
        );
    }

    /// Translation of `D3D12_EndCopyPass()`.
    fn end_copy_pass(&self, _command_buffer: &mut BackendCommandBuffer) {
        // no-op
    }

    fn blit(&self, command_buffer: &mut CommandBuffer, info: &BlitInfo<'_>) {
        self.blit_internal(command_buffer, info);
    }

    // Submission/Presentation

    fn supports_swapchain_composition(
        &self,
        window: Window,
        swapchain_composition: SwapchainComposition,
    ) -> bool {
        self.supports_swapchain_composition_internal(window, swapchain_composition)
    }

    fn supports_present_mode(&self, window: Window, present_mode: PresentMode) -> bool {
        self.supports_present_mode_internal(window, present_mode)
    }

    fn claim_window(&self, window: Window) -> Result<()> {
        self.claim_window_internal(window)
    }

    fn release_window(&self, window: Window) {
        self.release_window_internal(window);
    }

    fn set_swapchain_parameters(
        &self,
        window: Window,
        swapchain_composition: SwapchainComposition,
        present_mode: PresentMode,
    ) -> Result<()> {
        self.set_swapchain_parameters_internal(window, swapchain_composition, present_mode)
    }

    fn set_allowed_frames_in_flight(&self, allowed_frames_in_flight: u32) -> Result<()> {
        self.set_allowed_frames_in_flight_internal(allowed_frames_in_flight)
    }

    fn swapchain_texture_format(&self, window: Window) -> Result<TextureFormat> {
        self.swapchain_texture_format_internal(window)
    }

    fn acquire_command_buffer(&self) -> Result<Box<BackendCommandBuffer>> {
        Ok(Box::new(self.acquire_command_buffer_internal()?))
    }

    /// Translation of `D3D12_AcquireSwapchainTexture()`.
    fn acquire_swapchain_texture(
        &self,
        command_buffer: &mut BackendCommandBuffer,
        window: Window,
    ) -> Result<Option<BackendSwapchainTexture>> {
        self.acquire_swapchain_texture_internal(
            false,
            Self::d3d12_command_buffer(command_buffer),
            window,
        )
    }

    fn wait_for_swapchain(&self, window: Window) -> Result<()> {
        self.wait_for_swapchain_internal(window)
    }

    /// Translation of `D3D12_WaitAndAcquireSwapchainTexture()`.
    fn wait_and_acquire_swapchain_texture(
        &self,
        command_buffer: &mut BackendCommandBuffer,
        window: Window,
    ) -> Result<Option<BackendSwapchainTexture>> {
        self.acquire_swapchain_texture_internal(
            true,
            Self::d3d12_command_buffer(command_buffer),
            window,
        )
    }

    /// Translation of `D3D12_Submit()`.
    fn submit(&self, command_buffer: Box<BackendCommandBuffer>) -> Result<()> {
        self.submit_internal(Self::owned_command_buffer(command_buffer))?;
        Ok(())
    }

    /// Translation of `D3D12_SubmitAndAcquireFence()`.
    fn submit_and_acquire_fence(
        &self,
        command_buffer: Box<BackendCommandBuffer>,
    ) -> Result<BackendObject> {
        let mut d3d12_command_buffer = Self::owned_command_buffer(command_buffer);
        d3d12_command_buffer.auto_release_fence = false;
        let fence = self.submit_internal(d3d12_command_buffer)?;
        Ok(BackendObject(fence))
    }

    fn cancel(&self, command_buffer: Box<BackendCommandBuffer>) -> Result<()> {
        self.cancel_internal(Self::owned_command_buffer(command_buffer))
    }

    fn wait(&self) -> Result<()> {
        self.wait_internal()
    }

    fn wait_for_fences(&self, wait_all: bool, fences: &[&BackendObject]) -> Result<()> {
        let fences: Vec<Arc<D3D12Fence>> = fences.iter().map(|f| object::<D3D12Fence>(f)).collect();
        let fences: Vec<&D3D12Fence> = fences.iter().map(|f| &**f).collect();
        self.wait_for_fences_internal(wait_all, &fences)
    }

    fn query_fence(&self, fence: &BackendObject) -> bool {
        self.query_fence_internal(&object::<D3D12Fence>(fence))
    }

    fn release_fence(&self, fence: &BackendObject) {
        self.release_fence_internal(&object::<D3D12Fence>(fence));
    }

    // Feature Queries

    fn supports_texture_format(
        &self,
        format: TextureFormat,
        texture_type: TextureType,
        usage: TextureUsageFlags,
    ) -> bool {
        self.supports_texture_format_internal(format, texture_type, usage)
    }

    fn supports_sample_count(&self, format: TextureFormat, sample_count: SampleCount) -> bool {
        self.supports_sample_count_internal(format, sample_count)
    }
}
