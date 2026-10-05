// Rust translation of the shader, sampler, descriptor and pipeline parts of
// src/gpu/vulkan/SDL_gpu_vulkan.c from Simple DirectMedia Layer.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! Shaders, samplers, descriptor set layouts and pools, pipeline resource
//! layouts, and graphics and compute pipelines.
//!
//! The descriptor set layouts and the pipeline layouts are cached by the
//! counts of the resources they bind (upstream's hash tables); the layout
//! of a graphics pipeline is:
//!
//! * set 0: vertex resources (samplers, then storage textures, then storage
//!   buffers)
//! * set 1: vertex uniform buffers
//! * set 2: fragment resources
//! * set 3: fragment uniform buffers
//!
//! and of a compute pipeline:
//!
//! * set 0: samplers, then read-only textures, then read-only buffers
//! * set 1: read-write textures, then read-write buffers
//! * set 2: uniform buffers

use std::ffi::CString;
use std::ptr::null;
use std::sync::atomic::{AtomicI32, Ordering};
use std::sync::Arc;

use super::resources::VulkanSampler;
use super::tables::{
    sdl_to_vk_blend_factor, sdl_to_vk_blend_op, sdl_to_vk_compare_op, sdl_to_vk_cull_mode,
    sdl_to_vk_filter, sdl_to_vk_front_face, sdl_to_vk_primitive_type, sdl_to_vk_sample_count,
    sdl_to_vk_sampler_address_mode, sdl_to_vk_sampler_mipmap_mode, sdl_to_vk_stencil_op,
    sdl_to_vk_texture_format, sdl_to_vk_vertex_format, sdl_to_vk_vertex_input_rate,
};
use super::{lock, VulkanRenderer};
use crate::error::Result;
use crate::gpu::sysgpu::{
    ComputePipelineHeader, GraphicsPipelineHeader, MAX_COLOR_TARGET_BINDINGS,
    MAX_COMPUTE_WRITE_BUFFERS, MAX_COMPUTE_WRITE_TEXTURES, MAX_STORAGE_BUFFERS_PER_STAGE,
    MAX_STORAGE_TEXTURES_PER_STAGE, MAX_TEXTURE_SAMPLERS_PER_STAGE, MAX_UNIFORM_BUFFERS_PER_STAGE,
};
use crate::gpu::{
    ColorComponentFlags, ComputePipelineCreateInfo, FillMode, GraphicsPipelineCreateInfo,
    GraphicsPipelineTargetInfo, LoadOp, PrimitiveType, SamplerCreateInfo, ShaderCreateInfo,
    ShaderFormat, ShaderStage, StoreOp,
};
use crate::log::Category;
use crate::video::vk::*;

/// The size of the descriptor pools. Translation of `DESCRIPTOR_POOL_SIZE`.
pub(super) const DESCRIPTOR_POOL_SIZE: u32 = 128;

/// The most bindings of a descriptor set layout.
const MAX_DESCRIPTOR_SET_BINDINGS: usize = (MAX_TEXTURE_SAMPLERS_PER_STAGE
    + MAX_STORAGE_TEXTURES_PER_STAGE
    + MAX_STORAGE_BUFFERS_PER_STAGE
    + MAX_COMPUTE_WRITE_TEXTURES
    + MAX_COMPUTE_WRITE_BUFFERS
    + MAX_UNIFORM_BUFFERS_PER_STAGE) as usize;

/// Translation of `VulkanShader`.
#[derive(Debug)]
pub(super) struct VulkanShader {
    pub(super) shader_module: VkShaderModule,
    pub(super) entrypoint_name: CString,
    pub(super) stage: ShaderStage,
    pub(super) num_samplers: u32,
    pub(super) num_storage_textures: u32,
    pub(super) num_storage_buffers: u32,
    pub(super) num_uniform_buffers: u32,
    pub(super) reference_count: AtomicI32,
}

/// Translation of `DescriptorSetLayoutHashTableKey`.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub(super) struct DescriptorSetLayoutHashTableKey {
    pub(super) shader_stage: VkFlags,
    // Category 1: read resources
    pub(super) sampler_count: u32,
    pub(super) storage_buffer_count: u32,
    pub(super) storage_texture_count: u32,
    // Category 2: write resources
    pub(super) write_storage_buffer_count: u32,
    pub(super) write_storage_texture_count: u32,
    // Category 3: uniform buffers
    pub(super) uniform_buffer_count: u32,
}

/// Translation of `DescriptorSetLayout`.
#[derive(Debug)]
pub(super) struct DescriptorSetLayout {
    /// `DescriptorSetLayoutID`: the index of its pool in the descriptor set
    /// caches.
    pub(super) id: u32,
    pub(super) descriptor_set_layout: VkDescriptorSetLayout,

    // Category 1: read resources
    pub(super) sampler_count: u32,
    pub(super) storage_buffer_count: u32,
    pub(super) storage_texture_count: u32,
    // Category 2: write resources
    pub(super) write_storage_buffer_count: u32,
    pub(super) write_storage_texture_count: u32,
    // Category 3: uniform buffers
    pub(super) uniform_buffer_count: u32,
}

/// Translation of `GraphicsPipelineResourceLayoutHashTableKey`.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub(super) struct GraphicsPipelineResourceLayoutHashTableKey {
    pub(super) vertex_sampler_count: u32,
    pub(super) vertex_storage_texture_count: u32,
    pub(super) vertex_storage_buffer_count: u32,
    pub(super) vertex_uniform_buffer_count: u32,

    pub(super) fragment_sampler_count: u32,
    pub(super) fragment_storage_texture_count: u32,
    pub(super) fragment_storage_buffer_count: u32,
    pub(super) fragment_uniform_buffer_count: u32,
}

/// Translation of `VulkanGraphicsPipelineResourceLayout`.
#[derive(Debug)]
pub(super) struct VulkanGraphicsPipelineResourceLayout {
    pub(super) pipeline_layout: VkPipelineLayout,

    /*
     * Descriptor set layout is as follows:
     * 0: vertex resources
     * 1: vertex uniform buffers
     * 2: fragment resources
     * 3: fragment uniform buffers
     */
    #[allow(dead_code)] // (part 2: descriptor sets)
    pub(super) descriptor_set_layouts: [Arc<DescriptorSetLayout>; 4],

    pub(super) vertex_sampler_count: u32,
    pub(super) vertex_storage_texture_count: u32,
    pub(super) vertex_storage_buffer_count: u32,
    pub(super) vertex_uniform_buffer_count: u32,

    pub(super) fragment_sampler_count: u32,
    pub(super) fragment_storage_texture_count: u32,
    pub(super) fragment_storage_buffer_count: u32,
    pub(super) fragment_uniform_buffer_count: u32,
}

/// Translation of `VulkanGraphicsPipeline`.
#[derive(Debug)]
pub(super) struct VulkanGraphicsPipeline {
    pub(super) pipeline: VkPipeline,
    #[allow(dead_code)] // (part 2: draws)
    pub(super) primitive_type: PrimitiveType,

    #[allow(dead_code)] // (part 2: binding)
    pub(super) resource_layout: Arc<VulkanGraphicsPipelineResourceLayout>,

    pub(super) vertex_shader: Arc<VulkanShader>,
    pub(super) fragment_shader: Arc<VulkanShader>,

    pub(super) reference_count: AtomicI32,
}

/// Translation of `ComputePipelineResourceLayoutHashTableKey`.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub(super) struct ComputePipelineResourceLayoutHashTableKey {
    pub(super) sampler_count: u32,
    pub(super) readonly_storage_texture_count: u32,
    pub(super) readonly_storage_buffer_count: u32,
    pub(super) read_write_storage_texture_count: u32,
    pub(super) read_write_storage_buffer_count: u32,
    pub(super) uniform_buffer_count: u32,
}

/// Translation of `VulkanComputePipelineResourceLayout`.
#[derive(Debug)]
pub(super) struct VulkanComputePipelineResourceLayout {
    pub(super) pipeline_layout: VkPipelineLayout,

    /*
     * Descriptor set layout is as follows:
     * 0: samplers, then read-only textures, then read-only buffers
     * 1: write-only textures, then write-only buffers
     * 2: uniform buffers
     */
    #[allow(dead_code)] // (part 2: descriptor sets)
    pub(super) descriptor_set_layouts: [Arc<DescriptorSetLayout>; 3],

    pub(super) num_samplers: u32,
    pub(super) num_readonly_storage_textures: u32,
    pub(super) num_readonly_storage_buffers: u32,
    pub(super) num_read_write_storage_textures: u32,
    pub(super) num_read_write_storage_buffers: u32,
    pub(super) num_uniform_buffers: u32,
}

/// Translation of `VulkanComputePipeline`.
#[derive(Debug)]
pub(super) struct VulkanComputePipeline {
    pub(super) shader_module: VkShaderModule,
    pub(super) pipeline: VkPipeline,
    pub(super) resource_layout: Arc<VulkanComputePipelineResourceLayout>,
    pub(super) reference_count: AtomicI32,
}

/// Translation of `VulkanFramebuffer`.
#[derive(Debug)]
pub(super) struct VulkanFramebuffer {
    pub(super) framebuffer: VkFramebuffer,
    pub(super) reference_count: AtomicI32,
}

/// Translation of `RenderPassColorTargetDescription`.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Default)]
pub(super) struct RenderPassColorTargetDescription {
    pub(super) format: VkFormat,
    pub(super) load_op: LoadOp,
    pub(super) store_op: StoreOp,
}

/// Translation of `RenderPassDepthStencilTargetDescription`.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Default)]
pub(super) struct RenderPassDepthStencilTargetDescription {
    pub(super) format: VkFormat,
    pub(super) load_op: LoadOp,
    pub(super) store_op: StoreOp,
    pub(super) stencil_load_op: LoadOp,
    pub(super) stencil_store_op: StoreOp,
}

/// Translation of `RenderPassHashTableKey`. The entries past the counts
/// are left at their defaults, so whole keys compare like upstream's
/// `VULKAN_INTERNAL_RenderPassHashKeyMatch()`.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Default)]
pub(super) struct RenderPassHashTableKey {
    pub(super) color_target_descriptions:
        [RenderPassColorTargetDescription; MAX_COLOR_TARGET_BINDINGS as usize],
    pub(super) num_color_targets: u32,
    pub(super) resolve_target_formats: [VkFormat; MAX_COLOR_TARGET_BINDINGS as usize],
    pub(super) num_resolve_targets: u32,
    pub(super) depth_stencil_target_description: RenderPassDepthStencilTargetDescription,
    pub(super) sample_count: VkFlags,
}

/// Translation of `FramebufferHashTableKey`. The entries past the counts
/// are left null, so whole keys compare like upstream's
/// `VULKAN_INTERNAL_FramebufferHashKeyMatch()`.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Default)]
pub(super) struct FramebufferHashTableKey {
    pub(super) color_attachment_views: [VkImageView; MAX_COLOR_TARGET_BINDINGS as usize],
    pub(super) num_color_targets: u32,
    pub(super) resolve_attachment_views: [VkImageView; MAX_COLOR_TARGET_BINDINGS as usize],
    pub(super) num_resolve_attachments: u32,
    pub(super) depth_stencil_attachment_view: VkImageView,
    pub(super) width: u32,
    pub(super) height: u32,
}

impl FramebufferHashTableKey {
    /// Whether the framebuffer uses a view (`CheckOneFramebufferForRemoval()`).
    pub(super) fn contains_view(&self, view: VkImageView) -> bool {
        self.color_attachment_views[..self.num_color_targets as usize].contains(&view)
            || self.resolve_attachment_views[..self.num_resolve_attachments as usize]
                .contains(&view)
            || self.depth_stencil_attachment_view == view
    }
}

/// Translation of `DescriptorSetPool`.
#[derive(Debug, Default)]
pub(super) struct DescriptorSetPool {
    // It's a pool... of pools!!!
    pub(super) descriptor_pools: Vec<VkDescriptorPool>,

    // We'll just manage the descriptor sets ourselves instead of freeing the sets
    pub(super) descriptor_sets: Vec<VkDescriptorSet>,
    pub(super) descriptor_set_index: usize,
}

/// A command buffer acquires a cache at command buffer acquisition time.
/// Translation of `DescriptorSetCache`.
#[derive(Debug, Default)]
pub(super) struct DescriptorSetCache {
    // Pools are indexed by DescriptorSetLayoutID which increases monotonically
    // There's only a certain number of maximum layouts possible since we de-duplicate them.
    pub(super) pools: Vec<DescriptorSetPool>,
}

/// Whether the code is SPIR-V. Translation of
/// `VULKAN_INTERNAL_IsValidShaderBytecode()`.
pub(super) fn is_valid_shader_bytecode(code: &[u8]) -> bool {
    // SPIR-V bytecode has a 4 byte header containing 0x07230203. SPIR-V is
    // defined as a stream of words and not a stream of bytes so both byte
    // orders need to be considered.
    //
    // FIXME: It is uncertain if drivers are able to load both byte orders. If
    // needed we may need to do an optional swizzle internally so apps can
    // continue to treat shader code as an opaque blob.
    if code.len() < 4 {
        return false;
    }
    let magic: u32 = 0x07230203;
    let magic_inv: u32 = 0x03022307;
    code[..4] == magic.to_ne_bytes() || code[..4] == magic_inv.to_ne_bytes()
}

/// The code as words, for `pCode` (upstream casts the byte pointer, which
/// needn't be aligned here).
fn code_words(code: &[u8]) -> Vec<u32> {
    code.chunks(4)
        .map(|chunk| {
            let mut word = [0u8; 4];
            word[..chunk.len()].copy_from_slice(chunk);
            u32::from_ne_bytes(word)
        })
        .collect()
}

/// `SDL_GetStringProperty(props, name, NULL)` of an optional group.
fn string_property(props: &Option<crate::properties::Properties>, name: &str) -> Option<String> {
    props.as_ref().and_then(|p| p.get_string(name))
}

impl VulkanRenderer {
    /// Translation of `SDLToVK_PolygonMode()`.
    fn sdl_to_vk_polygon_mode(&self, mode: FillMode) -> i32 {
        if mode == FillMode::Fill {
            return VK_POLYGON_MODE_FILL; // always available!
        }

        if self.supports_fill_mode_non_solid && mode == FillMode::Line {
            return VK_POLYGON_MODE_LINE;
        }

        if !self.fill_mode_only_warning.swap(true, Ordering::SeqCst) {
            crate::log::warn!(
                Category::Gpu,
                "Unsupported fill mode requested, using FILL!"
            );
        }
        VK_POLYGON_MODE_FILL
    }

    // Descriptor pools

    /// Translation of `VULKAN_INTERNAL_AllocateDescriptorSets()`.
    fn allocate_descriptor_sets(
        &self,
        descriptor_pool: VkDescriptorPool,
        descriptor_set_layout: VkDescriptorSetLayout,
        descriptor_set_array: &mut [VkDescriptorSet],
    ) -> Result<()> {
        let descriptor_set_layouts = vec![descriptor_set_layout; descriptor_set_array.len()];

        let descriptor_set_allocate_info = VkDescriptorSetAllocateInfo {
            s_type: VK_STRUCTURE_TYPE_DESCRIPTOR_SET_ALLOCATE_INFO,
            p_next: null(),
            descriptor_pool,
            descriptor_set_count: descriptor_set_array.len() as u32,
            p_set_layouts: descriptor_set_layouts.as_ptr(),
        };

        // SAFETY: a valid allocate info and room for the sets.
        let vulkan_result = unsafe {
            (self.dev.allocate_descriptor_sets)(
                self.logical_device,
                &descriptor_set_allocate_info,
                descriptor_set_array.as_mut_ptr(),
            )
        };

        self.check(vulkan_result, "vkAllocateDescriptorSets")
    }

    /// Add a descriptor pool of `DESCRIPTOR_POOL_SIZE` sets to a pool.
    /// Translation of `VULKAN_INTERNAL_AllocateDescriptorsFromPool()`.
    pub(super) fn allocate_descriptors_from_pool(
        &self,
        descriptor_set_layout: &DescriptorSetLayout,
        descriptor_set_pool: &mut DescriptorSetPool,
    ) -> Result<()> {
        let mut descriptor_pool_sizes =
            [VkDescriptorPoolSize::default(); MAX_DESCRIPTOR_SET_BINDINGS];
        let layout = descriptor_set_layout;

        // Category 1
        let samplers = layout.sampler_count as usize;
        let storage_textures = samplers + layout.storage_texture_count as usize;
        let storage_buffers = storage_textures + layout.storage_buffer_count as usize;
        for size in &mut descriptor_pool_sizes[..samplers] {
            size.ty = VK_DESCRIPTOR_TYPE_COMBINED_IMAGE_SAMPLER;
            size.descriptor_count = DESCRIPTOR_POOL_SIZE;
        }

        for size in &mut descriptor_pool_sizes[samplers..storage_textures] {
            size.ty = VK_DESCRIPTOR_TYPE_SAMPLED_IMAGE; // Yes, we are declaring the storage image as a sampled image, because shaders are stupid.
            size.descriptor_count = DESCRIPTOR_POOL_SIZE;
        }

        for size in &mut descriptor_pool_sizes[storage_textures..storage_buffers] {
            size.ty = VK_DESCRIPTOR_TYPE_STORAGE_BUFFER;
            size.descriptor_count = DESCRIPTOR_POOL_SIZE;
        }

        // Category 2
        let write_textures = layout.write_storage_texture_count as usize;
        let write_buffers = write_textures + layout.write_storage_buffer_count as usize;
        for size in &mut descriptor_pool_sizes[..write_textures] {
            size.ty = VK_DESCRIPTOR_TYPE_STORAGE_IMAGE;
            size.descriptor_count = DESCRIPTOR_POOL_SIZE;
        }

        for size in &mut descriptor_pool_sizes[write_textures..write_buffers] {
            size.ty = VK_DESCRIPTOR_TYPE_STORAGE_BUFFER;
            size.descriptor_count = DESCRIPTOR_POOL_SIZE;
        }

        // Category 3
        for size in &mut descriptor_pool_sizes[..layout.uniform_buffer_count as usize] {
            size.ty = VK_DESCRIPTOR_TYPE_UNIFORM_BUFFER_DYNAMIC;
            size.descriptor_count = DESCRIPTOR_POOL_SIZE;
        }

        // Note (upstream): the categories are mutually exclusive, so the
        // sizes past the first category's (zeroed here, uninitialized in C)
        // are only counted when no other category is set.
        let descriptor_pool_info = VkDescriptorPoolCreateInfo {
            s_type: VK_STRUCTURE_TYPE_DESCRIPTOR_POOL_CREATE_INFO,
            p_next: null(),
            flags: 0,
            max_sets: DESCRIPTOR_POOL_SIZE,
            pool_size_count: layout.sampler_count
                + layout.storage_texture_count
                + layout.storage_buffer_count
                + layout.write_storage_texture_count
                + layout.write_storage_buffer_count
                + layout.uniform_buffer_count,
            p_pool_sizes: descriptor_pool_sizes.as_ptr(),
        };

        let mut pool = VK_NULL_HANDLE;
        // SAFETY: a valid create info.
        let vulkan_result = unsafe {
            (self.dev.create_descriptor_pool)(
                self.logical_device,
                &descriptor_pool_info,
                null(),
                &mut pool,
            )
        };

        self.check(vulkan_result, "vkCreateDescriptorPool")?;

        descriptor_set_pool.descriptor_pools.push(pool);

        let start = descriptor_set_pool.descriptor_sets.len();
        descriptor_set_pool
            .descriptor_sets
            .resize(start + DESCRIPTOR_POOL_SIZE as usize, VK_NULL_HANDLE);

        if let Err(e) = self.allocate_descriptor_sets(
            pool,
            layout.descriptor_set_layout,
            &mut descriptor_set_pool.descriptor_sets[start..],
        ) {
            // (upstream's count stays where it was)
            descriptor_set_pool.descriptor_sets.truncate(start);
            return Err(e);
        }

        Ok(())
    }

    /// A command buffer's descriptor set cache, from the pool or new.
    /// Translation of `VULKAN_INTERNAL_AcquireDescriptorSetCache()`.
    #[allow(dead_code)] // (part 2: AcquireCommandBuffer)
    pub(super) fn acquire_descriptor_set_cache(&self) -> DescriptorSetCache {
        lock(&self.descriptor_set_cache_pool)
            .pop()
            .unwrap_or_default()
    }

    /// Translation of `VULKAN_INTERNAL_ReturnDescriptorSetCacheToPool()`.
    #[allow(dead_code)] // (part 2: CleanCommandBuffer)
    pub(super) fn return_descriptor_set_cache_to_pool(
        &self,
        mut descriptor_set_cache: DescriptorSetCache,
    ) {
        for pool in &mut descriptor_set_cache.pools {
            pool.descriptor_set_index = 0;
        }

        lock(&self.descriptor_set_cache_pool).push(descriptor_set_cache);
    }

    /// The next descriptor set of a layout in a command buffer's cache.
    /// Translation of `VULKAN_INTERNAL_FetchDescriptorSet()`.
    #[allow(dead_code)] // (part 2: binding descriptor sets)
    pub(super) fn fetch_descriptor_set(
        &self,
        descriptor_set_cache: &mut DescriptorSetCache,
        descriptor_set_layout: &DescriptorSetLayout,
    ) -> VkDescriptorSet {
        // Grow the pool to meet the descriptor set layout ID
        let id = descriptor_set_layout.id as usize;
        if id >= descriptor_set_cache.pools.len() {
            descriptor_set_cache
                .pools
                .resize_with(id + 1, DescriptorSetPool::default);
        }

        let pool = &mut descriptor_set_cache.pools[id];

        if pool.descriptor_set_index == pool.descriptor_sets.len()
            && self
                .allocate_descriptors_from_pool(descriptor_set_layout, pool)
                .is_err()
        {
            return VK_NULL_HANDLE;
        }

        let descriptor_set = pool.descriptor_sets[pool.descriptor_set_index];
        pool.descriptor_set_index += 1;

        descriptor_set
    }

    /// Translation of `VULKAN_INTERNAL_DestroyDescriptorSetCache()`.
    pub(super) fn destroy_descriptor_set_cache(&self, descriptor_set_cache: DescriptorSetCache) {
        for pool in &descriptor_set_cache.pools {
            for &descriptor_pool in &pool.descriptor_pools {
                // SAFETY: a pool of the device the GPU no longer uses.
                unsafe {
                    (self.dev.destroy_descriptor_pool)(self.logical_device, descriptor_pool, null())
                };
            }
        }
    }

    /// The descriptor set layout for some resource counts, from the cache or
    /// new. Translation of `VULKAN_INTERNAL_FetchDescriptorSetLayout()`.
    ///
    /// NOTE: these categories should be mutually exclusive
    #[allow(clippy::too_many_arguments)]
    pub(super) fn fetch_descriptor_set_layout(
        &self,
        shader_stage: VkFlags,
        // Category 1: read resources
        sampler_count: u32,
        storage_texture_count: u32,
        storage_buffer_count: u32,
        // Category 2: write resources
        write_storage_texture_count: u32,
        write_storage_buffer_count: u32,
        // Category 3: uniform buffers
        uniform_buffer_count: u32,
    ) -> Result<Arc<DescriptorSetLayout>> {
        let key = DescriptorSetLayoutHashTableKey {
            shader_stage,
            sampler_count,
            storage_buffer_count,
            storage_texture_count,
            write_storage_buffer_count,
            write_storage_texture_count,
            uniform_buffer_count,
        };

        let mut table = lock(&self.descriptor_set_layout_hash_table);

        if let Some(layout) = table.get(&key) {
            return Ok(layout.clone());
        }

        let mut descriptor_set_layout_bindings =
            [VkDescriptorSetLayoutBinding::default(); MAX_DESCRIPTOR_SET_BINDINGS];

        let mut binding = |i: u32, descriptor_type: i32| {
            descriptor_set_layout_bindings[i as usize] = VkDescriptorSetLayoutBinding {
                binding: i,
                descriptor_type,
                descriptor_count: 1,
                stage_flags: shader_stage,
                p_immutable_samplers: null(),
            };
        };

        // Category 1
        for i in 0..sampler_count {
            binding(i, VK_DESCRIPTOR_TYPE_COMBINED_IMAGE_SAMPLER);
        }

        for i in sampler_count..sampler_count + storage_texture_count {
            binding(i, VK_DESCRIPTOR_TYPE_SAMPLED_IMAGE); // Yes, we are declaring the storage image as a sampled image, because shaders are stupid.
        }

        for i in sampler_count + storage_texture_count
            ..sampler_count + storage_texture_count + storage_buffer_count
        {
            binding(i, VK_DESCRIPTOR_TYPE_STORAGE_BUFFER);
        }

        // Category 2
        for i in 0..write_storage_texture_count {
            binding(i, VK_DESCRIPTOR_TYPE_STORAGE_IMAGE);
        }

        for i in
            write_storage_texture_count..write_storage_texture_count + write_storage_buffer_count
        {
            binding(i, VK_DESCRIPTOR_TYPE_STORAGE_BUFFER);
        }

        // Category 3
        for i in 0..uniform_buffer_count {
            binding(i, VK_DESCRIPTOR_TYPE_UNIFORM_BUFFER_DYNAMIC);
        }

        let descriptor_set_layout_create_info = VkDescriptorSetLayoutCreateInfo {
            s_type: VK_STRUCTURE_TYPE_DESCRIPTOR_SET_LAYOUT_CREATE_INFO,
            p_next: null(),
            flags: 0,
            binding_count: sampler_count
                + storage_texture_count
                + storage_buffer_count
                + write_storage_texture_count
                + write_storage_buffer_count
                + uniform_buffer_count,
            p_bindings: descriptor_set_layout_bindings.as_ptr(),
        };

        let mut descriptor_set_layout = VK_NULL_HANDLE;
        // SAFETY: a valid create info.
        let vulkan_result = unsafe {
            (self.dev.create_descriptor_set_layout)(
                self.logical_device,
                &descriptor_set_layout_create_info,
                null(),
                &mut descriptor_set_layout,
            )
        };

        if vulkan_result != VK_SUCCESS {
            drop(table);
            return Err(self.vk_error(vulkan_result, "vkCreateDescriptorSetLayout"));
        }

        let layout = Arc::new(DescriptorSetLayout {
            // (SDL_AtomicIncRef() returns the previous value)
            id: self.layout_resource_id.fetch_add(1, Ordering::SeqCst),
            descriptor_set_layout,
            sampler_count,
            storage_buffer_count,
            storage_texture_count,
            write_storage_buffer_count,
            write_storage_texture_count,
            uniform_buffer_count,
        });

        table.insert(key, layout.clone());

        Ok(layout)
    }

    /// Translation of `VULKAN_INTERNAL_DestroyDescriptorSetLayout()`.
    pub(super) fn destroy_descriptor_set_layout(&self, layout: &DescriptorSetLayout) {
        if layout.descriptor_set_layout != VK_NULL_HANDLE {
            // SAFETY: a layout of the device, no longer used.
            unsafe {
                (self.dev.destroy_descriptor_set_layout)(
                    self.logical_device,
                    layout.descriptor_set_layout,
                    null(),
                )
            };
        }
    }

    /// Translation of `VULKAN_INTERNAL_DestroyGraphicsPipelineResourceLayout()`
    /// (and `VULKAN_INTERNAL_DestroyComputePipelineResourceLayout()`).
    pub(super) fn destroy_pipeline_layout(&self, pipeline_layout: VkPipelineLayout) {
        if pipeline_layout != VK_NULL_HANDLE {
            // SAFETY: a layout of the device, no longer used.
            unsafe {
                (self.dev.destroy_pipeline_layout)(self.logical_device, pipeline_layout, null())
            };
        }
    }

    /// The pipeline layout for a vertex and a fragment shader, from the cache
    /// or new. Translation of
    /// `VULKAN_INTERNAL_FetchGraphicsPipelineResourceLayout()`.
    pub(super) fn fetch_graphics_pipeline_resource_layout(
        &self,
        vertex_shader: &VulkanShader,
        fragment_shader: &VulkanShader,
    ) -> Result<Arc<VulkanGraphicsPipelineResourceLayout>> {
        let key = GraphicsPipelineResourceLayoutHashTableKey {
            vertex_sampler_count: vertex_shader.num_samplers,
            vertex_storage_texture_count: vertex_shader.num_storage_textures,
            vertex_storage_buffer_count: vertex_shader.num_storage_buffers,
            vertex_uniform_buffer_count: vertex_shader.num_uniform_buffers,
            fragment_sampler_count: fragment_shader.num_samplers,
            fragment_storage_texture_count: fragment_shader.num_storage_textures,
            fragment_storage_buffer_count: fragment_shader.num_storage_buffers,
            fragment_uniform_buffer_count: fragment_shader.num_uniform_buffers,
        };

        let mut table = lock(&self.graphics_pipeline_resource_layout_hash_table);

        if let Some(layout) = table.get(&key) {
            return Ok(layout.clone());
        }

        // Note (upstream): C doesn't check these fetches (a failed one
        // would be a NULL dereference).
        let descriptor_set_layouts = [
            self.fetch_descriptor_set_layout(
                VK_SHADER_STAGE_VERTEX_BIT,
                vertex_shader.num_samplers,
                vertex_shader.num_storage_textures,
                vertex_shader.num_storage_buffers,
                0,
                0,
                0,
            )?,
            self.fetch_descriptor_set_layout(
                VK_SHADER_STAGE_VERTEX_BIT,
                0,
                0,
                0,
                0,
                0,
                vertex_shader.num_uniform_buffers,
            )?,
            self.fetch_descriptor_set_layout(
                VK_SHADER_STAGE_FRAGMENT_BIT,
                fragment_shader.num_samplers,
                fragment_shader.num_storage_textures,
                fragment_shader.num_storage_buffers,
                0,
                0,
                0,
            )?,
            self.fetch_descriptor_set_layout(
                VK_SHADER_STAGE_FRAGMENT_BIT,
                0,
                0,
                0,
                0,
                0,
                fragment_shader.num_uniform_buffers,
            )?,
        ];

        let vk_descriptor_set_layouts: Vec<VkDescriptorSetLayout> = descriptor_set_layouts
            .iter()
            .map(|l| l.descriptor_set_layout)
            .collect();

        // Create the pipeline layout

        let pipeline_layout_create_info = VkPipelineLayoutCreateInfo {
            s_type: VK_STRUCTURE_TYPE_PIPELINE_LAYOUT_CREATE_INFO,
            p_next: null(),
            flags: 0,
            set_layout_count: 4,
            p_set_layouts: vk_descriptor_set_layouts.as_ptr(),
            push_constant_range_count: 0,
            p_push_constant_ranges: null(),
        };

        let mut pipeline_layout = VK_NULL_HANDLE;
        // SAFETY: a valid create info with live set layouts.
        let vulkan_result = unsafe {
            (self.dev.create_pipeline_layout)(
                self.logical_device,
                &pipeline_layout_create_info,
                null(),
                &mut pipeline_layout,
            )
        };

        if vulkan_result != VK_SUCCESS {
            drop(table);
            return Err(self.vk_error(vulkan_result, "vkCreatePipelineLayout"));
        }

        let pipeline_resource_layout = Arc::new(VulkanGraphicsPipelineResourceLayout {
            pipeline_layout,
            descriptor_set_layouts,
            vertex_sampler_count: vertex_shader.num_samplers,
            vertex_storage_texture_count: vertex_shader.num_storage_textures,
            vertex_storage_buffer_count: vertex_shader.num_storage_buffers,
            vertex_uniform_buffer_count: vertex_shader.num_uniform_buffers,
            fragment_sampler_count: fragment_shader.num_samplers,
            fragment_storage_texture_count: fragment_shader.num_storage_textures,
            fragment_storage_buffer_count: fragment_shader.num_storage_buffers,
            fragment_uniform_buffer_count: fragment_shader.num_uniform_buffers,
        });

        table.insert(key, pipeline_resource_layout.clone());

        Ok(pipeline_resource_layout)
    }

    /// The pipeline layout for a compute pipeline, from the cache or new.
    /// Translation of `VULKAN_INTERNAL_FetchComputePipelineResourceLayout()`.
    pub(super) fn fetch_compute_pipeline_resource_layout(
        &self,
        createinfo: &ComputePipelineCreateInfo<'_>,
    ) -> Result<Arc<VulkanComputePipelineResourceLayout>> {
        let key = ComputePipelineResourceLayoutHashTableKey {
            sampler_count: createinfo.num_samplers,
            readonly_storage_texture_count: createinfo.num_readonly_storage_textures,
            readonly_storage_buffer_count: createinfo.num_readonly_storage_buffers,
            read_write_storage_texture_count: createinfo.num_readwrite_storage_textures,
            read_write_storage_buffer_count: createinfo.num_readwrite_storage_buffers,
            uniform_buffer_count: createinfo.num_uniform_buffers,
        };

        let mut table = lock(&self.compute_pipeline_resource_layout_hash_table);

        if let Some(layout) = table.get(&key) {
            return Ok(layout.clone());
        }

        // Note (upstream): C doesn't check these fetches.
        let descriptor_set_layouts = [
            self.fetch_descriptor_set_layout(
                VK_SHADER_STAGE_COMPUTE_BIT,
                createinfo.num_samplers,
                createinfo.num_readonly_storage_textures,
                createinfo.num_readonly_storage_buffers,
                0,
                0,
                0,
            )?,
            self.fetch_descriptor_set_layout(
                VK_SHADER_STAGE_COMPUTE_BIT,
                0,
                0,
                0,
                createinfo.num_readwrite_storage_textures,
                createinfo.num_readwrite_storage_buffers,
                0,
            )?,
            self.fetch_descriptor_set_layout(
                VK_SHADER_STAGE_COMPUTE_BIT,
                0,
                0,
                0,
                0,
                0,
                createinfo.num_uniform_buffers,
            )?,
        ];

        let vk_descriptor_set_layouts: Vec<VkDescriptorSetLayout> = descriptor_set_layouts
            .iter()
            .map(|l| l.descriptor_set_layout)
            .collect();

        // Create the pipeline layout

        let pipeline_layout_create_info = VkPipelineLayoutCreateInfo {
            s_type: VK_STRUCTURE_TYPE_PIPELINE_LAYOUT_CREATE_INFO,
            p_next: null(),
            flags: 0,
            set_layout_count: 3,
            p_set_layouts: vk_descriptor_set_layouts.as_ptr(),
            push_constant_range_count: 0,
            p_push_constant_ranges: null(),
        };

        let mut pipeline_layout = VK_NULL_HANDLE;
        // SAFETY: a valid create info with live set layouts.
        let vulkan_result = unsafe {
            (self.dev.create_pipeline_layout)(
                self.logical_device,
                &pipeline_layout_create_info,
                null(),
                &mut pipeline_layout,
            )
        };

        if vulkan_result != VK_SUCCESS {
            drop(table);
            return Err(self.vk_error(vulkan_result, "vkCreatePipelineLayout"));
        }

        let pipeline_resource_layout = Arc::new(VulkanComputePipelineResourceLayout {
            pipeline_layout,
            descriptor_set_layouts,
            num_samplers: createinfo.num_samplers,
            num_readonly_storage_textures: createinfo.num_readonly_storage_textures,
            num_readonly_storage_buffers: createinfo.num_readonly_storage_buffers,
            num_read_write_storage_textures: createinfo.num_readwrite_storage_textures,
            num_read_write_storage_buffers: createinfo.num_readwrite_storage_buffers,
            num_uniform_buffers: createinfo.num_uniform_buffers,
        });

        table.insert(key, pipeline_resource_layout.clone());

        Ok(pipeline_resource_layout)
    }

    /// A render pass compatible with a pipeline's targets, for creating the
    /// pipeline. Translation of `VULKAN_INTERNAL_CreateTransientRenderPass()`.
    fn create_transient_render_pass(
        &self,
        target_info: &GraphicsPipelineTargetInfo<'_>,
        sample_count: VkFlags,
    ) -> Result<VkRenderPass> {
        let mut attachment_descriptions =
            Vec::with_capacity(target_info.color_target_descriptions.len() + 1);
        let mut color_attachment_references =
            Vec::with_capacity(target_info.color_target_descriptions.len());

        for attachment_description in target_info.color_target_descriptions {
            color_attachment_references.push(VkAttachmentReference {
                attachment: attachment_descriptions.len() as u32,
                layout: VK_IMAGE_LAYOUT_COLOR_ATTACHMENT_OPTIMAL,
            });

            attachment_descriptions.push(VkAttachmentDescription {
                flags: 0,
                format: sdl_to_vk_texture_format(attachment_description.format),
                samples: sample_count,
                load_op: VK_ATTACHMENT_LOAD_OP_DONT_CARE,
                store_op: VK_ATTACHMENT_STORE_OP_DONT_CARE,
                stencil_load_op: VK_ATTACHMENT_LOAD_OP_DONT_CARE,
                stencil_store_op: VK_ATTACHMENT_STORE_OP_DONT_CARE,
                initial_layout: VK_IMAGE_LAYOUT_COLOR_ATTACHMENT_OPTIMAL,
                final_layout: VK_IMAGE_LAYOUT_COLOR_ATTACHMENT_OPTIMAL,
            });
        }

        let mut subpass = VkSubpassDescription {
            pipeline_bind_point: VK_PIPELINE_BIND_POINT_GRAPHICS,
            flags: 0,
            input_attachment_count: 0,
            p_input_attachments: null(),
            color_attachment_count: target_info.color_target_descriptions.len() as u32,
            p_color_attachments: color_attachment_references.as_ptr(),
            preserve_attachment_count: 0,
            p_preserve_attachments: null(),
            p_depth_stencil_attachment: null(),
            // Resolve attachments aren't needed for transient passes
            p_resolve_attachments: null(),
        };

        let depth_stencil_attachment_reference;
        if target_info.has_depth_stencil_target {
            depth_stencil_attachment_reference = VkAttachmentReference {
                attachment: attachment_descriptions.len() as u32,
                layout: VK_IMAGE_LAYOUT_DEPTH_STENCIL_ATTACHMENT_OPTIMAL,
            };

            attachment_descriptions.push(VkAttachmentDescription {
                flags: 0,
                format: sdl_to_vk_texture_format(target_info.depth_stencil_format),
                samples: sample_count,
                load_op: VK_ATTACHMENT_LOAD_OP_DONT_CARE,
                store_op: VK_ATTACHMENT_STORE_OP_DONT_CARE,
                stencil_load_op: VK_ATTACHMENT_LOAD_OP_DONT_CARE,
                stencil_store_op: VK_ATTACHMENT_STORE_OP_DONT_CARE,
                initial_layout: VK_IMAGE_LAYOUT_DEPTH_STENCIL_ATTACHMENT_OPTIMAL,
                final_layout: VK_IMAGE_LAYOUT_DEPTH_STENCIL_ATTACHMENT_OPTIMAL,
            });

            subpass.p_depth_stencil_attachment = &depth_stencil_attachment_reference;
        }

        let render_pass_create_info = VkRenderPassCreateInfo {
            s_type: VK_STRUCTURE_TYPE_RENDER_PASS_CREATE_INFO,
            p_next: null(),
            flags: 0,
            p_attachments: attachment_descriptions.as_ptr(),
            attachment_count: attachment_descriptions.len() as u32,
            subpass_count: 1,
            p_subpasses: &subpass,
            dependency_count: 0,
            p_dependencies: null(),
        };

        let mut render_pass = VK_NULL_HANDLE;
        // SAFETY: a valid create info whose arrays outlive the call.
        let result = unsafe {
            (self.dev.create_render_pass)(
                self.logical_device,
                &render_pass_create_info,
                null(),
                &mut render_pass,
            )
        };

        self.check(result, "vkCreateRenderPass")?;

        Ok(render_pass)
    }

    /// Translation of `VULKAN_CreateGraphicsPipeline()`.
    pub(super) fn create_graphics_pipeline_internal(
        &self,
        createinfo: &GraphicsPipelineCreateInfo<'_>,
        vertex_shader: Arc<VulkanShader>,
        fragment_shader: Arc<VulkanShader>,
    ) -> Result<(Arc<VulkanGraphicsPipeline>, GraphicsPipelineHeader)> {
        static DYNAMIC_STATES: [i32; 4] = [
            VK_DYNAMIC_STATE_VIEWPORT,
            VK_DYNAMIC_STATE_SCISSOR,
            VK_DYNAMIC_STATE_BLEND_CONSTANTS,
            VK_DYNAMIC_STATE_STENCIL_REFERENCE,
        ];

        // Create a "compatible" render pass

        // Note (upstream): C goes on with a null render pass when this
        // fails (pipeline creation then fails).
        let transient_render_pass = self
            .create_transient_render_pass(
                &createinfo.target_info,
                sdl_to_vk_sample_count(createinfo.multisample_state.sample_count),
            )
            .unwrap_or(VK_NULL_HANDLE);

        // Dynamic state

        let dynamic_state_create_info = VkPipelineDynamicStateCreateInfo {
            s_type: VK_STRUCTURE_TYPE_PIPELINE_DYNAMIC_STATE_CREATE_INFO,
            p_next: null(),
            flags: 0,
            dynamic_state_count: DYNAMIC_STATES.len() as u32,
            p_dynamic_states: DYNAMIC_STATES.as_ptr(),
        };

        // Shader stages

        vertex_shader.reference_count.fetch_add(1, Ordering::SeqCst);

        let shader_stage_create_infos = [
            VkPipelineShaderStageCreateInfo {
                s_type: VK_STRUCTURE_TYPE_PIPELINE_SHADER_STAGE_CREATE_INFO,
                p_next: null(),
                flags: 0,
                stage: VK_SHADER_STAGE_VERTEX_BIT,
                module: vertex_shader.shader_module,
                p_name: vertex_shader.entrypoint_name.as_ptr(),
                p_specialization_info: null(),
            },
            VkPipelineShaderStageCreateInfo {
                s_type: VK_STRUCTURE_TYPE_PIPELINE_SHADER_STAGE_CREATE_INFO,
                p_next: null(),
                flags: 0,
                stage: VK_SHADER_STAGE_FRAGMENT_BIT,
                module: fragment_shader.shader_module,
                p_name: fragment_shader.entrypoint_name.as_ptr(),
                p_specialization_info: null(),
            },
        ];

        fragment_shader
            .reference_count
            .fetch_add(1, Ordering::SeqCst);

        if self.debug_mode {
            if vertex_shader.stage != ShaderStage::Vertex {
                crate::sdl_assert_release!(
                    !"CreateGraphicsPipeline was passed a fragment shader for the vertex stage"
                );
            }
            if fragment_shader.stage != ShaderStage::Fragment {
                crate::sdl_assert_release!(
                    !"CreateGraphicsPipeline was passed a vertex shader for the fragment stage"
                );
            }
        }

        // Vertex input

        let vertex_input_state = &createinfo.vertex_input_state;
        let vertex_input_binding_descriptions: Vec<VkVertexInputBindingDescription> =
            vertex_input_state
                .vertex_buffer_descriptions
                .iter()
                .map(|d| VkVertexInputBindingDescription {
                    binding: d.slot,
                    input_rate: sdl_to_vk_vertex_input_rate(d.input_rate),
                    stride: d.pitch,
                })
                .collect();

        let vertex_input_attribute_descriptions: Vec<VkVertexInputAttributeDescription> =
            vertex_input_state
                .vertex_attributes
                .iter()
                .map(|a| VkVertexInputAttributeDescription {
                    binding: a.buffer_slot,
                    format: sdl_to_vk_vertex_format(a.format),
                    location: a.location,
                    offset: a.offset,
                })
                .collect();

        let vertex_input_state_create_info = VkPipelineVertexInputStateCreateInfo {
            s_type: VK_STRUCTURE_TYPE_PIPELINE_VERTEX_INPUT_STATE_CREATE_INFO,
            p_next: null(),
            flags: 0,
            vertex_binding_description_count: vertex_input_binding_descriptions.len() as u32,
            p_vertex_binding_descriptions: vertex_input_binding_descriptions.as_ptr(),
            vertex_attribute_description_count: vertex_input_attribute_descriptions.len() as u32,
            p_vertex_attribute_descriptions: vertex_input_attribute_descriptions.as_ptr(),
        };

        // Topology

        let input_assembly_state_create_info = VkPipelineInputAssemblyStateCreateInfo {
            s_type: VK_STRUCTURE_TYPE_PIPELINE_INPUT_ASSEMBLY_STATE_CREATE_INFO,
            p_next: null(),
            flags: 0,
            primitive_restart_enable: VK_FALSE,
            topology: sdl_to_vk_primitive_type(createinfo.primitive_type),
        };

        // Viewport

        // NOTE: viewport and scissor are dynamic, and must be set using the command buffer

        let viewport_state_create_info = VkPipelineViewportStateCreateInfo {
            s_type: VK_STRUCTURE_TYPE_PIPELINE_VIEWPORT_STATE_CREATE_INFO,
            p_next: null(),
            flags: 0,
            viewport_count: 1,
            p_viewports: null(),
            scissor_count: 1,
            p_scissors: null(),
        };

        // Rasterization

        let rasterizer_state = &createinfo.rasterizer_state;
        let rasterization_state_create_info = VkPipelineRasterizationStateCreateInfo {
            s_type: VK_STRUCTURE_TYPE_PIPELINE_RASTERIZATION_STATE_CREATE_INFO,
            p_next: null(),
            flags: 0,
            depth_clamp_enable: (!rasterizer_state.enable_depth_clip) as VkBool32,
            rasterizer_discard_enable: VK_FALSE,
            polygon_mode: self.sdl_to_vk_polygon_mode(rasterizer_state.fill_mode),
            cull_mode: sdl_to_vk_cull_mode(rasterizer_state.cull_mode),
            front_face: sdl_to_vk_front_face(rasterizer_state.front_face),
            depth_bias_enable: rasterizer_state.enable_depth_bias as VkBool32,
            depth_bias_constant_factor: rasterizer_state.depth_bias_constant_factor,
            depth_bias_clamp: rasterizer_state.depth_bias_clamp,
            depth_bias_slope_factor: rasterizer_state.depth_bias_slope_factor,
            line_width: 1.0,
        };

        // Multisample

        let sample_mask: u32 = 0xFFFFFFFF;

        let multisample_state_create_info = VkPipelineMultisampleStateCreateInfo {
            s_type: VK_STRUCTURE_TYPE_PIPELINE_MULTISAMPLE_STATE_CREATE_INFO,
            p_next: null(),
            flags: 0,
            rasterization_samples: sdl_to_vk_sample_count(
                createinfo.multisample_state.sample_count,
            ),
            sample_shading_enable: VK_FALSE,
            min_sample_shading: 1.0,
            p_sample_mask: &sample_mask,
            alpha_to_coverage_enable: createinfo.multisample_state.enable_alpha_to_coverage
                as VkBool32,
            alpha_to_one_enable: VK_FALSE,
        };

        // Depth Stencil State

        let depth_stencil_state = &createinfo.depth_stencil_state;
        let stencil_state = |state: &crate::gpu::StencilOpState| VkStencilOpState {
            fail_op: sdl_to_vk_stencil_op(state.fail_op),
            pass_op: sdl_to_vk_stencil_op(state.pass_op),
            depth_fail_op: sdl_to_vk_stencil_op(state.depth_fail_op),
            compare_op: sdl_to_vk_compare_op(state.compare_op),
            compare_mask: depth_stencil_state.compare_mask as u32,
            write_mask: depth_stencil_state.write_mask as u32,
            reference: 0,
        };
        let front_stencil_state = stencil_state(&depth_stencil_state.front_stencil_state);
        let back_stencil_state = stencil_state(&depth_stencil_state.back_stencil_state);

        let depth_stencil_state_create_info = VkPipelineDepthStencilStateCreateInfo {
            s_type: VK_STRUCTURE_TYPE_PIPELINE_DEPTH_STENCIL_STATE_CREATE_INFO,
            p_next: null(),
            flags: 0,
            depth_test_enable: depth_stencil_state.enable_depth_test as VkBool32,
            depth_write_enable: depth_stencil_state.enable_depth_write as VkBool32,
            depth_compare_op: sdl_to_vk_compare_op(depth_stencil_state.compare_op),
            depth_bounds_test_enable: VK_FALSE,
            stencil_test_enable: depth_stencil_state.enable_stencil_test as VkBool32,
            front: front_stencil_state,
            back: back_stencil_state,
            min_depth_bounds: 0.0, // unused
            max_depth_bounds: 0.0, // unused
        };

        // Color Blend

        let color_blend_attachment_states: Vec<VkPipelineColorBlendAttachmentState> = createinfo
            .target_info
            .color_target_descriptions
            .iter()
            .map(|description| {
                let blend_state = description.blend_state;
                let color_write_mask = if blend_state.enable_color_write_mask {
                    blend_state.color_write_mask
                } else {
                    ColorComponentFlags(0xF)
                };

                VkPipelineColorBlendAttachmentState {
                    blend_enable: blend_state.enable_blend as VkBool32,
                    src_color_blend_factor: sdl_to_vk_blend_factor(
                        blend_state.src_color_blendfactor,
                    ),
                    dst_color_blend_factor: sdl_to_vk_blend_factor(
                        blend_state.dst_color_blendfactor,
                    ),
                    color_blend_op: sdl_to_vk_blend_op(blend_state.color_blend_op),
                    src_alpha_blend_factor: sdl_to_vk_blend_factor(
                        blend_state.src_alpha_blendfactor,
                    ),
                    dst_alpha_blend_factor: sdl_to_vk_blend_factor(
                        blend_state.dst_alpha_blendfactor,
                    ),
                    alpha_blend_op: sdl_to_vk_blend_op(blend_state.alpha_blend_op),
                    color_write_mask: color_write_mask.bits() as VkFlags,
                }
            })
            .collect();

        let color_blend_state_create_info = VkPipelineColorBlendStateCreateInfo {
            s_type: VK_STRUCTURE_TYPE_PIPELINE_COLOR_BLEND_STATE_CREATE_INFO,
            p_next: null(),
            flags: 0,
            attachment_count: color_blend_attachment_states.len() as u32,
            p_attachments: color_blend_attachment_states.as_ptr(),
            blend_constants: [1.0, 1.0, 1.0, 1.0],

            // We don't support LogicOp, so this is easy.
            logic_op_enable: VK_FALSE,
            logic_op: 0,
        };

        // Pipeline Layout

        // FIXME (upstream): when this or the pipeline fails, the shaders'
        // reference counts taken above aren't given back (so they are
        // never destroyed), and here the transient render pass leaks.
        let resource_layout = match self
            .fetch_graphics_pipeline_resource_layout(&vertex_shader, &fragment_shader)
        {
            Ok(layout) => layout,
            Err(_) => {
                return Err(self.set_string_error("Failed to initialize pipeline resource layout!"))
            }
        };

        // Pipeline

        let vk_pipeline_create_info = VkGraphicsPipelineCreateInfo {
            s_type: VK_STRUCTURE_TYPE_GRAPHICS_PIPELINE_CREATE_INFO,
            p_next: null(),
            flags: 0,
            stage_count: 2,
            p_stages: shader_stage_create_infos.as_ptr(),
            p_vertex_input_state: &vertex_input_state_create_info,
            p_input_assembly_state: &input_assembly_state_create_info,
            p_tessellation_state: null(),
            p_viewport_state: &viewport_state_create_info,
            p_rasterization_state: &rasterization_state_create_info,
            p_multisample_state: &multisample_state_create_info,
            p_depth_stencil_state: &depth_stencil_state_create_info,
            p_color_blend_state: &color_blend_state_create_info,
            p_dynamic_state: &dynamic_state_create_info,
            layout: resource_layout.pipeline_layout,
            render_pass: transient_render_pass,
            subpass: 0,
            base_pipeline_handle: VK_NULL_HANDLE,
            base_pipeline_index: 0,
        };

        // TODO: enable pipeline caching
        let mut pipeline = VK_NULL_HANDLE;
        // SAFETY: a valid create info whose arrays and structures outlive
        // the call.
        let vulkan_result = unsafe {
            (self.dev.create_graphics_pipelines)(
                self.logical_device,
                VK_NULL_HANDLE,
                1,
                &vk_pipeline_create_info,
                null(),
                &mut pipeline,
            )
        };

        // SAFETY: the render pass made above, only used to create the
        // pipeline.
        unsafe {
            (self.dev.destroy_render_pass)(self.logical_device, transient_render_pass, null())
        };

        self.check(vulkan_result, "vkCreateGraphicsPipelines")?;

        if let Some(name) = string_property(
            &createinfo.props,
            crate::gpu::PROP_GPU_GRAPHICSPIPELINE_CREATE_NAME_STRING,
        ) {
            self.set_object_name(VK_OBJECT_TYPE_PIPELINE, pipeline, &name);
        }

        // Put this data in the pipeline we can do validation in gpu.c
        let header = GraphicsPipelineHeader {
            num_vertex_samplers: resource_layout.vertex_sampler_count,
            num_vertex_storage_buffers: resource_layout.vertex_storage_buffer_count,
            num_vertex_storage_textures: resource_layout.vertex_storage_texture_count,
            num_vertex_uniform_buffers: resource_layout.vertex_uniform_buffer_count,
            num_fragment_samplers: resource_layout.fragment_sampler_count,
            num_fragment_storage_buffers: resource_layout.fragment_storage_buffer_count,
            num_fragment_storage_textures: resource_layout.fragment_storage_texture_count,
            num_fragment_uniform_buffers: resource_layout.fragment_uniform_buffer_count,
        };

        let graphics_pipeline = Arc::new(VulkanGraphicsPipeline {
            pipeline,
            primitive_type: createinfo.primitive_type,
            resource_layout,
            vertex_shader,
            fragment_shader,
            reference_count: AtomicI32::new(0),
        });

        Ok((graphics_pipeline, header))
    }

    /// Translation of `VULKAN_CreateComputePipeline()`.
    pub(super) fn create_compute_pipeline_internal(
        &self,
        createinfo: &ComputePipelineCreateInfo<'_>,
    ) -> Result<(Arc<VulkanComputePipeline>, ComputePipelineHeader)> {
        if createinfo.format != ShaderFormat::SPIRV {
            return Err(self.set_string_error("Incompatible shader format for Vulkan!"));
        }

        if !is_valid_shader_bytecode(createinfo.code) {
            return Err(self.set_string_error("The provided shader code is not valid SPIR-V!"));
        }

        let code = code_words(createinfo.code);
        let shader_module_create_info = VkShaderModuleCreateInfo {
            s_type: VK_STRUCTURE_TYPE_SHADER_MODULE_CREATE_INFO,
            p_next: null(),
            flags: 0,
            code_size: createinfo.code.len(),
            p_code: code.as_ptr(),
        };

        let mut shader_module = VK_NULL_HANDLE;
        // SAFETY: a valid create info whose code outlives the call.
        let vulkan_result = unsafe {
            (self.dev.create_shader_module)(
                self.logical_device,
                &shader_module_create_info,
                null(),
                &mut shader_module,
            )
        };

        self.check(vulkan_result, "vkCreateShaderModule")?;

        // Note (upstream): C passes the entry point as given (NULL isn't
        // valid there); a name with a NUL byte can't be passed here.
        let entrypoint = match CString::new(createinfo.entrypoint) {
            Ok(entrypoint) => entrypoint,
            Err(_) => {
                // SAFETY: the module just created.
                unsafe {
                    (self.dev.destroy_shader_module)(self.logical_device, shader_module, null())
                };
                return Err(self.set_string_error("Invalid compute shader entry point!"));
            }
        };

        let pipeline_shader_stage_create_info = VkPipelineShaderStageCreateInfo {
            s_type: VK_STRUCTURE_TYPE_PIPELINE_SHADER_STAGE_CREATE_INFO,
            p_next: null(),
            flags: 0,
            stage: VK_SHADER_STAGE_COMPUTE_BIT,
            module: shader_module,
            p_name: entrypoint.as_ptr(),
            p_specialization_info: null(),
        };

        let resource_layout = match self.fetch_compute_pipeline_resource_layout(createinfo) {
            Ok(layout) => layout,
            Err(e) => {
                // SAFETY: the module just created.
                unsafe {
                    (self.dev.destroy_shader_module)(self.logical_device, shader_module, null())
                };
                return Err(e);
            }
        };

        let vk_shader_create_info = VkComputePipelineCreateInfo {
            s_type: VK_STRUCTURE_TYPE_COMPUTE_PIPELINE_CREATE_INFO,
            p_next: null(),
            flags: 0,
            stage: pipeline_shader_stage_create_info,
            layout: resource_layout.pipeline_layout,
            base_pipeline_handle: VK_NULL_HANDLE,
            base_pipeline_index: 0,
        };

        let mut pipeline = VK_NULL_HANDLE;
        // SAFETY: a valid create info whose name outlives the call.
        let vulkan_result = unsafe {
            (self.dev.create_compute_pipelines)(
                self.logical_device,
                VK_NULL_HANDLE,
                1,
                &vk_shader_create_info,
                null(),
                &mut pipeline,
            )
        };

        let compute_pipeline = VulkanComputePipeline {
            shader_module,
            pipeline,
            resource_layout,
            reference_count: AtomicI32::new(0),
        };

        if vulkan_result != VK_SUCCESS {
            self.destroy_compute_pipeline(&compute_pipeline);
            self.check(vulkan_result, "vkCreateComputePipeline")?;
        }

        if let Some(name) = string_property(
            &createinfo.props,
            crate::gpu::PROP_GPU_COMPUTEPIPELINE_CREATE_NAME_STRING,
        ) {
            self.set_object_name(VK_OBJECT_TYPE_PIPELINE, pipeline, &name);
        }

        // Track these here for debug layer
        let layout = &compute_pipeline.resource_layout;
        let header = ComputePipelineHeader {
            num_samplers: layout.num_samplers,
            num_readonly_storage_textures: layout.num_readonly_storage_textures,
            num_readonly_storage_buffers: layout.num_readonly_storage_buffers,
            num_readwrite_storage_textures: layout.num_read_write_storage_textures,
            num_readwrite_storage_buffers: layout.num_read_write_storage_buffers,
            num_uniform_buffers: layout.num_uniform_buffers,
        };

        Ok((Arc::new(compute_pipeline), header))
    }

    /// Translation of `VULKAN_CreateSampler()`.
    pub(super) fn create_sampler_internal(
        &self,
        createinfo: &SamplerCreateInfo,
    ) -> Result<Arc<VulkanSampler>> {
        let vk_sampler_create_info = VkSamplerCreateInfo {
            s_type: VK_STRUCTURE_TYPE_SAMPLER_CREATE_INFO,
            p_next: null(),
            flags: 0,
            mag_filter: sdl_to_vk_filter(createinfo.mag_filter),
            min_filter: sdl_to_vk_filter(createinfo.min_filter),
            mipmap_mode: sdl_to_vk_sampler_mipmap_mode(createinfo.mipmap_mode),
            address_mode_u: sdl_to_vk_sampler_address_mode(createinfo.address_mode_u),
            address_mode_v: sdl_to_vk_sampler_address_mode(createinfo.address_mode_v),
            address_mode_w: sdl_to_vk_sampler_address_mode(createinfo.address_mode_w),
            mip_lod_bias: createinfo.mip_lod_bias,
            anisotropy_enable: createinfo.enable_anisotropy as VkBool32,
            max_anisotropy: createinfo.max_anisotropy,
            compare_enable: createinfo.enable_compare as VkBool32,
            compare_op: sdl_to_vk_compare_op(createinfo.compare_op),
            min_lod: createinfo.min_lod,
            max_lod: createinfo.max_lod,
            border_color: VK_BORDER_COLOR_FLOAT_TRANSPARENT_BLACK, // arbitrary, unused
            unnormalized_coordinates: VK_FALSE,
        };

        let mut sampler = VK_NULL_HANDLE;
        // SAFETY: a valid create info.
        let vulkan_result = unsafe {
            (self.dev.create_sampler)(
                self.logical_device,
                &vk_sampler_create_info,
                null(),
                &mut sampler,
            )
        };

        self.check(vulkan_result, "vkCreateSampler")?;

        if let Some(name) = string_property(
            &createinfo.props,
            crate::gpu::PROP_GPU_SAMPLER_CREATE_NAME_STRING,
        ) {
            self.set_object_name(VK_OBJECT_TYPE_SAMPLER, sampler, &name);
        }

        Ok(Arc::new(VulkanSampler {
            sampler,
            reference_count: AtomicI32::new(0),
        }))
    }

    /// Translation of `VULKAN_CreateShader()`.
    pub(super) fn create_shader_internal(
        &self,
        createinfo: &ShaderCreateInfo<'_>,
    ) -> Result<Arc<VulkanShader>> {
        if !is_valid_shader_bytecode(createinfo.code) {
            return Err(self.set_string_error("The provided shader code is not valid SPIR-V!"));
        }

        // (an empty entry point is C's NULL one)
        let entrypoint = if createinfo.entrypoint.is_empty() {
            "main"
        } else {
            createinfo.entrypoint
        };
        let Ok(entrypoint_name) = CString::new(entrypoint) else {
            return Err(self.set_string_error("Invalid shader entry point!"));
        };

        let code = code_words(createinfo.code);
        let vk_shader_module_create_info = VkShaderModuleCreateInfo {
            s_type: VK_STRUCTURE_TYPE_SHADER_MODULE_CREATE_INFO,
            p_next: null(),
            flags: 0,
            code_size: createinfo.code.len(),
            p_code: code.as_ptr(),
        };

        let mut shader_module = VK_NULL_HANDLE;
        // SAFETY: a valid create info whose code outlives the call.
        let vulkan_result = unsafe {
            (self.dev.create_shader_module)(
                self.logical_device,
                &vk_shader_module_create_info,
                null(),
                &mut shader_module,
            )
        };

        self.check(vulkan_result, "vkCreateShaderModule")?;

        // Note (upstream): C names the module in debug mode without
        // checking for VK_EXT_debug_utils (a NULL call without it); it is
        // checked here.
        if let Some(name) = string_property(
            &createinfo.props,
            crate::gpu::PROP_GPU_SHADER_CREATE_NAME_STRING,
        ) {
            self.set_object_name(VK_OBJECT_TYPE_SHADER_MODULE, shader_module, &name);
        }

        Ok(Arc::new(VulkanShader {
            shader_module,
            entrypoint_name,
            stage: createinfo.stage,
            num_samplers: createinfo.num_samplers,
            num_storage_textures: createinfo.num_storage_textures,
            num_storage_buffers: createinfo.num_storage_buffers,
            num_uniform_buffers: createinfo.num_uniform_buffers,
            reference_count: AtomicI32::new(0),
        }))
    }

    /// Translation of `VULKAN_INTERNAL_DestroyGraphicsPipeline()`.
    pub(super) fn destroy_graphics_pipeline(&self, graphics_pipeline: &VulkanGraphicsPipeline) {
        // SAFETY: the pipeline, unused by the GPU.
        unsafe {
            (self.dev.destroy_pipeline)(self.logical_device, graphics_pipeline.pipeline, null())
        };

        graphics_pipeline
            .vertex_shader
            .reference_count
            .fetch_sub(1, Ordering::SeqCst);
        graphics_pipeline
            .fragment_shader
            .reference_count
            .fetch_sub(1, Ordering::SeqCst);
    }

    /// Translation of `VULKAN_INTERNAL_DestroyComputePipeline()`.
    pub(super) fn destroy_compute_pipeline(&self, compute_pipeline: &VulkanComputePipeline) {
        if compute_pipeline.pipeline != VK_NULL_HANDLE {
            // SAFETY: the pipeline, unused by the GPU.
            unsafe {
                (self.dev.destroy_pipeline)(self.logical_device, compute_pipeline.pipeline, null())
            };
        }

        if compute_pipeline.shader_module != VK_NULL_HANDLE {
            // SAFETY: the pipeline's module.
            unsafe {
                (self.dev.destroy_shader_module)(
                    self.logical_device,
                    compute_pipeline.shader_module,
                    null(),
                )
            };
        }
    }

    /// Translation of `VULKAN_INTERNAL_DestroyShader()`.
    pub(super) fn destroy_shader(&self, vulkan_shader: &VulkanShader) {
        // SAFETY: the module, which no pipeline is being created with.
        unsafe {
            (self.dev.destroy_shader_module)(
                self.logical_device,
                vulkan_shader.shader_module,
                null(),
            )
        };
    }
}
