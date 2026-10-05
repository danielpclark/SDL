// Rust translation of the memory binding of src/gpu/vulkan/SDL_gpu_vulkan.c
// from Simple DirectMedia Layer.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! Binding buffers and images to memory: choosing the memory types for a
//! resource, suballocating a region (or allocating device memory) with the
//! [`MemoryAllocator`](super::memory::MemoryAllocator), and binding.

use std::ffi::c_void;
use std::ptr::null;
use std::sync::atomic::Ordering;
use std::sync::Arc;

use super::memory::{
    next_highest_alignment, AllocationKey, UsedRegion, LARGE_ALLOCATION_INCREMENT,
    SMALL_ALLOCATION_SIZE, SMALL_ALLOCATION_THRESHOLD,
};
use super::resources::VulkanBufferType;
use super::{lock, VulkanRenderer};
use crate::error::Result;
use crate::log::Category;
use crate::video::vk::*;

/// Why a resource couldn't be bound to memory (`Uint8` 0 and 2 of
/// `VULKAN_INTERNAL_BindResourceMemory()`).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum BindError {
    /// The binding failed (0).
    Failed,
    /// Out of memory (2): the caller handles it.
    OutOfMemory,
}

/// Whether `memory_type_index` isn't in `memory_type_index_array` yet.
/// Translation of `VULKAN_INTERNAL_CheckMemoryTypeArrayUnique()`.
fn check_memory_type_array_unique(memory_type_index: u32, memory_type_index_array: &[u32]) -> bool {
    !memory_type_index_array.contains(&memory_type_index)
}

/// Returns an array of memory type indices in order of preference.
/// Memory types are requested with the following three guidelines:
///
/// Required: Absolutely necessary
/// Preferred: Nice to have, but not necessary
/// Tolerable: Can be allowed if there are no other options
///
/// We return memory types in this order:
/// 1. Required and preferred. This is the best category.
/// 2. Required only.
/// 3. Required, preferred, and tolerable.
/// 4. Required and tolerable. This is the worst category.
///
/// Translation of `VULKAN_INTERNAL_FindBestMemoryTypes()`.
pub(super) fn find_best_memory_types(
    memory_properties: &VkPhysicalDeviceMemoryProperties,
    type_filter: u32,
    required_properties: VkMemoryPropertyFlags,
    preferred_properties: VkMemoryPropertyFlags,
    tolerable_properties: VkMemoryPropertyFlags,
) -> Vec<u32> {
    let mut result = Vec::with_capacity(memory_properties.memory_type_count as usize);
    let count = (memory_properties.memory_type_count as usize).min(VK_MAX_MEMORY_TYPES);
    let types = &memory_properties.memory_types[..count];

    // The four passes: whether all the preferred properties are there (or
    // none is), and all the tolerable ones (or none).
    for (preferred, tolerable) in [(true, false), (false, false), (true, true), (false, true)] {
        for (i, memory_type) in (0u32..).zip(types) {
            let flags = memory_type.property_flags;
            let preferred_ok = if preferred {
                flags & preferred_properties == preferred_properties
            } else {
                flags & preferred_properties == 0
            };
            let tolerable_ok = if tolerable {
                flags & tolerable_properties == tolerable_properties
            } else {
                flags & tolerable_properties == 0
            };
            if (type_filter & (1u32 << i)) != 0
                && flags & required_properties == required_properties
                && preferred_ok
                && tolerable_ok
                && check_memory_type_array_unique(i, &result)
            {
                result.push(i);
            }
        }
    }

    result
}

impl VulkanRenderer {
    /// Translation of `VULKAN_INTERNAL_FindBestBufferMemoryTypes()`.
    fn find_best_buffer_memory_types(
        &self,
        buffer: VkBuffer,
        required_memory_properties: VkMemoryPropertyFlags,
        preferred_memory_properties: VkMemoryPropertyFlags,
        tolerable_memory_properties: VkMemoryPropertyFlags,
    ) -> (Vec<u32>, VkMemoryRequirements) {
        let mut memory_requirements = VkMemoryRequirements::default();
        // SAFETY: a live buffer of the device.
        unsafe {
            (self.dev.get_buffer_memory_requirements)(
                self.logical_device,
                buffer,
                &mut memory_requirements,
            )
        };

        (
            find_best_memory_types(
                &self.memory_properties,
                memory_requirements.memory_type_bits,
                required_memory_properties,
                preferred_memory_properties,
                tolerable_memory_properties,
            ),
            memory_requirements,
        )
    }

    /// Translation of `VULKAN_INTERNAL_FindBestImageMemoryTypes()`.
    fn find_best_image_memory_types(
        &self,
        image: VkImage,
        preferred_memory_property_flags: VkMemoryPropertyFlags,
    ) -> (Vec<u32>, VkMemoryRequirements) {
        let mut memory_requirements = VkMemoryRequirements::default();
        // SAFETY: a live image of the device.
        unsafe {
            (self.dev.get_image_memory_requirements)(
                self.logical_device,
                image,
                &mut memory_requirements,
            )
        };

        (
            find_best_memory_types(
                &self.memory_properties,
                memory_requirements.memory_type_bits,
                0,
                preferred_memory_property_flags,
                0,
            ),
            memory_requirements,
        )
    }

    /// Give a used region back. Translation of
    /// `VULKAN_INTERNAL_RemoveMemoryUsedRegion()` (taking `allocatorLock`).
    pub(super) fn remove_memory_used_region(&self, used_region: &Arc<UsedRegion>) {
        let allocator = self.memory_allocator.lock();
        allocator
            .borrow_mut()
            .remove_memory_used_region(used_region);
    }

    /// Free an allocation without used regions. Translation of
    /// `VULKAN_INTERNAL_DeallocateMemory()`.
    pub(super) fn deallocate_memory(&self, memory_type_index: usize, allocation_index: usize) {
        let allocator = self.memory_allocator.lock();
        let memory = allocator
            .borrow_mut()
            .deallocate_memory(memory_type_index, allocation_index);

        // SAFETY: the allocation's memory, which nothing is bound to.
        unsafe { (self.dev.free_memory)(self.logical_device, memory, null()) };
    }

    /// Allocate a block of a memory type, persistently mapped if it's
    /// host-visible, with one free region over all of it. Translation of
    /// `VULKAN_INTERNAL_AllocateMemory()`; the caller holds `allocatorLock`.
    fn allocate_memory(
        &self,
        memory_type_index: u32,
        allocation_size: VkDeviceSize,
        is_host_visible: bool,
    ) -> Option<AllocationKey> {
        let alloc_info = VkMemoryAllocateInfo {
            s_type: VK_STRUCTURE_TYPE_MEMORY_ALLOCATE_INFO,
            p_next: null(),
            allocation_size,
            memory_type_index,
        };

        let mut memory = VK_NULL_HANDLE;
        // SAFETY: a valid allocate info.
        let result = unsafe {
            (self.dev.allocate_memory)(self.logical_device, &alloc_info, null(), &mut memory)
        };

        if result != VK_SUCCESS {
            // Uh oh, we couldn't allocate, time to clean up
            return None;
        }

        let allocator = self.memory_allocator.lock();

        // Persistent mapping for host-visible memory
        let mut map_pointer = None;
        if is_host_visible {
            let mut pointer: *mut c_void = std::ptr::null_mut();
            // SAFETY: the memory just allocated, host-visible.
            let result = unsafe {
                (self.dev.map_memory)(
                    self.logical_device,
                    memory,
                    0,
                    VK_WHOLE_SIZE,
                    0,
                    &mut pointer,
                )
            };
            if result != VK_SUCCESS {
                // FIXME (upstream): the allocation stays in the list,
                // without its free region (nothing is ever placed in it,
                // and it is only freed with the device).
                allocator.borrow_mut().add_allocation(
                    memory_type_index,
                    allocation_size,
                    memory,
                    None,
                    false,
                );
                let _ = self.vk_error(result, "vkMapMemory");
                return None;
            }
            map_pointer = std::ptr::NonNull::new(pointer.cast::<u8>());
        }

        let key = allocator.borrow_mut().add_allocation(
            memory_type_index,
            allocation_size,
            memory,
            map_pointer,
            true,
        );
        Some(key)
    }

    /// Translation of `VULKAN_INTERNAL_BindBufferMemory()`.
    fn bind_buffer_memory(
        &self,
        used_region: &UsedRegion,
        aligned_offset: VkDeviceSize,
        buffer: VkBuffer,
    ) -> Result<()> {
        let vulkan_result = {
            let _memory_lock = lock(&used_region.allocation.memory_lock);

            // SAFETY: a live buffer and the allocation's memory.
            unsafe {
                (self.dev.bind_buffer_memory)(
                    self.logical_device,
                    buffer,
                    used_region.allocation.memory,
                    aligned_offset,
                )
            }
        };

        self.check(vulkan_result, "vkBindBufferMemory")
    }

    /// Translation of `VULKAN_INTERNAL_BindImageMemory()`.
    fn bind_image_memory(
        &self,
        used_region: &UsedRegion,
        aligned_offset: VkDeviceSize,
        image: VkImage,
    ) -> Result<()> {
        let vulkan_result = {
            let _memory_lock = lock(&used_region.allocation.memory_lock);

            // SAFETY: a live image and the allocation's memory.
            unsafe {
                (self.dev.bind_image_memory)(
                    self.logical_device,
                    image,
                    used_region.allocation.memory,
                    aligned_offset,
                )
            }
        };

        self.check(vulkan_result, "vkBindImageMemory")
    }

    /// Bind a buffer or an image to memory of a type: a free region, or a
    /// new allocation. Translation of `VULKAN_INTERNAL_BindResourceMemory()`.
    fn bind_resource_memory(
        &self,
        memory_type_index: u32,
        memory_requirements: &VkMemoryRequirements,
        resource_size: VkDeviceSize, // may be different from requirements size!
        dedicated: bool, // the entire memory allocation should be used for this resource
        buffer: VkBuffer, // may be VK_NULL_HANDLE
        image: VkImage,  // may be VK_NULL_HANDLE
    ) -> std::result::Result<Arc<UsedRegion>, BindError> {
        let is_host_visible = (self.memory_properties.memory_types[memory_type_index as usize]
            .property_flags
            & VK_MEMORY_PROPERTY_HOST_VISIBLE_BIT)
            != 0;

        let required_size = memory_requirements.size;
        let alignment = memory_requirements.alignment;
        let is_buffer = buffer != VK_NULL_HANDLE;

        if (buffer == VK_NULL_HANDLE && image == VK_NULL_HANDLE)
            || (buffer != VK_NULL_HANDLE && image != VK_NULL_HANDLE)
        {
            crate::log::error!(
                Category::Gpu,
                "BindResourceMemory must be given either a VulkanBuffer or a VulkanTexture"
            );
            return Err(BindError::Failed);
        }

        let allocator = self.memory_allocator.lock();

        let allocation_size = if dedicated {
            // Force an allocation
            required_size
        } else {
            let selected =
                allocator
                    .borrow()
                    .select_region(memory_type_index, required_size, alignment);

            if let Some(selected) = selected {
                let used_region = allocator.borrow_mut().use_region(
                    selected.allocation,
                    Some(selected.region),
                    selected.aligned_offset,
                    required_size,
                    resource_size,
                    alignment,
                    is_buffer,
                );

                drop(allocator);

                return self.bind_used_region(used_region, selected.aligned_offset, buffer, image);
            }

            // No suitable free regions exist, allocate a new memory region
            {
                let mut a = allocator.borrow_mut();
                if a.allocations_to_defrag.is_empty()
                    && !self.defrag_in_progress.load(Ordering::SeqCst)
                {
                    // Mark currently fragmented allocations for defrag
                    a.mark_allocations_for_defrag();
                }
            }

            if required_size > SMALL_ALLOCATION_THRESHOLD {
                // allocate a page of required size aligned to LARGE_ALLOCATION_INCREMENT increments
                next_highest_alignment(required_size, LARGE_ALLOCATION_INCREMENT)
            } else {
                SMALL_ALLOCATION_SIZE
            }
        };

        // Uh oh, we're out of memory
        let Some(allocation) =
            self.allocate_memory(memory_type_index, allocation_size, is_host_visible)
        else {
            // Responsibility of the caller to handle being out of memory
            return Err(BindError::OutOfMemory);
        };

        let used_region = allocator.borrow_mut().use_region(
            allocation,
            None,
            0,
            required_size,
            resource_size,
            alignment,
            is_buffer,
        );

        drop(allocator);

        self.bind_used_region(used_region, 0, buffer, image)
    }

    /// Bind the buffer or image to a used region, giving the region back if
    /// that fails (the end of `VULKAN_INTERNAL_BindResourceMemory()`).
    fn bind_used_region(
        &self,
        used_region: Arc<UsedRegion>,
        aligned_offset: VkDeviceSize,
        buffer: VkBuffer,
        image: VkImage,
    ) -> std::result::Result<Arc<UsedRegion>, BindError> {
        let result = if buffer != VK_NULL_HANDLE {
            self.bind_buffer_memory(&used_region, aligned_offset, buffer)
        } else {
            self.bind_image_memory(&used_region, aligned_offset, image)
        };
        if result.is_err() {
            self.remove_memory_used_region(&used_region);

            return Err(BindError::Failed);
        }

        Ok(used_region)
    }

    /// Translation of `VULKAN_INTERNAL_BindMemoryForImage()`.
    pub(super) fn bind_memory_for_image(
        &self,
        image: VkImage,
    ) -> std::result::Result<Arc<UsedRegion>, BindError> {
        /* Vulkan memory types have several memory properties.
         *
         * Unlike buffers, images are always optimally stored device-local,
         * so that is the only property we prefer here.
         *
         * If memory is constrained, it is fine for the texture to not
         * be device-local.
         */
        let preferred_memory_property_flags = VK_MEMORY_PROPERTY_DEVICE_LOCAL_BIT;

        let (memory_types_to_try, memory_requirements) =
            self.find_best_image_memory_types(image, preferred_memory_property_flags);

        let mut bind_result = Err(BindError::Failed);
        let mut selected_memory_type_index = 0;
        for &memory_type in &memory_types_to_try {
            bind_result = self.bind_resource_memory(
                memory_type,
                &memory_requirements,
                memory_requirements.size,
                false,
                VK_NULL_HANDLE,
                image,
            );

            if bind_result.is_ok() {
                selected_memory_type_index = memory_type;
                break;
            }
        }

        // Check for warnings on success
        if bind_result.is_ok()
            && !self
                .out_of_device_local_memory_warning
                .load(Ordering::SeqCst)
            && (self.memory_properties.memory_types[selected_memory_type_index as usize]
                .property_flags
                & VK_MEMORY_PROPERTY_DEVICE_LOCAL_BIT)
                == 0
        {
            crate::log::warn!(
                Category::Gpu,
                "Out of device-local memory, allocating textures on host-local memory!"
            );
            self.out_of_device_local_memory_warning
                .store(true, Ordering::SeqCst);
        }

        bind_result
    }

    /// Translation of `VULKAN_INTERNAL_BindMemoryForBuffer()`.
    pub(super) fn bind_memory_for_buffer(
        &self,
        buffer: VkBuffer,
        size: VkDeviceSize,
        ty: VulkanBufferType,
        dedicated: bool,
    ) -> std::result::Result<Arc<UsedRegion>, BindError> {
        let mut required_memory_property_flags = 0;
        let mut preferred_memory_property_flags = 0;
        let mut tolerable_memory_property_flags = 0;

        /* Buffers need to be optimally bound to a memory type
         * based on their use case and the architecture of the system.
         *
         * It is important to understand the distinction between device and host.
         *
         * On a traditional high-performance desktop computer,
         * the "device" would be the GPU, and the "host" would be the CPU.
         * Memory being copied between these two must cross the PCI bus.
         * On these systems we have to be concerned about bandwidth limitations
         * and causing memory stalls, so we have taken a great deal of care
         * to structure this API to guide the client towards optimal usage.
         *
         * Other kinds of devices do not necessarily have this distinction.
         * On an iPhone or Nintendo Switch, all memory is accessible both to the
         * GPU and the CPU at all times. These kinds of systems are known as
         * UMA, or Unified Memory Architecture. A desktop computer using the
         * CPU's integrated graphics can also be thought of as UMA.
         *
         * Vulkan memory types have several memory properties.
         * The relevant memory properties are as follows:
         *
         * DEVICE_LOCAL:
         *   This memory is on-device and most efficient for device access.
         *   On UMA systems all memory is device-local.
         *   If memory is not device-local, then it is host-local.
         *
         * HOST_VISIBLE:
         *   This memory can be mapped for host access, meaning we can obtain
         *   a pointer to directly access the memory.
         *
         * HOST_COHERENT:
         *   Host-coherent memory does not require cache management operations
         *   when mapped, so we always set this alongside HOST_VISIBLE
         *   to avoid extra record keeping.
         *
         * HOST_CACHED:
         *   Host-cached memory is faster to access than uncached memory
         *   but memory of this type might not always be available.
         *
         * GPU buffers, like vertex buffers, indirect buffers, etc
         * are optimally stored in device-local memory.
         * However, if device-local memory is low, these buffers
         * can be accessed from host-local memory with a performance penalty.
         *
         * Uniform buffers must be host-visible and coherent because
         * the client uses them to quickly push small amounts of data.
         * We prefer uniform buffers to also be device-local because
         * they are accessed by shaders, but the amount of memory
         * that is both device-local and host-visible
         * is often constrained, particularly on low-end devices.
         *
         * Transfer buffers must be host-visible and coherent because
         * the client uses them to stage data to be transferred
         * to device-local memory, or to read back data transferred
         * from the device. We prefer the cache bit for performance
         * but it isn't strictly necessary. We tolerate device-local
         * memory in this situation because, as mentioned above,
         * on certain devices all memory is device-local, and even
         * though the transfer isn't strictly necessary it is still
         * useful for correctly timelining data.
         */
        // (VulkanBufferType has no other values: upstream's "Unrecognized
        // buffer type!" can't happen.)
        match ty {
            VulkanBufferType::Gpu => {
                preferred_memory_property_flags |= VK_MEMORY_PROPERTY_DEVICE_LOCAL_BIT;
            }
            VulkanBufferType::Uniform => {
                required_memory_property_flags |=
                    VK_MEMORY_PROPERTY_HOST_VISIBLE_BIT | VK_MEMORY_PROPERTY_HOST_COHERENT_BIT;

                preferred_memory_property_flags |= VK_MEMORY_PROPERTY_DEVICE_LOCAL_BIT;
            }
            VulkanBufferType::Transfer => {
                required_memory_property_flags |=
                    VK_MEMORY_PROPERTY_HOST_VISIBLE_BIT | VK_MEMORY_PROPERTY_HOST_COHERENT_BIT;

                preferred_memory_property_flags |= VK_MEMORY_PROPERTY_HOST_CACHED_BIT;

                tolerable_memory_property_flags |= VK_MEMORY_PROPERTY_DEVICE_LOCAL_BIT;
            }
        }

        let (memory_types_to_try, memory_requirements) = self.find_best_buffer_memory_types(
            buffer,
            required_memory_property_flags,
            preferred_memory_property_flags,
            tolerable_memory_property_flags,
        );

        let mut bind_result = Err(BindError::Failed);
        let mut selected_memory_type_index = 0;
        for &memory_type in &memory_types_to_try {
            bind_result = self.bind_resource_memory(
                memory_type,
                &memory_requirements,
                size,
                dedicated,
                buffer,
                VK_NULL_HANDLE,
            );

            if bind_result.is_ok() {
                selected_memory_type_index = memory_type;
                break;
            }
        }

        // Check for warnings on success
        if bind_result.is_ok() {
            let device_local = (self.memory_properties.memory_types
                [selected_memory_type_index as usize]
                .property_flags
                & VK_MEMORY_PROPERTY_DEVICE_LOCAL_BIT)
                == VK_MEMORY_PROPERTY_DEVICE_LOCAL_BIT;
            match ty {
                VulkanBufferType::Gpu => {
                    if !self
                        .out_of_device_local_memory_warning
                        .load(Ordering::SeqCst)
                        && !device_local
                    {
                        crate::log::warn!(
                            Category::Gpu,
                            "Out of device-local memory, allocating buffers on host-local memory, expect degraded performance!"
                        );
                        self.out_of_device_local_memory_warning
                            .store(true, Ordering::SeqCst);
                    }
                }
                VulkanBufferType::Uniform => {
                    if !self.outof_bar_memory_warning.load(Ordering::SeqCst) && !device_local {
                        crate::log::warn!(
                            Category::Gpu,
                            "Out of BAR memory, allocating uniform buffers on host-local memory, expect degraded performance!"
                        );
                        self.outof_bar_memory_warning.store(true, Ordering::SeqCst);
                    }
                }
                VulkanBufferType::Transfer => {
                    if !self.integrated_memory_notification.load(Ordering::SeqCst) && device_local {
                        crate::log::info!(
                            Category::Gpu,
                            "Integrated memory detected, allocating TransferBuffers on device-local memory!"
                        );
                        self.integrated_memory_notification
                            .store(true, Ordering::SeqCst);
                    }
                }
            }
        }

        bind_result
    }
}
