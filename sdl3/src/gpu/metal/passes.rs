// Rust translation of the copy, render and compute pass parts of
// src/gpu/metal/SDL_gpu_metal.m from Simple DirectMedia Layer.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! Copy passes (uploads, downloads, copies and mipmap generation on a blit
//! encoder), render passes (targets, dynamic state, bindings, draws,
//! indirect ones too), compute passes and dispatches, and blits through
//! the front end's blit pipelines (`SDL_GPU_BlitCommon()`).
//!
//! A command buffer's slots hold retained handles, as upstream's arrays of
//! strong `id`s do; the bindings are made from them when a draw or
//! dispatch needs them.

use std::sync::Arc;

use super::commands::{ids, MetalCommandBuffer, UniformStage, UNIFORM_BUFFERS};
use super::pipelines::{MetalComputePipeline, MetalGraphicsPipeline};
use super::resources::{BufferContainer, MetalSampler, TextureContainer};
use super::sys::*;
use super::tables::*;
use super::{lock, object, MetalRenderer, METAL_FIRST_VERTEX_BUFFER_SLOT};
use crate::core::apple::foundation::NSRange;
use crate::core::apple::objc::{class, msg_send, Id, NSInteger, NSUInteger, StrongPtr};
use crate::gpu::sysgpu::{blit_common, index_size, MAX_STORAGE_BUFFERS_PER_STAGE};
use crate::gpu::{
    BlitInfo, Buffer, BufferBinding, BufferLocation, BufferRegion, ColorTargetInfo, CommandBuffer,
    DepthStencilTargetInfo, IndexElementSize, IndexedIndirectDrawCommand, IndirectDrawCommand,
    StorageBufferReadWriteBinding, StorageTextureReadWriteBinding, StoreOp, Texture,
    TextureLocation, TextureRegion, TextureSamplerBinding, TextureTransferInfo, TextureType,
    TransferBufferLocation, Viewport,
};
use crate::log::Category;
use crate::video::{FColor, Rect};

/// The stage graphics resources are bound to.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum GraphicsStage {
    Vertex,
    Fragment,
}

/// `NSMakeRange()` of slot indices.
fn range(location: u32, length: u32) -> NSRange {
    NSRange::new(location as NSUInteger, length as NSUInteger)
}

impl MetalRenderer {
    // Copy Pass

    /// Translation of `METAL_BeginCopyPass()`.
    pub(super) fn begin_copy_pass_internal(command_buffer: &mut MetalCommandBuffer) {
        // SAFETY: an autoreleased blit encoder of the command buffer,
        // retained.
        command_buffer.blit_encoder =
            unsafe { StrongPtr::retain(msg_send![command_buffer.handle, blitCommandEncoder]) };
    }

    /// Translation of `METAL_UploadToTexture()`.
    pub(super) fn upload_to_texture_internal(
        &self,
        command_buffer: &mut MetalCommandBuffer,
        source: &TextureTransferInfo<'_>,
        destination: &TextureRegion<'_>,
        cycle: bool,
    ) {
        let buffer_container = object::<BufferContainer>(&source.transfer_buffer.raw);
        let texture_container = object::<TextureContainer>(&destination.texture.raw);

        let metal_texture = self.prepare_texture_for_write(&texture_container, cycle);
        let active_buffer = buffer_container.active();
        let format = texture_container.info.format;

        // SAFETY: a blit encoder copying between a buffer and a texture.
        unsafe {
            let _: () = msg_send![command_buffer.blit_encoder,
                     copyFromBuffer: active_buffer.handle.get()
                       sourceOffset: source.offset as NSUInteger
                  sourceBytesPerRow: format.bytes_per_row(destination.w as i32) as NSUInteger
                // sourceBytesPerImage expects the stride between 2D images (slices) of a 3D texture, not the size of the entire region
                sourceBytesPerImage: format.calculate_size(destination.w, destination.h, 1) as NSUInteger
                         sourceSize: mtl_size_make(destination.w, destination.h, destination.d)
                          toTexture: metal_texture.handle.get()
                   destinationSlice: destination.layer as NSUInteger
                   destinationLevel: destination.mip_level as NSUInteger
                  destinationOrigin: mtl_origin_make(destination.x, destination.y, destination.z)];
        }

        command_buffer.track_texture(&metal_texture);
        command_buffer.track_buffer(&active_buffer);
    }

    /// Translation of `METAL_UploadToBuffer()`.
    pub(super) fn upload_to_buffer_internal(
        &self,
        command_buffer: &mut MetalCommandBuffer,
        source: &TransferBufferLocation<'_>,
        destination: &BufferRegion<'_>,
        cycle: bool,
    ) {
        let transfer_container = object::<BufferContainer>(&source.transfer_buffer.raw);
        let buffer_container = object::<BufferContainer>(&destination.buffer.raw);

        let metal_buffer = self.prepare_buffer_for_write(&buffer_container, cycle);
        let transfer_buffer = transfer_container.active();

        // SAFETY: a blit encoder copying between buffers.
        unsafe {
            let _: () = msg_send![command_buffer.blit_encoder,
                   copyFromBuffer: transfer_buffer.handle.get()
                     sourceOffset: source.offset as NSUInteger
                         toBuffer: metal_buffer.handle.get()
                destinationOffset: destination.offset as NSUInteger
                             size: destination.size as NSUInteger];
        }

        command_buffer.track_buffer(&metal_buffer);
        command_buffer.track_buffer(&transfer_buffer);
    }

    /// Translation of `METAL_CopyTextureToTexture()`.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn copy_texture_to_texture_internal(
        &self,
        command_buffer: &mut MetalCommandBuffer,
        source: &TextureLocation<'_>,
        destination: &TextureLocation<'_>,
        w: u32,
        h: u32,
        d: u32,
        cycle: bool,
    ) {
        let src_container = object::<TextureContainer>(&source.texture.raw);
        let dst_container = object::<TextureContainer>(&destination.texture.raw);

        let src_texture = src_container.active();
        let dst_texture = self.prepare_texture_for_write(&dst_container, cycle);

        // SAFETY: a blit encoder copying between textures.
        unsafe {
            let _: () = msg_send![command_buffer.blit_encoder,
                  copyFromTexture: src_texture.handle.get()
                      sourceSlice: source.layer as NSUInteger
                      sourceLevel: source.mip_level as NSUInteger
                     sourceOrigin: mtl_origin_make(source.x, source.y, source.z)
                       sourceSize: mtl_size_make(w, h, d)
                        toTexture: dst_texture.handle.get()
                 destinationSlice: destination.layer as NSUInteger
                 destinationLevel: destination.mip_level as NSUInteger
                destinationOrigin: mtl_origin_make(destination.x, destination.y, destination.z)];
        }

        command_buffer.track_texture(&src_texture);
        command_buffer.track_texture(&dst_texture);
    }

    /// Translation of `METAL_CopyBufferToBuffer()`.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn copy_buffer_to_buffer_internal(
        &self,
        command_buffer: &mut MetalCommandBuffer,
        source: &Buffer,
        source_offset: u32,
        destination: &BackendBufferRef<'_>,
        destination_offset: u32,
        size: u32,
        cycle: bool,
    ) {
        let src_container = object::<BufferContainer>(&source.raw);
        let dst_container = destination.container();

        let src_buffer = src_container.active();
        let dst_buffer = self.prepare_buffer_for_write(&dst_container, cycle);

        // SAFETY: a blit encoder copying between buffers.
        unsafe {
            let _: () = msg_send![command_buffer.blit_encoder,
                   copyFromBuffer: src_buffer.handle.get()
                     sourceOffset: source_offset as NSUInteger
                         toBuffer: dst_buffer.handle.get()
                destinationOffset: destination_offset as NSUInteger
                             size: size as NSUInteger];
        }

        command_buffer.track_buffer(&src_buffer);
        command_buffer.track_buffer(&dst_buffer);
    }

    /// Translation of `METAL_CopyBufferToBuffer()` with buffer locations.
    pub(super) fn copy_buffer_location_internal(
        &self,
        command_buffer: &mut MetalCommandBuffer,
        source: &BufferLocation<'_>,
        destination: &BufferLocation<'_>,
        size: u32,
        cycle: bool,
    ) {
        self.copy_buffer_to_buffer_internal(
            command_buffer,
            source.buffer,
            source.offset,
            &BackendBufferRef::Buffer(destination.buffer),
            destination.offset,
            size,
            cycle,
        );
    }

    /// Translation of `METAL_DownloadFromTexture()`.
    pub(super) fn download_from_texture_internal(
        &self,
        command_buffer: &mut MetalCommandBuffer,
        source: &TextureRegion<'_>,
        destination: &TextureTransferInfo<'_>,
    ) {
        let texture_container = object::<TextureContainer>(&source.texture.raw);
        let metal_texture = texture_container.active();
        let buffer_container = object::<BufferContainer>(&destination.transfer_buffer.raw);
        let mut buffer_stride = destination.pixels_per_row;
        let mut buffer_image_height = destination.rows_per_layer;

        let dst_buffer = self.prepare_buffer_for_write(&buffer_container, false);

        let region_origin = mtl_origin_make(source.x, source.y, source.z);

        let region_size = mtl_size_make(source.w, source.h, source.d);

        if buffer_stride == 0 || buffer_image_height == 0 {
            buffer_stride = source.w;
            buffer_image_height = source.h;
        }

        let bytes_per_row = texture_container
            .info
            .format
            .bytes_per_row(buffer_stride as i32);
        let bytes_per_depth_slice = bytes_per_row.wrapping_mul(buffer_image_height);

        // SAFETY: a blit encoder copying from a texture to a buffer.
        unsafe {
            let _: () = msg_send![command_buffer.blit_encoder,
                         copyFromTexture: metal_texture.handle.get()
                             sourceSlice: source.layer as NSUInteger
                             sourceLevel: source.mip_level as NSUInteger
                            sourceOrigin: region_origin
                              sourceSize: region_size
                                toBuffer: dst_buffer.handle.get()
                       destinationOffset: destination.offset as NSUInteger
                  destinationBytesPerRow: bytes_per_row as NSUInteger
                destinationBytesPerImage: bytes_per_depth_slice as NSUInteger];
        }

        command_buffer.track_texture(&metal_texture);
        command_buffer.track_buffer(&dst_buffer);
    }

    /// Translation of `METAL_DownloadFromBuffer()`.
    pub(super) fn download_from_buffer_internal(
        &self,
        command_buffer: &mut MetalCommandBuffer,
        source: &BufferRegion<'_>,
        destination: &TransferBufferLocation<'_>,
    ) {
        // (upstream passes the transfer buffer location as a buffer
        // location: both start with the object and the offset)
        self.copy_buffer_to_buffer_internal(
            command_buffer,
            source.buffer,
            source.offset,
            &BackendBufferRef::Transfer(destination.transfer_buffer),
            destination.offset,
            source.size,
            false,
        );
    }

    /// Translation of `METAL_EndCopyPass()`.
    pub(super) fn end_copy_pass_internal(command_buffer: &mut MetalCommandBuffer) {
        // SAFETY: ending the blit encoder.
        let _: () = unsafe { msg_send![command_buffer.blit_encoder, endEncoding] };
        command_buffer.blit_encoder = StrongPtr::nil();
    }

    /// Translation of `METAL_GenerateMipmaps()`.
    pub(super) fn generate_mipmaps_internal(command_buffer: &mut CommandBuffer, texture: &Texture) {
        let container = object::<TextureContainer>(&texture.raw);
        let metal_texture = container.active();
        let Some(metal_command_buffer) = command_buffer.backend_mut::<MetalCommandBuffer>() else {
            return;
        };

        Self::begin_copy_pass_internal(metal_command_buffer);
        // SAFETY: a blit encoder generating a texture's mipmaps.
        let _: () = unsafe {
            msg_send![metal_command_buffer.blit_encoder, generateMipmapsForTexture: metal_texture.handle.get()]
        };
        Self::end_copy_pass_internal(metal_command_buffer);

        metal_command_buffer.track_texture(&metal_texture);
    }

    // Render Pass

    /// Translation of `METAL_SetViewport()`.
    pub(super) fn set_viewport_internal(command_buffer: &MetalCommandBuffer, viewport: &Viewport) {
        let metal_viewport = MTLViewport {
            origin_x: viewport.x as f64,
            origin_y: viewport.y as f64,
            width: viewport.w as f64,
            height: viewport.h as f64,
            znear: viewport.min_depth as f64,
            zfar: viewport.max_depth as f64,
        };

        // SAFETY: a render encoder's state.
        let _: () =
            unsafe { msg_send![command_buffer.render_encoder, setViewport: metal_viewport] };
    }

    /// Translation of `METAL_SetScissor()`.
    pub(super) fn set_scissor_internal(command_buffer: &MetalCommandBuffer, scissor: &Rect) {
        let metal_scissor = MTLScissorRect {
            x: scissor.x as NSUInteger,
            y: scissor.y as NSUInteger,
            width: scissor.w as NSUInteger,
            height: scissor.h as NSUInteger,
        };

        // SAFETY: a render encoder's state.
        let _: () =
            unsafe { msg_send![command_buffer.render_encoder, setScissorRect: metal_scissor] };
    }

    /// Translation of `METAL_SetBlendConstants()`.
    pub(super) fn set_blend_constants_internal(
        command_buffer: &MetalCommandBuffer,
        blend_constants: FColor,
    ) {
        // SAFETY: a render encoder's state.
        let _: () = unsafe {
            msg_send![command_buffer.render_encoder, setBlendColorRed: blend_constants.r
                                                                green: blend_constants.g
                                                                 blue: blend_constants.b
                                                                alpha: blend_constants.a]
        };
    }

    /// Translation of `METAL_SetStencilReference()`.
    pub(super) fn set_stencil_reference_internal(
        command_buffer: &MetalCommandBuffer,
        reference: u8,
    ) {
        // SAFETY: a render encoder's state.
        let _: () = unsafe {
            msg_send![command_buffer.render_encoder, setStencilReferenceValue: reference as u32]
        };
    }

    /// Translation of `METAL_BeginRenderPass()`.
    pub(super) fn begin_render_pass_internal(
        &self,
        command_buffer: &mut MetalCommandBuffer,
        color_target_infos: &[ColorTargetInfo<'_>],
        depth_stencil_target_info: Option<&DepthStencilTargetInfo<'_>>,
    ) {
        let mut vp_width = u32::MAX;
        let mut vp_height = u32::MAX;

        // SAFETY: an autoreleased render pass descriptor configured through
        // its attachments, then a render encoder for it (retained).
        unsafe {
            let pass_descriptor: Id =
                msg_send![class!(MTLRenderPassDescriptor), renderPassDescriptor];
            let color_attachments: Id = msg_send![pass_descriptor, colorAttachments];

            for (i, info) in color_target_infos.iter().enumerate() {
                let container = object::<TextureContainer>(&info.texture.raw);
                let texture = self.prepare_texture_for_write(&container, info.cycle);

                let attachment = super::pipelines::subscript(color_attachments, i as u32);
                let _: () = msg_send![attachment, setTexture: texture.handle.get()];
                let _: () = msg_send![attachment, setLevel: info.mip_level as NSUInteger];
                if container.info.texture_type == TextureType::Texture3D {
                    let _: () = msg_send![attachment, setDepthPlane: info.layer_or_depth_plane as NSUInteger];
                } else {
                    let _: () =
                        msg_send![attachment, setSlice: info.layer_or_depth_plane as NSUInteger];
                }
                let clear_color = MTLClearColor {
                    red: info.clear_color.r as f64,
                    green: info.clear_color.g as f64,
                    blue: info.clear_color.b as f64,
                    alpha: info.clear_color.a as f64,
                };
                let _: () = msg_send![attachment, setClearColor: clear_color];
                let _: () =
                    msg_send![attachment, setLoadAction: sdl_to_metal_load_op(info.load_op)];
                let _: () =
                    msg_send![attachment, setStoreAction: sdl_to_metal_store_op(info.store_op)];

                command_buffer.track_texture(&texture);

                if info.store_op == StoreOp::Resolve || info.store_op == StoreOp::ResolveAndStore {
                    if let Some(resolve) = info.resolve_texture {
                        let resolve_container = object::<TextureContainer>(&resolve.raw);
                        let resolve_texture = self.prepare_texture_for_write(
                            &resolve_container,
                            info.cycle_resolve_texture,
                        );

                        let _: () =
                            msg_send![attachment, setResolveTexture: resolve_texture.handle.get()];
                        let _: () = msg_send![attachment, setResolveSlice: info.resolve_layer as NSUInteger];
                        let _: () = msg_send![attachment, setResolveLevel: info.resolve_mip_level as NSUInteger];

                        command_buffer.track_texture(&resolve_texture);
                    }
                }
            }

            if let Some(ds) = depth_stencil_target_info {
                let container = object::<TextureContainer>(&ds.texture.raw);
                let texture = self.prepare_texture_for_write(&container, ds.cycle);

                let depth_attachment: Id = msg_send![pass_descriptor, depthAttachment];
                let _: () = msg_send![depth_attachment, setTexture: texture.handle.get()];
                let _: () = msg_send![depth_attachment, setLevel: ds.mip_level as NSUInteger];
                let _: () = msg_send![depth_attachment, setSlice: ds.layer as NSUInteger];
                let _: () =
                    msg_send![depth_attachment, setLoadAction: sdl_to_metal_load_op(ds.load_op)];
                let _: () =
                    msg_send![depth_attachment, setStoreAction: sdl_to_metal_store_op(ds.store_op)];
                let _: () = msg_send![depth_attachment, setClearDepth: ds.clear_depth as f64];

                if container.info.format.is_stencil_format() {
                    let stencil_attachment: Id = msg_send![pass_descriptor, stencilAttachment];
                    let _: () = msg_send![stencil_attachment, setTexture: texture.handle.get()];
                    let _: () = msg_send![stencil_attachment, setLoadAction: sdl_to_metal_load_op(ds.stencil_load_op)];
                    let _: () = msg_send![stencil_attachment, setStoreAction: sdl_to_metal_store_op(ds.stencil_store_op)];
                    let _: () =
                        msg_send![stencil_attachment, setClearStencil: ds.clear_stencil as u32];
                }

                command_buffer.track_texture(&texture);
            }

            command_buffer.render_encoder = StrongPtr::retain(
                msg_send![command_buffer.handle, renderCommandEncoderWithDescriptor: pass_descriptor],
            );
        }

        // The viewport cannot be larger than the smallest target.
        for info in color_target_infos {
            let header = &info.texture.info;
            // (a mip level of 32 or more is rejected by the front end)
            let w = header.width.wrapping_shr(info.mip_level);
            let h = header.height.wrapping_shr(info.mip_level);

            if w < vp_width {
                vp_width = w;
            }

            if h < vp_height {
                vp_height = h;
            }
        }

        if let Some(ds) = depth_stencil_target_info {
            let header = &ds.texture.info;
            let w = header.width >> ds.mip_level;
            let h = header.height >> ds.mip_level;

            if w < vp_width {
                vp_width = w;
            }

            if h < vp_height {
                vp_height = h;
            }
        }

        // Set sensible default states
        let viewport = Viewport {
            x: 0.0,
            y: 0.0,
            w: vp_width as f32,
            h: vp_height as f32,
            min_depth: 0.0,
            max_depth: 1.0,
        };
        Self::set_viewport_internal(command_buffer, &viewport);

        let scissor_rect = Rect {
            x: 0,
            y: 0,
            w: vp_width as i32,
            h: vp_height as i32,
        };
        Self::set_scissor_internal(command_buffer, &scissor_rect);

        let blend_constants = FColor {
            r: 1.0,
            g: 1.0,
            b: 1.0,
            a: 1.0,
        };
        Self::set_blend_constants_internal(command_buffer, blend_constants);

        Self::set_stencil_reference_internal(command_buffer, 0);
    }

    /// Translation of `METAL_BindGraphicsPipeline()`.
    pub(super) fn bind_graphics_pipeline_internal(
        &self,
        command_buffer: &mut MetalCommandBuffer,
        pipeline: Arc<MetalGraphicsPipeline>,
    ) {
        let rast = &pipeline.rasterizer_state;

        // SAFETY: a render encoder's pipeline and rasterizer state.
        unsafe {
            let encoder = command_buffer.render_encoder.get();
            let _: () = msg_send![encoder, setRenderPipelineState: pipeline.handle.get()];

            // Apply rasterizer state
            let _: () =
                msg_send![encoder, setTriangleFillMode: sdl_to_metal_polygon_mode(rast.fill_mode)];
            let _: () = msg_send![encoder, setCullMode: sdl_to_metal_cull_mode(rast.cull_mode)];
            let _: () =
                msg_send![encoder, setFrontFacingWinding: sdl_to_metal_front_face(rast.front_face)];
            let _: () = msg_send![encoder, setDepthClipMode: sdl_to_metal_depth_clip_mode(rast.enable_depth_clip)];
            let depth_bias = if rast.enable_depth_bias {
                rast.depth_bias_constant_factor
            } else {
                0.0
            };
            let slope_scale = if rast.enable_depth_bias {
                rast.depth_bias_slope_factor
            } else {
                0.0
            };
            let clamp = if rast.enable_depth_bias {
                rast.depth_bias_clamp
            } else {
                0.0
            };
            let _: () =
                msg_send![encoder, setDepthBias: depth_bias slopeScale: slope_scale clamp: clamp];

            // Apply depth-stencil state
            if !pipeline.depth_stencil_state.is_nil() {
                let _: () =
                    msg_send![encoder, setDepthStencilState: pipeline.depth_stencil_state.get()];
            }
        }

        // Mark that bindings are needed
        command_buffer.need_vertex_sampler_bind = true;
        command_buffer.need_vertex_storage_texture_bind = true;
        command_buffer.need_vertex_storage_buffer_bind = true;
        command_buffer.need_fragment_sampler_bind = true;
        command_buffer.need_fragment_storage_texture_bind = true;
        command_buffer.need_fragment_storage_buffer_bind = true;

        command_buffer.need_vertex_uniform_buffer_bind = [true; UNIFORM_BUFFERS];
        command_buffer.need_fragment_uniform_buffer_bind = [true; UNIFORM_BUFFERS];

        for i in 0..pipeline.header.num_vertex_uniform_buffers as usize {
            if command_buffer.vertex_uniform_buffers[i].is_none() {
                command_buffer.vertex_uniform_buffers[i] =
                    self.acquire_uniform_buffer_from_pool(command_buffer);
            }
        }

        for i in 0..pipeline.header.num_fragment_uniform_buffers as usize {
            if command_buffer.fragment_uniform_buffers[i].is_none() {
                command_buffer.fragment_uniform_buffers[i] =
                    self.acquire_uniform_buffer_from_pool(command_buffer);
            }
        }

        command_buffer.graphics_pipeline = Some(pipeline);
    }

    /// Translation of `METAL_BindVertexBuffers()`.
    pub(super) fn bind_vertex_buffers_internal(
        command_buffer: &mut MetalCommandBuffer,
        first_slot: u32,
        bindings: &[BufferBinding<'_>],
    ) {
        for (i, binding) in bindings.iter().enumerate() {
            let slot = first_slot as usize + i;
            let current_buffer = object::<BufferContainer>(&binding.buffer.raw).active();
            if command_buffer.vertex_buffers[slot].get() != current_buffer.handle.get()
                || command_buffer.vertex_buffer_offsets[slot] != binding.offset
            {
                command_buffer.vertex_buffers[slot] = current_buffer.handle.clone();
                command_buffer.vertex_buffer_offsets[slot] = binding.offset;
                command_buffer.need_vertex_buffer_bind = true;
                command_buffer.track_buffer(&current_buffer);
            }
        }

        command_buffer.vertex_buffer_count = command_buffer
            .vertex_buffer_count
            .max(first_slot + bindings.len() as u32);
    }

    /// Translation of `METAL_BindIndexBuffer()`.
    pub(super) fn bind_index_buffer_internal(
        command_buffer: &mut MetalCommandBuffer,
        binding: &BufferBinding<'_>,
        index_element_size: IndexElementSize,
    ) {
        let index_buffer = object::<BufferContainer>(&binding.buffer.raw).active();
        command_buffer.index_buffer_offset = binding.offset;
        command_buffer.index_element_size = index_element_size;

        command_buffer.track_buffer(&index_buffer);
        command_buffer.index_buffer = Some(index_buffer);
    }

    /// Translation of `METAL_BindVertexSamplers()` and
    /// `METAL_BindFragmentSamplers()`.
    pub(super) fn bind_graphics_samplers(
        command_buffer: &mut MetalCommandBuffer,
        stage: GraphicsStage,
        first_slot: u32,
        texture_sampler_bindings: &[TextureSamplerBinding<'_>],
    ) {
        for (i, binding) in texture_sampler_bindings.iter().enumerate() {
            let slot = first_slot as usize + i;
            let texture_container = object::<TextureContainer>(&binding.texture.raw);
            let sampler = object::<MetalSampler>(&binding.sampler.raw);
            let active_texture = texture_container.active();

            let (samplers, textures, need_bind) = match stage {
                GraphicsStage::Vertex => (
                    &mut command_buffer.vertex_samplers,
                    &mut command_buffer.vertex_textures,
                    &mut command_buffer.need_vertex_sampler_bind,
                ),
                GraphicsStage::Fragment => (
                    &mut command_buffer.fragment_samplers,
                    &mut command_buffer.fragment_textures,
                    &mut command_buffer.need_fragment_sampler_bind,
                ),
            };

            if samplers[slot].get() != sampler.handle.get() {
                samplers[slot] = sampler.handle.clone();
                *need_bind = true;
            }

            if textures[slot].get() != active_texture.handle.get() {
                textures[slot] = active_texture.handle.clone();
                *need_bind = true;

                command_buffer.track_texture(&active_texture);
            }
        }
    }

    /// Translation of `METAL_BindVertexStorageTextures()` and
    /// `METAL_BindFragmentStorageTextures()`.
    pub(super) fn bind_graphics_storage_textures(
        command_buffer: &mut MetalCommandBuffer,
        stage: GraphicsStage,
        first_slot: u32,
        storage_textures: &[&Texture],
    ) {
        for (i, texture) in storage_textures.iter().enumerate() {
            let slot = first_slot as usize + i;
            let active_texture = object::<TextureContainer>(&texture.raw).active();

            let (textures, need_bind) = match stage {
                GraphicsStage::Vertex => (
                    &mut command_buffer.vertex_storage_textures,
                    &mut command_buffer.need_vertex_storage_texture_bind,
                ),
                GraphicsStage::Fragment => (
                    &mut command_buffer.fragment_storage_textures,
                    &mut command_buffer.need_fragment_storage_texture_bind,
                ),
            };

            if textures[slot].get() != active_texture.handle.get() {
                textures[slot] = active_texture.handle.clone();
                *need_bind = true;

                command_buffer.track_texture(&active_texture);
            }
        }
    }

    /// Translation of `METAL_BindVertexStorageBuffers()` and
    /// `METAL_BindFragmentStorageBuffers()`.
    pub(super) fn bind_graphics_storage_buffers(
        command_buffer: &mut MetalCommandBuffer,
        stage: GraphicsStage,
        first_slot: u32,
        storage_buffers: &[&Buffer],
    ) {
        for (i, buffer) in storage_buffers.iter().enumerate() {
            let slot = first_slot as usize + i;
            let active_buffer = object::<BufferContainer>(&buffer.raw).active();

            let (buffers, need_bind) = match stage {
                GraphicsStage::Vertex => (
                    &mut command_buffer.vertex_storage_buffers,
                    &mut command_buffer.need_vertex_storage_buffer_bind,
                ),
                GraphicsStage::Fragment => (
                    &mut command_buffer.fragment_storage_buffers,
                    &mut command_buffer.need_fragment_storage_buffer_bind,
                ),
            };

            if buffers[slot].get() != active_buffer.handle.get() {
                buffers[slot] = active_buffer.handle.clone();
                *need_bind = true;

                command_buffer.track_buffer(&active_buffer);
            }
        }
    }

    /// Translation of `METAL_INTERNAL_BindGraphicsResources()`.
    ///
    /// This function assumes that it's called from within an autorelease pool
    fn bind_graphics_resources(command_buffer: &mut MetalCommandBuffer) {
        let Some(graphics_pipeline) = command_buffer.graphics_pipeline.clone() else {
            return;
        };
        let header = &graphics_pipeline.header;
        let offsets: [NSUInteger; MAX_STORAGE_BUFFERS_PER_STAGE as usize] = [0; 8];
        let encoder = command_buffer.render_encoder.get();

        // SAFETY: a render encoder's bindings, with C arrays of the slots'
        // handles at least as long as the ranges.
        unsafe {
            // Vertex Buffers
            if command_buffer.need_vertex_buffer_bind {
                let count = command_buffer.vertex_buffer_count as usize;
                let metal_buffers = ids(&command_buffer.vertex_buffers);
                let mut buffer_offsets = [0 as NSUInteger; super::commands::VERTEX_BUFFERS];
                for (o, &offset) in buffer_offsets
                    .iter_mut()
                    .zip(&command_buffer.vertex_buffer_offsets)
                    .take(count)
                {
                    *o = offset as NSUInteger;
                }
                let r = range(METAL_FIRST_VERTEX_BUFFER_SLOT, count as u32);
                let _: () = msg_send![encoder, setVertexBuffers: metal_buffers.as_ptr()
                                                         offsets: buffer_offsets.as_ptr()
                                                       withRange: r];
                command_buffer.need_vertex_buffer_bind = false;
            }

            // Vertex Samplers+Textures

            if command_buffer.need_vertex_sampler_bind {
                if header.num_vertex_samplers > 0 {
                    let _: () = msg_send![encoder, setVertexSamplerStates: ids(&command_buffer.vertex_samplers).as_ptr()
                                                                withRange: range(0, header.num_vertex_samplers)];
                    let _: () = msg_send![encoder, setVertexTextures: ids(&command_buffer.vertex_textures).as_ptr()
                                                           withRange: range(0, header.num_vertex_samplers)];
                }
                command_buffer.need_vertex_sampler_bind = false;
            }

            // Vertex Storage Textures

            if command_buffer.need_vertex_storage_texture_bind {
                if header.num_vertex_storage_textures > 0 {
                    let _: () = msg_send![encoder, setVertexTextures: ids(&command_buffer.vertex_storage_textures).as_ptr()
                                                           withRange: range(header.num_vertex_samplers,
                                                                            header.num_vertex_storage_textures)];
                }
                command_buffer.need_vertex_storage_texture_bind = false;
            }

            // Vertex Storage Buffers

            if command_buffer.need_vertex_storage_buffer_bind {
                if header.num_vertex_storage_buffers > 0 {
                    let _: () = msg_send![encoder, setVertexBuffers: ids(&command_buffer.vertex_storage_buffers).as_ptr()
                                                            offsets: offsets.as_ptr()
                                                          withRange: range(header.num_vertex_uniform_buffers,
                                                                           header.num_vertex_storage_buffers)];
                }
                command_buffer.need_vertex_storage_buffer_bind = false;
            }

            // Vertex Uniform Buffers

            for i in 0..header.num_vertex_uniform_buffers as usize {
                if command_buffer.need_vertex_uniform_buffer_bind[i] {
                    if header.num_vertex_uniform_buffers as usize > i {
                        if let Some(index) = command_buffer.vertex_uniform_buffers[i] {
                            let uniform_buffer = &command_buffer.used_uniform_buffers[index];
                            let _: () = msg_send![encoder, setVertexBuffer: uniform_buffer.handle.get()
                                                                    offset: uniform_buffer.draw_offset as NSUInteger
                                                                   atIndex: i as NSUInteger];
                        }
                    }
                    command_buffer.need_vertex_uniform_buffer_bind[i] = false;
                }
            }

            // Fragment Samplers+Textures

            if command_buffer.need_fragment_sampler_bind {
                if header.num_fragment_samplers > 0 {
                    let _: () = msg_send![encoder, setFragmentSamplerStates: ids(&command_buffer.fragment_samplers).as_ptr()
                                                                  withRange: range(0, header.num_fragment_samplers)];
                    let _: () = msg_send![encoder, setFragmentTextures: ids(&command_buffer.fragment_textures).as_ptr()
                                                             withRange: range(0, header.num_fragment_samplers)];
                }
                command_buffer.need_fragment_sampler_bind = false;
            }

            // Fragment Storage Textures

            if command_buffer.need_fragment_storage_texture_bind {
                if header.num_fragment_storage_textures > 0 {
                    let _: () = msg_send![encoder, setFragmentTextures: ids(&command_buffer.fragment_storage_textures).as_ptr()
                                                             withRange: range(header.num_fragment_samplers,
                                                                              header.num_fragment_storage_textures)];
                }
                command_buffer.need_fragment_storage_texture_bind = false;
            }

            // Fragment Storage Buffers

            if command_buffer.need_fragment_storage_buffer_bind {
                if header.num_fragment_storage_buffers > 0 {
                    let _: () = msg_send![encoder, setFragmentBuffers: ids(&command_buffer.fragment_storage_buffers).as_ptr()
                                                              offsets: offsets.as_ptr()
                                                            withRange: range(header.num_fragment_uniform_buffers,
                                                                             header.num_fragment_storage_buffers)];
                }
                command_buffer.need_fragment_storage_buffer_bind = false;
            }

            // Fragment Uniform Buffers

            for i in 0..header.num_fragment_uniform_buffers as usize {
                if command_buffer.need_fragment_uniform_buffer_bind[i] {
                    if header.num_fragment_uniform_buffers as usize > i {
                        if let Some(index) = command_buffer.fragment_uniform_buffers[i] {
                            let uniform_buffer = &command_buffer.used_uniform_buffers[index];
                            let _: () = msg_send![encoder, setFragmentBuffer: uniform_buffer.handle.get()
                                                                      offset: uniform_buffer.draw_offset as NSUInteger
                                                                     atIndex: i as NSUInteger];
                        }
                    }
                    command_buffer.need_fragment_uniform_buffer_bind[i] = false;
                }
            }
        }
    }

    /// Translation of `METAL_INTERNAL_BindComputeResources()`.
    ///
    /// This function assumes that it's called from within an autorelease pool
    fn bind_compute_resources(command_buffer: &mut MetalCommandBuffer) {
        let Some(compute_pipeline) = command_buffer.compute_pipeline.clone() else {
            return;
        };
        let header = &compute_pipeline.header;
        let offsets: [NSUInteger; MAX_STORAGE_BUFFERS_PER_STAGE as usize] = [0; 8];
        let encoder = command_buffer.compute_encoder.get();

        // SAFETY: a compute encoder's bindings, with C arrays of the slots'
        // handles at least as long as the ranges.
        unsafe {
            if command_buffer.need_compute_sampler_bind {
                if header.num_samplers > 0 {
                    let _: () = msg_send![encoder, setTextures: ids(&command_buffer.compute_sampler_textures).as_ptr()
                                                     withRange: range(0, header.num_samplers)];
                    let _: () = msg_send![encoder, setSamplerStates: ids(&command_buffer.compute_samplers).as_ptr()
                                                          withRange: range(0, header.num_samplers)];
                }
                command_buffer.need_compute_sampler_bind = false;
            }

            if command_buffer.need_compute_read_only_storage_texture_bind {
                if header.num_readonly_storage_textures > 0 {
                    let _: () = msg_send![encoder, setTextures: ids(&command_buffer.compute_read_only_textures).as_ptr()
                                                     withRange: range(header.num_samplers,
                                                                      header.num_readonly_storage_textures)];
                }
                command_buffer.need_compute_read_only_storage_texture_bind = false;
            }

            if command_buffer.need_compute_read_only_storage_buffer_bind {
                if header.num_readonly_storage_buffers > 0 {
                    let _: () = msg_send![encoder, setBuffers: ids(&command_buffer.compute_read_only_buffers).as_ptr()
                                                      offsets: offsets.as_ptr()
                                                    withRange: range(header.num_uniform_buffers,
                                                                     header.num_readonly_storage_buffers)];
                }
                command_buffer.need_compute_read_only_storage_buffer_bind = false;
            }

            for i in 0..UNIFORM_BUFFERS {
                if command_buffer.need_compute_uniform_buffer_bind[i]
                    && header.num_uniform_buffers as usize > i
                {
                    if let Some(index) = command_buffer.compute_uniform_buffers[i] {
                        let uniform_buffer = &command_buffer.used_uniform_buffers[index];
                        let _: () = msg_send![encoder, setBuffer: uniform_buffer.handle.get()
                                                          offset: uniform_buffer.draw_offset as NSUInteger
                                                         atIndex: i as NSUInteger];
                    }
                }
                command_buffer.need_compute_uniform_buffer_bind[i] = false;
            }
        }
    }

    /// Translation of `METAL_DrawIndexedPrimitives()`.
    pub(super) fn draw_indexed_primitives_internal(
        command_buffer: &mut MetalCommandBuffer,
        num_indices: u32,
        num_instances: u32,
        first_index: u32,
        vertex_offset: i32,
        first_instance: u32,
    ) {
        let Some(pipeline) = command_buffer.graphics_pipeline.clone() else {
            return;
        };
        let primitive_type = pipeline.primitive_type;
        let index_size = index_size(command_buffer.index_element_size);

        Self::bind_graphics_resources(command_buffer);

        let Some(index_buffer) = command_buffer.index_buffer.clone() else {
            return;
        };
        // SAFETY: a render encoder's draw with the bound index buffer.
        unsafe {
            let _: () = msg_send![command_buffer.render_encoder,
                drawIndexedPrimitives: sdl_to_metal_primitive_type(primitive_type)
                           indexCount: num_indices as NSUInteger
                            indexType: sdl_to_metal_index_type(command_buffer.index_element_size)
                          indexBuffer: index_buffer.handle.get()
                    indexBufferOffset: command_buffer
                        .index_buffer_offset
                        .wrapping_add(first_index.wrapping_mul(index_size)) as NSUInteger
                        instanceCount: num_instances as NSUInteger
                           baseVertex: vertex_offset as NSInteger
                         baseInstance: first_instance as NSUInteger];
        }
    }

    /// Translation of `METAL_DrawPrimitives()`.
    pub(super) fn draw_primitives_internal(
        command_buffer: &mut MetalCommandBuffer,
        num_vertices: u32,
        num_instances: u32,
        first_vertex: u32,
        first_instance: u32,
    ) {
        let Some(pipeline) = command_buffer.graphics_pipeline.clone() else {
            return;
        };
        let primitive_type = pipeline.primitive_type;

        Self::bind_graphics_resources(command_buffer);

        // SAFETY: a render encoder's draw.
        unsafe {
            let _: () = msg_send![command_buffer.render_encoder,
                drawPrimitives: sdl_to_metal_primitive_type(primitive_type)
                   vertexStart: first_vertex as NSUInteger
                   vertexCount: num_vertices as NSUInteger
                 instanceCount: num_instances as NSUInteger
                  baseInstance: first_instance as NSUInteger];
        }
    }

    /// Translation of `METAL_DrawPrimitivesIndirect()`.
    pub(super) fn draw_primitives_indirect_internal(
        command_buffer: &mut MetalCommandBuffer,
        buffer: &BufferContainer,
        offset: u32,
        draw_count: u32,
    ) {
        let metal_buffer = buffer.active();
        let Some(pipeline) = command_buffer.graphics_pipeline.clone() else {
            return;
        };
        let primitive_type = pipeline.primitive_type;

        Self::bind_graphics_resources(command_buffer);

        /* Metal: "We have multi-draw at home!"
         * Multi-draw at home:
         */
        for i in 0..draw_count {
            let indirect_offset = offset as usize + size_of::<IndirectDrawCommand>() * i as usize;
            // SAFETY: a render encoder's indirect draw.
            unsafe {
                let _: () = msg_send![command_buffer.render_encoder,
                          drawPrimitives: sdl_to_metal_primitive_type(primitive_type)
                          indirectBuffer: metal_buffer.handle.get()
                    indirectBufferOffset: indirect_offset as NSUInteger];
            }
        }

        command_buffer.track_buffer(&metal_buffer);
    }

    /// Translation of `METAL_DrawIndexedPrimitivesIndirect()`.
    pub(super) fn draw_indexed_primitives_indirect_internal(
        command_buffer: &mut MetalCommandBuffer,
        buffer: &BufferContainer,
        offset: u32,
        draw_count: u32,
    ) {
        let metal_buffer = buffer.active();
        let Some(pipeline) = command_buffer.graphics_pipeline.clone() else {
            return;
        };
        let primitive_type = pipeline.primitive_type;

        Self::bind_graphics_resources(command_buffer);

        let Some(index_buffer) = command_buffer.index_buffer.clone() else {
            return;
        };
        for i in 0..draw_count {
            let indirect_offset =
                offset as usize + size_of::<IndexedIndirectDrawCommand>() * i as usize;
            // SAFETY: a render encoder's indirect draw with the bound index
            // buffer.
            unsafe {
                let _: () = msg_send![command_buffer.render_encoder,
                    drawIndexedPrimitives: sdl_to_metal_primitive_type(primitive_type)
                                indexType: sdl_to_metal_index_type(command_buffer.index_element_size)
                              indexBuffer: index_buffer.handle.get()
                        indexBufferOffset: command_buffer.index_buffer_offset as NSUInteger
                           indirectBuffer: metal_buffer.handle.get()
                     indirectBufferOffset: indirect_offset as NSUInteger];
            }
        }

        command_buffer.track_buffer(&metal_buffer);
    }

    /// Translation of `METAL_EndRenderPass()`.
    pub(super) fn end_render_pass_internal(command_buffer: &mut MetalCommandBuffer) {
        // SAFETY: ending the render encoder.
        let _: () = unsafe { msg_send![command_buffer.render_encoder, endEncoding] };
        command_buffer.render_encoder = StrongPtr::nil();

        command_buffer.vertex_buffers = Default::default();
        command_buffer.vertex_buffer_offsets = [0; super::commands::VERTEX_BUFFERS];
        command_buffer.vertex_buffer_count = 0;
        command_buffer.vertex_samplers = Default::default();
        command_buffer.vertex_textures = Default::default();
        command_buffer.fragment_samplers = Default::default();
        command_buffer.fragment_textures = Default::default();
        command_buffer.vertex_storage_textures = Default::default();
        command_buffer.fragment_storage_textures = Default::default();
        command_buffer.vertex_storage_buffers = Default::default();
        command_buffer.fragment_storage_buffers = Default::default();
    }

    // Blit

    /// Translation of `METAL_Blit()`.
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

    // Compute State

    /// Translation of `METAL_BeginComputePass()`.
    pub(super) fn begin_compute_pass_internal(
        &self,
        command_buffer: &mut MetalCommandBuffer,
        storage_texture_bindings: &[StorageTextureReadWriteBinding<'_>],
        storage_buffer_bindings: &[StorageBufferReadWriteBinding<'_>],
    ) {
        // SAFETY: an autoreleased compute encoder of the command buffer,
        // retained.
        command_buffer.compute_encoder =
            unsafe { StrongPtr::retain(msg_send![command_buffer.handle, computeCommandEncoder]) };

        for (i, binding) in storage_texture_bindings.iter().enumerate() {
            let texture_container = object::<TextureContainer>(&binding.texture.raw);

            let texture = self.prepare_texture_for_write(&texture_container, binding.cycle);

            command_buffer.track_texture(&texture);

            // SAFETY: a +1 view of one level and slice of the texture.
            let texture_view = unsafe {
                StrongPtr::from_owned(msg_send![texture.handle,
                    newTextureViewWithPixelFormat: sdl_to_metal_texture_format(texture_container.info.format)
                                      textureType: sdl_to_metal_texture_type(texture_container.info.texture_type, false)
                                           levels: NSRange::new(binding.mip_level as NSUInteger, 1)
                                           slices: NSRange::new(binding.layer as NSUInteger, 1)])
            };

            command_buffer.compute_read_write_textures[i] = texture_view;
        }

        for (i, binding) in storage_buffer_bindings.iter().enumerate() {
            let buffer_container = object::<BufferContainer>(&binding.buffer.raw);

            let buffer = self.prepare_buffer_for_write(&buffer_container, binding.cycle);

            command_buffer.track_buffer(&buffer);

            command_buffer.compute_read_write_buffers[i] = buffer.handle.clone();
        }
    }

    /// Translation of `METAL_BindComputePipeline()`.
    pub(super) fn bind_compute_pipeline_internal(
        &self,
        command_buffer: &mut MetalCommandBuffer,
        pipeline: Arc<MetalComputePipeline>,
    ) {
        // SAFETY: a compute encoder's pipeline state.
        let _: () = unsafe {
            msg_send![command_buffer.compute_encoder, setComputePipelineState: pipeline.handle.get()]
        };

        command_buffer.need_compute_uniform_buffer_bind = [true; UNIFORM_BUFFERS];

        for i in 0..pipeline.header.num_uniform_buffers as usize {
            if command_buffer.compute_uniform_buffers[i].is_none() {
                command_buffer.compute_uniform_buffers[i] =
                    self.acquire_uniform_buffer_from_pool(command_buffer);
            }
        }

        command_buffer.need_compute_sampler_bind = true;
        command_buffer.need_compute_read_only_storage_texture_bind = true;
        command_buffer.need_compute_read_only_storage_buffer_bind = true;

        let header = &pipeline.header;
        // SAFETY: a compute encoder's bindings of the write-only resources,
        // with C arrays at least as long as the ranges.
        unsafe {
            // Bind write-only resources
            if header.num_readwrite_storage_textures > 0 {
                let _: () = msg_send![command_buffer.compute_encoder,
                    setTextures: ids(&command_buffer.compute_read_write_textures).as_ptr()
                      withRange: range(
                          header.num_samplers + header.num_readonly_storage_textures,
                          header.num_readwrite_storage_textures)];
            }

            let offsets = [0 as NSUInteger; super::commands::COMPUTE_WRITE_BUFFERS];
            if header.num_readwrite_storage_buffers > 0 {
                let _: () = msg_send![command_buffer.compute_encoder,
                    setBuffers: ids(&command_buffer.compute_read_write_buffers).as_ptr()
                       offsets: offsets.as_ptr()
                     withRange: range(
                         header.num_uniform_buffers + header.num_readonly_storage_buffers,
                         header.num_readwrite_storage_buffers)];
            }
        }

        command_buffer.compute_pipeline = Some(pipeline);
    }

    /// Translation of `METAL_BindComputeSamplers()`.
    pub(super) fn bind_compute_samplers_internal(
        command_buffer: &mut MetalCommandBuffer,
        first_slot: u32,
        texture_sampler_bindings: &[TextureSamplerBinding<'_>],
    ) {
        for (i, binding) in texture_sampler_bindings.iter().enumerate() {
            let slot = first_slot as usize + i;
            let texture_container = object::<TextureContainer>(&binding.texture.raw);
            let sampler = object::<MetalSampler>(&binding.sampler.raw);
            let active_texture = texture_container.active();

            if command_buffer.compute_samplers[slot].get() != sampler.handle.get() {
                command_buffer.compute_samplers[slot] = sampler.handle.clone();
                command_buffer.need_compute_sampler_bind = true;
            }

            if command_buffer.compute_sampler_textures[slot].get() != active_texture.handle.get() {
                command_buffer.track_texture(&active_texture);

                command_buffer.compute_sampler_textures[slot] = active_texture.handle.clone();

                command_buffer.need_compute_sampler_bind = true;
            }
        }
    }

    /// Translation of `METAL_BindComputeStorageTextures()`.
    pub(super) fn bind_compute_storage_textures_internal(
        command_buffer: &mut MetalCommandBuffer,
        first_slot: u32,
        storage_textures: &[&Texture],
    ) {
        for (i, texture) in storage_textures.iter().enumerate() {
            let slot = first_slot as usize + i;
            let active_texture = object::<TextureContainer>(&texture.raw).active();

            if command_buffer.compute_read_only_textures[slot].get() != active_texture.handle.get()
            {
                command_buffer.track_texture(&active_texture);

                command_buffer.compute_read_only_textures[slot] = active_texture.handle.clone();

                command_buffer.need_compute_read_only_storage_texture_bind = true;
            }
        }
    }

    /// Translation of `METAL_BindComputeStorageBuffers()`.
    pub(super) fn bind_compute_storage_buffers_internal(
        command_buffer: &mut MetalCommandBuffer,
        first_slot: u32,
        storage_buffers: &[&Buffer],
    ) {
        for (i, buffer) in storage_buffers.iter().enumerate() {
            let slot = first_slot as usize + i;
            let active_buffer = object::<BufferContainer>(&buffer.raw).active();

            if command_buffer.compute_read_only_buffers[slot].get() != active_buffer.handle.get() {
                command_buffer.track_buffer(&active_buffer);

                command_buffer.compute_read_only_buffers[slot] = active_buffer.handle.clone();

                command_buffer.need_compute_read_only_storage_buffer_bind = true;
            }
        }
    }

    /// Translation of `METAL_DispatchCompute()`.
    pub(super) fn dispatch_compute_internal(
        command_buffer: &mut MetalCommandBuffer,
        groupcount_x: u32,
        groupcount_y: u32,
        groupcount_z: u32,
    ) {
        let Some(pipeline) = command_buffer.compute_pipeline.clone() else {
            return;
        };
        let threadgroups = mtl_size_make(groupcount_x, groupcount_y, groupcount_z);
        let threads_per_threadgroup = mtl_size_make(
            pipeline.threadcount_x,
            pipeline.threadcount_y,
            pipeline.threadcount_z,
        );

        Self::bind_compute_resources(command_buffer);

        // SAFETY: a compute encoder's dispatch.
        unsafe {
            let _: () = msg_send![command_buffer.compute_encoder,
                 dispatchThreadgroups: threadgroups
                threadsPerThreadgroup: threads_per_threadgroup];
        }
    }

    /// Translation of `METAL_DispatchComputeIndirect()`.
    pub(super) fn dispatch_compute_indirect_internal(
        command_buffer: &mut MetalCommandBuffer,
        buffer: &BufferContainer,
        offset: u32,
    ) {
        let metal_buffer = buffer.active();
        let Some(pipeline) = command_buffer.compute_pipeline.clone() else {
            return;
        };
        let threads_per_threadgroup = mtl_size_make(
            pipeline.threadcount_x,
            pipeline.threadcount_y,
            pipeline.threadcount_z,
        );

        Self::bind_compute_resources(command_buffer);

        // SAFETY: a compute encoder's indirect dispatch.
        unsafe {
            let _: () = msg_send![command_buffer.compute_encoder,
                dispatchThreadgroupsWithIndirectBuffer: metal_buffer.handle.get()
                                  indirectBufferOffset: offset as NSUInteger
                                 threadsPerThreadgroup: threads_per_threadgroup];
        }

        command_buffer.track_buffer(&metal_buffer);
    }

    /// Translation of `METAL_EndComputePass()`.
    pub(super) fn end_compute_pass_internal(command_buffer: &mut MetalCommandBuffer) {
        // SAFETY: ending the compute encoder.
        let _: () = unsafe { msg_send![command_buffer.compute_encoder, endEncoding] };
        command_buffer.compute_encoder = StrongPtr::nil();

        command_buffer.compute_samplers = Default::default();
        command_buffer.compute_sampler_textures = Default::default();
        command_buffer.compute_read_write_textures = Default::default();
        command_buffer.compute_read_write_buffers = Default::default();
        command_buffer.compute_read_only_textures = Default::default();
        command_buffer.compute_read_only_buffers = Default::default();
    }

    /// Translation of `METAL_PushVertexUniformData()`,
    /// `METAL_PushFragmentUniformData()` and `METAL_PushComputeUniformData()`.
    pub(super) fn push_stage_uniform_data(
        &self,
        command_buffer: &mut MetalCommandBuffer,
        stage: UniformStage,
        slot_index: u32,
        data: &[u8],
    ) {
        self.push_uniform_data(command_buffer, stage, slot_index, data);
    }
}

/// The destination of a buffer copy: a buffer, or a transfer buffer
/// (`METAL_DownloadFromBuffer()` passes one as the other).
pub(super) enum BackendBufferRef<'a> {
    Buffer(&'a Buffer),
    Transfer(&'a crate::gpu::TransferBuffer),
}

impl BackendBufferRef<'_> {
    fn container(&self) -> Arc<BufferContainer> {
        match self {
            BackendBufferRef::Buffer(b) => object::<BufferContainer>(&b.raw),
            BackendBufferRef::Transfer(t) => object::<BufferContainer>(&t.raw),
        }
    }
}
