// Rust translation of the resource parts of src/gpu/metal/SDL_gpu_metal.m
// from Simple DirectMedia Layer.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! Textures and buffers in their cycling containers, transfer buffers and
//! their mapping, uniform buffers and their pool, samplers, shaders (MSL
//! source and Metal libraries), the deferred destruction of released
//! containers (`PerformPendingDestroys`) and the debug names.
//!
//! A container's textures (or buffers) are `Arc`s: the container holds
//! them, and so does every command buffer that uses one, counting itself
//! in the texture's `referenceCount` as upstream's tracking does. A
//! released container waits in [`PendingDestroys`] until none of its
//! textures is used, then is dropped (`METAL_INTERNAL_DestroyTextureContainer()`),
//! releasing the Metal objects as upstream sets their handles to nil.

use std::ffi::c_void;
use std::sync::atomic::{AtomicI32, Ordering};
use std::sync::{Arc, Mutex};

use super::sys::*;
use super::tables::*;
use super::{lock, MetalRenderer};
use crate::core::apple::block::{dispatch_get_global_queue, dispatch_release};
use crate::core::apple::foundation::{ns_string, string_from_ns, NSRange, NS_UTF8_STRING_ENCODING};
use crate::core::apple::objc::{class, msg_send, Bool, Id, NSUInteger, StrongPtr, NIL};
use crate::error::{Error, Result};
use crate::gpu::{
    SampleCount, SamplerCreateInfo, ShaderCreateInfo, ShaderFormat, ShaderStage, TextureCreateInfo,
    TextureFormat, TextureType, TextureUsageFlags, PROP_GPU_SAMPLER_CREATE_NAME_STRING,
    PROP_GPU_TEXTURE_CREATE_NAME_STRING,
};
use crate::log::Category;
use crate::properties::Properties;

/// Translation of `MetalTexture`.
#[derive(Debug)]
pub(super) struct MetalTexture {
    pub(super) handle: StrongPtr,
    pub(super) reference_count: AtomicI32,
}

/// Translation of `MetalTextureContainer`.
#[derive(Debug)]
pub(super) struct TextureContainer {
    /// The creation info (`header.info`), with its own copy of the
    /// properties.
    pub(super) info: TextureCreateInfo,
    pub(super) state: Mutex<TextureContainerState>,
    pub(super) can_be_cycled: bool,
}

/// The cycled textures of a [`TextureContainer`].
#[derive(Debug)]
pub(super) struct TextureContainerState {
    pub(super) active_texture: Arc<MetalTexture>,
    pub(super) textures: Vec<Arc<MetalTexture>>,
    pub(super) debug_name: Option<String>,
}

impl TextureContainer {
    /// The active texture (`activeTexture`).
    pub(super) fn active(&self) -> Arc<MetalTexture> {
        lock(&self.state).active_texture.clone()
    }
}

/// Translation of `MetalBuffer`.
#[derive(Debug)]
pub(super) struct MetalBuffer {
    pub(super) handle: StrongPtr,
    pub(super) reference_count: AtomicI32,
}

/// Translation of `MetalBufferContainer` (transfer buffers too).
#[derive(Debug)]
pub(super) struct BufferContainer {
    pub(super) size: u32,
    pub(super) is_private: bool,
    pub(super) is_write_only: bool,
    pub(super) state: Mutex<BufferContainerState>,
}

/// The cycled buffers of a [`BufferContainer`].
#[derive(Debug)]
pub(super) struct BufferContainerState {
    pub(super) active_buffer: Arc<MetalBuffer>,
    pub(super) buffers: Vec<Arc<MetalBuffer>>,
    pub(super) debug_name: Option<String>,
}

impl BufferContainer {
    /// The active buffer (`activeBuffer`).
    pub(super) fn active(&self) -> Arc<MetalBuffer> {
        lock(&self.state).active_buffer.clone()
    }
}

/// Translation of `MetalUniformBuffer`.
#[derive(Debug)]
pub(super) struct MetalUniformBuffer {
    pub(super) handle: StrongPtr,
    pub(super) write_offset: u32,
    pub(super) draw_offset: u32,
}

/// Translation of `MetalSampler`.
#[derive(Debug)]
pub(super) struct MetalSampler {
    pub(super) handle: StrongPtr,
}

/// Translation of `MetalShader`.
#[derive(Debug)]
pub(super) struct MetalShader {
    #[allow(dead_code)] // (kept alive with its function, as upstream does)
    pub(super) library: StrongPtr,
    pub(super) function: StrongPtr,

    pub(super) stage: ShaderStage,
    pub(super) num_samplers: u32,
    pub(super) num_uniform_buffers: u32,
    pub(super) num_storage_buffers: u32,
    pub(super) num_storage_textures: u32,
}

/// The released containers (`bufferContainersToDestroy`,
/// `textureContainersToDestroy`, with `disposeLock`).
#[derive(Debug, Default)]
pub(super) struct PendingDestroys {
    pub(super) buffer_containers_to_destroy: Vec<Arc<BufferContainer>>,
    pub(super) texture_containers_to_destroy: Vec<Arc<TextureContainer>>,
}

/// Translation of `METAL_INTERNAL_NextHighestAlignment()`.
// FIXME: This should be moved into SDL_sysgpu.h
pub(super) fn next_highest_alignment(n: u32, align: u32) -> u32 {
    align.wrapping_mul(n.wrapping_add(align).wrapping_sub(1) / align)
}

/// Translation of `METAL_INTERNAL_IsValidMetalLibrary()`.
pub(super) fn is_valid_metal_library(code: &[u8]) -> bool {
    // Metal libraries have a 4 byte header containing `MTLB`.
    if code.len() < 4 {
        return false;
    }
    &code[..4] == b"MTLB"
}

/// `[[error description] UTF8String]` (empty for nil).
///
/// # Safety
///
/// `error` must be nil or an `NSError`.
pub(super) unsafe fn error_description(error: Id) -> String {
    if error.is_null() {
        return String::new();
    }
    // SAFETY: the caller's contract.
    unsafe {
        let description: Id = msg_send![error, description];
        string_from_ns(description).unwrap_or_default()
    }
}

/// `object.label = @(text)`.
///
/// # Safety
///
/// `object` must be nil or a Metal object with a label.
pub(super) unsafe fn set_label(object: Id, text: &str) {
    let label = ns_string(text);
    // SAFETY: the caller's contract.
    let _: () = unsafe { msg_send![object, setLabel: label.get()] };
}

/// The resource options of a buffer container's buffers.
fn buffer_resource_options(is_private: bool, is_write_only: bool) -> NSUInteger {
    if is_private {
        MTL_RESOURCE_STORAGE_MODE_PRIVATE
    } else if is_write_only {
        MTL_RESOURCE_CPU_CACHE_MODE_WRITE_COMBINED
    } else {
        MTL_RESOURCE_CPU_CACHE_MODE_DEFAULT_CACHE
    }
}

impl MetalRenderer {
    // Shader Compilation

    /// Translation of `METAL_INTERNAL_CompileShader()`: the library and
    /// the function (`MetalLibraryFunction`), or `None` (with nil handles
    /// upstream) after logging why.
    ///
    /// This function assumes that it's called from within an autorelease pool
    pub(super) fn compile_shader(
        &self,
        format: ShaderFormat,
        code: &[u8],
        entrypoint: &str,
    ) -> Result<Option<(StrongPtr, StrongPtr)>> {
        // (an empty entry point is C's NULL one)
        let entrypoint = if entrypoint.is_empty() {
            "main0"
        } else {
            entrypoint
        };

        let mut error: Id = NIL;
        // SAFETY: the device's methods with their arguments; the error is
        // an autoreleased out-parameter.
        let library = unsafe {
            if format == ShaderFormat::MSL {
                let code_string: Id = msg_send![class!(NSString), alloc];
                let code_string: Id = msg_send![code_string, initWithBytes: code.as_ptr() as *const c_void
                                                             length: code.len() as NSUInteger
                                                           encoding: NS_UTF8_STRING_ENCODING];
                let code_string = StrongPtr::from_owned(code_string);
                StrongPtr::from_owned(
                    msg_send![self.device, newLibraryWithSource: code_string.get()
                                                                         options: NIL
                                                                           error: &mut error as *mut Id],
                )
            } else if format == ShaderFormat::METALLIB {
                if !is_valid_metal_library(code) {
                    return Err(self.set_string_error(
                        "The provided shader code is not a valid Metal library!",
                    ));
                }
                let data = dispatch_data_create(
                    code.as_ptr() as *const c_void,
                    code.len(),
                    dispatch_get_global_queue(0, 0),
                    DISPATCH_DATA_DESTRUCTOR_DEFAULT,
                );
                let library: Id = msg_send![self.device, newLibraryWithData: data
                                                                      error: &mut error as *mut Id];
                // (ARC releases the data at the end of the scope)
                dispatch_release(data);
                StrongPtr::from_owned(library)
            } else {
                crate::sdl_assert!(!"SDL_gpu.c should have already validated this!");
                return Ok(None);
            }
        };

        if library.is_nil() {
            crate::log::error!(
                Category::Gpu,
                "Creating MTLLibrary failed: {}",
                // SAFETY: nil or the NSError the call set.
                unsafe { error_description(error) }
            );
            return Ok(None);
        } else if !error.is_null() {
            crate::log::warn!(
                Category::Gpu,
                "Creating MTLLibrary failed: {}",
                // SAFETY: as above.
                unsafe { error_description(error) }
            );
        }

        let name = ns_string(entrypoint);
        // SAFETY: a library's method returning a +1 function, or nil.
        let function =
            unsafe { StrongPtr::from_owned(msg_send![library, newFunctionWithName: name.get()]) };
        if function.is_nil() {
            crate::log::error!(Category::Gpu, "Creating MTLFunction failed");
            return Ok(None);
        }

        Ok(Some((library, function)))
    }

    // Disposal

    /// Translation of `METAL_ReleaseTexture()`.
    pub(super) fn release_texture_container(&self, container: Arc<TextureContainer>) {
        lock(&self.dispose)
            .texture_containers_to_destroy
            .push(container);
    }

    /// Translation of `METAL_ReleaseBuffer()` (and
    /// `METAL_ReleaseTransferBuffer()`).
    pub(super) fn release_buffer_container(&self, container: Arc<BufferContainer>) {
        lock(&self.dispose)
            .buffer_containers_to_destroy
            .push(container);
    }

    /// Translation of `METAL_INTERNAL_PerformPendingDestroys()`: drop the
    /// released containers none of whose objects is in use.
    ///
    /// This function assumes that it's called from within an autorelease pool
    pub(super) fn perform_pending_destroys(&self) {
        let mut dispose = lock(&self.dispose);

        let mut i = dispose.buffer_containers_to_destroy.len();
        while i > 0 {
            i -= 1;
            let reference_count: i32 = lock(&dispose.buffer_containers_to_destroy[i].state)
                .buffers
                .iter()
                .map(|b| b.reference_count.load(Ordering::SeqCst))
                .sum();

            if reference_count == 0 {
                // METAL_INTERNAL_DestroyBufferContainer()
                dispose.buffer_containers_to_destroy.swap_remove(i);
            }
        }

        let mut i = dispose.texture_containers_to_destroy.len();
        while i > 0 {
            i -= 1;
            let reference_count: i32 = lock(&dispose.texture_containers_to_destroy[i].state)
                .textures
                .iter()
                .map(|t| t.reference_count.load(Ordering::SeqCst))
                .sum();

            if reference_count == 0 {
                // METAL_INTERNAL_DestroyTextureContainer()
                dispose.texture_containers_to_destroy.swap_remove(i);
            }
        }
    }

    // Debug Naming

    /// Translation of `METAL_SetBufferName()`.
    pub(super) fn set_buffer_name_internal(&self, container: &BufferContainer, text: &str) {
        if self.debug_mode {
            let mut state = lock(&container.state);
            state.debug_name = Some(text.to_owned());

            for buffer in &state.buffers {
                // SAFETY: a buffer.
                unsafe { set_label(buffer.handle.get(), text) };
            }
        }
    }

    /// Translation of `METAL_SetTextureName()`.
    pub(super) fn set_texture_name_internal(&self, container: &TextureContainer, text: &str) {
        if self.debug_mode {
            let mut state = lock(&container.state);
            state.debug_name = Some(text.to_owned());

            for texture in &state.textures {
                // SAFETY: a texture.
                unsafe { set_label(texture.handle.get(), text) };
            }
        }
    }

    // Resource Creation

    /// Translation of `METAL_CreateSampler()`.
    pub(super) fn create_sampler_internal(
        &self,
        createinfo: &SamplerCreateInfo,
    ) -> Result<MetalSampler> {
        // SAFETY: a new sampler descriptor, configured through its
        // properties, then a +1 sampler state from the device.
        let sampler = unsafe {
            let sampler_desc = StrongPtr::from_owned(msg_send![class!(MTLSamplerDescriptor), new]);

            let _: () = msg_send![sampler_desc, setSAddressMode: sdl_to_metal_sampler_address_mode(createinfo.address_mode_u)];
            let _: () = msg_send![sampler_desc, setTAddressMode: sdl_to_metal_sampler_address_mode(createinfo.address_mode_v)];
            let _: () = msg_send![sampler_desc, setRAddressMode: sdl_to_metal_sampler_address_mode(createinfo.address_mode_w)];
            let _: () = msg_send![sampler_desc, setMinFilter: sdl_to_metal_min_mag_filter(createinfo.min_filter)];
            let _: () = msg_send![sampler_desc, setMagFilter: sdl_to_metal_min_mag_filter(createinfo.mag_filter)];
            let _: () = msg_send![sampler_desc, setMipFilter: sdl_to_metal_mip_filter(createinfo.mipmap_mode)]; // FIXME: Is this right with non-mipmapped samplers?
            let _: () = msg_send![sampler_desc, setLodMinClamp: createinfo.min_lod];
            let _: () = msg_send![sampler_desc, setLodMaxClamp: createinfo.max_lod];
            let max_anisotropy = (if createinfo.enable_anisotropy {
                createinfo.max_anisotropy
            } else {
                1.0
            }) as NSUInteger;
            let _: () = msg_send![sampler_desc, setMaxAnisotropy: max_anisotropy];
            let compare_function = if createinfo.enable_compare {
                sdl_to_metal_compare_op(createinfo.compare_op)
            } else {
                MTL_COMPARE_FUNCTION_ALWAYS
            };
            let _: () = msg_send![sampler_desc, setCompareFunction: compare_function];

            if self.debug_mode {
                if let Some(name) = createinfo
                    .props
                    .as_ref()
                    .and_then(|p| p.get_string(PROP_GPU_SAMPLER_CREATE_NAME_STRING))
                {
                    set_label(sampler_desc.get(), &name);
                }
            }

            StrongPtr::from_owned(
                msg_send![self.device, newSamplerStateWithDescriptor: sampler_desc.get()],
            )
        };
        if sampler.is_nil() {
            return Err(self.set_string_error("Failed to create sampler"));
        }

        Ok(MetalSampler { handle: sampler })
    }

    /// Translation of `METAL_CreateShader()`.
    pub(super) fn create_shader_internal(
        &self,
        createinfo: &ShaderCreateInfo<'_>,
    ) -> Result<MetalShader> {
        let Some((library, function)) =
            self.compile_shader(createinfo.format, createinfo.code, createinfo.entrypoint)?
        else {
            return Err(Error::new("Failed to create shader"));
        };

        Ok(MetalShader {
            library,
            function,
            stage: createinfo.stage,
            num_samplers: createinfo.num_samplers,
            num_storage_buffers: createinfo.num_storage_buffers,
            num_storage_textures: createinfo.num_storage_textures,
            num_uniform_buffers: createinfo.num_uniform_buffers,
        })
    }

    /// Translation of `METAL_INTERNAL_CreateTexture()`.
    ///
    /// This function assumes that it's called from within an autorelease pool
    pub(super) fn create_texture_internal(
        &self,
        createinfo: &TextureCreateInfo,
    ) -> Result<Option<MetalTexture>> {
        // SAFETY: a new texture descriptor, configured through its
        // properties, then a +1 texture from the device.
        let texture = unsafe {
            let texture_descriptor =
                StrongPtr::from_owned(msg_send![class!(MTLTextureDescriptor), new]);

            let _: () = msg_send![texture_descriptor, setTextureType: sdl_to_metal_texture_type(
                createinfo.texture_type,
                createinfo.sample_count != SampleCount::One
            )];
            let _: () = msg_send![texture_descriptor, setPixelFormat: sdl_to_metal_texture_format(createinfo.format)];
            // This format isn't natively supported so let's swizzle!
            if createinfo.format == TextureFormat::B4G4R4A4_UNORM {
                if available(10, 15) {
                    let swizzle = MTLTextureSwizzleChannels {
                        red: MTL_TEXTURE_SWIZZLE_GREEN,
                        green: MTL_TEXTURE_SWIZZLE_BLUE,
                        blue: MTL_TEXTURE_SWIZZLE_ALPHA,
                        alpha: MTL_TEXTURE_SWIZZLE_RED,
                    };
                    let _: () = msg_send![texture_descriptor, setSwizzle: swizzle];
                } else {
                    return Err(self.set_string_error(
                        "SDL_GPU_TEXTUREFORMAT_B4G4R4A4_UNORM is not supported",
                    ));
                }
            }

            let _: () = msg_send![texture_descriptor, setWidth: createinfo.width as NSUInteger];
            let _: () = msg_send![texture_descriptor, setHeight: createinfo.height as NSUInteger];
            let depth = if createinfo.texture_type == TextureType::Texture3D {
                createinfo.layer_count_or_depth
            } else {
                1
            };
            let _: () = msg_send![texture_descriptor, setDepth: depth as NSUInteger];
            let _: () = msg_send![texture_descriptor, setMipmapLevelCount: createinfo.num_levels as NSUInteger];
            let _: () = msg_send![texture_descriptor, setSampleCount: sdl_to_metal_sample_count(createinfo.sample_count)];
            let array_length = if createinfo.texture_type == TextureType::Texture2DArray
                || createinfo.texture_type == TextureType::CubeArray
            {
                createinfo.layer_count_or_depth
            } else {
                1
            };
            let _: () = msg_send![texture_descriptor, setArrayLength: array_length as NSUInteger];
            let _: () = msg_send![texture_descriptor, setStorageMode: MTL_STORAGE_MODE_PRIVATE];

            let mut usage = 0;
            if createinfo.usage.intersects(
                TextureUsageFlags::COLOR_TARGET | TextureUsageFlags::DEPTH_STENCIL_TARGET,
            ) {
                usage |= MTL_TEXTURE_USAGE_RENDER_TARGET;
            }
            if createinfo.usage.intersects(
                TextureUsageFlags::SAMPLER
                    | TextureUsageFlags::GRAPHICS_STORAGE_READ
                    | TextureUsageFlags::COMPUTE_STORAGE_READ,
            ) {
                usage |= MTL_TEXTURE_USAGE_SHADER_READ;
            }
            if createinfo.usage.intersects(
                TextureUsageFlags::COMPUTE_STORAGE_WRITE
                    | TextureUsageFlags::COMPUTE_STORAGE_SIMULTANEOUS_READ_WRITE,
            ) {
                usage |= MTL_TEXTURE_USAGE_SHADER_WRITE;
            }
            let _: () = msg_send![texture_descriptor, setUsage: usage];

            StrongPtr::from_owned(
                msg_send![self.device, newTextureWithDescriptor: texture_descriptor.get()],
            )
        };
        if texture.is_nil() {
            crate::log::error!(Category::Gpu, "Failed to create MTLTexture!");
            return Ok(None);
        }

        let metal_texture = MetalTexture {
            handle: texture,
            reference_count: AtomicI32::new(0),
        };

        if self.debug_mode {
            if let Some(name) = createinfo
                .props
                .as_ref()
                .and_then(|p| p.get_string(PROP_GPU_TEXTURE_CREATE_NAME_STRING))
            {
                // SAFETY: a texture.
                unsafe { set_label(metal_texture.handle.get(), &name) };
            }
        }

        Ok(Some(metal_texture))
    }

    /// Translation of `METAL_SupportsSampleCount()`.
    pub(super) fn supports_sample_count_internal(&self, sample_count: SampleCount) -> bool {
        let mtl_sample_count = sdl_to_metal_sample_count(sample_count);
        // SAFETY: a device method taking a count.
        let r: Bool =
            unsafe { msg_send![self.device, supportsTextureSampleCount: mtl_sample_count] };
        r.is_true()
    }

    /// Translation of `METAL_CreateTexture()`.
    pub(super) fn create_texture_container(
        &self,
        createinfo: &TextureCreateInfo,
    ) -> Result<TextureContainer> {
        let texture = match self.create_texture_internal(createinfo) {
            Ok(Some(texture)) => Arc::new(texture),
            Ok(None) => return Err(self.set_string_error("Failed to create texture")),
            Err(e) => return Err(e),
        };

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
            .and_then(|p| p.get_string(PROP_GPU_TEXTURE_CREATE_NAME_STRING));

        Ok(TextureContainer {
            info,
            state: Mutex::new(TextureContainerState {
                active_texture: texture.clone(),
                textures: vec![texture],
                debug_name,
            }),
            can_be_cycled: true,
        })
    }

    /// Translation of `METAL_INTERNAL_PrepareTextureForWrite()`: the
    /// texture to write to, cycling to an unused (or new) one when asked.
    ///
    /// This function assumes that it's called from within an autorelease pool
    pub(super) fn prepare_texture_for_write(
        &self,
        container: &TextureContainer,
        cycle: bool,
    ) -> Arc<MetalTexture> {
        let mut state = lock(&container.state);

        // Cycle the active texture handle if needed
        if cycle && container.can_be_cycled {
            if let Some(texture) = state
                .textures
                .iter()
                .find(|t| t.reference_count.load(Ordering::SeqCst) == 0)
                .cloned()
            {
                state.active_texture = texture;
                return state.active_texture.clone();
            }

            // Note (upstream): C stores a NULL texture when the creation
            // fails, and crashes using it; the active texture stays here.
            match self.create_texture_internal(&container.info) {
                Ok(Some(texture)) => {
                    let texture = Arc::new(texture);
                    state.textures.push(texture.clone());
                    state.active_texture = texture;
                }
                _ => crate::log::error!(Category::Gpu, "Failed to cycle texture!"),
            }
        }

        state.active_texture.clone()
    }

    /// Translation of `METAL_INTERNAL_CreateBuffer()`.
    ///
    /// This function assumes that it's called from within an autorelease pool
    fn create_buffer(
        &self,
        size: u32,
        resource_options: NSUInteger,
        debug_name: Option<&str>,
    ) -> Option<MetalBuffer> {
        // Storage buffers have to be 4-aligned, so might as well align them all
        let size = next_highest_alignment(size, 4);

        // SAFETY: a device method returning a +1 buffer, or nil.
        let buffer_handle = unsafe {
            StrongPtr::from_owned(
                msg_send![self.device, newBufferWithLength: size as NSUInteger
                                                                     options: resource_options],
            )
        };
        if buffer_handle.is_nil() {
            crate::log::error!(Category::Gpu, "Could not create buffer");
            return None;
        }

        if let Some(debug_name) = debug_name {
            // SAFETY: a buffer.
            unsafe { set_label(buffer_handle.get(), debug_name) };
        }

        Some(MetalBuffer {
            handle: buffer_handle,
            reference_count: AtomicI32::new(0),
        })
    }

    /// Translation of `METAL_INTERNAL_CreateBufferContainer()`.
    ///
    /// This function assumes that it's called from within an autorelease pool
    pub(super) fn create_buffer_container(
        &self,
        size: u32,
        is_private: bool,
        is_write_only: bool,
        debug_name: Option<&str>,
    ) -> Result<BufferContainer> {
        let resource_options = buffer_resource_options(is_private, is_write_only);

        // Note (upstream): C goes on with a NULL buffer when the creation
        // fails, and crashes using it; the creation fails here.
        let Some(buffer) = self.create_buffer(size, resource_options, debug_name) else {
            return Err(Error::new("Could not create buffer"));
        };
        let buffer = Arc::new(buffer);

        Ok(BufferContainer {
            size,
            is_private,
            is_write_only,
            state: Mutex::new(BufferContainerState {
                active_buffer: buffer.clone(),
                buffers: vec![buffer],
                // Note (upstream): C sets debugName to NULL, then copies
                // the name only `if (container->debugName != NULL)`, so the
                // container never keeps it and the buffers made when it
                // cycles have no label; here it keeps it.
                debug_name: debug_name.map(str::to_owned),
            }),
        })
    }

    /// Translation of `METAL_INTERNAL_CreateUniformBuffer()`.
    ///
    /// This function assumes that it's called from within an autorelease pool
    pub(super) fn create_uniform_buffer(&self, size: u32) -> Option<MetalUniformBuffer> {
        // SAFETY: a device method returning a +1 buffer, or nil.
        let buffer_handle = unsafe {
            StrongPtr::from_owned(
                msg_send![self.device, newBufferWithLength: size as NSUInteger
                                                                     options: MTL_RESOURCE_CPU_CACHE_MODE_WRITE_COMBINED],
            )
        };
        if buffer_handle.is_nil() {
            crate::log::error!(Category::Gpu, "Could not create uniform buffer");
            return None;
        }

        Some(MetalUniformBuffer {
            handle: buffer_handle,
            write_offset: 0,
            draw_offset: 0,
        })
    }

    /// Translation of `METAL_INTERNAL_PrepareBufferForWrite()`: the buffer
    /// to write to, cycling to an unused (or new) one when asked and the
    /// active one is in use.
    ///
    /// This function assumes that it's called from within an autorelease pool
    pub(super) fn prepare_buffer_for_write(
        &self,
        container: &BufferContainer,
        cycle: bool,
    ) -> Arc<MetalBuffer> {
        let mut state = lock(&container.state);

        // Cycle if needed
        if cycle && state.active_buffer.reference_count.load(Ordering::SeqCst) > 0 {
            if let Some(buffer) = state
                .buffers
                .iter()
                .find(|b| b.reference_count.load(Ordering::SeqCst) == 0)
                .cloned()
            {
                state.active_buffer = buffer;
                return state.active_buffer.clone();
            }

            let resource_options =
                buffer_resource_options(container.is_private, container.is_write_only);

            // Note (upstream): C stores a NULL buffer when the creation
            // fails, and crashes using it; the active buffer stays here.
            let debug_name = state.debug_name.clone();
            match self.create_buffer(container.size, resource_options, debug_name.as_deref()) {
                Some(buffer) => {
                    let buffer = Arc::new(buffer);
                    state.buffers.push(buffer.clone());
                    state.active_buffer = buffer;
                }
                None => crate::log::error!(Category::Gpu, "Failed to cycle buffer!"),
            }
        }

        state.active_buffer.clone()
    }

    // TransferBuffer Data

    /// Translation of `METAL_MapTransferBuffer()`.
    pub(super) fn map_transfer_buffer_internal(
        &self,
        container: &BufferContainer,
        cycle: bool,
    ) -> *mut u8 {
        let buffer = self.prepare_buffer_for_write(container, cycle);
        // SAFETY: a shared buffer's contents pointer.
        unsafe {
            let contents: *mut c_void = msg_send![buffer.handle, contents];
            contents as *mut u8
        }
    }

    /// Translation of `METAL_UnmapTransferBuffer()`.
    pub(super) fn unmap_transfer_buffer_internal(&self, container: &BufferContainer) {
        // FIXME: Is this necessary?
        let buffer = container.active();
        // SAFETY: a buffer's storage mode, and the range of its contents
        // that was written.
        unsafe {
            let storage_mode: NSUInteger = msg_send![buffer.handle, storageMode];
            if storage_mode == MTL_STORAGE_MODE_MANAGED {
                let _: () = msg_send![buffer.handle, didModifyRange: NSRange::new(0, container.size as NSUInteger)];
            }
        }
    }
}
