// Rust translation of the GPU render state parts of src/render/SDL_render.c
// and include/SDL3/SDL_render.h from Simple DirectMedia Layer.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! Custom GPU render states: a fragment shader of the application's, with
//! the samplers, storage textures, storage buffers and fragment uniforms
//! it takes, that the GPU renderer draws with instead of its own shader
//! while the state is set ([`Renderer::set_gpu_render_state`]).
//!
//! Upstream, a state points at its renderer and its functions take only the
//! state; here the renderer keeps its states, which are addressed with
//! [`GpuRenderState`] handles (as textures are with [`Texture`] handles),
//! so the functions are the renderer's. A state shares its shader,
//! textures, samplers and buffers with the application (upstream copies
//! the pointers, which the application keeps valid), and is dropped with
//! the renderer if it isn't destroyed first.
//!
//! [`Texture`]: super::Texture

use std::sync::Arc;

use super::sysrender::{GpuRenderStateData, GpuRenderStateUniformBuffer};
use super::{Renderer, PROP_RENDERER_GPU_DEVICE_POINTER};
use crate::error::{Error, Result};
use crate::gpu;

/// A custom GPU render state of a [`Renderer`]: a handle, valid until
/// [`Renderer::destroy_gpu_render_state`] or the renderer is dropped.
/// Translation of `SDL_GPURenderState *`.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct GpuRenderState {
    pub(crate) renderer: u32,
    pub(crate) index: u32,
    pub(crate) generation: u32,
}

/// A texture-sampler pair a GPU render state binds for its fragment shader
/// (an `SDL_GPUTextureSamplerBinding` the state keeps).
#[derive(Clone, Debug)]
pub struct GpuRenderStateSamplerBinding {
    /// The texture to bind
    pub texture: Arc<gpu::Texture>,
    /// The sampler to bind
    pub sampler: Arc<gpu::Sampler>,
}

/// A structure specifying the parameters of a GPU render state.
/// Translation of `SDL_GPURenderStateCreateInfo` (without `props`: there
/// are no extensions).
#[derive(Clone, Debug)]
pub struct GpuRenderStateCreateInfo {
    /// The fragment shader to use when this render state is active (upstream
    /// requires one too: "A fragment_shader is required")
    pub fragment_shader: Arc<gpu::Shader>,
    /// Additional fragment samplers to bind when this render state is
    /// active, after the texture drawn (the slots after its samplers)
    pub sampler_bindings: Vec<GpuRenderStateSamplerBinding>,
    /// Storage textures to bind when this render state is active
    pub storage_textures: Vec<Arc<gpu::Texture>>,
    /// Storage buffers to bind when this render state is active
    pub storage_buffers: Vec<Arc<gpu::Buffer>>,
}

impl GpuRenderStateCreateInfo {
    /// The parameters of a state with `fragment_shader` and nothing else to
    /// bind.
    pub fn new(fragment_shader: Arc<gpu::Shader>) -> GpuRenderStateCreateInfo {
        GpuRenderStateCreateInfo {
            fragment_shader,
            sampler_bindings: Vec::new(),
            storage_textures: Vec::new(),
            storage_buffers: Vec::new(),
        }
    }
}

impl Renderer {
    /// The data of `state`, a state of this renderer (upstream's
    /// `if (!state) return SDL_InvalidParamError("state")`).
    fn gpu_render_state_data(&mut self, state: GpuRenderState) -> Result<&mut GpuRenderStateData> {
        if state.renderer != self.id {
            return Err(Error::new(
                "GPU render state was not created with this renderer",
            ));
        }
        self.gpu_render_states
            .get_mut(state)
            .ok_or_else(|| Error::invalid_param("state"))
    }

    /// Translation of `FlushRenderCommandsIfGPURenderStateNeeded()`.
    fn flush_if_gpu_render_state_needed(&mut self, state: GpuRenderState) -> Result<()> {
        let generation = self.render_command_generation;
        if self.gpu_render_state_data(state)?.last_command_generation == generation {
            // the current command queue depends on this state, flush the queue now before it changes
            return self.flush_render_commands();
        }
        Ok(())
    }

    /// Create custom GPU render state. Translation of
    /// `SDL_CreateGPURenderState()`.
    pub fn create_gpu_render_state(
        &mut self,
        createinfo: &GpuRenderStateCreateInfo,
    ) -> Result<GpuRenderState> {
        self.sync_window()?;

        // (the fragment shader can't be missing)

        if self
            .props
            .get_any::<gpu::Device>(PROP_RENDERER_GPU_DEVICE_POINTER)
            .is_none()
        {
            return Err(Error::new("Renderer isn't associated with a GPU device"));
        }

        let state = GpuRenderStateData {
            last_command_generation: 0,
            fragment_shader: createinfo.fragment_shader.clone(),
            sampler_bindings: createinfo.sampler_bindings.clone(),
            storage_textures: createinfo.storage_textures.clone(),
            storage_buffers: createinfo.storage_buffers.clone(),
            uniform_buffers: Vec::new(),
        };

        Ok(self.gpu_render_states.insert(self.id, state))
    }

    /// Set sampler bindings variables in a custom GPU render state: the
    /// bindings are kept and will be bound with
    /// [`gpu::RenderPass::bind_fragment_samplers`] during draw call
    /// execution. Translation of `SDL_SetGPURenderStateSamplerBindings()`.
    pub fn set_gpu_render_state_sampler_bindings(
        &mut self,
        state: GpuRenderState,
        sampler_bindings: &[GpuRenderStateSamplerBinding],
    ) -> Result<()> {
        self.flush_if_gpu_render_state_needed(state)?;

        self.gpu_render_state_data(state)?.sampler_bindings = sampler_bindings.to_vec();

        Ok(())
    }

    /// Set storage textures variables in a custom GPU render state: the
    /// textures are kept and will be bound with
    /// [`gpu::RenderPass::bind_fragment_storage_textures`] during draw call
    /// execution. Translation of `SDL_SetGPURenderStateStorageTextures()`.
    pub fn set_gpu_render_state_storage_textures(
        &mut self,
        state: GpuRenderState,
        storage_textures: &[Arc<gpu::Texture>],
    ) -> Result<()> {
        self.flush_if_gpu_render_state_needed(state)?;

        self.gpu_render_state_data(state)?.storage_textures = storage_textures.to_vec();

        Ok(())
    }

    /// Set storage buffers variables in a custom GPU render state: the
    /// buffers are kept and will be bound with
    /// [`gpu::RenderPass::bind_fragment_storage_buffers`] during draw call
    /// execution. Translation of `SDL_SetGPURenderStateStorageBuffers()`.
    pub fn set_gpu_render_state_storage_buffers(
        &mut self,
        state: GpuRenderState,
        storage_buffers: &[Arc<gpu::Buffer>],
    ) -> Result<()> {
        self.flush_if_gpu_render_state_needed(state)?;

        self.gpu_render_state_data(state)?.storage_buffers = storage_buffers.to_vec();

        Ok(())
    }

    /// Set fragment shader uniform variables in a custom GPU render state:
    /// the data is copied and will be pushed with
    /// [`gpu::CommandBuffer::push_fragment_uniform_data`] to `slot_index`
    /// during draw call execution. Translation of
    /// `SDL_SetGPURenderStateFragmentUniforms()`.
    pub fn set_gpu_render_state_fragment_uniforms(
        &mut self,
        state: GpuRenderState,
        slot_index: u32,
        data: &[u8],
    ) -> Result<()> {
        self.flush_if_gpu_render_state_needed(state)?;

        let state = self.gpu_render_state_data(state)?;
        for buffer in &mut state.uniform_buffers {
            if buffer.slot_index == slot_index {
                buffer.data = data.to_vec();
                return Ok(());
            }
        }

        state.uniform_buffers.push(GpuRenderStateUniformBuffer {
            slot_index,
            data: data.to_vec(),
        });
        Ok(())
    }

    /// Set custom GPU render state for subsequent draw calls, or `None` to
    /// clear it. This allows using custom shaders with the GPU renderer.
    /// Translation of `SDL_SetGPURenderState()`.
    pub fn set_gpu_render_state(&mut self, state: Option<GpuRenderState>) -> Result<()> {
        self.sync_window()?;

        // (a destroyed state is refused: upstream would keep a dangling
        // pointer)
        if let Some(state) = state {
            self.gpu_render_state_data(state)?;
        }

        self.gpu_render_state = state;
        Ok(())
    }

    /// Destroy custom GPU render state. Translation of
    /// `SDL_DestroyGPURenderState()`.
    pub fn destroy_gpu_render_state(&mut self, state: GpuRenderState) {
        if self.gpu_render_state_data(state).is_err() {
            return;
        }

        let _ = self.flush_if_gpu_render_state_needed(state);

        // (dropping the data frees the uniform buffers and the bindings; a
        // renderer still set to the state draws without one, where upstream
        // would keep a dangling pointer)
        self.gpu_render_states.remove(state);
        if self.gpu_render_state == Some(state) {
            self.gpu_render_state = None;
        }
    }
}
