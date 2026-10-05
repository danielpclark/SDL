// Rust translation of the render pass, compute pass and copy pass parts of
// src/gpu/vulkan/SDL_gpu_vulkan.c from Simple DirectMedia Layer.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! Render passes (with the render pass and framebuffer caches), the
//! dynamic state, the resource bindings and their descriptor sets, draws,
//! compute passes and dispatches, uploads, downloads, copies, mipmap
//! generation and blits (`vkCmdBlitImage`, as upstream's backend does,
//! rather than the front end's blit pipelines).

use std::ptr::null;
use std::sync::atomic::AtomicI32;
use std::sync::Arc;

use super::commands::MAX_UBO_SECTION_SIZE;
use super::pipelines::{
    DescriptorSetLayout, FramebufferHashTableKey, RenderPassColorTargetDescription,
    RenderPassDepthStencilTargetDescription, RenderPassHashTableKey, VulkanComputePipeline,
    VulkanFramebuffer, VulkanGraphicsPipeline,
};
use super::resources::{
    get_texture_subresource_index, BufferContainer, SubresourceRef, TextureContainer,
    UniformBuffer, VulkanCommandBuffer, VulkanSampler, VulkanTextureUsageMode,
    VULKAN_BUFFER_USAGE_MODE_COMPUTE_STORAGE_READ,
    VULKAN_BUFFER_USAGE_MODE_COMPUTE_STORAGE_READ_WRITE, VULKAN_BUFFER_USAGE_MODE_COPY_DESTINATION,
    VULKAN_BUFFER_USAGE_MODE_COPY_SOURCE,
};
use super::tables::{
    sdl_to_vk_filter, sdl_to_vk_index_type, sdl_to_vk_load_op, sdl_to_vk_sample_count,
    sdl_to_vk_store_op, sdl_to_vk_texture_format,
};
use super::{lock, object, VulkanRenderer};
use crate::error::Result;
use crate::gpu::sysgpu::{
    MAX_COLOR_TARGET_BINDINGS, MAX_COMPUTE_WRITE_BUFFERS, MAX_STORAGE_BUFFERS_PER_STAGE,
    MAX_STORAGE_TEXTURES_PER_STAGE,
};
use crate::gpu::{
    BlitInfo, BufferBinding, BufferLocation, BufferRegion, ColorTargetInfo, DepthStencilTargetInfo,
    IndexElementSize, IndexedIndirectDrawCommand, IndirectDrawCommand, LoadOp,
    StorageBufferReadWriteBinding, StorageTextureReadWriteBinding, StoreOp, Texture,
    TextureLocation, TextureRegion, TextureSamplerBinding, TextureTransferInfo, TextureType,
    TransferBufferLocation, Viewport,
};
use crate::video::vk::*;
use crate::video::{FColor, FlipMode, Rect};

/// Whether a store op resolves (`SDL_GPU_STOREOP_RESOLVE` or
/// `SDL_GPU_STOREOP_RESOLVE_AND_STORE`).
fn resolves(store_op: StoreOp) -> bool {
    matches!(store_op, StoreOp::Resolve | StoreOp::ResolveAndStore)
}

/// The texture container of a front-end texture.
fn container(texture: &Texture) -> Arc<TextureContainer> {
    object::<TextureContainer>(&texture.raw)
}

/// The buffer container of a front-end buffer.
fn buffer_container(buffer: &crate::gpu::Buffer) -> Arc<BufferContainer> {
    object::<BufferContainer>(&buffer.raw)
}

/// The buffer container of a front-end transfer buffer.
fn transfer_buffer_container(buffer: &crate::gpu::TransferBuffer) -> Arc<BufferContainer> {
    object::<BufferContainer>(&buffer.raw)
}

/// The active buffer of a container.
fn active_buffer(container: &BufferContainer) -> Arc<super::resources::VulkanBuffer> {
    lock(&container.state).active_buffer.clone()
}

/// The active texture of a container.
fn active_texture(container: &TextureContainer) -> Arc<super::resources::VulkanTexture> {
    lock(&container.state).active_texture.clone()
}

/// `width >> level`. Note (upstream): a level of 32 or more shifts by the
/// type's width or more, which is undefined in C; the shift count wraps
/// here.
fn mip_size(size: u32, level: u32) -> u32 {
    size.wrapping_shr(level)
}

/// The descriptor writes of a bind, collected before the
/// `VkWriteDescriptorSet`s point at their infos (upstream fills fixed
/// arrays as it goes).
#[derive(Default)]
struct DescriptorWrites {
    writes: Vec<(VkDescriptorSet, u32, i32, DescriptorInfo)>,
    image_infos: Vec<VkDescriptorImageInfo>,
    buffer_infos: Vec<VkDescriptorBufferInfo>,
}

/// Which info a write points at.
#[derive(Clone, Copy)]
enum DescriptorInfo {
    Image(usize),
    Buffer(usize),
}

impl DescriptorWrites {
    fn image(
        &mut self,
        dst_set: VkDescriptorSet,
        dst_binding: u32,
        descriptor_type: i32,
        info: VkDescriptorImageInfo,
    ) {
        self.writes.push((
            dst_set,
            dst_binding,
            descriptor_type,
            DescriptorInfo::Image(self.image_infos.len()),
        ));
        self.image_infos.push(info);
    }

    fn buffer(
        &mut self,
        dst_set: VkDescriptorSet,
        dst_binding: u32,
        descriptor_type: i32,
        info: VkDescriptorBufferInfo,
    ) {
        self.writes.push((
            dst_set,
            dst_binding,
            descriptor_type,
            DescriptorInfo::Buffer(self.buffer_infos.len()),
        ));
        self.buffer_infos.push(info);
    }

    /// `vkUpdateDescriptorSets()` with the writes.
    fn update(&self, renderer: &VulkanRenderer) {
        let write_descriptor_sets: Vec<VkWriteDescriptorSet> = self
            .writes
            .iter()
            .map(|&(dst_set, dst_binding, descriptor_type, info)| {
                let (p_image_info, p_buffer_info) = match info {
                    DescriptorInfo::Image(i) => (&self.image_infos[i] as *const _, null()),
                    DescriptorInfo::Buffer(i) => (null(), &self.buffer_infos[i] as *const _),
                };
                VkWriteDescriptorSet {
                    s_type: VK_STRUCTURE_TYPE_WRITE_DESCRIPTOR_SET,
                    p_next: null(),
                    descriptor_count: 1,
                    descriptor_type,
                    dst_array_element: 0,
                    dst_binding,
                    dst_set,
                    p_texel_buffer_view: null(),
                    p_image_info,
                    p_buffer_info,
                }
            })
            .collect();

        // SAFETY: writes of the command buffer's own descriptor sets, with
        // infos that live until the call returns.
        unsafe {
            (renderer.dev.update_descriptor_sets)(
                renderer.logical_device,
                write_descriptor_sets.len() as u32,
                write_descriptor_sets.as_ptr(),
                0,
                null(),
            )
        };
    }
}

/// A combined image sampler's info.
fn sampler_info(sampler: VkSampler, image_view: VkImageView) -> VkDescriptorImageInfo {
    VkDescriptorImageInfo {
        sampler,
        image_view,
        image_layout: VK_IMAGE_LAYOUT_SHADER_READ_ONLY_OPTIMAL,
    }
}

/// A storage image's info (in the general layout).
fn storage_image_info(image_view: VkImageView) -> VkDescriptorImageInfo {
    VkDescriptorImageInfo {
        sampler: VK_NULL_HANDLE,
        image_view,
        image_layout: VK_IMAGE_LAYOUT_GENERAL,
    }
}

/// A whole storage buffer's info.
fn storage_buffer_info(buffer: VkBuffer) -> VkDescriptorBufferInfo {
    VkDescriptorBufferInfo {
        buffer,
        offset: 0,
        range: VK_WHOLE_SIZE,
    }
}

/// A uniform buffer section's info.
fn uniform_buffer_info(uniform_buffer: &Option<Arc<UniformBuffer>>) -> VkDescriptorBufferInfo {
    VkDescriptorBufferInfo {
        buffer: uniform_buffer
            .as_ref()
            .map_or(VK_NULL_HANDLE, |u| lock(&u.state).buffer.buffer),
        offset: 0,
        range: MAX_UBO_SECTION_SIZE as VkDeviceSize,
    }
}

/// A uniform buffer's draw offset.
fn draw_offset(uniform_buffer: &Option<Arc<UniformBuffer>>) -> u32 {
    uniform_buffer
        .as_ref()
        .map_or(0, |u| lock(&u.state).draw_offset)
}

impl VulkanRenderer {
    /// The next descriptor set of a layout from the command buffer's cache
    /// (`VULKAN_INTERNAL_FetchDescriptorSet(renderer, commandBuffer, ...)`).
    fn command_buffer_descriptor_set(
        &self,
        command_buffer: &mut VulkanCommandBuffer,
        descriptor_set_layout: &DescriptorSetLayout,
    ) -> VkDescriptorSet {
        match command_buffer.descriptor_set_cache.as_mut() {
            Some(cache) => self.fetch_descriptor_set(cache, descriptor_set_layout),
            None => VK_NULL_HANDLE,
        }
    }

    /// Write and bind the descriptor sets of the bound graphics pipeline
    /// that changed, and the vertex buffers. Translation of
    /// `VULKAN_INTERNAL_BindGraphicsDescriptorSets()`.
    ///
    /// Note (upstream): C's array of buffer infos has room for the storage
    /// buffers only, and overflows when uniform buffers are written too.
    fn bind_graphics_descriptor_sets(&self, command_buffer: &mut VulkanCommandBuffer) {
        let cb = command_buffer;
        if !cb.need_vertex_buffer_bind
            && !cb.need_new_vertex_resource_descriptor_set
            && !cb.need_new_vertex_uniform_descriptor_set
            && !cb.need_new_vertex_uniform_offsets
            && !cb.need_new_fragment_resource_descriptor_set
            && !cb.need_new_fragment_uniform_descriptor_set
            && !cb.need_new_fragment_uniform_offsets
        {
            return;
        }

        if cb.need_vertex_buffer_bind && cb.vertex_buffer_count > 0 {
            // SAFETY: a command buffer in a render pass and live buffers.
            unsafe {
                (self.dev.cmd_bind_vertex_buffers)(
                    cb.command_buffer,
                    0,
                    cb.vertex_buffer_count,
                    cb.vertex_buffers.as_ptr(),
                    cb.vertex_buffer_offsets.as_ptr(),
                )
            };

            cb.need_vertex_buffer_bind = false;
        }

        // Note (upstream): C dereferences a NULL pipeline when none is bound.
        let Some(pipeline) = cb.current_graphics_pipeline.clone() else {
            return;
        };
        let resource_layout = &pipeline.resource_layout;
        let mut writes = DescriptorWrites::default();
        let mut dynamic_offsets = Vec::with_capacity(8);

        if cb.need_new_vertex_resource_descriptor_set {
            let descriptor_set_layout = &resource_layout.descriptor_set_layouts[0];

            cb.vertex_resource_descriptor_set =
                self.command_buffer_descriptor_set(cb, descriptor_set_layout);
            let set = cb.vertex_resource_descriptor_set;

            for i in 0..resource_layout.vertex_sampler_count {
                writes.image(
                    set,
                    i,
                    VK_DESCRIPTOR_TYPE_COMBINED_IMAGE_SAMPLER,
                    sampler_info(
                        cb.vertex_sampler_bindings[i as usize],
                        cb.vertex_sampler_texture_view_bindings[i as usize],
                    ),
                );
            }

            for i in 0..resource_layout.vertex_storage_texture_count {
                writes.image(
                    set,
                    resource_layout.vertex_sampler_count + i,
                    VK_DESCRIPTOR_TYPE_SAMPLED_IMAGE, // Yes, we are declaring a storage image as a sampled image, because shaders are stupid.
                    storage_image_info(cb.vertex_storage_texture_view_bindings[i as usize]),
                );
            }

            for i in 0..resource_layout.vertex_storage_buffer_count {
                writes.buffer(
                    set,
                    resource_layout.vertex_sampler_count
                        + resource_layout.vertex_storage_texture_count
                        + i,
                    VK_DESCRIPTOR_TYPE_STORAGE_BUFFER,
                    storage_buffer_info(cb.vertex_storage_buffer_bindings[i as usize]),
                );
            }

            cb.need_new_vertex_resource_descriptor_set = false;
        }

        if cb.need_new_vertex_uniform_descriptor_set {
            let descriptor_set_layout = &resource_layout.descriptor_set_layouts[1];

            cb.vertex_uniform_descriptor_set =
                self.command_buffer_descriptor_set(cb, descriptor_set_layout);

            for i in 0..resource_layout.vertex_uniform_buffer_count {
                writes.buffer(
                    cb.vertex_uniform_descriptor_set,
                    i,
                    VK_DESCRIPTOR_TYPE_UNIFORM_BUFFER_DYNAMIC,
                    uniform_buffer_info(&cb.vertex_uniform_buffers[i as usize]),
                );
            }

            cb.need_new_vertex_uniform_descriptor_set = false;
        }

        for i in 0..resource_layout.vertex_uniform_buffer_count {
            dynamic_offsets.push(draw_offset(&cb.vertex_uniform_buffers[i as usize]));
        }

        if cb.need_new_fragment_resource_descriptor_set {
            let descriptor_set_layout = &resource_layout.descriptor_set_layouts[2];

            cb.fragment_resource_descriptor_set =
                self.command_buffer_descriptor_set(cb, descriptor_set_layout);
            let set = cb.fragment_resource_descriptor_set;

            for i in 0..resource_layout.fragment_sampler_count {
                writes.image(
                    set,
                    i,
                    VK_DESCRIPTOR_TYPE_COMBINED_IMAGE_SAMPLER,
                    sampler_info(
                        cb.fragment_sampler_bindings[i as usize],
                        cb.fragment_sampler_texture_view_bindings[i as usize],
                    ),
                );
            }

            for i in 0..resource_layout.fragment_storage_texture_count {
                writes.image(
                    set,
                    resource_layout.fragment_sampler_count + i,
                    VK_DESCRIPTOR_TYPE_SAMPLED_IMAGE, // Yes, we are declaring a storage image as a sampled image, because shaders are stupid.
                    storage_image_info(cb.fragment_storage_texture_view_bindings[i as usize]),
                );
            }

            for i in 0..resource_layout.fragment_storage_buffer_count {
                writes.buffer(
                    set,
                    resource_layout.fragment_sampler_count
                        + resource_layout.fragment_storage_texture_count
                        + i,
                    VK_DESCRIPTOR_TYPE_STORAGE_BUFFER,
                    storage_buffer_info(cb.fragment_storage_buffer_bindings[i as usize]),
                );
            }

            cb.need_new_fragment_resource_descriptor_set = false;
        }

        if cb.need_new_fragment_uniform_descriptor_set {
            let descriptor_set_layout = &resource_layout.descriptor_set_layouts[3];

            cb.fragment_uniform_descriptor_set =
                self.command_buffer_descriptor_set(cb, descriptor_set_layout);

            for i in 0..resource_layout.fragment_uniform_buffer_count {
                writes.buffer(
                    cb.fragment_uniform_descriptor_set,
                    i,
                    VK_DESCRIPTOR_TYPE_UNIFORM_BUFFER_DYNAMIC,
                    uniform_buffer_info(&cb.fragment_uniform_buffers[i as usize]),
                );
            }

            cb.need_new_fragment_uniform_descriptor_set = false;
        }

        for i in 0..resource_layout.fragment_uniform_buffer_count {
            dynamic_offsets.push(draw_offset(&cb.fragment_uniform_buffers[i as usize]));
        }

        writes.update(self);

        let sets = [
            cb.vertex_resource_descriptor_set,
            cb.vertex_uniform_descriptor_set,
            cb.fragment_resource_descriptor_set,
            cb.fragment_uniform_descriptor_set,
        ];

        // SAFETY: a command buffer in a render pass, the pipeline's layout
        // and sets of its set layouts.
        unsafe {
            (self.dev.cmd_bind_descriptor_sets)(
                cb.command_buffer,
                VK_PIPELINE_BIND_POINT_GRAPHICS,
                resource_layout.pipeline_layout,
                0,
                4,
                sets.as_ptr(),
                dynamic_offsets.len() as u32,
                dynamic_offsets.as_ptr(),
            )
        };

        cb.need_new_vertex_uniform_offsets = false;
        cb.need_new_fragment_uniform_offsets = false;
    }

    /// Translation of `VULKAN_DrawIndexedPrimitives()`.
    pub(super) fn draw_indexed_primitives_internal(
        &self,
        command_buffer: &mut VulkanCommandBuffer,
        num_indices: u32,
        num_instances: u32,
        first_index: u32,
        vertex_offset: i32,
        first_instance: u32,
    ) {
        self.bind_graphics_descriptor_sets(command_buffer);

        // SAFETY: a command buffer in a render pass with a pipeline bound.
        unsafe {
            (self.dev.cmd_draw_indexed)(
                command_buffer.command_buffer,
                num_indices,
                num_instances,
                first_index,
                vertex_offset,
                first_instance,
            )
        };
    }

    /// Translation of `VULKAN_DrawPrimitives()`.
    pub(super) fn draw_primitives_internal(
        &self,
        command_buffer: &mut VulkanCommandBuffer,
        num_vertices: u32,
        num_instances: u32,
        first_vertex: u32,
        first_instance: u32,
    ) {
        self.bind_graphics_descriptor_sets(command_buffer);

        // SAFETY: as above.
        unsafe {
            (self.dev.cmd_draw)(
                command_buffer.command_buffer,
                num_vertices,
                num_instances,
                first_vertex,
                first_instance,
            )
        };
    }

    /// Translation of `VULKAN_DrawPrimitivesIndirect()` (and, with
    /// `indexed`, `VULKAN_DrawIndexedPrimitivesIndirect()`).
    pub(super) fn draw_primitives_indirect_internal(
        &self,
        command_buffer: &mut VulkanCommandBuffer,
        buffer: &BufferContainer,
        offset: u32,
        draw_count: u32,
        indexed: bool,
    ) {
        let vulkan_buffer = active_buffer(buffer);
        let pitch = if indexed {
            std::mem::size_of::<IndexedIndirectDrawCommand>() as u32
        } else {
            std::mem::size_of::<IndirectDrawCommand>() as u32
        };
        let draw = if indexed {
            self.dev.cmd_draw_indexed_indirect
        } else {
            self.dev.cmd_draw_indirect
        };

        self.bind_graphics_descriptor_sets(command_buffer);

        if self.supports_multi_draw_indirect {
            // Real multi-draw!
            // SAFETY: a command buffer in a render pass and an indirect
            // buffer.
            unsafe {
                draw(
                    command_buffer.command_buffer,
                    vulkan_buffer.buffer,
                    offset as VkDeviceSize,
                    draw_count,
                    pitch,
                )
            };
        } else {
            // Fake multi-draw...
            for i in 0..draw_count {
                // SAFETY: as above.
                unsafe {
                    draw(
                        command_buffer.command_buffer,
                        vulkan_buffer.buffer,
                        offset.wrapping_add(pitch.wrapping_mul(i)) as VkDeviceSize,
                        1,
                        pitch,
                    )
                };
            }
        }

        command_buffer.track_buffer(&vulkan_buffer);
    }

    // Render Pass

    /// Translation of `VULKAN_INTERNAL_CreateRenderPass()`.
    fn create_render_pass(
        &self,
        color_target_infos: &[ColorTargetInfo<'_>],
        depth_stencil_target_info: Option<&DepthStencilTargetInfo<'_>>,
    ) -> Result<VkRenderPass> {
        const N: usize = MAX_COLOR_TARGET_BINDINGS as usize;
        let mut attachment_descriptions =
            [VkAttachmentDescription::default(); 2 * N + 1 /* depth */];
        let mut color_attachment_references = [VkAttachmentReference::default(); N];
        let mut resolve_references = [VkAttachmentReference::default(); N];
        let depth_stencil_attachment_reference: VkAttachmentReference;

        let mut attachment_description_count = 0usize;
        let mut resolve_reference_count = 0usize;

        for (color_attachment_reference_count, info) in color_target_infos.iter().enumerate() {
            let header = &info.texture.info;
            attachment_descriptions[attachment_description_count] = VkAttachmentDescription {
                flags: 0,
                format: sdl_to_vk_texture_format(header.format),
                samples: sdl_to_vk_sample_count(header.sample_count),
                load_op: sdl_to_vk_load_op(info.load_op),
                store_op: sdl_to_vk_store_op(info.store_op),
                stencil_load_op: VK_ATTACHMENT_LOAD_OP_DONT_CARE,
                stencil_store_op: VK_ATTACHMENT_STORE_OP_DONT_CARE,
                initial_layout: VK_IMAGE_LAYOUT_COLOR_ATTACHMENT_OPTIMAL,
                final_layout: VK_IMAGE_LAYOUT_COLOR_ATTACHMENT_OPTIMAL,
            };

            color_attachment_references[color_attachment_reference_count] = VkAttachmentReference {
                attachment: attachment_description_count as u32,
                layout: VK_IMAGE_LAYOUT_COLOR_ATTACHMENT_OPTIMAL,
            };

            resolve_references[color_attachment_reference_count].layout =
                VK_IMAGE_LAYOUT_COLOR_ATTACHMENT_OPTIMAL;

            attachment_description_count += 1;

            match info.resolve_texture.filter(|_| resolves(info.store_op)) {
                Some(resolve_texture) => {
                    let resolve_header = &resolve_texture.info;

                    attachment_descriptions[attachment_description_count] =
                        VkAttachmentDescription {
                            flags: 0,
                            format: sdl_to_vk_texture_format(resolve_header.format),
                            samples: sdl_to_vk_sample_count(resolve_header.sample_count),
                            load_op: VK_ATTACHMENT_LOAD_OP_DONT_CARE, // The texture will be overwritten anyway
                            store_op: VK_ATTACHMENT_STORE_OP_STORE, // Always store the resolve texture
                            stencil_load_op: VK_ATTACHMENT_LOAD_OP_DONT_CARE,
                            stencil_store_op: VK_ATTACHMENT_STORE_OP_DONT_CARE,
                            initial_layout: VK_IMAGE_LAYOUT_COLOR_ATTACHMENT_OPTIMAL,
                            final_layout: VK_IMAGE_LAYOUT_COLOR_ATTACHMENT_OPTIMAL,
                        };

                    resolve_references[color_attachment_reference_count].attachment =
                        attachment_description_count as u32;

                    attachment_description_count += 1;
                    resolve_reference_count += 1;
                }
                None => {
                    resolve_references[color_attachment_reference_count].attachment =
                        VK_ATTACHMENT_UNUSED;
                }
            }
        }

        let mut subpass = VkSubpassDescription {
            pipeline_bind_point: VK_PIPELINE_BIND_POINT_GRAPHICS,
            flags: 0,
            input_attachment_count: 0,
            p_input_attachments: null(),
            color_attachment_count: color_target_infos.len() as u32,
            p_color_attachments: color_attachment_references.as_ptr(),
            preserve_attachment_count: 0,
            p_preserve_attachments: null(),
            p_depth_stencil_attachment: null(),
            p_resolve_attachments: null(),
        };

        if let Some(info) = depth_stencil_target_info {
            let header = &info.texture.info;

            attachment_descriptions[attachment_description_count] = VkAttachmentDescription {
                flags: 0,
                format: sdl_to_vk_texture_format(header.format),
                samples: sdl_to_vk_sample_count(header.sample_count),
                load_op: sdl_to_vk_load_op(info.load_op),
                store_op: sdl_to_vk_store_op(info.store_op),
                stencil_load_op: sdl_to_vk_load_op(info.stencil_load_op),
                stencil_store_op: sdl_to_vk_store_op(info.stencil_store_op),
                initial_layout: VK_IMAGE_LAYOUT_DEPTH_STENCIL_ATTACHMENT_OPTIMAL,
                final_layout: VK_IMAGE_LAYOUT_DEPTH_STENCIL_ATTACHMENT_OPTIMAL,
            };

            depth_stencil_attachment_reference = VkAttachmentReference {
                attachment: attachment_description_count as u32,
                layout: VK_IMAGE_LAYOUT_DEPTH_STENCIL_ATTACHMENT_OPTIMAL,
            };

            subpass.p_depth_stencil_attachment = &depth_stencil_attachment_reference;

            attachment_description_count += 1;
        }

        if resolve_reference_count > 0 {
            subpass.p_resolve_attachments = resolve_references.as_ptr();
        }

        let render_pass_create_info = VkRenderPassCreateInfo {
            s_type: VK_STRUCTURE_TYPE_RENDER_PASS_CREATE_INFO,
            p_next: null(),
            flags: 0,
            p_attachments: attachment_descriptions.as_ptr(),
            attachment_count: attachment_description_count as u32,
            subpass_count: 1,
            p_subpasses: &subpass,
            dependency_count: 0,
            p_dependencies: null(),
        };

        let mut render_pass = VK_NULL_HANDLE;
        // SAFETY: a valid create info, whose arrays live until the call
        // returns.
        let vulkan_result = unsafe {
            (self.dev.create_render_pass)(
                self.logical_device,
                &render_pass_create_info,
                null(),
                &mut render_pass,
            )
        };

        self.check(vulkan_result, "vkCreateRenderPass")?;

        Ok(render_pass)
    }

    /// The render pass for some targets, from the cache or new. Translation
    /// of `VULKAN_INTERNAL_FetchRenderPass()`.
    fn fetch_render_pass(
        &self,
        color_target_infos: &[ColorTargetInfo<'_>],
        depth_stencil_target_info: Option<&DepthStencilTargetInfo<'_>>,
    ) -> Result<VkRenderPass> {
        let mut key = RenderPassHashTableKey::default();

        for (i, info) in color_target_infos.iter().enumerate() {
            key.color_target_descriptions[i] = RenderPassColorTargetDescription {
                format: sdl_to_vk_texture_format(info.texture.info.format),
                load_op: info.load_op,
                store_op: info.store_op,
            };

            if let Some(resolve_texture) = info.resolve_texture {
                key.resolve_target_formats[key.num_resolve_targets as usize] =
                    sdl_to_vk_texture_format(resolve_texture.info.format);
                key.num_resolve_targets += 1;
            }
        }

        key.sample_count = VK_SAMPLE_COUNT_1_BIT;
        if let Some(first) = color_target_infos.first() {
            key.sample_count = sdl_to_vk_sample_count(first.texture.info.sample_count);
        } else if let Some(depth) = depth_stencil_target_info {
            key.sample_count = sdl_to_vk_sample_count(depth.texture.info.sample_count);
        }

        key.num_color_targets = color_target_infos.len() as u32;

        key.depth_stencil_target_description = match depth_stencil_target_info {
            None => RenderPassDepthStencilTargetDescription {
                format: 0,
                load_op: LoadOp::DontCare,
                store_op: StoreOp::DontCare,
                stencil_load_op: LoadOp::DontCare,
                stencil_store_op: StoreOp::DontCare,
            },
            Some(info) => RenderPassDepthStencilTargetDescription {
                format: sdl_to_vk_texture_format(info.texture.info.format),
                load_op: info.load_op,
                store_op: info.store_op,
                stencil_load_op: info.stencil_load_op,
                stencil_store_op: info.stencil_store_op,
            },
        };

        let mut table = lock(&self.render_pass_hash_table);

        if let Some(&render_pass) = table.get(&key) {
            return Ok(render_pass);
        }

        let render_pass_handle =
            self.create_render_pass(color_target_infos, depth_stencil_target_info)?;

        table.insert(key, render_pass_handle);

        Ok(render_pass_handle)
    }

    /// The framebuffer for a render pass's targets, from the cache or new.
    /// Translation of `VULKAN_INTERNAL_FetchFramebuffer()`.
    ///
    /// FIXME (upstream): the key has the resolve texture's subresource at
    /// the color target's layer and level (and whenever a resolve texture
    /// is set), the framebuffer at the resolve layer and level (when the
    /// store op resolves).
    fn fetch_framebuffer(
        &self,
        render_pass: VkRenderPass,
        color_target_infos: &[ColorTargetInfo<'_>],
        depth_stencil_target_info: Option<&DepthStencilTargetInfo<'_>>,
        width: u32,
        height: u32,
    ) -> Result<Arc<VulkanFramebuffer>> {
        const N: usize = MAX_COLOR_TARGET_BINDINGS as usize;
        let mut image_view_attachments = [VK_NULL_HANDLE; 2 * N + 1 /* depth */];
        let mut key = FramebufferHashTableKey {
            num_color_targets: color_target_infos.len() as u32,
            ..Default::default()
        };
        let mut attachment_count = 0usize;

        /// The render target view of a color target's subresource.
        fn render_target_view(
            texture: &Texture,
            layer_or_depth_plane: u32,
            mip_level: u32,
        ) -> VkImageView {
            let container = container(texture);
            let is_3d = container.info.texture_type == TextureType::Texture3D;
            let subresource = VulkanRenderer::fetch_texture_subresource(
                &container,
                if is_3d { 0 } else { layer_or_depth_plane },
                mip_level,
            );

            let rtv_index = if is_3d { layer_or_depth_plane } else { 0 };
            subresource
                .get()
                .render_target_views
                .get(rtv_index as usize)
                .copied()
                .unwrap_or(VK_NULL_HANDLE)
        }

        /// The first render target view of a texture's subresource.
        fn resolve_view(texture: &Texture, layer: u32, level: u32) -> VkImageView {
            let subresource =
                VulkanRenderer::fetch_texture_subresource(&container(texture), layer, level);
            subresource
                .get()
                .render_target_views
                .first()
                .copied()
                .unwrap_or(VK_NULL_HANDLE)
        }

        /// The depth-stencil view of a depth-stencil target.
        fn depth_stencil_view(info: &DepthStencilTargetInfo<'_>) -> VkImageView {
            let subresource = VulkanRenderer::fetch_texture_subresource(
                &container(info.texture),
                info.layer as u32,
                info.mip_level as u32,
            );
            subresource.get().depth_stencil_view
        }

        for (i, info) in color_target_infos.iter().enumerate() {
            key.color_attachment_views[i] =
                render_target_view(info.texture, info.layer_or_depth_plane, info.mip_level);

            if let Some(resolve_texture) = info.resolve_texture {
                key.resolve_attachment_views[key.num_resolve_attachments as usize] =
                    resolve_view(resolve_texture, info.layer_or_depth_plane, info.mip_level);
                key.num_resolve_attachments += 1;
            }
        }

        key.depth_stencil_attachment_view = match depth_stencil_target_info {
            None => VK_NULL_HANDLE,
            Some(info) => depth_stencil_view(info),
        };

        key.width = width;
        key.height = height;

        let mut table = lock(&self.framebuffer_hash_table);

        if let Some(framebuffer) = table.get(&key) {
            return Ok(framebuffer.clone());
        }

        // Create a new framebuffer

        for info in color_target_infos {
            image_view_attachments[attachment_count] =
                render_target_view(info.texture, info.layer_or_depth_plane, info.mip_level);

            attachment_count += 1;

            if let Some(resolve_texture) = info.resolve_texture.filter(|_| resolves(info.store_op))
            {
                image_view_attachments[attachment_count] =
                    resolve_view(resolve_texture, info.resolve_layer, info.resolve_mip_level);

                attachment_count += 1;
            }
        }

        if let Some(info) = depth_stencil_target_info {
            image_view_attachments[attachment_count] = depth_stencil_view(info);

            attachment_count += 1;
        }

        let framebuffer_info = VkFramebufferCreateInfo {
            s_type: VK_STRUCTURE_TYPE_FRAMEBUFFER_CREATE_INFO,
            p_next: null(),
            flags: 0,
            render_pass,
            attachment_count: attachment_count as u32,
            p_attachments: image_view_attachments.as_ptr(),
            width: key.width,
            height: key.height,
            layers: 1,
        };

        let mut framebuffer = VK_NULL_HANDLE;
        // SAFETY: a valid create info of live views.
        let result = unsafe {
            (self.dev.create_framebuffer)(
                self.logical_device,
                &framebuffer_info,
                null(),
                &mut framebuffer,
            )
        };

        if result != VK_SUCCESS {
            drop(table);
            return Err(self.vk_error(result, "vkCreateFramebuffer"));
        }

        let vulkan_framebuffer = Arc::new(VulkanFramebuffer {
            framebuffer,
            reference_count: AtomicI32::new(0),
        });

        table.insert(key, vulkan_framebuffer.clone());

        Ok(vulkan_framebuffer)
    }

    /// Translation of `VULKAN_INTERNAL_SetCurrentViewport()`.
    pub(super) fn set_current_viewport(
        &self,
        command_buffer: &mut VulkanCommandBuffer,
        viewport: &Viewport,
    ) {
        command_buffer.current_viewport = VkViewport {
            x: viewport.x,
            width: viewport.w,
            min_depth: viewport.min_depth,
            max_depth: viewport.max_depth,

            // Viewport flip for consistency with other backends
            y: viewport.y + viewport.h,
            height: -viewport.h,
        };

        // SAFETY: a command buffer being recorded.
        unsafe {
            (self.dev.cmd_set_viewport)(
                command_buffer.command_buffer,
                0,
                1,
                &command_buffer.current_viewport,
            )
        };
    }

    /// Translation of `VULKAN_INTERNAL_SetCurrentScissor()`.
    pub(super) fn set_current_scissor(
        &self,
        command_buffer: &mut VulkanCommandBuffer,
        scissor: &Rect,
    ) {
        command_buffer.current_scissor = VkRect2D {
            offset: VkOffset2D {
                x: scissor.x,
                y: scissor.y,
            },
            extent: VkExtent2D {
                width: scissor.w as u32,
                height: scissor.h as u32,
            },
        };

        // SAFETY: a command buffer being recorded.
        unsafe {
            (self.dev.cmd_set_scissor)(
                command_buffer.command_buffer,
                0,
                1,
                &command_buffer.current_scissor,
            )
        };
    }

    /// Translation of `VULKAN_INTERNAL_SetCurrentBlendConstants()`.
    pub(super) fn set_current_blend_constants(
        &self,
        command_buffer: &mut VulkanCommandBuffer,
        blend_constants: FColor,
    ) {
        command_buffer.blend_constants = [
            blend_constants.r,
            blend_constants.g,
            blend_constants.b,
            blend_constants.a,
        ];

        // SAFETY: a command buffer being recorded.
        unsafe {
            (self.dev.cmd_set_blend_constants)(
                command_buffer.command_buffer,
                &command_buffer.blend_constants,
            )
        };
    }

    /// Translation of `VULKAN_INTERNAL_SetCurrentStencilReference()`.
    pub(super) fn set_current_stencil_reference(
        &self,
        command_buffer: &mut VulkanCommandBuffer,
        reference: u8,
    ) {
        command_buffer.stencil_ref = reference;

        // SAFETY: a command buffer being recorded.
        unsafe {
            (self.dev.cmd_set_stencil_reference)(
                command_buffer.command_buffer,
                VK_STENCIL_FACE_FRONT_AND_BACK,
                command_buffer.stencil_ref as u32,
            )
        };
    }

    /// Bind combined texture samplers of a graphics stage (the loop of
    /// `VULKAN_BindVertexSamplers()` and `VULKAN_BindFragmentSamplers()`).
    pub(super) fn bind_graphics_samplers(
        command_buffer: &mut VulkanCommandBuffer,
        fragment: bool,
        first_slot: u32,
        texture_sampler_bindings: &[TextureSamplerBinding<'_>],
    ) {
        for (i, binding) in texture_sampler_bindings.iter().enumerate() {
            let slot = first_slot as usize + i;
            let texture_container = container(binding.texture);
            let sampler = object::<VulkanSampler>(&binding.sampler.raw);
            let active = active_texture(&texture_container);
            let cb = &mut *command_buffer;

            let (sampler_bindings, view_bindings) = if fragment {
                (
                    &mut cb.fragment_sampler_bindings,
                    &mut cb.fragment_sampler_texture_view_bindings,
                )
            } else {
                (
                    &mut cb.vertex_sampler_bindings,
                    &mut cb.vertex_sampler_texture_view_bindings,
                )
            };

            let mut changed = false;
            let new_sampler = sampler_bindings[slot] != sampler.sampler;
            if new_sampler {
                sampler_bindings[slot] = sampler.sampler;
                changed = true;
            }
            let new_view = view_bindings[slot] != active.full_view;
            if new_view {
                view_bindings[slot] = active.full_view;
                changed = true;
            }

            if new_sampler {
                cb.track_sampler(&sampler);
            }
            if new_view {
                cb.track_texture(&active);
            }
            if changed {
                if fragment {
                    cb.need_new_fragment_resource_descriptor_set = true;
                } else {
                    cb.need_new_vertex_resource_descriptor_set = true;
                }
            }
        }
    }

    /// Bind storage textures of a graphics stage (the loop of
    /// `VULKAN_BindVertexStorageTextures()` and
    /// `VULKAN_BindFragmentStorageTextures()`).
    pub(super) fn bind_graphics_storage_textures(
        command_buffer: &mut VulkanCommandBuffer,
        fragment: bool,
        first_slot: u32,
        storage_textures: &[&Texture],
    ) {
        for (i, texture) in storage_textures.iter().enumerate() {
            let slot = first_slot as usize + i;
            let active = active_texture(&container(texture));
            let cb = &mut *command_buffer;
            let view_bindings = if fragment {
                &mut cb.fragment_storage_texture_view_bindings
            } else {
                &mut cb.vertex_storage_texture_view_bindings
            };

            if view_bindings[slot] != active.full_view {
                view_bindings[slot] = active.full_view;
                cb.track_texture(&active);

                if fragment {
                    cb.need_new_fragment_resource_descriptor_set = true;
                } else {
                    cb.need_new_vertex_resource_descriptor_set = true;
                }
            }
        }
    }

    /// Bind storage buffers of a graphics stage (the loop of
    /// `VULKAN_BindVertexStorageBuffers()` and
    /// `VULKAN_BindFragmentStorageBuffers()`).
    pub(super) fn bind_graphics_storage_buffers(
        command_buffer: &mut VulkanCommandBuffer,
        fragment: bool,
        first_slot: u32,
        storage_buffers: &[&crate::gpu::Buffer],
    ) {
        for (i, buffer) in storage_buffers.iter().enumerate() {
            let slot = first_slot as usize + i;
            let active = active_buffer(&buffer_container(buffer));
            let cb = &mut *command_buffer;
            let buffer_bindings = if fragment {
                &mut cb.fragment_storage_buffer_bindings
            } else {
                &mut cb.vertex_storage_buffer_bindings
            };

            if buffer_bindings[slot] != active.buffer {
                buffer_bindings[slot] = active.buffer;
                cb.track_buffer(&active);

                if fragment {
                    cb.need_new_fragment_resource_descriptor_set = true;
                } else {
                    cb.need_new_vertex_resource_descriptor_set = true;
                }
            }
        }
    }

    /// Begin a render pass on color and depth-stencil targets. Translation
    /// of `VULKAN_BeginRenderPass()`.
    pub(super) fn begin_render_pass_internal(
        &self,
        vulkan_command_buffer: &mut VulkanCommandBuffer,
        color_target_infos: &[ColorTargetInfo<'_>],
        depth_stencil_target_info: Option<&DepthStencilTargetInfo<'_>>,
    ) {
        let mut clear_count = 0usize;
        let mut total_color_attachment_count = 0usize;
        let mut framebuffer_width = u32::MAX;
        let mut framebuffer_height = u32::MAX;

        for info in color_target_infos {
            let header = &info.texture.info;

            let w = mip_size(header.width, info.mip_level);
            let h = mip_size(header.height, info.mip_level);

            // The framebuffer cannot be larger than the smallest attachment.

            framebuffer_width = framebuffer_width.min(w);
            framebuffer_height = framebuffer_height.min(h);
        }

        if let Some(info) = depth_stencil_target_info {
            let header = &info.texture.info;

            let w = mip_size(header.width, info.mip_level as u32);
            let h = mip_size(header.height, info.mip_level as u32);

            // The framebuffer cannot be larger than the smallest attachment.

            framebuffer_width = framebuffer_width.min(w);
            framebuffer_height = framebuffer_height.min(h);
        }

        for info in color_target_infos {
            let texture_container = container(info.texture);
            let is_3d = texture_container.info.texture_type == TextureType::Texture3D;
            let subresource = self.prepare_texture_subresource_for_write(
                vulkan_command_buffer,
                &texture_container,
                if is_3d { 0 } else { info.layer_or_depth_plane },
                info.mip_level,
                info.cycle,
                VulkanTextureUsageMode::ColorAttachment,
            );

            vulkan_command_buffer.track_texture(&subresource.texture);
            vulkan_command_buffer
                .color_attachment_subresources
                .push(subresource);
            total_color_attachment_count += 1;
            clear_count += 1;

            if let Some(resolve_texture) = info.resolve_texture.filter(|_| resolves(info.store_op))
            {
                let resolve_container = container(resolve_texture);
                let resolve_subresource = self.prepare_texture_subresource_for_write(
                    vulkan_command_buffer,
                    &resolve_container,
                    info.resolve_layer,
                    info.resolve_mip_level,
                    info.cycle_resolve_texture,
                    VulkanTextureUsageMode::ColorAttachment,
                );

                vulkan_command_buffer.track_texture(&resolve_subresource.texture);
                vulkan_command_buffer
                    .resolve_attachment_subresources
                    .push(resolve_subresource);
                total_color_attachment_count += 1;
                clear_count += 1;
            }
        }

        if let Some(info) = depth_stencil_target_info {
            let texture_container = container(info.texture);
            let subresource = self.prepare_texture_subresource_for_write(
                vulkan_command_buffer,
                &texture_container,
                info.layer as u32,
                info.mip_level as u32,
                info.cycle,
                VulkanTextureUsageMode::DepthStencilAttachment,
            );

            vulkan_command_buffer.track_texture(&subresource.texture);
            vulkan_command_buffer.depth_stencil_attachment_subresource = Some(subresource);
            clear_count += 1;
        }

        // Fetch required render objects

        let Ok(render_pass) = self.fetch_render_pass(color_target_infos, depth_stencil_target_info)
        else {
            return;
        };

        let Ok(framebuffer) = self.fetch_framebuffer(
            render_pass,
            color_target_infos,
            depth_stencil_target_info,
            framebuffer_width,
            framebuffer_height,
        ) else {
            return;
        };

        vulkan_command_buffer.track_framebuffer(&framebuffer);

        // Set clear values

        let mut clear_values = vec![VkClearValue::default(); clear_count];

        let mut clear_index = 0;
        for info in color_target_infos {
            let c = info.clear_color;
            clear_values[clear_index].float32 = [c.r, c.g, c.b, c.a];
            clear_index += 1;

            if resolves(info.store_op) {
                // Skip over the resolve texture, we're not clearing it
                clear_index += 1;
            }
        }

        if let Some(info) = depth_stencil_target_info {
            // (the depthStencil member of the union: depth, then stencil)
            clear_values[total_color_attachment_count].float32 = [
                info.clear_depth,
                f32::from_bits(info.clear_stencil as u32),
                0.0,
                0.0,
            ];
        }

        let render_pass_begin_info = VkRenderPassBeginInfo {
            s_type: VK_STRUCTURE_TYPE_RENDER_PASS_BEGIN_INFO,
            p_next: null(),
            render_pass,
            framebuffer: framebuffer.framebuffer,
            p_clear_values: clear_values.as_ptr(),
            clear_value_count: clear_count as u32,
            render_area: VkRect2D {
                extent: VkExtent2D {
                    width: framebuffer_width,
                    height: framebuffer_height,
                },
                offset: VkOffset2D { x: 0, y: 0 },
            },
        };

        // SAFETY: a command buffer being recorded outside a pass, with a
        // render pass and a framebuffer of its targets.
        unsafe {
            (self.dev.cmd_begin_render_pass)(
                vulkan_command_buffer.command_buffer,
                &render_pass_begin_info,
                VK_SUBPASS_CONTENTS_INLINE,
            )
        };

        // Set sensible default states

        let default_viewport = Viewport {
            x: 0.0,
            y: 0.0,
            w: framebuffer_width as f32,
            h: framebuffer_height as f32,
            min_depth: 0.0,
            max_depth: 1.0,
        };

        self.set_current_viewport(vulkan_command_buffer, &default_viewport);

        let default_scissor = Rect {
            x: 0,
            y: 0,
            w: framebuffer_width as i32,
            h: framebuffer_height as i32,
        };

        self.set_current_scissor(vulkan_command_buffer, &default_scissor);

        let default_blend_constants = FColor {
            r: 1.0,
            g: 1.0,
            b: 1.0,
            a: 1.0,
        };

        self.set_current_blend_constants(vulkan_command_buffer, default_blend_constants);

        self.set_current_stencil_reference(vulkan_command_buffer, 0);
    }

    /// Translation of `VULKAN_BindGraphicsPipeline()`.
    pub(super) fn bind_graphics_pipeline_internal(
        &self,
        vulkan_command_buffer: &mut VulkanCommandBuffer,
        pipeline: &Arc<VulkanGraphicsPipeline>,
    ) {
        // SAFETY: a command buffer in a render pass and a live pipeline.
        unsafe {
            (self.dev.cmd_bind_pipeline)(
                vulkan_command_buffer.command_buffer,
                VK_PIPELINE_BIND_POINT_GRAPHICS,
                pipeline.pipeline,
            )
        };

        vulkan_command_buffer.current_graphics_pipeline = Some(pipeline.clone());

        vulkan_command_buffer.track_graphics_pipeline(pipeline);

        // Acquire uniform buffers if necessary
        for i in 0..pipeline.resource_layout.vertex_uniform_buffer_count as usize {
            if vulkan_command_buffer.vertex_uniform_buffers[i].is_none() {
                vulkan_command_buffer.vertex_uniform_buffers[i] =
                    self.acquire_uniform_buffer_from_pool(vulkan_command_buffer);
            }
        }

        for i in 0..pipeline.resource_layout.fragment_uniform_buffer_count as usize {
            if vulkan_command_buffer.fragment_uniform_buffers[i].is_none() {
                vulkan_command_buffer.fragment_uniform_buffers[i] =
                    self.acquire_uniform_buffer_from_pool(vulkan_command_buffer);
            }
        }

        // Mark bindings as needed
        vulkan_command_buffer.need_new_vertex_resource_descriptor_set = true;
        vulkan_command_buffer.need_new_fragment_resource_descriptor_set = true;
        vulkan_command_buffer.need_new_vertex_uniform_descriptor_set = true;
        vulkan_command_buffer.need_new_fragment_uniform_descriptor_set = true;
        vulkan_command_buffer.need_new_vertex_uniform_offsets = true;
        vulkan_command_buffer.need_new_fragment_uniform_offsets = true;
    }

    /// Translation of `VULKAN_BindVertexBuffers()`.
    pub(super) fn bind_vertex_buffers_internal(
        vulkan_command_buffer: &mut VulkanCommandBuffer,
        first_slot: u32,
        bindings: &[BufferBinding<'_>],
    ) {
        for (i, binding) in bindings.iter().enumerate() {
            let slot = first_slot as usize + i;
            let buffer = active_buffer(&buffer_container(binding.buffer));
            if vulkan_command_buffer.vertex_buffers[slot] != buffer.buffer
                || vulkan_command_buffer.vertex_buffer_offsets[slot]
                    != binding.offset as VkDeviceSize
            {
                vulkan_command_buffer.track_buffer(&buffer);

                vulkan_command_buffer.vertex_buffers[slot] = buffer.buffer;
                vulkan_command_buffer.vertex_buffer_offsets[slot] = binding.offset as VkDeviceSize;
                vulkan_command_buffer.need_vertex_buffer_bind = true;
            }
        }

        vulkan_command_buffer.vertex_buffer_count = vulkan_command_buffer
            .vertex_buffer_count
            .max(first_slot + bindings.len() as u32);
    }

    /// Translation of `VULKAN_BindIndexBuffer()`.
    pub(super) fn bind_index_buffer_internal(
        &self,
        vulkan_command_buffer: &mut VulkanCommandBuffer,
        binding: &BufferBinding<'_>,
        index_element_size: IndexElementSize,
    ) {
        let vulkan_buffer = active_buffer(&buffer_container(binding.buffer));

        vulkan_command_buffer.track_buffer(&vulkan_buffer);

        // SAFETY: a command buffer in a render pass and a live buffer.
        unsafe {
            (self.dev.cmd_bind_index_buffer)(
                vulkan_command_buffer.command_buffer,
                vulkan_buffer.buffer,
                binding.offset as VkDeviceSize,
                sdl_to_vk_index_type(index_element_size),
            )
        };
    }

    /// End a render pass, transitioning its targets back to their default
    /// usage. Translation of `VULKAN_EndRenderPass()`.
    pub(super) fn end_render_pass_internal(&self, vulkan_command_buffer: &mut VulkanCommandBuffer) {
        // SAFETY: a command buffer in a render pass.
        unsafe { (self.dev.cmd_end_render_pass)(vulkan_command_buffer.command_buffer) };

        for subresource in std::mem::take(&mut vulkan_command_buffer.color_attachment_subresources)
        {
            self.texture_subresource_transition_to_default_usage(
                vulkan_command_buffer,
                VulkanTextureUsageMode::ColorAttachment,
                &subresource.texture,
                subresource.index,
            );
        }

        for subresource in
            std::mem::take(&mut vulkan_command_buffer.resolve_attachment_subresources)
        {
            self.texture_subresource_transition_to_default_usage(
                vulkan_command_buffer,
                VulkanTextureUsageMode::ColorAttachment,
                &subresource.texture,
                subresource.index,
            );
        }

        if let Some(subresource) = vulkan_command_buffer
            .depth_stencil_attachment_subresource
            .take()
        {
            self.texture_subresource_transition_to_default_usage(
                vulkan_command_buffer,
                VulkanTextureUsageMode::DepthStencilAttachment,
                &subresource.texture,
                subresource.index,
            );
        }

        let cb = vulkan_command_buffer;
        cb.current_graphics_pipeline = None;

        cb.vertex_resource_descriptor_set = VK_NULL_HANDLE;
        cb.vertex_uniform_descriptor_set = VK_NULL_HANDLE;
        cb.fragment_resource_descriptor_set = VK_NULL_HANDLE;
        cb.fragment_uniform_descriptor_set = VK_NULL_HANDLE;

        // Reset bind state
        cb.vertex_buffers = Default::default();
        cb.vertex_buffer_offsets = Default::default();
        cb.vertex_buffer_count = 0;

        cb.vertex_sampler_bindings = Default::default();
        cb.vertex_sampler_texture_view_bindings = Default::default();
        cb.vertex_storage_texture_view_bindings = Default::default();
        cb.vertex_storage_buffer_bindings = Default::default();

        cb.fragment_sampler_bindings = Default::default();
        cb.fragment_sampler_texture_view_bindings = Default::default();
        cb.fragment_storage_texture_view_bindings = Default::default();
        cb.fragment_storage_buffer_bindings = Default::default();
    }

    // Compute Pass

    /// Begin a compute pass, transitioning its read-write resources.
    /// Translation of `VULKAN_BeginComputePass()`.
    pub(super) fn begin_compute_pass_internal(
        &self,
        vulkan_command_buffer: &mut VulkanCommandBuffer,
        storage_texture_bindings: &[StorageTextureReadWriteBinding<'_>],
        storage_buffer_bindings: &[StorageBufferReadWriteBinding<'_>],
    ) {
        vulkan_command_buffer
            .read_write_compute_storage_texture_subresources
            .clear();

        for (i, binding) in storage_texture_bindings.iter().enumerate() {
            let texture_container = container(binding.texture);
            let subresource = self.prepare_texture_subresource_for_write(
                vulkan_command_buffer,
                &texture_container,
                binding.layer,
                binding.mip_level,
                binding.cycle,
                VulkanTextureUsageMode::ComputeStorageReadWrite,
            );

            vulkan_command_buffer.read_write_compute_storage_texture_view_bindings[i] =
                subresource.get().compute_write_view;

            vulkan_command_buffer.track_texture(&subresource.texture);
            vulkan_command_buffer
                .read_write_compute_storage_texture_subresources
                .push(subresource);
        }

        for (i, binding) in storage_buffer_bindings.iter().enumerate() {
            let container = buffer_container(binding.buffer);
            let buffer = self.prepare_buffer_for_write(
                vulkan_command_buffer,
                &container,
                binding.cycle,
                VULKAN_BUFFER_USAGE_MODE_COMPUTE_STORAGE_READ_WRITE,
            );

            vulkan_command_buffer.read_write_compute_storage_buffer_bindings[i] = buffer.buffer;

            vulkan_command_buffer.track_buffer(&buffer);
            vulkan_command_buffer.read_write_compute_storage_buffers[i] = Some(buffer);
        }
    }

    /// Translation of `VULKAN_BindComputePipeline()`.
    pub(super) fn bind_compute_pipeline_internal(
        &self,
        vulkan_command_buffer: &mut VulkanCommandBuffer,
        vulkan_compute_pipeline: &Arc<VulkanComputePipeline>,
    ) {
        // SAFETY: a command buffer in a compute pass and a live pipeline.
        unsafe {
            (self.dev.cmd_bind_pipeline)(
                vulkan_command_buffer.command_buffer,
                VK_PIPELINE_BIND_POINT_COMPUTE,
                vulkan_compute_pipeline.pipeline,
            )
        };

        vulkan_command_buffer.current_compute_pipeline = Some(vulkan_compute_pipeline.clone());

        vulkan_command_buffer.track_compute_pipeline(vulkan_compute_pipeline);

        // Acquire uniform buffers if necessary
        for i in 0..vulkan_compute_pipeline.resource_layout.num_uniform_buffers as usize {
            if vulkan_command_buffer.compute_uniform_buffers[i].is_none() {
                vulkan_command_buffer.compute_uniform_buffers[i] =
                    self.acquire_uniform_buffer_from_pool(vulkan_command_buffer);
            }
        }

        // Mark binding as needed
        vulkan_command_buffer.need_new_compute_read_write_descriptor_set = true;
        vulkan_command_buffer.need_new_compute_read_only_descriptor_set = true;
        vulkan_command_buffer.need_new_compute_uniform_descriptor_set = true;
        vulkan_command_buffer.need_new_compute_uniform_offsets = true;
    }

    /// Translation of `VULKAN_BindComputeSamplers()`.
    pub(super) fn bind_compute_samplers_internal(
        vulkan_command_buffer: &mut VulkanCommandBuffer,
        first_slot: u32,
        texture_sampler_bindings: &[TextureSamplerBinding<'_>],
    ) {
        for (i, binding) in texture_sampler_bindings.iter().enumerate() {
            let slot = first_slot as usize + i;
            let active = active_texture(&container(binding.texture));
            let sampler = object::<VulkanSampler>(&binding.sampler.raw);

            if vulkan_command_buffer.compute_sampler_bindings[slot] != sampler.sampler {
                vulkan_command_buffer.track_sampler(&sampler);

                vulkan_command_buffer.compute_sampler_bindings[slot] = sampler.sampler;
                vulkan_command_buffer.need_new_compute_read_only_descriptor_set = true;
            }

            if vulkan_command_buffer.compute_sampler_texture_view_bindings[slot] != active.full_view
            {
                vulkan_command_buffer.track_texture(&active);

                vulkan_command_buffer.compute_sampler_texture_view_bindings[slot] =
                    active.full_view;
                vulkan_command_buffer.need_new_compute_read_only_descriptor_set = true;
            }
        }
    }

    /// Translation of `VULKAN_BindComputeStorageTextures()`.
    pub(super) fn bind_compute_storage_textures_internal(
        &self,
        vulkan_command_buffer: &mut VulkanCommandBuffer,
        first_slot: u32,
        storage_textures: &[&Texture],
    ) {
        for (i, texture) in storage_textures.iter().enumerate() {
            let slot = first_slot as usize + i;
            let active = active_texture(&container(texture));

            let same = vulkan_command_buffer.read_only_compute_storage_textures[slot]
                .as_ref()
                .is_some_and(|t| Arc::ptr_eq(t, &active));
            if !same {
                /* If a different texture as in this slot, transition it back to its default usage */
                if let Some(previous) =
                    vulkan_command_buffer.read_only_compute_storage_textures[slot].take()
                {
                    self.texture_transition_to_default_usage(
                        vulkan_command_buffer,
                        VulkanTextureUsageMode::ComputeStorageRead,
                        &previous,
                    );
                }

                /* Then transition the new texture and prepare it for binding */
                self.texture_transition_from_default_usage(
                    vulkan_command_buffer,
                    VulkanTextureUsageMode::ComputeStorageRead,
                    &active,
                );

                vulkan_command_buffer.track_texture(&active);

                vulkan_command_buffer.read_only_compute_storage_texture_view_bindings[slot] =
                    active.full_view;
                vulkan_command_buffer.read_only_compute_storage_textures[slot] = Some(active);
                vulkan_command_buffer.need_new_compute_read_only_descriptor_set = true;
            }
        }
    }

    /// Translation of `VULKAN_BindComputeStorageBuffers()`.
    pub(super) fn bind_compute_storage_buffers_internal(
        &self,
        vulkan_command_buffer: &mut VulkanCommandBuffer,
        first_slot: u32,
        storage_buffers: &[&crate::gpu::Buffer],
    ) {
        for (i, buffer) in storage_buffers.iter().enumerate() {
            let slot = first_slot as usize + i;
            let active = active_buffer(&buffer_container(buffer));

            let same = vulkan_command_buffer.read_only_compute_storage_buffers[slot]
                .as_ref()
                .is_some_and(|b| Arc::ptr_eq(b, &active));
            if !same {
                /* If a different buffer was in this slot, transition it back to its default usage */
                if let Some(previous) =
                    vulkan_command_buffer.read_only_compute_storage_buffers[slot].take()
                {
                    self.buffer_transition_to_default_usage(
                        vulkan_command_buffer,
                        VULKAN_BUFFER_USAGE_MODE_COMPUTE_STORAGE_READ,
                        &previous,
                    );
                }

                /* Then transition the new buffer and prepare it for binding */
                self.buffer_transition_from_default_usage(
                    vulkan_command_buffer,
                    VULKAN_BUFFER_USAGE_MODE_COMPUTE_STORAGE_READ,
                    &active,
                );

                vulkan_command_buffer.track_buffer(&active);

                vulkan_command_buffer.read_only_compute_storage_buffer_bindings[slot] =
                    active.buffer;
                vulkan_command_buffer.read_only_compute_storage_buffers[slot] = Some(active);
                vulkan_command_buffer.need_new_compute_read_only_descriptor_set = true;
            }
        }
    }

    /// Write and bind the descriptor sets of the bound compute pipeline that
    /// changed. Translation of `VULKAN_INTERNAL_BindComputeDescriptorSets()`.
    fn bind_compute_descriptor_sets(&self, command_buffer: &mut VulkanCommandBuffer) {
        let cb = command_buffer;
        if !cb.need_new_compute_read_only_descriptor_set
            && !cb.need_new_compute_read_write_descriptor_set
            && !cb.need_new_compute_uniform_descriptor_set
            && !cb.need_new_compute_uniform_offsets
        {
            return;
        }

        // Note (upstream): C dereferences a NULL pipeline when none is bound.
        let Some(pipeline) = cb.current_compute_pipeline.clone() else {
            return;
        };
        let resource_layout = &pipeline.resource_layout;
        let mut writes = DescriptorWrites::default();
        let mut dynamic_offsets = Vec::with_capacity(4);

        if cb.need_new_compute_read_only_descriptor_set {
            let descriptor_set_layout = &resource_layout.descriptor_set_layouts[0];

            cb.compute_read_only_descriptor_set =
                self.command_buffer_descriptor_set(cb, descriptor_set_layout);
            let set = cb.compute_read_only_descriptor_set;

            for i in 0..resource_layout.num_samplers {
                writes.image(
                    set,
                    i,
                    VK_DESCRIPTOR_TYPE_COMBINED_IMAGE_SAMPLER,
                    sampler_info(
                        cb.compute_sampler_bindings[i as usize],
                        cb.compute_sampler_texture_view_bindings[i as usize],
                    ),
                );
            }

            for i in 0..resource_layout.num_readonly_storage_textures {
                writes.image(
                    set,
                    resource_layout.num_samplers + i,
                    VK_DESCRIPTOR_TYPE_SAMPLED_IMAGE, // Yes, we are declaring the readonly storage texture as a sampled image, because shaders are stupid.
                    storage_image_info(
                        cb.read_only_compute_storage_texture_view_bindings[i as usize],
                    ),
                );
            }

            for i in 0..resource_layout.num_readonly_storage_buffers {
                writes.buffer(
                    set,
                    resource_layout.num_samplers
                        + resource_layout.num_readonly_storage_textures
                        + i,
                    VK_DESCRIPTOR_TYPE_STORAGE_BUFFER,
                    storage_buffer_info(cb.read_only_compute_storage_buffer_bindings[i as usize]),
                );
            }

            cb.need_new_compute_read_only_descriptor_set = false;
        }

        if cb.need_new_compute_read_write_descriptor_set {
            let descriptor_set_layout = &resource_layout.descriptor_set_layouts[1];

            cb.compute_read_write_descriptor_set =
                self.command_buffer_descriptor_set(cb, descriptor_set_layout);
            let set = cb.compute_read_write_descriptor_set;

            for i in 0..resource_layout.num_read_write_storage_textures {
                writes.image(
                    set,
                    i,
                    VK_DESCRIPTOR_TYPE_STORAGE_IMAGE,
                    storage_image_info(
                        cb.read_write_compute_storage_texture_view_bindings[i as usize],
                    ),
                );
            }

            for i in 0..resource_layout.num_read_write_storage_buffers {
                writes.buffer(
                    set,
                    resource_layout.num_read_write_storage_textures + i,
                    VK_DESCRIPTOR_TYPE_STORAGE_BUFFER,
                    storage_buffer_info(cb.read_write_compute_storage_buffer_bindings[i as usize]),
                );
            }

            cb.need_new_compute_read_write_descriptor_set = false;
        }

        if cb.need_new_compute_uniform_descriptor_set {
            let descriptor_set_layout = &resource_layout.descriptor_set_layouts[2];

            cb.compute_uniform_descriptor_set =
                self.command_buffer_descriptor_set(cb, descriptor_set_layout);

            for i in 0..resource_layout.num_uniform_buffers {
                writes.buffer(
                    cb.compute_uniform_descriptor_set,
                    i,
                    VK_DESCRIPTOR_TYPE_UNIFORM_BUFFER_DYNAMIC,
                    uniform_buffer_info(&cb.compute_uniform_buffers[i as usize]),
                );
            }

            cb.need_new_compute_uniform_descriptor_set = false;
        }

        for i in 0..resource_layout.num_uniform_buffers {
            dynamic_offsets.push(draw_offset(&cb.compute_uniform_buffers[i as usize]));
        }

        writes.update(self);

        let sets = [
            cb.compute_read_only_descriptor_set,
            cb.compute_read_write_descriptor_set,
            cb.compute_uniform_descriptor_set,
        ];

        // SAFETY: a command buffer in a compute pass, the pipeline's layout
        // and sets of its set layouts.
        unsafe {
            (self.dev.cmd_bind_descriptor_sets)(
                cb.command_buffer,
                VK_PIPELINE_BIND_POINT_COMPUTE,
                resource_layout.pipeline_layout,
                0,
                3,
                sets.as_ptr(),
                dynamic_offsets.len() as u32,
                dynamic_offsets.as_ptr(),
            )
        };

        cb.need_new_compute_uniform_offsets = false;
    }

    /// Translation of `VULKAN_DispatchCompute()`.
    pub(super) fn dispatch_compute_internal(
        &self,
        vulkan_command_buffer: &mut VulkanCommandBuffer,
        groupcount_x: u32,
        groupcount_y: u32,
        groupcount_z: u32,
    ) {
        self.bind_compute_descriptor_sets(vulkan_command_buffer);

        // SAFETY: a command buffer in a compute pass with a pipeline bound.
        unsafe {
            (self.dev.cmd_dispatch)(
                vulkan_command_buffer.command_buffer,
                groupcount_x,
                groupcount_y,
                groupcount_z,
            )
        };
    }

    /// Translation of `VULKAN_DispatchComputeIndirect()`.
    pub(super) fn dispatch_compute_indirect_internal(
        &self,
        vulkan_command_buffer: &mut VulkanCommandBuffer,
        buffer: &BufferContainer,
        offset: u32,
    ) {
        let vulkan_buffer = active_buffer(buffer);

        self.bind_compute_descriptor_sets(vulkan_command_buffer);

        // SAFETY: as above, with an indirect buffer.
        unsafe {
            (self.dev.cmd_dispatch_indirect)(
                vulkan_command_buffer.command_buffer,
                vulkan_buffer.buffer,
                offset as VkDeviceSize,
            )
        };

        vulkan_command_buffer.track_buffer(&vulkan_buffer);
    }

    /// End a compute pass, transitioning its resources back to their
    /// default usage. Translation of `VULKAN_EndComputePass()`.
    pub(super) fn end_compute_pass_internal(
        &self,
        vulkan_command_buffer: &mut VulkanCommandBuffer,
    ) {
        for subresource in std::mem::take(
            &mut vulkan_command_buffer.read_write_compute_storage_texture_subresources,
        ) {
            self.texture_subresource_transition_to_default_usage(
                vulkan_command_buffer,
                VulkanTextureUsageMode::ComputeStorageReadWrite,
                &subresource.texture,
                subresource.index,
            );
        }

        for i in 0..MAX_COMPUTE_WRITE_BUFFERS as usize {
            if let Some(buffer) = vulkan_command_buffer.read_write_compute_storage_buffers[i].take()
            {
                self.buffer_transition_to_default_usage(
                    vulkan_command_buffer,
                    VULKAN_BUFFER_USAGE_MODE_COMPUTE_STORAGE_READ_WRITE,
                    &buffer,
                );
            }
        }

        for i in 0..MAX_STORAGE_TEXTURES_PER_STAGE as usize {
            if let Some(texture) =
                vulkan_command_buffer.read_only_compute_storage_textures[i].take()
            {
                self.texture_transition_to_default_usage(
                    vulkan_command_buffer,
                    VulkanTextureUsageMode::ComputeStorageRead,
                    &texture,
                );
            }
        }

        for i in 0..MAX_STORAGE_BUFFERS_PER_STAGE as usize {
            if let Some(buffer) = vulkan_command_buffer.read_only_compute_storage_buffers[i].take()
            {
                self.buffer_transition_to_default_usage(
                    vulkan_command_buffer,
                    VULKAN_BUFFER_USAGE_MODE_COMPUTE_STORAGE_READ,
                    &buffer,
                );
            }
        }

        let cb = vulkan_command_buffer;
        // we don't need a barrier for sampler resources because sampler state is always the default if sampler bit is set
        cb.compute_sampler_texture_view_bindings = Default::default();
        cb.compute_sampler_bindings = Default::default();

        cb.read_write_compute_storage_texture_view_bindings = Default::default();
        cb.read_write_compute_storage_buffer_bindings = Default::default();

        cb.current_compute_pipeline = None;

        cb.compute_read_only_descriptor_set = VK_NULL_HANDLE;
        cb.compute_read_write_descriptor_set = VK_NULL_HANDLE;
        cb.compute_uniform_descriptor_set = VK_NULL_HANDLE;
    }

    // Copy Pass

    /// Translation of `VULKAN_UploadToTexture()`.
    pub(super) fn upload_to_texture_internal(
        &self,
        vulkan_command_buffer: &mut VulkanCommandBuffer,
        source: &TextureTransferInfo<'_>,
        destination: &TextureRegion<'_>,
        cycle: bool,
    ) {
        let transfer_buffer = active_buffer(&transfer_buffer_container(source.transfer_buffer));
        let vulkan_texture_container = container(destination.texture);

        let _defrag = self.defrag_lock.read().unwrap_or_else(|e| e.into_inner());

        // Note that the transfer buffer does not need a barrier, as it is synced by the client
        let vulkan_texture_subresource = self.prepare_texture_subresource_for_write(
            vulkan_command_buffer,
            &vulkan_texture_container,
            destination.layer,
            destination.mip_level,
            cycle,
            VulkanTextureUsageMode::CopyDestination,
        );
        let texture = &vulkan_texture_subresource.texture;

        let image_copy = VkBufferImageCopy {
            image_extent: VkExtent3D {
                width: destination.w,
                height: destination.h,
                depth: destination.d,
            },
            image_offset: VkOffset3D {
                x: destination.x as i32,
                y: destination.y as i32,
                z: destination.z as i32,
            },
            image_subresource: VkImageSubresourceLayers {
                aspect_mask: texture.aspect_flags,
                base_array_layer: destination.layer,
                layer_count: 1,
                mip_level: destination.mip_level,
            },
            buffer_offset: source.offset as VkDeviceSize,
            buffer_row_length: source.pixels_per_row,
            buffer_image_height: source.rows_per_layer,
        };

        // SAFETY: a command buffer outside a render pass, a transfer buffer
        // and an image in the copy destination layout.
        unsafe {
            (self.dev.cmd_copy_buffer_to_image)(
                vulkan_command_buffer.command_buffer,
                transfer_buffer.buffer,
                texture.image,
                VK_IMAGE_LAYOUT_TRANSFER_DST_OPTIMAL,
                1,
                &image_copy,
            )
        };

        self.texture_subresource_transition_to_default_usage(
            vulkan_command_buffer,
            VulkanTextureUsageMode::CopyDestination,
            texture,
            vulkan_texture_subresource.index,
        );

        vulkan_command_buffer.track_buffer(&transfer_buffer);
        vulkan_command_buffer.track_texture(texture);
        vulkan_command_buffer.track_texture_transfer(texture);
    }

    /// Translation of `VULKAN_UploadToBuffer()`.
    pub(super) fn upload_to_buffer_internal(
        &self,
        vulkan_command_buffer: &mut VulkanCommandBuffer,
        source: &TransferBufferLocation<'_>,
        destination: &BufferRegion<'_>,
        cycle: bool,
    ) {
        let transfer_buffer = active_buffer(&transfer_buffer_container(source.transfer_buffer));
        let buffer_container = buffer_container(destination.buffer);

        let _defrag = self.defrag_lock.read().unwrap_or_else(|e| e.into_inner());

        // Note that the transfer buffer does not need a barrier, as it is synced by the client
        let vulkan_buffer = self.prepare_buffer_for_write(
            vulkan_command_buffer,
            &buffer_container,
            cycle,
            VULKAN_BUFFER_USAGE_MODE_COPY_DESTINATION,
        );

        let buffer_copy = VkBufferCopy {
            src_offset: source.offset as VkDeviceSize,
            dst_offset: destination.offset as VkDeviceSize,
            size: destination.size as VkDeviceSize,
        };

        // SAFETY: a command buffer outside a render pass and live buffers.
        unsafe {
            (self.dev.cmd_copy_buffer)(
                vulkan_command_buffer.command_buffer,
                transfer_buffer.buffer,
                vulkan_buffer.buffer,
                1,
                &buffer_copy,
            )
        };

        self.buffer_transition_to_default_usage(
            vulkan_command_buffer,
            VULKAN_BUFFER_USAGE_MODE_COPY_DESTINATION,
            &vulkan_buffer,
        );

        vulkan_command_buffer.track_buffer(&transfer_buffer);
        vulkan_command_buffer.track_buffer(&vulkan_buffer);
        vulkan_command_buffer.track_buffer_transfer(&vulkan_buffer);
    }

    // Readback

    /// Translation of `VULKAN_DownloadFromTexture()`.
    pub(super) fn download_from_texture_internal(
        &self,
        vulkan_command_buffer: &mut VulkanCommandBuffer,
        source: &TextureRegion<'_>,
        destination: &TextureTransferInfo<'_>,
    ) {
        let texture_container = container(source.texture);
        let transfer_buffer =
            active_buffer(&transfer_buffer_container(destination.transfer_buffer));

        let _defrag = self.defrag_lock.read().unwrap_or_else(|e| e.into_inner());

        let vulkan_texture_subresource =
            Self::fetch_texture_subresource(&texture_container, source.layer, source.mip_level);
        let texture = &vulkan_texture_subresource.texture;

        // Note that the transfer buffer does not need a barrier, as it is synced by the client

        self.texture_subresource_transition_from_default_usage(
            vulkan_command_buffer,
            VulkanTextureUsageMode::CopySource,
            texture,
            vulkan_texture_subresource.index,
        );

        let image_copy = VkBufferImageCopy {
            image_extent: VkExtent3D {
                width: source.w,
                height: source.h,
                depth: source.d,
            },
            image_offset: VkOffset3D {
                x: source.x as i32,
                y: source.y as i32,
                z: source.z as i32,
            },
            image_subresource: VkImageSubresourceLayers {
                aspect_mask: texture.aspect_flags,
                base_array_layer: source.layer,
                layer_count: 1,
                mip_level: source.mip_level,
            },
            buffer_offset: destination.offset as VkDeviceSize,
            buffer_row_length: destination.pixels_per_row,
            buffer_image_height: destination.rows_per_layer,
        };

        // SAFETY: a command buffer outside a render pass, an image in the
        // copy source layout and a transfer buffer.
        unsafe {
            (self.dev.cmd_copy_image_to_buffer)(
                vulkan_command_buffer.command_buffer,
                texture.image,
                VK_IMAGE_LAYOUT_TRANSFER_SRC_OPTIMAL,
                transfer_buffer.buffer,
                1,
                &image_copy,
            )
        };

        self.texture_subresource_transition_to_default_usage(
            vulkan_command_buffer,
            VulkanTextureUsageMode::CopySource,
            texture,
            vulkan_texture_subresource.index,
        );

        vulkan_command_buffer.track_buffer(&transfer_buffer);
        vulkan_command_buffer.track_texture(texture);
        vulkan_command_buffer.track_texture_transfer(texture);
    }

    /// Translation of `VULKAN_DownloadFromBuffer()`.
    pub(super) fn download_from_buffer_internal(
        &self,
        vulkan_command_buffer: &mut VulkanCommandBuffer,
        source: &BufferRegion<'_>,
        destination: &TransferBufferLocation<'_>,
    ) {
        let buffer = active_buffer(&buffer_container(source.buffer));
        let transfer_buffer =
            active_buffer(&transfer_buffer_container(destination.transfer_buffer));

        let _defrag = self.defrag_lock.read().unwrap_or_else(|e| e.into_inner());

        // Note that transfer buffer does not need a barrier, as it is synced by the client
        self.buffer_transition_from_default_usage(
            vulkan_command_buffer,
            VULKAN_BUFFER_USAGE_MODE_COPY_SOURCE,
            &buffer,
        );

        let buffer_copy = VkBufferCopy {
            src_offset: source.offset as VkDeviceSize,
            dst_offset: destination.offset as VkDeviceSize,
            size: source.size as VkDeviceSize,
        };

        // SAFETY: a command buffer outside a render pass and live buffers.
        unsafe {
            (self.dev.cmd_copy_buffer)(
                vulkan_command_buffer.command_buffer,
                buffer.buffer,
                transfer_buffer.buffer,
                1,
                &buffer_copy,
            )
        };

        self.buffer_transition_to_default_usage(
            vulkan_command_buffer,
            VULKAN_BUFFER_USAGE_MODE_COPY_SOURCE,
            &buffer,
        );

        vulkan_command_buffer.track_buffer(&transfer_buffer);
        vulkan_command_buffer.track_buffer(&buffer);
        vulkan_command_buffer.track_buffer_transfer(&buffer);
    }

    /// Translation of `VULKAN_CopyTextureToTexture()`.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn copy_texture_to_texture_internal(
        &self,
        vulkan_command_buffer: &mut VulkanCommandBuffer,
        source: &TextureLocation<'_>,
        destination: &TextureLocation<'_>,
        w: u32,
        h: u32,
        d: u32,
        cycle: bool,
    ) {
        let _defrag = self.defrag_lock.read().unwrap_or_else(|e| e.into_inner());

        let src_subresource = Self::fetch_texture_subresource(
            &container(source.texture),
            source.layer,
            source.mip_level,
        );

        let dst_subresource = self.prepare_texture_subresource_for_write(
            vulkan_command_buffer,
            &container(destination.texture),
            destination.layer,
            destination.mip_level,
            cycle,
            VulkanTextureUsageMode::CopyDestination,
        );

        self.texture_subresource_transition_from_default_usage(
            vulkan_command_buffer,
            VulkanTextureUsageMode::CopySource,
            &src_subresource.texture,
            src_subresource.index,
        );

        let image_copy = VkImageCopy {
            src_offset: VkOffset3D {
                x: source.x as i32,
                y: source.y as i32,
                z: source.z as i32,
            },
            src_subresource: VkImageSubresourceLayers {
                aspect_mask: src_subresource.texture.aspect_flags,
                base_array_layer: source.layer,
                layer_count: 1,
                mip_level: source.mip_level,
            },
            dst_offset: VkOffset3D {
                x: destination.x as i32,
                y: destination.y as i32,
                z: destination.z as i32,
            },
            dst_subresource: VkImageSubresourceLayers {
                aspect_mask: dst_subresource.texture.aspect_flags,
                base_array_layer: destination.layer,
                layer_count: 1,
                mip_level: destination.mip_level,
            },
            extent: VkExtent3D {
                width: w,
                height: h,
                depth: d,
            },
        };

        // SAFETY: a command buffer outside a render pass and images in the
        // copy layouts.
        unsafe {
            (self.dev.cmd_copy_image)(
                vulkan_command_buffer.command_buffer,
                src_subresource.texture.image,
                VK_IMAGE_LAYOUT_TRANSFER_SRC_OPTIMAL,
                dst_subresource.texture.image,
                VK_IMAGE_LAYOUT_TRANSFER_DST_OPTIMAL,
                1,
                &image_copy,
            )
        };

        self.texture_subresource_transition_to_default_usage(
            vulkan_command_buffer,
            VulkanTextureUsageMode::CopySource,
            &src_subresource.texture,
            src_subresource.index,
        );

        self.texture_subresource_transition_to_default_usage(
            vulkan_command_buffer,
            VulkanTextureUsageMode::CopyDestination,
            &dst_subresource.texture,
            dst_subresource.index,
        );

        vulkan_command_buffer.track_texture(&src_subresource.texture);
        vulkan_command_buffer.track_texture(&dst_subresource.texture);
        vulkan_command_buffer.track_texture_transfer(&src_subresource.texture);
        vulkan_command_buffer.track_texture_transfer(&dst_subresource.texture);
    }

    /// Translation of `VULKAN_CopyBufferToBuffer()`.
    pub(super) fn copy_buffer_to_buffer_internal(
        &self,
        vulkan_command_buffer: &mut VulkanCommandBuffer,
        source: &BufferLocation<'_>,
        destination: &BufferLocation<'_>,
        size: u32,
        cycle: bool,
    ) {
        let src_container = buffer_container(source.buffer);
        let dst_container = buffer_container(destination.buffer);

        let _defrag = self.defrag_lock.read().unwrap_or_else(|e| e.into_inner());

        let dst_buffer = self.prepare_buffer_for_write(
            vulkan_command_buffer,
            &dst_container,
            cycle,
            VULKAN_BUFFER_USAGE_MODE_COPY_DESTINATION,
        );

        let src_buffer = active_buffer(&src_container);
        self.buffer_transition_from_default_usage(
            vulkan_command_buffer,
            VULKAN_BUFFER_USAGE_MODE_COPY_SOURCE,
            &src_buffer,
        );

        let buffer_copy = VkBufferCopy {
            src_offset: source.offset as VkDeviceSize,
            dst_offset: destination.offset as VkDeviceSize,
            size: size as VkDeviceSize,
        };

        // SAFETY: a command buffer outside a render pass and live buffers.
        unsafe {
            (self.dev.cmd_copy_buffer)(
                vulkan_command_buffer.command_buffer,
                src_buffer.buffer,
                dst_buffer.buffer,
                1,
                &buffer_copy,
            )
        };

        self.buffer_transition_to_default_usage(
            vulkan_command_buffer,
            VULKAN_BUFFER_USAGE_MODE_COPY_SOURCE,
            &src_buffer,
        );

        self.buffer_transition_to_default_usage(
            vulkan_command_buffer,
            VULKAN_BUFFER_USAGE_MODE_COPY_DESTINATION,
            &dst_buffer,
        );

        vulkan_command_buffer.track_buffer(&src_buffer);
        vulkan_command_buffer.track_buffer(&dst_buffer);
        vulkan_command_buffer.track_buffer_transfer(&src_buffer);
        vulkan_command_buffer.track_buffer_transfer(&dst_buffer);
    }

    /// Fill every mip level from the one above it with linear blits.
    /// Translation of `VULKAN_GenerateMipmaps()`.
    pub(super) fn generate_mipmaps_internal(
        &self,
        vulkan_command_buffer: &mut VulkanCommandBuffer,
        texture: &Texture,
    ) {
        let container = container(texture);
        let info = &container.info;

        let _defrag = self.defrag_lock.read().unwrap_or_else(|e| e.into_inner());

        // Blit each slice sequentially. Barriers, barriers everywhere!
        for layer_or_depth_index in 0..info.layer_count_or_depth {
            for level in 1..info.num_levels {
                let is_3d = info.texture_type == TextureType::Texture3D;
                let layer = if is_3d { 0 } else { layer_or_depth_index };
                let depth = if is_3d { layer_or_depth_index } else { 0 };

                let src_subresource_index =
                    get_texture_subresource_index(level - 1, layer, info.num_levels) as usize;
                let dst_subresource_index =
                    get_texture_subresource_index(level, layer, info.num_levels) as usize;

                let active = active_texture(&container);
                let src_texture_subresource = SubresourceRef {
                    texture: active.clone(),
                    index: src_subresource_index,
                };
                let dst_texture_subresource = SubresourceRef {
                    texture: active.clone(),
                    index: dst_subresource_index,
                };

                self.texture_subresource_transition_from_default_usage(
                    vulkan_command_buffer,
                    VulkanTextureUsageMode::CopySource,
                    &active,
                    src_texture_subresource.index,
                );

                self.texture_subresource_transition_from_default_usage(
                    vulkan_command_buffer,
                    VulkanTextureUsageMode::CopyDestination,
                    &active,
                    dst_texture_subresource.index,
                );

                let blit = VkImageBlit {
                    src_offsets: [
                        VkOffset3D {
                            x: 0,
                            y: 0,
                            z: depth as i32,
                        },
                        VkOffset3D {
                            x: mip_size(info.width, level - 1) as i32,
                            y: mip_size(info.height, level - 1) as i32,
                            z: depth as i32 + 1,
                        },
                    ],
                    dst_offsets: [
                        VkOffset3D {
                            x: 0,
                            y: 0,
                            z: depth as i32,
                        },
                        VkOffset3D {
                            x: mip_size(info.width, level) as i32,
                            y: mip_size(info.height, level) as i32,
                            z: depth as i32 + 1,
                        },
                    ],
                    src_subresource: VkImageSubresourceLayers {
                        aspect_mask: VK_IMAGE_ASPECT_COLOR_BIT,
                        base_array_layer: layer,
                        layer_count: 1,
                        mip_level: level - 1,
                    },
                    dst_subresource: VkImageSubresourceLayers {
                        aspect_mask: VK_IMAGE_ASPECT_COLOR_BIT,
                        base_array_layer: layer,
                        layer_count: 1,
                        mip_level: level,
                    },
                };

                // SAFETY: a command buffer outside a render pass, with the
                // two levels in the copy layouts.
                unsafe {
                    (self.dev.cmd_blit_image)(
                        vulkan_command_buffer.command_buffer,
                        active.image,
                        VK_IMAGE_LAYOUT_TRANSFER_SRC_OPTIMAL,
                        active.image,
                        VK_IMAGE_LAYOUT_TRANSFER_DST_OPTIMAL,
                        1,
                        &blit,
                        VK_FILTER_LINEAR,
                    )
                };

                self.texture_subresource_transition_to_default_usage(
                    vulkan_command_buffer,
                    VulkanTextureUsageMode::CopySource,
                    &active,
                    src_texture_subresource.index,
                );

                self.texture_subresource_transition_to_default_usage(
                    vulkan_command_buffer,
                    VulkanTextureUsageMode::CopyDestination,
                    &active,
                    dst_texture_subresource.index,
                );

                vulkan_command_buffer.track_texture(&src_texture_subresource.texture);
                vulkan_command_buffer.track_texture(&dst_texture_subresource.texture);
                vulkan_command_buffer.track_texture_transfer(&src_texture_subresource.texture);
                vulkan_command_buffer.track_texture_transfer(&dst_texture_subresource.texture);
            }
        }
    }

    /// Blit a region of a texture to a region of another. Translation of
    /// `VULKAN_Blit()`.
    ///
    /// FIXME (upstream): with `cycle` and the CLEAR load op, the clear's
    /// render pass cycles the destination, which the command buffer then
    /// uses, so the blit cycles it again and the clear is lost.
    pub(super) fn blit_internal(
        &self,
        vulkan_command_buffer: &mut VulkanCommandBuffer,
        info: &BlitInfo<'_>,
    ) {
        let src_header = &info.source.texture.info;
        let dst_header = &info.destination.texture.info;
        let src_is_3d = src_header.texture_type == TextureType::Texture3D;
        let dst_is_3d = dst_header.texture_type == TextureType::Texture3D;
        let src_layer = if src_is_3d {
            0
        } else {
            info.source.layer_or_depth_plane
        };
        let src_depth = if src_is_3d {
            info.source.layer_or_depth_plane
        } else {
            0
        };
        let dst_layer = if dst_is_3d {
            0
        } else {
            info.destination.layer_or_depth_plane
        };
        let dst_depth = if dst_is_3d {
            info.destination.layer_or_depth_plane
        } else {
            0
        };

        // Using BeginRenderPass to clear because vkCmdClearColorImage requires barriers anyway
        if info.load_op == LoadOp::Clear {
            let target_info = ColorTargetInfo {
                texture: info.destination.texture,
                mip_level: info.destination.mip_level,
                layer_or_depth_plane: info.destination.layer_or_depth_plane,
                load_op: LoadOp::Clear,
                store_op: StoreOp::Store,
                clear_color: info.clear_color,
                cycle: info.cycle,
                resolve_texture: None,
                resolve_mip_level: 0,
                resolve_layer: 0,
                cycle_resolve_texture: false,
            };
            self.begin_render_pass_internal(
                vulkan_command_buffer,
                std::slice::from_ref(&target_info),
                None,
            );
            self.end_render_pass_internal(vulkan_command_buffer);
        }

        let src_subresource = Self::fetch_texture_subresource(
            &container(info.source.texture),
            src_layer,
            info.source.mip_level,
        );

        let dst_subresource = self.prepare_texture_subresource_for_write(
            vulkan_command_buffer,
            &container(info.destination.texture),
            dst_layer,
            info.destination.mip_level,
            info.cycle,
            VulkanTextureUsageMode::CopyDestination,
        );

        self.texture_subresource_transition_from_default_usage(
            vulkan_command_buffer,
            VulkanTextureUsageMode::CopySource,
            &src_subresource.texture,
            src_subresource.index,
        );

        let src = src_subresource.get();
        let dst = dst_subresource.get();
        let mut region = VkImageBlit {
            src_subresource: VkImageSubresourceLayers {
                aspect_mask: src_subresource.texture.aspect_flags,
                base_array_layer: src.layer,
                layer_count: 1,
                mip_level: src.level,
            },
            src_offsets: [
                VkOffset3D {
                    x: info.source.x as i32,
                    y: info.source.y as i32,
                    z: src_depth as i32,
                },
                VkOffset3D {
                    x: info.source.x.wrapping_add(info.source.w) as i32,
                    y: info.source.y.wrapping_add(info.source.h) as i32,
                    z: src_depth as i32 + 1,
                },
            ],
            dst_subresource: VkImageSubresourceLayers {
                aspect_mask: dst_subresource.texture.aspect_flags,
                base_array_layer: dst.layer,
                layer_count: 1,
                mip_level: dst.level,
            },
            dst_offsets: [
                VkOffset3D {
                    x: info.destination.x as i32,
                    y: info.destination.y as i32,
                    z: dst_depth as i32,
                },
                VkOffset3D {
                    x: info.destination.x.wrapping_add(info.destination.w) as i32,
                    y: info.destination.y.wrapping_add(info.destination.h) as i32,
                    z: dst_depth as i32 + 1,
                },
            ],
        };

        if matches!(
            info.flip_mode,
            FlipMode::Horizontal | FlipMode::HorizontalAndVertical
        ) {
            // flip the x positions
            let swap = region.src_offsets[0].x;
            region.src_offsets[0].x = region.src_offsets[1].x;
            region.src_offsets[1].x = swap;
        }

        if matches!(
            info.flip_mode,
            FlipMode::Vertical | FlipMode::HorizontalAndVertical
        ) {
            // flip the y positions
            let swap = region.src_offsets[0].y;
            region.src_offsets[0].y = region.src_offsets[1].y;
            region.src_offsets[1].y = swap;
        }

        // SAFETY: a command buffer outside a render pass and images in the
        // copy layouts.
        unsafe {
            (self.dev.cmd_blit_image)(
                vulkan_command_buffer.command_buffer,
                src_subresource.texture.image,
                VK_IMAGE_LAYOUT_TRANSFER_SRC_OPTIMAL,
                dst_subresource.texture.image,
                VK_IMAGE_LAYOUT_TRANSFER_DST_OPTIMAL,
                1,
                &region,
                sdl_to_vk_filter(info.filter),
            )
        };

        self.texture_subresource_transition_to_default_usage(
            vulkan_command_buffer,
            VulkanTextureUsageMode::CopySource,
            &src_subresource.texture,
            src_subresource.index,
        );

        self.texture_subresource_transition_to_default_usage(
            vulkan_command_buffer,
            VulkanTextureUsageMode::CopyDestination,
            &dst_subresource.texture,
            dst_subresource.index,
        );

        vulkan_command_buffer.track_texture(&src_subresource.texture);
        vulkan_command_buffer.track_texture(&dst_subresource.texture);
    }
}
