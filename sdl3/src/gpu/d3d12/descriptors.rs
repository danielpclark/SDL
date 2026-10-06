// Rust translation of the descriptor heap parts of src/gpu/d3d12/SDL_gpu_d3d12.c
// from Simple DirectMedia Layer: descriptor heaps, the staging descriptor
// pools and the pools of shader-visible heaps.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! Descriptor heaps. Resources keep their views in *staging* descriptors:
//! slots of CPU-only heaps, one pool of them per heap type, which grows a
//! heap at a time. Command buffers copy the descriptors they bind into
//! shader-visible heaps, which come from a pool per heap type (CBV/SRV/UAV
//! and sampler).
//!
//! Upstream's `D3D12StagingDescriptor` is a copy of a free slot that the
//! resource owning it hands back with `ReleaseStagingDescriptorHandle()`.
//! Here a [`StagingDescriptor`] owns its slot and gives it back to its pool
//! when it is dropped; a resource's descriptors go when the resource is
//! destroyed, as upstream releases them. The pools only keep the free
//! slots' CPU handles (`freeDescriptors`), which is all upstream reads of
//! them.

use std::sync::{Arc, Mutex};

use super::d3d::*;
use super::{d3d12_error, lock, D3D12Renderer, STAGING_HEAP_DESCRIPTOR_COUNT};
use crate::error::Result;

/// Translation of `D3D12DescriptorHeap`.
#[derive(Debug)]
pub(super) struct DescriptorHeap {
    #[allow(dead_code)] // (part 2: binding; it owns the heap)
    pub(super) handle: D3d12DescriptorHeap,
    pub(super) heap_type: u32,
    pub(super) descriptor_heap_cpu_start: CpuDescriptorHandle,
    /// only used by GPU heaps
    #[allow(dead_code)] // (part 2: binding)
    pub(super) descriptor_heap_gpu_start: GpuDescriptorHandle,
    #[allow(dead_code)] // (part 2: binding)
    pub(super) max_descriptors: u32,
    pub(super) descriptor_size: u32,
    #[allow(dead_code)] // (kept as upstream does)
    pub(super) staging: bool,

    /// only used by GPU heaps
    pub(super) current_descriptor_index: u32,
}

/// Create a descriptor heap: a CPU-only one for `staging`, else a
/// shader-visible one. Translation of `D3D12_INTERNAL_CreateDescriptorHeap()`
/// (with the device and debug mode, as the pools are made while the
/// renderer is).
pub(super) fn create_descriptor_heap(
    device: &D3d12Device,
    debug_mode: bool,
    heap_type: u32,
    descriptor_count: u32,
    staging: bool,
) -> Result<DescriptorHeap> {
    let heap_desc = DescriptorHeapDesc {
        num_descriptors: descriptor_count,
        ty: heap_type,
        flags: if staging {
            D3D12_DESCRIPTOR_HEAP_FLAG_NONE
        } else {
            D3D12_DESCRIPTOR_HEAP_FLAG_SHADER_VISIBLE
        },
        node_mask: 0,
    };

    let handle = device.create_descriptor_heap(&heap_desc).map_err(|res| {
        d3d12_error(
            debug_mode,
            Some(device),
            "Failed to create descriptor heap!",
            res,
        )
    })?;

    let descriptor_heap_cpu_start = handle.cpu_descriptor_handle_for_heap_start();
    let descriptor_heap_gpu_start = if !staging {
        handle.gpu_descriptor_handle_for_heap_start()
    } else {
        GpuDescriptorHandle::default()
    };
    Ok(DescriptorHeap {
        handle,
        heap_type,
        descriptor_heap_cpu_start,
        descriptor_heap_gpu_start,
        max_descriptors: descriptor_count,
        descriptor_size: device.descriptor_handle_increment_size(heap_type),
        staging,
        current_descriptor_index: 0,
    })
}

/// The only thing we care about with staging descriptors is being able to
/// grab a free descriptor. Translation of `D3D12StagingDescriptorPool`.
#[derive(Debug)]
pub(super) struct StagingDescriptorPool {
    /// The type of its heaps (upstream reads `heaps[0]->heapType`).
    pub(super) heap_type: u32,
    /// The heaps and free descriptors, with the pool's `lock`.
    pub(super) state: Mutex<StagingDescriptorPoolState>,
}

/// The heaps and free slots of a [`StagingDescriptorPool`].
#[derive(Debug)]
pub(super) struct StagingDescriptorPoolState {
    pub(super) heaps: Vec<DescriptorHeap>,
    /// Descriptor handles are owned by resources, so these can be thought
    /// of as descriptions of a free index within a heap.
    pub(super) free_descriptors: Vec<CpuDescriptorHandle>,
}

impl StagingDescriptorPoolState {
    /// Add `heap`'s slots to the free ones (the loops of
    /// `CreateStagingDescriptorPool()` and `ExpandStagingDescriptorPool()`).
    fn add_heap(&mut self, heap: DescriptorHeap) {
        for i in 0..STAGING_HEAP_DESCRIPTOR_COUNT as usize {
            self.free_descriptors.push(CpuDescriptorHandle {
                ptr: heap.descriptor_heap_cpu_start.ptr + i * heap.descriptor_size as usize,
            });
        }
        self.heaps.push(heap);
    }
}

/// Translation of `D3D12_INTERNAL_CreateStagingDescriptorPool()`.
pub(super) fn create_staging_descriptor_pool(
    device: &D3d12Device,
    debug_mode: bool,
    heap_type: u32,
) -> Result<Arc<StagingDescriptorPool>> {
    let heap = create_descriptor_heap(
        device,
        debug_mode,
        heap_type,
        STAGING_HEAP_DESCRIPTOR_COUNT,
        true,
    )?;

    let mut state = StagingDescriptorPoolState {
        heaps: Vec::with_capacity(1),
        free_descriptors: Vec::with_capacity(STAGING_HEAP_DESCRIPTOR_COUNT as usize),
    };
    state.add_heap(heap);

    Ok(Arc::new(StagingDescriptorPool {
        heap_type,
        state: Mutex::new(state),
    }))
}

/// A descriptor slot of a staging heap, owned by the resource whose view
/// it holds, and given back to its pool when dropped (upstream's
/// `D3D12_INTERNAL_ReleaseStagingDescriptorHandle()`). Translation of
/// `D3D12StagingDescriptor`; upstream's "no descriptor" (a NULL `heap`) is
/// `Option::None`.
#[derive(Debug)]
pub(super) struct StagingDescriptor {
    pool: Arc<StagingDescriptorPool>,
    pub(super) cpu_handle: CpuDescriptorHandle,
}

impl Drop for StagingDescriptor {
    /// Translation of `D3D12_INTERNAL_ReleaseStagingDescriptorHandle()`.
    fn drop(&mut self) {
        lock(&self.pool.state)
            .free_descriptors
            .push(self.cpu_handle);
    }
}

/// A pool of shader-visible heaps of one type. Translation of
/// `D3D12GPUDescriptorHeapPool` (its `lock` is the mutex; the heaps
/// acquired by command buffers aren't in it).
#[derive(Debug)]
pub(super) struct GpuDescriptorHeapPool {
    pub(super) heaps: Mutex<Vec<DescriptorHeap>>,
}

impl D3D12Renderer {
    /// Take a free slot of the staging heaps of `heap_type`, adding a heap
    /// to the pool when none is left. Translation of
    /// `D3D12_INTERNAL_AssignStagingDescriptorHandle()`.
    pub(super) fn assign_staging_descriptor_handle(
        &self,
        heap_type: u32,
    ) -> Result<StagingDescriptor> {
        let pool = &self.staging_descriptor_pools[heap_type as usize];

        let mut state = lock(&pool.state);

        if state.free_descriptors.is_empty() {
            self.expand_staging_descriptor_pool(pool, &mut state)?;
        }

        let cpu_handle = state
            .free_descriptors
            .pop()
            .expect("a staging descriptor pool was just expanded");

        Ok(StagingDescriptor {
            pool: pool.clone(),
            cpu_handle,
        })
    }

    /// If the pool is empty, we need to refill it! Translation of
    /// `D3D12_INTERNAL_ExpandStagingDescriptorPool()` (the caller holds the
    /// pool's lock).
    fn expand_staging_descriptor_pool(
        &self,
        pool: &StagingDescriptorPool,
        state: &mut StagingDescriptorPoolState,
    ) -> Result<()> {
        let heap = create_descriptor_heap(
            &self.device,
            self.debug_mode,
            pool.heap_type,
            STAGING_HEAP_DESCRIPTOR_COUNT,
            true,
        )?;

        state.add_heap(heap);

        Ok(())
    }

    /// A shader-visible heap of `descriptor_heap_type` (CBV/SRV/UAV or
    /// sampler) from its pool, or a new one. Translation of
    /// `D3D12_INTERNAL_AcquireGPUDescriptorHeapFromPool()` but the tracking
    /// in the command buffer, which part 2 adds.
    ///
    /// Note (upstream): C goes on with a NULL heap when a new one can't be
    /// made; the error is returned here.
    #[cfg_attr(not(test), allow(dead_code))] // (part 2: command buffers)
    pub(super) fn acquire_gpu_descriptor_heap_from_pool(
        &self,
        descriptor_heap_type: u32,
    ) -> Result<DescriptorHeap> {
        let pool = &self.gpu_descriptor_heap_pools[descriptor_heap_type as usize];

        let mut heaps = lock(&pool.heaps);
        match heaps.pop() {
            Some(heap) => Ok(heap),
            None => create_descriptor_heap(
                &self.device,
                self.debug_mode,
                descriptor_heap_type,
                if descriptor_heap_type == D3D12_DESCRIPTOR_HEAP_TYPE_CBV_SRV_UAV {
                    super::VIEW_GPU_DESCRIPTOR_COUNT
                } else {
                    super::SAMPLER_GPU_DESCRIPTOR_COUNT
                },
                false,
            ),
        }
    }

    /// Translation of `D3D12_INTERNAL_ReturnGPUDescriptorHeapToPool()`.
    #[cfg_attr(not(test), allow(dead_code))] // (part 2: command buffers)
    pub(super) fn return_gpu_descriptor_heap_to_pool(&self, mut heap: DescriptorHeap) {
        let pool = &self.gpu_descriptor_heap_pools[heap.heap_type as usize];

        heap.current_descriptor_index = 0;

        lock(&pool.heaps).push(heap);
    }
}
