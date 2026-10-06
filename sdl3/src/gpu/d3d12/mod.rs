// Rust translation of src/gpu/d3d12/SDL_gpu_d3d12.c from Simple
// DirectMedia Layer (part 1 of 2: devices and resources).
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! The Direct3D 12 GPU backend ("direct3d12", tried before Vulkan on
//! Windows): the GPU API on a Direct3D 12 device. `d3d12.dll`, `dxgi.dll`
//! (and, in debug mode, `dxgidebug.dll`, and the PIX runtime when it is
//! there) are loaded at run time, as upstream does, and the COM interfaces
//! are declared in [`d3d`] (the DXGI ones are the Direct3D 11 renderer's).
//! It takes DXBC shaders, and DXIL ones on devices with shader model 6.
//!
//! # What is translated (part 1)
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
//! * the blit shaders (DXBC, [`shaders`], made by
//!   `tools/gen_d3d12_shaders.py`) and samplers, made with the device;
//! * `SupportsTextureFormat`, `SupportsSampleCount`, and `Wait` (without
//!   command buffers to wait for yet).
//!
//! # What is left (part 2)
//!
//! Command buffers and their allocators, barriers and resource tracking
//! (the default resource states and subresource indices are here),
//! render, compute and copy passes (uploads, downloads with the texture
//! pitch workaround, copies, blits through the blit pipelines, mipmap
//! generation), uniform data, descriptor binding into the shader-visible
//! heaps, the debug labels (PIX), windows and swapchains, fences,
//! submission, cancelling, and `WaitForFences`. Every
//! [`GpuDriver`](crate::gpu::sysgpu::GpuDriver) method of these is marked
//! `// part 2` below and fails with "not translated yet" (or does nothing).
//!
//! The OpenXR parts (`HAVE_GPU_OPENXR`) and the Xbox (GDK) code are not
//! translated.

mod d3d;
mod descriptors;
mod device;
mod pipelines;
mod resources;
mod shaders;
mod tables;
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

use d3d::*;
use descriptors::{GpuDescriptorHeapPool, StagingDescriptorPool};
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

/// The error of the methods part 2 translates.
fn not_translated(func: &str) -> Error {
    Error::new(format!(
        "Direct3D 12 {func} is unsupported, not translated yet"
    ))
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
#[allow(dead_code)] // (part 2: debug labels)
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
#[allow(dead_code)] // (part 2: blits)
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
/// its descriptor heaps, and `submitLock` is the one `Wait` takes. Part 2
/// adds the command buffers, the fence pool and the claimed windows.
///
/// The fields are released in their order (upstream's
/// `D3D12_INTERNAL_DestroyRenderer()`): the pools and resources, then the
/// Direct3D objects, then the libraries.
struct D3D12Renderer {
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
    #[cfg_attr(not(test), allow(dead_code))] // (part 2: command buffers)
    gpu_descriptor_heap_pools: [GpuDescriptorHeapPool; 2],

    // Locks
    submit_lock: Mutex<()>,

    #[allow(dead_code)] // (part 2: frames in flight)
    allowed_frames_in_flight: AtomicU32,
    props: Properties,
    semantic: CString,

    debug_mode: bool,
    gpu_upload_heap_supported: bool,
    #[allow(dead_code)] // (part 2: downloads)
    unrestricted_buffer_texture_copy_pitch_supported: bool,
    #[allow(dead_code)] // (part 2: swapchains)
    supports_tearing: bool,
    info_queue_message_callback_supported: bool,
    #[allow(dead_code)] // (part 2: debug labels)
    winpixeventruntime_fns: WinPixEventRuntimeFns,
    serialize_root_signature: PfnD3d12SerializeRootSignature,

    // Indirect command signatures
    #[allow(dead_code)] // (part 2: indirect draws)
    indirect_draw_command_signature: D3d12CommandSignature,
    #[allow(dead_code)] // (part 2: indirect draws)
    indirect_indexed_draw_command_signature: D3d12CommandSignature,
    #[allow(dead_code)] // (part 2: indirect dispatches)
    indirect_dispatch_command_signature: D3d12CommandSignature,

    #[allow(dead_code)] // (part 2: submission)
    command_queue: D3d12CommandQueue,
    debug_info_queue: Option<D3d12InfoQueue>,
    device: D3d12Device,
    /// Note (upstream): C never releases the debug interface; it is
    /// released here, after the device.
    #[allow(dead_code)] // (kept alive, as upstream does)
    d3d12_debug: Option<D3d12Debug>,
    #[allow(dead_code)] // (part 2: swapchains)
    adapter: DxgiAdapter1,
    #[allow(dead_code)] // (part 2: swapchains)
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

    /// Translation of `D3D12_Wait()`: there are no submitted command
    /// buffers to wait for until part 2, so this destroys what was
    /// released.
    fn wait_internal(&self) -> Result<()> {
        let _submit = lock(&self.submit_lock);

        // part 2: signal a fence at the end of the command queue, block on
        // it and clean up the submitted command buffers.

        self.perform_pending_destroys();

        Ok(())
    }

    /// Translation of `D3D12_DestroyDevice()` (the rest of
    /// `D3D12_INTERNAL_DestroyRenderer()` is dropping the renderer).
    fn destroy_device(&mut self) {
        // Release blit pipeline structures
        self.release_blit_pipelines();

        // Flush any remaining GPU work...
        let _ = self.wait_internal();

        // part 2: release the claimed windows.
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

    fn insert_debug_label(&self, _command_buffer: &mut BackendCommandBuffer, _text: &str) {
        // part 2
    }

    fn push_debug_group(&self, _command_buffer: &mut BackendCommandBuffer, _name: &str) {
        // part 2
    }

    fn pop_debug_group(&self, _command_buffer: &mut BackendCommandBuffer) {
        // part 2
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
        _command_buffer: &mut BackendCommandBuffer,
        _color_target_infos: &[ColorTargetInfo<'_>],
        _depth_stencil_target_info: Option<&DepthStencilTargetInfo<'_>>,
    ) {
        // part 2
    }

    fn bind_graphics_pipeline(
        &self,
        _command_buffer: &mut BackendCommandBuffer,
        _graphics_pipeline: &BackendObject,
    ) {
        // part 2
    }

    fn set_viewport(&self, _command_buffer: &mut BackendCommandBuffer, _viewport: &Viewport) {
        // part 2
    }

    fn set_scissor(&self, _command_buffer: &mut BackendCommandBuffer, _scissor: &Rect) {
        // part 2
    }

    fn set_blend_constants(
        &self,
        _command_buffer: &mut BackendCommandBuffer,
        _blend_constants: FColor,
    ) {
        // part 2
    }

    fn set_stencil_reference(&self, _command_buffer: &mut BackendCommandBuffer, _reference: u8) {
        // part 2
    }

    fn bind_vertex_buffers(
        &self,
        _command_buffer: &mut BackendCommandBuffer,
        _first_slot: u32,
        _bindings: &[BufferBinding<'_>],
    ) {
        // part 2
    }

    fn bind_index_buffer(
        &self,
        _command_buffer: &mut BackendCommandBuffer,
        _binding: &BufferBinding<'_>,
        _index_element_size: IndexElementSize,
    ) {
        // part 2
    }

    fn bind_vertex_samplers(
        &self,
        _command_buffer: &mut BackendCommandBuffer,
        _first_slot: u32,
        _texture_sampler_bindings: &[TextureSamplerBinding<'_>],
    ) {
        // part 2
    }

    fn bind_vertex_storage_textures(
        &self,
        _command_buffer: &mut BackendCommandBuffer,
        _first_slot: u32,
        _storage_textures: &[&Texture],
    ) {
        // part 2
    }

    fn bind_vertex_storage_buffers(
        &self,
        _command_buffer: &mut BackendCommandBuffer,
        _first_slot: u32,
        _storage_buffers: &[&super::Buffer],
    ) {
        // part 2
    }

    fn bind_fragment_samplers(
        &self,
        _command_buffer: &mut BackendCommandBuffer,
        _first_slot: u32,
        _texture_sampler_bindings: &[TextureSamplerBinding<'_>],
    ) {
        // part 2
    }

    fn bind_fragment_storage_textures(
        &self,
        _command_buffer: &mut BackendCommandBuffer,
        _first_slot: u32,
        _storage_textures: &[&Texture],
    ) {
        // part 2
    }

    fn bind_fragment_storage_buffers(
        &self,
        _command_buffer: &mut BackendCommandBuffer,
        _first_slot: u32,
        _storage_buffers: &[&super::Buffer],
    ) {
        // part 2
    }

    fn push_vertex_uniform_data(
        &self,
        _command_buffer: &mut BackendCommandBuffer,
        _slot_index: u32,
        _data: &[u8],
    ) {
        // part 2
    }

    fn push_fragment_uniform_data(
        &self,
        _command_buffer: &mut BackendCommandBuffer,
        _slot_index: u32,
        _data: &[u8],
    ) {
        // part 2
    }

    fn draw_indexed_primitives(
        &self,
        _command_buffer: &mut BackendCommandBuffer,
        _num_indices: u32,
        _num_instances: u32,
        _first_index: u32,
        _vertex_offset: i32,
        _first_instance: u32,
    ) {
        // part 2
    }

    fn draw_primitives(
        &self,
        _command_buffer: &mut BackendCommandBuffer,
        _num_vertices: u32,
        _num_instances: u32,
        _first_vertex: u32,
        _first_instance: u32,
    ) {
        // part 2
    }

    fn draw_primitives_indirect(
        &self,
        _command_buffer: &mut BackendCommandBuffer,
        _buffer: &BackendObject,
        _offset: u32,
        _draw_count: u32,
    ) {
        // part 2
    }

    fn draw_indexed_primitives_indirect(
        &self,
        _command_buffer: &mut BackendCommandBuffer,
        _buffer: &BackendObject,
        _offset: u32,
        _draw_count: u32,
    ) {
        // part 2
    }

    fn end_render_pass(&self, _command_buffer: &mut BackendCommandBuffer) {
        // part 2
    }

    // Compute Pass

    fn begin_compute_pass(
        &self,
        _command_buffer: &mut BackendCommandBuffer,
        _storage_texture_bindings: &[StorageTextureReadWriteBinding<'_>],
        _storage_buffer_bindings: &[StorageBufferReadWriteBinding<'_>],
    ) {
        // part 2
    }

    fn bind_compute_pipeline(
        &self,
        _command_buffer: &mut BackendCommandBuffer,
        _compute_pipeline: &BackendObject,
    ) {
        // part 2
    }

    fn bind_compute_samplers(
        &self,
        _command_buffer: &mut BackendCommandBuffer,
        _first_slot: u32,
        _texture_sampler_bindings: &[TextureSamplerBinding<'_>],
    ) {
        // part 2
    }

    fn bind_compute_storage_textures(
        &self,
        _command_buffer: &mut BackendCommandBuffer,
        _first_slot: u32,
        _storage_textures: &[&Texture],
    ) {
        // part 2
    }

    fn bind_compute_storage_buffers(
        &self,
        _command_buffer: &mut BackendCommandBuffer,
        _first_slot: u32,
        _storage_buffers: &[&super::Buffer],
    ) {
        // part 2
    }

    fn push_compute_uniform_data(
        &self,
        _command_buffer: &mut BackendCommandBuffer,
        _slot_index: u32,
        _data: &[u8],
    ) {
        // part 2
    }

    fn dispatch_compute(
        &self,
        _command_buffer: &mut BackendCommandBuffer,
        _groupcount_x: u32,
        _groupcount_y: u32,
        _groupcount_z: u32,
    ) {
        // part 2
    }

    fn dispatch_compute_indirect(
        &self,
        _command_buffer: &mut BackendCommandBuffer,
        _buffer: &BackendObject,
        _offset: u32,
    ) {
        // part 2
    }

    fn end_compute_pass(&self, _command_buffer: &mut BackendCommandBuffer) {
        // part 2
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

    fn begin_copy_pass(&self, _command_buffer: &mut BackendCommandBuffer) {
        // part 2 (a no-op upstream too)
    }

    fn upload_to_texture(
        &self,
        _command_buffer: &mut BackendCommandBuffer,
        _source: &TextureTransferInfo<'_>,
        _destination: &TextureRegion<'_>,
        _cycle: bool,
    ) {
        // part 2
    }

    fn upload_to_buffer(
        &self,
        _command_buffer: &mut BackendCommandBuffer,
        _source: &TransferBufferLocation<'_>,
        _destination: &BufferRegion<'_>,
        _cycle: bool,
    ) {
        // part 2
    }

    fn copy_texture_to_texture(
        &self,
        _command_buffer: &mut BackendCommandBuffer,
        _source: &TextureLocation<'_>,
        _destination: &TextureLocation<'_>,
        _w: u32,
        _h: u32,
        _d: u32,
        _cycle: bool,
    ) {
        // part 2
    }

    fn copy_buffer_to_buffer(
        &self,
        _command_buffer: &mut BackendCommandBuffer,
        _source: &BufferLocation<'_>,
        _destination: &BufferLocation<'_>,
        _size: u32,
        _cycle: bool,
    ) {
        // part 2
    }

    fn generate_mipmaps(&self, _command_buffer: &mut CommandBuffer, _texture: &Texture) {
        // part 2
    }

    fn download_from_texture(
        &self,
        _command_buffer: &mut BackendCommandBuffer,
        _source: &TextureRegion<'_>,
        _destination: &TextureTransferInfo<'_>,
    ) {
        // part 2
    }

    fn download_from_buffer(
        &self,
        _command_buffer: &mut BackendCommandBuffer,
        _source: &BufferRegion<'_>,
        _destination: &TransferBufferLocation<'_>,
    ) {
        // part 2
    }

    fn end_copy_pass(&self, _command_buffer: &mut BackendCommandBuffer) {
        // part 2 (a no-op upstream too)
    }

    fn blit(&self, _command_buffer: &mut CommandBuffer, _info: &BlitInfo<'_>) {
        // part 2
    }

    // Submission/Presentation

    fn supports_swapchain_composition(
        &self,
        _window: Window,
        _swapchain_composition: SwapchainComposition,
    ) -> bool {
        // part 2
        false
    }

    fn supports_present_mode(&self, _window: Window, _present_mode: PresentMode) -> bool {
        // part 2
        false
    }

    fn claim_window(&self, _window: Window) -> Result<()> {
        // part 2
        Err(not_translated("ClaimWindow"))
    }

    fn release_window(&self, _window: Window) {
        // part 2
    }

    fn set_swapchain_parameters(
        &self,
        _window: Window,
        _swapchain_composition: SwapchainComposition,
        _present_mode: PresentMode,
    ) -> Result<()> {
        // part 2
        Err(not_translated("SetSwapchainParameters"))
    }

    fn set_allowed_frames_in_flight(&self, _allowed_frames_in_flight: u32) -> Result<()> {
        // part 2
        Err(not_translated("SetAllowedFramesInFlight"))
    }

    fn swapchain_texture_format(&self, _window: Window) -> Result<TextureFormat> {
        // part 2
        Err(not_translated("GetSwapchainTextureFormat"))
    }

    fn acquire_command_buffer(&self) -> Result<Box<BackendCommandBuffer>> {
        // part 2
        Err(not_translated("AcquireCommandBuffer"))
    }

    fn acquire_swapchain_texture(
        &self,
        _command_buffer: &mut BackendCommandBuffer,
        _window: Window,
    ) -> Result<Option<BackendSwapchainTexture>> {
        // part 2
        Err(not_translated("AcquireSwapchainTexture"))
    }

    fn wait_for_swapchain(&self, _window: Window) -> Result<()> {
        // part 2
        Err(not_translated("WaitForSwapchain"))
    }

    fn wait_and_acquire_swapchain_texture(
        &self,
        _command_buffer: &mut BackendCommandBuffer,
        _window: Window,
    ) -> Result<Option<BackendSwapchainTexture>> {
        // part 2
        Err(not_translated("WaitAndAcquireSwapchainTexture"))
    }

    fn submit(&self, _command_buffer: Box<BackendCommandBuffer>) -> Result<()> {
        // part 2
        Err(not_translated("Submit"))
    }

    fn submit_and_acquire_fence(
        &self,
        _command_buffer: Box<BackendCommandBuffer>,
    ) -> Result<BackendObject> {
        // part 2
        Err(not_translated("SubmitAndAcquireFence"))
    }

    fn cancel(&self, _command_buffer: Box<BackendCommandBuffer>) -> Result<()> {
        // part 2
        Err(not_translated("Cancel"))
    }

    fn wait(&self) -> Result<()> {
        self.wait_internal()
    }

    fn wait_for_fences(&self, _wait_all: bool, _fences: &[&BackendObject]) -> Result<()> {
        // part 2
        Err(not_translated("WaitForFences"))
    }

    fn query_fence(&self, _fence: &BackendObject) -> bool {
        // part 2
        false
    }

    fn release_fence(&self, _fence: &BackendObject) {
        // part 2
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
