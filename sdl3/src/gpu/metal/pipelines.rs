// Rust translation of the pipeline parts of src/gpu/metal/SDL_gpu_metal.m
// from Simple DirectMedia Layer.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! Graphics pipelines (with their depth-stencil states) and compute
//! pipelines.

use super::resources::{error_description, set_label, MetalShader};
use super::sys::*;
use super::tables::*;
use super::{object, MetalRenderer, METAL_FIRST_VERTEX_BUFFER_SLOT};
use crate::core::apple::objc::{class, msg_send, Bool, Id, NSUInteger, StrongPtr, NIL};
use crate::error::{Error, Result};
use crate::gpu::sysgpu::{ComputePipelineHeader, GraphicsPipelineHeader};
use crate::gpu::{
    ColorComponentFlags, ComputePipelineCreateInfo, GraphicsPipelineCreateInfo, PrimitiveType,
    RasterizerState, ShaderStage, PROP_GPU_COMPUTEPIPELINE_CREATE_NAME_STRING,
    PROP_GPU_GRAPHICSPIPELINE_CREATE_NAME_STRING,
};

/// Translation of `MetalGraphicsPipeline`.
#[derive(Debug)]
pub(super) struct MetalGraphicsPipeline {
    pub(super) header: GraphicsPipelineHeader,

    pub(super) handle: StrongPtr,

    pub(super) rasterizer_state: RasterizerState,
    pub(super) primitive_type: PrimitiveType,

    pub(super) depth_stencil_state: StrongPtr,
}

/// Translation of `MetalComputePipeline`.
#[derive(Debug)]
pub(super) struct MetalComputePipeline {
    pub(super) header: ComputePipelineHeader,

    pub(super) handle: StrongPtr,
    pub(super) threadcount_x: u32,
    pub(super) threadcount_y: u32,
    pub(super) threadcount_z: u32,
}

/// `array[index]` of a Metal descriptor array (`objectAtIndexedSubscript:`).
///
/// # Safety
///
/// `array` must be a Metal descriptor array.
pub(super) unsafe fn subscript(array: Id, index: u32) -> Id {
    // SAFETY: the caller's contract.
    unsafe { msg_send![array, objectAtIndexedSubscript: index as NSUInteger] }
}

impl MetalRenderer {
    // Pipeline Creation

    /// Translation of `METAL_CreateComputePipeline()`.
    pub(super) fn create_compute_pipeline_internal(
        &self,
        createinfo: &ComputePipelineCreateInfo<'_>,
    ) -> Result<MetalComputePipeline> {
        let Some((_library, function)) =
            self.compile_shader(createinfo.format, createinfo.code, createinfo.entrypoint)?
        else {
            return Err(Error::new("Failed to create compute pipeline"));
        };

        let mut error: Id = NIL;
        // SAFETY: a new compute pipeline descriptor with the function, then
        // a +1 pipeline state from the device (the error an autoreleased
        // out-parameter).
        let handle = unsafe {
            let descriptor =
                StrongPtr::from_owned(msg_send![class!(MTLComputePipelineDescriptor), new]);
            let _: () = msg_send![descriptor, setComputeFunction: function.get()];

            if self.debug_mode {
                if let Some(name) = createinfo
                    .props
                    .as_ref()
                    .and_then(|p| p.get_string(PROP_GPU_COMPUTEPIPELINE_CREATE_NAME_STRING))
                {
                    set_label(descriptor.get(), &name);
                }
            }

            StrongPtr::from_owned(
                msg_send![self.device, newComputePipelineStateWithDescriptor: descriptor.get()
                                                                                      options: MTL_PIPELINE_OPTION_NONE
                                                                                   reflection: std::ptr::null_mut::<Id>()
                                                                                        error: &mut error as *mut Id],
            )
        };
        if !error.is_null() {
            return Err(self.set_string_error(&format!(
                "Creating compute pipeline failed: {}",
                // SAFETY: the NSError the call set.
                unsafe { error_description(error) }
            )));
        }

        Ok(MetalComputePipeline {
            handle,
            header: ComputePipelineHeader {
                num_samplers: createinfo.num_samplers,
                num_readonly_storage_textures: createinfo.num_readonly_storage_textures,
                num_readwrite_storage_textures: createinfo.num_readwrite_storage_textures,
                num_readonly_storage_buffers: createinfo.num_readonly_storage_buffers,
                num_readwrite_storage_buffers: createinfo.num_readwrite_storage_buffers,
                num_uniform_buffers: createinfo.num_uniform_buffers,
            },
            threadcount_x: createinfo.threadcount_x,
            threadcount_y: createinfo.threadcount_y,
            threadcount_z: createinfo.threadcount_z,
        })
    }

    /// Translation of `METAL_CreateGraphicsPipeline()`.
    pub(super) fn create_graphics_pipeline_internal(
        &self,
        createinfo: &GraphicsPipelineCreateInfo<'_>,
    ) -> Result<MetalGraphicsPipeline> {
        let vertex_shader = object::<MetalShader>(&createinfo.vertex_shader.raw);
        let fragment_shader = object::<MetalShader>(&createinfo.fragment_shader.raw);

        if self.debug_mode {
            if vertex_shader.stage != ShaderStage::Vertex {
                crate::sdl_assert_release!(
                    !"CreateGraphicsPipeline was passed a fragment shader for the vertex stage"
                );
            }
            if fragment_shader.stage != ShaderStage::Fragment {
                crate::sdl_assert_release!(
                    !"CreateGraphicsPipeline was passed a vertex shader for the fragment stage"
                );
            }
        }
        #[cfg(target_os = "visionos")]
        {
            // The default is depth clipping enabled and it can't be changed
            if !createinfo.rasterizer_state.enable_depth_clip {
                crate::sdl_assert_release!(
                    !"Rasterizer state enable_depth_clip must be true on this platform"
                );
            }
        }

        let mut error: Id = NIL;
        // SAFETY: new descriptors configured through their properties and
        // subscripts, then +1 states from the device (the error an
        // autoreleased out-parameter).
        let (pipeline_state, depth_stencil_state) = unsafe {
            let pipeline_descriptor =
                StrongPtr::from_owned(msg_send![class!(MTLRenderPipelineDescriptor), new]);

            // Blend

            let color_attachments: Id = msg_send![pipeline_descriptor, colorAttachments];
            for (i, description) in createinfo
                .target_info
                .color_target_descriptions
                .iter()
                .enumerate()
            {
                let blend_state = &description.blend_state;
                let color_write_mask = if blend_state.enable_color_write_mask {
                    blend_state.color_write_mask
                } else {
                    ColorComponentFlags(0xF)
                };

                let attachment = subscript(color_attachments, i as u32);
                let _: () = msg_send![attachment, setPixelFormat: sdl_to_metal_texture_format(description.format)];
                let _: () = msg_send![attachment, setWriteMask: sdl_to_metal_color_write_mask(color_write_mask)];
                let _: () =
                    msg_send![attachment, setBlendingEnabled: Bool::from(blend_state.enable_blend)];
                let _: () = msg_send![attachment, setRgbBlendOperation: sdl_to_metal_blend_op(blend_state.color_blend_op)];
                let _: () = msg_send![attachment, setAlphaBlendOperation: sdl_to_metal_blend_op(blend_state.alpha_blend_op)];
                let _: () = msg_send![attachment, setSourceRGBBlendFactor: sdl_to_metal_blend_factor(blend_state.src_color_blendfactor)];
                let _: () = msg_send![attachment, setSourceAlphaBlendFactor: sdl_to_metal_blend_factor(blend_state.src_alpha_blendfactor)];
                let _: () = msg_send![attachment, setDestinationRGBBlendFactor: sdl_to_metal_blend_factor(blend_state.dst_color_blendfactor)];
                let _: () = msg_send![attachment, setDestinationAlphaBlendFactor: sdl_to_metal_blend_factor(blend_state.dst_alpha_blendfactor)];
            }

            // Multisample

            let _: () = msg_send![pipeline_descriptor, setRasterSampleCount: sdl_to_metal_sample_count(createinfo.multisample_state.sample_count)];
            let _: () = msg_send![pipeline_descriptor, setAlphaToCoverageEnabled: Bool::from(createinfo.multisample_state.enable_alpha_to_coverage)];

            // Depth Stencil

            let mut depth_stencil_state = StrongPtr::nil();
            if createinfo.target_info.has_depth_stencil_target {
                let depth_stencil_format = createinfo.target_info.depth_stencil_format;
                let _: () = msg_send![pipeline_descriptor, setDepthAttachmentPixelFormat: sdl_to_metal_texture_format(depth_stencil_format)];
                if depth_stencil_format.is_stencil_format() {
                    let _: () = msg_send![pipeline_descriptor, setStencilAttachmentPixelFormat: sdl_to_metal_texture_format(depth_stencil_format)];
                }

                let ds = &createinfo.depth_stencil_state;
                let mut front_stencil_descriptor = StrongPtr::nil();
                let mut back_stencil_descriptor = StrongPtr::nil();
                if ds.enable_stencil_test {
                    for (descriptor, state) in [
                        (&mut front_stencil_descriptor, &ds.front_stencil_state),
                        (&mut back_stencil_descriptor, &ds.back_stencil_state),
                    ] {
                        *descriptor =
                            StrongPtr::from_owned(msg_send![class!(MTLStencilDescriptor), new]);
                        let _: () = msg_send![*descriptor, setStencilCompareFunction: sdl_to_metal_compare_op(state.compare_op)];
                        let _: () = msg_send![*descriptor, setStencilFailureOperation: sdl_to_metal_stencil_op(state.fail_op)];
                        let _: () = msg_send![*descriptor, setDepthStencilPassOperation: sdl_to_metal_stencil_op(state.pass_op)];
                        let _: () = msg_send![*descriptor, setDepthFailureOperation: sdl_to_metal_stencil_op(state.depth_fail_op)];
                        let _: () = msg_send![*descriptor, setReadMask: ds.compare_mask as u32];
                        let _: () = msg_send![*descriptor, setWriteMask: ds.write_mask as u32];
                    }
                }

                let depth_stencil_descriptor =
                    StrongPtr::from_owned(msg_send![class!(MTLDepthStencilDescriptor), new]);
                let depth_compare_function = if ds.enable_depth_test {
                    sdl_to_metal_compare_op(ds.compare_op)
                } else {
                    MTL_COMPARE_FUNCTION_ALWAYS
                };
                let _: () = msg_send![depth_stencil_descriptor, setDepthCompareFunction: depth_compare_function];
                // Disable write when test is disabled, to match other APIs' behavior
                let _: () = msg_send![depth_stencil_descriptor, setDepthWriteEnabled: Bool::from(ds.enable_depth_write && ds.enable_depth_test)];
                let _: () = msg_send![depth_stencil_descriptor, setFrontFaceStencil: front_stencil_descriptor.get()];
                let _: () = msg_send![depth_stencil_descriptor, setBackFaceStencil: back_stencil_descriptor.get()];

                depth_stencil_state = StrongPtr::from_owned(
                    msg_send![self.device, newDepthStencilStateWithDescriptor: depth_stencil_descriptor.get()],
                );
            }

            // Shaders

            let _: () =
                msg_send![pipeline_descriptor, setVertexFunction: vertex_shader.function.get()];
            let _: () =
                msg_send![pipeline_descriptor, setFragmentFunction: fragment_shader.function.get()];

            // Vertex Descriptor

            let vertex_input_state = &createinfo.vertex_input_state;
            if !vertex_input_state.vertex_buffer_descriptions.is_empty() {
                let vertex_descriptor: Id =
                    msg_send![class!(MTLVertexDescriptor), vertexDescriptor];
                let attributes: Id = msg_send![vertex_descriptor, attributes];
                let layouts: Id = msg_send![vertex_descriptor, layouts];

                for attribute in vertex_input_state.vertex_attributes {
                    let loc = attribute.location;
                    let a = subscript(attributes, loc);
                    let _: () =
                        msg_send![a, setFormat: sdl_to_metal_vertex_format(attribute.format)];
                    let _: () = msg_send![a, setOffset: attribute.offset as NSUInteger];
                    let _: () = msg_send![a, setBufferIndex: (METAL_FIRST_VERTEX_BUFFER_SLOT + attribute.buffer_slot) as NSUInteger];
                }

                for description in vertex_input_state.vertex_buffer_descriptions {
                    let binding = METAL_FIRST_VERTEX_BUFFER_SLOT + description.slot;
                    let layout = subscript(layouts, binding);
                    let _: () = msg_send![layout, setStepFunction: sdl_to_metal_step_function(description.input_rate)];
                    let _: () = msg_send![layout, setStepRate: 1 as NSUInteger];
                    let _: () = msg_send![layout, setStride: description.pitch as NSUInteger];
                }

                let _: () = msg_send![pipeline_descriptor, setVertexDescriptor: vertex_descriptor];
            }

            if self.debug_mode {
                if let Some(name) = createinfo
                    .props
                    .as_ref()
                    .and_then(|p| p.get_string(PROP_GPU_GRAPHICSPIPELINE_CREATE_NAME_STRING))
                {
                    set_label(pipeline_descriptor.get(), &name);
                }
            }

            // Create the graphics pipeline

            let pipeline_state = StrongPtr::from_owned(
                msg_send![self.device, newRenderPipelineStateWithDescriptor: pipeline_descriptor.get()
                                                                                                         error: &mut error as *mut Id],
            );
            (pipeline_state, depth_stencil_state)
        };
        if !error.is_null() {
            return Err(self.set_string_error(&format!(
                "Creating render pipeline failed: {}",
                // SAFETY: the NSError the call set.
                unsafe { error_description(error) }
            )));
        }

        Ok(MetalGraphicsPipeline {
            handle: pipeline_state,
            depth_stencil_state,
            rasterizer_state: createinfo.rasterizer_state,
            primitive_type: createinfo.primitive_type,
            header: GraphicsPipelineHeader {
                num_vertex_samplers: vertex_shader.num_samplers,
                num_vertex_uniform_buffers: vertex_shader.num_uniform_buffers,
                num_vertex_storage_buffers: vertex_shader.num_storage_buffers,
                num_vertex_storage_textures: vertex_shader.num_storage_textures,
                num_fragment_samplers: fragment_shader.num_samplers,
                num_fragment_uniform_buffers: fragment_shader.num_uniform_buffers,
                num_fragment_storage_buffers: fragment_shader.num_storage_buffers,
                num_fragment_storage_textures: fragment_shader.num_storage_textures,
            },
        })
    }
}
