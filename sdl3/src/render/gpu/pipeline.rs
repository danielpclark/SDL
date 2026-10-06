// Rust translation of src/render/gpu/SDL_pipeline_gpu.c and SDL_pipeline_gpu.h
// from Simple DirectMedia Layer.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! The GPU renderer's graphics pipelines, made on first use and cached by
//! the state they're made for.

use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::sync::Arc;

use super::shaders::{FragmentShaderId, VertexShaderId};
use super::{convert_blend_factor, convert_blend_operation, Shaders};
use crate::error::{Error, Result};
use crate::gpu::{self, ColorComponentFlags, PrimitiveType, TextureFormat};
use crate::video::BlendMode;

/// What a pipeline is made for. Translation of `GPU_PipelineParameters`.
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub(super) struct PipelineParameters {
    pub(super) blend_mode: BlendMode,
    pub(super) frag_shader: FragmentShaderId,
    pub(super) vert_shader: VertexShaderId,
    pub(super) attachment_format: TextureFormat,
    pub(super) primitive_type: PrimitiveType,
    pub(super) custom_frag_shader: Option<CustomShader>,
}

/// A render state's fragment shader in the parameters of a pipeline
/// (`custom_frag_shader`), compared and hashed by its address as
/// upstream's pointer is. The cache keeps it with the key, so the address
/// can't be reused by another shader while the pipeline is cached.
#[derive(Clone, Debug)]
pub(super) struct CustomShader(pub(super) Arc<gpu::Shader>);

impl PartialEq for CustomShader {
    fn eq(&self, other: &CustomShader) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
}

impl Eq for CustomShader {}

impl Hash for CustomShader {
    fn hash<H: Hasher>(&self, state: &mut H) {
        Arc::as_ptr(&self.0).hash(state);
    }
}

/// The pipelines made so far. Translation of `GPU_PipelineCache` (its hash
/// table, keyed by the parameters as upstream's murmur3 hash and `memcmp()`
/// key it); dropping it is `GPU_DestroyPipelineCache()`, which releases the
/// pipelines.
#[derive(Default)]
pub(super) struct PipelineCache {
    table: HashMap<PipelineParameters, gpu::GraphicsPipeline>,
}

/// The color blend state of a blend mode (`MakePipeline()`'s
/// `ad.blend_state`).
///
/// Note (upstream): C hands the GPU API `SDL_GPU_BLENDFACTOR_INVALID` and
/// `SDL_GPU_BLENDOP_INVALID` for a mode `GPU_SupportsBlendMode()` refuses
/// (which the front end never draws with); here such a mode has no blend
/// state, and no pipeline.
pub(super) fn blend_state(blend: BlendMode) -> Option<gpu::ColorTargetBlendState> {
    Some(gpu::ColorTargetBlendState {
        enable_blend: blend != BlendMode::NONE,
        color_write_mask: ColorComponentFlags(0xF),
        alpha_blend_op: convert_blend_operation(blend.alpha_operation())?,
        dst_alpha_blendfactor: convert_blend_factor(blend.dst_alpha_factor())?,
        src_alpha_blendfactor: convert_blend_factor(blend.src_alpha_factor())?,
        color_blend_op: convert_blend_operation(blend.color_operation())?,
        dst_color_blendfactor: convert_blend_factor(blend.dst_color_factor())?,
        src_color_blendfactor: convert_blend_factor(blend.src_color_factor())?,
        enable_color_write_mask: false,
    })
}

/// The vertex layout of a vertex shader (`MakePipeline()`'s vertex buffer
/// description and attributes): a position and a color, and texture
/// coordinates for the triangle shaders.
pub(super) fn vertex_layout(
    vert_shader: VertexShaderId,
) -> (gpu::VertexBufferDescription, Vec<gpu::VertexAttribute>) {
    let mut vertex_buffer_desc = gpu::VertexBufferDescription::default();
    let mut attribs = Vec::with_capacity(4);

    let have_attr_uv = match vert_shader {
        VertexShaderId::TriColor => true,
        VertexShaderId::TriTexture => true,
        VertexShaderId::Linepoint => false,
    };

    let mut push = |format: gpu::VertexElementFormat, floats: u32| {
        attribs.push(gpu::VertexAttribute {
            location: attribs.len() as u32,
            buffer_slot: 0,
            format,
            offset: vertex_buffer_desc.pitch,
        });
        vertex_buffer_desc.pitch += floats * size_of::<f32>() as u32;
    };

    // Position
    push(gpu::VertexElementFormat::Float2, 2);

    // Color
    push(gpu::VertexElementFormat::Float4, 4);

    if have_attr_uv {
        // UVs
        push(gpu::VertexElementFormat::Float2, 2);
    }

    (vertex_buffer_desc, attribs)
}

/// Translation of `MakePipeline()`.
fn make_pipeline(
    device: &gpu::Device,
    shaders: &Shaders,
    params: &PipelineParameters,
) -> Result<gpu::GraphicsPipeline> {
    let blend_state = blend_state(params.blend_mode)
        .ok_or_else(|| Error::new(format!("Unsupported blend mode {:#x}", params.blend_mode.0)))?;
    let ad = gpu::ColorTargetDescription {
        format: params.attachment_format,
        blend_state,
    };

    let (vertex_buffer_desc, attribs) = vertex_layout(params.vert_shader);

    let pci = gpu::GraphicsPipelineCreateInfo {
        vertex_shader: shaders.vertex_shader(params.vert_shader),
        fragment_shader: shaders
            .fragment_shader(params.frag_shader)
            .ok_or_else(|| Error::new("No custom fragment shader"))?,
        vertex_input_state: gpu::VertexInputState {
            vertex_buffer_descriptions: &[vertex_buffer_desc],
            vertex_attributes: &attribs,
        },
        primitive_type: params.primitive_type,
        rasterizer_state: gpu::RasterizerState {
            cull_mode: gpu::CullMode::None,
            fill_mode: gpu::FillMode::Fill,
            front_face: gpu::FrontFace::CounterClockwise,
            enable_depth_clip: true,
            ..Default::default()
        },
        multisample_state: gpu::MultisampleState {
            sample_count: gpu::SampleCount::One,
            enable_mask: false,
            ..Default::default()
        },
        depth_stencil_state: gpu::DepthStencilState::default(),
        target_info: gpu::GraphicsPipelineTargetInfo {
            color_target_descriptions: &[ad],
            depth_stencil_format: TextureFormat::INVALID,
            has_depth_stencil_target: false,
        },
        props: None,
    };

    device.create_graphics_pipeline(&pci)
}

impl PipelineCache {
    /// The pipeline for `params`, made the first time. Translation of
    /// `GPU_GetPipeline()`.
    pub(super) fn get_pipeline(
        &mut self,
        shaders: &Shaders,
        device: &gpu::Device,
        params: &PipelineParameters,
    ) -> Result<&gpu::GraphicsPipeline> {
        if !self.table.contains_key(params) {
            let pipeline = make_pipeline(device, shaders, params)?;
            self.table.insert(params.clone(), pipeline);
        }
        Ok(&self.table[params])
    }

    /// The number of pipelines made (for the tests).
    #[cfg(test)]
    pub(super) fn len(&self) -> usize {
        self.table.len()
    }
}
