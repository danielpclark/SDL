// Rust translation of the resource parts of src/gpu/d3d12/SDL_gpu_d3d12.c
// from Simple DirectMedia Layer: buffers, transfer and uniform buffers,
// textures, samplers and shaders, their release and destruction, cycling
// and the debug names.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! Buffers and textures. The front end's handles are containers
//! ([`BufferContainer`], [`TextureContainer`]) whose active buffer or
//! texture can be cycled to a fresh one while the GPU still uses the old;
//! the [`D3D12Buffer`]s and [`D3D12Texture`]s are what command buffers
//! track.
//!
//! Upstream's objects are reference counted by hand (`referenceCount`,
//! incremented when a command buffer tracks the object) and destroyed in
//! `D3D12_INTERNAL_PerformPendingDestroys()` once released and unused;
//! that is kept. The objects are also `Arc`s, and their COM references and
//! staging descriptors go when the last `Arc` does (upstream's
//! `D3D12_INTERNAL_Destroy*()`): that is the dispose list's, as the
//! container (which refers to its active buffer or texture too) goes when
//! the front end drops its handle, right after releasing it. A buffer's or
//! texture's container is a `Weak` back reference with its index, as
//! upstream's `container` and `containerIndex`.

use std::ptr::NonNull;
use std::sync::atomic::{AtomicBool, AtomicI32, AtomicPtr, Ordering};
use std::sync::{Arc, Mutex, Weak};

use super::d3d::*;
use super::descriptors::StagingDescriptor;
use super::pipelines::{D3D12ComputePipeline, D3D12GraphicsPipeline};
use super::tables::{
    compare_op, sample_count, sampler_address_mode, sdl_to_d3d12_depth_format, sdl_to_d3d12_filter,
    sdl_to_d3d12_texture_format, sdl_to_d3d12_typeless_format,
};
use super::{lock, D3D12Renderer};
use crate::core::windows::utf8_to_wide;
use crate::error::Result;
use crate::gpu::sysgpu::UNIFORM_BUFFER_SIZE;
use crate::gpu::{
    BufferUsageFlags, SampleCount, SamplerCreateInfo, ShaderCreateInfo, ShaderFormat, ShaderStage,
    TextureCreateInfo, TextureType, TextureUsageFlags,
};
use crate::log::Category;
use crate::properties::Properties;

// Enums

/// Translation of `D3D12BufferType`.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum D3D12BufferType {
    Gpu,
    Uniform,
    Upload,
    Download,
}

// Structures

/// A buffer's or texture's place in its container (`container`,
/// `containerIndex`).
#[derive(Debug)]
pub(super) struct ContainerRef<C> {
    #[allow(dead_code)] // (part 2: barriers read the container's usage)
    pub(super) container: Weak<C>,
    #[allow(dead_code)] // (kept as upstream does)
    pub(super) index: usize,
}

/// Translation of `D3D12Buffer`.
#[derive(Debug)]
pub(super) struct D3D12Buffer {
    pub(super) container: Mutex<Option<ContainerRef<BufferContainer>>>,

    pub(super) handle: D3d12Resource,
    #[allow(dead_code)] // (part 2: binding)
    pub(super) uav_descriptor: Option<StagingDescriptor>,
    #[allow(dead_code)] // (part 2: binding)
    pub(super) srv_descriptor: Option<StagingDescriptor>,
    #[allow(dead_code)] // (part 2: binding)
    pub(super) virtual_address: u64,
    /// NULL except for upload buffers and fast uniform buffers
    pub(super) map_pointer: AtomicPtr<u8>,
    pub(super) reference_count: AtomicI32,
    /// used for initial resource barrier
    #[allow(dead_code)] // (part 2: barriers)
    pub(super) transitioned: AtomicBool,
}

impl Drop for D3D12Buffer {
    /// Translation of `D3D12_INTERNAL_DestroyBuffer()`: the descriptors
    /// and the resource are released after the unmapping, as fields.
    fn drop(&mut self) {
        if !self.map_pointer.get_mut().is_null() {
            self.handle.unmap();
        }
    }
}

/// The front end's buffer and transfer buffer handles. Translation of
/// `D3D12BufferContainer`.
#[derive(Debug)]
pub(super) struct BufferContainer {
    pub(super) usage: BufferUsageFlags,
    pub(super) size: u32,
    pub(super) ty: D3D12BufferType,
    pub(super) state: Mutex<BufferContainerState>,
}

/// The cycled buffers of a [`BufferContainer`].
#[derive(Debug)]
pub(super) struct BufferContainerState {
    pub(super) active_buffer: Arc<D3D12Buffer>,
    pub(super) buffers: Vec<Arc<D3D12Buffer>>,
    pub(super) debug_name: Option<String>,
}

/// Translation of `D3D12UniformBuffer` (moved between the renderer's pool
/// and the command buffers using it).
#[derive(Debug)]
pub(super) struct UniformBuffer {
    pub(super) buffer: Arc<D3D12Buffer>,
    pub(super) write_offset: u32,
    pub(super) draw_offset: u32,
}

/// Null views represent by heap = NULL. Translation of
/// `D3D12TextureSubresource` (its `parent` is the texture that holds it).
#[derive(Debug, Default)]
pub(super) struct TextureSubresource {
    pub(super) layer: u32,
    pub(super) level: u32,
    pub(super) depth: u32,
    pub(super) index: u32,

    /// One per depth slice (empty if not a color target)
    pub(super) rtv_handles: Vec<StagingDescriptor>,

    /// `None` if not a compute storage write texture
    pub(super) uav_handle: Option<StagingDescriptor>,
    /// `None` if not a depth stencil target
    pub(super) dsv_handle: Option<StagingDescriptor>,
}

/// Translation of `D3D12Texture`; its destruction
/// (`D3D12_INTERNAL_DestroyTexture()`) releases the views and the resource
/// as fields.
#[derive(Debug)]
pub(super) struct D3D12Texture {
    pub(super) container: Mutex<Option<ContainerRef<TextureContainer>>>,

    /// layerCount * num_levels
    #[allow(dead_code)] // (part 2: passes)
    pub(super) subresources: Vec<TextureSubresource>,

    pub(super) resource: D3d12Resource,
    #[allow(dead_code)] // (part 2: binding)
    pub(super) srv_handle: Option<StagingDescriptor>,

    pub(super) reference_count: AtomicI32,
}

/// The front end's texture handles. Translation of
/// `D3D12TextureContainer`.
///
/// Not translated: `externallyManaged`, for OpenXR's swapchain images.
#[derive(Debug)]
pub(super) struct TextureContainer {
    /// The creation info (`header.info`), with its own copy of the
    /// properties.
    pub(super) info: TextureCreateInfo,
    pub(super) state: Mutex<TextureContainerState>,
    /// Swapchain images cannot be cycled
    #[allow(dead_code)] // (part 2: cycling on writes)
    pub(super) can_be_cycled: bool,
}

/// The cycled textures of a [`TextureContainer`].
#[derive(Debug)]
pub(super) struct TextureContainerState {
    pub(super) active_texture: Arc<D3D12Texture>,
    pub(super) textures: Vec<Arc<D3D12Texture>>,
    pub(super) debug_name: Option<String>,
}

/// Translation of `D3D12Sampler`; its destruction
/// (`D3D12_INTERNAL_DestroySampler()`) gives the descriptor back.
#[derive(Debug)]
pub(super) struct D3D12Sampler {
    #[allow(dead_code)] // (kept as upstream does)
    pub(super) create_info: SamplerCreateInfo,
    #[allow(dead_code)] // (part 2: binding)
    pub(super) handle: StagingDescriptor,
    pub(super) reference_count: AtomicI32,
}

/// Translation of `D3D12Shader`.
#[derive(Debug)]
pub(super) struct D3D12Shader {
    pub(super) bytecode: Vec<u8>,

    pub(super) stage: ShaderStage,
    pub(super) num_samplers: u32,
    pub(super) num_uniform_buffers: u32,
    pub(super) num_storage_buffers: u32,
    pub(super) num_storage_textures: u32,
}

/// What `PerformPendingDestroys()` destroys once unused (the
/// `*ToDestroy` arrays, with `disposeLock`).
#[derive(Debug, Default)]
pub(super) struct PendingDestroys {
    pub(super) buffers_to_destroy: Vec<Arc<D3D12Buffer>>,
    pub(super) textures_to_destroy: Vec<Arc<D3D12Texture>>,
    pub(super) samplers_to_destroy: Vec<Arc<D3D12Sampler>>,
    pub(super) graphics_pipelines_to_destroy: Vec<Arc<D3D12GraphicsPipeline>>,
    pub(super) compute_pipelines_to_destroy: Vec<Arc<D3D12ComputePipeline>>,
}

// SAFETY: Direct3D 12 resources may be used from any thread (they are
// free-threaded); SDL serializes what isn't, as upstream does.
unsafe impl Send for D3D12Buffer {}
// SAFETY: as for Send.
unsafe impl Sync for D3D12Buffer {}
// SAFETY: as for D3D12Buffer.
unsafe impl Send for D3D12Texture {}
// SAFETY: as for D3D12Buffer.
unsafe impl Sync for D3D12Texture {}
// SAFETY: a descriptor is a CPU handle of a heap its pool keeps alive;
// its slot is written under the renderer's rules, as upstream's.
unsafe impl Send for StagingDescriptor {}
// SAFETY: as for Send.
unsafe impl Sync for StagingDescriptor {}

// Helpers

/// Translation of `D3D12_INTERNAL_Align()`.
#[cfg_attr(not(test), allow(dead_code))] // (part 2: uniform data)
pub(super) fn align(location: u32, alignment: u32) -> u32 {
    (location.wrapping_add(alignment - 1)) & !(alignment - 1)
}

/// Translation of `D3D12_INTERNAL_CalcSubresource()`.
pub(super) fn calc_subresource(mip_level: u32, layer: u32, num_levels: u32) -> u32 {
    mip_level + (layer * num_levels)
}

/// Translation of `D3D12_INTERNAL_CalcSubresourceWithPlane()`.
#[cfg_attr(not(test), allow(dead_code))] // (part 2: barriers)
pub(super) fn calc_subresource_with_plane(
    mip_level: u32,
    layer: u32,
    plane_slice: u32,
    num_levels: u32,
    array_size: u32,
) -> u32 {
    mip_level + (layer * num_levels) + (plane_slice * num_levels * array_size)
}

/// The state a texture is in between passes. Translation of
/// `D3D12_INTERNAL_DefaultTextureResourceState()`.
#[allow(clippy::if_same_then_else)] // (a branch per usage, as upstream has)
pub(super) fn default_texture_resource_state(usage_flags: TextureUsageFlags) -> u32 {
    // NOTE: order matters here!

    if usage_flags.contains(TextureUsageFlags::SAMPLER) {
        D3D12_RESOURCE_STATE_ALL_SHADER_RESOURCE
    } else if usage_flags.contains(TextureUsageFlags::GRAPHICS_STORAGE_READ) {
        D3D12_RESOURCE_STATE_ALL_SHADER_RESOURCE
    } else if usage_flags.contains(TextureUsageFlags::COLOR_TARGET) {
        D3D12_RESOURCE_STATE_RENDER_TARGET
    } else if usage_flags.contains(TextureUsageFlags::DEPTH_STENCIL_TARGET) {
        D3D12_RESOURCE_STATE_DEPTH_WRITE
    } else if usage_flags.contains(TextureUsageFlags::COMPUTE_STORAGE_READ) {
        D3D12_RESOURCE_STATE_NON_PIXEL_SHADER_RESOURCE
    } else if usage_flags.contains(TextureUsageFlags::COMPUTE_STORAGE_WRITE) {
        D3D12_RESOURCE_STATE_UNORDERED_ACCESS
    } else if usage_flags.contains(TextureUsageFlags::COMPUTE_STORAGE_SIMULTANEOUS_READ_WRITE) {
        D3D12_RESOURCE_STATE_UNORDERED_ACCESS
    } else {
        crate::log::error!(Category::Gpu, "Texture has no default usage mode!");
        D3D12_RESOURCE_STATE_ALL_SHADER_RESOURCE
    }
}

/// The state a buffer is in between passes. Translation of
/// `D3D12_INTERNAL_DefaultBufferResourceState()` (of its container's
/// usage).
#[cfg_attr(not(test), allow(dead_code))] // (part 2: barriers)
pub(super) fn default_buffer_resource_state(usage: BufferUsageFlags) -> u32 {
    let mut states = D3D12_RESOURCE_STATE_COMMON;

    if usage.contains(BufferUsageFlags::VERTEX) {
        states |= D3D12_RESOURCE_STATE_VERTEX_AND_CONSTANT_BUFFER;
    }
    if usage.contains(BufferUsageFlags::INDEX) {
        states |= D3D12_RESOURCE_STATE_INDEX_BUFFER;
    }
    if usage.contains(BufferUsageFlags::INDIRECT) {
        states |= D3D12_RESOURCE_STATE_INDIRECT_ARGUMENT;
    }
    if usage.contains(BufferUsageFlags::GRAPHICS_STORAGE_READ) {
        states |= D3D12_RESOURCE_STATE_ALL_SHADER_RESOURCE;
    }
    if usage.contains(BufferUsageFlags::COMPUTE_STORAGE_READ) {
        states |= D3D12_RESOURCE_STATE_NON_PIXEL_SHADER_RESOURCE;
    }

    // If no read flags are set, read-write can be the default.
    if states == 0 && usage.contains(BufferUsageFlags::COMPUTE_STORAGE_WRITE) {
        return D3D12_RESOURCE_STATE_UNORDERED_ACCESS;
    }

    if states == 0 {
        crate::log::error!(Category::Gpu, "Buffer has no default usage mode!");
        return D3D12_RESOURCE_STATE_VERTEX_AND_CONSTANT_BUFFER;
    }

    states
}

/// A float creation property of a texture, 0 when unset.
fn float_property(props: Option<&Properties>, name: &str) -> f32 {
    props.and_then(|p| p.get_float(name)).unwrap_or(0.0)
}

impl D3D12Renderer {
    // Release / Cleanup

    /// Translation of `D3D12_INTERNAL_ReleaseBuffer()` (the caller holds
    /// `disposeLock`).
    fn release_buffer_internal(dispose: &mut PendingDestroys, buffer: &Arc<D3D12Buffer>) {
        dispose.buffers_to_destroy.push(buffer.clone());
    }

    /// Release every buffer of a container (whose handle the front end
    /// drops next). Translation of
    /// `D3D12_INTERNAL_ReleaseBufferContainer()`.
    pub(super) fn release_buffer_container(&self, container: &BufferContainer) {
        let mut dispose = lock(&self.dispose);

        let buffers = std::mem::take(&mut lock(&container.state).buffers);
        for buffer in &buffers {
            Self::release_buffer_internal(&mut dispose, buffer);
        }

        // Containers are just client handles, so we can free immediately
    }

    /// Translation of `D3D12_INTERNAL_ReleaseTexture()` (the caller holds
    /// `disposeLock`).
    fn release_texture_internal(dispose: &mut PendingDestroys, texture: &Arc<D3D12Texture>) {
        dispose.textures_to_destroy.push(texture.clone());
    }

    /// Release every texture of a container (whose handle the front end
    /// drops next). Translation of
    /// `D3D12_INTERNAL_ReleaseTextureContainer()`.
    pub(super) fn release_texture_container(&self, container: &TextureContainer) {
        let mut dispose = lock(&self.dispose);

        let textures = std::mem::take(&mut lock(&container.state).textures);
        for texture in &textures {
            Self::release_texture_internal(&mut dispose, texture);
        }

        // Containers are just client handles, so we can destroy immediately
        // (the copy of the properties goes with the container).
    }

    /// Destroy what was released and is no longer used. Translation of
    /// `D3D12_INTERNAL_PerformPendingDestroys()`.
    pub(super) fn perform_pending_destroys(&self) {
        let mut guard = lock(&self.dispose);
        let dispose = &mut *guard;

        fn unused(reference_count: &AtomicI32) -> bool {
            reference_count.load(Ordering::SeqCst) == 0
        }

        // (Dropping the last reference is D3D12_INTERNAL_Destroy*().)
        for i in (0..dispose.buffers_to_destroy.len()).rev() {
            if unused(&dispose.buffers_to_destroy[i].reference_count) {
                drop(dispose.buffers_to_destroy.swap_remove(i));
            }
        }

        for i in (0..dispose.textures_to_destroy.len()).rev() {
            if unused(&dispose.textures_to_destroy[i].reference_count) {
                drop(dispose.textures_to_destroy.swap_remove(i));
            }
        }

        for i in (0..dispose.samplers_to_destroy.len()).rev() {
            if unused(&dispose.samplers_to_destroy[i].reference_count) {
                drop(dispose.samplers_to_destroy.swap_remove(i));
            }
        }

        for i in (0..dispose.graphics_pipelines_to_destroy.len()).rev() {
            if unused(&dispose.graphics_pipelines_to_destroy[i].reference_count) {
                drop(dispose.graphics_pipelines_to_destroy.swap_remove(i));
            }
        }

        for i in (0..dispose.compute_pipelines_to_destroy.len()).rev() {
            if unused(&dispose.compute_pipelines_to_destroy[i].reference_count) {
                drop(dispose.compute_pipelines_to_destroy.swap_remove(i));
            }
        }
    }

    // Debug Naming

    /// Translation of `D3D12_INTERNAL_SetPipelineStateName()`.
    pub(super) fn set_pipeline_state_name(&self, pipeline_state: &D3d12PipelineState, text: &str) {
        if self.debug_mode {
            pipeline_state.set_name(&utf8_to_wide(text));
        }
    }

    /// Translation of `D3D12_INTERNAL_SetResourceName()`.
    pub(super) fn set_resource_name(&self, resource: &D3d12Resource, text: Option<&str>) {
        if let (true, Some(text)) = (self.debug_mode, text) {
            resource.set_name(&utf8_to_wide(text));
        }
    }

    /// Translation of `D3D12_SetBufferName()`.
    pub(super) fn set_buffer_name_internal(&self, container: &BufferContainer, text: &str) {
        if self.debug_mode {
            let buffers = {
                let mut state = lock(&container.state);
                state.debug_name = Some(text.to_owned());
                state.buffers.clone()
            };

            for buffer in &buffers {
                self.set_resource_name(&buffer.handle, Some(text));
            }
        }
    }

    /// Translation of `D3D12_SetTextureName()`.
    pub(super) fn set_texture_name_internal(&self, container: &TextureContainer, text: &str) {
        if self.debug_mode {
            let textures = {
                let mut state = lock(&container.state);
                state.debug_name = Some(text.to_owned());
                state.textures.clone()
            };

            for texture in &textures {
                self.set_resource_name(&texture.resource, Some(text));
            }
        }
    }

    // State Creation

    /// Translation of `D3D12_CreateSampler()`.
    ///
    /// Note (upstream): C ignores a failure to get a descriptor and writes
    /// the sampler to a NULL handle; the error is returned here.
    pub(super) fn create_sampler_internal(
        &self,
        createinfo: &SamplerCreateInfo,
    ) -> Result<Arc<D3D12Sampler>> {
        let sampler_desc = SamplerDesc {
            filter: sdl_to_d3d12_filter(
                createinfo.min_filter,
                createinfo.mag_filter,
                createinfo.mipmap_mode,
                createinfo.enable_compare,
                createinfo.enable_anisotropy,
            ),
            address_u: sampler_address_mode(createinfo.address_mode_u),
            address_v: sampler_address_mode(createinfo.address_mode_v),
            address_w: sampler_address_mode(createinfo.address_mode_w),
            max_anisotropy: createinfo.max_anisotropy as u32,
            comparison_func: compare_op(createinfo.compare_op),
            min_lod: createinfo.min_lod,
            max_lod: createinfo.max_lod,
            mip_lod_bias: createinfo.mip_lod_bias,
            border_color: [0.0; 4],
        };

        let handle = self.assign_staging_descriptor_handle(D3D12_DESCRIPTOR_HEAP_TYPE_SAMPLER)?;

        self.device.create_sampler(&sampler_desc, handle.cpu_handle);

        // Ignore name property because it is not applicable to D3D12.

        Ok(Arc::new(D3D12Sampler {
            create_info: createinfo.clone(),
            handle,
            reference_count: AtomicI32::new(0),
        }))
    }

    /// Translation of `D3D12_INTERNAL_CreateShaderBytecode()`, which
    /// validates the code (the shader keeps a copy of it).
    fn check_shader_bytecode(&self, code: &[u8], format: ShaderFormat) -> Result<()> {
        if !is_valid_shader_bytecode(code) {
            if format == ShaderFormat::DXBC {
                return Err(self.set_string_error("The provided shader code is not valid DXBC!"));
            }
            return Err(self.set_string_error("The provided shader code is not valid DXIL!"));
        }
        Ok(())
    }

    /// Translation of `D3D12_CreateShader()`.
    pub(super) fn create_shader_internal(
        &self,
        createinfo: &ShaderCreateInfo<'_>,
    ) -> Result<D3D12Shader> {
        self.check_shader_bytecode(createinfo.code, createinfo.format)?;

        // Ignore name property because it is not applicable to D3D12.

        Ok(D3D12Shader {
            bytecode: createinfo.code.to_vec(),
            stage: createinfo.stage,
            num_samplers: createinfo.num_samplers,
            num_storage_buffers: createinfo.num_storage_buffers,
            num_storage_textures: createinfo.num_storage_textures,
            num_uniform_buffers: createinfo.num_uniform_buffers,
        })
    }

    /// Validate the code of a compute pipeline (`CreateShaderBytecode()`
    /// without the copy).
    pub(super) fn check_compute_bytecode(&self, code: &[u8], format: ShaderFormat) -> Result<()> {
        self.check_shader_bytecode(code, format)
    }

    /// Translation of `D3D12_INTERNAL_CreateTexture()`.
    ///
    /// Note (upstream): C ignores a failure to get a descriptor and writes
    /// the view to a NULL handle; the error is returned here.
    pub(super) fn create_texture_internal(
        &self,
        createinfo: &TextureCreateInfo,
        is_swapchain_texture: bool,
        debug_name: Option<&str>,
    ) -> Result<Arc<D3D12Texture>> {
        let usage = createinfo.usage;
        let mut heap_flags = D3D12_HEAP_FLAG_NONE;
        let mut resource_flags = 0;
        let mut clear_value = ClearValue::default();
        let mut use_clear_value = false;
        let needs_srv = usage.intersects(
            TextureUsageFlags::SAMPLER
                | TextureUsageFlags::GRAPHICS_STORAGE_READ
                | TextureUsageFlags::COMPUTE_STORAGE_READ,
        );
        let needs_uav = usage.intersects(
            TextureUsageFlags::COMPUTE_STORAGE_WRITE
                | TextureUsageFlags::COMPUTE_STORAGE_SIMULTANEOUS_READ_WRITE,
        );
        let props = createinfo.props.as_ref();

        let is_3d = createinfo.texture_type == TextureType::Texture3D;
        let layer_count = if is_3d {
            1
        } else {
            createinfo.layer_count_or_depth
        };
        let depth = if is_3d {
            createinfo.layer_count_or_depth
        } else {
            1
        };
        let is_multisample = createinfo.sample_count > SampleCount::One;

        let mut format = sdl_to_d3d12_texture_format(createinfo.format);

        if usage.contains(TextureUsageFlags::COLOR_TARGET) {
            resource_flags |= D3D12_RESOURCE_FLAG_ALLOW_RENDER_TARGET;
            use_clear_value = true;
            clear_value.format = format;
            clear_value.u.color = [
                float_property(
                    props,
                    crate::gpu::PROP_GPU_TEXTURE_CREATE_D3D12_CLEAR_R_FLOAT,
                ),
                float_property(
                    props,
                    crate::gpu::PROP_GPU_TEXTURE_CREATE_D3D12_CLEAR_G_FLOAT,
                ),
                float_property(
                    props,
                    crate::gpu::PROP_GPU_TEXTURE_CREATE_D3D12_CLEAR_B_FLOAT,
                ),
                float_property(
                    props,
                    crate::gpu::PROP_GPU_TEXTURE_CREATE_D3D12_CLEAR_A_FLOAT,
                ),
            ];
        }

        if usage.contains(TextureUsageFlags::DEPTH_STENCIL_TARGET) {
            resource_flags |= D3D12_RESOURCE_FLAG_ALLOW_DEPTH_STENCIL;
            use_clear_value = true;
            clear_value.format = sdl_to_d3d12_depth_format(createinfo.format);
            clear_value.u.depth_stencil = DepthStencilValue {
                depth: float_property(
                    props,
                    crate::gpu::PROP_GPU_TEXTURE_CREATE_D3D12_CLEAR_DEPTH_FLOAT,
                ),
                stencil: props
                    .and_then(|p| {
                        p.get_number(crate::gpu::PROP_GPU_TEXTURE_CREATE_D3D12_CLEAR_STENCIL_NUMBER)
                    })
                    .unwrap_or(0) as u8,
            };
            format = if needs_srv {
                sdl_to_d3d12_typeless_format(createinfo.format)
            } else {
                sdl_to_d3d12_depth_format(createinfo.format)
            };
        }

        if needs_uav {
            resource_flags |= D3D12_RESOURCE_FLAG_ALLOW_UNORDERED_ACCESS;
        }

        let heap_properties = HeapProperties {
            ty: D3D12_HEAP_TYPE_DEFAULT,
            cpu_page_property: D3D12_CPU_PAGE_PROPERTY_UNKNOWN,
            memory_pool_preference: D3D12_MEMORY_POOL_UNKNOWN,
            creation_node_mask: 0, // We don't do multi-adapter operation
            visible_node_mask: 0,  // We don't do multi-adapter operation
        };

        if is_swapchain_texture {
            heap_flags = D3D12_HEAP_FLAG_ALLOW_DISPLAY;
        }

        let desc = if !is_3d {
            ResourceDesc {
                dimension: D3D12_RESOURCE_DIMENSION_TEXTURE2D,
                alignment: if is_swapchain_texture {
                    0
                } else if is_multisample {
                    D3D12_DEFAULT_MSAA_RESOURCE_PLACEMENT_ALIGNMENT
                } else {
                    D3D12_DEFAULT_RESOURCE_PLACEMENT_ALIGNMENT
                },
                width: createinfo.width as u64,
                height: createinfo.height,
                depth_or_array_size: createinfo.layer_count_or_depth as u16,
                mip_levels: createinfo.num_levels as u16,
                format,
                sample_desc: crate::render::direct3d11::d3d::SampleDesc {
                    count: sample_count(createinfo.sample_count),
                    quality: if is_multisample {
                        D3D12_STANDARD_MULTISAMPLE_PATTERN
                    } else {
                        0
                    },
                },
                layout: D3D12_TEXTURE_LAYOUT_UNKNOWN, // Apparently this is the most efficient choice
                flags: resource_flags,
            }
        } else {
            ResourceDesc {
                dimension: D3D12_RESOURCE_DIMENSION_TEXTURE3D,
                alignment: D3D12_DEFAULT_RESOURCE_PLACEMENT_ALIGNMENT,
                width: createinfo.width as u64,
                height: createinfo.height,
                depth_or_array_size: createinfo.layer_count_or_depth as u16,
                mip_levels: createinfo.num_levels as u16,
                format,
                sample_desc: crate::render::direct3d11::d3d::SampleDesc {
                    count: 1,
                    quality: 0,
                },
                layout: D3D12_TEXTURE_LAYOUT_UNKNOWN,
                flags: resource_flags,
            }
        };

        let initial_state = if is_swapchain_texture {
            D3D12_RESOURCE_STATE_PRESENT
        } else {
            default_texture_resource_state(usage)
        };

        let handle = self
            .device
            .create_committed_resource(
                &heap_properties,
                heap_flags,
                &desc,
                initial_state,
                use_clear_value.then_some(&clear_value),
            )
            .map_err(|res| self.set_error("Failed to create texture!", res))?;

        // Create the SRV if applicable
        let mut srv_handle = None;
        if needs_srv {
            let srv =
                self.assign_staging_descriptor_handle(D3D12_DESCRIPTOR_HEAP_TYPE_CBV_SRV_UAV)?;

            let mut srv_desc = ShaderResourceViewDesc {
                format: sdl_to_d3d12_texture_format(createinfo.format),
                shader_4_component_mapping: D3D12_DEFAULT_SHADER_4_COMPONENT_MAPPING,
                ..Default::default()
            };

            let num_levels = createinfo.num_levels;
            match createinfo.texture_type {
                TextureType::Cube => {
                    srv_desc.view_dimension = D3D12_SRV_DIMENSION_TEXTURECUBE;
                    srv_desc.u.texture_cube = Tex3dSrv {
                        mip_levels: num_levels,
                        most_detailed_mip: 0,
                        resource_min_lod_clamp: 0.0,
                    };
                }
                TextureType::CubeArray => {
                    srv_desc.view_dimension = D3D12_SRV_DIMENSION_TEXTURECUBEARRAY;
                    srv_desc.u.texture_cube_array = TexCubeArraySrv {
                        mip_levels: num_levels,
                        most_detailed_mip: 0,
                        first_2d_array_face: 0,
                        num_cubes: createinfo.layer_count_or_depth / 6,
                        resource_min_lod_clamp: 0.0,
                    };
                }
                TextureType::Texture2DArray => {
                    srv_desc.view_dimension = D3D12_SRV_DIMENSION_TEXTURE2DARRAY;
                    srv_desc.u.texture_2d_array = Tex2dArraySrv {
                        mip_levels: num_levels,
                        most_detailed_mip: 0,
                        first_array_slice: 0,
                        array_size: layer_count,
                        resource_min_lod_clamp: 0.0,
                        plane_slice: 0,
                    };
                }
                TextureType::Texture3D => {
                    srv_desc.view_dimension = D3D12_SRV_DIMENSION_TEXTURE3D;
                    srv_desc.u.texture_3d = Tex3dSrv {
                        mip_levels: num_levels,
                        most_detailed_mip: 0,
                        resource_min_lod_clamp: 0.0, // default behavior
                    };
                }
                TextureType::Texture2D if is_multisample => {
                    srv_desc.view_dimension = D3D12_SRV_DIMENSION_TEXTURE2DMS;
                    srv_desc.u.texture_2dms = Tex2dmsSrv {
                        unused_field_nothing_to_define: 0,
                    };
                }
                TextureType::Texture2D => {
                    srv_desc.view_dimension = D3D12_SRV_DIMENSION_TEXTURE2D;
                    srv_desc.u.texture_2d = Tex2dSrv {
                        mip_levels: num_levels,
                        most_detailed_mip: 0,
                        plane_slice: 0,
                        resource_min_lod_clamp: 0.0, // default behavior
                    };
                }
            }

            self.device
                .create_shader_resource_view(&handle, &srv_desc, srv.cpu_handle);
            srv_handle = Some(srv);
        }

        let is_array_like = matches!(
            createinfo.texture_type,
            TextureType::Texture2DArray | TextureType::Cube | TextureType::CubeArray
        );
        let view_format = sdl_to_d3d12_texture_format(createinfo.format);

        let subresource_count = (createinfo.num_levels * layer_count) as usize;
        let mut subresources: Vec<TextureSubresource> = Vec::with_capacity(subresource_count);
        subresources.resize_with(subresource_count, Default::default);
        for layer_index in 0..layer_count {
            for level_index in 0..createinfo.num_levels {
                let subresource_index =
                    calc_subresource(level_index, layer_index, createinfo.num_levels);
                let subresource = &mut subresources[subresource_index as usize];

                subresource.layer = layer_index;
                subresource.level = level_index;
                subresource.depth = depth;
                subresource.index = subresource_index;

                // Create RTV if needed
                if usage.contains(TextureUsageFlags::COLOR_TARGET) {
                    subresource.rtv_handles.reserve_exact(depth as usize);

                    for depth_index in 0..depth {
                        let rtv =
                            self.assign_staging_descriptor_handle(D3D12_DESCRIPTOR_HEAP_TYPE_RTV)?;

                        let mut rtv_desc = RenderTargetViewDesc {
                            format: view_format,
                            ..Default::default()
                        };

                        if is_array_like {
                            rtv_desc.view_dimension = D3D12_RTV_DIMENSION_TEXTURE2DARRAY;
                            rtv_desc.u.texture_2d_array = Tex2dArrayRtv {
                                mip_slice: level_index,
                                first_array_slice: layer_index,
                                array_size: 1,
                                plane_slice: 0,
                            };
                        } else if is_3d {
                            rtv_desc.view_dimension = D3D12_RTV_DIMENSION_TEXTURE3D;
                            rtv_desc.u.texture_3d = Tex3dRtv {
                                mip_slice: level_index,
                                first_w_slice: depth_index,
                                w_size: 1,
                            };
                        } else if is_multisample {
                            rtv_desc.view_dimension = D3D12_RTV_DIMENSION_TEXTURE2DMS;
                        } else {
                            rtv_desc.view_dimension = D3D12_RTV_DIMENSION_TEXTURE2D;
                            rtv_desc.u.texture_2d = Tex2dRtv {
                                mip_slice: level_index,
                                plane_slice: 0,
                            };
                        }

                        self.device
                            .create_render_target_view(&handle, &rtv_desc, rtv.cpu_handle);
                        subresource.rtv_handles.push(rtv);
                    }
                }

                // Create DSV if needed
                if usage.contains(TextureUsageFlags::DEPTH_STENCIL_TARGET) {
                    let dsv =
                        self.assign_staging_descriptor_handle(D3D12_DESCRIPTOR_HEAP_TYPE_DSV)?;

                    let mut dsv_desc = DepthStencilViewDesc {
                        format: sdl_to_d3d12_depth_format(createinfo.format),
                        flags: 0,
                        ..Default::default()
                    };

                    if is_array_like {
                        dsv_desc.view_dimension = D3D12_DSV_DIMENSION_TEXTURE2DARRAY;
                        dsv_desc.u.texture_2d_array = Tex2dArrayDsv {
                            mip_slice: level_index,
                            first_array_slice: layer_index,
                            array_size: 1,
                        };
                    } else if is_multisample {
                        dsv_desc.view_dimension = D3D12_DSV_DIMENSION_TEXTURE2DMS;
                    } else {
                        dsv_desc.view_dimension = D3D12_DSV_DIMENSION_TEXTURE2D;
                        dsv_desc.u.texture_2d = Tex2dDsv {
                            mip_slice: level_index,
                        };
                    }

                    self.device
                        .create_depth_stencil_view(&handle, &dsv_desc, dsv.cpu_handle);
                    subresource.dsv_handle = Some(dsv);
                }

                // Create subresource UAV if necessary
                if needs_uav {
                    let uav = self
                        .assign_staging_descriptor_handle(D3D12_DESCRIPTOR_HEAP_TYPE_CBV_SRV_UAV)?;

                    let mut uav_desc = UnorderedAccessViewDesc {
                        format: view_format,
                        ..Default::default()
                    };

                    if is_array_like {
                        uav_desc.view_dimension = D3D12_UAV_DIMENSION_TEXTURE2DARRAY;
                        uav_desc.u.texture_2d_array = Tex2dArrayRtv {
                            mip_slice: level_index,
                            first_array_slice: layer_index,
                            array_size: 1,
                            plane_slice: 0,
                        };
                    } else if is_3d {
                        uav_desc.view_dimension = D3D12_UAV_DIMENSION_TEXTURE3D;
                        uav_desc.u.texture_3d = Tex3dRtv {
                            mip_slice: level_index,
                            first_w_slice: 0,
                            w_size: depth,
                        };
                    } else {
                        uav_desc.view_dimension = D3D12_UAV_DIMENSION_TEXTURE2D;
                        uav_desc.u.texture_2d = Tex2dRtv {
                            mip_slice: level_index,
                            plane_slice: 0,
                        };
                    }

                    self.device
                        .create_unordered_access_view(&handle, &uav_desc, uav.cpu_handle);
                    subresource.uav_handle = Some(uav);
                }
            }
        }

        self.set_resource_name(&handle, debug_name);

        Ok(Arc::new(D3D12Texture {
            container: Mutex::new(None),
            subresources,
            resource: handle,
            srv_handle,
            reference_count: AtomicI32::new(0),
        }))
    }

    /// Translation of `D3D12_CreateTexture()`.
    pub(super) fn create_texture_container(
        &self,
        createinfo: &TextureCreateInfo,
    ) -> Result<Arc<TextureContainer>> {
        // Copy properties so we don't lose information when the client destroys them
        let props = Properties::new();
        if let Some(src) = &createinfo.props {
            props.copy_from(src)?;
        }
        let info = TextureCreateInfo {
            props: Some(props),
            ..createinfo.clone()
        };

        let debug_name = createinfo
            .props
            .as_ref()
            .and_then(|p| p.get_string(crate::gpu::PROP_GPU_TEXTURE_CREATE_NAME_STRING));

        let texture = self.create_texture_internal(createinfo, false, debug_name.as_deref())?;

        let container = Arc::new(TextureContainer {
            info,
            state: Mutex::new(TextureContainerState {
                active_texture: texture.clone(),
                textures: vec![texture.clone()],
                debug_name,
            }),
            can_be_cycled: true,
        });

        *lock(&texture.container) = Some(ContainerRef {
            container: Arc::downgrade(&container),
            index: 0,
        });

        Ok(container)
    }

    /// Make the container's active texture one the GPU doesn't use: a
    /// previously-cycled one, or a new one. Translation of
    /// `D3D12_INTERNAL_CycleActiveTexture()`.
    #[cfg_attr(not(test), allow(dead_code))] // (part 2: writes with cycling)
    pub(super) fn cycle_active_texture(&self, container: &Arc<TextureContainer>) {
        let debug_name = {
            let mut state = lock(&container.state);

            // If a previously-cycled texture is available, we can use that.
            if let Some(texture) = state
                .textures
                .iter()
                .find(|t| t.reference_count.load(Ordering::SeqCst) == 0)
                .cloned()
            {
                state.active_texture = texture;
                return;
            }

            state.debug_name.clone()
        };

        // No texture is available, generate a new one.
        let Ok(texture) =
            self.create_texture_internal(&container.info, false, debug_name.as_deref())
        else {
            return;
        };

        let mut state = lock(&container.state);
        *lock(&texture.container) = Some(ContainerRef {
            container: Arc::downgrade(container),
            index: state.textures.len(),
        });
        state.textures.push(texture.clone());

        state.active_texture = texture;
    }

    /// Translation of `D3D12_INTERNAL_CreateBuffer()`.
    ///
    /// Note (upstream): C ignores a failure to get a descriptor and writes
    /// the view to a NULL handle; the error is returned here.
    pub(super) fn create_buffer_internal(
        &self,
        usage_flags: BufferUsageFlags,
        size: u32,
        ty: D3D12BufferType,
        debug_name: Option<&str>,
    ) -> Result<Arc<D3D12Buffer>> {
        let mut resource_flags = 0;
        let mut initial_state = D3D12_RESOURCE_STATE_COMMON;

        if usage_flags.contains(BufferUsageFlags::COMPUTE_STORAGE_WRITE) {
            resource_flags |= D3D12_RESOURCE_FLAG_ALLOW_UNORDERED_ACCESS;
        }

        let mut heap_properties = HeapProperties {
            creation_node_mask: 0, // We don't do multi-adapter operation
            visible_node_mask: 0,  // We don't do multi-adapter operation
            cpu_page_property: D3D12_CPU_PAGE_PROPERTY_UNKNOWN,
            memory_pool_preference: D3D12_MEMORY_POOL_UNKNOWN,
            ty: 0,
        };
        let heap_flags = D3D12_HEAP_FLAG_NONE;

        match ty {
            D3D12BufferType::Gpu => {
                heap_properties.ty = D3D12_HEAP_TYPE_DEFAULT;
            }
            D3D12BufferType::Upload => {
                heap_properties.ty = D3D12_HEAP_TYPE_UPLOAD;
                initial_state = D3D12_RESOURCE_STATE_GENERIC_READ;
            }
            D3D12BufferType::Download => {
                heap_properties.ty = D3D12_HEAP_TYPE_READBACK;
                initial_state = D3D12_RESOURCE_STATE_COPY_DEST;
            }
            D3D12BufferType::Uniform => {
                // D3D12 is badly designed, so we have to check if the fast path for uniform buffers is enabled
                if self.gpu_upload_heap_supported {
                    heap_properties.ty = D3D12_HEAP_TYPE_GPU_UPLOAD;
                } else {
                    heap_properties.ty = D3D12_HEAP_TYPE_UPLOAD;
                    initial_state = D3D12_RESOURCE_STATE_GENERIC_READ;
                }
            }
        }
        // (The enum has no other values: no "Unrecognized buffer type!".)

        let desc = ResourceDesc {
            dimension: D3D12_RESOURCE_DIMENSION_BUFFER,
            alignment: D3D12_DEFAULT_RESOURCE_PLACEMENT_ALIGNMENT,
            width: size as u64,
            height: 1,
            depth_or_array_size: 1,
            mip_levels: 1,
            format: crate::render::direct3d11::d3d::DXGI_FORMAT_UNKNOWN,
            sample_desc: crate::render::direct3d11::d3d::SampleDesc {
                count: 1,
                quality: 0,
            },
            layout: D3D12_TEXTURE_LAYOUT_ROW_MAJOR,
            flags: resource_flags,
        };

        let handle = self
            .device
            .create_committed_resource(&heap_properties, heap_flags, &desc, initial_state, None)
            .map_err(|res| self.set_error("Could not create buffer!", res))?;

        let mut uav_descriptor = None;
        if usage_flags.contains(BufferUsageFlags::COMPUTE_STORAGE_WRITE) {
            let uav =
                self.assign_staging_descriptor_handle(D3D12_DESCRIPTOR_HEAP_TYPE_CBV_SRV_UAV)?;

            let uav_desc = UnorderedAccessViewDesc {
                view_dimension: D3D12_UAV_DIMENSION_BUFFER,
                format: crate::render::direct3d11::d3d::DXGI_FORMAT_R32_TYPELESS,
                u: UavUnion {
                    buffer: BufferUav {
                        first_element: 0,
                        num_elements: size / size_of::<u32>() as u32,
                        flags: D3D12_BUFFER_UAV_FLAG_RAW,
                        counter_offset_in_bytes: 0, // TODO: support counters?
                        structure_byte_stride: 0,
                    },
                },
            };

            // Create UAV
            self.device
                .create_unordered_access_view(&handle, &uav_desc, uav.cpu_handle);
            uav_descriptor = Some(uav);
        }

        let mut srv_descriptor = None;
        if usage_flags.intersects(
            BufferUsageFlags::GRAPHICS_STORAGE_READ | BufferUsageFlags::COMPUTE_STORAGE_READ,
        ) {
            let srv =
                self.assign_staging_descriptor_handle(D3D12_DESCRIPTOR_HEAP_TYPE_CBV_SRV_UAV)?;

            let srv_desc = ShaderResourceViewDesc {
                format: crate::render::direct3d11::d3d::DXGI_FORMAT_R32_TYPELESS,
                shader_4_component_mapping: D3D12_DEFAULT_SHADER_4_COMPONENT_MAPPING,
                view_dimension: D3D12_SRV_DIMENSION_BUFFER,
                u: SrvUnion {
                    buffer: BufferSrv {
                        first_element: 0,
                        num_elements: size / size_of::<u32>() as u32,
                        flags: D3D12_BUFFER_SRV_FLAG_RAW,
                        structure_byte_stride: 0,
                    },
                },
            };

            // Create SRV
            self.device
                .create_shader_resource_view(&handle, &srv_desc, srv.cpu_handle);
            srv_descriptor = Some(srv);
        }

        let virtual_address = if matches!(ty, D3D12BufferType::Gpu | D3D12BufferType::Uniform) {
            handle.gpu_virtual_address()
        } else {
            0
        };

        // Persistently map upload buffers
        let mut map_pointer = std::ptr::null_mut();
        if ty == D3D12BufferType::Upload {
            map_pointer = handle
                .map()
                .map_err(|res| self.set_error("Failed to map upload buffer!", res))?;
        }

        self.set_resource_name(&handle, debug_name);

        Ok(Arc::new(D3D12Buffer {
            container: Mutex::new(None),
            handle,
            uav_descriptor,
            srv_descriptor,
            virtual_address,
            map_pointer: AtomicPtr::new(map_pointer),
            reference_count: AtomicI32::new(0),
            transitioned: AtomicBool::new(initial_state != D3D12_RESOURCE_STATE_COMMON),
        }))
    }

    /// Translation of `D3D12_INTERNAL_CreateBufferContainer()`.
    pub(super) fn create_buffer_container(
        &self,
        usage_flags: BufferUsageFlags,
        size: u32,
        ty: D3D12BufferType,
        debug_name: Option<&str>,
    ) -> Result<Arc<BufferContainer>> {
        let buffer = self.create_buffer_internal(usage_flags, size, ty, debug_name)?;

        let container = Arc::new(BufferContainer {
            usage: usage_flags,
            size,
            ty,
            state: Mutex::new(BufferContainerState {
                active_buffer: buffer.clone(),
                buffers: vec![buffer.clone()],
                debug_name: debug_name.map(str::to_owned),
            }),
        });

        *lock(&buffer.container) = Some(ContainerRef {
            container: Arc::downgrade(&container),
            index: 0,
        });

        Ok(container)
    }

    /// Make the container's active buffer one the GPU doesn't use: a
    /// previously-cycled one, or a new one. Translation of
    /// `D3D12_INTERNAL_CycleActiveBuffer()`.
    pub(super) fn cycle_active_buffer(&self, container: &Arc<BufferContainer>) {
        let debug_name = {
            let mut state = lock(&container.state);

            // If a previously-cycled buffer is available, we can use that.
            if let Some(buffer) = state
                .buffers
                .iter()
                .find(|b| b.reference_count.load(Ordering::SeqCst) == 0)
                .cloned()
            {
                state.active_buffer = buffer;
                return;
            }

            state.debug_name.clone()
        };

        // No buffer handle is available, create a new one.
        let Ok(buffer) = self.create_buffer_internal(
            container.usage,
            container.size,
            container.ty,
            debug_name.as_deref(),
        ) else {
            return;
        };

        let mut state = lock(&container.state);
        *lock(&buffer.container) = Some(ContainerRef {
            container: Arc::downgrade(container),
            index: state.buffers.len(),
        });
        state.buffers.push(buffer.clone());

        state.active_buffer = buffer;

        if self.debug_mode {
            // (CreateBuffer already named it; upstream names it again.)
            self.set_resource_name(&state.active_buffer.handle, state.debug_name.as_deref());
        }
    }

    // TransferBuffer Data

    /// Translation of `D3D12_MapTransferBuffer()`.
    ///
    /// Note (upstream): C returns whatever `ID3D12Resource_Map()` leaves in
    /// its pointer when mapping a download buffer fails; the error is
    /// returned here.
    pub(super) fn map_transfer_buffer_internal(
        &self,
        container: &Arc<BufferContainer>,
        cycle: bool,
    ) -> Result<NonNull<u8>> {
        let active = lock(&container.state).active_buffer.clone();
        if cycle && active.reference_count.load(Ordering::SeqCst) > 0 {
            self.cycle_active_buffer(container);
        }

        let active = lock(&container.state).active_buffer.clone();
        // Upload buffers are persistently mapped, download buffers are not
        let data_pointer = if container.ty == D3D12BufferType::Upload {
            active.map_pointer.load(Ordering::SeqCst)
        } else {
            active
                .handle
                .map()
                .map_err(|res| self.set_error("Failed to map transfer buffer!", res))?
        };

        NonNull::new(data_pointer)
            .ok_or_else(|| self.set_string_error("Transfer buffer memory is not mapped!"))
    }

    /// Translation of `D3D12_UnmapTransferBuffer()`.
    pub(super) fn unmap_transfer_buffer_internal(&self, container: &BufferContainer) {
        // Upload buffers are persistently mapped, download buffers are not
        if container.ty == D3D12BufferType::Download {
            lock(&container.state).active_buffer.handle.unmap();
        }
    }

    // Uniform buffers

    /// A uniform buffer from the pool (or a new one), mapped. Translation of
    /// `D3D12_INTERNAL_AcquireUniformBufferFromPool()` but the tracking in
    /// the command buffer, which part 2 adds.
    ///
    /// Note (upstream): C leaks the uniform buffer when making its buffer
    /// or mapping it fails; it is dropped here.
    #[cfg_attr(not(test), allow(dead_code))] // (part 2: uniform data)
    pub(super) fn acquire_uniform_buffer_from_pool(&self) -> Result<UniformBuffer> {
        let pooled = lock(&self.uniform_buffer_pool).pop();
        let mut uniform_buffer = match pooled {
            Some(uniform_buffer) => uniform_buffer,
            None => UniformBuffer {
                buffer: self.create_buffer_internal(
                    BufferUsageFlags::default(),
                    UNIFORM_BUFFER_SIZE,
                    D3D12BufferType::Uniform,
                    None,
                )?,
                write_offset: 0,
                draw_offset: 0,
            },
        };

        uniform_buffer.draw_offset = 0;
        uniform_buffer.write_offset = 0;

        let map_pointer = uniform_buffer
            .buffer
            .handle
            .map()
            .map_err(|res| self.set_error("Failed to map buffer pool!", res))?;
        uniform_buffer
            .buffer
            .map_pointer
            .store(map_pointer, Ordering::SeqCst);

        Ok(uniform_buffer)
    }

    /// Translation of `D3D12_INTERNAL_ReturnUniformBufferToPool()`.
    #[cfg_attr(not(test), allow(dead_code))] // (part 2: command buffer cleanup)
    pub(super) fn return_uniform_buffer_to_pool(&self, uniform_buffer: UniformBuffer) {
        lock(&self.uniform_buffer_pool).push(uniform_buffer);
    }

    // Disposal

    /// Translation of `D3D12_ReleaseSampler()`.
    pub(super) fn release_sampler_internal(&self, sampler: Arc<D3D12Sampler>) {
        lock(&self.dispose).samplers_to_destroy.push(sampler);
    }

    /// Translation of `D3D12_ReleaseComputePipeline()`.
    pub(super) fn release_compute_pipeline_internal(&self, pipeline: Arc<D3D12ComputePipeline>) {
        lock(&self.dispose)
            .compute_pipelines_to_destroy
            .push(pipeline);
    }

    /// Translation of `D3D12_ReleaseGraphicsPipeline()`.
    pub(super) fn release_graphics_pipeline_internal(&self, pipeline: Arc<D3D12GraphicsPipeline>) {
        lock(&self.dispose)
            .graphics_pipelines_to_destroy
            .push(pipeline);
    }
}

/// Both DXIL and DXBC bytecode have a 4 byte header containing `DXBC`.
/// Translation of `D3D12_INTERNAL_IsValidShaderBytecode()`.
pub(super) fn is_valid_shader_bytecode(code: &[u8]) -> bool {
    code.len() >= 4 && &code[..4] == b"DXBC"
}
