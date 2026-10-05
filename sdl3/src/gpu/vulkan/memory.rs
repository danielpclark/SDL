// Rust translation of the memory allocator of src/gpu/vulkan/SDL_gpu_vulkan.c
// from Simple DirectMedia Layer.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! The memory allocator: device memory is allocated in large blocks per
//! memory type (`VulkanMemoryAllocation`), which buffers and images are
//! suballocated from. Each allocation keeps its used regions and its free
//! regions; each memory type's suballocator keeps all its free regions
//! sorted by size, largest first, which the search for a region walks from
//! the smallest. Freed regions merge with their neighbours. Allocations
//! that fragment are marked for defragmentation (see
//! `VulkanRenderer::defragment_memory`).
//!
//! Upstream's pointer graph becomes keys: the allocations and the free
//! regions live in arenas of the [`MemoryAllocator`] (guarded by the
//! renderer's `allocatorLock`), and an allocation's used regions are
//! [`UsedRegion`]s shared with the buffer or texture bound to them, which
//! keep the allocation's memory handle, mapping and locks
//! ([`MemoryAllocation`]). The bookkeeping is the same as upstream's, down
//! to the order of the lists, so the same regions get picked.

use std::sync::atomic::AtomicI32;
use std::sync::{Arc, Mutex, OnceLock, Weak};

use super::resources::{VulkanBuffer, VulkanTexture};
use crate::video::vk::*;

/// Allocations of this size or less are made in small allocations.
/// Translation of `SMALL_ALLOCATION_THRESHOLD` (2 MiB).
pub(super) const SMALL_ALLOCATION_THRESHOLD: VkDeviceSize = 2097152;
/// The size of a small allocation. Translation of `SMALL_ALLOCATION_SIZE`
/// (16 MiB).
pub(super) const SMALL_ALLOCATION_SIZE: VkDeviceSize = 16777216;
/// Large allocations are sized in multiples of this. Translation of
/// `LARGE_ALLOCATION_INCREMENT` (64 MiB).
pub(super) const LARGE_ALLOCATION_INCREMENT: VkDeviceSize = 67108864;

/// `n` rounded up to a multiple of `align`. Translation of
/// `VULKAN_INTERNAL_NextHighestAlignment()`.
pub(super) fn next_highest_alignment(n: VkDeviceSize, align: VkDeviceSize) -> VkDeviceSize {
    align.wrapping_mul(n.wrapping_add(align).wrapping_sub(1) / align)
}

/// Translation of `VULKAN_INTERNAL_NextHighestAlignment32()`.
#[allow(dead_code)] // (part 2: uniform buffer offsets)
pub(super) fn next_highest_alignment32(n: u32, align: u32) -> u32 {
    align.wrapping_mul(n.wrapping_add(align).wrapping_sub(1) / align)
}

/// A minimal arena: values addressed by the index they were stored at,
/// whose slots are reused once removed.
#[derive(Debug)]
struct Slab<T> {
    slots: Vec<Option<T>>,
    vacant: Vec<usize>,
}

impl<T> Slab<T> {
    const fn new() -> Slab<T> {
        Slab {
            slots: Vec::new(),
            vacant: Vec::new(),
        }
    }

    fn insert(&mut self, value: T) -> usize {
        match self.vacant.pop() {
            Some(i) => {
                self.slots[i] = Some(value);
                i
            }
            None => {
                self.slots.push(Some(value));
                self.slots.len() - 1
            }
        }
    }

    /// The index the next [`Slab::insert`] stores at.
    fn next_key(&self) -> usize {
        self.vacant.last().copied().unwrap_or(self.slots.len())
    }

    fn remove(&mut self, i: usize) -> T {
        let value = self.slots[i].take().expect("vacant slab slot");
        self.vacant.push(i);
        value
    }

    fn get(&self, i: usize) -> &T {
        self.slots[i].as_ref().expect("vacant slab slot")
    }

    fn get_mut(&mut self, i: usize) -> &mut T {
        self.slots[i].as_mut().expect("vacant slab slot")
    }
}

/// An allocation in the allocator (`VulkanMemoryAllocation *`).
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub(super) struct AllocationKey(usize);

/// A free region in the allocator (`VulkanMemoryFreeRegion *`).
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub(super) struct FreeRegionKey(usize);

/// The part of a `VulkanMemoryAllocation` that its users share: the device
/// memory, its persistent mapping and the locks.
#[derive(Debug)]
pub(super) struct MemoryAllocation {
    pub(super) memory: VkDeviceMemory,
    pub(super) size: VkDeviceSize,
    /// The persistent mapping of host-visible memory (`mapPointer`).
    map_pointer: Option<std::ptr::NonNull<u8>>,
    /// `memoryLock`: binds to the memory are serialized.
    pub(super) memory_lock: Mutex<()>,
    /// Used to avoid defrag races
    pub(super) reference_count: AtomicI32,
    key: AllocationKey,
}

// SAFETY: the mapping is the device memory's, valid while the allocation
// lives; the backend only hands it out as the transfer buffers' memory,
// whose accesses the front end serializes (`map_transfer_buffer`).
unsafe impl Send for MemoryAllocation {}
// SAFETY: as for Send; the other fields are thread-safe.
unsafe impl Sync for MemoryAllocation {}

impl MemoryAllocation {
    /// The mapping of host-visible memory (`mapPointer`).
    pub(super) fn map_pointer(&self) -> Option<std::ptr::NonNull<u8>> {
        self.map_pointer
    }
}

/// The buffer or texture a used region is bound to (`vulkanBuffer` /
/// `vulkanTexture`).
#[derive(Debug)]
pub(super) enum RegionResource {
    Buffer(Weak<VulkanBuffer>),
    Texture(Weak<VulkanTexture>),
}

/// A region of an allocation bound to a buffer or texture. Translation of
/// `VulkanMemoryUsedRegion`.
#[derive(Debug)]
pub(super) struct UsedRegion {
    pub(super) allocation: Arc<MemoryAllocation>,
    pub(super) offset: VkDeviceSize,
    pub(super) size: VkDeviceSize,
    /// differs from offset based on alignment
    pub(super) resource_offset: VkDeviceSize,
    /// differs from size based on alignment
    pub(super) resource_size: VkDeviceSize,
    #[allow(dead_code)] // (kept as upstream does)
    pub(super) alignment: VkDeviceSize,
    pub(super) is_buffer: bool,
    /// Set once the resource is created (upstream's `vulkanBuffer` /
    /// `vulkanTexture` union).
    pub(super) resource: OnceLock<RegionResource>,
}

/// Translation of `VulkanMemoryFreeRegion`.
#[derive(Debug)]
struct FreeRegion {
    allocation: AllocationKey,
    offset: VkDeviceSize,
    size: VkDeviceSize,
    allocation_index: usize,
    sorted_index: usize,
}

/// The allocator's bookkeeping of a `VulkanMemoryAllocation`.
#[derive(Debug)]
struct AllocationState {
    shared: Arc<MemoryAllocation>,
    /// The suballocator (memory type) of the allocation (`allocator`).
    sub_allocator: usize,
    used_regions: Vec<Arc<UsedRegion>>,
    free_regions: Vec<FreeRegionKey>,
    available_for_allocation: bool,
    free_space: VkDeviceSize,
    used_space: VkDeviceSize,
}

/// Translation of `VulkanMemorySubAllocator`.
#[derive(Debug)]
struct SubAllocator {
    #[allow(dead_code)] // (kept as upstream does)
    memory_type_index: u32,
    allocations: Vec<AllocationKey>,
    sorted_free_regions: Vec<FreeRegionKey>,
}

/// The allocator: one suballocator per memory type (translation of
/// `VulkanMemoryAllocator`), with the renderer's state that
/// `allocatorLock` guards (`allocationsToDefrag`, `checkEmptyAllocations`).
#[derive(Debug)]
pub(super) struct MemoryAllocator {
    sub_allocators: Vec<SubAllocator>,
    allocations: Slab<AllocationState>,
    free_regions: Slab<FreeRegion>,
    pub(super) allocations_to_defrag: Vec<AllocationKey>,
    pub(super) check_empty_allocations: bool,
}

/// What [`MemoryAllocator::select_region`] found for a resource.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct SelectedRegion {
    pub(super) allocation: AllocationKey,
    pub(super) region: FreeRegionKey,
    pub(super) aligned_offset: VkDeviceSize,
}

/// The state of an allocation, for the tests.
#[derive(Clone, Debug, PartialEq, Eq)]
#[cfg(test)]
pub(super) struct AllocationInfo {
    pub(super) key: AllocationKey,
    pub(super) size: VkDeviceSize,
    pub(super) available_for_allocation: bool,
    pub(super) free_space: VkDeviceSize,
    pub(super) used_space: VkDeviceSize,
    /// (offset, size) of the used regions, in the allocation's order.
    pub(super) used_regions: Vec<(VkDeviceSize, VkDeviceSize)>,
    /// (offset, size) of the free regions, in the allocation's order.
    pub(super) free_regions: Vec<(VkDeviceSize, VkDeviceSize)>,
}

impl MemoryAllocator {
    /// The allocator with its `VK_MAX_MEMORY_TYPES` empty suballocators
    /// (from `VULKAN_CreateDevice()`).
    pub(super) fn new() -> MemoryAllocator {
        MemoryAllocator {
            sub_allocators: (0..VK_MAX_MEMORY_TYPES as u32)
                .map(|i| SubAllocator {
                    memory_type_index: i,
                    allocations: Vec::new(),
                    sorted_free_regions: Vec::with_capacity(4),
                })
                .collect(),
            allocations: Slab::new(),
            free_regions: Slab::new(),
            allocations_to_defrag: Vec::with_capacity(4),
            check_empty_allocations: false,
        }
    }

    /// Take an allocation's free regions out of its suballocator's sorted
    /// list (it is being defragmented). Translation of
    /// `VULKAN_INTERNAL_MakeMemoryUnavailable()`.
    pub(super) fn make_memory_unavailable(&mut self, key: AllocationKey) {
        let allocation = self.allocations.get_mut(key.0);
        allocation.available_for_allocation = false;
        let sub = allocation.sub_allocator;
        let free_regions = allocation.free_regions.clone();

        for free_region in free_regions {
            let sorted_index = self.free_regions.get(free_region.0).sorted_index;
            let sorted = &mut self.sub_allocators[sub].sorted_free_regions;

            // close the gap in the sorted list
            if sorted.len() > 1 {
                for j in sorted_index..sorted.len() - 1 {
                    sorted[j] = sorted[j + 1];
                    self.free_regions.get_mut(sorted[j].0).sorted_index = j;
                }
            }

            sorted.pop();
        }
    }

    /// Mark the allocations with more than one free region for defrag.
    /// Translation of `VULKAN_INTERNAL_MarkAllocationsForDefrag()`.
    pub(super) fn mark_allocations_for_defrag(&mut self) {
        for memory_type in 0..VK_MAX_MEMORY_TYPES {
            for allocation_index in 0..self.sub_allocators[memory_type].allocations.len() {
                let key = self.sub_allocators[memory_type].allocations[allocation_index];
                let allocation = self.allocations.get(key.0);
                if allocation.available_for_allocation && allocation.free_regions.len() > 1 {
                    self.allocations_to_defrag.push(key);

                    self.make_memory_unavailable(key);
                }
            }
        }
    }

    /// Translation of `VULKAN_INTERNAL_RemoveMemoryFreeRegion()`.
    fn remove_memory_free_region(&mut self, free_region_key: FreeRegionKey) {
        let free_region = self.free_regions.remove(free_region_key.0);
        let allocation = self.allocations.get_mut(free_region.allocation.0);
        let sub = allocation.sub_allocator;

        if allocation.available_for_allocation {
            let sorted = &mut self.sub_allocators[sub].sorted_free_regions;
            // close the gap in the sorted list
            if sorted.len() > 1 {
                for i in free_region.sorted_index..sorted.len() - 1 {
                    sorted[i] = sorted[i + 1];
                    self.free_regions.get_mut(sorted[i].0).sorted_index = i;
                }
            }

            sorted.pop();
        }

        // close the gap in the buffer list
        let allocation = self.allocations.get_mut(free_region.allocation.0);
        let count = allocation.free_regions.len();
        if count > 1 && free_region.allocation_index != count - 1 {
            let last = allocation.free_regions[count - 1];
            allocation.free_regions[free_region.allocation_index] = last;
            self.free_regions.get_mut(last.0).allocation_index = free_region.allocation_index;
        }

        let allocation = self.allocations.get_mut(free_region.allocation.0);
        allocation.free_regions.pop();

        allocation.free_space -= free_region.size;
    }

    /// Add a free region, merged with an adjacent one. Translation of
    /// `VULKAN_INTERNAL_NewMemoryFreeRegion()`.
    pub(super) fn new_memory_free_region(
        &mut self,
        key: AllocationKey,
        offset: VkDeviceSize,
        size: VkDeviceSize,
    ) {
        // look for an adjacent region to merge
        let free_regions = self.allocations.get(key.0).free_regions.clone();
        for &region_key in free_regions.iter().rev() {
            let region = self.free_regions.get(region_key.0);

            // check left side
            if region.offset + region.size == offset {
                let new_offset = region.offset;
                let new_size = region.size + size;

                self.remove_memory_free_region(region_key);
                self.new_memory_free_region(key, new_offset, new_size);
                return;
            }

            // check right side
            if region.offset == offset + size {
                let new_offset = offset;
                let new_size = region.size + size;

                self.remove_memory_free_region(region_key);
                self.new_memory_free_region(key, new_offset, new_size);
                return;
            }
        }

        // region is not contiguous with another free region, make a new one
        let allocation = self.allocations.get(key.0);
        let allocation_index = allocation.free_regions.len();
        let sub = allocation.sub_allocator;
        let available = allocation.available_for_allocation;
        let new_free_region = FreeRegionKey(self.free_regions.insert(FreeRegion {
            allocation: key,
            offset,
            size,
            allocation_index,
            sorted_index: 0,
        }));

        let allocation = self.allocations.get_mut(key.0);
        allocation.free_space += size;
        allocation.free_regions.push(new_free_region);

        if available {
            let mut insertion_index = 0;
            for &sorted in &self.sub_allocators[sub].sorted_free_regions {
                if self.free_regions.get(sorted.0).size < size {
                    // this is where the new region should go
                    break;
                }

                insertion_index += 1;
            }

            // perform insertion sort
            let sorted = &mut self.sub_allocators[sub].sorted_free_regions;
            sorted.insert(insertion_index, new_free_region);
            for (i, region) in sorted.iter().enumerate().skip(insertion_index) {
                self.free_regions.get_mut(region.0).sorted_index = i;
            }
        }
    }

    /// Translation of `VULKAN_INTERNAL_NewMemoryUsedRegion()`.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn new_memory_used_region(
        &mut self,
        key: AllocationKey,
        offset: VkDeviceSize,
        size: VkDeviceSize,
        resource_offset: VkDeviceSize,
        resource_size: VkDeviceSize,
        alignment: VkDeviceSize,
        is_buffer: bool,
    ) -> Arc<UsedRegion> {
        let allocation = self.allocations.get_mut(key.0);
        let memory_used_region = Arc::new(UsedRegion {
            allocation: allocation.shared.clone(),
            offset,
            size,
            resource_offset,
            resource_size,
            alignment,
            is_buffer,
            resource: OnceLock::new(),
        });

        allocation.used_space += size;

        allocation.used_regions.push(memory_used_region.clone());

        memory_used_region
    }

    /// Give a used region back to its allocation. Translation of
    /// `VULKAN_INTERNAL_RemoveMemoryUsedRegion()`.
    pub(super) fn remove_memory_used_region(&mut self, used_region: &Arc<UsedRegion>) {
        let key = used_region.allocation.key;
        let allocation = self.allocations.get_mut(key.0);

        if let Some(i) = allocation
            .used_regions
            .iter()
            .position(|r| Arc::ptr_eq(r, used_region))
        {
            // plug the hole
            allocation.used_regions.swap_remove(i);
        }

        allocation.used_space -= used_region.size;

        self.new_memory_free_region(key, used_region.offset, used_region.size);

        if self.allocations.get(key.0).used_regions.is_empty() {
            self.check_empty_allocations = true;
        }
    }

    /// Add a new allocation (`vkAllocateMemory` succeeded) to its
    /// suballocator, with one free region over all of it unless `free` is
    /// false. The bookkeeping half of `VULKAN_INTERNAL_AllocateMemory()`.
    pub(super) fn add_allocation(
        &mut self,
        memory_type_index: u32,
        allocation_size: VkDeviceSize,
        memory: VkDeviceMemory,
        map_pointer: Option<std::ptr::NonNull<u8>>,
        free: bool,
    ) -> AllocationKey {
        let key = AllocationKey(self.allocations.next_key());
        let shared = Arc::new(MemoryAllocation {
            memory,
            size: allocation_size,
            map_pointer,
            memory_lock: Mutex::new(()),
            reference_count: AtomicI32::new(0),
            key,
        });
        let inserted = self.allocations.insert(AllocationState {
            shared,
            sub_allocator: memory_type_index as usize,
            used_regions: Vec::with_capacity(1),
            free_regions: Vec::with_capacity(1),
            available_for_allocation: true,
            free_space: 0, // added by FreeRegions
            used_space: 0, // added by UsedRegions
        });
        debug_assert_eq!(inserted, key.0);

        self.sub_allocators[memory_type_index as usize]
            .allocations
            .push(key);

        if free {
            self.new_memory_free_region(key, 0, allocation_size);
        }

        key
    }

    /// Remove the `allocation_index`th allocation of a memory type, which
    /// has no used regions left, and return its memory for the caller to
    /// free (`vkFreeMemory`). The bookkeeping of
    /// `VULKAN_INTERNAL_DeallocateMemory()`.
    pub(super) fn deallocate_memory(
        &mut self,
        memory_type_index: usize,
        allocation_index: usize,
    ) -> VkDeviceMemory {
        let key = self.sub_allocators[memory_type_index].allocations[allocation_index];

        // If this allocation was marked for defrag, cancel that
        if let Some(i) = self.allocations_to_defrag.iter().position(|&k| k == key) {
            self.allocations_to_defrag.swap_remove(i);
        }

        // Note (upstream): C removes the free regions in a loop that skips
        // the ones the removals move down; with no used regions left the
        // free regions have merged into one, so they all go either way.
        while let Some(&region) = self.allocations.get(key.0).free_regions.first() {
            self.remove_memory_free_region(region);
        }

        /* no need to iterate used regions because deallocate
         * only happens when there are 0 used regions
         */
        let allocation = self.allocations.remove(key.0);

        self.sub_allocators[memory_type_index]
            .allocations
            .swap_remove(allocation_index);

        allocation.shared.memory
    }

    /// The search of `VULKAN_INTERNAL_BindResourceMemory()` for a free
    /// region with room for `required_size` bytes at `alignment`, among the
    /// small allocations for a small resource and the others for the
    /// others; the region is then split by [`MemoryAllocator::use_region`].
    pub(super) fn select_region(
        &self,
        memory_type_index: u32,
        required_size: VkDeviceSize,
        alignment: VkDeviceSize,
    ) -> Option<SelectedRegion> {
        let small_allocation = required_size <= SMALL_ALLOCATION_THRESHOLD;
        let allocator = &self.sub_allocators[memory_type_index as usize];

        // Search for a suitable existing free region
        for &region_key in allocator.sorted_free_regions.iter().rev() {
            let region = self.free_regions.get(region_key.0);
            let allocation_size = self.allocations.get(region.allocation.0).shared.size;

            if small_allocation && allocation_size != SMALL_ALLOCATION_SIZE {
                // region is not in a small allocation
                continue;
            }

            if !small_allocation && allocation_size == SMALL_ALLOCATION_SIZE {
                // allocation is not small and current region is in a small allocation
                continue;
            }

            let aligned_offset = next_highest_alignment(region.offset, alignment);

            if aligned_offset + required_size <= region.offset + region.size {
                return Some(SelectedRegion {
                    allocation: region.allocation,
                    region: region_key,
                    aligned_offset,
                });
            }
        }
        None
    }

    /// Make a used region of `required_size` bytes at `aligned_offset` in
    /// the free region `region` (from [`MemoryAllocator::select_region`],
    /// or the first free region of a new allocation for `None`), and put
    /// the rest back as a free region. From
    /// `VULKAN_INTERNAL_BindResourceMemory()`.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn use_region(
        &mut self,
        allocation: AllocationKey,
        region: Option<FreeRegionKey>,
        aligned_offset: VkDeviceSize,
        required_size: VkDeviceSize,
        resource_size: VkDeviceSize,
        alignment: VkDeviceSize,
        is_buffer: bool,
    ) -> Arc<UsedRegion> {
        let region_key = match region {
            Some(region) => region,
            None => self.allocations.get(allocation.0).free_regions[0],
        };
        let (region_offset, region_size) = {
            let region = self.free_regions.get(region_key.0);
            (region.offset, region.size)
        };

        let used_region = self.new_memory_used_region(
            allocation,
            region_offset,
            required_size + (aligned_offset - region_offset),
            aligned_offset,
            resource_size,
            alignment,
            is_buffer,
        );

        let new_region_size = region_size - ((aligned_offset - region_offset) + required_size);
        let new_region_offset = aligned_offset + required_size;

        // remove and add modified region to re-sort
        self.remove_memory_free_region(region_key);

        // if size is 0, no need to re-insert
        if new_region_size != 0 {
            self.new_memory_free_region(allocation, new_region_offset, new_region_size);
        }

        used_region
    }

    /// The number of allocations of a memory type.
    pub(super) fn allocation_count(&self, memory_type_index: usize) -> usize {
        self.sub_allocators[memory_type_index].allocations.len()
    }

    /// The `index`th allocation of a memory type.
    pub(super) fn allocation_key(&self, memory_type_index: usize, index: usize) -> AllocationKey {
        self.sub_allocators[memory_type_index].allocations[index]
    }

    /// The used regions of an allocation.
    pub(super) fn used_regions(&self, key: AllocationKey) -> Vec<Arc<UsedRegion>> {
        self.allocations.get(key.0).used_regions.clone()
    }

    /// The shared part of an allocation.
    pub(super) fn allocation(&self, key: AllocationKey) -> &Arc<MemoryAllocation> {
        &self.allocations.get(key.0).shared
    }

    /// The state of an allocation.
    #[cfg(test)]
    pub(super) fn allocation_info(&self, key: AllocationKey) -> AllocationInfo {
        let a = self.allocations.get(key.0);
        AllocationInfo {
            key,
            size: a.shared.size,
            available_for_allocation: a.available_for_allocation,
            free_space: a.free_space,
            used_space: a.used_space,
            used_regions: a.used_regions.iter().map(|r| (r.offset, r.size)).collect(),
            free_regions: a
                .free_regions
                .iter()
                .map(|r| {
                    let r = self.free_regions.get(r.0);
                    (r.offset, r.size)
                })
                .collect(),
        }
    }

    /// The sorted free regions of a memory type, as (allocation, offset,
    /// size), largest first.
    #[cfg(test)]
    pub(super) fn sorted_free_regions(
        &self,
        memory_type_index: usize,
    ) -> Vec<(AllocationKey, VkDeviceSize, VkDeviceSize)> {
        self.sub_allocators[memory_type_index]
            .sorted_free_regions
            .iter()
            .map(|r| {
                let r = self.free_regions.get(r.0);
                (r.allocation, r.offset, r.size)
            })
            .collect()
    }

    /// Check the indices the free regions keep of their places in the lists.
    #[cfg(test)]
    pub(super) fn check_indices(&self) {
        for (sub_index, sub) in self.sub_allocators.iter().enumerate() {
            for (i, r) in sub.sorted_free_regions.iter().enumerate() {
                let region = self.free_regions.get(r.0);
                assert_eq!(region.sorted_index, i);
                assert_eq!(
                    self.allocations.get(region.allocation.0).sub_allocator,
                    sub_index
                );
            }
            for w in sub.sorted_free_regions.windows(2) {
                assert!(self.free_regions.get(w[0].0).size >= self.free_regions.get(w[1].0).size);
            }
            for key in &sub.allocations {
                let a = self.allocations.get(key.0);
                for (i, r) in a.free_regions.iter().enumerate() {
                    assert_eq!(self.free_regions.get(r.0).allocation_index, i);
                }
                let free: VkDeviceSize = a
                    .free_regions
                    .iter()
                    .map(|r| self.free_regions.get(r.0).size)
                    .sum();
                assert_eq!(free, a.free_space);
                let used: VkDeviceSize = a.used_regions.iter().map(|r| r.size).sum();
                assert_eq!(used, a.used_space);
            }
        }
    }
}
