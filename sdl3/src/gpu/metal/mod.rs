// Rust translation of src/gpu/metal/SDL_gpu_metal.m from Simple DirectMedia
// Layer.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! The Metal GPU backend ("metal", tried first on Apple platforms): the GPU
//! API on a Metal device. Metal, QuartzCore and CoreGraphics are linked, as
//! upstream links them, and the Objective-C classes are reached through
//! the runtime (`core::apple::objc`). It takes MSL source and Metal
//! libraries (metallib).
//!
//! * `PrepareDriver`, the device (the low-power one when preferred, the
//!   hardware requirements), the device properties, and
//!   `SupportsTextureFormat` ([`device`]);
//! * the conversion tables ([`tables`]) and the framework declarations
//!   ([`sys`]);
//! * textures and buffers in their cycling containers, transfer buffers
//!   and their mapping, uniform buffers and their pool, samplers, shaders,
//!   the deferred destruction and the debug names ([`resources`]);
//! * graphics pipelines (with their depth-stencil states) and compute
//!   pipelines ([`pipelines`]);
//! * command buffers and their pool, resource tracking, fences, uniform
//!   data, submission (with presentation and the cleanup of finished
//!   command buffers), cancelling and waiting ([`commands`]);
//! * copy passes, render passes (targets, dynamic state, bindings, draws,
//!   indirect ones too), compute passes and dispatches, mipmap generation
//!   on a blit encoder, and blits through the front end's blit pipelines
//!   with the blit shaders ([`shaders`], made by
//!   `tools/gen_metal_shaders.py`) ([`passes`]);
//! * claimed windows, their Metal views and layers (from the video
//!   driver), compositions, present modes, frames in flight and swapchain
//!   texture acquisition ([`swapchain`]).
//!
//! The OpenXR entry points (`HAVE_GPU_OPENXR`, all of which fail on Metal
//! upstream) are not translated, nor are the iOS, tvOS and visionOS
//! branches.

mod commands;
mod device;
mod passes;
mod pipelines;
mod resources;
mod shaders;
mod swapchain;
mod sys;
mod tables;
#[cfg(test)]
mod tests;

use std::ptr::NonNull;
use std::sync::atomic::AtomicU32;
use std::sync::{Arc, Mutex, MutexGuard};

use commands::{AvailableCommandBuffers, MetalCommandBuffer, MetalFence, UniformStage};
use passes::GraphicsStage;
use pipelines::{MetalComputePipeline, MetalGraphicsPipeline};
use resources::{BufferContainer, MetalUniformBuffer, PendingDestroys, TextureContainer};
use swapchain::WindowEntry;

use super::sysgpu::{
    BackendCommandBuffer, BackendDevice, BackendObject, BackendSwapchainTexture, BlitPipelineCache,
    BlitShaders, ComputePipelineHeader, GpuBootstrap, GpuDriver, GraphicsPipelineHeader,
};
use super::{
    BlitInfo, BufferBinding, BufferLocation, BufferRegion, BufferUsageFlags, ColorTargetInfo,
    CommandBuffer, ComputePipelineCreateInfo, DepthStencilTargetInfo, GraphicsPipelineCreateInfo,
    IndexElementSize, PresentMode, SampleCount, Sampler, SamplerCreateInfo, Shader,
    ShaderCreateInfo, ShaderFormat, StorageBufferReadWriteBinding, StorageTextureReadWriteBinding,
    SwapchainComposition, Texture, TextureCreateInfo, TextureFormat, TextureLocation,
    TextureRegion, TextureSamplerBinding, TextureTransferInfo, TextureType, TextureUsageFlags,
    TransferBufferLocation, TransferBufferUsage, Viewport,
};
use crate::core::apple::objc::{autoreleasepool, StrongPtr};
use crate::error::{Error, Result};
use crate::log::Category;
use crate::properties::Properties;
use crate::video::{FColor, Rect, Window};

// Defines

/// `METAL_FIRST_VERTEX_BUFFER_SLOT`.
const METAL_FIRST_VERTEX_BUFFER_SLOT: u32 = 14;

/// The Metal backend. Translation of `MetalDriver`.
pub(crate) static METAL_DRIVER: GpuBootstrap = GpuBootstrap {
    name: "metal",
    prepare_driver: device::prepare_driver,
    create_device: metal_create_device,
};

/// `METAL_CreateDevice()` for the bootstrap: the renderer as a driver.
fn metal_create_device(
    debug_mode: bool,
    prefer_low_power: bool,
    props: &Properties,
) -> Result<BackendDevice> {
    let renderer = autoreleasepool(|| device::create_device(debug_mode, prefer_low_power, props))?;
    Ok(BackendDevice {
        driver: Box::new(renderer),
        shader_formats: ShaderFormat::MSL | ShaderFormat::METALLIB,
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

/// A backend object of this backend.
fn object<T: std::any::Any + Send + Sync>(object: &BackendObject) -> Arc<T> {
    object
        .downcast::<T>()
        .expect("a GPU object of another backend")
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

impl BlitResources {
    /// The blit shaders (when they were all made), the linear and nearest
    /// samplers, and the pipelines.
    fn parts(
        &mut self,
    ) -> (
        Option<BlitShaders<'_>>,
        Option<&Sampler>,
        Option<&Sampler>,
        &mut BlitPipelineCache,
    ) {
        let BlitResources {
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

/// The Metal device. Translation of `MetalRenderer`.
///
/// Upstream's locks guard the same state here: `windowLock` the claimed
/// windows, `acquireCommandBufferLock` the available command buffers,
/// `submitLock` the submitted ones, `fenceLock` the fence pool,
/// `acquireUniformBufferLock` the uniform buffer pool and `disposeLock`
/// the pending destroys. The containers' cycling (which upstream doesn't
/// lock) is behind each container's own lock.
struct MetalRenderer {
    device: StrongPtr,
    queue: StrongPtr,

    debug_mode: bool,
    props: Properties,
    allowed_frames_in_flight: AtomicU32,

    /// `claimedWindows`, with `windowLock`.
    claimed_windows: Mutex<Vec<Arc<WindowEntry>>>,

    /// `availableCommandBuffers`, with `acquireCommandBufferLock`.
    available_command_buffers: Mutex<AvailableCommandBuffers>,

    /// `submittedCommandBuffers`, with `submitLock`.
    submitted_command_buffers: Mutex<Vec<MetalCommandBuffer>>,

    /// `availableFences`, with `fenceLock`.
    available_fences: Mutex<Vec<Arc<MetalFence>>>,

    /// `uniformBufferPool`, with `acquireUniformBufferLock`.
    uniform_buffer_pool: Mutex<Vec<MetalUniformBuffer>>,

    /// The `*ContainersToDestroy` arrays, with `disposeLock`.
    dispose: Mutex<PendingDestroys>,

    // Blit
    blit: Mutex<BlitResources>,
}

impl MetalRenderer {
    /// `SET_STRING_ERROR_AND_RETURN` with this renderer's debug mode.
    fn set_string_error(&self, msg: &str) -> Error {
        set_string_error(self.debug_mode, msg)
    }

    /// The Metal command buffer of a backend command buffer.
    fn metal_command_buffer(command_buffer: &mut BackendCommandBuffer) -> &mut MetalCommandBuffer {
        command_buffer
            .downcast_mut::<MetalCommandBuffer>()
            .expect("a command buffer of another backend")
    }

    /// An owned Metal command buffer.
    fn owned_command_buffer(command_buffer: Box<BackendCommandBuffer>) -> MetalCommandBuffer {
        *command_buffer
            .downcast::<MetalCommandBuffer>()
            .expect("a command buffer of another backend")
    }
}

impl GpuDriver for MetalRenderer {
    // Device

    fn destroy(&mut self) {
        autoreleasepool(|| self.destroy_device());
    }

    /// Translation of `METAL_GetDeviceProperties()`.
    fn properties(&self) -> Properties {
        self.props.clone()
    }

    // State Creation

    fn create_compute_pipeline(
        &self,
        createinfo: &ComputePipelineCreateInfo<'_>,
    ) -> Result<(BackendObject, ComputePipelineHeader)> {
        let pipeline = autoreleasepool(|| self.create_compute_pipeline_internal(createinfo))?;
        let header = pipeline.header;
        Ok((BackendObject::new(pipeline), header))
    }

    fn create_graphics_pipeline(
        &self,
        createinfo: &GraphicsPipelineCreateInfo<'_>,
    ) -> Result<(BackendObject, GraphicsPipelineHeader)> {
        let pipeline = autoreleasepool(|| self.create_graphics_pipeline_internal(createinfo))?;
        let header = pipeline.header;
        Ok((BackendObject::new(pipeline), header))
    }

    fn create_sampler(&self, createinfo: &SamplerCreateInfo) -> Result<BackendObject> {
        let sampler = autoreleasepool(|| self.create_sampler_internal(createinfo))?;
        Ok(BackendObject::new(sampler))
    }

    fn create_shader(&self, createinfo: &ShaderCreateInfo<'_>) -> Result<BackendObject> {
        let shader = autoreleasepool(|| self.create_shader_internal(createinfo))?;
        Ok(BackendObject::new(shader))
    }

    fn create_texture(&self, createinfo: &TextureCreateInfo) -> Result<BackendObject> {
        let container = autoreleasepool(|| self.create_texture_container(createinfo))?;
        Ok(BackendObject::new(container))
    }

    /// Translation of `METAL_CreateBuffer()`.
    fn create_buffer(
        &self,
        _usage_flags: BufferUsageFlags,
        size: u32,
        debug_name: Option<&str>,
    ) -> Result<BackendObject> {
        let container =
            autoreleasepool(|| self.create_buffer_container(size, true, false, debug_name))?;
        Ok(BackendObject::new(container))
    }

    /// Translation of `METAL_CreateTransferBuffer()`.
    fn create_transfer_buffer(
        &self,
        usage: TransferBufferUsage,
        size: u32,
        debug_name: Option<&str>,
    ) -> Result<BackendObject> {
        let container = autoreleasepool(|| {
            self.create_buffer_container(
                size,
                false,
                usage == TransferBufferUsage::Upload,
                debug_name,
            )
        })?;
        Ok(BackendObject::new(container))
    }

    // Debug Naming

    fn set_buffer_name(&self, buffer: &BackendObject, text: &str) {
        autoreleasepool(|| self.set_buffer_name_internal(&object::<BufferContainer>(buffer), text));
    }

    fn set_texture_name(&self, texture: &BackendObject, text: &str) {
        autoreleasepool(|| {
            self.set_texture_name_internal(&object::<TextureContainer>(texture), text)
        });
    }

    fn insert_debug_label(&self, command_buffer: &mut BackendCommandBuffer, text: &str) {
        autoreleasepool(|| {
            Self::insert_debug_label_internal(Self::metal_command_buffer(command_buffer), text)
        });
    }

    fn push_debug_group(&self, command_buffer: &mut BackendCommandBuffer, name: &str) {
        autoreleasepool(|| {
            Self::push_debug_group_internal(Self::metal_command_buffer(command_buffer), name)
        });
    }

    fn pop_debug_group(&self, command_buffer: &mut BackendCommandBuffer) {
        autoreleasepool(|| {
            Self::pop_debug_group_internal(Self::metal_command_buffer(command_buffer))
        });
    }

    // Disposal

    fn release_texture(&self, texture: &BackendObject) {
        self.release_texture_container(object::<TextureContainer>(texture));
    }

    /// Translation of `METAL_ReleaseSampler()`: the sampler is freed when
    /// the front end drops its handle.
    fn release_sampler(&self, _sampler: &BackendObject) {}

    fn release_buffer(&self, buffer: &BackendObject) {
        self.release_buffer_container(object::<BufferContainer>(buffer));
    }

    /// Translation of `METAL_ReleaseTransferBuffer()`.
    fn release_transfer_buffer(&self, transfer_buffer: &BackendObject) {
        self.release_buffer_container(object::<BufferContainer>(transfer_buffer));
    }

    /// Translation of `METAL_ReleaseShader()`: the shader is freed when the
    /// front end drops its handle.
    fn release_shader(&self, _shader: &BackendObject) {}

    /// Translation of `METAL_ReleaseComputePipeline()` (as for shaders).
    fn release_compute_pipeline(&self, _compute_pipeline: &BackendObject) {}

    /// Translation of `METAL_ReleaseGraphicsPipeline()` (as for shaders).
    fn release_graphics_pipeline(&self, _graphics_pipeline: &BackendObject) {}

    // Render Pass

    fn begin_render_pass(
        &self,
        command_buffer: &mut BackendCommandBuffer,
        color_target_infos: &[ColorTargetInfo<'_>],
        depth_stencil_target_info: Option<&DepthStencilTargetInfo<'_>>,
    ) {
        autoreleasepool(|| {
            self.begin_render_pass_internal(
                Self::metal_command_buffer(command_buffer),
                color_target_infos,
                depth_stencil_target_info,
            )
        });
    }

    fn bind_graphics_pipeline(
        &self,
        command_buffer: &mut BackendCommandBuffer,
        graphics_pipeline: &BackendObject,
    ) {
        autoreleasepool(|| {
            self.bind_graphics_pipeline_internal(
                Self::metal_command_buffer(command_buffer),
                object::<MetalGraphicsPipeline>(graphics_pipeline),
            )
        });
    }

    fn set_viewport(&self, command_buffer: &mut BackendCommandBuffer, viewport: &Viewport) {
        autoreleasepool(|| {
            Self::set_viewport_internal(Self::metal_command_buffer(command_buffer), viewport)
        });
    }

    fn set_scissor(&self, command_buffer: &mut BackendCommandBuffer, scissor: &Rect) {
        autoreleasepool(|| {
            Self::set_scissor_internal(Self::metal_command_buffer(command_buffer), scissor)
        });
    }

    fn set_blend_constants(
        &self,
        command_buffer: &mut BackendCommandBuffer,
        blend_constants: FColor,
    ) {
        autoreleasepool(|| {
            Self::set_blend_constants_internal(
                Self::metal_command_buffer(command_buffer),
                blend_constants,
            )
        });
    }

    fn set_stencil_reference(&self, command_buffer: &mut BackendCommandBuffer, reference: u8) {
        autoreleasepool(|| {
            Self::set_stencil_reference_internal(
                Self::metal_command_buffer(command_buffer),
                reference,
            )
        });
    }

    fn bind_vertex_buffers(
        &self,
        command_buffer: &mut BackendCommandBuffer,
        first_slot: u32,
        bindings: &[BufferBinding<'_>],
    ) {
        Self::bind_vertex_buffers_internal(
            Self::metal_command_buffer(command_buffer),
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
            Self::metal_command_buffer(command_buffer),
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
            Self::metal_command_buffer(command_buffer),
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
            Self::metal_command_buffer(command_buffer),
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
            Self::metal_command_buffer(command_buffer),
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
            Self::metal_command_buffer(command_buffer),
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
            Self::metal_command_buffer(command_buffer),
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
            Self::metal_command_buffer(command_buffer),
            GraphicsStage::Fragment,
            first_slot,
            storage_buffers,
        );
    }

    /// Translation of `METAL_PushVertexUniformData()`.
    fn push_vertex_uniform_data(
        &self,
        command_buffer: &mut BackendCommandBuffer,
        slot_index: u32,
        data: &[u8],
    ) {
        autoreleasepool(|| {
            self.push_stage_uniform_data(
                Self::metal_command_buffer(command_buffer),
                UniformStage::Vertex,
                slot_index,
                data,
            )
        });
    }

    /// Translation of `METAL_PushFragmentUniformData()`.
    fn push_fragment_uniform_data(
        &self,
        command_buffer: &mut BackendCommandBuffer,
        slot_index: u32,
        data: &[u8],
    ) {
        autoreleasepool(|| {
            self.push_stage_uniform_data(
                Self::metal_command_buffer(command_buffer),
                UniformStage::Fragment,
                slot_index,
                data,
            )
        });
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
        autoreleasepool(|| {
            Self::draw_indexed_primitives_internal(
                Self::metal_command_buffer(command_buffer),
                num_indices,
                num_instances,
                first_index,
                vertex_offset,
                first_instance,
            )
        });
    }

    fn draw_primitives(
        &self,
        command_buffer: &mut BackendCommandBuffer,
        num_vertices: u32,
        num_instances: u32,
        first_vertex: u32,
        first_instance: u32,
    ) {
        autoreleasepool(|| {
            Self::draw_primitives_internal(
                Self::metal_command_buffer(command_buffer),
                num_vertices,
                num_instances,
                first_vertex,
                first_instance,
            )
        });
    }

    fn draw_primitives_indirect(
        &self,
        command_buffer: &mut BackendCommandBuffer,
        buffer: &BackendObject,
        offset: u32,
        draw_count: u32,
    ) {
        autoreleasepool(|| {
            Self::draw_primitives_indirect_internal(
                Self::metal_command_buffer(command_buffer),
                &object::<BufferContainer>(buffer),
                offset,
                draw_count,
            )
        });
    }

    fn draw_indexed_primitives_indirect(
        &self,
        command_buffer: &mut BackendCommandBuffer,
        buffer: &BackendObject,
        offset: u32,
        draw_count: u32,
    ) {
        autoreleasepool(|| {
            Self::draw_indexed_primitives_indirect_internal(
                Self::metal_command_buffer(command_buffer),
                &object::<BufferContainer>(buffer),
                offset,
                draw_count,
            )
        });
    }

    fn end_render_pass(&self, command_buffer: &mut BackendCommandBuffer) {
        autoreleasepool(|| {
            Self::end_render_pass_internal(Self::metal_command_buffer(command_buffer))
        });
    }

    // Compute Pass

    fn begin_compute_pass(
        &self,
        command_buffer: &mut BackendCommandBuffer,
        storage_texture_bindings: &[StorageTextureReadWriteBinding<'_>],
        storage_buffer_bindings: &[StorageBufferReadWriteBinding<'_>],
    ) {
        autoreleasepool(|| {
            self.begin_compute_pass_internal(
                Self::metal_command_buffer(command_buffer),
                storage_texture_bindings,
                storage_buffer_bindings,
            )
        });
    }

    fn bind_compute_pipeline(
        &self,
        command_buffer: &mut BackendCommandBuffer,
        compute_pipeline: &BackendObject,
    ) {
        autoreleasepool(|| {
            self.bind_compute_pipeline_internal(
                Self::metal_command_buffer(command_buffer),
                object::<MetalComputePipeline>(compute_pipeline),
            )
        });
    }

    fn bind_compute_samplers(
        &self,
        command_buffer: &mut BackendCommandBuffer,
        first_slot: u32,
        texture_sampler_bindings: &[TextureSamplerBinding<'_>],
    ) {
        Self::bind_compute_samplers_internal(
            Self::metal_command_buffer(command_buffer),
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
            Self::metal_command_buffer(command_buffer),
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
            Self::metal_command_buffer(command_buffer),
            first_slot,
            storage_buffers,
        );
    }

    /// Translation of `METAL_PushComputeUniformData()`.
    fn push_compute_uniform_data(
        &self,
        command_buffer: &mut BackendCommandBuffer,
        slot_index: u32,
        data: &[u8],
    ) {
        autoreleasepool(|| {
            self.push_stage_uniform_data(
                Self::metal_command_buffer(command_buffer),
                UniformStage::Compute,
                slot_index,
                data,
            )
        });
    }

    fn dispatch_compute(
        &self,
        command_buffer: &mut BackendCommandBuffer,
        groupcount_x: u32,
        groupcount_y: u32,
        groupcount_z: u32,
    ) {
        autoreleasepool(|| {
            Self::dispatch_compute_internal(
                Self::metal_command_buffer(command_buffer),
                groupcount_x,
                groupcount_y,
                groupcount_z,
            )
        });
    }

    fn dispatch_compute_indirect(
        &self,
        command_buffer: &mut BackendCommandBuffer,
        buffer: &BackendObject,
        offset: u32,
    ) {
        autoreleasepool(|| {
            Self::dispatch_compute_indirect_internal(
                Self::metal_command_buffer(command_buffer),
                &object::<BufferContainer>(buffer),
                offset,
            )
        });
    }

    fn end_compute_pass(&self, command_buffer: &mut BackendCommandBuffer) {
        autoreleasepool(|| {
            Self::end_compute_pass_internal(Self::metal_command_buffer(command_buffer))
        });
    }

    // TransferBuffer Data

    fn map_transfer_buffer(
        &self,
        transfer_buffer: &BackendObject,
        cycle: bool,
    ) -> Result<NonNull<u8>> {
        let contents = autoreleasepool(|| {
            self.map_transfer_buffer_internal(&object::<BufferContainer>(transfer_buffer), cycle)
        });
        NonNull::new(contents).ok_or_else(|| Error::new("Failed to map transfer buffer"))
    }

    fn unmap_transfer_buffer(&self, transfer_buffer: &BackendObject) {
        autoreleasepool(|| {
            self.unmap_transfer_buffer_internal(&object::<BufferContainer>(transfer_buffer))
        });
    }

    // Copy Pass

    fn begin_copy_pass(&self, command_buffer: &mut BackendCommandBuffer) {
        autoreleasepool(|| {
            Self::begin_copy_pass_internal(Self::metal_command_buffer(command_buffer))
        });
    }

    fn upload_to_texture(
        &self,
        command_buffer: &mut BackendCommandBuffer,
        source: &TextureTransferInfo<'_>,
        destination: &TextureRegion<'_>,
        cycle: bool,
    ) {
        autoreleasepool(|| {
            self.upload_to_texture_internal(
                Self::metal_command_buffer(command_buffer),
                source,
                destination,
                cycle,
            )
        });
    }

    fn upload_to_buffer(
        &self,
        command_buffer: &mut BackendCommandBuffer,
        source: &TransferBufferLocation<'_>,
        destination: &BufferRegion<'_>,
        cycle: bool,
    ) {
        autoreleasepool(|| {
            self.upload_to_buffer_internal(
                Self::metal_command_buffer(command_buffer),
                source,
                destination,
                cycle,
            )
        });
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
        autoreleasepool(|| {
            self.copy_texture_to_texture_internal(
                Self::metal_command_buffer(command_buffer),
                source,
                destination,
                w,
                h,
                d,
                cycle,
            )
        });
    }

    fn copy_buffer_to_buffer(
        &self,
        command_buffer: &mut BackendCommandBuffer,
        source: &BufferLocation<'_>,
        destination: &BufferLocation<'_>,
        size: u32,
        cycle: bool,
    ) {
        autoreleasepool(|| {
            self.copy_buffer_location_internal(
                Self::metal_command_buffer(command_buffer),
                source,
                destination,
                size,
                cycle,
            )
        });
    }

    fn generate_mipmaps(&self, command_buffer: &mut CommandBuffer, texture: &Texture) {
        autoreleasepool(|| Self::generate_mipmaps_internal(command_buffer, texture));
    }

    fn download_from_texture(
        &self,
        command_buffer: &mut BackendCommandBuffer,
        source: &TextureRegion<'_>,
        destination: &TextureTransferInfo<'_>,
    ) {
        autoreleasepool(|| {
            self.download_from_texture_internal(
                Self::metal_command_buffer(command_buffer),
                source,
                destination,
            )
        });
    }

    fn download_from_buffer(
        &self,
        command_buffer: &mut BackendCommandBuffer,
        source: &BufferRegion<'_>,
        destination: &TransferBufferLocation<'_>,
    ) {
        autoreleasepool(|| {
            self.download_from_buffer_internal(
                Self::metal_command_buffer(command_buffer),
                source,
                destination,
            )
        });
    }

    fn end_copy_pass(&self, command_buffer: &mut BackendCommandBuffer) {
        autoreleasepool(|| {
            Self::end_copy_pass_internal(Self::metal_command_buffer(command_buffer))
        });
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
        autoreleasepool(|| self.claim_window_internal(window))
    }

    fn release_window(&self, window: Window) {
        autoreleasepool(|| self.release_window_internal(window));
    }

    fn set_swapchain_parameters(
        &self,
        window: Window,
        swapchain_composition: SwapchainComposition,
        present_mode: PresentMode,
    ) -> Result<()> {
        autoreleasepool(|| {
            self.set_swapchain_parameters_internal(window, swapchain_composition, present_mode)
        })
    }

    /// Translation of `METAL_SetAllowedFramesInFlight()`.
    fn set_allowed_frames_in_flight(&self, allowed_frames_in_flight: u32) -> Result<()> {
        autoreleasepool(|| {
            self.wait_internal()?;

            self.allowed_frames_in_flight.store(
                allowed_frames_in_flight,
                std::sync::atomic::Ordering::SeqCst,
            );
            Ok(())
        })
    }

    fn swapchain_texture_format(&self, window: Window) -> Result<TextureFormat> {
        self.swapchain_texture_format_internal(window)
    }

    fn acquire_command_buffer(&self) -> Result<Box<BackendCommandBuffer>> {
        let command_buffer = autoreleasepool(|| self.acquire_command_buffer_internal());
        Ok(Box::new(command_buffer))
    }

    /// Translation of `METAL_AcquireSwapchainTexture()`.
    fn acquire_swapchain_texture(
        &self,
        command_buffer: &mut BackendCommandBuffer,
        window: Window,
    ) -> Result<Option<BackendSwapchainTexture>> {
        autoreleasepool(|| {
            self.acquire_swapchain_texture_internal(
                false,
                Self::metal_command_buffer(command_buffer),
                window,
            )
        })
    }

    fn wait_for_swapchain(&self, window: Window) -> Result<()> {
        autoreleasepool(|| self.wait_for_swapchain_internal(window))
    }

    /// Translation of `METAL_WaitAndAcquireSwapchainTexture()`.
    fn wait_and_acquire_swapchain_texture(
        &self,
        command_buffer: &mut BackendCommandBuffer,
        window: Window,
    ) -> Result<Option<BackendSwapchainTexture>> {
        autoreleasepool(|| {
            self.acquire_swapchain_texture_internal(
                true,
                Self::metal_command_buffer(command_buffer),
                window,
            )
        })
    }

    /// Translation of `METAL_Submit()`.
    fn submit(&self, command_buffer: Box<BackendCommandBuffer>) -> Result<()> {
        autoreleasepool(|| {
            self.submit_internal(Self::owned_command_buffer(command_buffer), false)
                .map(|_| ())
        })
    }

    /// Translation of `METAL_SubmitAndAcquireFence()`.
    fn submit_and_acquire_fence(
        &self,
        command_buffer: Box<BackendCommandBuffer>,
    ) -> Result<BackendObject> {
        autoreleasepool(|| {
            let fence = self
                .submit_internal(Self::owned_command_buffer(command_buffer), true)?
                .expect("asked for a fence");
            Ok(BackendObject(fence))
        })
    }

    fn cancel(&self, command_buffer: Box<BackendCommandBuffer>) -> Result<()> {
        autoreleasepool(|| self.cancel_internal(Self::owned_command_buffer(command_buffer)))
    }

    fn wait(&self) -> Result<()> {
        autoreleasepool(|| self.wait_internal())
    }

    fn wait_for_fences(&self, wait_all: bool, fences: &[&BackendObject]) -> Result<()> {
        let fences: Vec<Arc<MetalFence>> = fences.iter().map(|f| object::<MetalFence>(f)).collect();
        autoreleasepool(|| self.wait_for_fences_internal(wait_all, &fences));
        Ok(())
    }

    /// Translation of `METAL_QueryFence()`.
    fn query_fence(&self, fence: &BackendObject) -> bool {
        !Self::is_fence_busy(&object::<MetalFence>(fence))
    }

    fn release_fence(&self, fence: &BackendObject) {
        self.release_fence_internal(&object::<MetalFence>(fence));
    }

    // Feature Queries

    fn supports_texture_format(
        &self,
        format: TextureFormat,
        texture_type: TextureType,
        usage: TextureUsageFlags,
    ) -> bool {
        autoreleasepool(|| self.supports_texture_format_internal(format, texture_type, usage))
    }

    fn supports_sample_count(
        &self,
        _format: TextureFormat,
        desired_sample_count: SampleCount,
    ) -> bool {
        autoreleasepool(|| self.supports_sample_count_internal(desired_sample_count))
    }
}

// SAFETY: Metal devices, queues and resources may be used from any thread;
// the renderer's mutable state is behind its locks, as upstream's is.
unsafe impl Send for MetalRenderer {}
// SAFETY: as for Send.
unsafe impl Sync for MetalRenderer {}
