// Rust translation of the command buffer, fence, uniform buffer and
// submission parts of src/gpu/vulkan/SDL_gpu_vulkan.c from Simple
// DirectMedia Layer.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! Command buffers, their per-thread pools, fences, the uniform buffer
//! pool, submission (with presentation, the cleanup of finished command
//! buffers and the defragmentation hook), cancelling and waiting.
//!
//! A command buffer is a [`VulkanCommandBuffer`] value: the pool of the thread
//! that acquired it hands it out, the front end owns it while it is
//! recorded, the submitted list holds it while the GPU runs it, and
//! cleaning it puts it back into its pool (upstream moves the same pointer
//! between these lists).

use std::collections::HashMap;
use std::ptr::null;
use std::sync::atomic::{AtomicI32, Ordering};
use std::sync::Arc;
use std::thread::ThreadId;

use super::memory::next_highest_alignment32;
use super::resources::{UniformBuffer, VulkanCommandBuffer, VulkanTextureUsageMode};
use super::{lock, VulkanRenderer};
use crate::error::Result;
use crate::gpu::sysgpu::{MAX_PRESENT_COUNT, MAX_UNIFORM_BUFFERS_PER_STAGE, UNIFORM_BUFFER_SIZE};
use crate::log::Category;
use crate::video::vk::*;

/// The size of the uniform buffer sections the shaders see. Translation
/// of `MAX_UBO_SECTION_SIZE` (4 KiB).
pub(super) const MAX_UBO_SECTION_SIZE: u32 = 4096;

/// A fence, shared by the command buffer it signals, the swapchains that
/// wait on it and the application. Translation of `VulkanFenceHandle`.
#[derive(Debug)]
pub(super) struct VulkanFenceHandle {
    pub(super) fence: VkFence,
    pub(super) reference_count: AtomicI32,
}

/// The command buffers of a thread. Translation of `VulkanCommandPool`
/// (its `threadID` is its key in the renderer's pools).
#[derive(Debug)]
pub(super) struct VulkanCommandPool {
    pub(super) command_pool: VkCommandPool,
    pub(super) inactive_command_buffers: Vec<VulkanCommandBuffer>,
}

/// The renderer's command pools (`commandPoolHashTable`).
pub(super) type CommandPoolHashTable = HashMap<ThreadId, VulkanCommandPool>;

/// Which stage uniform data is pushed for. Translation of
/// `VulkanUniformBufferStage`.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum VulkanUniformBufferStage {
    Vertex,
    Fragment,
    Compute,
}

impl VulkanCommandBuffer {
    /// The command buffer's uniform buffers of a stage.
    fn uniform_buffers(
        &mut self,
        stage: VulkanUniformBufferStage,
    ) -> &mut [Option<Arc<UniformBuffer>>; MAX_UNIFORM_BUFFERS_PER_STAGE as usize] {
        match stage {
            VulkanUniformBufferStage::Vertex => &mut self.vertex_uniform_buffers,
            VulkanUniformBufferStage::Fragment => &mut self.fragment_uniform_buffers,
            VulkanUniformBufferStage::Compute => &mut self.compute_uniform_buffers,
        }
    }
}

impl VulkanRenderer {
    /// Translation of `VULKAN_INTERNAL_DestroyCommandPool()`.
    pub(super) fn destroy_command_pool(&self, command_pool: VulkanCommandPool) {
        // SAFETY: the pool, whose command buffers the GPU is done with.
        unsafe {
            (self.dev.destroy_command_pool)(self.logical_device, command_pool.command_pool, null())
        };

        // (the inactive command buffers go with the pool)
    }

    // Command Buffers

    /// Translation of `VULKAN_INTERNAL_BeginCommandBuffer()`.
    fn begin_command_buffer(&self, command_buffer: &VulkanCommandBuffer) -> Result<()> {
        let begin_info = VkCommandBufferBeginInfo {
            s_type: VK_STRUCTURE_TYPE_COMMAND_BUFFER_BEGIN_INFO,
            p_next: null(),
            flags: VK_COMMAND_BUFFER_USAGE_ONE_TIME_SUBMIT_BIT,
            p_inheritance_info: null(),
        };

        // SAFETY: a reset command buffer and a valid begin info.
        let result =
            unsafe { (self.dev.begin_command_buffer)(command_buffer.command_buffer, &begin_info) };

        self.check(result, "vkBeginCommandBuffer")
    }

    /// Translation of `VULKAN_INTERNAL_EndCommandBuffer()`.
    fn end_command_buffer(&self, command_buffer: &VulkanCommandBuffer) -> Result<()> {
        // SAFETY: a command buffer being recorded.
        let result = unsafe { (self.dev.end_command_buffer)(command_buffer.command_buffer) };

        self.check(result, "vkEndCommandBuffer")
    }

    /// Allocate a command buffer into a pool's inactive ones. Translation of
    /// `VULKAN_INTERNAL_AllocateCommandBuffer()`.
    fn allocate_command_buffer(
        &self,
        vulkan_command_pool: &mut VulkanCommandPool,
        thread_id: ThreadId,
    ) -> Result<()> {
        let allocate_info = VkCommandBufferAllocateInfo {
            s_type: VK_STRUCTURE_TYPE_COMMAND_BUFFER_ALLOCATE_INFO,
            p_next: null(),
            command_pool: vulkan_command_pool.command_pool,
            command_buffer_count: 1,
            level: VK_COMMAND_BUFFER_LEVEL_PRIMARY,
        };

        let mut command_buffer_handle: VkCommandBuffer = std::ptr::null_mut();
        // SAFETY: a valid allocate info and room for one command buffer.
        let vulkan_result = unsafe {
            (self.dev.allocate_command_buffers)(
                self.logical_device,
                &allocate_info,
                &mut command_buffer_handle,
            )
        };

        self.check(vulkan_result, "vkAllocateCommandBuffers")?;

        let mut command_buffer = VulkanCommandBuffer::new(command_buffer_handle);
        command_buffer.command_pool = Some(thread_id);

        // Pool it!
        vulkan_command_pool
            .inactive_command_buffers
            .push(command_buffer);

        Ok(())
    }

    /// The command pool of a thread, made if needed. Translation of
    /// `VULKAN_INTERNAL_FetchCommandPool()` (the caller holds
    /// `acquireCommandBufferLock`, the pools' lock).
    fn fetch_command_pool<'a>(
        &self,
        pools: &'a mut CommandPoolHashTable,
        thread_id: ThreadId,
    ) -> Result<&'a mut VulkanCommandPool> {
        if pools.contains_key(&thread_id) {
            return Ok(pools.get_mut(&thread_id).expect("just found"));
        }

        let command_pool_create_info = VkCommandPoolCreateInfo {
            s_type: VK_STRUCTURE_TYPE_COMMAND_POOL_CREATE_INFO,
            p_next: null(),
            flags: VK_COMMAND_POOL_CREATE_RESET_COMMAND_BUFFER_BIT,
            queue_family_index: self.queue_family_index,
        };

        let mut command_pool = VK_NULL_HANDLE;
        // SAFETY: a valid create info.
        let vulkan_result = unsafe {
            (self.dev.create_command_pool)(
                self.logical_device,
                &command_pool_create_info,
                null(),
                &mut command_pool,
            )
        };

        self.check(vulkan_result, "vkCreateCommandPool")?;

        let mut vulkan_command_pool = VulkanCommandPool {
            command_pool,
            inactive_command_buffers: Vec::new(),
        };

        if let Err(e) = self.allocate_command_buffer(&mut vulkan_command_pool, thread_id) {
            self.destroy_command_pool(vulkan_command_pool);
            return Err(e);
        }

        Ok(pools.entry(thread_id).or_insert(vulkan_command_pool))
    }

    /// An inactive command buffer of the thread's pool, allocated if there
    /// is none. Translation of
    /// `VULKAN_INTERNAL_GetInactiveCommandBufferFromPool()`.
    fn get_inactive_command_buffer_from_pool(
        &self,
        pools: &mut CommandPoolHashTable,
        thread_id: ThreadId,
    ) -> Result<VulkanCommandBuffer> {
        let command_pool = self.fetch_command_pool(pools, thread_id)?;

        if command_pool.inactive_command_buffers.is_empty() {
            self.allocate_command_buffer(command_pool, thread_id)?;
        }

        Ok(command_pool
            .inactive_command_buffers
            .pop()
            .expect("a command buffer was just allocated"))
    }

    /// A command buffer being recorded, from the calling thread's pool.
    /// Translation of `VULKAN_AcquireCommandBuffer()`.
    pub(super) fn acquire_command_buffer_internal(&self) -> Result<VulkanCommandBuffer> {
        let thread_id = std::thread::current().id();

        let (mut command_buffer, descriptor_set_cache) = {
            let mut pools = lock(&self.command_pools);

            let command_buffer =
                self.get_inactive_command_buffer_from_pool(&mut pools, thread_id)?;

            let descriptor_set_cache = self.acquire_descriptor_set_cache();

            (command_buffer, descriptor_set_cache)
        };

        command_buffer.descriptor_set_cache = Some(descriptor_set_cache);

        // Reset state
        let cb = &mut command_buffer;

        cb.current_compute_pipeline = None;
        cb.current_graphics_pipeline = None;

        cb.color_attachment_subresources.clear();
        cb.resolve_attachment_subresources.clear();
        cb.depth_stencil_attachment_subresource = None;

        cb.vertex_uniform_buffers = Default::default();
        cb.fragment_uniform_buffers = Default::default();
        cb.compute_uniform_buffers = Default::default();

        cb.need_vertex_buffer_bind = false;
        cb.need_new_vertex_resource_descriptor_set = true;
        cb.need_new_vertex_uniform_descriptor_set = true;
        cb.need_new_vertex_uniform_offsets = true;
        cb.need_new_fragment_resource_descriptor_set = true;
        cb.need_new_fragment_uniform_descriptor_set = true;
        cb.need_new_fragment_uniform_offsets = true;

        cb.need_new_compute_read_only_descriptor_set = true;
        cb.need_new_compute_uniform_descriptor_set = true;
        cb.need_new_compute_uniform_offsets = true;

        cb.vertex_resource_descriptor_set = VK_NULL_HANDLE;
        cb.vertex_uniform_descriptor_set = VK_NULL_HANDLE;
        cb.fragment_resource_descriptor_set = VK_NULL_HANDLE;
        cb.fragment_uniform_descriptor_set = VK_NULL_HANDLE;

        cb.compute_read_only_descriptor_set = VK_NULL_HANDLE;
        cb.compute_read_write_descriptor_set = VK_NULL_HANDLE;
        cb.compute_uniform_descriptor_set = VK_NULL_HANDLE;

        cb.vertex_buffers = Default::default();
        cb.vertex_buffer_offsets = Default::default();
        cb.vertex_buffer_count = 0;

        cb.vertex_sampler_texture_view_bindings = Default::default();
        cb.vertex_sampler_bindings = Default::default();
        cb.vertex_storage_texture_view_bindings = Default::default();
        cb.vertex_storage_buffer_bindings = Default::default();

        cb.fragment_sampler_texture_view_bindings = Default::default();
        cb.fragment_sampler_bindings = Default::default();
        cb.fragment_storage_texture_view_bindings = Default::default();
        cb.fragment_storage_buffer_bindings = Default::default();

        cb.read_write_compute_storage_texture_subresources.clear();
        cb.read_write_compute_storage_buffers = Default::default();
        cb.compute_sampler_texture_view_bindings = Default::default();
        cb.compute_sampler_bindings = Default::default();
        cb.read_only_compute_storage_texture_view_bindings = Default::default();
        cb.read_only_compute_storage_buffer_bindings = Default::default();
        cb.read_only_compute_storage_textures = Default::default();
        cb.read_only_compute_storage_buffers = Default::default();

        cb.auto_release_fence = true;

        cb.swapchain_requested = false;
        cb.is_defrag = false;

        /* Reset the command buffer here to avoid resets being called
         * from a separate thread than where the command buffer was acquired
         */
        // Note (upstream): when the reset or the begin fails, C loses the
        // command buffer and its descriptor set cache; here they are
        // dropped (the command buffer's memory goes with its pool).
        // SAFETY: a command buffer of this thread's pool, not in use.
        let result = unsafe {
            (self.dev.reset_command_buffer)(
                cb.command_buffer,
                VK_COMMAND_BUFFER_RESET_RELEASE_RESOURCES_BIT,
            )
        };

        self.check(result, "vkResetCommandBuffer")?;

        self.begin_command_buffer(cb)?;

        Ok(command_buffer)
    }

    // Fences

    /// Whether a fence is signaled. Translation of `VULKAN_QueryFence()`.
    pub(super) fn query_fence_internal(&self, fence: &VulkanFenceHandle) -> bool {
        // SAFETY: a fence of the device.
        let result = unsafe { (self.dev.get_fence_status)(self.logical_device, fence.fence) };

        if result == VK_SUCCESS {
            true
        } else if result == VK_NOT_READY {
            false
        } else {
            let _ = self.set_string_error(&format!(
                "vkGetFenceStatus: {}",
                super::tables::vk_error_messages(result)
            ));
            false
        }
    }

    /// Translation of `VULKAN_INTERNAL_ReturnFenceToPool()`.
    fn return_fence_to_pool(&self, fence_handle: Arc<VulkanFenceHandle>) {
        lock(&self.fence_pool).push(fence_handle);
    }

    /// Drop a reference to a fence, which goes back to the pool with the
    /// last one. Translation of `VULKAN_ReleaseFence()`.
    pub(super) fn release_fence_internal(&self, handle: &Arc<VulkanFenceHandle>) {
        // (SDL_AtomicDecRef() is true when the count reaches zero)
        if handle.reference_count.fetch_sub(1, Ordering::SeqCst) == 1 {
            self.return_fence_to_pool(handle.clone());
        }
    }

    /// An unsignaled fence, from the pool or new. Translation of
    /// `VULKAN_INTERNAL_AcquireFenceFromPool()`.
    ///
    /// Note (upstream): C checks the pool's count before taking its lock;
    /// the lock is taken first here.
    fn acquire_fence_from_pool(&self) -> Result<Arc<VulkanFenceHandle>> {
        let mut pool = lock(&self.fence_pool);

        let Some(handle) = pool.pop() else {
            drop(pool);

            // Create fence
            let fence_create_info = VkFenceCreateInfo {
                s_type: VK_STRUCTURE_TYPE_FENCE_CREATE_INFO,
                p_next: null(),
                flags: 0,
            };

            let mut fence = VK_NULL_HANDLE;
            // SAFETY: a valid create info.
            let vulkan_result = unsafe {
                (self.dev.create_fence)(self.logical_device, &fence_create_info, null(), &mut fence)
            };

            self.check(vulkan_result, "vkCreateFence")?;

            return Ok(Arc::new(VulkanFenceHandle {
                fence,
                reference_count: AtomicI32::new(0),
            }));
        };

        // SAFETY: a signaled fence nothing waits on any more.
        let vulkan_result =
            unsafe { (self.dev.reset_fences)(self.logical_device, 1, &handle.fence) };

        drop(pool);

        self.check(vulkan_result, "vkResetFences")?;

        Ok(handle)
    }

    // Uniform buffers

    /// A uniform buffer for a command buffer, from the pool or new.
    /// Translation of `VULKAN_INTERNAL_AcquireUniformBufferFromPool()`.
    ///
    /// Note (upstream): C doesn't check the creation of a new one (a NULL
    /// dereference when it fails); `None` here.
    pub(super) fn acquire_uniform_buffer_from_pool(
        &self,
        command_buffer: &mut VulkanCommandBuffer,
    ) -> Option<Arc<UniformBuffer>> {
        let uniform_buffer = {
            let mut pool = lock(&self.uniform_buffer_pool);

            match pool.pop() {
                Some(uniform_buffer) => uniform_buffer,
                None => match self.create_uniform_buffer(UNIFORM_BUFFER_SIZE) {
                    Ok(uniform_buffer) => uniform_buffer,
                    Err(_) => return None,
                },
            }
        };

        command_buffer.track_uniform_buffer(&uniform_buffer);

        Some(uniform_buffer)
    }

    /// Translation of `VULKAN_INTERNAL_ReturnUniformBufferToPool()` (the
    /// caller holds `acquireUniformBufferLock`, the pool's lock).
    fn return_uniform_buffer_to_pool(
        pool: &mut Vec<Arc<UniformBuffer>>,
        uniform_buffer: Arc<UniformBuffer>,
    ) {
        {
            let mut state = lock(&uniform_buffer.state);
            state.write_offset = 0;
            state.draw_offset = 0;
        }
        pool.push(uniform_buffer);
    }

    /// Copy uniform data into a stage's uniform buffer for the following
    /// draws or dispatches. Translation of `VULKAN_INTERNAL_PushUniformData()`.
    pub(super) fn push_uniform_data(
        &self,
        command_buffer: &mut VulkanCommandBuffer,
        uniform_buffer_stage: VulkanUniformBufferStage,
        slot_index: u32,
        data: &[u8],
    ) {
        let length = data.len() as u32;
        let block_size = next_highest_alignment32(length, self.min_ubo_alignment);
        let slot = slot_index as usize;

        // Note (upstream): C writes past the end of the uniform buffer when
        // the data is longer than one; such data is dropped here.
        if length > UNIFORM_BUFFER_SIZE {
            crate::log::error!(Category::Gpu, "Uniform data is too large!");
            return;
        }

        if command_buffer.uniform_buffers(uniform_buffer_stage)[slot].is_none() {
            let uniform_buffer = self.acquire_uniform_buffer_from_pool(command_buffer);
            command_buffer.uniform_buffers(uniform_buffer_stage)[slot] = uniform_buffer;
        }
        let Some(mut uniform_buffer) =
            command_buffer.uniform_buffers(uniform_buffer_stage)[slot].clone()
        else {
            return;
        };

        // If there is no more room, acquire a new uniform buffer
        let (write_offset, size) = {
            let state = lock(&uniform_buffer.state);
            (state.write_offset, state.buffer.size)
        };
        if write_offset as VkDeviceSize
            + block_size as VkDeviceSize
            + MAX_UBO_SECTION_SIZE as VkDeviceSize
            >= size
        {
            let Some(new_uniform_buffer) = self.acquire_uniform_buffer_from_pool(command_buffer)
            else {
                return;
            };
            uniform_buffer = new_uniform_buffer;

            {
                let mut state = lock(&uniform_buffer.state);
                state.draw_offset = 0;
                state.write_offset = 0;
            }

            command_buffer.uniform_buffers(uniform_buffer_stage)[slot] =
                Some(uniform_buffer.clone());
            match uniform_buffer_stage {
                VulkanUniformBufferStage::Vertex => {
                    command_buffer.need_new_vertex_uniform_descriptor_set = true
                }
                VulkanUniformBufferStage::Fragment => {
                    command_buffer.need_new_fragment_uniform_descriptor_set = true
                }
                VulkanUniformBufferStage::Compute => {
                    command_buffer.need_new_compute_uniform_descriptor_set = true
                }
            }
        }

        {
            let mut state = lock(&uniform_buffer.state);
            state.draw_offset = state.write_offset;

            let used_region = &state.buffer.used_region;
            let Some(map_pointer) = used_region.allocation.map_pointer() else {
                crate::log::error!(Category::Gpu, "Uniform buffer memory is not mapped!");
                return;
            };

            // SAFETY: uniform buffers are in mapped host-visible memory, and
            // the data fits in the buffer past the write offset (checked
            // above).
            unsafe {
                let dst = map_pointer
                    .as_ptr()
                    .add(used_region.resource_offset as usize + state.write_offset as usize);
                std::ptr::copy_nonoverlapping(data.as_ptr(), dst, data.len());
            }

            state.write_offset += block_size;
        }

        match uniform_buffer_stage {
            VulkanUniformBufferStage::Vertex => {
                command_buffer.need_new_vertex_uniform_offsets = true
            }
            VulkanUniformBufferStage::Fragment => {
                command_buffer.need_new_fragment_uniform_offsets = true
            }
            VulkanUniformBufferStage::Compute => {
                command_buffer.need_new_compute_uniform_offsets = true
            }
        }
    }

    // Submission

    /// Give back what a finished (or cancelled) command buffer holds and
    /// return it to its pool. Translation of
    /// `VULKAN_INTERNAL_CleanCommandBuffer()`; the caller holds `submitLock`
    /// and has taken the command buffer out of the submitted list.
    pub(super) fn clean_command_buffer(
        &self,
        mut command_buffer: VulkanCommandBuffer,
        _cancel: bool,
    ) {
        if command_buffer.auto_release_fence {
            if let Some(fence) = &command_buffer.in_flight_fence {
                self.release_fence_internal(fence);
            }
        }
        // (the application holds a fence it acquired)
        command_buffer.in_flight_fence = None;

        // Uniform buffers are now available
        {
            let mut pool = lock(&self.uniform_buffer_pool);

            for uniform_buffer in command_buffer.used_uniform_buffers.drain(..) {
                Self::return_uniform_buffer_to_pool(&mut pool, uniform_buffer);
            }
        }

        // Decrement reference counts

        for buffer in command_buffer.used_buffers.drain(..) {
            buffer.reference_count.fetch_sub(1, Ordering::SeqCst);
        }

        for buffer in command_buffer.buffers_used_in_pending_transfers.drain(..) {
            buffer
                .used_region
                .allocation
                .reference_count
                .fetch_sub(1, Ordering::SeqCst);
        }

        for texture in command_buffer.used_textures.drain(..) {
            texture.reference_count.fetch_sub(1, Ordering::SeqCst);
        }

        for texture in command_buffer.textures_used_in_pending_transfers.drain(..) {
            if let Some(used_region) = &texture.used_region {
                used_region
                    .allocation
                    .reference_count
                    .fetch_sub(1, Ordering::SeqCst);
            }
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

        for framebuffer in command_buffer.used_framebuffers.drain(..) {
            framebuffer.reference_count.fetch_sub(1, Ordering::SeqCst);
        }

        // Reset presentation data

        command_buffer.present_datas.clear();
        command_buffer.wait_semaphores.clear();
        command_buffer.signal_semaphores.clear();
        command_buffer.swapchain_requested = false;

        // Reset defrag state

        if command_buffer.is_defrag {
            self.defrag_in_progress.store(false, Ordering::SeqCst);
        }

        // Return command buffer to pool

        let descriptor_set_cache = command_buffer.descriptor_set_cache.take();
        let mut pools = lock(&self.command_pools);

        if let Some(pool) = command_buffer
            .command_pool
            .and_then(|thread_id| pools.get_mut(&thread_id))
        {
            pool.inactive_command_buffers.push(command_buffer);
        }

        // Release descriptor set cache

        if let Some(descriptor_set_cache) = descriptor_set_cache {
            self.return_descriptor_set_cache_to_pool(descriptor_set_cache);
        }

        drop(pools);

        // (the caller removed it from the submitted list)
    }

    /// Clean the submitted command buffers whose fence is signaled (from
    /// the most recent one, as upstream's loops). The caller holds
    /// `submitLock`, whose data is the list.
    fn clean_finished_command_buffers(&self, submitted: &mut Vec<VulkanCommandBuffer>) {
        for i in (0..submitted.len()).rev() {
            let Some(fence) = &submitted[i].in_flight_fence else {
                continue;
            };
            // SAFETY: the fence of a submitted command buffer.
            let result = unsafe { (self.dev.get_fence_status)(self.logical_device, fence.fence) };

            if result == VK_SUCCESS {
                let command_buffer = submitted.swap_remove(i);
                self.clean_command_buffer(command_buffer, false);
            }
        }
    }

    /// Wait for fences (all of them, or any), then clean the command
    /// buffers that are done. Translation of `VULKAN_WaitForFences()`.
    pub(super) fn wait_for_fences_internal(
        &self,
        wait_all: bool,
        fences: &[&VulkanFenceHandle],
    ) -> Result<()> {
        let vk_fences: Vec<VkFence> = fences.iter().map(|f| f.fence).collect();

        // SAFETY: fences of the device.
        let result = unsafe {
            (self.dev.wait_for_fences)(
                self.logical_device,
                vk_fences.len() as u32,
                vk_fences.as_ptr(),
                wait_all as VkBool32,
                u64::MAX,
            )
        };

        self.check(result, "vkWaitForFences")?;

        let mut submitted = lock(&self.submit_lock);

        self.clean_finished_command_buffers(&mut submitted);

        self.perform_pending_destroys();

        Ok(())
    }

    /// Wait for the device to be idle, then clean every submitted command
    /// buffer. Translation of `VULKAN_Wait()`.
    pub(super) fn wait_internal(&self) -> Result<()> {
        let mut submitted = lock(&self.submit_lock);

        // SAFETY: the device.
        let result = unsafe { (self.dev.device_wait_idle)(self.logical_device) };

        if result != VK_SUCCESS {
            return Err(self.vk_error(result, "vkDeviceWaitIdle"));
        }

        while let Some(command_buffer) = submitted.pop() {
            self.clean_command_buffer(command_buffer, false);
        }

        self.perform_pending_destroys();

        Ok(())
    }

    /// Submit a command buffer and present the swapchain textures it
    /// acquired, returning its fence. Translation of `VULKAN_Submit()`
    /// (and of `VULKAN_SubmitAndAcquireFence()` when the command buffer's
    /// `auto_release_fence` is false).
    ///
    /// FIXME (upstream): when ending the command buffer, acquiring the fence
    /// or the submission fails, the command buffer is lost (and the
    /// resources it tracks are never destroyed).
    pub(super) fn submit_internal(
        &self,
        mut command_buffer: VulkanCommandBuffer,
    ) -> Result<Arc<VulkanFenceHandle>> {
        let claimed_window_count = lock(&self.claimed_windows).len();
        let perform_cleanups = (claimed_window_count > 0 && command_buffer.swapchain_requested)
            || claimed_window_count == 0;

        let mut submitted = lock(&self.submit_lock);

        // FIXME: Can this just be permanent?
        let wait_stages =
            [VK_PIPELINE_STAGE_COLOR_ATTACHMENT_OUTPUT_BIT; MAX_PRESENT_COUNT as usize];

        for j in 0..command_buffer.present_datas.len() {
            let present_data = &command_buffer.present_datas[j];
            let Some(container) = present_data
                .window_data
                .texture_container(present_data.swapchain_image_index)
            else {
                continue;
            };
            let swapchain_texture_subresource = Self::fetch_texture_subresource(&container, 0, 0);

            self.texture_subresource_transition_from_default_usage(
                &mut command_buffer,
                VulkanTextureUsageMode::Present,
                &swapchain_texture_subresource.texture,
                swapchain_texture_subresource.index,
            );
        }

        let allocations_to_defrag_count = {
            let allocator = self.memory_allocator.lock();
            let count = allocator.borrow().allocations_to_defrag.len();
            count
        };
        if perform_cleanups
            && allocations_to_defrag_count > 0
            && !self.defrag_in_progress.load(Ordering::SeqCst)
            && self.defragment_memory(&mut command_buffer).is_err()
        {
            crate::log::error!(
                Category::Gpu,
                "{}",
                "Failed to defragment memory, likely OOM!"
            );
        }

        self.end_command_buffer(&command_buffer)?;

        let in_flight_fence = self.acquire_fence_from_pool()?;

        // Command buffer has a reference to the in-flight fence
        in_flight_fence
            .reference_count
            .fetch_add(1, Ordering::SeqCst);
        command_buffer.in_flight_fence = Some(in_flight_fence.clone());

        let submit_info = VkSubmitInfo {
            s_type: VK_STRUCTURE_TYPE_SUBMIT_INFO,
            p_next: null(),
            command_buffer_count: 1,
            p_command_buffers: &command_buffer.command_buffer,
            p_wait_dst_stage_mask: wait_stages.as_ptr(),
            p_wait_semaphores: command_buffer.wait_semaphores.as_ptr(),
            wait_semaphore_count: command_buffer.wait_semaphores.len() as u32,
            p_signal_semaphores: command_buffer.signal_semaphores.as_ptr(),
            signal_semaphore_count: command_buffer.signal_semaphores.len() as u32,
        };

        // SAFETY: the device's queue, a recorded command buffer, its
        // semaphores and an unsignaled fence.
        let vulkan_result = unsafe {
            (self.dev.queue_submit)(self.unified_queue, 1, &submit_info, in_flight_fence.fence)
        };

        self.check(vulkan_result, "vkQueueSubmit")?;

        // Present, if applicable
        let allowed_frames_in_flight = self.allowed_frames_in_flight.load(Ordering::SeqCst);
        for present_data in &command_buffer.present_datas {
            let mut window_data = lock(&present_data.window_data.data);

            let render_finished_semaphore = window_data
                .render_finished_semaphore
                .get(present_data.swapchain_image_index as usize)
                .copied()
                .unwrap_or(VK_NULL_HANDLE);
            let present_info = VkPresentInfoKHR {
                s_type: VK_STRUCTURE_TYPE_PRESENT_INFO_KHR,
                p_next: null(),
                p_wait_semaphores: &render_finished_semaphore,
                wait_semaphore_count: 1,
                p_swapchains: &window_data.swapchain,
                swapchain_count: 1,
                p_image_indices: &present_data.swapchain_image_index,
                p_results: std::ptr::null_mut(),
            };

            // SAFETY: the device's queue and the swapchain's acquired image.
            let present_result =
                unsafe { (self.dev.queue_present_khr)(self.unified_queue, &present_info) };

            if present_result == VK_SUCCESS
                || present_result == VK_SUBOPTIMAL_KHR
                || present_result == VK_ERROR_OUT_OF_DATE_KHR
            {
                // If presenting, the swapchain is using the in-flight fence
                let frame_counter = window_data.frame_counter as usize;
                window_data.in_flight_fences[frame_counter] = Some(in_flight_fence.clone());
                in_flight_fence
                    .reference_count
                    .fetch_add(1, Ordering::SeqCst);

                // On the Android platform, VK_SUBOPTIMAL_KHR is returned whenever the device is rotated. We'll just ignore this for now.
                if present_result == VK_SUBOPTIMAL_KHR {
                    window_data.needs_swapchain_recreate = true;
                }
                if present_result == VK_ERROR_OUT_OF_DATE_KHR {
                    window_data.needs_swapchain_recreate = true;
                }
            } else if present_result == VK_ERROR_SURFACE_LOST_KHR {
                // Android can destroy the surface at any time when the app goes into the background,
                // even after successfully acquiring a swapchain texture and before presenting it.
                window_data.needs_swapchain_recreate = true;
                window_data.needs_surface_recreate = true;
            } else {
                drop(window_data);
                // (VULKAN_INTERNAL_ReleaseCommandBuffer())
                submitted.push(command_buffer);
                drop(submitted);

                return Err(self.vk_error(present_result, "vkQueuePresentKHR"));
            }

            window_data.frame_counter = (window_data.frame_counter + 1) % allowed_frames_in_flight;
        }

        if perform_cleanups {
            self.clean_finished_command_buffers(&mut submitted);

            let check_empty_allocations = {
                let allocator = self.memory_allocator.lock();
                let check = allocator.borrow().check_empty_allocations;
                check
            };
            if check_empty_allocations {
                let allocator = self.memory_allocator.lock();

                for i in 0..VK_MAX_MEMORY_TYPES {
                    let count = allocator.borrow().allocation_count(i);
                    for j in (0..count).rev() {
                        let empty = {
                            let a = allocator.borrow();
                            a.used_regions(a.allocation_key(i, j)).is_empty()
                        };
                        if empty {
                            self.deallocate_memory(i, j);
                        }
                    }
                }

                allocator.borrow_mut().check_empty_allocations = false;
            }

            self.perform_pending_destroys();
        }

        // Mark command buffer as submitted
        // (VULKAN_INTERNAL_ReleaseCommandBuffer())
        submitted.push(command_buffer);

        Ok(in_flight_fence)
    }

    /// Throw away a command buffer's commands. Translation of
    /// `VULKAN_Cancel()`.
    pub(super) fn cancel_internal(&self, mut command_buffer: VulkanCommandBuffer) -> Result<()> {
        // SAFETY: a command buffer being recorded, not submitted.
        let result = unsafe {
            (self.dev.reset_command_buffer)(
                command_buffer.command_buffer,
                VK_COMMAND_BUFFER_RESET_RELEASE_RESOURCES_BIT,
            )
        };
        self.check(result, "vkResetCommandBuffer")?;

        command_buffer.auto_release_fence = false;
        let _submit = lock(&self.submit_lock);
        self.clean_command_buffer(command_buffer, true);

        Ok(())
    }
}
