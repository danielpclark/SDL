// Rust translation of src/gpu/vulkan/SDL_gpu_vulkan.c from Simple
// DirectMedia Layer.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! The Vulkan GPU backend ("vulkan"): the GPU API on a Vulkan device. The
//! Vulkan loader is the video driver's (`SDL_Vulkan_LoadLibrary()`), and
//! every Vulkan function is looked up at run time through its
//! `vkGetInstanceProcAddr` ([`vkfuncs`]).
//!
//! * instance and physical device selection, the logical device, and the
//!   `SDL_GPUVulkanOptions` and device creation properties ([`device`]);
//! * the memory allocator: suballocation from large allocations per memory
//!   type, with free-region merging and the defragmentation of fragmented
//!   allocations ([`memory`], [`binding`],
//!   `VulkanRenderer::defragment_memory` in [`resources`]);
//! * buffers, transfer buffers (mapping, cycling), uniform buffers,
//!   textures (with their views, subresources and cycling), samplers and
//!   SPIR-V shaders, their release and the deferred destruction
//!   (`PerformPendingDestroys`), memory barriers and resource tracking
//!   ([`resources`]);
//! * descriptor set layouts, descriptor pools and caches, pipeline layouts,
//!   graphics pipelines (with their transient render passes) and compute
//!   pipelines ([`pipelines`]);
//! * command buffers and their per-thread pools, fences, uniform data,
//!   submission (with presentation, the cleanup of finished command
//!   buffers, the defragmentation and the freeing of empty allocations),
//!   cancelling and waiting ([`commands`]);
//! * render passes (with the render pass and framebuffer caches), the
//!   dynamic state, the bindings and their descriptor sets, draws
//!   (indirect ones too), compute passes and dispatches, uploads,
//!   downloads, copies, blits and mipmap generation ([`passes`]);
//! * claimed windows, their surfaces (made by the video driver: X11,
//!   Wayland, Windows, or the offscreen driver's headless surfaces) and
//!   swapchains, present modes, compositions, frames in flight and
//!   swapchain texture acquisition ([`swapchain`]);
//! * the format and usage tables ([`tables`]), `SupportsTextureFormat`,
//!   `SupportsSampleCount`, the device properties, the debug names and the
//!   debug labels.
//!
//! Blits and mipmaps use `vkCmdBlitImage`, as upstream's backend does, so
//! the front end's blit pipelines aren't needed here.
//!
//! The OpenXR parts (`HAVE_GPU_OPENXR`) are not translated.

mod binding;
mod commands;
mod device;
mod memory;
mod passes;
mod pipelines;
mod resources;
mod swapchain;
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

use commands::{VulkanFenceHandle, VulkanUniformBufferStage};
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

/// The Vulkan device. Translation of `VulkanRenderer`.
///
/// Upstream's locks guard the same state here: `allocatorLock` is the
/// recursive [`ReentrantMutex`] around the allocator (allocation re-enters
/// it during defrag), `disposeLock` the pending destroys, and the fetch
/// locks the caches; `acquireCommandBufferLock` the command pools,
/// `submitLock` the submitted command buffers, `windowLock` the claimed
/// windows, and the fence pool has its own.
struct VulkanRenderer {
    #[allow(dead_code)] // (kept as upstream's globals)
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
    allowed_frames_in_flight: AtomicU32,

    #[allow(dead_code)] // (read when the device is created)
    supports: VulkanExtensions,
    supports_debug_utils: bool,
    #[allow(dead_code)] // (read when the instance is created)
    supports_colorspace: bool,
    #[allow(dead_code)] // (kept as upstream does)
    supports_physical_device_properties2: bool,
    #[allow(dead_code)] // (kept as upstream does)
    supports_portability_enumeration: bool,
    supports_fill_mode_non_solid: bool,
    supports_multi_draw_indirect: bool,

    /// `memoryAllocator`, with `allocatorLock`.
    memory_allocator: ReentrantMutex<RefCell<MemoryAllocator>>,
    memory_properties: VkPhysicalDeviceMemoryProperties,

    /// `claimedWindows`, with `windowLock`.
    claimed_windows: Mutex<Vec<Arc<swapchain::WindowEntry>>>,

    queue_family_index: u32,
    unified_queue: VkQueue,

    /// `submittedCommandBuffers`, with `submitLock`.
    submit_lock: Mutex<Vec<VulkanCommandBuffer>>,

    /// `fencePool`, with its lock.
    fence_pool: Mutex<Vec<Arc<commands::VulkanFenceHandle>>>,

    /// `commandPoolHashTable`, with `acquireCommandBufferLock`.
    command_pools: Mutex<commands::CommandPoolHashTable>,
    /// The deferred resource destruction, with `disposeLock`.
    dispose: Mutex<PendingDestroys>,

    /// `renderPassHashTable` (with `renderPassFetchLock`).
    render_pass_hash_table: Mutex<HashMap<RenderPassHashTableKey, VkRenderPass>>,
    /// `framebufferHashTable` (with `framebufferFetchLock`).
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

    /// The command buffer of the front end's.
    fn vulkan_command_buffer(
        command_buffer: &mut BackendCommandBuffer,
    ) -> &mut VulkanCommandBuffer {
        command_buffer
            .downcast_mut::<VulkanCommandBuffer>()
            .expect("not a Vulkan command buffer")
    }

    /// A command buffer the front end gives back.
    fn owned_command_buffer(command_buffer: Box<BackendCommandBuffer>) -> VulkanCommandBuffer {
        *command_buffer
            .downcast::<VulkanCommandBuffer>()
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
        command_buffer: &mut BackendCommandBuffer,
        color_target_infos: &[ColorTargetInfo<'_>],
        depth_stencil_target_info: Option<&DepthStencilTargetInfo<'_>>,
    ) {
        self.begin_render_pass_internal(
            Self::vulkan_command_buffer(command_buffer),
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
            Self::vulkan_command_buffer(command_buffer),
            &object::<VulkanGraphicsPipeline>(graphics_pipeline),
        );
    }

    /// Translation of `VULKAN_SetViewport()`.
    fn set_viewport(&self, command_buffer: &mut BackendCommandBuffer, viewport: &Viewport) {
        self.set_current_viewport(Self::vulkan_command_buffer(command_buffer), viewport);
    }

    /// Translation of `VULKAN_SetScissor()`.
    fn set_scissor(&self, command_buffer: &mut BackendCommandBuffer, scissor: &Rect) {
        self.set_current_scissor(Self::vulkan_command_buffer(command_buffer), scissor);
    }

    /// Translation of `VULKAN_SetBlendConstants()`.
    fn set_blend_constants(
        &self,
        command_buffer: &mut BackendCommandBuffer,
        blend_constants: FColor,
    ) {
        self.set_current_blend_constants(
            Self::vulkan_command_buffer(command_buffer),
            blend_constants,
        );
    }

    /// Translation of `VULKAN_SetStencilReference()`.
    fn set_stencil_reference(&self, command_buffer: &mut BackendCommandBuffer, reference: u8) {
        self.set_current_stencil_reference(Self::vulkan_command_buffer(command_buffer), reference);
    }

    fn bind_vertex_buffers(
        &self,
        command_buffer: &mut BackendCommandBuffer,
        first_slot: u32,
        bindings: &[BufferBinding<'_>],
    ) {
        Self::bind_vertex_buffers_internal(
            Self::vulkan_command_buffer(command_buffer),
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
        self.bind_index_buffer_internal(
            Self::vulkan_command_buffer(command_buffer),
            binding,
            index_element_size,
        );
    }

    /// Translation of `VULKAN_BindVertexSamplers()`.
    fn bind_vertex_samplers(
        &self,
        command_buffer: &mut BackendCommandBuffer,
        first_slot: u32,
        texture_sampler_bindings: &[TextureSamplerBinding<'_>],
    ) {
        Self::bind_graphics_samplers(
            Self::vulkan_command_buffer(command_buffer),
            false,
            first_slot,
            texture_sampler_bindings,
        );
    }

    /// Translation of `VULKAN_BindVertexStorageTextures()`.
    fn bind_vertex_storage_textures(
        &self,
        command_buffer: &mut BackendCommandBuffer,
        first_slot: u32,
        storage_textures: &[&Texture],
    ) {
        Self::bind_graphics_storage_textures(
            Self::vulkan_command_buffer(command_buffer),
            false,
            first_slot,
            storage_textures,
        );
    }

    /// Translation of `VULKAN_BindVertexStorageBuffers()`.
    fn bind_vertex_storage_buffers(
        &self,
        command_buffer: &mut BackendCommandBuffer,
        first_slot: u32,
        storage_buffers: &[&super::Buffer],
    ) {
        Self::bind_graphics_storage_buffers(
            Self::vulkan_command_buffer(command_buffer),
            false,
            first_slot,
            storage_buffers,
        );
    }

    /// Translation of `VULKAN_BindFragmentSamplers()`.
    fn bind_fragment_samplers(
        &self,
        command_buffer: &mut BackendCommandBuffer,
        first_slot: u32,
        texture_sampler_bindings: &[TextureSamplerBinding<'_>],
    ) {
        Self::bind_graphics_samplers(
            Self::vulkan_command_buffer(command_buffer),
            true,
            first_slot,
            texture_sampler_bindings,
        );
    }

    /// Translation of `VULKAN_BindFragmentStorageTextures()`.
    fn bind_fragment_storage_textures(
        &self,
        command_buffer: &mut BackendCommandBuffer,
        first_slot: u32,
        storage_textures: &[&Texture],
    ) {
        Self::bind_graphics_storage_textures(
            Self::vulkan_command_buffer(command_buffer),
            true,
            first_slot,
            storage_textures,
        );
    }

    /// Translation of `VULKAN_BindFragmentStorageBuffers()`.
    fn bind_fragment_storage_buffers(
        &self,
        command_buffer: &mut BackendCommandBuffer,
        first_slot: u32,
        storage_buffers: &[&super::Buffer],
    ) {
        Self::bind_graphics_storage_buffers(
            Self::vulkan_command_buffer(command_buffer),
            true,
            first_slot,
            storage_buffers,
        );
    }

    /// Translation of `VULKAN_PushVertexUniformData()`.
    fn push_vertex_uniform_data(
        &self,
        command_buffer: &mut BackendCommandBuffer,
        slot_index: u32,
        data: &[u8],
    ) {
        self.push_uniform_data(
            Self::vulkan_command_buffer(command_buffer),
            VulkanUniformBufferStage::Vertex,
            slot_index,
            data,
        );
    }

    /// Translation of `VULKAN_PushFragmentUniformData()`.
    fn push_fragment_uniform_data(
        &self,
        command_buffer: &mut BackendCommandBuffer,
        slot_index: u32,
        data: &[u8],
    ) {
        self.push_uniform_data(
            Self::vulkan_command_buffer(command_buffer),
            VulkanUniformBufferStage::Fragment,
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
            Self::vulkan_command_buffer(command_buffer),
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
            Self::vulkan_command_buffer(command_buffer),
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
            Self::vulkan_command_buffer(command_buffer),
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
            Self::vulkan_command_buffer(command_buffer),
            &object::<BufferContainer>(buffer),
            offset,
            draw_count,
            true,
        );
    }

    fn end_render_pass(&self, command_buffer: &mut BackendCommandBuffer) {
        self.end_render_pass_internal(Self::vulkan_command_buffer(command_buffer));
    }

    // Compute Pass

    fn begin_compute_pass(
        &self,
        command_buffer: &mut BackendCommandBuffer,
        storage_texture_bindings: &[StorageTextureReadWriteBinding<'_>],
        storage_buffer_bindings: &[StorageBufferReadWriteBinding<'_>],
    ) {
        self.begin_compute_pass_internal(
            Self::vulkan_command_buffer(command_buffer),
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
            Self::vulkan_command_buffer(command_buffer),
            &object::<VulkanComputePipeline>(compute_pipeline),
        );
    }

    fn bind_compute_samplers(
        &self,
        command_buffer: &mut BackendCommandBuffer,
        first_slot: u32,
        texture_sampler_bindings: &[TextureSamplerBinding<'_>],
    ) {
        Self::bind_compute_samplers_internal(
            Self::vulkan_command_buffer(command_buffer),
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
        self.bind_compute_storage_textures_internal(
            Self::vulkan_command_buffer(command_buffer),
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
        self.bind_compute_storage_buffers_internal(
            Self::vulkan_command_buffer(command_buffer),
            first_slot,
            storage_buffers,
        );
    }

    /// Translation of `VULKAN_PushComputeUniformData()`.
    fn push_compute_uniform_data(
        &self,
        command_buffer: &mut BackendCommandBuffer,
        slot_index: u32,
        data: &[u8],
    ) {
        self.push_uniform_data(
            Self::vulkan_command_buffer(command_buffer),
            VulkanUniformBufferStage::Compute,
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
            Self::vulkan_command_buffer(command_buffer),
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
            Self::vulkan_command_buffer(command_buffer),
            &object::<BufferContainer>(buffer),
            offset,
        );
    }

    fn end_compute_pass(&self, command_buffer: &mut BackendCommandBuffer) {
        self.end_compute_pass_internal(Self::vulkan_command_buffer(command_buffer));
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

    /// Translation of `VULKAN_BeginCopyPass()`.
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
            Self::vulkan_command_buffer(command_buffer),
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
            Self::vulkan_command_buffer(command_buffer),
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
            Self::vulkan_command_buffer(command_buffer),
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
            Self::vulkan_command_buffer(command_buffer),
            source,
            destination,
            size,
            cycle,
        );
    }

    fn generate_mipmaps(&self, command_buffer: &mut CommandBuffer, texture: &Texture) {
        if let Some(vulkan_command_buffer) = command_buffer.backend_mut::<VulkanCommandBuffer>() {
            self.generate_mipmaps_internal(vulkan_command_buffer, texture);
        }
    }

    fn download_from_texture(
        &self,
        command_buffer: &mut BackendCommandBuffer,
        source: &TextureRegion<'_>,
        destination: &TextureTransferInfo<'_>,
    ) {
        self.download_from_texture_internal(
            Self::vulkan_command_buffer(command_buffer),
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
            Self::vulkan_command_buffer(command_buffer),
            source,
            destination,
        );
    }

    /// Translation of `VULKAN_EndCopyPass()`.
    fn end_copy_pass(&self, _command_buffer: &mut BackendCommandBuffer) {
        // no-op
    }

    fn blit(&self, command_buffer: &mut CommandBuffer, info: &BlitInfo<'_>) {
        if let Some(vulkan_command_buffer) = command_buffer.backend_mut::<VulkanCommandBuffer>() {
            self.blit_internal(vulkan_command_buffer, info);
        }
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

    /// Translation of `VULKAN_AcquireSwapchainTexture()`.
    fn acquire_swapchain_texture(
        &self,
        command_buffer: &mut BackendCommandBuffer,
        window: Window,
    ) -> Result<Option<BackendSwapchainTexture>> {
        self.acquire_swapchain_texture_internal(
            false,
            Self::vulkan_command_buffer(command_buffer),
            window,
        )
    }

    fn wait_for_swapchain(&self, window: Window) -> Result<()> {
        self.wait_for_swapchain_internal(window)
    }

    /// Translation of `VULKAN_WaitAndAcquireSwapchainTexture()`.
    fn wait_and_acquire_swapchain_texture(
        &self,
        command_buffer: &mut BackendCommandBuffer,
        window: Window,
    ) -> Result<Option<BackendSwapchainTexture>> {
        self.acquire_swapchain_texture_internal(
            true,
            Self::vulkan_command_buffer(command_buffer),
            window,
        )
    }

    fn submit(&self, command_buffer: Box<BackendCommandBuffer>) -> Result<()> {
        self.submit_internal(Self::owned_command_buffer(command_buffer))?;
        Ok(())
    }

    /// Translation of `VULKAN_SubmitAndAcquireFence()`.
    fn submit_and_acquire_fence(
        &self,
        command_buffer: Box<BackendCommandBuffer>,
    ) -> Result<BackendObject> {
        let mut vulkan_command_buffer = Self::owned_command_buffer(command_buffer);
        vulkan_command_buffer.auto_release_fence = false;
        let fence = self.submit_internal(vulkan_command_buffer)?;
        Ok(BackendObject(fence))
    }

    fn cancel(&self, command_buffer: Box<BackendCommandBuffer>) -> Result<()> {
        self.cancel_internal(Self::owned_command_buffer(command_buffer))
    }

    fn wait(&self) -> Result<()> {
        self.wait_internal()
    }

    fn wait_for_fences(&self, wait_all: bool, fences: &[&BackendObject]) -> Result<()> {
        let fences: Vec<Arc<VulkanFenceHandle>> = fences
            .iter()
            .map(|f| object::<VulkanFenceHandle>(f))
            .collect();
        let fences: Vec<&VulkanFenceHandle> = fences.iter().map(|f| &**f).collect();
        self.wait_for_fences_internal(wait_all, &fences)
    }

    fn query_fence(&self, fence: &BackendObject) -> bool {
        self.query_fence_internal(&object::<VulkanFenceHandle>(fence))
    }

    fn release_fence(&self, fence: &BackendObject) {
        self.release_fence_internal(&object::<VulkanFenceHandle>(fence));
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
