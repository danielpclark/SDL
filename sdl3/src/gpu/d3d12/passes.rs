// Rust translation of the render, compute and copy pass parts of
// src/gpu/d3d12/SDL_gpu_d3d12.c from Simple DirectMedia Layer.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! Render passes (targets, the dynamic state, the bindings and their root
//! parameters, draws, indirect ones too), compute passes and dispatches,
//! copy passes (uploads and downloads with the texture pitch workaround,
//! copies), and blits and mipmaps, which go through the front end's blit
//! pipelines.

use std::sync::atomic::Ordering;
use std::sync::Arc;

use super::commands::{D3D12CommandBuffer, SubresourceRef, TextureDownload, UniformStage};
use super::d3d::*;
use super::pipelines::{D3D12ComputePipeline, D3D12GraphicsPipeline};
use super::resources::{
    align, calc_subresource, BufferContainer, D3D12Buffer, D3D12BufferType, D3D12Sampler,
    TextureContainer,
};
use super::tables::{sdl_to_d3d12_texture_format, SDL_TO_D3D12_PRIMITIVE_TYPE};
use super::{lock, object, D3D12Renderer};
use crate::gpu::sysgpu::{
    blit_common, fetch_blit_pipeline, BlitPipelineCache, BlitShaders, MAX_COLOR_TARGET_BINDINGS,
    MAX_STORAGE_BUFFERS_PER_STAGE, MAX_STORAGE_TEXTURES_PER_STAGE, MAX_TEXTURE_SAMPLERS_PER_STAGE,
    MAX_UNIFORM_BUFFERS_PER_STAGE,
};
use crate::gpu::{
    BlitInfo, BlitRegion, BufferBinding, BufferLocation, BufferRegion, ColorTargetInfo,
    CommandBuffer, DepthStencilTargetInfo, Filter, IndexElementSize, LoadOp, Sampler,
    StorageBufferReadWriteBinding, StorageTextureReadWriteBinding, StoreOp, Texture,
    TextureLocation, TextureRegion, TextureSamplerBinding, TextureTransferInfo, TextureType,
    TransferBufferLocation, Viewport,
};
use crate::log::Category;
use crate::render::direct3d11::d3d::{DXGI_FORMAT_R16_UINT, DXGI_FORMAT_R32_UINT};
use crate::video::{FColor, FlipMode, Rect};

/// Which stage a graphics binding is for.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum GraphicsStage {
    Vertex,
    Fragment,
}

impl D3D12CommandBuffer {
    /// The sampler and texture descriptor slots of a stage.
    fn sampler_slots(
        &mut self,
        stage: GraphicsStage,
    ) -> (
        &mut [CpuDescriptorHandle; MAX_TEXTURE_SAMPLERS_PER_STAGE as usize],
        &mut [CpuDescriptorHandle; MAX_TEXTURE_SAMPLERS_PER_STAGE as usize],
        &mut bool,
    ) {
        match stage {
            GraphicsStage::Vertex => (
                &mut self.vertex_sampler_descriptor_handles,
                &mut self.vertex_sampler_texture_descriptor_handles,
                &mut self.need_vertex_sampler_bind,
            ),
            GraphicsStage::Fragment => (
                &mut self.fragment_sampler_descriptor_handles,
                &mut self.fragment_sampler_texture_descriptor_handles,
                &mut self.need_fragment_sampler_bind,
            ),
        }
    }

    /// The storage texture descriptor slots of a stage.
    fn storage_texture_slots(
        &mut self,
        stage: GraphicsStage,
    ) -> (
        &mut [CpuDescriptorHandle; MAX_STORAGE_TEXTURES_PER_STAGE as usize],
        &mut bool,
    ) {
        match stage {
            GraphicsStage::Vertex => (
                &mut self.vertex_storage_texture_descriptor_handles,
                &mut self.need_vertex_storage_texture_bind,
            ),
            GraphicsStage::Fragment => (
                &mut self.fragment_storage_texture_descriptor_handles,
                &mut self.need_fragment_storage_texture_bind,
            ),
        }
    }

    /// The storage buffer descriptor slots of a stage.
    fn storage_buffer_slots(
        &mut self,
        stage: GraphicsStage,
    ) -> (
        &mut [CpuDescriptorHandle; MAX_STORAGE_BUFFERS_PER_STAGE as usize],
        &mut bool,
    ) {
        match stage {
            GraphicsStage::Vertex => (
                &mut self.vertex_storage_buffer_descriptor_handles,
                &mut self.need_vertex_storage_buffer_bind,
            ),
            GraphicsStage::Fragment => (
                &mut self.fragment_storage_buffer_descriptor_handles,
                &mut self.need_fragment_storage_buffer_bind,
            ),
        }
    }
}

/// `SDL_max(x >> shift, 1)` with the shift wrapping (a shift by 32 or more
/// is undefined in C).
fn mip_size(x: u32, shift: u32) -> u32 {
    x.wrapping_shr(shift).max(1)
}

impl D3D12Renderer {
    // Render Pass

    /// Translation of `D3D12_SetViewport()`.
    pub(super) fn set_viewport_internal(cb: &D3D12CommandBuffer, viewport: &Viewport) {
        let d3d12_viewport = D3d12Viewport {
            top_left_x: viewport.x,
            top_left_y: viewport.y,
            width: viewport.w,
            height: viewport.h,
            min_depth: viewport.min_depth,
            max_depth: viewport.max_depth,
        };
        cb.graphics_command_list.rs_set_viewport(&d3d12_viewport);
    }

    /// Translation of `D3D12_SetScissor()`.
    pub(super) fn set_scissor_internal(cb: &D3D12CommandBuffer, scissor: &Rect) {
        let scissor_rect = D3d12Rect {
            left: scissor.x,
            top: scissor.y,
            right: scissor.x.wrapping_add(scissor.w),
            bottom: scissor.y.wrapping_add(scissor.h),
        };
        cb.graphics_command_list.rs_set_scissor_rect(&scissor_rect);
    }

    /// Translation of `D3D12_SetBlendConstants()`.
    pub(super) fn set_blend_constants_internal(cb: &D3D12CommandBuffer, blend_constants: FColor) {
        let blend_factor = [
            blend_constants.r,
            blend_constants.g,
            blend_constants.b,
            blend_constants.a,
        ];
        cb.graphics_command_list.om_set_blend_factor(&blend_factor);
    }

    /// Translation of `D3D12_SetStencilReference()`.
    pub(super) fn set_stencil_reference_internal(cb: &D3D12CommandBuffer, reference: u8) {
        cb.graphics_command_list
            .om_set_stencil_ref(reference as u32);
    }

    /// The subresource of a container's active texture. Translation of
    /// `D3D12_INTERNAL_FetchTextureSubresource()`.
    pub(super) fn fetch_texture_subresource(
        container: &TextureContainer,
        layer: u32,
        level: u32,
    ) -> SubresourceRef {
        let index = calc_subresource(level, layer, container.info.num_levels);
        SubresourceRef {
            texture: lock(&container.state).active_texture.clone(),
            index: index as usize,
        }
    }

    /// The subresource a pass writes, of a fresh texture when `cycle` asks
    /// for one and the active one is in use, transitioned to
    /// `destination_usage_mode`. Translation of
    /// `D3D12_INTERNAL_PrepareTextureSubresourceForWrite()`.
    pub(super) fn prepare_texture_subresource_for_write(
        &self,
        cb: &D3D12CommandBuffer,
        container: &Arc<TextureContainer>,
        layer: u32,
        level: u32,
        cycle: bool,
        destination_usage_mode: u32,
    ) -> SubresourceRef {
        let mut subresource = Self::fetch_texture_subresource(container, layer, level);

        if container.can_be_cycled
            && cycle
            && subresource.texture.reference_count.load(Ordering::SeqCst) > 0
        {
            self.cycle_active_texture(container);

            subresource = Self::fetch_texture_subresource(container, layer, level);
        }

        cb.texture_subresource_transition_from_default_usage(destination_usage_mode, &subresource);

        subresource
    }

    /// The buffer a pass writes, a fresh one when `cycle` asks for one and
    /// the active one is in use, transitioned to `destination_state`.
    /// Translation of `D3D12_INTERNAL_PrepareBufferForWrite()`.
    pub(super) fn prepare_buffer_for_write(
        &self,
        cb: &D3D12CommandBuffer,
        container: &Arc<BufferContainer>,
        cycle: bool,
        destination_state: u32,
    ) -> Arc<D3D12Buffer> {
        let active = lock(&container.state).active_buffer.clone();
        if cycle && active.reference_count.load(Ordering::SeqCst) > 0 {
            self.cycle_active_buffer(container);
        }

        let active = lock(&container.state).active_buffer.clone();
        cb.buffer_transition_from_default_usage(destination_state, &active);

        active
    }

    /// Translation of `D3D12_BeginRenderPass()`.
    pub(super) fn begin_render_pass_internal(
        &self,
        cb: &mut D3D12CommandBuffer,
        color_target_infos: &[ColorTargetInfo<'_>],
        depth_stencil_target_info: Option<&DepthStencilTargetInfo<'_>>,
    ) {
        let mut framebuffer_width = u32::MAX;
        let mut framebuffer_height = u32::MAX;

        for info in color_target_infos {
            let header = &info.texture.info;
            let h = header.height.wrapping_shr(info.mip_level);
            let w = header.width.wrapping_shr(info.mip_level);

            // The framebuffer cannot be larger than the smallest target.

            framebuffer_width = framebuffer_width.min(w);
            framebuffer_height = framebuffer_height.min(h);
        }

        if let Some(info) = depth_stencil_target_info {
            let header = &info.texture.info;
            let h = header.height.wrapping_shr(info.mip_level as u32);
            let w = header.width.wrapping_shr(info.mip_level as u32);

            // The framebuffer cannot be larger than the smallest target.

            framebuffer_width = framebuffer_width.min(w);
            framebuffer_height = framebuffer_height.min(h);
        }

        let mut rtvs = [CpuDescriptorHandle::default(); MAX_COLOR_TARGET_BINDINGS as usize];

        for (i, info) in color_target_infos.iter().enumerate() {
            let container = object::<TextureContainer>(&info.texture.raw);
            let is_3d = container.info.texture_type == TextureType::Texture3D;
            let subresource = self.prepare_texture_subresource_for_write(
                cb,
                &container,
                if is_3d { 0 } else { info.layer_or_depth_plane },
                info.mip_level,
                info.cycle,
                D3D12_RESOURCE_STATE_RENDER_TARGET,
            );

            let rtv_index = if is_3d { info.layer_or_depth_plane } else { 0 };
            // (upstream indexes the views without a check)
            let rtv = subresource
                .get()
                .rtv_handles
                .get(rtv_index as usize)
                .map_or(CpuDescriptorHandle::default(), |d| d.cpu_handle);

            if info.load_op == LoadOp::Clear {
                let clear_color = [
                    info.clear_color.r,
                    info.clear_color.g,
                    info.clear_color.b,
                    info.clear_color.a,
                ];

                cb.graphics_command_list
                    .clear_render_target_view(rtv, &clear_color);
            }

            rtvs[i] = rtv;
            cb.track_texture(&subresource.texture);
            cb.color_target_subresources[i] = Some(subresource);

            if matches!(info.store_op, StoreOp::Resolve | StoreOp::ResolveAndStore) {
                if let Some(resolve_texture) = info.resolve_texture {
                    let resolve_container = object::<TextureContainer>(&resolve_texture.raw);
                    let resolve_subresource = self.prepare_texture_subresource_for_write(
                        cb,
                        &resolve_container,
                        info.resolve_layer,
                        info.resolve_mip_level,
                        info.cycle_resolve_texture,
                        D3D12_RESOURCE_STATE_RESOLVE_DEST,
                    );

                    cb.track_texture(&resolve_subresource.texture);
                    cb.color_resolve_subresources[i] = Some(resolve_subresource);
                }
            }
        }

        let mut dsv = CpuDescriptorHandle::default();
        if let Some(info) = depth_stencil_target_info {
            let container = object::<TextureContainer>(&info.texture.raw);
            let subresource = self.prepare_texture_subresource_for_write(
                cb,
                &container,
                info.layer as u32,
                info.mip_level as u32,
                info.cycle,
                D3D12_RESOURCE_STATE_DEPTH_WRITE,
            );
            let dsv_handle = subresource
                .get()
                .dsv_handle
                .as_ref()
                .map_or(CpuDescriptorHandle::default(), |d| d.cpu_handle);

            if info.load_op == LoadOp::Clear || info.stencil_load_op == LoadOp::Clear {
                let mut clear_flags = 0;
                if info.load_op == LoadOp::Clear {
                    clear_flags |= D3D12_CLEAR_FLAG_DEPTH;
                }
                if info.stencil_load_op == LoadOp::Clear {
                    clear_flags |= D3D12_CLEAR_FLAG_STENCIL;
                }

                cb.graphics_command_list.clear_depth_stencil_view(
                    dsv_handle,
                    clear_flags,
                    info.clear_depth,
                    info.clear_stencil,
                );
            }

            dsv = dsv_handle;
            cb.track_texture(&subresource.texture);
            cb.depth_stencil_texture_subresource = Some(subresource);
        }

        cb.graphics_command_list.om_set_render_targets(
            &rtvs[..color_target_infos.len()],
            depth_stencil_target_info.map(|_| &dsv),
        );

        // Set sensible default states
        let default_viewport = Viewport {
            x: 0.0,
            y: 0.0,
            w: framebuffer_width as f32,
            h: framebuffer_height as f32,
            min_depth: 0.0,
            max_depth: 1.0,
        };

        Self::set_viewport_internal(cb, &default_viewport);

        let default_scissor = Rect {
            x: 0,
            y: 0,
            w: framebuffer_width as i32,
            h: framebuffer_height as i32,
        };

        Self::set_scissor_internal(cb, &default_scissor);

        Self::set_stencil_reference_internal(cb, 0);

        let blend_constants = FColor {
            r: 1.0,
            g: 1.0,
            b: 1.0,
            a: 1.0,
        };

        Self::set_blend_constants_internal(cb, blend_constants);
    }

    /// Translation of `D3D12_BindGraphicsPipeline()`.
    pub(super) fn bind_graphics_pipeline_internal(
        &self,
        cb: &mut D3D12CommandBuffer,
        pipeline: &Arc<D3D12GraphicsPipeline>,
    ) {
        cb.current_graphics_pipeline = Some(pipeline.clone());

        // Set the pipeline state
        let list = &cb.graphics_command_list;
        list.set_pipeline_state(&pipeline.pipeline_state);
        list.set_graphics_root_signature(&pipeline.root_signature.handle);
        list.ia_set_primitive_topology(
            SDL_TO_D3D12_PRIMITIVE_TYPE[pipeline.primitive_type as usize],
        );

        // Mark that bindings are needed
        cb.need_vertex_sampler_bind = true;
        cb.need_vertex_storage_texture_bind = true;
        cb.need_vertex_storage_buffer_bind = true;
        cb.need_fragment_sampler_bind = true;
        cb.need_fragment_storage_texture_bind = true;
        cb.need_fragment_storage_buffer_bind = true;

        for i in 0..MAX_UNIFORM_BUFFERS_PER_STAGE as usize {
            cb.need_vertex_uniform_buffer_bind[i] = true;
            cb.need_fragment_uniform_buffer_bind[i] = true;
        }

        for i in 0..pipeline.header.num_vertex_uniform_buffers as usize {
            if cb.vertex_uniform_buffers[i].is_none() {
                cb.vertex_uniform_buffers[i] = self.acquire_uniform_buffer(cb);
            }
        }

        for i in 0..pipeline.header.num_fragment_uniform_buffers as usize {
            if cb.fragment_uniform_buffers[i].is_none() {
                cb.fragment_uniform_buffers[i] = self.acquire_uniform_buffer(cb);
            }
        }

        cb.track_graphics_pipeline(pipeline);
    }

    /// Translation of `D3D12_BindVertexBuffers()`.
    pub(super) fn bind_vertex_buffers_internal(
        cb: &mut D3D12CommandBuffer,
        first_slot: u32,
        bindings: &[BufferBinding<'_>],
    ) {
        for (i, binding) in bindings.iter().enumerate() {
            let slot = first_slot as usize + i;
            let container = object::<BufferContainer>(&binding.buffer.raw);
            let current_buffer = lock(&container.state).active_buffer.clone();

            let same = cb.vertex_buffers[slot]
                .as_ref()
                .is_some_and(|b| Arc::ptr_eq(b, &current_buffer));
            if !same || cb.vertex_buffer_offsets[slot] != binding.offset {
                cb.track_buffer(&current_buffer);

                cb.vertex_buffers[slot] = Some(current_buffer);
                cb.vertex_buffer_offsets[slot] = binding.offset;
                cb.need_vertex_buffer_bind = true;
            }
        }

        cb.vertex_buffer_count = cb
            .vertex_buffer_count
            .max(first_slot + bindings.len() as u32);
    }

    /// Translation of `D3D12_BindIndexBuffer()`.
    pub(super) fn bind_index_buffer_internal(
        cb: &mut D3D12CommandBuffer,
        binding: &BufferBinding<'_>,
        index_element_size: IndexElementSize,
    ) {
        let container = object::<BufferContainer>(&binding.buffer.raw);
        let buffer = lock(&container.state).active_buffer.clone();

        cb.track_buffer(&buffer);

        let view = IndexBufferView {
            buffer_location: buffer.virtual_address + binding.offset as u64,
            size_in_bytes: buffer.size.wrapping_sub(binding.offset),
            format: if index_element_size == IndexElementSize::Bits16 {
                DXGI_FORMAT_R16_UINT
            } else {
                DXGI_FORMAT_R32_UINT
            },
        };

        cb.graphics_command_list.ia_set_index_buffer(&view);
    }

    /// Translation of `D3D12_BindVertexSamplers()` and
    /// `D3D12_BindFragmentSamplers()` (and the samplers of
    /// `D3D12_BindComputeSamplers()`, which `stage` `None` stands for).
    fn bind_samplers(
        cb: &mut D3D12CommandBuffer,
        stage: Option<GraphicsStage>,
        first_slot: u32,
        texture_sampler_bindings: &[TextureSamplerBinding<'_>],
    ) {
        for (i, binding) in texture_sampler_bindings.iter().enumerate() {
            let slot = first_slot as usize + i;
            let container = object::<TextureContainer>(&binding.texture.raw);
            let sampler = object::<D3D12Sampler>(&binding.sampler.raw);
            let active_texture = lock(&container.state).active_texture.clone();

            let (samplers, textures, need_bind) = match stage {
                Some(stage) => cb.sampler_slots(stage),
                None => (
                    &mut cb.compute_sampler_descriptor_handles,
                    &mut cb.compute_sampler_texture_descriptor_handles,
                    &mut cb.need_compute_sampler_bind,
                ),
            };
            let mut track_sampler = false;
            let mut track_texture = false;

            if samplers[slot].ptr != sampler.handle.cpu_handle.ptr {
                track_sampler = true;

                samplers[slot] = sampler.handle.cpu_handle;
                *need_bind = true;
            }

            let srv = active_texture.srv_cpu_handle();
            if textures[slot].ptr != srv.ptr {
                track_texture = true;

                textures[slot] = srv;
                *need_bind = true;
            }

            if track_sampler {
                cb.track_sampler(&sampler);
            }
            if track_texture {
                cb.track_texture(&active_texture);
            }
        }
    }

    /// Translation of `D3D12_BindVertexSamplers()` and
    /// `D3D12_BindFragmentSamplers()`.
    pub(super) fn bind_graphics_samplers(
        cb: &mut D3D12CommandBuffer,
        stage: GraphicsStage,
        first_slot: u32,
        texture_sampler_bindings: &[TextureSamplerBinding<'_>],
    ) {
        Self::bind_samplers(cb, Some(stage), first_slot, texture_sampler_bindings);
    }

    /// Translation of `D3D12_BindVertexStorageTextures()` and
    /// `D3D12_BindFragmentStorageTextures()`.
    pub(super) fn bind_graphics_storage_textures(
        cb: &mut D3D12CommandBuffer,
        stage: GraphicsStage,
        first_slot: u32,
        storage_textures: &[&Texture],
    ) {
        for (i, texture) in storage_textures.iter().enumerate() {
            let slot = first_slot as usize + i;
            let container = object::<TextureContainer>(&texture.raw);
            let texture = lock(&container.state).active_texture.clone();

            let srv = texture.srv_cpu_handle();
            let (handles, need_bind) = cb.storage_texture_slots(stage);
            if handles[slot].ptr != srv.ptr {
                handles[slot] = srv;
                *need_bind = true;

                cb.track_texture(&texture);
            }
        }
    }

    /// Translation of `D3D12_BindVertexStorageBuffers()` and
    /// `D3D12_BindFragmentStorageBuffers()`.
    pub(super) fn bind_graphics_storage_buffers(
        cb: &mut D3D12CommandBuffer,
        stage: GraphicsStage,
        first_slot: u32,
        storage_buffers: &[&crate::gpu::Buffer],
    ) {
        for (i, buffer) in storage_buffers.iter().enumerate() {
            let slot = first_slot as usize + i;
            let container = object::<BufferContainer>(&buffer.raw);
            let active_buffer = lock(&container.state).active_buffer.clone();

            let srv = active_buffer.srv_cpu_handle();
            let (handles, need_bind) = cb.storage_buffer_slots(stage);
            if handles[slot].ptr != srv.ptr {
                handles[slot] = srv;
                *need_bind = true;

                cb.track_buffer(&active_buffer);
            }
        }
    }

    /// Write a table of descriptors and set it as a graphics or compute
    /// root parameter (the pattern of the `Bind*Resources()` functions).
    fn bind_descriptor_table(
        &self,
        cb: &mut D3D12CommandBuffer,
        compute: bool,
        heap_type: u32,
        handles: &[CpuDescriptorHandle],
        root_index: i32,
    ) {
        let Some(gpu_descriptor_handle) = self.write_gpu_descriptors(cb, heap_type, handles) else {
            return;
        };

        if compute {
            cb.graphics_command_list
                .set_compute_root_descriptor_table(root_index, gpu_descriptor_handle);
        } else {
            cb.graphics_command_list
                .set_graphics_root_descriptor_table(root_index, gpu_descriptor_handle);
        }
    }

    /// Bind what the draw needs that changed. Translation of
    /// `D3D12_INTERNAL_BindGraphicsResources()`.
    fn bind_graphics_resources(&self, cb: &mut D3D12CommandBuffer) {
        let Some(graphics_pipeline) = cb.current_graphics_pipeline.clone() else {
            return;
        };
        let header = &graphics_pipeline.header;
        let root_signature = &graphics_pipeline.root_signature;

        /* Acquire GPU descriptor heaps if we haven't yet */
        if cb.gpu_descriptor_heaps[0].is_none() && self.set_gpu_descriptor_heaps(cb).is_err() {
            return;
        }

        if cb.need_vertex_buffer_bind {
            let vertex_buffer_views: Vec<VertexBufferView> = (0..cb.vertex_buffer_count as usize)
                .map(|i| {
                    let offset = cb.vertex_buffer_offsets[i];
                    // (upstream reads a NULL buffer of an unbound slot)
                    let (address, size) = cb.vertex_buffers[i]
                        .as_ref()
                        .map_or((0, 0), |b| (b.virtual_address, b.size));
                    VertexBufferView {
                        buffer_location: address + offset as u64,
                        size_in_bytes: size.wrapping_sub(offset),
                        stride_in_bytes: graphics_pipeline.vertex_strides[i],
                    }
                })
                .collect();

            cb.graphics_command_list
                .ia_set_vertex_buffers(0, &vertex_buffer_views);

            cb.need_vertex_buffer_bind = false;
        }

        for stage in [GraphicsStage::Vertex, GraphicsStage::Fragment] {
            let (
                num_samplers,
                num_storage_textures,
                num_storage_buffers,
                num_uniform_buffers,
                sampler_root_index,
                sampler_texture_root_index,
                storage_texture_root_index,
                storage_buffer_root_index,
                uniform_buffer_root_index,
                shader_stage,
            ) = match stage {
                GraphicsStage::Vertex => (
                    header.num_vertex_samplers as usize,
                    header.num_vertex_storage_textures as usize,
                    header.num_vertex_storage_buffers as usize,
                    header.num_vertex_uniform_buffers as usize,
                    root_signature.vertex_sampler_root_index,
                    root_signature.vertex_sampler_texture_root_index,
                    root_signature.vertex_storage_texture_root_index,
                    root_signature.vertex_storage_buffer_root_index,
                    &root_signature.vertex_uniform_buffer_root_index,
                    UniformStage::Vertex,
                ),
                GraphicsStage::Fragment => (
                    header.num_fragment_samplers as usize,
                    header.num_fragment_storage_textures as usize,
                    header.num_fragment_storage_buffers as usize,
                    header.num_fragment_uniform_buffers as usize,
                    root_signature.fragment_sampler_root_index,
                    root_signature.fragment_sampler_texture_root_index,
                    root_signature.fragment_storage_texture_root_index,
                    root_signature.fragment_storage_buffer_root_index,
                    &root_signature.fragment_uniform_buffer_root_index,
                    UniformStage::Fragment,
                ),
            };

            let (samplers, textures, need_bind) = cb.sampler_slots(stage);
            if *need_bind {
                *need_bind = false;
                let samplers = samplers[..num_samplers].to_vec();
                let textures = textures[..num_samplers].to_vec();
                if num_samplers > 0 {
                    self.bind_descriptor_table(
                        cb,
                        false,
                        D3D12_DESCRIPTOR_HEAP_TYPE_SAMPLER,
                        &samplers,
                        sampler_root_index,
                    );

                    self.bind_descriptor_table(
                        cb,
                        false,
                        D3D12_DESCRIPTOR_HEAP_TYPE_CBV_SRV_UAV,
                        &textures,
                        sampler_texture_root_index,
                    );
                }
            }

            let (handles, need_bind) = cb.storage_texture_slots(stage);
            if *need_bind {
                *need_bind = false;
                let handles = handles[..num_storage_textures].to_vec();
                if num_storage_textures > 0 {
                    self.bind_descriptor_table(
                        cb,
                        false,
                        D3D12_DESCRIPTOR_HEAP_TYPE_CBV_SRV_UAV,
                        &handles,
                        storage_texture_root_index,
                    );
                }
            }

            let (handles, need_bind) = cb.storage_buffer_slots(stage);
            if *need_bind {
                *need_bind = false;
                let handles = handles[..num_storage_buffers].to_vec();
                if num_storage_buffers > 0 {
                    self.bind_descriptor_table(
                        cb,
                        false,
                        D3D12_DESCRIPTOR_HEAP_TYPE_CBV_SRV_UAV,
                        &handles,
                        storage_buffer_root_index,
                    );
                }
            }

            for (i, &root_index) in uniform_buffer_root_index.iter().enumerate() {
                let need_bind = match stage {
                    GraphicsStage::Vertex => &mut cb.need_vertex_uniform_buffer_bind[i],
                    GraphicsStage::Fragment => &mut cb.need_fragment_uniform_buffer_bind[i],
                };
                if *need_bind {
                    *need_bind = false;
                    if num_uniform_buffers > i {
                        if let Some(uniform_buffer) = cb.uniform_buffer(shader_stage, i) {
                            let address = uniform_buffer.buffer.virtual_address
                                + uniform_buffer.draw_offset as u64;
                            cb.graphics_command_list
                                .set_graphics_root_constant_buffer_view(root_index, address);
                        }
                    }
                }
            }
        }
    }

    /// Translation of `D3D12_DrawIndexedPrimitives()`.
    pub(super) fn draw_indexed_primitives_internal(
        &self,
        cb: &mut D3D12CommandBuffer,
        num_indices: u32,
        num_instances: u32,
        first_index: u32,
        vertex_offset: i32,
        first_instance: u32,
    ) {
        self.bind_graphics_resources(cb);

        cb.graphics_command_list.draw_indexed_instanced(
            num_indices,
            num_instances,
            first_index,
            vertex_offset,
            first_instance,
        );
    }

    /// Translation of `D3D12_DrawPrimitives()`.
    pub(super) fn draw_primitives_internal(
        &self,
        cb: &mut D3D12CommandBuffer,
        num_vertices: u32,
        num_instances: u32,
        first_vertex: u32,
        first_instance: u32,
    ) {
        self.bind_graphics_resources(cb);

        cb.graphics_command_list.draw_instanced(
            num_vertices,
            num_instances,
            first_vertex,
            first_instance,
        );
    }

    /// Translation of `D3D12_DrawPrimitivesIndirect()` and
    /// `D3D12_DrawIndexedPrimitivesIndirect()`.
    pub(super) fn draw_primitives_indirect_internal(
        &self,
        cb: &mut D3D12CommandBuffer,
        container: &BufferContainer,
        offset: u32,
        draw_count: u32,
        indexed: bool,
    ) {
        let buffer = lock(&container.state).active_buffer.clone();

        self.bind_graphics_resources(cb);

        cb.graphics_command_list.execute_indirect(
            if indexed {
                &self.indirect_indexed_draw_command_signature
            } else {
                &self.indirect_draw_command_signature
            },
            draw_count,
            &buffer.handle,
            offset as u64,
        );

        cb.track_buffer(&buffer);
    }

    /// Translation of `D3D12_EndRenderPass()`.
    pub(super) fn end_render_pass_internal(cb: &mut D3D12CommandBuffer) {
        for i in 0..MAX_COLOR_TARGET_BINDINGS as usize {
            let Some(color_target) = cb.color_target_subresources[i].clone() else {
                continue;
            };
            if let Some(resolve) = cb.color_resolve_subresources[i].clone() {
                // Resolving requires some extra barriers
                cb.texture_subresource_barrier(
                    D3D12_RESOURCE_STATE_RENDER_TARGET,
                    D3D12_RESOURCE_STATE_RESOLVE_SOURCE,
                    &color_target,
                );

                cb.graphics_command_list.resolve_subresource(
                    &resolve.texture.resource,
                    resolve.get().index,
                    &color_target.texture.resource,
                    color_target.get().index,
                    sdl_to_d3d12_texture_format(color_target.texture.info.format),
                );

                cb.texture_subresource_transition_to_default_usage(
                    D3D12_RESOURCE_STATE_RESOLVE_SOURCE,
                    &color_target,
                );

                cb.texture_subresource_transition_to_default_usage(
                    D3D12_RESOURCE_STATE_RESOLVE_DEST,
                    &resolve,
                );
            } else {
                cb.texture_subresource_transition_to_default_usage(
                    D3D12_RESOURCE_STATE_RENDER_TARGET,
                    &color_target,
                );
            }
        }

        if let Some(depth_stencil) = cb.depth_stencil_texture_subresource.take() {
            cb.texture_subresource_transition_to_default_usage(
                D3D12_RESOURCE_STATE_DEPTH_WRITE,
                &depth_stencil,
            );
        }

        cb.current_graphics_pipeline = None;

        cb.graphics_command_list.om_set_render_targets(&[], None);

        // Reset bind state
        cb.color_target_subresources = Default::default();
        cb.color_resolve_subresources = Default::default();
        cb.depth_stencil_texture_subresource = None;

        cb.vertex_buffers = Default::default();
        cb.vertex_buffer_offsets = Default::default();
        cb.vertex_buffer_count = 0;

        cb.vertex_sampler_texture_descriptor_handles = Default::default();
        cb.vertex_sampler_descriptor_handles = Default::default();
        cb.vertex_storage_texture_descriptor_handles = Default::default();
        cb.vertex_storage_buffer_descriptor_handles = Default::default();

        cb.fragment_sampler_texture_descriptor_handles = Default::default();
        cb.fragment_sampler_descriptor_handles = Default::default();
        cb.fragment_storage_texture_descriptor_handles = Default::default();
        cb.fragment_storage_buffer_descriptor_handles = Default::default();
    }

    // Compute Pass

    /// Translation of `D3D12_BeginComputePass()`.
    pub(super) fn begin_compute_pass_internal(
        &self,
        cb: &mut D3D12CommandBuffer,
        storage_texture_bindings: &[StorageTextureReadWriteBinding<'_>],
        storage_buffer_bindings: &[StorageBufferReadWriteBinding<'_>],
    ) {
        cb.compute_read_write_storage_texture_subresource_count =
            storage_texture_bindings.len() as u32;
        cb.compute_read_write_storage_buffer_count = storage_buffer_bindings.len() as u32;

        /* Read-write resources will be actually bound in BindComputePipeline
         * after the root signature is set.
         * We also have to scan to see which barriers we actually need because depth slices aren't separate subresources
         */
        for (i, binding) in storage_texture_bindings.iter().enumerate() {
            let container = object::<TextureContainer>(&binding.texture.raw);

            let subresource = self.prepare_texture_subresource_for_write(
                cb,
                &container,
                binding.layer,
                binding.mip_level,
                binding.cycle,
                D3D12_RESOURCE_STATE_UNORDERED_ACCESS,
            );

            cb.compute_read_write_storage_texture_descriptor_handles[i] = subresource
                .get()
                .uav_handle
                .as_ref()
                .map_or(CpuDescriptorHandle::default(), |d| d.cpu_handle);

            cb.track_texture(&subresource.texture);
            cb.compute_read_write_storage_texture_subresources[i] = Some(subresource);
        }

        for (i, binding) in storage_buffer_bindings.iter().enumerate() {
            let container = object::<BufferContainer>(&binding.buffer.raw);

            let buffer = self.prepare_buffer_for_write(
                cb,
                &container,
                binding.cycle,
                D3D12_RESOURCE_STATE_UNORDERED_ACCESS,
            );

            cb.compute_read_write_storage_buffer_descriptor_handles[i] = buffer.uav_cpu_handle();

            cb.track_buffer(&buffer);
            cb.compute_read_write_storage_buffers[i] = Some(buffer);
        }
    }

    /// Translation of `D3D12_BindComputePipeline()`.
    pub(super) fn bind_compute_pipeline_internal(
        &self,
        cb: &mut D3D12CommandBuffer,
        pipeline: &Arc<D3D12ComputePipeline>,
    ) {
        /* Acquire GPU descriptor heaps if we haven't yet */
        if cb.gpu_descriptor_heaps[0].is_none() && self.set_gpu_descriptor_heaps(cb).is_err() {
            return;
        }

        cb.graphics_command_list
            .set_pipeline_state(&pipeline.pipeline_state);

        cb.graphics_command_list
            .set_compute_root_signature(&pipeline.root_signature.handle);

        cb.current_compute_pipeline = Some(pipeline.clone());

        cb.need_compute_sampler_bind = true;
        cb.need_compute_read_only_storage_texture_bind = true;
        cb.need_compute_read_only_storage_buffer_bind = true;

        cb.need_compute_uniform_buffer_bind = [true; MAX_UNIFORM_BUFFERS_PER_STAGE as usize];

        for i in 0..pipeline.header.num_uniform_buffers as usize {
            if cb.compute_uniform_buffers[i].is_none() {
                cb.compute_uniform_buffers[i] = self.acquire_uniform_buffer(cb);
            }
        }

        cb.track_compute_pipeline(pipeline);

        // Bind write-only resources after setting root signature
        // (upstream copies the pipeline's count of handles, then writes the
        // pass's count of them)
        if pipeline.header.num_readwrite_storage_textures > 0 {
            let count = cb.compute_read_write_storage_texture_subresource_count as usize;
            let handles =
                cb.compute_read_write_storage_texture_descriptor_handles[..count].to_vec();
            self.bind_descriptor_table(
                cb,
                true,
                D3D12_DESCRIPTOR_HEAP_TYPE_CBV_SRV_UAV,
                &handles,
                pipeline
                    .root_signature
                    .read_write_storage_texture_root_index,
            );
        }

        if pipeline.header.num_readwrite_storage_buffers > 0 {
            let count = cb.compute_read_write_storage_buffer_count as usize;
            let handles = cb.compute_read_write_storage_buffer_descriptor_handles[..count].to_vec();
            self.bind_descriptor_table(
                cb,
                true,
                D3D12_DESCRIPTOR_HEAP_TYPE_CBV_SRV_UAV,
                &handles,
                pipeline.root_signature.read_write_storage_buffer_root_index,
            );
        }
    }

    /// Translation of `D3D12_BindComputeSamplers()`.
    pub(super) fn bind_compute_samplers_internal(
        cb: &mut D3D12CommandBuffer,
        first_slot: u32,
        texture_sampler_bindings: &[TextureSamplerBinding<'_>],
    ) {
        Self::bind_samplers(cb, None, first_slot, texture_sampler_bindings);
    }

    /// Translation of `D3D12_BindComputeStorageTextures()`.
    pub(super) fn bind_compute_storage_textures_internal(
        cb: &mut D3D12CommandBuffer,
        first_slot: u32,
        storage_textures: &[&Texture],
    ) {
        for (i, texture) in storage_textures.iter().enumerate() {
            let slot = first_slot as usize + i;
            let container = object::<TextureContainer>(&texture.raw);
            let active_texture = lock(&container.state).active_texture.clone();

            let same = cb.compute_read_only_storage_textures[slot]
                .as_ref()
                .is_some_and(|t| Arc::ptr_eq(t, &active_texture));
            if !same {
                /* If a different texture was in this slot, transition it back to its default usage */
                if let Some(previous) = cb.compute_read_only_storage_textures[slot].take() {
                    cb.texture_transition_to_default_usage(
                        D3D12_RESOURCE_STATE_NON_PIXEL_SHADER_RESOURCE,
                        &previous,
                    );
                }

                /* Then transition the new texture and prepare it for binding */
                cb.texture_transition_from_default_usage(
                    D3D12_RESOURCE_STATE_NON_PIXEL_SHADER_RESOURCE,
                    &active_texture,
                );

                cb.track_texture(&active_texture);

                cb.compute_read_only_storage_texture_descriptor_handles[slot] =
                    active_texture.srv_cpu_handle();
                cb.compute_read_only_storage_textures[slot] = Some(active_texture);
                cb.need_compute_read_only_storage_texture_bind = true;
            }
        }
    }

    /// Translation of `D3D12_BindComputeStorageBuffers()`.
    pub(super) fn bind_compute_storage_buffers_internal(
        cb: &mut D3D12CommandBuffer,
        first_slot: u32,
        storage_buffers: &[&crate::gpu::Buffer],
    ) {
        for (i, buffer) in storage_buffers.iter().enumerate() {
            let slot = first_slot as usize + i;
            let container = object::<BufferContainer>(&buffer.raw);
            let buffer = lock(&container.state).active_buffer.clone();

            let same = cb.compute_read_only_storage_buffers[slot]
                .as_ref()
                .is_some_and(|b| Arc::ptr_eq(b, &buffer));
            if !same {
                /* If a different buffer was in this slot, transition it back to its default usage */
                if let Some(previous) = cb.compute_read_only_storage_buffers[slot].take() {
                    cb.buffer_transition_to_default_usage(
                        D3D12_RESOURCE_STATE_NON_PIXEL_SHADER_RESOURCE,
                        &previous,
                    );
                }

                /* Then transition the new buffer and prepare it for binding */
                cb.buffer_transition_from_default_usage(
                    D3D12_RESOURCE_STATE_NON_PIXEL_SHADER_RESOURCE,
                    &buffer,
                );

                cb.track_buffer(&buffer);

                cb.compute_read_only_storage_buffer_descriptor_handles[slot] =
                    buffer.srv_cpu_handle();
                cb.compute_read_only_storage_buffers[slot] = Some(buffer);
                cb.need_compute_read_only_storage_buffer_bind = true;
            }
        }
    }

    /// Bind what the dispatch needs that changed. Translation of
    /// `D3D12_INTERNAL_BindComputeResources()`.
    fn bind_compute_resources(&self, cb: &mut D3D12CommandBuffer) {
        let Some(compute_pipeline) = cb.current_compute_pipeline.clone() else {
            return;
        };
        let header = &compute_pipeline.header;
        let root_signature = &compute_pipeline.root_signature;

        /* Acquire GPU descriptor heaps if we haven't yet */
        if cb.gpu_descriptor_heaps[0].is_none() && self.set_gpu_descriptor_heaps(cb).is_err() {
            return;
        }

        if cb.need_compute_sampler_bind {
            let num_samplers = header.num_samplers as usize;
            if num_samplers > 0 {
                let samplers = cb.compute_sampler_descriptor_handles[..num_samplers].to_vec();
                self.bind_descriptor_table(
                    cb,
                    true,
                    D3D12_DESCRIPTOR_HEAP_TYPE_SAMPLER,
                    &samplers,
                    root_signature.sampler_root_index,
                );

                let textures =
                    cb.compute_sampler_texture_descriptor_handles[..num_samplers].to_vec();
                self.bind_descriptor_table(
                    cb,
                    true,
                    D3D12_DESCRIPTOR_HEAP_TYPE_CBV_SRV_UAV,
                    &textures,
                    root_signature.sampler_texture_root_index,
                );
            }
            cb.need_compute_sampler_bind = false;
        }

        if cb.need_compute_read_only_storage_texture_bind {
            let count = header.num_readonly_storage_textures as usize;
            if count > 0 {
                let handles =
                    cb.compute_read_only_storage_texture_descriptor_handles[..count].to_vec();
                self.bind_descriptor_table(
                    cb,
                    true,
                    D3D12_DESCRIPTOR_HEAP_TYPE_CBV_SRV_UAV,
                    &handles,
                    root_signature.read_only_storage_texture_root_index,
                );
            }
            cb.need_compute_read_only_storage_texture_bind = false;
        }

        if cb.need_compute_read_only_storage_buffer_bind {
            let count = header.num_readonly_storage_buffers as usize;
            if count > 0 {
                let handles =
                    cb.compute_read_only_storage_buffer_descriptor_handles[..count].to_vec();
                self.bind_descriptor_table(
                    cb,
                    true,
                    D3D12_DESCRIPTOR_HEAP_TYPE_CBV_SRV_UAV,
                    &handles,
                    root_signature.read_only_storage_buffer_root_index,
                );
            }
            cb.need_compute_read_only_storage_buffer_bind = false;
        }

        for i in 0..MAX_UNIFORM_BUFFERS_PER_STAGE as usize {
            if cb.need_compute_uniform_buffer_bind[i] && header.num_uniform_buffers as usize > i {
                if let Some(uniform_buffer) = cb.uniform_buffer(UniformStage::Compute, i) {
                    let address =
                        uniform_buffer.buffer.virtual_address + uniform_buffer.draw_offset as u64;
                    cb.graphics_command_list
                        .set_compute_root_constant_buffer_view(
                            root_signature.uniform_buffer_root_index[i],
                            address,
                        );
                }
            }
            cb.need_compute_uniform_buffer_bind[i] = false;
        }
    }

    /// Translation of `D3D12_DispatchCompute()`.
    pub(super) fn dispatch_compute_internal(
        &self,
        cb: &mut D3D12CommandBuffer,
        groupcount_x: u32,
        groupcount_y: u32,
        groupcount_z: u32,
    ) {
        self.bind_compute_resources(cb);
        cb.graphics_command_list
            .dispatch(groupcount_x, groupcount_y, groupcount_z);
    }

    /// Translation of `D3D12_DispatchComputeIndirect()`.
    pub(super) fn dispatch_compute_indirect_internal(
        &self,
        cb: &mut D3D12CommandBuffer,
        container: &BufferContainer,
        offset: u32,
    ) {
        let buffer = lock(&container.state).active_buffer.clone();

        self.bind_compute_resources(cb);
        cb.graphics_command_list.execute_indirect(
            &self.indirect_dispatch_command_signature,
            1,
            &buffer.handle,
            offset as u64,
        );

        cb.track_buffer(&buffer);
    }

    /// Translation of `D3D12_EndComputePass()`.
    pub(super) fn end_compute_pass_internal(cb: &mut D3D12CommandBuffer) {
        for i in 0..cb.compute_read_write_storage_texture_subresource_count as usize {
            if let Some(subresource) = cb.compute_read_write_storage_texture_subresources[i].take()
            {
                cb.texture_subresource_transition_to_default_usage(
                    D3D12_RESOURCE_STATE_UNORDERED_ACCESS,
                    &subresource,
                );
            }
        }
        cb.compute_read_write_storage_texture_subresource_count = 0;

        for i in 0..cb.compute_read_write_storage_buffer_count as usize {
            if let Some(buffer) = cb.compute_read_write_storage_buffers[i].take() {
                cb.buffer_transition_to_default_usage(
                    D3D12_RESOURCE_STATE_UNORDERED_ACCESS,
                    &buffer,
                );
            }
        }
        cb.compute_read_write_storage_buffer_count = 0;

        for i in 0..MAX_STORAGE_TEXTURES_PER_STAGE as usize {
            if let Some(texture) = cb.compute_read_only_storage_textures[i].take() {
                cb.texture_transition_to_default_usage(
                    D3D12_RESOURCE_STATE_NON_PIXEL_SHADER_RESOURCE,
                    &texture,
                );
            }
        }

        for i in 0..MAX_STORAGE_BUFFERS_PER_STAGE as usize {
            if let Some(buffer) = cb.compute_read_only_storage_buffers[i].take() {
                cb.buffer_transition_to_default_usage(
                    D3D12_RESOURCE_STATE_NON_PIXEL_SHADER_RESOURCE,
                    &buffer,
                );
            }
        }

        cb.compute_sampler_texture_descriptor_handles = Default::default();
        cb.compute_sampler_descriptor_handles = Default::default();

        cb.compute_read_write_storage_texture_descriptor_handles = Default::default();
        cb.compute_read_write_storage_buffer_descriptor_handles = Default::default();

        cb.current_compute_pipeline = None;
    }

    // Copy Pass

    /// Translation of `D3D12_UploadToTexture()`.
    ///
    /// Note (upstream): C reads past the end of the transfer buffer when
    /// the region doesn't fit in it; the rows that don't fit are skipped
    /// here.
    pub(super) fn upload_to_texture_internal(
        &self,
        cb: &mut D3D12CommandBuffer,
        source: &TextureTransferInfo<'_>,
        destination: &TextureRegion<'_>,
        cycle: bool,
    ) {
        let transfer_buffer_container = object::<BufferContainer>(&source.transfer_buffer.raw);
        let transfer_buffer = lock(&transfer_buffer_container.state).active_buffer.clone();
        let mut pixels_per_row = source.pixels_per_row;
        let mut rows_per_slice = source.rows_per_layer;

        // Note that the transfer buffer does not need a barrier, as it is synced by the client.

        let texture_container = object::<TextureContainer>(&destination.texture.raw);
        let texture_subresource = self.prepare_texture_subresource_for_write(
            cb,
            &texture_container,
            destination.layer,
            destination.mip_level,
            cycle,
            D3D12_RESOURCE_STATE_COPY_DEST,
        );

        /* Unless the UnrestrictedBufferTextureCopyPitchSupported feature is supported, D3D12 requires
         * texture data row pitch to be 256 byte aligned, which is obviously insane. Instead of exposing
         * that restriction to the client, which is a huge rake to step on, and a restriction that no
         * other backend requires, we're going to copy data to a temporary buffer, copy THAT data to the
         * texture, and then get rid of the temporary buffer ASAP. If we're lucky and the row pitch and
         * depth pitch are already aligned, we can skip all of that.
         *
         * D3D12 also requires offsets to be 512 byte aligned. We'll fix that for the client and warn them as well.
         *
         * And just for some extra fun, D3D12 doesn't actually support depth pitch, so we have to realign that too!
         */

        if pixels_per_row == 0 {
            pixels_per_row = destination.w;
        }

        if rows_per_slice == 0 {
            rows_per_slice = destination.h;
        }

        let format = texture_container.info.format;
        let block_width = format.block_width() as u32;
        let block_size = format.texel_block_size();
        let row_pitch = pixels_per_row.div_ceil(block_width) * block_size;
        let block_height = rows_per_slice.div_ceil(block_width);

        let bytes_per_slice = rows_per_slice * row_pitch;

        let mut aligned_row_pitch;
        let mut needs_realignment;
        let needs_placement_copy;
        if self.unrestricted_buffer_texture_copy_pitch_supported {
            aligned_row_pitch = row_pitch;
            needs_realignment = false;
            needs_placement_copy = false;
        } else {
            aligned_row_pitch = destination.w.div_ceil(block_width) * block_size;
            aligned_row_pitch = align(aligned_row_pitch, D3D12_TEXTURE_DATA_PITCH_ALIGNMENT);
            needs_realignment = rows_per_slice != destination.h || row_pitch != aligned_row_pitch;
            needs_placement_copy = !source
                .offset
                .is_multiple_of(D3D12_TEXTURE_DATA_PLACEMENT_ALIGNMENT);
        }

        let mut aligned_bytes_per_slice = aligned_row_pitch * block_height;
        if !self.unrestricted_buffer_texture_copy_pitch_supported
            && destination.d > 1
            && !aligned_bytes_per_slice.is_multiple_of(D3D12_TEXTURE_DATA_PLACEMENT_ALIGNMENT)
        {
            needs_realignment = true;
            aligned_bytes_per_slice = align(
                aligned_bytes_per_slice,
                D3D12_TEXTURE_DATA_PLACEMENT_ALIGNMENT,
            );
        }

        let mut source_location = TextureCopyLocation {
            ty: D3D12_TEXTURE_COPY_TYPE_PLACED_FOOTPRINT,
            ..Default::default()
        };
        let mut footprint = PlacedSubresourceFootprint {
            offset: 0,
            footprint: SubresourceFootprint {
                format: sdl_to_d3d12_texture_format(format),
                row_pitch: aligned_row_pitch,
                ..Default::default()
            },
        };

        let destination_location = TextureCopyLocation {
            resource: texture_subresource.texture.resource.as_ptr().cast(),
            ty: D3D12_TEXTURE_COPY_TYPE_SUBRESOURCE_INDEX,
            u: TextureCopyLocationUnion {
                subresource_index: texture_subresource.get().index,
            },
        };

        let transfer_map = transfer_buffer.map_pointer.load(Ordering::SeqCst);
        let transfer_size = transfer_buffer.size as usize;

        if needs_realignment || needs_placement_copy {
            let size = if needs_realignment {
                aligned_bytes_per_slice * destination.d
            } else {
                aligned_row_pitch * block_height * destination.d
            };
            let Ok(temporary_buffer) = self.create_buffer_internal(
                crate::gpu::BufferUsageFlags::default(),
                size,
                D3D12BufferType::Upload,
                None,
            ) else {
                return;
            };
            let temporary_map = temporary_buffer.map_pointer.load(Ordering::SeqCst);
            let copy = |dst: usize, src: usize, len: usize| {
                if transfer_map.is_null()
                    || temporary_map.is_null()
                    || dst + len > size as usize
                    || src + len > transfer_size
                {
                    return;
                }
                // SAFETY: both buffers are persistently mapped upload
                // buffers of their sizes, and the range is in both
                // (checked above).
                unsafe {
                    std::ptr::copy_nonoverlapping(
                        transfer_map.add(src),
                        temporary_map.add(dst),
                        len,
                    )
                };
            };

            source_location.resource = temporary_buffer.handle.as_ptr().cast();

            if needs_realignment {
                for slice_index in 0..destination.d {
                    for row_index in 0..block_height {
                        copy(
                            (slice_index * aligned_bytes_per_slice + row_index * aligned_row_pitch)
                                as usize,
                            source.offset as usize
                                + (slice_index * bytes_per_slice + row_index * row_pitch) as usize,
                            row_pitch as usize,
                        );
                    }

                    footprint.footprint.width = destination.w;
                    footprint.footprint.height = destination.h;
                    footprint.footprint.depth = 1;
                    footprint.offset = (slice_index * aligned_bytes_per_slice) as u64;
                    source_location.u.placed_footprint = footprint;

                    cb.graphics_command_list.copy_texture_region(
                        &destination_location,
                        destination.x,
                        destination.y,
                        destination.z + slice_index,
                        &source_location,
                        None,
                    );
                }
            } else {
                copy(
                    0,
                    source.offset as usize,
                    (aligned_row_pitch * block_height * destination.d) as usize,
                );

                footprint.offset = 0;
                footprint.footprint.width = destination.w;
                footprint.footprint.height = destination.h;
                footprint.footprint.depth = destination.d;
                source_location.u.placed_footprint = footprint;

                cb.graphics_command_list.copy_texture_region(
                    &destination_location,
                    destination.x,
                    destination.y,
                    destination.z,
                    &source_location,
                    None,
                );
            }

            cb.track_buffer(&temporary_buffer);
            Self::release_buffer_internal(&mut lock(&self.dispose), &temporary_buffer);

            if self.debug_mode {
                if needs_realignment {
                    crate::log::warn!(
                        Category::Gpu,
                        "Texture upload row pitch not aligned to 256 bytes! This is suboptimal on D3D12!"
                    );
                } else {
                    crate::log::warn!(
                        Category::Gpu,
                        "Texture upload offset not aligned to 512 bytes! This is suboptimal on D3D12!"
                    );
                }
            }
        } else {
            source_location.resource = transfer_buffer.handle.as_ptr().cast();
            footprint.offset = source.offset as u64;
            footprint.footprint.width = destination.w;
            footprint.footprint.height = destination.h;
            footprint.footprint.depth = destination.d;
            source_location.u.placed_footprint = footprint;

            cb.graphics_command_list.copy_texture_region(
                &destination_location,
                destination.x,
                destination.y,
                destination.z,
                &source_location,
                None,
            );
        }

        cb.texture_subresource_transition_to_default_usage(
            D3D12_RESOURCE_STATE_COPY_DEST,
            &texture_subresource,
        );

        cb.track_buffer(&transfer_buffer);
        cb.track_texture(&texture_subresource.texture);
    }

    /// Translation of `D3D12_UploadToBuffer()`.
    pub(super) fn upload_to_buffer_internal(
        &self,
        cb: &mut D3D12CommandBuffer,
        source: &TransferBufferLocation<'_>,
        destination: &BufferRegion<'_>,
        cycle: bool,
    ) {
        let transfer_buffer_container = object::<BufferContainer>(&source.transfer_buffer.raw);
        let transfer_buffer = lock(&transfer_buffer_container.state).active_buffer.clone();
        let buffer_container = object::<BufferContainer>(&destination.buffer.raw);

        // The transfer buffer does not need a barrier, it is synced by the client.

        let buffer = self.prepare_buffer_for_write(
            cb,
            &buffer_container,
            cycle,
            D3D12_RESOURCE_STATE_COPY_DEST,
        );

        cb.graphics_command_list.copy_buffer_region(
            &buffer.handle,
            destination.offset as u64,
            &transfer_buffer.handle,
            source.offset as u64,
            destination.size as u64,
        );

        cb.buffer_transition_to_default_usage(D3D12_RESOURCE_STATE_COPY_DEST, &buffer);

        cb.track_buffer(&transfer_buffer);
        cb.track_buffer(&buffer);
    }

    /// Translation of `D3D12_CopyTextureToTexture()`.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn copy_texture_to_texture_internal(
        &self,
        cb: &mut D3D12CommandBuffer,
        source: &TextureLocation<'_>,
        destination: &TextureLocation<'_>,
        w: u32,
        h: u32,
        d: u32,
        cycle: bool,
    ) {
        let source_container = object::<TextureContainer>(&source.texture.raw);
        let source_subresource =
            Self::fetch_texture_subresource(&source_container, source.layer, source.mip_level);

        let destination_container = object::<TextureContainer>(&destination.texture.raw);
        let destination_subresource = self.prepare_texture_subresource_for_write(
            cb,
            &destination_container,
            destination.layer,
            destination.mip_level,
            cycle,
            D3D12_RESOURCE_STATE_COPY_DEST,
        );

        cb.texture_subresource_transition_from_default_usage(
            D3D12_RESOURCE_STATE_COPY_SOURCE,
            &source_subresource,
        );

        let source_location = TextureCopyLocation {
            ty: D3D12_TEXTURE_COPY_TYPE_SUBRESOURCE_INDEX,
            u: TextureCopyLocationUnion {
                subresource_index: source_subresource.get().index,
            },
            resource: source_subresource.texture.resource.as_ptr().cast(),
        };

        let destination_location = TextureCopyLocation {
            ty: D3D12_TEXTURE_COPY_TYPE_SUBRESOURCE_INDEX,
            u: TextureCopyLocationUnion {
                subresource_index: destination_subresource.get().index,
            },
            resource: destination_subresource.texture.resource.as_ptr().cast(),
        };

        let source_box = D3d12Box {
            left: source.x,
            top: source.y,
            front: source.z,
            right: source.x + w,
            bottom: source.y + h,
            back: source.z + d,
        };

        cb.graphics_command_list.copy_texture_region(
            &destination_location,
            destination.x,
            destination.y,
            destination.z,
            &source_location,
            Some(&source_box),
        );

        cb.texture_subresource_transition_to_default_usage(
            D3D12_RESOURCE_STATE_COPY_SOURCE,
            &source_subresource,
        );

        cb.texture_subresource_transition_to_default_usage(
            D3D12_RESOURCE_STATE_COPY_DEST,
            &destination_subresource,
        );

        cb.track_texture(&source_subresource.texture);

        cb.track_texture(&destination_subresource.texture);
    }

    /// Translation of `D3D12_CopyBufferToBuffer()`.
    pub(super) fn copy_buffer_to_buffer_internal(
        &self,
        cb: &mut D3D12CommandBuffer,
        source: &BufferLocation<'_>,
        destination: &BufferLocation<'_>,
        size: u32,
        cycle: bool,
    ) {
        let source_container = object::<BufferContainer>(&source.buffer.raw);
        let destination_container = object::<BufferContainer>(&destination.buffer.raw);

        let source_buffer = lock(&source_container.state).active_buffer.clone();
        let destination_buffer = self.prepare_buffer_for_write(
            cb,
            &destination_container,
            cycle,
            D3D12_RESOURCE_STATE_COPY_DEST,
        );

        cb.buffer_transition_from_default_usage(D3D12_RESOURCE_STATE_COPY_SOURCE, &source_buffer);

        cb.graphics_command_list.copy_buffer_region(
            &destination_buffer.handle,
            destination.offset as u64,
            &source_buffer.handle,
            source.offset as u64,
            size as u64,
        );

        cb.buffer_transition_to_default_usage(D3D12_RESOURCE_STATE_COPY_SOURCE, &source_buffer);

        cb.buffer_transition_to_default_usage(D3D12_RESOURCE_STATE_COPY_DEST, &destination_buffer);

        cb.track_buffer(&source_buffer);
        cb.track_buffer(&destination_buffer);
    }

    /// Translation of `D3D12_DownloadFromTexture()`.
    pub(super) fn download_from_texture_internal(
        &self,
        cb: &mut D3D12CommandBuffer,
        source: &TextureRegion<'_>,
        destination: &TextureTransferInfo<'_>,
    ) {
        let mut pixels_per_row = destination.pixels_per_row;
        let mut rows_per_slice = destination.rows_per_layer;
        let source_container = object::<TextureContainer>(&source.texture.raw);
        let source_subresource =
            Self::fetch_texture_subresource(&source_container, source.layer, source.mip_level);
        let destination_container = object::<BufferContainer>(&destination.transfer_buffer.raw);
        let destination_buffer = lock(&destination_container.state).active_buffer.clone();

        /* Unless the UnrestrictedBufferTextureCopyPitchSupported feature is supported, D3D12 requires
         * texture data row pitch to be 256 byte aligned, which is obviously insane. Instead of exposing
         * that restriction to the client, which is a huge rake to step on, and a restriction that no
         * other backend requires, we're going to copy data to a temporary buffer, copy THAT data to the
         * texture, and then get rid of the temporary buffer ASAP. If we're lucky and the row pitch and
         * depth pitch are already aligned, we can skip all of that.
         *
         * D3D12 also requires offsets to be 512 byte aligned. We'll fix that for the client and warn them as well.
         *
         * And just for some extra fun, D3D12 doesn't actually support depth pitch, so we have to realign that too!
         *
         * Since this is an async download we have to do all these fixups after the command is finished,
         * so we'll cache the metadata and map and copy it when the command buffer is cleaned.
         */

        if pixels_per_row == 0 {
            pixels_per_row = source.w;
        }

        let format = source_container.info.format;
        let row_pitch = format.bytes_per_row(pixels_per_row as i32);

        if rows_per_slice == 0 {
            rows_per_slice = source.h;
        }

        let aligned_row_pitch;
        let needs_realignment;
        let needs_placement_copy;
        if self.unrestricted_buffer_texture_copy_pitch_supported {
            aligned_row_pitch = row_pitch;
            needs_realignment = false;
            needs_placement_copy = false;
        } else {
            aligned_row_pitch = align(row_pitch, D3D12_TEXTURE_DATA_PITCH_ALIGNMENT);
            needs_realignment = rows_per_slice != source.h || row_pitch != aligned_row_pitch;
            needs_placement_copy = !destination
                .offset
                .is_multiple_of(D3D12_TEXTURE_DATA_PLACEMENT_ALIGNMENT);
        }

        let source_location = TextureCopyLocation {
            ty: D3D12_TEXTURE_COPY_TYPE_SUBRESOURCE_INDEX,
            u: TextureCopyLocationUnion {
                subresource_index: source_subresource.get().index,
            },
            resource: source_subresource.texture.resource.as_ptr().cast(),
        };

        let source_box = D3d12Box {
            left: source.x,
            top: source.y,
            front: source.z,
            right: source.x + source.w,
            bottom: source.y + rows_per_slice,
            back: source.z + source.d,
        };

        let mut footprint = PlacedSubresourceFootprint {
            offset: 0,
            footprint: SubresourceFootprint {
                format: sdl_to_d3d12_texture_format(format),
                width: source.w,
                height: rows_per_slice,
                depth: source.d,
                row_pitch: aligned_row_pitch,
            },
        };

        let mut texture_download = None;
        let destination_resource;
        if needs_realignment || needs_placement_copy {
            let Ok(temporary_buffer) = self.create_buffer_internal(
                crate::gpu::BufferUsageFlags::default(),
                aligned_row_pitch * rows_per_slice * source.d,
                D3D12BufferType::Download,
                None,
            ) else {
                return;
            };

            destination_resource = temporary_buffer.handle.as_ptr().cast();
            footprint.offset = 0;
            texture_download = Some(TextureDownload {
                temporary_buffer,
                destination_buffer: destination_buffer.clone(),
                buffer_offset: destination.offset,
                width: source.w,
                height: rows_per_slice,
                depth: source.d,
                bytes_per_row: row_pitch,
                bytes_per_depth_slice: row_pitch * rows_per_slice,
                aligned_bytes_per_row: aligned_row_pitch,
            });

            if self.debug_mode {
                crate::log::warn!(
                    Category::Gpu,
                    "Texture pitch or offset not aligned properly! This is suboptimal on D3D12!"
                );
            }
        } else {
            destination_resource = destination_buffer.handle.as_ptr().cast();
            footprint.offset = destination.offset as u64;
        }

        let destination_location = TextureCopyLocation {
            resource: destination_resource,
            ty: D3D12_TEXTURE_COPY_TYPE_PLACED_FOOTPRINT,
            u: TextureCopyLocationUnion {
                placed_footprint: footprint,
            },
        };

        cb.texture_subresource_transition_from_default_usage(
            D3D12_RESOURCE_STATE_COPY_SOURCE,
            &source_subresource,
        );

        cb.graphics_command_list.copy_texture_region(
            &destination_location,
            0,
            0,
            0,
            &source_location,
            Some(&source_box),
        );

        cb.texture_subresource_transition_to_default_usage(
            D3D12_RESOURCE_STATE_COPY_SOURCE,
            &source_subresource,
        );

        cb.track_buffer(&destination_buffer);
        cb.track_texture(&source_subresource.texture);

        if let Some(texture_download) = texture_download {
            let temporary_buffer = texture_download.temporary_buffer.clone();
            cb.track_buffer(&temporary_buffer);

            cb.texture_downloads.push(texture_download);

            Self::release_buffer_internal(&mut lock(&self.dispose), &temporary_buffer);
        }
    }

    /// Translation of `D3D12_DownloadFromBuffer()`.
    pub(super) fn download_from_buffer_internal(
        &self,
        cb: &mut D3D12CommandBuffer,
        source: &BufferRegion<'_>,
        destination: &TransferBufferLocation<'_>,
    ) {
        let source_container = object::<BufferContainer>(&source.buffer.raw);
        let destination_container = object::<BufferContainer>(&destination.transfer_buffer.raw);

        let source_buffer = lock(&source_container.state).active_buffer.clone();
        cb.buffer_transition_from_default_usage(D3D12_RESOURCE_STATE_COPY_SOURCE, &source_buffer);

        let destination_buffer = lock(&destination_container.state).active_buffer.clone();

        cb.graphics_command_list.copy_buffer_region(
            &destination_buffer.handle,
            destination.offset as u64,
            &source_buffer.handle,
            source.offset as u64,
            source.size as u64,
        );

        cb.buffer_transition_to_default_usage(D3D12_RESOURCE_STATE_COPY_SOURCE, &source_buffer);

        cb.track_buffer(&source_buffer);
        cb.track_buffer(&destination_buffer);
    }

    // Blits

    /// Translation of `D3D12_GenerateMipmaps()`: a blit per level of every
    /// layer, through the front end (`SDL_BlitGPUTexture()`).
    pub(super) fn generate_mipmaps_internal(
        &self,
        command_buffer: &mut CommandBuffer,
        texture: &Texture,
    ) {
        let container = object::<TextureContainer>(&texture.raw);
        let header = &container.info;

        let fetched = {
            let mut blit = lock(&self.blit);
            let (shaders, _, _, pipelines) = blit.parts();
            match shaders {
                Some(shaders) => fetch_blit_pipeline(
                    &command_buffer.device,
                    header.texture_type,
                    header.format,
                    &shaders,
                    pipelines,
                )
                .is_ok(),
                None => false,
            }
        };

        if !fetched {
            crate::log::warn!(Category::Gpu, "Could not fetch blit pipeline");
            return;
        }

        // We have to do this one subresource at a time
        for layer_or_depth_index in 0..header.layer_count_or_depth {
            for level_index in 1..header.num_levels {
                let blit_info = BlitInfo {
                    source: BlitRegion {
                        texture,
                        mip_level: level_index - 1,
                        layer_or_depth_plane: layer_or_depth_index,
                        x: 0,
                        y: 0,
                        w: mip_size(header.width, level_index - 1),
                        h: mip_size(header.height, level_index - 1),
                    },
                    destination: BlitRegion {
                        texture,
                        mip_level: level_index,
                        layer_or_depth_plane: layer_or_depth_index,
                        x: 0,
                        y: 0,
                        w: mip_size(header.width, level_index),
                        h: mip_size(header.height, level_index),
                    },
                    load_op: LoadOp::DontCare,
                    clear_color: FColor::default(),
                    flip_mode: FlipMode::None,
                    filter: Filter::Linear,
                    cycle: false,
                };

                let _ = command_buffer.blit_texture(&blit_info);
            }
        }

        let active_texture = lock(&container.state).active_texture.clone();
        if let Some(cb) = command_buffer.backend_mut::<D3D12CommandBuffer>() {
            cb.track_texture(&active_texture);
        }
    }

    /// Translation of `D3D12_Blit()`.
    pub(super) fn blit_internal(&self, command_buffer: &mut CommandBuffer, info: &BlitInfo<'_>) {
        let mut blit = lock(&self.blit);
        let (shaders, linear, nearest, pipelines) = blit.parts();
        // (upstream goes on with the NULL shaders or samplers that failed
        // to be made, and the blit fails)
        let (Some(shaders), Some(linear), Some(nearest)) = (shaders, linear, nearest) else {
            crate::log::error!(Category::Gpu, "Failed to create GPU pipeline for blit");
            return;
        };

        let _ = blit_common(command_buffer, info, linear, nearest, &shaders, pipelines);
    }
}

impl super::BlitResources {
    /// The blit shaders (when they were all made), the linear and nearest
    /// samplers, and the pipelines.
    pub(super) fn parts(
        &mut self,
    ) -> (
        Option<BlitShaders<'_>>,
        Option<&Sampler>,
        Option<&Sampler>,
        &mut BlitPipelineCache,
    ) {
        let super::BlitResources {
            vertex_shader,
            from_2d_shader,
            from_2d_array_shader,
            from_3d_shader,
            from_cube_shader,
            from_cube_array_shader,
            nearest_sampler,
            linear_sampler,
            pipelines,
        } = self;
        let shaders = (|| {
            Some(BlitShaders {
                vertex: vertex_shader.as_ref()?,
                from_2d: from_2d_shader.as_ref()?,
                from_2d_array: from_2d_array_shader.as_ref()?,
                from_3d: from_3d_shader.as_ref()?,
                from_cube: from_cube_shader.as_ref()?,
                from_cube_array: from_cube_array_shader.as_ref()?,
            })
        })();
        (
            shaders,
            linear_sampler.as_ref(),
            nearest_sampler.as_ref(),
            pipelines,
        )
    }
}
