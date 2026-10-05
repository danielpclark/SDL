// Rust translation of src/gpu/vulkan/SDL_gpu_vulkan.c from Simple
// DirectMedia Layer (part 1 of 2: devices and resources).
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! The Vulkan GPU backend ("vulkan"): the GPU API on a Vulkan device. The
//! Vulkan loader is the video driver's (`SDL_Vulkan_LoadLibrary()`), and
//! every Vulkan function is looked up at run time through its
//! `vkGetInstanceProcAddr` ([`vkfuncs`]).
//!
//! # What is translated (part 1)
//!
//! * instance and physical device selection, the logical device, and the
//!   `SDL_GPUVulkanOptions` and device creation properties ([`device`]);
//! * the memory allocator: suballocation from large allocations per memory
//!   type, with free-region merging and the defragmentation of fragmented
//!   allocations ([`memory`], [`binding`],
//!   `VulkanRenderer::defragment_memory` in [`resources`]);
//! * buffers, transfer buffers (mapping, cycling), uniform buffers,
//!   textures (with their views and subresources), samplers and SPIR-V
//!   shaders, their release and the deferred destruction
//!   (`PerformPendingDestroys`), memory barriers and resource tracking
//!   ([`resources`]);
//! * descriptor set layouts, descriptor pools and caches, pipeline layouts,
//!   graphics pipelines (with their transient render passes) and compute
//!   pipelines ([`pipelines`]);
//! * the format and usage tables ([`tables`]), `SupportsTextureFormat`,
//!   `SupportsSampleCount`, the device properties, the debug names and the
//!   debug labels, and `Wait` (without command buffers to clean yet).
//!
//! # What is left (part 2)
//!
//! Command buffers and their pools, render, compute and copy passes (with
//! the render pass and framebuffer caches, which part 1 only declares and
//! destroys), uploads, downloads, copies, blits and mipmap generation,
//! uniform data, descriptor set binding, windows and swapchains, fences,
//! submission (which runs `defragment_memory` and frees empty
//! allocations), cancelling, and `WaitForFences`. Every
//! [`GpuDriver`](crate::gpu::sysgpu::GpuDriver) method of these is marked
//! `// part 2` below and fails with "not translated yet" (or does nothing).
//! Part 2 also adds the transition of a new texture to its default layout
//! (see `create_texture_container`), texture cycling
//! (`VULKAN_INTERNAL_CycleActiveTexture`) and the `Prepare*ForWrite`
//! helpers, and extends [`resources::VulkanCommandBuffer`].
//!
//! The OpenXR parts (`HAVE_GPU_OPENXR`) are not translated.

mod binding;
mod device;
mod memory;
mod pipelines;
mod resources;
mod tables;
#[cfg(test)]
mod test_spirv;
#[cfg(test)]
mod tests;
mod vkfuncs;

use std::cell::RefCell;
use std::collections::HashMap;
use std::ptr::NonNull;
use std::sync::atomic::{AtomicBool, AtomicU32};
use std::sync::{Arc, Mutex, MutexGuard, RwLock};

use device::VulkanExtensions;
use memory::MemoryAllocator;
use pipelines::{
    ComputePipelineResourceLayoutHashTableKey, DescriptorSetCache, DescriptorSetLayout,
    DescriptorSetLayoutHashTableKey, GraphicsPipelineResourceLayoutHashTableKey,
    RenderPassHashTableKey, VulkanComputePipeline, VulkanComputePipelineResourceLayout,
    VulkanGraphicsPipeline, VulkanGraphicsPipelineResourceLayout, VulkanShader,
};
use resources::{
    BufferContainer, FramebufferHashTable, PendingDestroys, TextureContainer, UniformBuffer,
    VulkanBufferType, VulkanCommandBuffer, VulkanSampler,
};
use tables::{sdl_to_vk_sample_count, sdl_to_vk_texture_format, vk_error_messages};
use vkfuncs::{DeviceFunctions, GlobalFunctions, InstanceFunctions};

use super::sysgpu::{
    BackendCommandBuffer, BackendDevice, BackendObject, BackendSwapchainTexture,
    ComputePipelineHeader, GpuBootstrap, GpuDriver, GraphicsPipelineHeader,
};
use super::{
    BlitInfo, BufferBinding, BufferLocation, BufferRegion, BufferUsageFlags, ColorTargetInfo,
    CommandBuffer, ComputePipelineCreateInfo, DepthStencilTargetInfo, GraphicsPipelineCreateInfo,
    IndexElementSize, PresentMode, SampleCount, SamplerCreateInfo, ShaderCreateInfo, ShaderFormat,
    StorageBufferReadWriteBinding, StorageTextureReadWriteBinding, SwapchainComposition, Texture,
    TextureCreateInfo, TextureFormat, TextureLocation, TextureRegion, TextureSamplerBinding,
    TextureTransferInfo, TextureType, TextureUsageFlags, TransferBufferLocation,
    TransferBufferUsage, Viewport,
};
use crate::error::{Error, Result};
use crate::log::Category;
use crate::properties::Properties;
use crate::thread::ReentrantMutex;
use crate::video::vk::*;
use crate::video::{FColor, Rect, Window};

/// The Vulkan backend. Translation of `VulkanDriver`.
pub(crate) static VULKAN_DRIVER: GpuBootstrap = GpuBootstrap {
    name: "vulkan",
    prepare_driver: device::prepare_driver,
    create_device: vulkan_create_device,
};

/// `VULKAN_CreateDevice()` for the bootstrap: the renderer as a driver.
fn vulkan_create_device(
    debug_mode: bool,
    prefer_low_power: bool,
    props: &Properties,
) -> Result<BackendDevice> {
    let renderer = device::create_device(debug_mode, prefer_low_power, props)?;
    Ok(BackendDevice {
        driver: Box::new(renderer),
        shader_formats: ShaderFormat::SPIRV,
    })
}

/// Lock a mutex, going on with the data of a holder that panicked.
fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

/// The error of a failed call, logged in debug mode. Translation of
/// `SET_STRING_ERROR`.
fn set_string_error(debug_mode: bool, msg: &str) -> Error {
    if debug_mode {
        crate::log::error!(Category::Gpu, "{}", msg);
    }
    Error::new(msg.to_owned())
}

/// The error of a failed Vulkan call, logged in debug mode. The error of
/// `CHECK_VULKAN_ERROR_AND_RETURN`.
fn vulkan_error(debug_mode: bool, res: VkResult, func: &str) -> Error {
    let message = format!("{} {}", func, vk_error_messages(res));
    if debug_mode {
        crate::log::error!(Category::Gpu, "{}", message);
    }
    Error::new(message)
}

/// The error of the methods part 2 translates.
fn not_translated(func: &str) -> Error {
    Error::new(format!("Vulkan {func} is unsupported, not translated yet"))
}

/// The Vulkan device. Translation of `VulkanRenderer`.
///
/// Upstream's locks guard the same state here: `allocatorLock` is the
/// recursive [`ReentrantMutex`] around the allocator (allocation re-enters
/// it during defrag), `disposeLock` the pending destroys, and the fetch
/// locks the caches. Part 2 adds the command pools, the submitted command
/// buffers, the fence pool and the claimed windows.
struct VulkanRenderer {
    #[allow(dead_code)] // (part 2: surfaces)
    vk_get_instance_proc_addr: PfnVkGetInstanceProcAddr,
    #[allow(dead_code)] // (kept as upstream's globals)
    global: GlobalFunctions,
    instance: VkInstance,
    inst: InstanceFunctions,
    physical_device: VkPhysicalDevice,
    physical_device_properties: VkPhysicalDeviceProperties2,
    #[allow(dead_code)] // (read when the device is created)
    physical_device_driver_properties: VkPhysicalDeviceDriverProperties,
    logical_device: VkDevice,
    dev: DeviceFunctions,
    integrated_memory_notification: AtomicBool,
    out_of_device_local_memory_warning: AtomicBool,
    outof_bar_memory_warning: AtomicBool,
    fill_mode_only_warning: AtomicBool,

    // OpenXR
    #[allow(dead_code)] // (OpenXR isn't translated)
    minimum_vk_version: u32,

    debug_mode: bool,
    #[allow(dead_code)] // (kept as upstream does)
    prefer_low_power: bool,
    #[allow(dead_code)] // (kept as upstream does)
    require_hardware_acceleration: bool,
    props: Properties,
    #[allow(dead_code)] // (part 2: frames in flight)
    allowed_frames_in_flight: AtomicU32,

    #[allow(dead_code)] // (part 2: swapchains)
    supports: VulkanExtensions,
    supports_debug_utils: bool,
    #[allow(dead_code)] // (part 2: swapchains)
    supports_colorspace: bool,
    #[allow(dead_code)] // (kept as upstream does)
    supports_physical_device_properties2: bool,
    #[allow(dead_code)] // (kept as upstream does)
    supports_portability_enumeration: bool,
    supports_fill_mode_non_solid: bool,
    #[allow(dead_code)] // (part 2: indirect draws)
    supports_multi_draw_indirect: bool,

    /// `memoryAllocator`, with `allocatorLock`.
    memory_allocator: ReentrantMutex<RefCell<MemoryAllocator>>,
    memory_properties: VkPhysicalDeviceMemoryProperties,

    queue_family_index: u32,
    #[allow(dead_code)] // (part 2: submission)
    unified_queue: VkQueue,

    /// `submitLock` (part 2 puts the submitted command buffers here).
    submit_lock: Mutex<()>,
    /// The deferred resource destruction, with `disposeLock`.
    dispose: Mutex<PendingDestroys>,

    /// `renderPassHashTable` (filled by part 2).
    render_pass_hash_table: Mutex<HashMap<RenderPassHashTableKey, VkRenderPass>>,
    /// `framebufferHashTable` (filled by part 2).
    framebuffer_hash_table: Mutex<FramebufferHashTable>,
    graphics_pipeline_resource_layout_hash_table: Mutex<
        HashMap<
            GraphicsPipelineResourceLayoutHashTableKey,
            Arc<VulkanGraphicsPipelineResourceLayout>,
        >,
    >,
    compute_pipeline_resource_layout_hash_table: Mutex<
        HashMap<
            ComputePipelineResourceLayoutHashTableKey,
            Arc<VulkanComputePipelineResourceLayout>,
        >,
    >,
    descriptor_set_layout_hash_table:
        Mutex<HashMap<DescriptorSetLayoutHashTableKey, Arc<DescriptorSetLayout>>>,

    /// `uniformBufferPool` (with `acquireUniformBufferLock`).
    uniform_buffer_pool: Mutex<Vec<Arc<UniformBuffer>>>,

    /// `descriptorSetCachePool` (with `acquireCommandBufferLock`).
    descriptor_set_cache_pool: Mutex<Vec<DescriptorSetCache>>,

    layout_resource_id: AtomicU32,

    #[allow(dead_code)] // (part 2: uniform buffer offsets)
    min_ubo_alignment: u32,

    /// We don't want transfer commands to block each other,
    /// but we want all transfers to block during defrag.
    defrag_lock: RwLock<()>,

    defrag_in_progress: AtomicBool,
}

// SAFETY: the Vulkan handles are plain values, and the objects they name
// are used under the renderer's locks as upstream uses them (the device's
// external synchronization rules are upstream's).
unsafe impl Send for VulkanRenderer {}
// SAFETY: as for Send.
unsafe impl Sync for VulkanRenderer {}

impl VulkanRenderer {
    /// `SET_STRING_ERROR` with this renderer's debug mode.
    pub(super) fn set_string_error(&self, msg: &str) -> Error {
        set_string_error(self.debug_mode, msg)
    }

    /// The error of a failed Vulkan call, logged in debug mode.
    pub(super) fn vk_error(&self, res: VkResult, func: &str) -> Error {
        vulkan_error(self.debug_mode, res, func)
    }

    /// `CHECK_VULKAN_ERROR_AND_RETURN`: an error for a failed Vulkan call.
    pub(super) fn check(&self, res: VkResult, func: &str) -> Result<()> {
        if res != VK_SUCCESS {
            return Err(self.vk_error(res, func));
        }
        Ok(())
    }

    /// Translation of `VULKAN_Wait()`.
    fn wait_internal(&self) -> Result<()> {
        let _submit = lock(&self.submit_lock);

        // SAFETY: the device.
        let result = unsafe { (self.dev.device_wait_idle)(self.logical_device) };

        if result != VK_SUCCESS {
            return Err(self.vk_error(result, "vkDeviceWaitIdle"));
        }

        // part 2: clean every submitted command buffer
        // (VULKAN_INTERNAL_CleanCommandBuffer); none can be submitted yet.

        self.perform_pending_destroys();

        Ok(())
    }

    /// Whether a texture format can be made with a type and usage.
    /// Translation of `VULKAN_SupportsTextureFormat()`.
    ///
    /// FIXME (upstream): the ASTC HDR (`*_FLOAT`) formats are queried even
    /// when `VK_EXT_texture_compression_astc_hdr` isn't enabled, which the
    /// validation layers report as an invalid format.
    fn supports_texture_format_internal(
        &self,
        format: TextureFormat,
        texture_type: TextureType,
        usage: TextureUsageFlags,
    ) -> bool {
        let vulkan_format = sdl_to_vk_texture_format(format);
        let mut vulkan_usage = 0;
        let mut create_flags = 0;
        let mut properties = VkImageFormatProperties::default();

        if usage.contains(TextureUsageFlags::SAMPLER) {
            vulkan_usage |= VK_IMAGE_USAGE_SAMPLED_BIT;
        }
        if usage.contains(TextureUsageFlags::COLOR_TARGET) {
            vulkan_usage |= VK_IMAGE_USAGE_COLOR_ATTACHMENT_BIT;
        }
        if usage.contains(TextureUsageFlags::DEPTH_STENCIL_TARGET) {
            vulkan_usage |= VK_IMAGE_USAGE_DEPTH_STENCIL_ATTACHMENT_BIT;
        }
        if usage.intersects(
            TextureUsageFlags::GRAPHICS_STORAGE_READ
                | TextureUsageFlags::COMPUTE_STORAGE_READ
                | TextureUsageFlags::COMPUTE_STORAGE_WRITE
                | TextureUsageFlags::COMPUTE_STORAGE_SIMULTANEOUS_READ_WRITE,
        ) {
            vulkan_usage |= VK_IMAGE_USAGE_STORAGE_BIT;
        }

        if matches!(texture_type, TextureType::Cube | TextureType::CubeArray) {
            create_flags = VK_IMAGE_CREATE_CUBE_COMPATIBLE_BIT;
        }

        // SAFETY: the physical device, with a structure to fill.
        let vulkan_result = unsafe {
            (self.inst.get_physical_device_image_format_properties)(
                self.physical_device,
                vulkan_format,
                if texture_type == TextureType::Texture3D {
                    VK_IMAGE_TYPE_3D
                } else {
                    VK_IMAGE_TYPE_2D
                },
                VK_IMAGE_TILING_OPTIMAL,
                vulkan_usage,
                create_flags,
                &mut properties,
            )
        };

        vulkan_result == VK_SUCCESS
    }

    /// The command buffer of the front end's, for the debug labels.
    fn vulkan_command_buffer(
        command_buffer: &mut BackendCommandBuffer,
    ) -> &mut VulkanCommandBuffer {
        command_buffer
            .downcast_mut::<VulkanCommandBuffer>()
            .expect("not a Vulkan command buffer")
    }

    /// A debug label (`VkDebugUtilsLabelEXT`) with a name.
    fn with_label(text: &str, f: impl FnOnce(&VkDebugUtilsLabelEXT)) {
        let Ok(name) = std::ffi::CString::new(text) else {
            return;
        };
        let label_info = VkDebugUtilsLabelEXT {
            s_type: VK_STRUCTURE_TYPE_DEBUG_UTILS_LABEL_EXT,
            p_label_name: name.as_ptr(),
            ..Default::default()
        };
        f(&label_info);
    }
}

/// The backend object as one of the backend's types.
fn object<T: std::any::Any + Send + Sync>(object: &BackendObject) -> Arc<T> {
    object
        .downcast::<T>()
        .expect("a GPU object of another backend")
}

impl GpuDriver for VulkanRenderer {
    // Device

    fn destroy(&mut self) {
        self.destroy_device();
    }

    /// Translation of `VULKAN_GetDeviceProperties()`.
    fn properties(&self) -> Properties {
        self.props.clone()
    }

    // State Creation

    fn create_compute_pipeline(
        &self,
        createinfo: &ComputePipelineCreateInfo<'_>,
    ) -> Result<(BackendObject, ComputePipelineHeader)> {
        let (pipeline, header) = self.create_compute_pipeline_internal(createinfo)?;
        Ok((BackendObject(pipeline), header))
    }

    fn create_graphics_pipeline(
        &self,
        createinfo: &GraphicsPipelineCreateInfo<'_>,
    ) -> Result<(BackendObject, GraphicsPipelineHeader)> {
        let vertex_shader = object::<VulkanShader>(&createinfo.vertex_shader.raw);
        let fragment_shader = object::<VulkanShader>(&createinfo.fragment_shader.raw);
        let (pipeline, header) =
            self.create_graphics_pipeline_internal(createinfo, vertex_shader, fragment_shader)?;
        Ok((BackendObject(pipeline), header))
    }

    fn create_sampler(&self, createinfo: &SamplerCreateInfo) -> Result<BackendObject> {
        Ok(BackendObject(self.create_sampler_internal(createinfo)?))
    }

    fn create_shader(&self, createinfo: &ShaderCreateInfo<'_>) -> Result<BackendObject> {
        Ok(BackendObject(self.create_shader_internal(createinfo)?))
    }

    fn create_texture(&self, createinfo: &TextureCreateInfo) -> Result<BackendObject> {
        Ok(BackendObject(self.create_texture_container(createinfo)?))
    }

    /// Translation of `VULKAN_CreateBuffer()`.
    fn create_buffer(
        &self,
        usage_flags: BufferUsageFlags,
        size: u32,
        debug_name: Option<&str>,
    ) -> Result<BackendObject> {
        Ok(BackendObject(self.create_buffer_container(
            size as VkDeviceSize,
            usage_flags,
            VulkanBufferType::Gpu,
            false,
            debug_name,
        )?))
    }

    /// Translation of `VULKAN_CreateTransferBuffer()`.
    fn create_transfer_buffer(
        &self,
        _usage: TransferBufferUsage,
        size: u32,
        debug_name: Option<&str>,
    ) -> Result<BackendObject> {
        Ok(BackendObject(self.create_buffer_container(
            size as VkDeviceSize,
            BufferUsageFlags(0),
            VulkanBufferType::Transfer,
            true, // Dedicated allocations preserve the data even if a defrag is triggered.
            debug_name,
        )?))
    }

    // Debug Naming

    /// Translation of `VULKAN_SetBufferName()`.
    fn set_buffer_name(&self, buffer: &BackendObject, text: &str) {
        let container = object::<BufferContainer>(buffer);

        if self.debug_mode && self.supports_debug_utils {
            let buffers = {
                let mut state = lock(&container.state);
                state.debug_name = Some(text.to_owned());
                state.buffers.clone()
            };

            for buffer in buffers {
                // (VULKAN_INTERNAL_SetBufferName())
                self.set_object_name(VK_OBJECT_TYPE_BUFFER, buffer.buffer, text);
            }
        }
    }

    /// Translation of `VULKAN_SetTextureName()`.
    fn set_texture_name(&self, texture: &BackendObject, text: &str) {
        let container = object::<TextureContainer>(texture);

        if self.debug_mode && self.supports_debug_utils {
            let textures = {
                let mut state = lock(&container.state);
                state.debug_name = Some(text.to_owned());
                state.textures.clone()
            };

            for texture in textures {
                // (VULKAN_INTERNAL_SetTextureName())
                self.set_object_name(VK_OBJECT_TYPE_IMAGE, texture.image, text);
            }
        }
    }

    /// Translation of `VULKAN_InsertDebugLabel()`.
    fn insert_debug_label(&self, command_buffer: &mut BackendCommandBuffer, text: &str) {
        let vulkan_command_buffer = Self::vulkan_command_buffer(command_buffer);
        if let (true, Some(f)) = (
            self.supports_debug_utils,
            self.inst.cmd_insert_debug_utils_label_ext,
        ) {
            Self::with_label(text, |label_info| {
                // SAFETY: a command buffer being recorded and a label.
                unsafe { f(vulkan_command_buffer.command_buffer, label_info) }
            });
        }
    }

    /// Translation of `VULKAN_PushDebugGroup()`.
    fn push_debug_group(&self, command_buffer: &mut BackendCommandBuffer, name: &str) {
        let vulkan_command_buffer = Self::vulkan_command_buffer(command_buffer);
        if let (true, Some(f)) = (
            self.supports_debug_utils,
            self.inst.cmd_begin_debug_utils_label_ext,
        ) {
            Self::with_label(name, |label_info| {
                // SAFETY: a command buffer being recorded and a label.
                unsafe { f(vulkan_command_buffer.command_buffer, label_info) }
            });
        }
    }

    /// Translation of `VULKAN_PopDebugGroup()`.
    fn pop_debug_group(&self, command_buffer: &mut BackendCommandBuffer) {
        let vulkan_command_buffer = Self::vulkan_command_buffer(command_buffer);
        if let (true, Some(f)) = (
            self.supports_debug_utils,
            self.inst.cmd_end_debug_utils_label_ext,
        ) {
            // SAFETY: a command buffer being recorded.
            unsafe { f(vulkan_command_buffer.command_buffer) };
        }
    }

    // Disposal

    fn release_texture(&self, texture: &BackendObject) {
        self.release_texture_container(&object::<TextureContainer>(texture));
    }

    /// Translation of `VULKAN_ReleaseSampler()`.
    fn release_sampler(&self, sampler: &BackendObject) {
        let vulkan_sampler = object::<VulkanSampler>(sampler);
        lock(&self.dispose).samplers_to_destroy.push(vulkan_sampler);
    }

    /// Translation of `VULKAN_ReleaseBuffer()`.
    fn release_buffer(&self, buffer: &BackendObject) {
        self.release_buffer_container(&object::<BufferContainer>(buffer));
    }

    /// Translation of `VULKAN_ReleaseTransferBuffer()`.
    fn release_transfer_buffer(&self, transfer_buffer: &BackendObject) {
        self.release_buffer_container(&object::<BufferContainer>(transfer_buffer));
    }

    /// Translation of `VULKAN_ReleaseShader()`.
    fn release_shader(&self, shader: &BackendObject) {
        let vulkan_shader = object::<VulkanShader>(shader);
        lock(&self.dispose).shaders_to_destroy.push(vulkan_shader);
    }

    /// Translation of `VULKAN_ReleaseComputePipeline()`.
    fn release_compute_pipeline(&self, compute_pipeline: &BackendObject) {
        let pipeline = object::<VulkanComputePipeline>(compute_pipeline);
        lock(&self.dispose)
            .compute_pipelines_to_destroy
            .push(pipeline);
    }

    /// Translation of `VULKAN_ReleaseGraphicsPipeline()`.
    fn release_graphics_pipeline(&self, graphics_pipeline: &BackendObject) {
        let pipeline = object::<VulkanGraphicsPipeline>(graphics_pipeline);
        lock(&self.dispose)
            .graphics_pipelines_to_destroy
            .push(pipeline);
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

    /// Translation of `VULKAN_UnmapTransferBuffer()`.
    fn unmap_transfer_buffer(&self, _transfer_buffer: &BackendObject) {
        // no-op because transfer buffers are persistently mapped
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

    /// Translation of `VULKAN_SupportsSampleCount()`.
    fn supports_sample_count(&self, format: TextureFormat, sample_count: SampleCount) -> bool {
        let limits = &self.physical_device_properties.properties.limits;
        let bits = if format.is_depth_format() {
            limits.framebuffer_depth_sample_counts
        } else {
            limits.framebuffer_color_sample_counts
        };
        let vk_sample_count = sdl_to_vk_sample_count(sample_count);
        bits & vk_sample_count != 0
    }
}
