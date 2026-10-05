// Rust translation of the resource parts of src/gpu/vulkan/SDL_gpu_vulkan.c
// from Simple DirectMedia Layer: buffers, transfer buffers, textures,
// samplers, their containers, memory barriers, resource tracking, release
// and destruction, and memory defragmentation.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! Buffers and textures. The front end's handles are containers
//! ([`BufferContainer`], [`TextureContainer`]) whose active buffer or
//! texture can be cycled to a fresh one while the GPU still uses the old;
//! the [`VulkanBuffer`]s and [`VulkanTexture`]s are what command buffers
//! track and the allocator's regions point back at.
//!
//! Upstream's objects are reference counted by hand (`referenceCount`,
//! incremented when a command buffer tracks the object) and destroyed in
//! `VULKAN_INTERNAL_PerformPendingDestroys()` once released and unused;
//! that is kept. The objects are also `Arc`s, which only keep the Rust
//! values alive: the Vulkan objects go when they are destroyed, as
//! upstream's do. A buffer's or texture's container is a `Weak` back
//! reference with its index, as upstream's `container` and
//! `containerIndex`.

use std::collections::HashMap;
use std::ffi::CString;
use std::ptr::{null, NonNull};
use std::sync::atomic::{AtomicBool, AtomicI32, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, Weak};
use std::thread::ThreadId;

use super::commands::VulkanFenceHandle;
use super::memory::{RegionResource, UsedRegion};
use super::pipelines::{
    DescriptorSetCache, FramebufferHashTableKey, VulkanComputePipeline, VulkanFramebuffer,
    VulkanGraphicsPipeline, VulkanShader,
};
use super::swapchain::WindowEntry;
use super::tables::{sdl_to_vk_sample_count, sdl_to_vk_texture_format, swizzle_for_sdl_format};
use super::{lock, VulkanRenderer};
use crate::error::Result;
use crate::gpu::sysgpu::{
    MAX_COMPUTE_WRITE_BUFFERS, MAX_COMPUTE_WRITE_TEXTURES, MAX_STORAGE_BUFFERS_PER_STAGE,
    MAX_STORAGE_TEXTURES_PER_STAGE, MAX_TEXTURE_SAMPLERS_PER_STAGE, MAX_UNIFORM_BUFFERS_PER_STAGE,
    MAX_VERTEX_BUFFERS,
};
use crate::gpu::{BufferUsageFlags, TextureCreateInfo, TextureType, TextureUsageFlags};
use crate::log::Category;
use crate::properties::Properties;
use crate::video::vk::*;

// Memory structures

/// What a buffer is for. Translation of `VulkanBufferType`.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum VulkanBufferType {
    Gpu,
    Uniform,
    Transfer,
}

/// A buffer's or texture's place in its container (`container`,
/// `containerIndex`).
#[derive(Debug)]
pub(super) struct ContainerRef<C> {
    pub(super) container: Weak<C>,
    pub(super) index: usize,
}

/// Translation of `VulkanBuffer`.
#[derive(Debug)]
pub(super) struct VulkanBuffer {
    pub(super) container: Mutex<Option<ContainerRef<BufferContainer>>>,

    pub(super) buffer: VkBuffer,
    pub(super) used_region: Arc<UsedRegion>,

    // Needed for uniforms and defrag
    pub(super) ty: VulkanBufferType,
    pub(super) usage: BufferUsageFlags,
    pub(super) size: VkDeviceSize,

    pub(super) reference_count: AtomicI32,
    pub(super) transitioned: AtomicBool,
    /// so that defrag doesn't double-free
    pub(super) marked_for_destroy: AtomicBool,
    pub(super) uniform_buffer_for_defrag: Mutex<Weak<UniformBuffer>>,
}

/// The front end's buffer and transfer buffer handles. Translation of
/// `VulkanBufferContainer`.
#[derive(Debug)]
pub(super) struct BufferContainer {
    pub(super) state: Mutex<BufferContainerState>,
    pub(super) dedicated: bool,
}

/// The cycled buffers of a [`BufferContainer`].
#[derive(Debug)]
pub(super) struct BufferContainerState {
    pub(super) active_buffer: Arc<VulkanBuffer>,
    pub(super) buffers: Vec<Arc<VulkanBuffer>>,
    pub(super) debug_name: Option<String>,
}

/// Translation of `VulkanUniformBuffer`.
#[derive(Debug)]
pub(super) struct UniformBuffer {
    pub(super) state: Mutex<UniformBufferState>,
}

/// The buffer and offsets of a [`UniformBuffer`].
#[derive(Debug)]
pub(super) struct UniformBufferState {
    pub(super) buffer: Arc<VulkanBuffer>,
    pub(super) draw_offset: u32,
    pub(super) write_offset: u32,
}

/// Translation of `VulkanSampler`.
#[derive(Debug)]
pub(super) struct VulkanSampler {
    pub(super) sampler: VkSampler,
    pub(super) reference_count: AtomicI32,
}

/// Textures are made up of individual subresources.
/// This helps us barrier the resource efficiently.
/// Translation of `VulkanTextureSubresource` (its `parent` is the texture
/// that holds it).
#[derive(Debug, Default)]
pub(super) struct TextureSubresource {
    pub(super) layer: u32,
    pub(super) level: u32,

    /// One render target view per depth slice
    pub(super) render_target_views: Vec<VkImageView>,
    pub(super) compute_write_view: VkImageView,
    pub(super) depth_stencil_view: VkImageView,
}

/// Translation of `VulkanTexture`.
#[derive(Debug)]
pub(super) struct VulkanTexture {
    pub(super) container: Mutex<Option<ContainerRef<TextureContainer>>>,

    /// `None` for textures our allocator doesn't manage (the swapchain's).
    pub(super) used_region: Option<Arc<UsedRegion>>,

    pub(super) image: VkImage,
    /// used for samplers and storage reads
    pub(super) full_view: VkImageView,
    pub(super) swizzle: VkComponentMapping,
    pub(super) aspect_flags: VkImageAspectFlags,
    /// used for cleanup only
    #[allow(dead_code)] // (the views are a Vec of this length here)
    pub(super) depth: u32,

    // used to avoid indirection on barriers
    pub(super) level_count: u32,
    pub(super) layer_count: u32,
    pub(super) ty: TextureType,

    // FIXME: It'd be nice if we didn't have to have this on the texture...
    /// used for defrag transitions only.
    pub(super) usage: TextureUsageFlags,

    pub(super) subresources: Vec<TextureSubresource>,

    /// so that defrag doesn't double-free
    pub(super) marked_for_destroy: AtomicBool,
    /// true for XR swapchain images
    pub(super) externally_managed: bool,
    pub(super) reference_count: AtomicI32,
}

/// The front end's texture handles. Translation of
/// `VulkanTextureContainer`.
#[derive(Debug)]
pub(super) struct TextureContainer {
    /// The creation info (`header.info`), with its own copy of the
    /// properties.
    pub(super) info: TextureCreateInfo,
    pub(super) state: Mutex<TextureContainerState>,
    pub(super) can_be_cycled: bool,
    /// true for XR swapchain images
    #[allow(dead_code)] // (OpenXR isn't translated)
    pub(super) externally_managed: bool,
}

/// The cycled textures of a [`TextureContainer`].
#[derive(Debug)]
pub(super) struct TextureContainerState {
    pub(super) active_texture: Arc<VulkanTexture>,
    pub(super) textures: Vec<Arc<VulkanTexture>>,
    pub(super) debug_name: Option<String>,
}

// SAFETY: the Vulkan handles are plain values; the objects they name are
// used under the renderer's locks, as upstream uses them.
unsafe impl Send for VulkanTexture {}
// SAFETY: as for Send.
unsafe impl Sync for VulkanTexture {}

/// A buffer usage mode (`VulkanBufferUsageModeFlags`).
pub(super) type VulkanBufferUsageModeFlags = u32;

pub(super) const VULKAN_BUFFER_USAGE_MODE_COPY_SOURCE: VulkanBufferUsageModeFlags = 1 << 0;
pub(super) const VULKAN_BUFFER_USAGE_MODE_COPY_DESTINATION: VulkanBufferUsageModeFlags = 1 << 1;
pub(super) const VULKAN_BUFFER_USAGE_MODE_VERTEX_READ: VulkanBufferUsageModeFlags = 1 << 2;
pub(super) const VULKAN_BUFFER_USAGE_MODE_INDEX_READ: VulkanBufferUsageModeFlags = 1 << 3;
pub(super) const VULKAN_BUFFER_USAGE_MODE_INDIRECT: VulkanBufferUsageModeFlags = 1 << 4;
pub(super) const VULKAN_BUFFER_USAGE_MODE_GRAPHICS_STORAGE_READ: VulkanBufferUsageModeFlags =
    1 << 5;
pub(super) const VULKAN_BUFFER_USAGE_MODE_COMPUTE_STORAGE_READ: VulkanBufferUsageModeFlags = 1 << 6;
pub(super) const VULKAN_BUFFER_USAGE_MODE_COMPUTE_STORAGE_READ_WRITE: VulkanBufferUsageModeFlags =
    1 << 7;

/// Translation of `VulkanTextureUsageMode`.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum VulkanTextureUsageMode {
    Uninitialized,
    CopySource,
    CopyDestination,
    Sampler,
    GraphicsStorageRead,
    ComputeStorageRead,
    ComputeStorageReadWrite,
    ColorAttachment,
    DepthStencilAttachment,
    Present,
}

/// A subresource of a texture (upstream's `VulkanTextureSubresource *`,
/// whose `parent` is the texture): the texture and the subresource's index.
#[derive(Clone, Debug)]
pub(super) struct SubresourceRef {
    pub(super) texture: Arc<VulkanTexture>,
    pub(super) index: usize,
}

impl SubresourceRef {
    /// The subresource.
    pub(super) fn get(&self) -> &TextureSubresource {
        &self.texture.subresources[self.index]
    }
}

/// A swapchain image a command buffer presents. Translation of
/// `VulkanPresentData`.
#[derive(Debug)]
pub(super) struct PresentData {
    pub(super) window_data: Arc<WindowEntry>,
    pub(super) swapchain_image_index: u32,
}

/// Translation of `VulkanCommandBuffer` (its `renderer` is the renderer the
/// driver calls are made on, and its `commandPool` the key of its pool).
pub(super) struct VulkanCommandBuffer {
    pub(super) command_buffer: VkCommandBuffer,
    /// The thread whose command pool the command buffer is from.
    pub(super) command_pool: Option<ThreadId>,

    pub(super) present_datas: Vec<PresentData>,
    pub(super) wait_semaphores: Vec<VkSemaphore>,
    pub(super) signal_semaphores: Vec<VkSemaphore>,

    pub(super) current_compute_pipeline: Option<Arc<VulkanComputePipeline>>,
    pub(super) current_graphics_pipeline: Option<Arc<VulkanGraphicsPipeline>>,

    // Keep track of resources transitioned away from their default state to barrier them on pass end
    pub(super) color_attachment_subresources: Vec<SubresourceRef>,
    pub(super) resolve_attachment_subresources: Vec<SubresourceRef>,

    /// may be NULL
    pub(super) depth_stencil_attachment_subresource: Option<SubresourceRef>,

    // Dynamic state
    pub(super) current_viewport: VkViewport,
    pub(super) current_scissor: VkRect2D,
    pub(super) blend_constants: [f32; 4],
    pub(super) stencil_ref: u8,

    // Resource bind state
    /// acquired when command buffer is acquired
    pub(super) descriptor_set_cache: Option<DescriptorSetCache>,

    pub(super) need_new_vertex_resource_descriptor_set: bool,
    pub(super) need_new_vertex_uniform_descriptor_set: bool,
    pub(super) need_new_vertex_uniform_offsets: bool,
    pub(super) need_new_fragment_resource_descriptor_set: bool,
    pub(super) need_new_fragment_uniform_descriptor_set: bool,
    pub(super) need_new_fragment_uniform_offsets: bool,

    pub(super) need_new_compute_read_only_descriptor_set: bool,
    pub(super) need_new_compute_read_write_descriptor_set: bool,
    pub(super) need_new_compute_uniform_descriptor_set: bool,
    pub(super) need_new_compute_uniform_offsets: bool,

    pub(super) vertex_resource_descriptor_set: VkDescriptorSet,
    pub(super) vertex_uniform_descriptor_set: VkDescriptorSet,
    pub(super) fragment_resource_descriptor_set: VkDescriptorSet,
    pub(super) fragment_uniform_descriptor_set: VkDescriptorSet,

    pub(super) compute_read_only_descriptor_set: VkDescriptorSet,
    pub(super) compute_read_write_descriptor_set: VkDescriptorSet,
    pub(super) compute_uniform_descriptor_set: VkDescriptorSet,

    pub(super) vertex_buffers: [VkBuffer; MAX_VERTEX_BUFFERS as usize],
    pub(super) vertex_buffer_offsets: [VkDeviceSize; MAX_VERTEX_BUFFERS as usize],
    pub(super) vertex_buffer_count: u32,
    pub(super) need_vertex_buffer_bind: bool,

    pub(super) vertex_sampler_texture_view_bindings: [VkImageView; SAMPLERS],
    pub(super) vertex_sampler_bindings: [VkSampler; SAMPLERS],
    pub(super) vertex_storage_texture_view_bindings: [VkImageView; STORAGE_TEXTURES],
    pub(super) vertex_storage_buffer_bindings: [VkBuffer; STORAGE_BUFFERS],

    pub(super) fragment_sampler_texture_view_bindings: [VkImageView; SAMPLERS],
    pub(super) fragment_sampler_bindings: [VkSampler; SAMPLERS],
    pub(super) fragment_storage_texture_view_bindings: [VkImageView; STORAGE_TEXTURES],
    pub(super) fragment_storage_buffer_bindings: [VkBuffer; STORAGE_BUFFERS],

    pub(super) compute_sampler_texture_view_bindings: [VkImageView; SAMPLERS],
    pub(super) compute_sampler_bindings: [VkSampler; SAMPLERS],
    pub(super) read_only_compute_storage_texture_view_bindings: [VkImageView; STORAGE_TEXTURES],
    pub(super) read_only_compute_storage_buffer_bindings: [VkBuffer; STORAGE_BUFFERS],

    // Track these separately because barriers can happen mid compute pass
    pub(super) read_only_compute_storage_textures: [Option<Arc<VulkanTexture>>; STORAGE_TEXTURES],
    pub(super) read_only_compute_storage_buffers: [Option<Arc<VulkanBuffer>>; STORAGE_BUFFERS],

    pub(super) read_write_compute_storage_texture_view_bindings:
        [VkImageView; MAX_COMPUTE_WRITE_TEXTURES as usize],
    pub(super) read_write_compute_storage_buffer_bindings:
        [VkBuffer; MAX_COMPUTE_WRITE_BUFFERS as usize],

    // Track these separately because they are barriered when the compute pass begins
    pub(super) read_write_compute_storage_texture_subresources: Vec<SubresourceRef>,
    pub(super) read_write_compute_storage_buffers:
        [Option<Arc<VulkanBuffer>>; MAX_COMPUTE_WRITE_BUFFERS as usize],

    // Uniform buffers
    pub(super) vertex_uniform_buffers: [Option<Arc<UniformBuffer>>; UNIFORM_BUFFERS],
    pub(super) fragment_uniform_buffers: [Option<Arc<UniformBuffer>>; UNIFORM_BUFFERS],
    pub(super) compute_uniform_buffers: [Option<Arc<UniformBuffer>>; UNIFORM_BUFFERS],

    // Track used resources
    pub(super) used_buffers: Vec<Arc<VulkanBuffer>>,
    pub(super) buffers_used_in_pending_transfers: Vec<Arc<VulkanBuffer>>,
    pub(super) used_textures: Vec<Arc<VulkanTexture>>,
    pub(super) textures_used_in_pending_transfers: Vec<Arc<VulkanTexture>>,
    pub(super) used_samplers: Vec<Arc<VulkanSampler>>,
    pub(super) used_graphics_pipelines: Vec<Arc<VulkanGraphicsPipeline>>,
    pub(super) used_compute_pipelines: Vec<Arc<VulkanComputePipeline>>,
    pub(super) used_framebuffers: Vec<Arc<VulkanFramebuffer>>,
    pub(super) used_uniform_buffers: Vec<Arc<UniformBuffer>>,

    pub(super) in_flight_fence: Option<Arc<VulkanFenceHandle>>,
    pub(super) auto_release_fence: bool,

    pub(super) swapchain_requested: bool,
    /// Whether this CB was created for defragging
    pub(super) is_defrag: bool,
}

impl std::fmt::Debug for VulkanCommandBuffer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("VulkanCommandBuffer")
            .field("command_buffer", &self.command_buffer)
            .field("command_pool", &self.command_pool)
            .field("is_defrag", &self.is_defrag)
            .finish_non_exhaustive()
    }
}

const SAMPLERS: usize = MAX_TEXTURE_SAMPLERS_PER_STAGE as usize;
const STORAGE_TEXTURES: usize = MAX_STORAGE_TEXTURES_PER_STAGE as usize;
const STORAGE_BUFFERS: usize = MAX_STORAGE_BUFFERS_PER_STAGE as usize;
const UNIFORM_BUFFERS: usize = MAX_UNIFORM_BUFFERS_PER_STAGE as usize;

impl VulkanCommandBuffer {
    /// A command buffer recording into `command_buffer`, tracking nothing:
    /// the state `VULKAN_INTERNAL_AllocateCommandBuffer()` gives it.
    pub(super) fn new(command_buffer: VkCommandBuffer) -> VulkanCommandBuffer {
        VulkanCommandBuffer {
            command_buffer,
            command_pool: None,
            // Presentation tracking
            present_datas: Vec::with_capacity(1),
            wait_semaphores: Vec::with_capacity(1),
            signal_semaphores: Vec::with_capacity(1),
            current_compute_pipeline: None,
            current_graphics_pipeline: None,
            color_attachment_subresources: Vec::new(),
            resolve_attachment_subresources: Vec::new(),
            depth_stencil_attachment_subresource: None,
            current_viewport: VkViewport::default(),
            current_scissor: VkRect2D::default(),
            blend_constants: [0.0; 4],
            stencil_ref: 0,
            descriptor_set_cache: None,
            // Resource bind tracking
            need_new_vertex_resource_descriptor_set: true,
            need_new_vertex_uniform_descriptor_set: true,
            need_new_vertex_uniform_offsets: true,
            need_new_fragment_resource_descriptor_set: true,
            need_new_fragment_uniform_descriptor_set: true,
            need_new_fragment_uniform_offsets: true,
            need_new_compute_read_only_descriptor_set: true,
            need_new_compute_read_write_descriptor_set: true,
            need_new_compute_uniform_descriptor_set: true,
            need_new_compute_uniform_offsets: true,
            vertex_resource_descriptor_set: VK_NULL_HANDLE,
            vertex_uniform_descriptor_set: VK_NULL_HANDLE,
            fragment_resource_descriptor_set: VK_NULL_HANDLE,
            fragment_uniform_descriptor_set: VK_NULL_HANDLE,
            compute_read_only_descriptor_set: VK_NULL_HANDLE,
            compute_read_write_descriptor_set: VK_NULL_HANDLE,
            compute_uniform_descriptor_set: VK_NULL_HANDLE,
            vertex_buffers: Default::default(),
            vertex_buffer_offsets: Default::default(),
            vertex_buffer_count: 0,
            need_vertex_buffer_bind: false,
            vertex_sampler_texture_view_bindings: Default::default(),
            vertex_sampler_bindings: Default::default(),
            vertex_storage_texture_view_bindings: Default::default(),
            vertex_storage_buffer_bindings: Default::default(),
            fragment_sampler_texture_view_bindings: Default::default(),
            fragment_sampler_bindings: Default::default(),
            fragment_storage_texture_view_bindings: Default::default(),
            fragment_storage_buffer_bindings: Default::default(),
            compute_sampler_texture_view_bindings: Default::default(),
            compute_sampler_bindings: Default::default(),
            read_only_compute_storage_texture_view_bindings: Default::default(),
            read_only_compute_storage_buffer_bindings: Default::default(),
            read_only_compute_storage_textures: Default::default(),
            read_only_compute_storage_buffers: Default::default(),
            read_write_compute_storage_texture_view_bindings: Default::default(),
            read_write_compute_storage_buffer_bindings: Default::default(),
            read_write_compute_storage_texture_subresources: Vec::new(),
            read_write_compute_storage_buffers: Default::default(),
            vertex_uniform_buffers: Default::default(),
            fragment_uniform_buffers: Default::default(),
            compute_uniform_buffers: Default::default(),
            // Resource tracking
            used_buffers: Vec::with_capacity(4),
            buffers_used_in_pending_transfers: Vec::with_capacity(4),
            used_textures: Vec::with_capacity(4),
            textures_used_in_pending_transfers: Vec::with_capacity(4),
            used_samplers: Vec::with_capacity(4),
            used_graphics_pipelines: Vec::with_capacity(4),
            used_compute_pipelines: Vec::with_capacity(4),
            used_framebuffers: Vec::with_capacity(4),
            used_uniform_buffers: Vec::with_capacity(4),
            in_flight_fence: None,
            auto_release_fence: false,
            swapchain_requested: false,
            is_defrag: false,
        }
    }
}

// SAFETY: a command buffer is recorded by one thread at a time (it is
// owned by the front end's command buffer, or by the submitting thread).
unsafe impl Send for VulkanCommandBuffer {}

/// The resources released but not destroyed yet (`texturesToDestroy` and
/// the other lists `disposeLock` guards).
#[derive(Debug, Default)]
pub(super) struct PendingDestroys {
    pub(super) textures_to_destroy: Vec<Arc<VulkanTexture>>,
    pub(super) buffers_to_destroy: Vec<Arc<VulkanBuffer>>,
    pub(super) samplers_to_destroy: Vec<Arc<VulkanSampler>>,
    pub(super) graphics_pipelines_to_destroy: Vec<Arc<VulkanGraphicsPipeline>>,
    pub(super) compute_pipelines_to_destroy: Vec<Arc<VulkanComputePipeline>>,
    pub(super) shaders_to_destroy: Vec<Arc<VulkanShader>>,
    pub(super) framebuffers_to_destroy: Vec<Arc<VulkanFramebuffer>>,
}

// Resource tracking

/// Track a resource in a list unless it's there already, taking a
/// reference. Translation of `TRACK_RESOURCE`.
fn track_resource<T>(list: &mut Vec<Arc<T>>, resource: &Arc<T>, reference_count: &AtomicI32) {
    if list.iter().rev().any(|r| Arc::ptr_eq(r, resource)) {
        return;
    }

    list.push(resource.clone());
    reference_count.fetch_add(1, Ordering::SeqCst);
}

impl VulkanCommandBuffer {
    /// Translation of `VULKAN_INTERNAL_TrackBuffer()`.
    pub(super) fn track_buffer(&mut self, buffer: &Arc<VulkanBuffer>) {
        track_resource(&mut self.used_buffers, buffer, &buffer.reference_count);
    }

    /// Use this function when a GPU buffer is part of a transfer operation.
    /// Note that this isn't for transfer buffers, those don't need to
    /// refcount their allocations. Translation of
    /// `VULKAN_INTERNAL_TrackBufferTransfer()`.
    pub(super) fn track_buffer_transfer(&mut self, buffer: &Arc<VulkanBuffer>) {
        track_resource(
            &mut self.buffers_used_in_pending_transfers,
            buffer,
            &buffer.used_region.allocation.reference_count,
        );
    }

    /// Translation of `VULKAN_INTERNAL_TrackTexture()`.
    pub(super) fn track_texture(&mut self, texture: &Arc<VulkanTexture>) {
        track_resource(&mut self.used_textures, texture, &texture.reference_count);
    }

    /// Use this when a texture is part of a transfer operation.
    /// Translation of `VULKAN_INTERNAL_TrackTextureTransfer()`.
    pub(super) fn track_texture_transfer(&mut self, texture: &Arc<VulkanTexture>) {
        // Textures not managed by our allocator (i.e. the swapchain) don't need to be refcounted.
        let Some(used_region) = &texture.used_region else {
            return;
        };

        track_resource(
            &mut self.textures_used_in_pending_transfers,
            texture,
            &used_region.allocation.reference_count,
        );
    }

    /// Translation of `VULKAN_INTERNAL_TrackSampler()`.
    pub(super) fn track_sampler(&mut self, sampler: &Arc<VulkanSampler>) {
        track_resource(&mut self.used_samplers, sampler, &sampler.reference_count);
    }

    /// Translation of `VULKAN_INTERNAL_TrackGraphicsPipeline()`.
    pub(super) fn track_graphics_pipeline(&mut self, pipeline: &Arc<VulkanGraphicsPipeline>) {
        track_resource(
            &mut self.used_graphics_pipelines,
            pipeline,
            &pipeline.reference_count,
        );
    }

    /// Translation of `VULKAN_INTERNAL_TrackComputePipeline()`.
    pub(super) fn track_compute_pipeline(&mut self, pipeline: &Arc<VulkanComputePipeline>) {
        track_resource(
            &mut self.used_compute_pipelines,
            pipeline,
            &pipeline.reference_count,
        );
    }

    /// Translation of `VULKAN_INTERNAL_TrackFramebuffer()`.
    pub(super) fn track_framebuffer(&mut self, framebuffer: &Arc<VulkanFramebuffer>) {
        track_resource(
            &mut self.used_framebuffers,
            framebuffer,
            &framebuffer.reference_count,
        );
    }

    /// Translation of `VULKAN_INTERNAL_TrackUniformBuffer()`.
    pub(super) fn track_uniform_buffer(&mut self, uniform_buffer: &Arc<UniformBuffer>) {
        if self
            .used_uniform_buffers
            .iter()
            .rev()
            .any(|u| Arc::ptr_eq(u, uniform_buffer))
        {
            return;
        }

        self.used_uniform_buffers.push(uniform_buffer.clone());

        let buffer = lock(&uniform_buffer.state).buffer.clone();
        self.track_buffer(&buffer);
    }
}

/// The stages and accesses of buffer usage modes. Translation of
/// `VULKAN_INTERNAL_SetMemoryBarrierFlags()`.
fn set_memory_barrier_flags(
    usage_mode_flags: VulkanBufferUsageModeFlags,
    stage_flags: &mut VkPipelineStageFlags,
    access_mask: &mut VkAccessFlags,
) {
    // Combinable read flags
    if usage_mode_flags & VULKAN_BUFFER_USAGE_MODE_VERTEX_READ != 0 {
        *stage_flags |= VK_PIPELINE_STAGE_VERTEX_INPUT_BIT;
        *access_mask |= VK_ACCESS_VERTEX_ATTRIBUTE_READ_BIT;
    }

    if usage_mode_flags & VULKAN_BUFFER_USAGE_MODE_INDEX_READ != 0 {
        *stage_flags |= VK_PIPELINE_STAGE_VERTEX_INPUT_BIT;
        *access_mask |= VK_ACCESS_INDEX_READ_BIT;
    }

    if usage_mode_flags & VULKAN_BUFFER_USAGE_MODE_INDIRECT != 0 {
        *stage_flags |= VK_PIPELINE_STAGE_DRAW_INDIRECT_BIT;
        *access_mask |= VK_ACCESS_INDIRECT_COMMAND_READ_BIT;
    }

    if usage_mode_flags & VULKAN_BUFFER_USAGE_MODE_GRAPHICS_STORAGE_READ != 0 {
        *stage_flags |= VK_PIPELINE_STAGE_VERTEX_SHADER_BIT | VK_PIPELINE_STAGE_FRAGMENT_SHADER_BIT;
        *access_mask |= VK_ACCESS_SHADER_READ_BIT;
    }

    if usage_mode_flags & VULKAN_BUFFER_USAGE_MODE_COMPUTE_STORAGE_READ != 0 {
        *stage_flags |= VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT;
        *access_mask |= VK_ACCESS_SHADER_READ_BIT;
    }

    // Transfer flags (these will never be combined with other usages)
    if usage_mode_flags & VULKAN_BUFFER_USAGE_MODE_COPY_SOURCE != 0 {
        *stage_flags |= VK_PIPELINE_STAGE_TRANSFER_BIT;
        *access_mask |= VK_ACCESS_TRANSFER_READ_BIT;
    }

    if usage_mode_flags & VULKAN_BUFFER_USAGE_MODE_COPY_DESTINATION != 0 {
        *stage_flags |= VK_PIPELINE_STAGE_TRANSFER_BIT;
        *access_mask |= VK_ACCESS_TRANSFER_WRITE_BIT;
    }

    // Read-write flag
    if usage_mode_flags & VULKAN_BUFFER_USAGE_MODE_COMPUTE_STORAGE_READ_WRITE != 0 {
        *stage_flags |= VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT;
        *access_mask |= VK_ACCESS_SHADER_READ_BIT | VK_ACCESS_SHADER_WRITE_BIT;
    }
}

/// The stages, accesses and layout of a texture usage mode as a barrier's
/// source or destination; `None` for the modes that can't be one.
fn texture_usage_mode_barrier(
    mode: VulkanTextureUsageMode,
    destination: bool,
) -> Option<(VkPipelineStageFlags, VkAccessFlags, VkImageLayout)> {
    use VulkanTextureUsageMode as M;
    Some(match mode {
        M::Uninitialized if !destination => (
            VK_PIPELINE_STAGE_TOP_OF_PIPE_BIT,
            0,
            VK_IMAGE_LAYOUT_UNDEFINED,
        ),
        M::CopySource => (
            VK_PIPELINE_STAGE_TRANSFER_BIT,
            VK_ACCESS_TRANSFER_READ_BIT,
            VK_IMAGE_LAYOUT_TRANSFER_SRC_OPTIMAL,
        ),
        M::CopyDestination => (
            VK_PIPELINE_STAGE_TRANSFER_BIT,
            VK_ACCESS_TRANSFER_WRITE_BIT,
            VK_IMAGE_LAYOUT_TRANSFER_DST_OPTIMAL,
        ),
        M::Sampler => (
            VK_PIPELINE_STAGE_VERTEX_SHADER_BIT | VK_PIPELINE_STAGE_FRAGMENT_SHADER_BIT,
            VK_ACCESS_SHADER_READ_BIT,
            VK_IMAGE_LAYOUT_SHADER_READ_ONLY_OPTIMAL,
        ),
        M::GraphicsStorageRead => (
            VK_PIPELINE_STAGE_VERTEX_SHADER_BIT | VK_PIPELINE_STAGE_FRAGMENT_SHADER_BIT,
            VK_ACCESS_SHADER_READ_BIT,
            VK_IMAGE_LAYOUT_GENERAL,
        ),
        M::ComputeStorageRead => (
            VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
            VK_ACCESS_SHADER_READ_BIT,
            VK_IMAGE_LAYOUT_GENERAL,
        ),
        M::ComputeStorageReadWrite => (
            VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
            VK_ACCESS_SHADER_READ_BIT | VK_ACCESS_SHADER_WRITE_BIT,
            VK_IMAGE_LAYOUT_GENERAL,
        ),
        M::ColorAttachment if !destination => (
            VK_PIPELINE_STAGE_COLOR_ATTACHMENT_OUTPUT_BIT,
            VK_ACCESS_COLOR_ATTACHMENT_WRITE_BIT,
            VK_IMAGE_LAYOUT_COLOR_ATTACHMENT_OPTIMAL,
        ),
        M::ColorAttachment => (
            VK_PIPELINE_STAGE_COLOR_ATTACHMENT_OUTPUT_BIT,
            VK_ACCESS_COLOR_ATTACHMENT_READ_BIT | VK_ACCESS_COLOR_ATTACHMENT_WRITE_BIT,
            VK_IMAGE_LAYOUT_COLOR_ATTACHMENT_OPTIMAL,
        ),
        M::DepthStencilAttachment if !destination => (
            VK_PIPELINE_STAGE_EARLY_FRAGMENT_TESTS_BIT | VK_PIPELINE_STAGE_LATE_FRAGMENT_TESTS_BIT,
            VK_ACCESS_DEPTH_STENCIL_ATTACHMENT_WRITE_BIT,
            VK_IMAGE_LAYOUT_DEPTH_STENCIL_ATTACHMENT_OPTIMAL,
        ),
        M::DepthStencilAttachment => (
            VK_PIPELINE_STAGE_EARLY_FRAGMENT_TESTS_BIT | VK_PIPELINE_STAGE_LATE_FRAGMENT_TESTS_BIT,
            VK_ACCESS_DEPTH_STENCIL_ATTACHMENT_READ_BIT
                | VK_ACCESS_DEPTH_STENCIL_ATTACHMENT_WRITE_BIT,
            VK_IMAGE_LAYOUT_DEPTH_STENCIL_ATTACHMENT_OPTIMAL,
        ),
        M::Present if destination => (
            VK_PIPELINE_STAGE_BOTTOM_OF_PIPE_BIT,
            0,
            VK_IMAGE_LAYOUT_PRESENT_SRC_KHR,
        ),
        _ => return None,
    })
}

/// The default usage mode of a buffer. Translation of
/// `VULKAN_INTERNAL_DefaultBufferUsageMode()`.
pub(super) fn default_buffer_usage_mode(buffer: &VulkanBuffer) -> VulkanBufferUsageModeFlags {
    let mut flags = 0;

    if buffer.usage.contains(BufferUsageFlags::VERTEX) {
        flags |= VULKAN_BUFFER_USAGE_MODE_VERTEX_READ;
    }
    if buffer.usage.contains(BufferUsageFlags::INDEX) {
        flags |= VULKAN_BUFFER_USAGE_MODE_INDEX_READ;
    }
    if buffer.usage.contains(BufferUsageFlags::INDIRECT) {
        flags |= VULKAN_BUFFER_USAGE_MODE_INDIRECT;
    }
    if buffer
        .usage
        .contains(BufferUsageFlags::GRAPHICS_STORAGE_READ)
    {
        flags |= VULKAN_BUFFER_USAGE_MODE_GRAPHICS_STORAGE_READ;
    }
    if buffer
        .usage
        .contains(BufferUsageFlags::COMPUTE_STORAGE_READ)
    {
        flags |= VULKAN_BUFFER_USAGE_MODE_COMPUTE_STORAGE_READ;
    }

    // If no read flags are set, read-write can be the default.
    if flags == 0
        && buffer
            .usage
            .contains(BufferUsageFlags::COMPUTE_STORAGE_WRITE)
    {
        flags = VULKAN_BUFFER_USAGE_MODE_COMPUTE_STORAGE_READ_WRITE;
    }

    if flags == 0 {
        crate::log::error!(Category::Gpu, "Buffer has no default usage mode!");
        return VULKAN_BUFFER_USAGE_MODE_VERTEX_READ;
    }

    flags
}

/// The default usage mode of a texture. Translation of
/// `VULKAN_INTERNAL_DefaultTextureUsageMode()`.
pub(super) fn default_texture_usage_mode(texture: &VulkanTexture) -> VulkanTextureUsageMode {
    // NOTE: order matters here!
    // NOTE: graphics storage bits and sampler bit are mutually exclusive!
    let usage = texture.usage;
    if usage.contains(TextureUsageFlags::SAMPLER) {
        VulkanTextureUsageMode::Sampler
    } else if usage.contains(TextureUsageFlags::GRAPHICS_STORAGE_READ) {
        VulkanTextureUsageMode::GraphicsStorageRead
    } else if usage.contains(TextureUsageFlags::COLOR_TARGET) {
        VulkanTextureUsageMode::ColorAttachment
    } else if usage.contains(TextureUsageFlags::DEPTH_STENCIL_TARGET) {
        VulkanTextureUsageMode::DepthStencilAttachment
    } else if usage.contains(TextureUsageFlags::COMPUTE_STORAGE_READ) {
        VulkanTextureUsageMode::ComputeStorageRead
    } else if usage.contains(TextureUsageFlags::COMPUTE_STORAGE_WRITE)
        || usage.contains(TextureUsageFlags::COMPUTE_STORAGE_SIMULTANEOUS_READ_WRITE)
    {
        VulkanTextureUsageMode::ComputeStorageReadWrite
    } else {
        crate::log::error!(Category::Gpu, "Texture has no default usage mode!");
        VulkanTextureUsageMode::Sampler
    }
}

/// The index of a texture's subresource. Translation of
/// `VULKAN_INTERNAL_GetTextureSubresourceIndex()`.
pub(super) fn get_texture_subresource_index(mip_level: u32, layer: u32, num_levels: u32) -> u32 {
    mip_level + (layer * num_levels)
}

impl VulkanBuffer {
    /// Set the buffer's container (`container`, `containerIndex`).
    pub(super) fn set_container(&self, container: Option<ContainerRef<BufferContainer>>) {
        *lock(&self.container) = container;
    }
}

impl VulkanTexture {
    /// Set the texture's container (`container`, `containerIndex`).
    pub(super) fn set_container(&self, container: Option<ContainerRef<TextureContainer>>) {
        *lock(&self.container) = container;
    }
}

/// Lock a container's state (the containers are plain memory upstream).
fn state<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    lock(m)
}

impl VulkanRenderer {
    // Memory Barriers

    /*
     * In Vulkan, we must manually synchronize operations that write to resources on the GPU
     * so that read-after-write, write-after-read, and write-after-write hazards do not occur.
     * Additionally, textures are required to be in specific layouts for specific use cases.
     * Both of these tasks are accomplished with vkCmdPipelineBarrier.
     *
     * To insert the correct barriers, we keep track of "usage modes" for buffers and textures.
     * These indicate the current usage of that resource on the command buffer.
     * The transition from one usage mode to another indicates how the barrier should be constructed.
     *
     * For buffer reads, read usage modes can be combined.
     * This can be a useful shortcut in certain cases, like when reading GLTF data.
     *
     * Pipeline barriers cannot be inserted during a render pass, but they can be inserted
     * during a compute or copy pass.
     *
     * This means that the "default" usage mode of any given resource should be that it should be
     * ready for a graphics-read operation, because we cannot barrier during a render pass.
     * In the case where a resource is only used in compute, its default usage mode can be compute-read.
     * This strategy allows us to avoid expensive record keeping of command buffer/resource usage mode pairs,
     * and it fully covers synchronization between all combinations of stages.
     *
     * In Upload and Copy functions, we transition the resource immediately before and after the copy command.
     *
     * When binding a resource for compute, we transition when the Bind functions are called.
     * If a bind slot containing a resource is overwritten, we transition the resource in that slot back to its default.
     * When EndComputePass is called we transition all bound resources back to their default state.
     *
     * When binding a texture as a render pass attachment, we transition the resource on BeginRenderPass
     * and transition it back to its default on EndRenderPass.
     *
     * This strategy imposes certain limitations on resource usage flags.
     * For example, a texture cannot have both the SAMPLER and STORAGE_READ usage flags,
     * because then it is impossible for the backend to infer which default usage mode the texture should use.
     *
     * Sync hazards can be detected by setting VK_KHRONOS_VALIDATION_VALIDATE_SYNC=1 when using validation layers.
     */

    /// Translation of `VULKAN_INTERNAL_BufferMemoryBarrier()`.
    pub(super) fn buffer_memory_barrier(
        &self,
        command_buffer: &mut VulkanCommandBuffer,
        source_usage_mode: VulkanBufferUsageModeFlags,
        destination_usage_mode: VulkanBufferUsageModeFlags,
        buffer: &VulkanBuffer,
    ) {
        let mut src_stages = 0;
        let mut dst_stages = 0;
        let mut memory_barrier = VkBufferMemoryBarrier {
            s_type: VK_STRUCTURE_TYPE_BUFFER_MEMORY_BARRIER,
            src_queue_family_index: VK_QUEUE_FAMILY_IGNORED,
            dst_queue_family_index: VK_QUEUE_FAMILY_IGNORED,
            buffer: buffer.buffer,
            offset: 0,
            size: buffer.size,
            ..Default::default()
        };

        set_memory_barrier_flags(
            source_usage_mode,
            &mut src_stages,
            &mut memory_barrier.src_access_mask,
        );

        set_memory_barrier_flags(
            destination_usage_mode,
            &mut dst_stages,
            &mut memory_barrier.dst_access_mask,
        );

        // SAFETY: a command buffer being recorded and a live buffer.
        unsafe {
            (self.dev.cmd_pipeline_barrier)(
                command_buffer.command_buffer,
                src_stages,
                dst_stages,
                0,
                0,
                null(),
                1,
                &memory_barrier,
                0,
                null(),
            )
        };

        buffer.transitioned.store(true, Ordering::SeqCst);
    }

    /// Translation of `VULKAN_INTERNAL_TextureMemoryBarrier()`.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn texture_memory_barrier(
        &self,
        command_buffer: &mut VulkanCommandBuffer,
        source_usage_mode: VulkanTextureUsageMode,
        destination_usage_mode: VulkanTextureUsageMode,
        base_level: u32,
        level_count: u32,
        base_layer: u32,
        layer_count: u32,
        texture: &VulkanTexture,
    ) {
        let Some((src_stages, src_access_mask, old_layout)) =
            texture_usage_mode_barrier(source_usage_mode, false)
        else {
            crate::log::error!(Category::Gpu, "Unrecognized texture source barrier type!");
            return;
        };

        let Some((dst_stages, dst_access_mask, new_layout)) =
            texture_usage_mode_barrier(destination_usage_mode, true)
        else {
            crate::log::error!(
                Category::Gpu,
                "Unrecognized texture destination barrier type!"
            );
            return;
        };

        let memory_barrier = VkImageMemoryBarrier {
            s_type: VK_STRUCTURE_TYPE_IMAGE_MEMORY_BARRIER,
            p_next: null(),
            src_access_mask,
            dst_access_mask,
            old_layout,
            new_layout,
            src_queue_family_index: VK_QUEUE_FAMILY_IGNORED,
            dst_queue_family_index: VK_QUEUE_FAMILY_IGNORED,
            image: texture.image,
            subresource_range: VkImageSubresourceRange {
                aspect_mask: texture.aspect_flags,
                base_mip_level: base_level,
                level_count,
                base_array_layer: base_layer,
                layer_count,
            },
        };

        // SAFETY: a command buffer being recorded and a live image.
        unsafe {
            (self.dev.cmd_pipeline_barrier)(
                command_buffer.command_buffer,
                src_stages,
                dst_stages,
                0,
                0,
                null(),
                0,
                null(),
                1,
                &memory_barrier,
            )
        };
    }

    /// Transitions the entire texture with a single barrier call.
    /// Translation of `VULKAN_INTERNAL_FullTextureMemoryBarrier()`.
    pub(super) fn full_texture_memory_barrier(
        &self,
        command_buffer: &mut VulkanCommandBuffer,
        source_usage_mode: VulkanTextureUsageMode,
        destination_usage_mode: VulkanTextureUsageMode,
        texture: &VulkanTexture,
    ) {
        self.texture_memory_barrier(
            command_buffer,
            source_usage_mode,
            destination_usage_mode,
            0,
            texture.level_count,
            0,
            texture.layer_count,
            texture,
        );
    }

    /// Translation of `VULKAN_INTERNAL_TextureSubresourceMemoryBarrier()`
    /// (the subresource is `texture.subresources[subresource]`).
    pub(super) fn texture_subresource_memory_barrier(
        &self,
        command_buffer: &mut VulkanCommandBuffer,
        source_usage_mode: VulkanTextureUsageMode,
        destination_usage_mode: VulkanTextureUsageMode,
        texture: &VulkanTexture,
        subresource: usize,
    ) {
        let mut layer_count = 1;

        // VK_KHR_maintenance9 adds the ability to independently transition arbitrary subsets of slices in a 3D texture
        // but otherwise it is not necessarily supported by the driver.
        // As a workaround we have to transition the whole texture instead of just the subresource.
        // If VK_KHR_maintenance9 becomes widely supported, this can be removed.
        // See https://docs.vulkan.org/features/latest/features/proposals/VK_KHR_maintenance9.html#_barriers_with_2d_array_compatible_3d_images
        if texture.ty == TextureType::Texture3D {
            layer_count = VK_REMAINING_ARRAY_LAYERS;
        }

        let subresource = &texture.subresources[subresource];
        self.texture_memory_barrier(
            command_buffer,
            source_usage_mode,
            destination_usage_mode,
            subresource.level,
            1,
            subresource.layer,
            layer_count,
            texture,
        );
    }

    /// Translation of `VULKAN_INTERNAL_BufferTransitionFromDefaultUsage()`.
    pub(super) fn buffer_transition_from_default_usage(
        &self,
        command_buffer: &mut VulkanCommandBuffer,
        destination_usage_mode: VulkanBufferUsageModeFlags,
        buffer: &VulkanBuffer,
    ) {
        self.buffer_memory_barrier(
            command_buffer,
            default_buffer_usage_mode(buffer),
            destination_usage_mode,
            buffer,
        );
    }

    /// Translation of `VULKAN_INTERNAL_BufferTransitionToDefaultUsage()`.
    pub(super) fn buffer_transition_to_default_usage(
        &self,
        command_buffer: &mut VulkanCommandBuffer,
        source_usage_mode: VulkanBufferUsageModeFlags,
        buffer: &VulkanBuffer,
    ) {
        self.buffer_memory_barrier(
            command_buffer,
            source_usage_mode,
            default_buffer_usage_mode(buffer),
            buffer,
        );
    }

    /// Translation of
    /// `VULKAN_INTERNAL_TextureSubresourceTransitionFromDefaultUsage()`.
    pub(super) fn texture_subresource_transition_from_default_usage(
        &self,
        command_buffer: &mut VulkanCommandBuffer,
        destination_usage_mode: VulkanTextureUsageMode,
        texture: &VulkanTexture,
        subresource: usize,
    ) {
        self.texture_subresource_memory_barrier(
            command_buffer,
            default_texture_usage_mode(texture),
            destination_usage_mode,
            texture,
            subresource,
        );
    }

    /// Translation of `VULKAN_INTERNAL_TextureTransitionFromDefaultUsage()`.
    pub(super) fn texture_transition_from_default_usage(
        &self,
        command_buffer: &mut VulkanCommandBuffer,
        destination_usage_mode: VulkanTextureUsageMode,
        texture: &VulkanTexture,
    ) {
        self.full_texture_memory_barrier(
            command_buffer,
            default_texture_usage_mode(texture),
            destination_usage_mode,
            texture,
        );
    }

    /// Translation of
    /// `VULKAN_INTERNAL_TextureSubresourceTransitionToDefaultUsage()`.
    pub(super) fn texture_subresource_transition_to_default_usage(
        &self,
        command_buffer: &mut VulkanCommandBuffer,
        source_usage_mode: VulkanTextureUsageMode,
        texture: &VulkanTexture,
        subresource: usize,
    ) {
        self.texture_subresource_memory_barrier(
            command_buffer,
            source_usage_mode,
            default_texture_usage_mode(texture),
            texture,
            subresource,
        );
    }

    /// Translation of `VULKAN_INTERNAL_TextureTransitionToDefaultUsage()`.
    pub(super) fn texture_transition_to_default_usage(
        &self,
        command_buffer: &mut VulkanCommandBuffer,
        source_usage_mode: VulkanTextureUsageMode,
        texture: &VulkanTexture,
    ) {
        self.full_texture_memory_barrier(
            command_buffer,
            source_usage_mode,
            default_texture_usage_mode(texture),
            texture,
        );
    }

    // Resource Disposal

    /// Translation of `VULKAN_INTERNAL_ReleaseFramebuffer()` (the caller
    /// holds `disposeLock`).
    pub(super) fn release_framebuffer(
        &self,
        dispose: &mut PendingDestroys,
        framebuffer: Arc<VulkanFramebuffer>,
    ) {
        dispose.framebuffers_to_destroy.push(framebuffer);
    }

    /// Translation of `VULKAN_INTERNAL_DestroyFramebuffer()`.
    pub(super) fn destroy_framebuffer(&self, framebuffer: &VulkanFramebuffer) {
        // SAFETY: a framebuffer of the device the GPU no longer uses.
        unsafe {
            (self.dev.destroy_framebuffer)(self.logical_device, framebuffer.framebuffer, null())
        };
    }

    /// Forget (and release) the cached framebuffers with a view. Translation
    /// of `VULKAN_INTERNAL_RemoveFramebuffersContainingView()` (with
    /// `CheckOneFramebufferForRemoval()`).
    pub(super) fn remove_framebuffers_containing_view(
        &self,
        dispose: &mut PendingDestroys,
        view: VkImageView,
    ) {
        let removed: Vec<Arc<VulkanFramebuffer>> = {
            let mut table = lock(&self.framebuffer_hash_table);
            let keys: Vec<FramebufferHashTableKey> = table
                .keys()
                .filter(|key| key.contains_view(view))
                .cloned()
                .collect();
            keys.iter().filter_map(|key| table.remove(key)).collect()
        };

        // (the hash table's destroy callback)
        for framebuffer in removed {
            self.release_framebuffer(dispose, framebuffer);
        }
    }

    /// Destroy a texture's views and image and give its memory back.
    /// Translation of `VULKAN_INTERNAL_DestroyTexture()` (the caller holds
    /// `disposeLock`, which releasing its framebuffers takes).
    pub(super) fn destroy_texture(&self, dispose: &mut PendingDestroys, texture: &VulkanTexture) {
        let device = self.logical_device;
        // Clean up subresources
        for subresource in &texture.subresources {
            if !subresource.render_target_views.is_empty() {
                for &view in &subresource.render_target_views {
                    self.remove_framebuffers_containing_view(dispose, view);
                }

                for &view in &subresource.render_target_views {
                    // SAFETY: a view of the image, unused by the GPU.
                    unsafe { (self.dev.destroy_image_view)(device, view, null()) };
                }
            }

            if subresource.compute_write_view != VK_NULL_HANDLE {
                // SAFETY: as above.
                unsafe {
                    (self.dev.destroy_image_view)(device, subresource.compute_write_view, null())
                };
            }

            if subresource.depth_stencil_view != VK_NULL_HANDLE {
                self.remove_framebuffers_containing_view(dispose, subresource.depth_stencil_view);
                // SAFETY: as above.
                unsafe {
                    (self.dev.destroy_image_view)(device, subresource.depth_stencil_view, null())
                };
            }
        }

        if texture.full_view != VK_NULL_HANDLE {
            // SAFETY: as above.
            unsafe { (self.dev.destroy_image_view)(device, texture.full_view, null()) };
        }

        /* Don't free an externally managed VkImage (e.g. XR swapchain images) */
        if texture.image != VK_NULL_HANDLE && !texture.externally_managed {
            // SAFETY: the texture's image, unused by the GPU.
            unsafe { (self.dev.destroy_image)(device, texture.image, null()) };
        }

        if let Some(used_region) = &texture.used_region {
            self.remove_memory_used_region(used_region);
        }
    }

    /// Translation of `VULKAN_INTERNAL_DestroyBuffer()`.
    pub(super) fn destroy_buffer(&self, buffer: &VulkanBuffer) {
        // SAFETY: the buffer, unused by the GPU.
        unsafe { (self.dev.destroy_buffer)(self.logical_device, buffer.buffer, null()) };

        self.remove_memory_used_region(&buffer.used_region);
    }

    /// Translation of `VULKAN_INTERNAL_DestroySampler()`.
    pub(super) fn destroy_sampler(&self, sampler: &VulkanSampler) {
        // SAFETY: the sampler, unused by the GPU.
        unsafe { (self.dev.destroy_sampler)(self.logical_device, sampler.sampler, null()) };
    }

    /// Destroy the released resources the GPU is done with. Translation of
    /// `VULKAN_INTERNAL_PerformPendingDestroys()`.
    pub(super) fn perform_pending_destroys(&self) {
        let mut guard = lock(&self.dispose);
        let dispose = &mut *guard;

        fn unused(reference_count: &AtomicI32) -> bool {
            reference_count.load(Ordering::SeqCst) == 0
        }

        for i in (0..dispose.textures_to_destroy.len()).rev() {
            if unused(&dispose.textures_to_destroy[i].reference_count) {
                let texture = dispose.textures_to_destroy.swap_remove(i);
                self.destroy_texture(dispose, &texture);
            }
        }

        for i in (0..dispose.buffers_to_destroy.len()).rev() {
            if unused(&dispose.buffers_to_destroy[i].reference_count) {
                let buffer = dispose.buffers_to_destroy.swap_remove(i);
                self.destroy_buffer(&buffer);
            }
        }

        for i in (0..dispose.graphics_pipelines_to_destroy.len()).rev() {
            if unused(&dispose.graphics_pipelines_to_destroy[i].reference_count) {
                let pipeline = dispose.graphics_pipelines_to_destroy.swap_remove(i);
                self.destroy_graphics_pipeline(&pipeline);
            }
        }

        for i in (0..dispose.compute_pipelines_to_destroy.len()).rev() {
            if unused(&dispose.compute_pipelines_to_destroy[i].reference_count) {
                let pipeline = dispose.compute_pipelines_to_destroy.swap_remove(i);
                self.destroy_compute_pipeline(&pipeline);
            }
        }

        for i in (0..dispose.shaders_to_destroy.len()).rev() {
            if unused(&dispose.shaders_to_destroy[i].reference_count) {
                let shader = dispose.shaders_to_destroy.swap_remove(i);
                self.destroy_shader(&shader);
            }
        }

        for i in (0..dispose.samplers_to_destroy.len()).rev() {
            if unused(&dispose.samplers_to_destroy[i].reference_count) {
                let sampler = dispose.samplers_to_destroy.swap_remove(i);
                self.destroy_sampler(&sampler);
            }
        }

        for i in (0..dispose.framebuffers_to_destroy.len()).rev() {
            if unused(&dispose.framebuffers_to_destroy[i].reference_count) {
                let framebuffer = dispose.framebuffers_to_destroy.swap_remove(i);
                self.destroy_framebuffer(&framebuffer);
            }
        }
    }

    /// Translation of `VULKAN_INTERNAL_ReleaseTexture()` (the caller holds
    /// `disposeLock`).
    pub(super) fn release_texture_internal(
        &self,
        dispose: &mut PendingDestroys,
        texture: &Arc<VulkanTexture>,
    ) {
        if texture.marked_for_destroy.load(Ordering::SeqCst) {
            return;
        }

        dispose.textures_to_destroy.push(texture.clone());

        texture.marked_for_destroy.store(true, Ordering::SeqCst);
    }

    /// Translation of `VULKAN_INTERNAL_ReleaseBuffer()` (the caller holds
    /// `disposeLock`).
    pub(super) fn release_buffer_internal(
        &self,
        dispose: &mut PendingDestroys,
        buffer: &Arc<VulkanBuffer>,
    ) {
        if buffer.marked_for_destroy.load(Ordering::SeqCst) {
            return;
        }

        dispose.buffers_to_destroy.push(buffer.clone());

        buffer.marked_for_destroy.store(true, Ordering::SeqCst);
        buffer.set_container(None);
    }

    /// Release every buffer of a container (whose handle the front end
    /// drops next). Translation of
    /// `VULKAN_INTERNAL_ReleaseBufferContainer()`.
    pub(super) fn release_buffer_container(&self, buffer_container: &BufferContainer) {
        // Containers are just client handles, so we can free immediately
        let buffers = std::mem::take(&mut state(&buffer_container.state).buffers);

        let mut dispose = lock(&self.dispose);
        for buffer in &buffers {
            self.release_buffer_internal(&mut dispose, buffer);
        }
    }

    // Data Buffer

    /// Translation of `VULKAN_INTERNAL_CreateBuffer()`.
    pub(super) fn create_buffer_internal(
        &self,
        size: VkDeviceSize,
        usage_flags: BufferUsageFlags,
        ty: VulkanBufferType,
        dedicated: bool,
        debug_name: Option<&str>,
    ) -> Result<Arc<VulkanBuffer>> {
        let mut vulkan_usage_flags = 0;

        if usage_flags.contains(BufferUsageFlags::VERTEX) {
            vulkan_usage_flags |= VK_BUFFER_USAGE_VERTEX_BUFFER_BIT;
        }

        if usage_flags.contains(BufferUsageFlags::INDEX) {
            vulkan_usage_flags |= VK_BUFFER_USAGE_INDEX_BUFFER_BIT;
        }

        if usage_flags.intersects(
            BufferUsageFlags::GRAPHICS_STORAGE_READ
                | BufferUsageFlags::COMPUTE_STORAGE_READ
                | BufferUsageFlags::COMPUTE_STORAGE_WRITE,
        ) {
            vulkan_usage_flags |= VK_BUFFER_USAGE_STORAGE_BUFFER_BIT;
        }

        if usage_flags.contains(BufferUsageFlags::INDIRECT) {
            vulkan_usage_flags |= VK_BUFFER_USAGE_INDIRECT_BUFFER_BIT;
        }

        if ty == VulkanBufferType::Uniform {
            vulkan_usage_flags |= VK_BUFFER_USAGE_UNIFORM_BUFFER_BIT;
        } else {
            // GPU buffers need transfer bits for defrag, transfer buffers need them for transfers
            vulkan_usage_flags |=
                VK_BUFFER_USAGE_TRANSFER_SRC_BIT | VK_BUFFER_USAGE_TRANSFER_DST_BIT;
        }

        let mut createinfo = VkBufferCreateInfo {
            s_type: VK_STRUCTURE_TYPE_BUFFER_CREATE_INFO,
            p_next: null(),
            flags: 0,
            size,
            usage: vulkan_usage_flags,
            sharing_mode: VK_SHARING_MODE_EXCLUSIVE,
            queue_family_index_count: 1,
            p_queue_family_indices: &self.queue_family_index,
        };

        // Set transfer bits so we can defrag
        createinfo.usage |= VK_BUFFER_USAGE_TRANSFER_SRC_BIT | VK_BUFFER_USAGE_TRANSFER_DST_BIT;

        let mut vk_buffer = VK_NULL_HANDLE;
        // SAFETY: a valid create info.
        let vulkan_result = unsafe {
            (self.dev.create_buffer)(self.logical_device, &createinfo, null(), &mut vk_buffer)
        };
        self.check(vulkan_result, "vkCreateBuffer")?;

        let used_region = match self.bind_memory_for_buffer(vk_buffer, size, ty, dedicated) {
            Ok(used_region) => used_region,
            Err(_) => {
                // SAFETY: the buffer just created.
                unsafe { (self.dev.destroy_buffer)(self.logical_device, vk_buffer, null()) };
                return Err(self.set_string_error("Failed to bind memory for buffer!"));
            }
        };

        let buffer = Arc::new(VulkanBuffer {
            container: Mutex::new(None),
            buffer: vk_buffer,
            used_region,
            ty,
            usage: usage_flags,
            size,
            reference_count: AtomicI32::new(0),
            transitioned: AtomicBool::new(false),
            marked_for_destroy: AtomicBool::new(false),
            uniform_buffer_for_defrag: Mutex::new(Weak::new()),
        });

        let _ = buffer
            .used_region
            .resource
            .set(RegionResource::Buffer(Arc::downgrade(&buffer))); // lol

        if let Some(debug_name) = debug_name {
            self.set_object_name(VK_OBJECT_TYPE_BUFFER, vk_buffer, debug_name);
        }

        Ok(buffer)
    }

    /// Translation of `VULKAN_INTERNAL_CreateBufferContainer()`.
    pub(super) fn create_buffer_container(
        &self,
        size: VkDeviceSize,
        usage_flags: BufferUsageFlags,
        ty: VulkanBufferType,
        dedicated: bool,
        debug_name: Option<&str>,
    ) -> Result<Arc<BufferContainer>> {
        let buffer = self.create_buffer_internal(size, usage_flags, ty, dedicated, debug_name)?;

        let buffer_container = Arc::new(BufferContainer {
            state: Mutex::new(BufferContainerState {
                active_buffer: buffer.clone(),
                buffers: vec![buffer.clone()],
                debug_name: debug_name.map(str::to_owned),
            }),
            dedicated,
        });
        buffer.set_container(Some(ContainerRef {
            container: Arc::downgrade(&buffer_container),
            index: 0,
        }));

        Ok(buffer_container)
    }

    /// Translation of `VULKAN_INTERNAL_CreateUniformBuffer()`.
    pub(super) fn create_uniform_buffer(&self, size: u32) -> Result<Arc<UniformBuffer>> {
        let buffer = self.create_buffer_internal(
            size as VkDeviceSize,
            BufferUsageFlags(0),
            VulkanBufferType::Uniform,
            false,
            None,
        )?;

        let uniform_buffer = Arc::new(UniformBuffer {
            state: Mutex::new(UniformBufferState {
                buffer: buffer.clone(),
                draw_offset: 0,
                write_offset: 0,
            }),
        });
        *lock(&buffer.uniform_buffer_for_defrag) = Arc::downgrade(&uniform_buffer);

        Ok(uniform_buffer)
    }

    /// Make a container's active buffer one the GPU doesn't use, a new one
    /// if needed. Translation of `VULKAN_INTERNAL_CycleActiveBuffer()`.
    ///
    /// The container's lock isn't held while the new buffer is created
    /// (upstream has no lock there), so that the allocator's lock, which
    /// defragmentation holds while it re-points containers, comes first.
    pub(super) fn cycle_active_buffer(&self, container: &Arc<BufferContainer>) {
        let (size, usage, ty, debug_name) = {
            let mut container_state = state(&container.state);

            // If a previously-cycled buffer is available, we can use that.
            if let Some(buffer) = container_state
                .buffers
                .iter()
                .find(|b| b.reference_count.load(Ordering::SeqCst) == 0)
                .cloned()
            {
                container_state.active_buffer = buffer;
                return;
            }

            let active = &container_state.active_buffer;
            (
                active.size,
                active.usage,
                active.ty,
                container_state.debug_name.clone(),
            )
        };

        // No buffer handle is available, create a new one.
        let Ok(buffer) = self.create_buffer_internal(
            size,
            usage,
            ty,
            container.dedicated,
            debug_name.as_deref(),
        ) else {
            return;
        };

        let mut container_state = state(&container.state);
        buffer.set_container(Some(ContainerRef {
            container: Arc::downgrade(container),
            index: container_state.buffers.len(),
        }));
        container_state.buffers.push(buffer.clone());

        container_state.active_buffer = buffer;
    }

    /// The memory of a transfer buffer's active buffer, cycled first if
    /// asked and the GPU uses it. Translation of
    /// `VULKAN_MapTransferBuffer()`.
    pub(super) fn map_transfer_buffer_internal(
        &self,
        transfer_buffer_container: &Arc<BufferContainer>,
        cycle: bool,
    ) -> Result<NonNull<u8>> {
        let active = state(&transfer_buffer_container.state)
            .active_buffer
            .clone();
        if cycle && active.reference_count.load(Ordering::SeqCst) > 0 {
            self.cycle_active_buffer(transfer_buffer_container);
        }

        let active = state(&transfer_buffer_container.state)
            .active_buffer
            .clone();
        let used_region = &active.used_region;
        // Note (upstream): transfer buffers are in host-visible memory, so
        // C adds the offset to the mapping without checking it.
        let map_pointer = used_region
            .allocation
            .map_pointer()
            .ok_or_else(|| self.set_string_error("Transfer buffer memory is not mapped!"))?;
        // SAFETY: the region lies in the allocation, which the mapping
        // covers whole.
        Ok(unsafe { map_pointer.add(used_region.resource_offset as usize) })
    }

    // Texture Subresource Utilities

    /// Translation of `VULKAN_INTERNAL_CreateRenderTargetView()`.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn create_render_target_view(
        &self,
        texture: &VulkanTexture,
        layer_or_depth: u32,
        level: u32,
        format: VkFormat,
        swizzle: VkComponentMapping,
    ) -> Result<VkImageView> {
        // create framebuffer compatible views for RenderTarget
        let image_view_create_info = VkImageViewCreateInfo {
            s_type: VK_STRUCTURE_TYPE_IMAGE_VIEW_CREATE_INFO,
            p_next: null(),
            flags: 0,
            image: texture.image,
            format,
            components: swizzle,
            subresource_range: VkImageSubresourceRange {
                aspect_mask: texture.aspect_flags,
                base_mip_level: level,
                level_count: 1,
                base_array_layer: layer_or_depth,
                layer_count: 1,
            },
            view_type: VK_IMAGE_VIEW_TYPE_2D,
        };

        let mut view = VK_NULL_HANDLE;
        // SAFETY: a valid create info of a live image.
        let vulkan_result = unsafe {
            (self.dev.create_image_view)(
                self.logical_device,
                &image_view_create_info,
                null(),
                &mut view,
            )
        };
        self.check(vulkan_result, "vkCreateImageView")?;
        Ok(view)
    }

    /// Translation of `VULKAN_INTERNAL_CreateSubresourceView()`.
    fn create_subresource_view(
        &self,
        createinfo: &TextureCreateInfo,
        texture: &VulkanTexture,
        layer: u32,
        level: u32,
        swizzle: VkComponentMapping,
    ) -> Result<VkImageView> {
        // create framebuffer compatible views for RenderTarget
        let image_view_create_info = VkImageViewCreateInfo {
            s_type: VK_STRUCTURE_TYPE_IMAGE_VIEW_CREATE_INFO,
            p_next: null(),
            flags: 0,
            image: texture.image,
            format: sdl_to_vk_texture_format(createinfo.format),
            components: swizzle,
            subresource_range: VkImageSubresourceRange {
                aspect_mask: texture.aspect_flags,
                base_mip_level: level,
                level_count: 1,
                base_array_layer: layer,
                layer_count: 1,
            },
            view_type: if createinfo.texture_type == TextureType::Texture3D {
                VK_IMAGE_VIEW_TYPE_3D
            } else {
                VK_IMAGE_VIEW_TYPE_2D
            },
        };

        let mut view = VK_NULL_HANDLE;
        // SAFETY: a valid create info of a live image.
        let vulkan_result = unsafe {
            (self.dev.create_image_view)(
                self.logical_device,
                &image_view_create_info,
                null(),
                &mut view,
            )
        };
        self.check(vulkan_result, "vkCreateImageView")?;
        Ok(view)
    }

    /// Destroy a texture that isn't finished (from
    /// `VULKAN_INTERNAL_CreateTexture()`'s failure paths).
    fn destroy_unfinished_texture(&self, texture: &VulkanTexture) {
        let mut dispose = lock(&self.dispose);
        self.destroy_texture(&mut dispose, texture);
    }

    /// Translation of `VULKAN_INTERNAL_CreateTexture()`.
    pub(super) fn create_texture_internal(
        &self,
        createinfo: &TextureCreateInfo,
    ) -> Result<Arc<VulkanTexture>> {
        let mut image_create_flags = 0;
        let mut vk_usage_flags = VK_IMAGE_USAGE_TRANSFER_SRC_BIT | VK_IMAGE_USAGE_TRANSFER_DST_BIT;
        let is_3d = createinfo.texture_type == TextureType::Texture3D;
        let layer_count = if is_3d {
            1
        } else {
            createinfo.layer_count_or_depth
        };
        let depth = if is_3d {
            createinfo.layer_count_or_depth
        } else {
            1
        };
        let usage = createinfo.usage;

        let mut texture = VulkanTexture {
            container: Mutex::new(None),
            used_region: None,
            image: VK_NULL_HANDLE,
            full_view: VK_NULL_HANDLE,
            swizzle: swizzle_for_sdl_format(createinfo.format),
            aspect_flags: 0,
            depth,
            level_count: createinfo.num_levels,
            layer_count,
            ty: createinfo.texture_type,
            usage,
            subresources: Vec::new(),
            marked_for_destroy: AtomicBool::new(false),
            externally_managed: false,
            reference_count: AtomicI32::new(0),
        };

        if createinfo.format.is_depth_format() {
            texture.aspect_flags = VK_IMAGE_ASPECT_DEPTH_BIT;

            if createinfo.format.is_stencil_format() {
                texture.aspect_flags |= VK_IMAGE_ASPECT_STENCIL_BIT;
            }
        } else {
            texture.aspect_flags = VK_IMAGE_ASPECT_COLOR_BIT;
        }

        if matches!(
            createinfo.texture_type,
            TextureType::Cube | TextureType::CubeArray
        ) {
            image_create_flags |= VK_IMAGE_CREATE_CUBE_COMPATIBLE_BIT;
        } else if is_3d {
            image_create_flags |= VK_IMAGE_CREATE_2D_ARRAY_COMPATIBLE_BIT;
        }

        if usage.intersects(
            TextureUsageFlags::SAMPLER
                | TextureUsageFlags::GRAPHICS_STORAGE_READ
                | TextureUsageFlags::COMPUTE_STORAGE_READ,
        ) {
            vk_usage_flags |= VK_IMAGE_USAGE_SAMPLED_BIT;
        }
        if usage.contains(TextureUsageFlags::COLOR_TARGET) {
            vk_usage_flags |= VK_IMAGE_USAGE_COLOR_ATTACHMENT_BIT;
        }
        if usage.contains(TextureUsageFlags::DEPTH_STENCIL_TARGET) {
            vk_usage_flags |= VK_IMAGE_USAGE_DEPTH_STENCIL_ATTACHMENT_BIT;
        }
        if usage.intersects(
            TextureUsageFlags::COMPUTE_STORAGE_WRITE
                | TextureUsageFlags::COMPUTE_STORAGE_SIMULTANEOUS_READ_WRITE,
        ) {
            vk_usage_flags |= VK_IMAGE_USAGE_STORAGE_BIT;
        }

        let image_create_info = VkImageCreateInfo {
            s_type: VK_STRUCTURE_TYPE_IMAGE_CREATE_INFO,
            p_next: null(),
            flags: image_create_flags,
            image_type: if is_3d {
                VK_IMAGE_TYPE_3D
            } else {
                VK_IMAGE_TYPE_2D
            },
            format: sdl_to_vk_texture_format(createinfo.format),
            extent: VkExtent3D {
                width: createinfo.width,
                height: createinfo.height,
                depth,
            },
            mip_levels: createinfo.num_levels,
            array_layers: layer_count,
            samples: sdl_to_vk_sample_count(createinfo.sample_count),
            tiling: VK_IMAGE_TILING_OPTIMAL,
            usage: vk_usage_flags,
            sharing_mode: VK_SHARING_MODE_EXCLUSIVE,
            queue_family_index_count: 0,
            p_queue_family_indices: null(),
            initial_layout: VK_IMAGE_LAYOUT_UNDEFINED,
        };

        // SAFETY: a valid create info.
        let vulkan_result = unsafe {
            (self.dev.create_image)(
                self.logical_device,
                &image_create_info,
                null(),
                &mut texture.image,
            )
        };

        if vulkan_result != VK_SUCCESS {
            self.destroy_unfinished_texture(&texture);
            self.check(vulkan_result, "vkCreateImage")?;
        }

        match self.bind_memory_for_image(texture.image) {
            Ok(used_region) => texture.used_region = Some(used_region),
            Err(_) => {
                // SAFETY: the image just created.
                unsafe { (self.dev.destroy_image)(self.logical_device, texture.image, null()) };

                // FIXME (upstream): DestroyTexture destroys the image a
                // second time (texture->image is still set).
                self.destroy_unfinished_texture(&texture);
                return Err(self.set_string_error("Unable to bind memory for texture!"));
            }
        }

        if usage.intersects(
            TextureUsageFlags::SAMPLER
                | TextureUsageFlags::GRAPHICS_STORAGE_READ
                | TextureUsageFlags::COMPUTE_STORAGE_READ,
        ) {
            let view_type = match createinfo.texture_type {
                TextureType::Cube => VK_IMAGE_VIEW_TYPE_CUBE,
                TextureType::CubeArray => VK_IMAGE_VIEW_TYPE_CUBE_ARRAY,
                TextureType::Texture3D => VK_IMAGE_VIEW_TYPE_3D,
                TextureType::Texture2DArray => VK_IMAGE_VIEW_TYPE_2D_ARRAY,
                TextureType::Texture2D => VK_IMAGE_VIEW_TYPE_2D,
            };
            let image_view_create_info = VkImageViewCreateInfo {
                s_type: VK_STRUCTURE_TYPE_IMAGE_VIEW_CREATE_INFO,
                p_next: null(),
                flags: 0,
                image: texture.image,
                format: sdl_to_vk_texture_format(createinfo.format),
                components: texture.swizzle,
                subresource_range: VkImageSubresourceRange {
                    aspect_mask: texture.aspect_flags & !VK_IMAGE_ASPECT_STENCIL_BIT, // Can't sample stencil values
                    base_mip_level: 0,
                    level_count: createinfo.num_levels,
                    base_array_layer: 0,
                    layer_count,
                },
                view_type,
            };

            // SAFETY: a valid create info of the live image.
            let vulkan_result = unsafe {
                (self.dev.create_image_view)(
                    self.logical_device,
                    &image_view_create_info,
                    null(),
                    &mut texture.full_view,
                )
            };

            if vulkan_result != VK_SUCCESS {
                self.destroy_unfinished_texture(&texture);
                self.check(vulkan_result, "vkCreateImageView")?;
            }
        }

        // Define slices
        // Note (upstream): when a view fails, C destroys the render target
        // views it didn't create yet too (uninitialized memory); only the
        // views made are destroyed here.
        let format = sdl_to_vk_texture_format(createinfo.format);
        texture.subresources = (0..layer_count * createinfo.num_levels)
            .map(|_| TextureSubresource::default())
            .collect();

        for i in 0..layer_count {
            for j in 0..createinfo.num_levels {
                let subresource_index =
                    get_texture_subresource_index(j, i, createinfo.num_levels) as usize;

                if usage.contains(TextureUsageFlags::COLOR_TARGET) {
                    if depth > 1 {
                        for k in 0..depth {
                            match self.create_render_target_view(
                                &texture,
                                k,
                                j,
                                format,
                                texture.swizzle,
                            ) {
                                Ok(view) => texture.subresources[subresource_index]
                                    .render_target_views
                                    .push(view),
                                Err(e) => {
                                    self.destroy_unfinished_texture(&texture);
                                    return Err(e);
                                }
                            }
                        }
                    } else {
                        match self.create_render_target_view(
                            &texture,
                            i,
                            j,
                            format,
                            texture.swizzle,
                        ) {
                            Ok(view) => texture.subresources[subresource_index]
                                .render_target_views
                                .push(view),
                            Err(e) => {
                                self.destroy_unfinished_texture(&texture);
                                return Err(e);
                            }
                        }
                    }
                }

                if usage.intersects(
                    TextureUsageFlags::COMPUTE_STORAGE_WRITE
                        | TextureUsageFlags::COMPUTE_STORAGE_SIMULTANEOUS_READ_WRITE,
                ) {
                    match self.create_subresource_view(createinfo, &texture, i, j, texture.swizzle)
                    {
                        Ok(view) => {
                            texture.subresources[subresource_index].compute_write_view = view
                        }
                        Err(e) => {
                            self.destroy_unfinished_texture(&texture);
                            return Err(e);
                        }
                    }
                }

                if usage.contains(TextureUsageFlags::DEPTH_STENCIL_TARGET) {
                    match self.create_subresource_view(createinfo, &texture, i, j, texture.swizzle)
                    {
                        Ok(view) => {
                            texture.subresources[subresource_index].depth_stencil_view = view
                        }
                        Err(e) => {
                            self.destroy_unfinished_texture(&texture);
                            return Err(e);
                        }
                    }
                }

                texture.subresources[subresource_index].layer = i;
                texture.subresources[subresource_index].level = j;
            }
        }

        // Set debug name if applicable
        if let Some(name) = createinfo
            .props
            .as_ref()
            .and_then(|p| p.get_string(crate::gpu::PROP_GPU_TEXTURE_CREATE_NAME_STRING))
        {
            self.set_object_name(VK_OBJECT_TYPE_IMAGE, texture.image, &name);
        }

        let texture = Arc::new(texture);
        if let Some(used_region) = &texture.used_region {
            let _ = used_region
                .resource
                .set(RegionResource::Texture(Arc::downgrade(&texture))); // lol
        }

        Ok(texture)
    }

    /// Translation of `VULKAN_CreateTexture()`.
    pub(super) fn create_texture_container(
        &self,
        createinfo: &TextureCreateInfo,
    ) -> Result<Arc<TextureContainer>> {
        let texture = self.create_texture_internal(createinfo)?;

        // Copy properties so we don't lose information when the client destroys them
        let props = Properties::new();
        if let Some(src) = &createinfo.props {
            props.copy_from(src)?;
        }
        let debug_name = props.get_string(crate::gpu::PROP_GPU_TEXTURE_CREATE_NAME_STRING);
        let info = TextureCreateInfo {
            props: Some(props),
            ..createinfo.clone()
        };

        let container = Arc::new(TextureContainer {
            info,
            state: Mutex::new(TextureContainerState {
                active_texture: texture.clone(),
                textures: vec![texture.clone()],
                debug_name,
            }),
            can_be_cycled: true,
            externally_managed: false,
        });

        texture.set_container(Some(ContainerRef {
            container: Arc::downgrade(&container),
            index: 0,
        }));

        // Let's transition to the default barrier state, because for some reason Vulkan doesn't let us do that with initialLayout.
        // Only do this after "container" is set, so the texture
        // is fully initialized before any Submit that could trigger defrag.
        {
            // Note (upstream): C goes on with a NULL command buffer when none
            // can be acquired; the error is returned here.
            let mut barrier_command_buffer = match self.acquire_command_buffer_internal() {
                Ok(command_buffer) => command_buffer,
                Err(e) => {
                    self.release_texture_container(&container);
                    return Err(e);
                }
            };
            self.texture_transition_to_default_usage(
                &mut barrier_command_buffer,
                VulkanTextureUsageMode::Uninitialized,
                &texture,
            );

            barrier_command_buffer.track_texture(&texture);

            if let Err(e) = self.submit_internal(barrier_command_buffer) {
                self.release_texture_container(&container);
                return Err(e);
            }
        }

        Ok(container)
    }

    /// The subresource of a container's active texture. Translation of
    /// `VULKAN_INTERNAL_FetchTextureSubresource()`.
    pub(super) fn fetch_texture_subresource(
        container: &TextureContainer,
        layer: u32,
        level: u32,
    ) -> SubresourceRef {
        let index = get_texture_subresource_index(level, layer, container.info.num_levels);

        SubresourceRef {
            texture: state(&container.state).active_texture.clone(),
            index: index as usize,
        }
    }

    /// Make a container's active texture one the GPU doesn't use, a new one
    /// if needed. Translation of `VULKAN_INTERNAL_CycleActiveTexture()`.
    ///
    /// As in [`Self::cycle_active_buffer`], the container's lock isn't held
    /// while the new texture is created.
    pub(super) fn cycle_active_texture(
        &self,
        command_buffer: &mut VulkanCommandBuffer,
        container: &Arc<TextureContainer>,
    ) {
        {
            let mut container_state = state(&container.state);

            // If a previously-cycled texture is available, we can use that.
            if let Some(texture) = container_state
                .textures
                .iter()
                .find(|t| t.reference_count.load(Ordering::SeqCst) == 0)
                .cloned()
            {
                container_state.active_texture = texture;
                return;
            }
        }

        // No texture is available, generate a new one.
        let Ok(texture) = self.create_texture_internal(&container.info) else {
            return;
        };

        {
            let mut container_state = state(&container.state);
            texture.set_container(Some(ContainerRef {
                container: Arc::downgrade(container),
                index: container_state.textures.len(),
            }));
            container_state.textures.push(texture.clone());

            container_state.active_texture = texture.clone();
        }

        // Transition texture after storing it as the memory barrier might need to read the texture's container info
        self.texture_transition_to_default_usage(
            command_buffer,
            VulkanTextureUsageMode::Uninitialized,
            &texture,
        );
    }

    /// The container's active buffer, cycled first if asked and the GPU
    /// uses it, transitioned for a write. Translation of
    /// `VULKAN_INTERNAL_PrepareBufferForWrite()`.
    pub(super) fn prepare_buffer_for_write(
        &self,
        command_buffer: &mut VulkanCommandBuffer,
        buffer_container: &Arc<BufferContainer>,
        cycle: bool,
        destination_usage_mode: VulkanBufferUsageModeFlags,
    ) -> Arc<VulkanBuffer> {
        let active = state(&buffer_container.state).active_buffer.clone();
        if cycle && active.reference_count.load(Ordering::SeqCst) > 0 {
            self.cycle_active_buffer(buffer_container);
        }

        let active = state(&buffer_container.state).active_buffer.clone();
        self.buffer_transition_from_default_usage(command_buffer, destination_usage_mode, &active);

        active
    }

    /// A subresource of the container's active texture, cycled first if
    /// asked and the GPU uses it, transitioned for a write. Translation of
    /// `VULKAN_INTERNAL_PrepareTextureSubresourceForWrite()`.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn prepare_texture_subresource_for_write(
        &self,
        command_buffer: &mut VulkanCommandBuffer,
        texture_container: &Arc<TextureContainer>,
        layer: u32,
        level: u32,
        cycle: bool,
        destination_usage_mode: VulkanTextureUsageMode,
    ) -> SubresourceRef {
        let mut texture_subresource =
            Self::fetch_texture_subresource(texture_container, layer, level);

        if cycle
            && texture_container.can_be_cycled
            && texture_subresource
                .texture
                .reference_count
                .load(Ordering::SeqCst)
                > 0
        {
            self.cycle_active_texture(command_buffer, texture_container);

            texture_subresource = Self::fetch_texture_subresource(texture_container, layer, level);
        }

        // always do barrier because of layout transitions
        self.texture_subresource_transition_from_default_usage(
            command_buffer,
            destination_usage_mode,
            &texture_subresource.texture,
            texture_subresource.index,
        );

        texture_subresource
    }

    /// Release every texture of a container (whose handle the front end
    /// drops next). Translation of `VULKAN_ReleaseTexture()`.
    pub(super) fn release_texture_container(&self, container: &TextureContainer) {
        // Containers are just client handles, so we can destroy immediately
        let textures = std::mem::take(&mut state(&container.state).textures);

        let mut dispose = lock(&self.dispose);
        for texture in &textures {
            self.release_texture_internal(&mut dispose, texture);
        }
    }

    // Defragmentation

    /// Move the resources of an allocation marked for defrag to new memory,
    /// recording the copies into `command_buffer`. Translation of
    /// `VULKAN_INTERNAL_DefragmentMemory()`.
    ///
    /// Part 2's `Submit` calls this (with a command buffer it acquires for
    /// it) when allocations are marked for defrag and no defrag is in
    /// progress; cleaning that command buffer clears `defrag_in_progress`.
    pub(super) fn defragment_memory(&self, command_buffer: &mut VulkanCommandBuffer) -> Result<()> {
        self.defrag_in_progress.store(true, Ordering::SeqCst);
        command_buffer.is_defrag = true;

        let allocator = self.memory_allocator.lock();
        let _defrag = self.defrag_lock.write().unwrap_or_else(|e| e.into_inner());

        let used_regions =
            {
                let mut a = allocator.borrow_mut();

                // Find an allocation that doesn't have any pending transfer operations
                let Some(index_to_defrag) = a.allocations_to_defrag.iter().rposition(|&key| {
                    a.allocation(key).reference_count.load(Ordering::SeqCst) == 0
                }) else {
                    // Nothing is available to defrag, but it's not an error
                    return Ok(());
                };

                // Plug the hole
                let allocation = a.allocations_to_defrag.swap_remove(index_to_defrag);

                a.used_regions(allocation)
            };

        /* For each used region in the allocation
         * create a new resource, copy the data
         * and re-point the resource containers
         */
        for current_region in &used_regions {
            match current_region.resource.get() {
                Some(RegionResource::Buffer(buffer)) if current_region.is_buffer => {
                    let Some(buffer) = buffer.upgrade() else {
                        continue;
                    };
                    if buffer.marked_for_destroy.load(Ordering::SeqCst) {
                        continue;
                    }
                    self.defragment_buffer(command_buffer, current_region, &buffer)?;
                }
                Some(RegionResource::Texture(texture)) if !current_region.is_buffer => {
                    let Some(texture) = texture.upgrade() else {
                        continue;
                    };
                    if texture.marked_for_destroy.load(Ordering::SeqCst) {
                        continue;
                    }
                    self.defragment_texture(command_buffer, &texture)?;
                }
                _ => {}
            }
        }

        Ok(())
    }

    /// The buffer half of the defrag loop.
    fn defragment_buffer(
        &self,
        command_buffer: &mut VulkanCommandBuffer,
        current_region: &UsedRegion,
        buffer: &Arc<VulkanBuffer>,
    ) -> Result<()> {
        let container = lock(&buffer.container)
            .as_ref()
            .and_then(|c| c.container.upgrade().map(|container| (container, c.index)));
        let debug_name = container
            .as_ref()
            .and_then(|(c, _)| state(&c.state).debug_name.clone());

        let new_buffer = match self.create_buffer_internal(
            buffer.size,
            buffer.usage,
            buffer.ty,
            false,
            debug_name.as_deref(),
        ) {
            Ok(new_buffer) => new_buffer,
            Err(e) => {
                crate::log::error!(Category::Gpu, "{}", "Failed to allocate defrag buffer!");
                return Err(e);
            }
        };

        // Copy buffer contents if necessary
        if buffer.ty == VulkanBufferType::Gpu && buffer.transitioned.load(Ordering::SeqCst) {
            self.buffer_transition_from_default_usage(
                command_buffer,
                VULKAN_BUFFER_USAGE_MODE_COPY_SOURCE,
                buffer,
            );

            self.buffer_transition_from_default_usage(
                command_buffer,
                VULKAN_BUFFER_USAGE_MODE_COPY_DESTINATION,
                &new_buffer,
            );

            let buffer_copy = VkBufferCopy {
                src_offset: 0,
                dst_offset: 0,
                size: current_region.resource_size,
            };

            // SAFETY: a command buffer being recorded and live buffers.
            unsafe {
                (self.dev.cmd_copy_buffer)(
                    command_buffer.command_buffer,
                    buffer.buffer,
                    new_buffer.buffer,
                    1,
                    &buffer_copy,
                )
            };

            self.buffer_transition_to_default_usage(
                command_buffer,
                VULKAN_BUFFER_USAGE_MODE_COPY_DESTINATION,
                &new_buffer,
            );

            command_buffer.track_buffer(buffer);
            command_buffer.track_buffer(&new_buffer);
        }

        // re-point original container to new buffer
        let uniform_buffer = lock(&buffer.uniform_buffer_for_defrag).clone();
        if new_buffer.ty == VulkanBufferType::Uniform {
            if let Some(uniform_buffer) = uniform_buffer.upgrade() {
                lock(&uniform_buffer.state).buffer = new_buffer.clone();
            }
        } else if let Some((container, index)) = &container {
            new_buffer.set_container(Some(ContainerRef {
                container: Arc::downgrade(container),
                index: *index,
            }));
            let mut container_state = state(&container.state);
            if let Some(slot) = container_state.buffers.get_mut(*index) {
                *slot = new_buffer.clone();
            }
            if Arc::ptr_eq(&container_state.active_buffer, buffer) {
                container_state.active_buffer = new_buffer.clone();
            }
        }

        if uniform_buffer.strong_count() > 0 {
            *lock(&new_buffer.uniform_buffer_for_defrag) = uniform_buffer;
        }

        let mut dispose = lock(&self.dispose);
        self.release_buffer_internal(&mut dispose, buffer);
        Ok(())
    }

    /// The texture half of the defrag loop.
    fn defragment_texture(
        &self,
        command_buffer: &mut VulkanCommandBuffer,
        texture: &Arc<VulkanTexture>,
    ) -> Result<()> {
        let Some((container, index)) = lock(&texture.container)
            .as_ref()
            .and_then(|c| c.container.upgrade().map(|container| (container, c.index)))
        else {
            // Note (upstream): C reads the container of every texture not
            // marked for destroy; the released ones are marked.
            return Ok(());
        };

        let new_texture = match self.create_texture_internal(&container.info) {
            Ok(new_texture) => new_texture,
            Err(e) => {
                crate::log::error!(Category::Gpu, "{}", "Failed to allocate defrag buffer!");
                return Err(e);
            }
        };

        let info = &container.info;

        self.texture_transition_from_default_usage(
            command_buffer,
            VulkanTextureUsageMode::CopySource,
            texture,
        );

        self.full_texture_memory_barrier(
            command_buffer,
            VulkanTextureUsageMode::Uninitialized,
            VulkanTextureUsageMode::CopyDestination,
            &new_texture,
        );

        // Can only copy one mip level at a time
        for (src_subresource, dst_subresource) in
            texture.subresources.iter().zip(&new_texture.subresources)
        {
            let image_copy = VkImageCopy {
                src_subresource: VkImageSubresourceLayers {
                    aspect_mask: texture.aspect_flags,
                    mip_level: src_subresource.level,
                    base_array_layer: src_subresource.layer,
                    layer_count: 1,
                },
                src_offset: VkOffset3D { x: 0, y: 0, z: 0 },
                dst_subresource: VkImageSubresourceLayers {
                    aspect_mask: new_texture.aspect_flags,
                    mip_level: dst_subresource.level,
                    base_array_layer: dst_subresource.layer,
                    layer_count: 1,
                },
                dst_offset: VkOffset3D { x: 0, y: 0, z: 0 },
                // Note (upstream): a level of 32 or more shifts by the
                // type's width or more, which is undefined in C; the shift
                // count wraps here.
                extent: VkExtent3D {
                    width: info.width.wrapping_shr(src_subresource.level).max(1),
                    height: info.height.wrapping_shr(src_subresource.level).max(1),
                    depth: if info.texture_type == TextureType::Texture3D {
                        info.layer_count_or_depth
                    } else {
                        1
                    },
                },
            };

            // SAFETY: a command buffer being recorded and live images in
            // the copy layouts.
            unsafe {
                (self.dev.cmd_copy_image)(
                    command_buffer.command_buffer,
                    texture.image,
                    VK_IMAGE_LAYOUT_TRANSFER_SRC_OPTIMAL,
                    new_texture.image,
                    VK_IMAGE_LAYOUT_TRANSFER_DST_OPTIMAL,
                    1,
                    &image_copy,
                )
            };

            command_buffer.track_texture(texture);
            command_buffer.track_texture(&new_texture);
        }

        self.texture_transition_to_default_usage(
            command_buffer,
            VulkanTextureUsageMode::CopyDestination,
            &new_texture,
        );

        // re-point original container to new texture
        new_texture.set_container(Some(ContainerRef {
            container: Arc::downgrade(&container),
            index,
        }));
        {
            let mut container_state = state(&container.state);
            if let Some(slot) = container_state.textures.get_mut(index) {
                *slot = new_texture.clone();
            }
            if Arc::ptr_eq(texture, &container_state.active_texture) {
                container_state.active_texture = new_texture.clone();
            }
        }

        let mut dispose = lock(&self.dispose);
        self.release_texture_internal(&mut dispose, texture);
        Ok(())
    }

    /// Set the debug name of a Vulkan object, in debug mode with
    /// `VK_EXT_debug_utils` (the `VkDebugUtilsObjectNameInfoEXT` blocks of
    /// the creation functions).
    pub(super) fn set_object_name(&self, object_type: i32, object_handle: u64, name: &str) {
        if !(self.debug_mode && self.supports_debug_utils) {
            return;
        }
        let (Some(set_name), Ok(name)) = (
            self.inst.set_debug_utils_object_name_ext,
            CString::new(name),
        ) else {
            return;
        };
        let name_info = VkDebugUtilsObjectNameInfoEXT {
            s_type: VK_STRUCTURE_TYPE_DEBUG_UTILS_OBJECT_NAME_INFO_EXT,
            p_next: null(),
            object_type,
            object_handle,
            p_object_name: name.as_ptr(),
        };
        // SAFETY: a live object of the device and a NUL-terminated name.
        unsafe { set_name(self.logical_device, &name_info) };
    }
}

/// The framebuffer cache (`framebufferHashTable`).
pub(super) type FramebufferHashTable = HashMap<FramebufferHashTableKey, Arc<VulkanFramebuffer>>;
