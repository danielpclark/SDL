// Rust translation of the command buffer, fence, uniform buffer and
// submission parts of src/gpu/metal/SDL_gpu_metal.m from Simple
// DirectMedia Layer.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! Command buffers and their pool, resource tracking, fences and their
//! pool, uniform buffers and their pool and the uniform data, the debug
//! labels, submission (with presentation and the cleanup of finished
//! command buffers), cancelling and waiting.
//!
//! A command buffer is a [`MetalCommandBuffer`] value: the renderer's
//! available ones hand it out, the front end owns it while it is recorded,
//! the submitted list holds it while the GPU runs it, and cleaning it puts
//! it back with the available ones (upstream moves the same pointer between
//! these lists). The uniform buffers it uses are its own until then; its
//! slots name them by their index in its list (upstream's pointers).

use std::sync::atomic::{AtomicI32, Ordering};
use std::sync::{Arc, Mutex};

use super::pipelines::{MetalComputePipeline, MetalGraphicsPipeline};
use super::resources::{next_highest_alignment, MetalBuffer, MetalTexture, MetalUniformBuffer};
use super::swapchain::WindowEntry;
use super::sys::*;
use super::{lock, MetalRenderer};
use crate::core::apple::foundation::ns_string;
use crate::core::apple::objc::{msg_send, Id, NSUInteger, StrongPtr};
use crate::error::Result;
use crate::gpu::sysgpu::{
    MAX_COMPUTE_WRITE_BUFFERS, MAX_COMPUTE_WRITE_TEXTURES, MAX_STORAGE_BUFFERS_PER_STAGE,
    MAX_STORAGE_TEXTURES_PER_STAGE, MAX_TEXTURE_SAMPLERS_PER_STAGE, MAX_UNIFORM_BUFFERS_PER_STAGE,
    MAX_VERTEX_BUFFERS, UNIFORM_BUFFER_SIZE,
};
use crate::gpu::IndexElementSize;
use crate::log::Category;

pub(super) const SAMPLERS: usize = MAX_TEXTURE_SAMPLERS_PER_STAGE as usize;
pub(super) const STORAGE_TEXTURES: usize = MAX_STORAGE_TEXTURES_PER_STAGE as usize;
pub(super) const STORAGE_BUFFERS: usize = MAX_STORAGE_BUFFERS_PER_STAGE as usize;
pub(super) const UNIFORM_BUFFERS: usize = MAX_UNIFORM_BUFFERS_PER_STAGE as usize;
pub(super) const VERTEX_BUFFERS: usize = MAX_VERTEX_BUFFERS as usize;
pub(super) const COMPUTE_WRITE_TEXTURES: usize = MAX_COMPUTE_WRITE_TEXTURES as usize;
pub(super) const COMPUTE_WRITE_BUFFERS: usize = MAX_COMPUTE_WRITE_BUFFERS as usize;

/// The stage uniform data is pushed for (`SDL_GPUShaderStage` with the
/// backend's own `SDL_GPU_SHADERSTAGE_COMPUTE`).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum UniformStage {
    Vertex,
    Fragment,
    Compute,
}

/// A fence: the command buffer it was acquired for, shared by that command
/// buffer, the swapchains that wait on it and the application. Translation
/// of `MetalFence`.
#[derive(Debug)]
pub(super) struct MetalFence {
    pub(super) command_buffer: Mutex<StrongPtr>,
    pub(super) reference_count: AtomicI32,
}

/// Translation of `MetalCommandBuffer` (its `renderer` is the renderer the
/// driver calls are made on, and its `common` header is the front end's).
#[derive(Debug)]
pub(super) struct MetalCommandBuffer {
    // Native Handle
    pub(super) handle: StrongPtr,

    // Presentation
    pub(super) window_datas: Vec<Arc<WindowEntry>>,

    // Render Pass
    pub(super) render_encoder: StrongPtr,
    pub(super) graphics_pipeline: Option<Arc<MetalGraphicsPipeline>>,
    pub(super) index_buffer: Option<Arc<MetalBuffer>>,
    pub(super) index_buffer_offset: u32,
    pub(super) index_element_size: IndexElementSize,

    // Copy Pass
    pub(super) blit_encoder: StrongPtr,

    // Compute Pass
    pub(super) compute_encoder: StrongPtr,
    pub(super) compute_pipeline: Option<Arc<MetalComputePipeline>>,

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

    pub(super) vertex_buffers: [StrongPtr; VERTEX_BUFFERS],
    pub(super) vertex_buffer_offsets: [u32; VERTEX_BUFFERS],
    pub(super) vertex_buffer_count: u32,

    pub(super) vertex_samplers: [StrongPtr; SAMPLERS],
    pub(super) vertex_textures: [StrongPtr; SAMPLERS],
    pub(super) vertex_storage_textures: [StrongPtr; STORAGE_TEXTURES],
    pub(super) vertex_storage_buffers: [StrongPtr; STORAGE_BUFFERS],
    /// The uniform buffers, by their index in `used_uniform_buffers`.
    pub(super) vertex_uniform_buffers: [Option<usize>; UNIFORM_BUFFERS],

    pub(super) fragment_samplers: [StrongPtr; SAMPLERS],
    pub(super) fragment_textures: [StrongPtr; SAMPLERS],
    pub(super) fragment_storage_textures: [StrongPtr; STORAGE_TEXTURES],
    pub(super) fragment_storage_buffers: [StrongPtr; STORAGE_BUFFERS],
    pub(super) fragment_uniform_buffers: [Option<usize>; UNIFORM_BUFFERS],

    pub(super) compute_sampler_textures: [StrongPtr; SAMPLERS],
    pub(super) compute_samplers: [StrongPtr; SAMPLERS],
    pub(super) compute_read_only_textures: [StrongPtr; STORAGE_TEXTURES],
    pub(super) compute_read_only_buffers: [StrongPtr; STORAGE_BUFFERS],
    pub(super) compute_read_write_textures: [StrongPtr; COMPUTE_WRITE_TEXTURES],
    pub(super) compute_read_write_buffers: [StrongPtr; COMPUTE_WRITE_BUFFERS],
    pub(super) compute_uniform_buffers: [Option<usize>; UNIFORM_BUFFERS],

    pub(super) used_uniform_buffers: Vec<MetalUniformBuffer>,

    // Fences
    pub(super) fence: Option<Arc<MetalFence>>,

    // Reference Counting
    pub(super) used_buffers: Vec<Arc<MetalBuffer>>,
    pub(super) used_textures: Vec<Arc<MetalTexture>>,
}

/// The tracking of `TRACK_RESOURCE`: a reference the command buffer holds
/// (once) until it is cleaned, counted in the object's `referenceCount`.
fn track<T>(used: &mut Vec<Arc<T>>, resource: &Arc<T>, reference_count: impl Fn(&T) -> &AtomicI32) {
    if used.iter().any(|r| Arc::ptr_eq(r, resource)) {
        return;
    }
    used.push(resource.clone());
    reference_count(resource).fetch_add(1, Ordering::SeqCst);
}

/// The handles of a slot array, as the C arrays of objects Metal's range
/// setters take.
pub(super) fn ids<const N: usize>(handles: &[StrongPtr; N]) -> [Id; N] {
    std::array::from_fn(|i| handles[i].get())
}

impl MetalCommandBuffer {
    /// A command buffer without a native handle, tracking nothing: the
    /// state `METAL_INTERNAL_AllocateCommandBuffers()` gives it (the native
    /// Metal command buffer is created in `METAL_AcquireCommandBuffer`).
    fn new() -> MetalCommandBuffer {
        MetalCommandBuffer {
            handle: StrongPtr::nil(),
            window_datas: Vec::with_capacity(1),
            render_encoder: StrongPtr::nil(),
            graphics_pipeline: None,
            index_buffer: None,
            index_buffer_offset: 0,
            index_element_size: IndexElementSize::Bits16,
            blit_encoder: StrongPtr::nil(),
            compute_encoder: StrongPtr::nil(),
            compute_pipeline: None,
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
            vertex_buffer_offsets: [0; VERTEX_BUFFERS],
            vertex_buffer_count: 0,
            vertex_samplers: Default::default(),
            vertex_textures: Default::default(),
            vertex_storage_textures: Default::default(),
            vertex_storage_buffers: Default::default(),
            vertex_uniform_buffers: [None; UNIFORM_BUFFERS],
            fragment_samplers: Default::default(),
            fragment_textures: Default::default(),
            fragment_storage_textures: Default::default(),
            fragment_storage_buffers: Default::default(),
            fragment_uniform_buffers: [None; UNIFORM_BUFFERS],
            compute_sampler_textures: Default::default(),
            compute_samplers: Default::default(),
            compute_read_only_textures: Default::default(),
            compute_read_only_buffers: Default::default(),
            compute_read_write_textures: Default::default(),
            compute_read_write_buffers: Default::default(),
            compute_uniform_buffers: [None; UNIFORM_BUFFERS],
            used_uniform_buffers: Vec::new(),
            fence: None,
            // Reference Counting
            used_buffers: Vec::with_capacity(4),
            used_textures: Vec::with_capacity(4),
        }
    }

    // Resource tracking

    /// Translation of `METAL_INTERNAL_TrackBuffer()`.
    pub(super) fn track_buffer(&mut self, buffer: &Arc<MetalBuffer>) {
        track(&mut self.used_buffers, buffer, |b| &b.reference_count);
    }

    /// Translation of `METAL_INTERNAL_TrackTexture()`.
    pub(super) fn track_texture(&mut self, texture: &Arc<MetalTexture>) {
        track(&mut self.used_textures, texture, |t| &t.reference_count);
    }

    /// Translation of `METAL_INTERNAL_TrackUniformBuffer()`: the uniform
    /// buffer joins the command buffer's own, and its index there is
    /// returned (upstream's pointer).
    fn track_uniform_buffer(&mut self, uniform_buffer: MetalUniformBuffer) -> usize {
        self.used_uniform_buffers.push(uniform_buffer);
        self.used_uniform_buffers.len() - 1
    }

    /// The uniform buffer slots of a stage.
    pub(super) fn uniform_slots(
        &mut self,
        stage: UniformStage,
    ) -> &mut [Option<usize>; UNIFORM_BUFFERS] {
        match stage {
            UniformStage::Vertex => &mut self.vertex_uniform_buffers,
            UniformStage::Fragment => &mut self.fragment_uniform_buffers,
            UniformStage::Compute => &mut self.compute_uniform_buffers,
        }
    }

    /// The encoder recording now, if any (render, then blit, then compute,
    /// as upstream checks them).
    fn current_encoder(&self) -> Option<Id> {
        if !self.render_encoder.is_nil() {
            Some(self.render_encoder.get())
        } else if !self.blit_encoder.is_nil() {
            Some(self.blit_encoder.get())
        } else if !self.compute_encoder.is_nil() {
            Some(self.compute_encoder.get())
        } else {
            None
        }
    }
}

impl MetalRenderer {
    // Debug Naming

    /// Translation of `METAL_InsertDebugLabel()`.
    pub(super) fn insert_debug_label_internal(command_buffer: &MetalCommandBuffer, text: &str) {
        let label = ns_string(text);
        // SAFETY: the encoder's or command buffer's debug methods.
        unsafe {
            if let Some(encoder) = command_buffer.current_encoder() {
                let _: () = msg_send![encoder, insertDebugSignpost: label.get()];
            } else {
                // Metal doesn't have insertDebugSignpost for command buffers...
                let _: () = msg_send![command_buffer.handle, pushDebugGroup: label.get()];
                let _: () = msg_send![command_buffer.handle, popDebugGroup];
            }
        }
    }

    /// Translation of `METAL_PushDebugGroup()`.
    pub(super) fn push_debug_group_internal(command_buffer: &MetalCommandBuffer, name: &str) {
        let label = ns_string(name);
        // SAFETY: as above.
        unsafe {
            if let Some(encoder) = command_buffer.current_encoder() {
                let _: () = msg_send![encoder, pushDebugGroup: label.get()];
            } else {
                let _: () = msg_send![command_buffer.handle, pushDebugGroup: label.get()];
            }
        }
    }

    /// Translation of `METAL_PopDebugGroup()`.
    pub(super) fn pop_debug_group_internal(command_buffer: &MetalCommandBuffer) {
        // SAFETY: as above.
        unsafe {
            if let Some(encoder) = command_buffer.current_encoder() {
                let _: () = msg_send![encoder, popDebugGroup];
            } else {
                let _: () = msg_send![command_buffer.handle, popDebugGroup];
            }
        }
    }

    // Graphics State

    /// Translation of `METAL_INTERNAL_AllocateCommandBuffers()`.
    pub(super) fn allocate_command_buffers(
        available: &mut AvailableCommandBuffers,
        allocate_count: usize,
    ) {
        available.capacity += allocate_count;
        for _ in 0..allocate_count {
            available.buffers.push(MetalCommandBuffer::new());
        }
    }

    /// Translation of `METAL_AcquireCommandBuffer()` (with
    /// `METAL_INTERNAL_GetInactiveCommandBufferFromPool()`).
    pub(super) fn acquire_command_buffer_internal(&self) -> MetalCommandBuffer {
        let mut available = lock(&self.available_command_buffers);

        if available.buffers.is_empty() {
            let count = available.capacity;
            Self::allocate_command_buffers(&mut available, count);
        }

        let mut command_buffer = available.buffers.pop().expect("allocated above");
        // SAFETY: an autoreleased command buffer of the queue, retained.
        command_buffer.handle = unsafe { StrongPtr::retain(msg_send![self.queue, commandBuffer]) };

        command_buffer.graphics_pipeline = None;
        command_buffer.compute_pipeline = None;
        command_buffer.vertex_uniform_buffers = [None; UNIFORM_BUFFERS];
        command_buffer.fragment_uniform_buffers = [None; UNIFORM_BUFFERS];
        command_buffer.compute_uniform_buffers = [None; UNIFORM_BUFFERS];

        command_buffer
    }

    /// Translation of `METAL_INTERNAL_AcquireUniformBufferFromPool()`: the
    /// uniform buffer's index in the command buffer's.
    ///
    /// This function assumes that it's called from within an autorelease pool
    pub(super) fn acquire_uniform_buffer_from_pool(
        &self,
        command_buffer: &mut MetalCommandBuffer,
    ) -> Option<usize> {
        let uniform_buffer = {
            let mut pool = lock(&self.uniform_buffer_pool);
            match pool.pop() {
                Some(uniform_buffer) => Some(uniform_buffer),
                None => self.create_uniform_buffer(UNIFORM_BUFFER_SIZE),
            }
        };

        // Note (upstream): C tracks (and then uses) a NULL uniform buffer
        // when the creation fails; there is none here.
        uniform_buffer.map(|u| command_buffer.track_uniform_buffer(u))
    }

    /// Translation of `METAL_INTERNAL_ReturnUniformBufferToPool()` (the
    /// caller holds `acquireUniformBufferLock`).
    fn return_uniform_buffer_to_pool(
        pool: &mut Vec<MetalUniformBuffer>,
        mut uniform_buffer: MetalUniformBuffer,
    ) {
        uniform_buffer.write_offset = 0;
        uniform_buffer.draw_offset = 0;
        pool.push(uniform_buffer);
    }

    /// Translation of `METAL_INTERNAL_PushUniformData()`.
    ///
    /// This function assumes that it's called from within an autorelease pool
    pub(super) fn push_uniform_data(
        &self,
        command_buffer: &mut MetalCommandBuffer,
        shader_stage: UniformStage,
        slot_index: u32,
        data: &[u8],
    ) {
        let slot = slot_index as usize;

        if command_buffer.uniform_slots(shader_stage)[slot].is_none() {
            let acquired = self.acquire_uniform_buffer_from_pool(command_buffer);
            command_buffer.uniform_slots(shader_stage)[slot] = acquired;
        }
        let Some(mut index) = command_buffer.uniform_slots(shader_stage)[slot] else {
            crate::log::error!(Category::Gpu, "Could not create uniform buffer");
            return;
        };

        let aligned_data_length = next_highest_alignment(data.len() as u32, 256);

        if command_buffer.used_uniform_buffers[index].write_offset + aligned_data_length
            >= UNIFORM_BUFFER_SIZE
        {
            let Some(new_index) = self.acquire_uniform_buffer_from_pool(command_buffer) else {
                crate::log::error!(Category::Gpu, "Could not create uniform buffer");
                return;
            };
            index = new_index;

            let metal_uniform_buffer = &mut command_buffer.used_uniform_buffers[index];
            metal_uniform_buffer.write_offset = 0;
            metal_uniform_buffer.draw_offset = 0;

            command_buffer.uniform_slots(shader_stage)[slot] = Some(index);
        }

        let metal_uniform_buffer = &mut command_buffer.used_uniform_buffers[index];
        metal_uniform_buffer.draw_offset = metal_uniform_buffer.write_offset;

        // SAFETY: a shared buffer of UNIFORM_BUFFER_SIZE bytes, written
        // below its end (the check above).
        unsafe {
            let contents: *mut u8 = msg_send![metal_uniform_buffer.handle, contents];
            std::ptr::copy_nonoverlapping(
                data.as_ptr(),
                contents.add(metal_uniform_buffer.write_offset as usize),
                data.len(),
            );
        }

        metal_uniform_buffer.write_offset += aligned_data_length;

        match shader_stage {
            UniformStage::Vertex => command_buffer.need_vertex_uniform_buffer_bind[slot] = true,
            UniformStage::Fragment => command_buffer.need_fragment_uniform_buffer_bind[slot] = true,
            UniformStage::Compute => command_buffer.need_compute_uniform_buffer_bind[slot] = true,
        }
    }

    // Fences

    /// Translation of `METAL_INTERNAL_AcquireFence()` (with
    /// `METAL_INTERNAL_CreateFence()`).
    fn acquire_fence(&self, command_buffer: &mut MetalCommandBuffer) {
        // Acquire a fence from the pool
        let fence = lock(&self.available_fences).pop().unwrap_or_else(|| {
            Arc::new(MetalFence {
                command_buffer: Mutex::new(StrongPtr::nil()),
                reference_count: AtomicI32::new(0),
            })
        });

        // Associate the fence with the command buffer
        *lock(&fence.command_buffer) = command_buffer.handle.clone();
        fence.reference_count.fetch_add(1, Ordering::SeqCst);
        command_buffer.fence = Some(fence);
    }

    /// Translation of `METAL_INTERNAL_ReleaseFenceToPool()`.
    fn release_fence_to_pool(&self, fence: Arc<MetalFence>) {
        lock(&self.available_fences).push(fence);
    }

    /// Translation of `METAL_ReleaseFence()`.
    pub(super) fn release_fence_internal(&self, fence: &Arc<MetalFence>) {
        if fence.reference_count.fetch_sub(1, Ordering::SeqCst) == 1 {
            // Nothing references the fence anymore, so the command buffer can go too.
            *lock(&fence.command_buffer) = StrongPtr::nil();
            self.release_fence_to_pool(fence.clone());
        }
    }

    /// Translation of `METAL_INTERNAL_IsFenceBusy()`.
    pub(super) fn is_fence_busy(fence: &MetalFence) -> bool {
        let command_buffer = lock(&fence.command_buffer);
        // SAFETY: a command buffer's status (nil reads as 0, not busy).
        let status: NSUInteger = unsafe { msg_send![*command_buffer, status] };
        status == MTL_COMMAND_BUFFER_STATUS_COMMITTED
            || status == MTL_COMMAND_BUFFER_STATUS_SCHEDULED
    }

    /// Translation of `METAL_WaitForFences()`.
    pub(super) fn wait_for_fences_internal(&self, wait_all: bool, fences: &[Arc<MetalFence>]) {
        if wait_all {
            for fence in fences {
                let command_buffer = lock(&fence.command_buffer).clone();
                // SAFETY: waiting for a command buffer (a no-op on nil).
                let _: () = unsafe { msg_send![command_buffer, waitUntilCompleted] };
            }
        } else {
            let mut waiting = true;
            while waiting {
                for fence in fences {
                    if !Self::is_fence_busy(fence) {
                        waiting = false;
                        break;
                    }
                }
            }
        }

        self.perform_pending_destroys();
    }

    // Cleanup

    /// Translation of `METAL_INTERNAL_CleanCommandBuffer()`: end its
    /// passes, give back its uniform buffers and resources, and put it back
    /// with the available command buffers. The caller holds `submitLock`
    /// and has taken it out of the submitted ones.
    pub(super) fn clean_command_buffer(
        &self,
        mut command_buffer: MetalCommandBuffer,
        cancel: bool,
    ) {
        // End any active passes
        // SAFETY: ending the encoders still recording.
        unsafe {
            if !command_buffer.render_encoder.is_nil() {
                let _: () = msg_send![command_buffer.render_encoder, endEncoding];
                command_buffer.render_encoder = StrongPtr::nil();
            }
            if !command_buffer.compute_encoder.is_nil() {
                let _: () = msg_send![command_buffer.compute_encoder, endEncoding];
                command_buffer.compute_encoder = StrongPtr::nil();
            }
            if !command_buffer.blit_encoder.is_nil() {
                let _: () = msg_send![command_buffer.blit_encoder, endEncoding];
                command_buffer.blit_encoder = StrongPtr::nil();
            }
        }

        // Uniform buffers are now available

        {
            let mut pool = lock(&self.uniform_buffer_pool);
            for uniform_buffer in command_buffer.used_uniform_buffers.drain(..) {
                Self::return_uniform_buffer_to_pool(&mut pool, uniform_buffer);
            }
        }

        // Reference Counting

        for buffer in command_buffer.used_buffers.drain(..) {
            buffer.reference_count.fetch_sub(1, Ordering::SeqCst);
        }

        for texture in command_buffer.used_textures.drain(..) {
            texture.reference_count.fetch_sub(1, Ordering::SeqCst);
        }

        // Reset presentation
        command_buffer.window_datas.clear();

        // Reset bindings
        command_buffer.vertex_buffers = Default::default();
        command_buffer.vertex_buffer_offsets = [0; VERTEX_BUFFERS];
        command_buffer.vertex_buffer_count = 0;
        command_buffer.index_buffer = None;
        command_buffer.vertex_samplers = Default::default();
        command_buffer.vertex_textures = Default::default();
        command_buffer.fragment_samplers = Default::default();
        command_buffer.fragment_textures = Default::default();
        command_buffer.compute_samplers = Default::default();
        command_buffer.compute_sampler_textures = Default::default();
        command_buffer.vertex_storage_textures = Default::default();
        command_buffer.fragment_storage_textures = Default::default();
        command_buffer.compute_read_only_textures = Default::default();
        command_buffer.vertex_storage_buffers = Default::default();
        command_buffer.fragment_storage_buffers = Default::default();
        command_buffer.compute_read_only_buffers = Default::default();
        command_buffer.compute_read_write_textures = Default::default();
        command_buffer.compute_read_write_buffers = Default::default();

        command_buffer.need_vertex_buffer_bind = false;
        command_buffer.need_vertex_sampler_bind = false;
        command_buffer.need_vertex_storage_buffer_bind = false;
        command_buffer.need_vertex_storage_texture_bind = false;
        command_buffer.need_vertex_uniform_buffer_bind = [false; UNIFORM_BUFFERS];

        command_buffer.need_fragment_sampler_bind = false;
        command_buffer.need_fragment_storage_buffer_bind = false;
        command_buffer.need_fragment_storage_texture_bind = false;
        command_buffer.need_fragment_uniform_buffer_bind = [false; UNIFORM_BUFFERS];

        command_buffer.need_compute_sampler_bind = false;
        command_buffer.need_compute_read_only_storage_buffer_bind = false;
        command_buffer.need_compute_read_only_storage_texture_bind = false;
        command_buffer.need_compute_uniform_buffer_bind = [false; UNIFORM_BUFFERS];

        // Drop the command buffer's reference to the fence. A cancelled
        // command buffer never acquired one.
        if !cancel {
            if let Some(fence) = command_buffer.fence.take() {
                self.release_fence_internal(&fence);
            }
        }
        // (the pipelines and the native command buffer aren't reset until
        // the next acquisition upstream; dropping them here changes
        // nothing the GPU uses)
        command_buffer.graphics_pipeline = None;
        command_buffer.compute_pipeline = None;
        command_buffer.handle = StrongPtr::nil();

        // Return command buffer to pool
        lock(&self.available_command_buffers)
            .buffers
            .push(command_buffer);

        // (the caller removed this command buffer from the submitted list)
    }

    // Submission

    /// Translation of `METAL_INTERNAL_Submit()`: the fence, when `fence`
    /// is asked for.
    pub(super) fn submit_internal(
        &self,
        mut command_buffer: MetalCommandBuffer,
        fence: bool,
    ) -> Result<Option<Arc<MetalFence>>> {
        let mut submitted = lock(&self.submitted_command_buffers);

        self.acquire_fence(&mut command_buffer);
        let command_fence = command_buffer.fence.clone().expect("acquired above");

        // Give the caller its own reference while submitLock is held, another
        // thread could recycle this command buffer as soon as the lock is released.
        let result = if fence {
            command_fence.reference_count.fetch_add(1, Ordering::SeqCst);
            Some(command_fence.clone())
        } else {
            None
        };

        // Enqueue present requests, if applicable
        let allowed_frames_in_flight = self.allowed_frames_in_flight.load(Ordering::SeqCst);
        for entry in &command_buffer.window_datas {
            let mut window_data = lock(&entry.data);
            let drawable = std::mem::take(&mut window_data.drawable);
            // SAFETY: presenting the drawable the command buffer rendered to.
            let _: () =
                unsafe { msg_send![command_buffer.handle, presentDrawable: drawable.get()] };

            let frame_counter = window_data.frame_counter as usize;
            window_data.in_flight_fences[frame_counter] = Some(command_fence.clone());

            command_fence.reference_count.fetch_add(1, Ordering::SeqCst);

            window_data.frame_counter = (window_data.frame_counter + 1) % allowed_frames_in_flight;
        }

        // Submit the command buffer
        // SAFETY: committing the command buffer once.
        let _: () = unsafe { msg_send![command_buffer.handle, commit] };
        command_buffer.handle = StrongPtr::nil();

        // Mark the command buffer as submitted
        submitted.push(command_buffer);

        // Check if we can perform any cleanups
        let mut i = submitted.len();
        while i > 0 {
            i -= 1;
            let busy = submitted[i]
                .fence
                .as_ref()
                .is_some_and(|f| Self::is_fence_busy(f));
            if !busy {
                let command_buffer = submitted.swap_remove(i);
                self.clean_command_buffer(command_buffer, false);
            }
        }

        self.perform_pending_destroys();

        Ok(result)
    }

    /// Translation of `METAL_Cancel()`.
    pub(super) fn cancel_internal(&self, command_buffer: MetalCommandBuffer) -> Result<()> {
        let _submitted = lock(&self.submitted_command_buffers);
        self.clean_command_buffer(command_buffer, true);
        Ok(())
    }

    /// Translation of `METAL_Wait()`.
    pub(super) fn wait_internal(&self) -> Result<()> {
        /*
         * Wait for all submitted command buffers to complete.
         * Sort of equivalent to vkDeviceWaitIdle.
         */
        let fences: Vec<Arc<MetalFence>> = lock(&self.submitted_command_buffers)
            .iter()
            .filter_map(|c| c.fence.clone())
            .collect();
        for fence in &fences {
            self.wait_for_fences_internal(true, std::slice::from_ref(fence));
        }

        let mut submitted = lock(&self.submitted_command_buffers);

        while let Some(command_buffer) = submitted.pop() {
            self.clean_command_buffer(command_buffer, false);
        }

        self.perform_pending_destroys();

        Ok(())
    }
}

/// The available command buffers (`availableCommandBuffers` and its
/// capacity, with `acquireCommandBufferLock`).
#[derive(Debug, Default)]
pub(super) struct AvailableCommandBuffers {
    pub(super) buffers: Vec<MetalCommandBuffer>,
    pub(super) capacity: usize,
}
