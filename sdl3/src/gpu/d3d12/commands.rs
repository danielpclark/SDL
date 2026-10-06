// Rust translation of the command buffer, barrier, fence, uniform buffer
// and submission parts of src/gpu/d3d12/SDL_gpu_d3d12.c from Simple
// DirectMedia Layer.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! Command buffers (a command allocator and a graphics command list each),
//! the resource barriers and tracking, the debug labels, fences, the
//! uniform data, the shader-visible descriptor heaps a command buffer
//! writes into, submission (with presentation and the cleanup of finished
//! command buffers), cancelling and waiting.
//!
//! A command buffer is a [`D3D12CommandBuffer`] value: the renderer's
//! available ones hand it out, the front end owns it while it is recorded,
//! the submitted list holds it while the GPU runs it, and cleaning it puts
//! it back with the available ones (upstream moves the same pointer between
//! these lists). The uniform buffers and descriptor heaps it uses are its
//! own until then; its slots name them by their index in its lists
//! (upstream's pointers into them).

use std::ffi::CString;
use std::sync::atomic::{AtomicI32, Ordering};
use std::sync::Arc;

use windows_sys::Win32::Foundation::{CloseHandle, HANDLE, WAIT_FAILED};
use windows_sys::Win32::System::Threading::{
    CreateEventW, WaitForMultipleObjects, WaitForSingleObject, INFINITE,
};

use super::d3d::*;
use super::descriptors::DescriptorHeap;
use super::pipelines::{D3D12ComputePipeline, D3D12GraphicsPipeline};
use super::resources::{
    align, calc_subresource_with_plane, default_buffer_resource_state,
    default_texture_resource_state, D3D12Buffer, D3D12Sampler, D3D12Texture, TextureSubresource,
    UniformBuffer,
};
use super::swapchain::WindowEntry;
use super::{lock, D3D12Renderer};
use crate::error::Result;
use crate::gpu::sysgpu::{
    MAX_COLOR_TARGET_BINDINGS, MAX_COMPUTE_WRITE_BUFFERS, MAX_COMPUTE_WRITE_TEXTURES,
    MAX_STORAGE_BUFFERS_PER_STAGE, MAX_STORAGE_TEXTURES_PER_STAGE, MAX_TEXTURE_SAMPLERS_PER_STAGE,
    MAX_UNIFORM_BUFFERS_PER_STAGE, MAX_VERTEX_BUFFERS, UNIFORM_BUFFER_SIZE,
};
use crate::gpu::{PresentMode, TextureUsageFlags};
use crate::log::Category;

/// `D3D12_FENCE_UNSIGNALED_VALUE`
const D3D12_FENCE_UNSIGNALED_VALUE: u64 = 0;
/// `D3D12_FENCE_SIGNAL_VALUE`
pub(super) const D3D12_FENCE_SIGNAL_VALUE: u64 = 1;

const SAMPLERS: usize = MAX_TEXTURE_SAMPLERS_PER_STAGE as usize;
const STORAGE_TEXTURES: usize = MAX_STORAGE_TEXTURES_PER_STAGE as usize;
const STORAGE_BUFFERS: usize = MAX_STORAGE_BUFFERS_PER_STAGE as usize;
const UNIFORM_BUFFERS: usize = MAX_UNIFORM_BUFFERS_PER_STAGE as usize;
const COLOR_TARGETS: usize = MAX_COLOR_TARGET_BINDINGS as usize;

/// The stage uniform data is pushed for (the shader stages of
/// `D3D12_INTERNAL_PushUniformData()`; `SDL_GPU_SHADERSTAGE_COMPUTE` is the
/// backend's own).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum UniformStage {
    Vertex,
    Fragment,
    Compute,
}

/// The event a fence's waits block on (`event`), closed when dropped.
#[derive(Debug)]
pub(super) struct FenceEvent(pub(super) HANDLE);

impl Drop for FenceEvent {
    fn drop(&mut self) {
        if !self.0.is_null() {
            // SAFETY: the event this owns.
            unsafe { CloseHandle(self.0) };
        }
    }
}

/// A fence, shared by the command buffer it signals, the swapchains that
/// wait on it and the application. Translation of `D3D12Fence`; its
/// destruction (`D3D12_INTERNAL_DestroyFence()`) releases the fence and
/// closes the event as fields.
#[derive(Debug)]
pub(super) struct D3D12Fence {
    pub(super) handle: D3d12Fence,
    /// used for blocking
    pub(super) event: FenceEvent,
    pub(super) reference_count: AtomicI32,
}

// SAFETY: Direct3D 12 fences are free-threaded, and so are event handles.
unsafe impl Send for D3D12Fence {}
// SAFETY: as for Send.
unsafe impl Sync for D3D12Fence {}

/// A subresource of a texture (upstream's `D3D12TextureSubresource *`,
/// whose `parent` is the texture): the texture and the subresource's index
/// in it.
#[derive(Clone, Debug)]
pub(super) struct SubresourceRef {
    pub(super) texture: Arc<D3D12Texture>,
    pub(super) index: usize,
}

impl SubresourceRef {
    /// The subresource.
    pub(super) fn get(&self) -> &TextureSubresource {
        &self.texture.subresources[self.index]
    }
}

/// A swapchain texture a command buffer presents. Translation of
/// `D3D12PresentData`.
#[derive(Debug)]
pub(super) struct PresentData {
    pub(super) window_data: Arc<WindowEntry>,
    pub(super) swapchain_image_index: u32,
}

/// The copy out of a download's temporary buffer once the command buffer
/// is done (for the texture pitch workaround). Translation of
/// `D3D12TextureDownload`.
#[derive(Debug)]
pub(super) struct TextureDownload {
    pub(super) destination_buffer: Arc<D3D12Buffer>,
    pub(super) temporary_buffer: Arc<D3D12Buffer>,
    #[allow(dead_code)] // (kept as upstream does)
    pub(super) width: u32,
    pub(super) height: u32,
    pub(super) depth: u32,
    pub(super) buffer_offset: u32,
    pub(super) bytes_per_row: u32,
    pub(super) bytes_per_depth_slice: u32,
    pub(super) aligned_bytes_per_row: u32,
}

/// Translation of `D3D12CommandBuffer` (its `renderer` is the renderer the
/// driver calls are made on).
pub(super) struct D3D12CommandBuffer {
    pub(super) command_allocator: D3d12CommandAllocator,
    pub(super) graphics_command_list: D3d12GraphicsCommandList,
    pub(super) in_flight_fence: Option<Arc<D3D12Fence>>,
    pub(super) auto_release_fence: bool,

    // Presentation data
    pub(super) present_datas: Vec<PresentData>,

    pub(super) color_target_subresources: [Option<SubresourceRef>; COLOR_TARGETS],
    pub(super) color_resolve_subresources: [Option<SubresourceRef>; COLOR_TARGETS],
    pub(super) depth_stencil_texture_subresource: Option<SubresourceRef>,
    pub(super) current_graphics_pipeline: Option<Arc<D3D12GraphicsPipeline>>,
    pub(super) current_compute_pipeline: Option<Arc<D3D12ComputePipeline>>,

    /// Set at acquire time (`gpuDescriptorHeaps`): the current heap of each
    /// type, by its index in `used_descriptor_heaps`.
    pub(super) gpu_descriptor_heaps: [Option<usize>; 2],

    pub(super) used_descriptor_heaps: Vec<DescriptorHeap>,

    pub(super) used_uniform_buffers: Vec<UniformBuffer>,

    // Resource slot state
    pub(super) need_vertex_buffer_bind: bool,
    pub(super) need_vertex_sampler_bind: bool,
    pub(super) need_vertex_storage_texture_bind: bool,
    pub(super) need_vertex_storage_buffer_bind: bool,
    pub(super) need_vertex_uniform_buffer_bind: [bool; UNIFORM_BUFFERS],
    pub(super) need_fragment_sampler_bind: bool,
    pub(super) need_fragment_storage_texture_bind: bool,
    pub(super) need_fragment_storage_buffer_bind: bool,
    pub(super) need_fragment_uniform_buffer_bind: [bool; UNIFORM_BUFFERS],

    pub(super) need_compute_sampler_bind: bool,
    pub(super) need_compute_read_only_storage_texture_bind: bool,
    pub(super) need_compute_read_only_storage_buffer_bind: bool,
    pub(super) need_compute_uniform_buffer_bind: [bool; UNIFORM_BUFFERS],

    pub(super) vertex_buffers: [Option<Arc<D3D12Buffer>>; MAX_VERTEX_BUFFERS as usize],
    pub(super) vertex_buffer_offsets: [u32; MAX_VERTEX_BUFFERS as usize],
    pub(super) vertex_buffer_count: u32,

    pub(super) vertex_sampler_texture_descriptor_handles: [CpuDescriptorHandle; SAMPLERS],
    pub(super) vertex_sampler_descriptor_handles: [CpuDescriptorHandle; SAMPLERS],
    pub(super) vertex_storage_texture_descriptor_handles: [CpuDescriptorHandle; STORAGE_TEXTURES],
    pub(super) vertex_storage_buffer_descriptor_handles: [CpuDescriptorHandle; STORAGE_BUFFERS],

    pub(super) vertex_uniform_buffers: [Option<usize>; UNIFORM_BUFFERS],

    pub(super) fragment_sampler_texture_descriptor_handles: [CpuDescriptorHandle; SAMPLERS],
    pub(super) fragment_sampler_descriptor_handles: [CpuDescriptorHandle; SAMPLERS],
    pub(super) fragment_storage_texture_descriptor_handles: [CpuDescriptorHandle; STORAGE_TEXTURES],
    pub(super) fragment_storage_buffer_descriptor_handles: [CpuDescriptorHandle; STORAGE_BUFFERS],

    pub(super) fragment_uniform_buffers: [Option<usize>; UNIFORM_BUFFERS],

    pub(super) compute_sampler_texture_descriptor_handles: [CpuDescriptorHandle; SAMPLERS],
    pub(super) compute_sampler_descriptor_handles: [CpuDescriptorHandle; SAMPLERS],
    pub(super) compute_read_only_storage_texture_descriptor_handles:
        [CpuDescriptorHandle; STORAGE_TEXTURES],
    pub(super) compute_read_only_storage_buffer_descriptor_handles:
        [CpuDescriptorHandle; STORAGE_BUFFERS],

    // Track these separately because barriers can happen mid compute pass
    pub(super) compute_read_only_storage_textures: [Option<Arc<D3D12Texture>>; STORAGE_TEXTURES],
    pub(super) compute_read_only_storage_buffers: [Option<Arc<D3D12Buffer>>; STORAGE_BUFFERS],

    pub(super) compute_read_write_storage_texture_descriptor_handles:
        [CpuDescriptorHandle; MAX_COMPUTE_WRITE_TEXTURES as usize],
    pub(super) compute_read_write_storage_buffer_descriptor_handles:
        [CpuDescriptorHandle; MAX_COMPUTE_WRITE_BUFFERS as usize],

    // Track these separately because they are bound when the compute pass begins
    pub(super) compute_read_write_storage_texture_subresources:
        [Option<SubresourceRef>; MAX_COMPUTE_WRITE_TEXTURES as usize],
    pub(super) compute_read_write_storage_texture_subresource_count: u32,
    pub(super) compute_read_write_storage_buffers:
        [Option<Arc<D3D12Buffer>>; MAX_COMPUTE_WRITE_BUFFERS as usize],
    pub(super) compute_read_write_storage_buffer_count: u32,

    pub(super) compute_uniform_buffers: [Option<usize>; UNIFORM_BUFFERS],

    // Resource tracking
    pub(super) used_textures: Vec<Arc<D3D12Texture>>,
    pub(super) used_buffers: Vec<Arc<D3D12Buffer>>,
    pub(super) used_samplers: Vec<Arc<D3D12Sampler>>,
    pub(super) used_graphics_pipelines: Vec<Arc<D3D12GraphicsPipeline>>,
    pub(super) used_compute_pipelines: Vec<Arc<D3D12ComputePipeline>>,

    /// Used for texture pitch hack
    pub(super) texture_downloads: Vec<TextureDownload>,
}

// SAFETY: a command list and its allocator are used by one thread at a
// time (the command buffer's owner), as upstream requires.
unsafe impl Send for D3D12CommandBuffer {}

impl std::fmt::Debug for D3D12CommandBuffer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("D3D12CommandBuffer")
            .field("graphics_command_list", &self.graphics_command_list)
            .field("auto_release_fence", &self.auto_release_fence)
            .finish_non_exhaustive()
    }
}

/// Whether `a` and `b` are the same object.
fn same<T>(a: &Arc<T>, b: &Arc<T>) -> bool {
    Arc::ptr_eq(a, b)
}

/// The tracking of `TRACK_RESOURCE`: a reference the command buffer holds
/// (once) until it is cleaned, counted in the object's `referenceCount`.
fn track<T>(used: &mut Vec<Arc<T>>, resource: &Arc<T>, reference_count: impl Fn(&T) -> &AtomicI32) {
    if used.iter().any(|r| same(r, resource)) {
        return;
    }
    used.push(resource.clone());
    reference_count(resource).fetch_add(1, Ordering::SeqCst);
}

impl D3D12CommandBuffer {
    /// A command buffer recording into `graphics_command_list`, tracking
    /// nothing: the state `D3D12_INTERNAL_AllocateCommandBuffer()` gives
    /// it.
    fn new(
        command_allocator: D3d12CommandAllocator,
        graphics_command_list: D3d12GraphicsCommandList,
    ) -> D3D12CommandBuffer {
        D3D12CommandBuffer {
            command_allocator,
            graphics_command_list,
            in_flight_fence: None,
            auto_release_fence: false,
            // Window handling
            present_datas: Vec::with_capacity(1),
            color_target_subresources: Default::default(),
            color_resolve_subresources: Default::default(),
            depth_stencil_texture_subresource: None,
            current_graphics_pipeline: None,
            current_compute_pipeline: None,
            gpu_descriptor_heaps: [None; 2],
            used_descriptor_heaps: Vec::with_capacity(4),
            used_uniform_buffers: Vec::with_capacity(4),
            need_vertex_buffer_bind: false,
            need_vertex_sampler_bind: false,
            need_vertex_storage_texture_bind: false,
            need_vertex_storage_buffer_bind: false,
            need_vertex_uniform_buffer_bind: [false; UNIFORM_BUFFERS],
            need_fragment_sampler_bind: false,
            need_fragment_storage_texture_bind: false,
            need_fragment_storage_buffer_bind: false,
            need_fragment_uniform_buffer_bind: [false; UNIFORM_BUFFERS],
            need_compute_sampler_bind: false,
            need_compute_read_only_storage_texture_bind: false,
            need_compute_read_only_storage_buffer_bind: false,
            need_compute_uniform_buffer_bind: [false; UNIFORM_BUFFERS],
            vertex_buffers: Default::default(),
            vertex_buffer_offsets: [0; MAX_VERTEX_BUFFERS as usize],
            vertex_buffer_count: 0,
            vertex_sampler_texture_descriptor_handles: Default::default(),
            vertex_sampler_descriptor_handles: Default::default(),
            vertex_storage_texture_descriptor_handles: Default::default(),
            vertex_storage_buffer_descriptor_handles: Default::default(),
            vertex_uniform_buffers: [None; UNIFORM_BUFFERS],
            fragment_sampler_texture_descriptor_handles: Default::default(),
            fragment_sampler_descriptor_handles: Default::default(),
            fragment_storage_texture_descriptor_handles: Default::default(),
            fragment_storage_buffer_descriptor_handles: Default::default(),
            fragment_uniform_buffers: [None; UNIFORM_BUFFERS],
            compute_sampler_texture_descriptor_handles: Default::default(),
            compute_sampler_descriptor_handles: Default::default(),
            compute_read_only_storage_texture_descriptor_handles: Default::default(),
            compute_read_only_storage_buffer_descriptor_handles: Default::default(),
            compute_read_only_storage_textures: Default::default(),
            compute_read_only_storage_buffers: Default::default(),
            compute_read_write_storage_texture_descriptor_handles: Default::default(),
            compute_read_write_storage_buffer_descriptor_handles: Default::default(),
            compute_read_write_storage_texture_subresources: Default::default(),
            compute_read_write_storage_texture_subresource_count: 0,
            compute_read_write_storage_buffers: Default::default(),
            compute_read_write_storage_buffer_count: 0,
            compute_uniform_buffers: [None; UNIFORM_BUFFERS],
            // Resource tracking
            used_textures: Vec::with_capacity(4),
            used_buffers: Vec::with_capacity(4),
            used_samplers: Vec::with_capacity(4),
            used_graphics_pipelines: Vec::with_capacity(4),
            used_compute_pipelines: Vec::with_capacity(4),
            texture_downloads: Vec::with_capacity(4),
        }
    }

    // Barriers

    /// Translation of `D3D12_INTERNAL_ResourceBarrier()`.
    pub(super) fn resource_barrier(
        &self,
        source_state: u32,
        destination_state: u32,
        resource: &D3d12Resource,
        subresource_index: u32,
        needs_uav_barrier: bool,
    ) {
        let mut barrier_desc: [ResourceBarrier; 2] = Default::default();
        let mut num_barriers = 0;
        let resource = resource.as_ptr().cast();

        // No transition barrier is needed if the state is not changing.
        if source_state != destination_state {
            barrier_desc[num_barriers] = ResourceBarrier {
                ty: D3D12_RESOURCE_BARRIER_TYPE_TRANSITION,
                flags: 0,
                u: ResourceBarrierUnion {
                    transition: ResourceTransitionBarrier {
                        resource,
                        subresource: subresource_index,
                        state_before: source_state,
                        state_after: destination_state,
                    },
                },
            };

            num_barriers += 1;
        }

        if needs_uav_barrier {
            barrier_desc[num_barriers] = ResourceBarrier {
                ty: D3D12_RESOURCE_BARRIER_TYPE_UAV,
                flags: 0,
                u: ResourceBarrierUnion {
                    uav: ResourceUavBarrier { resource },
                },
            };

            num_barriers += 1;
        }

        if num_barriers > 0 {
            self.graphics_command_list
                .resource_barrier(&barrier_desc[..num_barriers]);
        }
    }

    /// Translation of `D3D12_INTERNAL_TextureSubresourceBarrier()`.
    pub(super) fn texture_subresource_barrier(
        &self,
        source_state: u32,
        destination_state: u32,
        texture_subresource: &SubresourceRef,
    ) {
        let texture = &texture_subresource.texture;
        let subresource = texture_subresource.get();
        let info = &texture.info;
        let needs_uav_barrier = info.usage.intersects(
            TextureUsageFlags::COMPUTE_STORAGE_WRITE
                | TextureUsageFlags::COMPUTE_STORAGE_SIMULTANEOUS_READ_WRITE,
        );

        self.resource_barrier(
            source_state,
            destination_state,
            &texture.resource,
            subresource.index,
            needs_uav_barrier,
        );

        // D3D12 stores planar values on a separate subresource.
        // Since depth-stencil is our only supported planar format,
        // just force an extra transition if we're using a stencil format.
        if info.format.is_stencil_format() {
            let plane_subresource_index = calc_subresource_with_plane(
                subresource.level,
                subresource.layer,
                1,
                info.num_levels,
                info.layer_count_or_depth,
            );

            self.resource_barrier(
                source_state,
                destination_state,
                &texture.resource,
                plane_subresource_index,
                needs_uav_barrier,
            );
        }
    }

    /// Translation of
    /// `D3D12_INTERNAL_TextureSubresourceTransitionFromDefaultUsage()`.
    pub(super) fn texture_subresource_transition_from_default_usage(
        &self,
        destination_usage_mode: u32,
        texture_subresource: &SubresourceRef,
    ) {
        self.texture_subresource_barrier(
            default_texture_resource_state(texture_subresource.texture.info.usage),
            destination_usage_mode,
            texture_subresource,
        );
    }

    /// Translation of `D3D12_INTERNAL_TextureTransitionFromDefaultUsage()`.
    pub(super) fn texture_transition_from_default_usage(
        &self,
        destination_usage_mode: u32,
        texture: &Arc<D3D12Texture>,
    ) {
        for index in 0..texture.subresources.len() {
            self.texture_subresource_transition_from_default_usage(
                destination_usage_mode,
                &SubresourceRef {
                    texture: texture.clone(),
                    index,
                },
            );
        }
    }

    /// Translation of
    /// `D3D12_INTERNAL_TextureSubresourceTransitionToDefaultUsage()`.
    pub(super) fn texture_subresource_transition_to_default_usage(
        &self,
        source_usage_mode: u32,
        texture_subresource: &SubresourceRef,
    ) {
        self.texture_subresource_barrier(
            source_usage_mode,
            default_texture_resource_state(texture_subresource.texture.info.usage),
            texture_subresource,
        );
    }

    /// Translation of `D3D12_INTERNAL_TextureTransitionToDefaultUsage()`.
    pub(super) fn texture_transition_to_default_usage(
        &self,
        source_usage_mode: u32,
        texture: &Arc<D3D12Texture>,
    ) {
        for index in 0..texture.subresources.len() {
            self.texture_subresource_transition_to_default_usage(
                source_usage_mode,
                &SubresourceRef {
                    texture: texture.clone(),
                    index,
                },
            );
        }
    }

    /// Translation of `D3D12_INTERNAL_BufferBarrier()`.
    pub(super) fn buffer_barrier(
        &self,
        source_state: u32,
        destination_state: u32,
        buffer: &D3D12Buffer,
    ) {
        self.resource_barrier(
            if buffer.transitioned.load(Ordering::SeqCst) {
                source_state
            } else {
                D3D12_RESOURCE_STATE_COMMON
            },
            destination_state,
            &buffer.handle,
            0,
            buffer
                .usage
                .contains(crate::gpu::BufferUsageFlags::COMPUTE_STORAGE_WRITE),
        );

        buffer.transitioned.store(true, Ordering::SeqCst);
    }

    /// Translation of `D3D12_INTERNAL_BufferTransitionFromDefaultUsage()`.
    pub(super) fn buffer_transition_from_default_usage(
        &self,
        destination_state: u32,
        buffer: &D3D12Buffer,
    ) {
        self.buffer_barrier(
            default_buffer_resource_state(buffer.usage),
            destination_state,
            buffer,
        );
    }

    /// Translation of `D3D12_INTERNAL_BufferTransitionToDefaultUsage()`.
    pub(super) fn buffer_transition_to_default_usage(
        &self,
        source_state: u32,
        buffer: &D3D12Buffer,
    ) {
        self.buffer_barrier(
            source_state,
            default_buffer_resource_state(buffer.usage),
            buffer,
        );
    }

    // Resource tracking

    /// Translation of `D3D12_INTERNAL_TrackTexture()`.
    pub(super) fn track_texture(&mut self, texture: &Arc<D3D12Texture>) {
        track(&mut self.used_textures, texture, |t| &t.reference_count);
    }

    /// Translation of `D3D12_INTERNAL_TrackBuffer()`.
    pub(super) fn track_buffer(&mut self, buffer: &Arc<D3D12Buffer>) {
        track(&mut self.used_buffers, buffer, |b| &b.reference_count);
    }

    /// Translation of `D3D12_INTERNAL_TrackSampler()`.
    pub(super) fn track_sampler(&mut self, sampler: &Arc<D3D12Sampler>) {
        track(&mut self.used_samplers, sampler, |s| &s.reference_count);
    }

    /// Translation of `D3D12_INTERNAL_TrackGraphicsPipeline()`.
    pub(super) fn track_graphics_pipeline(
        &mut self,
        graphics_pipeline: &Arc<D3D12GraphicsPipeline>,
    ) {
        track(&mut self.used_graphics_pipelines, graphics_pipeline, |p| {
            &p.reference_count
        });
    }

    /// Translation of `D3D12_INTERNAL_TrackComputePipeline()`.
    pub(super) fn track_compute_pipeline(&mut self, compute_pipeline: &Arc<D3D12ComputePipeline>) {
        track(&mut self.used_compute_pipelines, compute_pipeline, |p| {
            &p.reference_count
        });
    }

    /// Translation of `D3D12_INTERNAL_TrackUniformBuffer()` (with the
    /// tracking of its buffer): its index in `used_uniform_buffers`.
    fn track_uniform_buffer(&mut self, uniform_buffer: UniformBuffer) -> usize {
        let buffer = uniform_buffer.buffer.clone();
        self.used_uniform_buffers.push(uniform_buffer);

        self.track_buffer(&buffer);

        self.used_uniform_buffers.len() - 1
    }

    /// The uniform buffer slots of a stage.
    fn uniform_buffers(&mut self, stage: UniformStage) -> &mut [Option<usize>; UNIFORM_BUFFERS] {
        match stage {
            UniformStage::Vertex => &mut self.vertex_uniform_buffers,
            UniformStage::Fragment => &mut self.fragment_uniform_buffers,
            UniformStage::Compute => &mut self.compute_uniform_buffers,
        }
    }

    /// The uniform buffer bound to a stage's slot.
    pub(super) fn uniform_buffer(
        &self,
        stage: UniformStage,
        slot: usize,
    ) -> Option<&UniformBuffer> {
        let index = match stage {
            UniformStage::Vertex => self.vertex_uniform_buffers[slot],
            UniformStage::Fragment => self.fragment_uniform_buffers[slot],
            UniformStage::Compute => self.compute_uniform_buffers[slot],
        }?;
        self.used_uniform_buffers.get(index)
    }

    /// The current shader-visible heap of a type.
    pub(super) fn gpu_descriptor_heap(&mut self, heap_type: u32) -> Option<&mut DescriptorHeap> {
        let index = self.gpu_descriptor_heaps[heap_type as usize]?;
        self.used_descriptor_heaps.get_mut(index)
    }
}

impl D3D12Renderer {
    /// The command buffer of the front end's.
    pub(super) fn d3d12_command_buffer(
        command_buffer: &mut crate::gpu::sysgpu::BackendCommandBuffer,
    ) -> &mut D3D12CommandBuffer {
        command_buffer
            .downcast_mut::<D3D12CommandBuffer>()
            .expect("not a Direct3D 12 command buffer")
    }

    /// A command buffer the front end gives back.
    pub(super) fn owned_command_buffer(
        command_buffer: Box<crate::gpu::sysgpu::BackendCommandBuffer>,
    ) -> D3D12CommandBuffer {
        *command_buffer
            .downcast::<D3D12CommandBuffer>()
            .expect("not a Direct3D 12 command buffer")
    }

    // Debug labels

    /// Call a function of the PIX runtime with the command list and a
    /// text: no-op without the runtime (or with a text that has a NUL).
    fn pix_event(
        &self,
        command_buffer: &D3D12CommandBuffer,
        f: Option<PfnBeginEventOnCommandList>,
        text: &str,
    ) {
        let (Some(f), Ok(text)) = (f, CString::new(text)) else {
            return;
        };
        // SAFETY: the PIX runtime's function, with a command list being
        // recorded and a NUL-terminated string.
        unsafe {
            f(
                command_buffer.graphics_command_list.as_raw(),
                0, /*default color*/
                text.as_ptr(),
            )
        };
    }

    /// These debug functions now require the PIX runtime under Windows to
    /// avoid validation layer errors. Calling them without the PIX runtime
    /// in your path is a no-op. Translation of `D3D12_InsertDebugLabel()`.
    pub(super) fn insert_debug_label_internal(
        &self,
        command_buffer: &D3D12CommandBuffer,
        text: &str,
    ) {
        // Requires PIX runtime under Windows, no-op if DLL unavailable.
        let fns = &self.winpixeventruntime_fns;
        self.pix_event(command_buffer, fns.set_marker_on_command_list, text);
    }

    /// Translation of `D3D12_PushDebugGroup()`.
    pub(super) fn push_debug_group_internal(
        &self,
        command_buffer: &D3D12CommandBuffer,
        name: &str,
    ) {
        // Requires PIX runtime under Windows, no-op if DLL unavailable.
        let fns = &self.winpixeventruntime_fns;
        self.pix_event(command_buffer, fns.begin_event_on_command_list, name);
    }

    /// Translation of `D3D12_PopDebugGroup()`.
    pub(super) fn pop_debug_group_internal(&self, command_buffer: &D3D12CommandBuffer) {
        // Requires PIX runtime under Windows, no-op if DLL unavailable.
        if let Some(f) = self.winpixeventruntime_fns.end_event_on_command_list {
            // SAFETY: the PIX runtime's function, with a command list being
            // recorded.
            unsafe { f(command_buffer.graphics_command_list.as_raw()) };
        }
    }

    // Fences

    /// Translation of `D3D12_INTERNAL_ReleaseFenceToPool()`.
    fn release_fence_to_pool(&self, fence: Arc<D3D12Fence>) {
        lock(&self.fence_pool).push(fence);
    }

    /// Drop a reference to a fence, which goes back to the pool with the
    /// last one. Translation of `D3D12_ReleaseFence()`.
    pub(super) fn release_fence_internal(&self, fence: &Arc<D3D12Fence>) {
        // (SDL_AtomicDecRef() is true when the count reaches zero)
        if fence.reference_count.fetch_sub(1, Ordering::SeqCst) == 1 {
            self.release_fence_to_pool(fence.clone());
        }
    }

    /// Whether a fence is signaled. Translation of `D3D12_QueryFence()`.
    pub(super) fn query_fence_internal(&self, fence: &D3D12Fence) -> bool {
        fence.handle.completed_value() == D3D12_FENCE_SIGNAL_VALUE
    }

    /// An unsignaled fence with one reference, from the pool or new.
    /// Translation of `D3D12_INTERNAL_AcquireFence()`.
    pub(super) fn acquire_fence(&self) -> Result<Arc<D3D12Fence>> {
        let mut pool = lock(&self.fence_pool);

        let fence = match pool.pop() {
            None => {
                let handle = self
                    .device
                    .create_fence(D3D12_FENCE_UNSIGNALED_VALUE, D3D12_FENCE_FLAG_NONE)
                    .map_err(|res| self.set_error("Failed to create fence!", res))?;

                // SAFETY: no security attributes or name: an auto-reset,
                // unsignaled event.
                let event = unsafe { CreateEventW(std::ptr::null(), 0, 0, std::ptr::null()) };
                Arc::new(D3D12Fence {
                    handle,
                    event: FenceEvent(event),
                    reference_count: AtomicI32::new(0),
                })
            }
            Some(fence) => {
                fence.handle.signal(D3D12_FENCE_UNSIGNALED_VALUE);
                fence
            }
        };

        drop(pool);

        fence.reference_count.fetch_add(1, Ordering::SeqCst);
        Ok(fence)
    }

    // Command buffers

    /// Make a command buffer (an allocator and a command list) for the
    /// available ones. Translation of `D3D12_INTERNAL_AllocateCommandBuffer()`
    /// (the caller holds `acquireCommandBufferLock`, the available ones'
    /// lock).
    fn allocate_command_buffer(&self, available: &mut Vec<D3D12CommandBuffer>) -> Result<()> {
        let command_allocator = self
            .device
            .create_command_allocator(D3D12_COMMAND_LIST_TYPE_DIRECT)
            .map_err(|res| self.set_error("Failed to create ID3D12CommandAllocator", res))?;

        let command_list = self
            .device
            .create_command_list(0, D3D12_COMMAND_LIST_TYPE_DIRECT, &command_allocator)
            .map_err(|res| self.set_error("Failed to create ID3D12CommandList", res))?;

        // Add to inactive command buffer array
        available.push(D3D12CommandBuffer::new(command_allocator, command_list));

        Ok(())
    }

    /// Translation of `D3D12_INTERNAL_AcquireCommandBufferFromPool()`.
    fn acquire_command_buffer_from_pool(
        &self,
        available: &mut Vec<D3D12CommandBuffer>,
    ) -> Result<D3D12CommandBuffer> {
        if available.is_empty() {
            self.allocate_command_buffer(available)?;
        }

        Ok(available
            .pop()
            .expect("a command buffer was just allocated"))
    }

    /// A command buffer being recorded. Translation of
    /// `D3D12_AcquireCommandBuffer()`.
    pub(super) fn acquire_command_buffer_internal(&self) -> Result<D3D12CommandBuffer> {
        let mut command_buffer = {
            let mut available = lock(&self.command_buffer_pool);
            self.acquire_command_buffer_from_pool(&mut available)?
        };

        // Set the bind state
        let cb = &mut command_buffer;
        cb.current_graphics_pipeline = None;

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
        cb.vertex_uniform_buffers = Default::default();

        cb.fragment_sampler_texture_descriptor_handles = Default::default();
        cb.fragment_sampler_descriptor_handles = Default::default();
        cb.fragment_storage_texture_descriptor_handles = Default::default();
        cb.fragment_storage_buffer_descriptor_handles = Default::default();
        cb.fragment_uniform_buffers = Default::default();

        cb.compute_sampler_texture_descriptor_handles = Default::default();
        cb.compute_sampler_descriptor_handles = Default::default();
        cb.compute_read_only_storage_texture_descriptor_handles = Default::default();
        cb.compute_read_only_storage_buffer_descriptor_handles = Default::default();
        cb.compute_read_only_storage_textures = Default::default();
        cb.compute_read_only_storage_buffers = Default::default();
        cb.compute_read_write_storage_texture_subresources = Default::default();
        cb.compute_read_write_storage_buffers = Default::default();
        cb.compute_uniform_buffers = Default::default();

        cb.auto_release_fence = true;

        Ok(command_buffer)
    }

    // Uniform data

    /// A uniform buffer for a command buffer, from the pool or new, tracked
    /// by it: its index in the command buffer's. Translation of
    /// `D3D12_INTERNAL_AcquireUniformBufferFromPool()`.
    ///
    /// Note (upstream): C goes on with a NULL uniform buffer (and crashes)
    /// when none can be made; `None` here, and nothing is written.
    pub(super) fn acquire_uniform_buffer(
        &self,
        command_buffer: &mut D3D12CommandBuffer,
    ) -> Option<usize> {
        let uniform_buffer = self.acquire_uniform_buffer_from_pool().ok()?;

        Some(command_buffer.track_uniform_buffer(uniform_buffer))
    }

    /// Copy uniform data into a stage's uniform buffer for the following
    /// draws or dispatches. Translation of `D3D12_INTERNAL_PushUniformData()`.
    pub(super) fn push_uniform_data(
        &self,
        command_buffer: &mut D3D12CommandBuffer,
        shader_stage: UniformStage,
        slot_index: u32,
        data: &[u8],
    ) {
        let slot = slot_index as usize;
        let length = data.len() as u32;

        // Note (upstream): C writes past the end of the uniform buffer when
        // the data is longer than one; such data is dropped here.
        if length > UNIFORM_BUFFER_SIZE {
            crate::log::error!(Category::Gpu, "Uniform data is too large!");
            return;
        }

        if command_buffer.uniform_buffers(shader_stage)[slot].is_none() {
            let index = self.acquire_uniform_buffer(command_buffer);
            command_buffer.uniform_buffers(shader_stage)[slot] = index;
        }
        let Some(mut index) = command_buffer.uniform_buffers(shader_stage)[slot] else {
            return;
        };

        let block_size = align(length, 256);

        // If there is no more room, acquire a new uniform buffer
        if command_buffer.used_uniform_buffers[index].write_offset + block_size
            >= UNIFORM_BUFFER_SIZE
        {
            let old = &command_buffer.used_uniform_buffers[index].buffer;
            old.handle.unmap();
            old.map_pointer
                .store(std::ptr::null_mut(), Ordering::SeqCst);

            let Some(new_index) = self.acquire_uniform_buffer(command_buffer) else {
                command_buffer.uniform_buffers(shader_stage)[slot] = None;
                return;
            };
            index = new_index;

            let uniform_buffer = &mut command_buffer.used_uniform_buffers[index];
            uniform_buffer.draw_offset = 0;
            uniform_buffer.write_offset = 0;

            command_buffer.uniform_buffers(shader_stage)[slot] = Some(index);
        }

        let uniform_buffer = &mut command_buffer.used_uniform_buffers[index];
        uniform_buffer.draw_offset = uniform_buffer.write_offset;

        let map_pointer = uniform_buffer.buffer.map_pointer.load(Ordering::SeqCst);
        if map_pointer.is_null() {
            crate::log::error!(Category::Gpu, "Uniform buffer memory is not mapped!");
            return;
        }
        // SAFETY: the uniform buffer is mapped (it was when acquired, and is
        // until the command buffer is submitted), UNIFORM_BUFFER_SIZE long,
        // and the data fits past the write offset (checked above).
        unsafe {
            std::ptr::copy_nonoverlapping(
                data.as_ptr(),
                map_pointer.add(uniform_buffer.write_offset as usize),
                data.len(),
            );
        }

        uniform_buffer.write_offset += block_size;

        match shader_stage {
            UniformStage::Vertex => command_buffer.need_vertex_uniform_buffer_bind[slot] = true,
            UniformStage::Fragment => command_buffer.need_fragment_uniform_buffer_bind[slot] = true,
            UniformStage::Compute => command_buffer.need_compute_uniform_buffer_bind[slot] = true,
        }
    }

    // Descriptor heaps

    /// A shader-visible heap from its pool, tracked by the command buffer:
    /// its index in the command buffer's. Translation of
    /// `D3D12_INTERNAL_AcquireGPUDescriptorHeapFromPool()` (with
    /// `D3D12_INTERNAL_TrackGPUDescriptorHeap()`: a heap from the pool is
    /// never one the command buffer has).
    fn acquire_gpu_descriptor_heap(
        &self,
        command_buffer: &mut D3D12CommandBuffer,
        descriptor_heap_type: u32,
    ) -> Result<usize> {
        let heap = self.acquire_gpu_descriptor_heap_from_pool(descriptor_heap_type)?;
        command_buffer.used_descriptor_heaps.push(heap);
        Ok(command_buffer.used_descriptor_heaps.len() - 1)
    }

    /// Acquire a shader-visible heap of each type and set them on the
    /// command list. Translation of `D3D12_INTERNAL_SetGPUDescriptorHeaps()`.
    ///
    /// Note (upstream): C goes on with a NULL heap (and crashes) when one
    /// can't be made; the command buffer keeps its heaps here, and the
    /// descriptors aren't written.
    pub(super) fn set_gpu_descriptor_heaps(
        &self,
        command_buffer: &mut D3D12CommandBuffer,
    ) -> Result<()> {
        let view_heap = self
            .acquire_gpu_descriptor_heap(command_buffer, D3D12_DESCRIPTOR_HEAP_TYPE_CBV_SRV_UAV)?;
        let sampler_heap =
            self.acquire_gpu_descriptor_heap(command_buffer, D3D12_DESCRIPTOR_HEAP_TYPE_SAMPLER)?;

        command_buffer.gpu_descriptor_heaps[0] = Some(view_heap);
        command_buffer.gpu_descriptor_heaps[1] = Some(sampler_heap);

        let heaps = [
            &command_buffer.used_descriptor_heaps[view_heap].handle,
            &command_buffer.used_descriptor_heaps[sampler_heap].handle,
        ];

        command_buffer
            .graphics_command_list
            .set_descriptor_heaps(&heaps);
        Ok(())
    }

    /// Copy descriptors into the current shader-visible heap of their
    /// type: the GPU descriptor of the first. Translation of
    /// `D3D12_INTERNAL_WriteGPUDescriptors()`.
    pub(super) fn write_gpu_descriptors(
        &self,
        command_buffer: &mut D3D12CommandBuffer,
        heap_type: u32,
        resource_descriptor_handles: &[CpuDescriptorHandle],
    ) -> Option<GpuDescriptorHandle> {
        /* Descriptor overflow, acquire new heaps */
        let overflow = command_buffer
            .gpu_descriptor_heap(heap_type)
            .is_none_or(|heap| heap.current_descriptor_index >= heap.max_descriptors);
        if overflow && self.set_gpu_descriptor_heaps(command_buffer).is_err() {
            return None;
        }

        let heap = command_buffer.gpu_descriptor_heap(heap_type)?;

        // FIXME: need to error on overflow
        let mut gpu_heap_cpu_handle = CpuDescriptorHandle {
            ptr: heap.descriptor_heap_cpu_start.ptr
                + (heap.current_descriptor_index * heap.descriptor_size) as usize,
        };
        let gpu_base_descriptor = GpuDescriptorHandle {
            ptr: heap.descriptor_heap_gpu_start.ptr
                + (heap.current_descriptor_index * heap.descriptor_size) as u64,
        };

        for handle in resource_descriptor_handles {
            // This will crash the driver if it gets a null handle! Cool!
            if handle.ptr != 0 {
                // SAFETY: a slot of the shader-visible heap (FIXME above:
                // upstream doesn't check it's in range) and a staging
                // descriptor of a live resource, of the heap's type.
                unsafe {
                    self.device
                        .copy_descriptors_simple(1, gpu_heap_cpu_handle, *handle, heap_type)
                };

                heap.current_descriptor_index += 1;
                gpu_heap_cpu_handle.ptr += heap.descriptor_size as usize;
            }
        }

        Some(gpu_base_descriptor)
    }

    // Submission

    /// Copy a download's temporary buffer into its transfer buffer at the
    /// pitches asked for. Translation of `D3D12_INTERNAL_CopyTextureDownload()`.
    ///
    /// FIXME (upstream): a depth slice starts `sliceIndex * height` bytes
    /// into the temporary buffer, not `sliceIndex * height *
    /// alignedBytesPerRow`, so the slices of a 3D download past the first
    /// are read from the wrong place.
    ///
    /// Note (upstream): C copies past the ends of the buffers when the
    /// download doesn't fit; the rows that don't fit are skipped here.
    fn copy_texture_download(&self, download: &TextureDownload) -> Result<()> {
        let source_ptr = download
            .temporary_buffer
            .handle
            .map()
            .map_err(|res| self.set_error("Failed to map temporary buffer", res))?;

        // Note (upstream): C leaves the temporary buffer mapped when mapping
        // the destination fails.
        let dest_ptr = match download.destination_buffer.handle.map() {
            Ok(ptr) => ptr,
            Err(res) => return Err(self.set_error("Failed to map destination buffer", res)),
        };

        let source_size = download.temporary_buffer.size as usize;
        let dest_size = download.destination_buffer.size as usize;
        for slice_index in 0..download.depth as usize {
            for row_index in 0..download.height as usize {
                let dst = download.buffer_offset as usize
                    + (slice_index * download.bytes_per_depth_slice as usize)
                    + (row_index * download.bytes_per_row as usize);
                let src = (slice_index * download.height as usize)
                    + (row_index * download.aligned_bytes_per_row as usize);
                let len = download.bytes_per_row as usize;
                if dst + len > dest_size || src + len > source_size {
                    continue;
                }
                // SAFETY: both buffers are mapped, of their sizes, and the
                // row is in both (checked above).
                unsafe {
                    std::ptr::copy_nonoverlapping(source_ptr.add(src), dest_ptr.add(dst), len)
                };
            }
        }

        download.temporary_buffer.handle.unmap();

        download.destination_buffer.handle.unmap();

        Ok(())
    }

    /// Give back what a finished (or cancelled) command buffer holds and
    /// return it to the available ones. Translation of
    /// `D3D12_INTERNAL_CleanCommandBuffer()`; the caller holds `submitLock`
    /// (whose data is `submitted`) and has taken the command buffer out of
    /// the submitted list. When it fails before the command buffer is
    /// reset, a submitted one goes back to the submitted list, where
    /// upstream leaves it (a cancelled one is dropped: upstream loses it).
    pub(super) fn clean_command_buffer(
        &self,
        submitted: &mut Vec<D3D12CommandBuffer>,
        mut command_buffer: D3D12CommandBuffer,
        cancel: bool,
    ) -> Result<()> {
        let mut result = Ok(());

        // Perform deferred texture data copies
        for download in std::mem::take(&mut command_buffer.texture_downloads) {
            if !cancel {
                if let Err(e) = self.copy_texture_download(&download) {
                    result = result.and(Err(e));
                }
            }
        }

        let reset = || -> Result<()> {
            let res = command_buffer.command_allocator.reset();
            if res < 0 {
                return Err(self.set_error("Could not reset command allocator", res));
            }

            let res = command_buffer
                .graphics_command_list
                .reset(&command_buffer.command_allocator);
            if res < 0 {
                return Err(self.set_error("Could not reset command list", res));
            }
            Ok(())
        };
        if let Err(e) = result.and_then(|()| reset()) {
            if !cancel {
                submitted.push(command_buffer);
            }
            return Err(e);
        }

        // Return descriptor heaps to pool, pools own their own locks
        for heap in command_buffer.used_descriptor_heaps.drain(..) {
            self.return_gpu_descriptor_heap_to_pool(heap);
        }

        command_buffer.gpu_descriptor_heaps = [None; 2];

        // Uniform buffers are now available
        {
            let mut pool = lock(&self.uniform_buffer_pool);

            for uniform_buffer in command_buffer.used_uniform_buffers.drain(..) {
                Self::return_uniform_buffer_to_pool(&mut pool, uniform_buffer);
            }
        }
        command_buffer.vertex_uniform_buffers = Default::default();
        command_buffer.fragment_uniform_buffers = Default::default();
        command_buffer.compute_uniform_buffers = Default::default();

        // TODO: More reference counting

        for texture in command_buffer.used_textures.drain(..) {
            texture.reference_count.fetch_sub(1, Ordering::SeqCst);
        }

        for buffer in command_buffer.used_buffers.drain(..) {
            buffer.reference_count.fetch_sub(1, Ordering::SeqCst);
        }

        for sampler in command_buffer.used_samplers.drain(..) {
            sampler.reference_count.fetch_sub(1, Ordering::SeqCst);
        }

        for pipeline in command_buffer.used_graphics_pipelines.drain(..) {
            pipeline.reference_count.fetch_sub(1, Ordering::SeqCst);
        }

        for pipeline in command_buffer.used_compute_pipelines.drain(..) {
            pipeline.reference_count.fetch_sub(1, Ordering::SeqCst);
        }

        // (the bindings of the last pass hold references too; they are
        // reset when the command buffer is acquired again)
        command_buffer.vertex_buffers = Default::default();
        command_buffer.compute_read_only_storage_textures = Default::default();
        command_buffer.compute_read_only_storage_buffers = Default::default();
        command_buffer.compute_read_write_storage_texture_subresources = Default::default();
        command_buffer.compute_read_write_storage_buffers = Default::default();
        command_buffer.current_graphics_pipeline = None;
        command_buffer.current_compute_pipeline = None;

        // Reset presentation
        command_buffer.present_datas.clear();

        // The fence is now available (unless SubmitAndAcquireFence was called)
        if command_buffer.auto_release_fence {
            if let Some(fence) = &command_buffer.in_flight_fence {
                self.release_fence_internal(fence);
            }
        }
        // (the application holds a fence it acquired)
        command_buffer.in_flight_fence = None;

        // Return command buffer to pool
        lock(&self.command_buffer_pool).push(command_buffer);

        // (the caller removed it from the submitted list)

        Ok(())
    }

    /// Clean the submitted command buffers whose fence is signaled (from
    /// the most recent one, as upstream's loops): the first failure, after
    /// trying them all. The caller holds `submitLock`, whose data is the
    /// list.
    fn clean_finished_command_buffers(
        &self,
        submitted: &mut Vec<D3D12CommandBuffer>,
    ) -> Result<()> {
        let mut result = Ok(());

        for i in (0..submitted.len()).rev() {
            let fence_value = submitted[i]
                .in_flight_fence
                .as_ref()
                .map_or(D3D12_FENCE_SIGNAL_VALUE, |f| f.handle.completed_value());

            if fence_value == D3D12_FENCE_SIGNAL_VALUE {
                let command_buffer = submitted.swap_remove(i);
                if let Err(e) = self.clean_command_buffer(submitted, command_buffer, false) {
                    result = result.and(Err(e));
                }
            }
        }

        result
    }

    /// Submit a command buffer and present the swapchain textures it
    /// acquired: its fence. Translation of `D3D12_INTERNAL_Submit()` (the
    /// fence is `SubmitAndAcquireFence()`'s when the command buffer's
    /// `auto_release_fence` is false).
    ///
    /// FIXME (upstream): when closing the command list, converting it,
    /// acquiring the fence or enqueuing its signal fails, the command
    /// buffer is lost (and the resources it tracks are never destroyed).
    ///
    /// Note (upstream): C returns with `submitLock` held when closing the
    /// command list or enqueuing the fence signal fails; the lock's guard
    /// lets it go here.
    pub(super) fn submit_internal(
        &self,
        mut command_buffer: D3D12CommandBuffer,
    ) -> Result<Arc<D3D12Fence>> {
        let mut submitted = lock(&self.submit_lock);

        // Unmap uniform buffers
        for i in 0..UNIFORM_BUFFERS {
            for stage in [UniformStage::Vertex, UniformStage::Fragment] {
                if let Some(uniform_buffer) = command_buffer.uniform_buffer(stage, i) {
                    uniform_buffer.buffer.handle.unmap();
                    uniform_buffer
                        .buffer
                        .map_pointer
                        .store(std::ptr::null_mut(), Ordering::SeqCst);
                }
            }

            // TODO: compute uniforms
        }

        // Transition present textures to present mode
        for present_data in &command_buffer.present_datas {
            let swapchain_index = present_data.swapchain_image_index;
            let Some(container) = present_data.window_data.texture_container(swapchain_index)
            else {
                continue;
            };
            let subresource = Self::fetch_texture_subresource(&container, 0, 0);

            let barrier_desc = ResourceBarrier {
                ty: D3D12_RESOURCE_BARRIER_TYPE_TRANSITION,
                flags: D3D12_RESOURCE_BARRIER_FLAG_NONE,
                u: ResourceBarrierUnion {
                    transition: ResourceTransitionBarrier {
                        resource: subresource.texture.resource.as_ptr().cast(),
                        subresource: subresource.get().index,
                        state_before: D3D12_RESOURCE_STATE_RENDER_TARGET,
                        state_after: D3D12_RESOURCE_STATE_PRESENT,
                    },
                },
            };

            command_buffer
                .graphics_command_list
                .resource_barrier(&[barrier_desc]);
        }

        // Notify the command buffer that we have completed recording
        let res = command_buffer.graphics_command_list.close();
        self.drain_info_queue_messages();
        if res < 0 {
            return Err(self.set_error("Failed to close command list!", res));
        }

        let command_list = command_buffer
            .graphics_command_list
            .command_list()
            .map_err(|res| self.set_error("Failed to convert command list!", res))?;

        // Submit the command list to the queue
        self.command_queue.execute_command_list(&command_list);

        drop(command_list);

        // Acquire a fence and set it to the in-flight fence
        let in_flight_fence = self.acquire_fence()?;
        command_buffer.in_flight_fence = Some(in_flight_fence.clone());

        // Return the fence while submitLock is held, another thread could
        // recycle this command buffer as soon as the lock is released.
        // (the returned reference is the caller's)

        // Mark that a fence should be signaled after command list execution
        let res = self
            .command_queue
            .signal(&in_flight_fence.handle, D3D12_FENCE_SIGNAL_VALUE);
        if res < 0 {
            return Err(self.set_error("Failed to enqueue fence signal!", res));
        }

        let mut result = Ok(());

        // Present, if applicable
        let present_datas = std::mem::take(&mut command_buffer.present_datas);
        for present_data in &present_datas {
            let mut window_data = lock(&present_data.window_data.data);

            // NOTE: flip discard always supported since DXGI 1.4 is required
            let mut sync_interval = 1;
            if window_data.present_mode == PresentMode::Immediate
                || window_data.present_mode == PresentMode::Mailbox
            {
                sync_interval = 0;
            }

            let mut present_flags = 0;
            if self.supports_tearing && window_data.present_mode == PresentMode::Immediate {
                present_flags = DXGI_PRESENT_ALLOW_TEARING;
            }

            if let Some(swapchain) = &window_data.swapchain {
                let res = swapchain.present(sync_interval, present_flags);
                if res < 0 {
                    result = Err(self.set_error("Failed to present swapchain!", res));
                }
            }

            // (upstream releases the swapchain buffer it got when acquiring
            // here; the texture keeps its reference, see swapchain.rs)

            let frame_counter = window_data.frame_counter as usize;
            window_data.in_flight_fences[frame_counter] = Some(in_flight_fence.clone());
            in_flight_fence
                .reference_count
                .fetch_add(1, Ordering::SeqCst);

            // Normally this is '% allowedFramesInFlight', but the value gets clamped
            // at swapchain creation time, so use swapchainTextureCount instead
            window_data.frame_counter =
                (window_data.frame_counter + 1) % window_data.swapchain_texture_count.max(1);
        }
        command_buffer.present_datas = present_datas;

        // Mark the command buffer as submitted
        submitted.push(command_buffer);

        // Check for cleanups
        if let Err(e) = self.clean_finished_command_buffers(&mut submitted) {
            result = result.and(Err(e));
        }

        self.perform_pending_destroys();

        drop(submitted);

        result.map(|()| in_flight_fence)
    }

    /// Throw away a command buffer's commands. Translation of
    /// `D3D12_Cancel()`.
    ///
    /// Note (upstream): C loses the command buffer when closing its command
    /// list fails; it is dropped here.
    pub(super) fn cancel_internal(&self, mut command_buffer: D3D12CommandBuffer) -> Result<()> {
        // Notify the command buffer that we have completed recording
        let res = command_buffer.graphics_command_list.close();
        self.drain_info_queue_messages();
        if res < 0 {
            return Err(self.set_error("Failed to close command list!", res));
        }

        command_buffer.auto_release_fence = false;
        let mut submitted = lock(&self.submit_lock);
        self.clean_command_buffer(&mut submitted, command_buffer, true)
    }

    /// Wait for the GPU to finish everything submitted, then clean every
    /// submitted command buffer and destroy what was released. Translation
    /// of `D3D12_Wait()`.
    ///
    /// Note (upstream): C returns with `submitLock` held (and the fence
    /// lost) when setting the fence's event or the wait fails; the guard
    /// and the fence go here.
    pub(super) fn wait_internal(&self) -> Result<()> {
        let fence = self.acquire_fence()?;

        let mut submitted = lock(&self.submit_lock);

        let waited = (|| -> Result<()> {
            // Insert a signal into the end of the command queue...
            self.command_queue
                .signal(&fence.handle, D3D12_FENCE_SIGNAL_VALUE);

            // ...and then block on it.
            if fence.handle.completed_value() != D3D12_FENCE_SIGNAL_VALUE {
                let res = fence
                    .handle
                    .set_event_on_completion(D3D12_FENCE_SIGNAL_VALUE, fence.event.0);
                if res < 0 {
                    return Err(self.set_error("Setting fence event failed", res));
                }

                // SAFETY: the fence's event.
                let wait_result = unsafe { WaitForSingleObject(fence.event.0, INFINITE) };
                if wait_result == WAIT_FAILED {
                    return Err(self.set_string_error("Wait failed")); // TODO: is there a better way to report this?
                }
            }
            Ok(())
        })();

        self.release_fence_internal(&fence);
        waited?;

        let mut result = Ok(());

        // Clean up
        for i in (0..submitted.len()).rev() {
            if i >= submitted.len() {
                continue;
            }
            let command_buffer = submitted.swap_remove(i);
            if let Err(e) = self.clean_command_buffer(&mut submitted, command_buffer, false) {
                result = result.and(Err(e));
            }
        }

        self.perform_pending_destroys();

        result
    }

    /// Wait for fences (all of them, or any), then clean the command
    /// buffers that are done. Translation of `D3D12_WaitForFences()`.
    ///
    /// Note (upstream): C returns with `submitLock` held when setting a
    /// fence's event or the wait fails; the guard lets it go here.
    pub(super) fn wait_for_fences_internal(
        &self,
        wait_all: bool,
        fences: &[&D3D12Fence],
    ) -> Result<()> {
        let mut submitted = lock(&self.submit_lock);

        let mut events: Vec<HANDLE> = Vec::with_capacity(fences.len());
        for fence in fences {
            let res = fence
                .handle
                .set_event_on_completion(D3D12_FENCE_SIGNAL_VALUE, fence.event.0);
            if res < 0 {
                return Err(self.set_error("Setting fence event failed", res));
            }

            events.push(fence.event.0);
        }

        // SAFETY: the fences' events.
        let wait_result = unsafe {
            WaitForMultipleObjects(
                events.len() as u32,
                events.as_ptr(),
                wait_all as i32,
                INFINITE,
            )
        };

        if wait_result == WAIT_FAILED {
            return Err(self.set_string_error("Wait failed")); // TODO: is there a better way to report this?
        }

        // Check for cleanups
        let result = self.clean_finished_command_buffers(&mut submitted);

        self.perform_pending_destroys();

        result
    }
}
