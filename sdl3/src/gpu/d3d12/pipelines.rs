// Rust translation of the pipeline parts of src/gpu/d3d12/SDL_gpu_d3d12.c
// from Simple DirectMedia Layer: root signatures, the state conversions,
// graphics and compute pipelines.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! Pipelines and their root signatures.
//!
//! The root signature lets us define "root parameters" which are
//! essentially bind points for resources. These let us define the register
//! ranges as well as the register "space". The register space is akin to
//! the descriptor set index in Vulkan, which allows us to group resources
//! by stage so that the registers from the vertex and fragment shaders
//! don't clobber each other.
//!
//! Most of our root parameters are implemented as "descriptor tables" so
//! we can copy and then point to contiguous descriptor regions. Uniform
//! buffers are the exception - these have to be implemented as raw "root
//! descriptors" so that we can dynamically update the address that the
//! constant buffer view points to.
//!
//! The root signature has a maximum size of 64 DWORDs. A descriptor table
//! uses 1 DWORD. A root descriptor uses 2 DWORDS. This means our biggest
//! root signature uses 24 DWORDs total, well under the limit.
//!
//! The root parameter indices are created dynamically and stored in the
//! [`GraphicsRootSignature`] struct.
//!
//! Upstream fills fixed arrays of ranges and parameters, the parameters
//! pointing into the ranges as it goes; [`RootSignatureBuilder`] collects
//! the parameters first, in the same order, and points them at the ranges
//! once those are all written.

use std::sync::atomic::AtomicI32;
use std::sync::Arc;

use windows_sys::core::BOOL;

use super::d3d::*;
use super::resources::D3D12Shader;
use super::tables::{
    blend_factor, blend_factor_alpha, blend_op, compare_op, cull_mode, fill_mode, input_rate,
    primitive_topology_type, sample_count, sdl_to_d3d12_depth_format, sdl_to_d3d12_texture_format,
    stencil_op, vertex_format,
};
use super::D3D12Renderer;
use crate::error::Result;
use crate::gpu::sysgpu::{
    ComputePipelineHeader, GraphicsPipelineHeader, MAX_UNIFORM_BUFFERS_PER_STAGE,
    MAX_VERTEX_BUFFERS,
};
use crate::gpu::{
    ComputePipelineCreateInfo, DepthStencilState, FrontFace, GraphicsPipelineCreateInfo,
    PrimitiveType, RasterizerState, SampleCount, ShaderStage, VertexInputRate, VertexInputState,
};
use crate::render::direct3d11::d3d::SampleDesc;

/// Translation of `D3D12GraphicsRootSignature`.
#[derive(Debug)]
#[allow(dead_code)] // (part 2: binding reads the root indices)
pub(super) struct GraphicsRootSignature {
    pub(super) handle: D3d12RootSignature,

    pub(super) vertex_sampler_root_index: i32,
    pub(super) vertex_sampler_texture_root_index: i32,
    pub(super) vertex_storage_texture_root_index: i32,
    pub(super) vertex_storage_buffer_root_index: i32,

    pub(super) vertex_uniform_buffer_root_index: [i32; MAX_UNIFORM_BUFFERS_PER_STAGE as usize],

    pub(super) fragment_sampler_root_index: i32,
    pub(super) fragment_sampler_texture_root_index: i32,
    pub(super) fragment_storage_texture_root_index: i32,
    pub(super) fragment_storage_buffer_root_index: i32,

    pub(super) fragment_uniform_buffer_root_index: [i32; MAX_UNIFORM_BUFFERS_PER_STAGE as usize],
}

/// Translation of `D3D12GraphicsPipeline`; its destruction
/// (`D3D12_INTERNAL_DestroyGraphicsPipeline()`) releases the pipeline
/// state and the root signature as fields.
#[derive(Debug)]
pub(super) struct D3D12GraphicsPipeline {
    pub(super) header: GraphicsPipelineHeader,

    #[allow(dead_code)] // (part 2: binding)
    pub(super) pipeline_state: D3d12PipelineState,
    #[allow(dead_code)] // (part 2: binding)
    pub(super) root_signature: GraphicsRootSignature,
    #[allow(dead_code)] // (part 2: draws)
    pub(super) primitive_type: PrimitiveType,

    #[allow(dead_code)] // (part 2: vertex buffer binding)
    pub(super) vertex_strides: [u32; MAX_VERTEX_BUFFERS as usize],

    pub(super) reference_count: AtomicI32,
}

/// Translation of `D3D12ComputeRootSignature`.
#[derive(Debug)]
#[allow(dead_code)] // (part 2: binding reads the root indices)
pub(super) struct ComputeRootSignature {
    pub(super) handle: D3d12RootSignature,

    pub(super) sampler_root_index: i32,
    pub(super) sampler_texture_root_index: i32,
    pub(super) read_only_storage_texture_root_index: i32,
    pub(super) read_only_storage_buffer_root_index: i32,
    pub(super) read_write_storage_texture_root_index: i32,
    pub(super) read_write_storage_buffer_root_index: i32,
    pub(super) uniform_buffer_root_index: [i32; MAX_UNIFORM_BUFFERS_PER_STAGE as usize],
}

/// Translation of `D3D12ComputePipeline`; its destruction
/// (`D3D12_INTERNAL_DestroyComputePipeline()`) releases the pipeline state
/// and the root signature as fields.
#[derive(Debug)]
pub(super) struct D3D12ComputePipeline {
    pub(super) header: ComputePipelineHeader,

    #[allow(dead_code)] // (part 2: binding)
    pub(super) pipeline_state: D3d12PipelineState,
    #[allow(dead_code)] // (part 2: binding)
    pub(super) root_signature: ComputeRootSignature,

    pub(super) reference_count: AtomicI32,
}

// SAFETY: Direct3D 12 pipeline states and root signatures are free-threaded.
unsafe impl Send for D3D12GraphicsPipeline {}
// SAFETY: as for Send.
unsafe impl Sync for D3D12GraphicsPipeline {}
// SAFETY: as for D3D12GraphicsPipeline.
unsafe impl Send for D3D12ComputePipeline {}
// SAFETY: as for D3D12GraphicsPipeline.
unsafe impl Sync for D3D12ComputePipeline {}

/// A root parameter as the builder collects it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum RootParam {
    /// A descriptor table of one range.
    Table(DescriptorRange, u32),
    /// A root constant buffer view: register, space, visibility.
    Cbv(u32, u32, u32),
}

/// The root parameters of a root signature, in order.
#[derive(Debug, Default)]
pub(super) struct RootSignatureBuilder {
    pub(super) params: Vec<RootParam>,
}

impl RootSignatureBuilder {
    /// Add a descriptor table of one range: its root index.
    pub(super) fn table(
        &mut self,
        range_type: u32,
        num_descriptors: u32,
        base_shader_register: u32,
        register_space: u32,
        shader_visibility: u32,
    ) -> i32 {
        let descriptor_range = DescriptorRange {
            range_type,
            num_descriptors,
            base_shader_register,
            register_space,
            offset_in_descriptors_from_table_start: D3D12_DESCRIPTOR_RANGE_OFFSET_APPEND,
        };
        self.params
            .push(RootParam::Table(descriptor_range, shader_visibility));
        self.params.len() as i32 - 1
    }

    /// Add a root constant buffer view: its root index.
    pub(super) fn cbv(
        &mut self,
        shader_register: u32,
        register_space: u32,
        shader_visibility: u32,
    ) -> i32 {
        self.params.push(RootParam::Cbv(
            shader_register,
            register_space,
            shader_visibility,
        ));
        self.params.len() as i32 - 1
    }

    /// Serialize the root signature and create it on the device. The
    /// error is what upstream sets (or the `HRESULT`'s, where it sets
    /// none).
    pub(super) fn create(
        &self,
        renderer: &D3D12Renderer,
        flags: u32,
    ) -> Result<D3d12RootSignature> {
        let descriptor_ranges: Vec<DescriptorRange> = self
            .params
            .iter()
            .filter_map(|param| match param {
                RootParam::Table(range, _) => Some(*range),
                RootParam::Cbv(..) => None,
            })
            .collect();

        let mut range_count = 0;
        let root_parameters: Vec<RootParameter> = self
            .params
            .iter()
            .map(|param| match *param {
                RootParam::Table(_, shader_visibility) => {
                    let parameter = RootParameter {
                        parameter_type: D3D12_ROOT_PARAMETER_TYPE_DESCRIPTOR_TABLE,
                        u: RootParameterUnion {
                            descriptor_table: RootDescriptorTable {
                                num_descriptor_ranges: 1,
                                descriptor_ranges: &descriptor_ranges[range_count],
                            },
                        },
                        shader_visibility,
                    };
                    range_count += 1;
                    parameter
                }
                RootParam::Cbv(shader_register, register_space, shader_visibility) => {
                    RootParameter {
                        parameter_type: D3D12_ROOT_PARAMETER_TYPE_CBV,
                        u: RootParameterUnion {
                            descriptor: RootDescriptor {
                                shader_register,
                                register_space,
                            },
                        },
                        shader_visibility,
                    }
                }
            })
            .collect();

        // Create the root signature description
        let root_signature_desc = RootSignatureDesc {
            num_parameters: root_parameters.len() as u32,
            parameters: root_parameters.as_ptr(),
            num_static_samplers: 0,
            static_samplers: std::ptr::null(),
            flags,
        };

        // Serialize the root signature
        let mut serialized_root_signature: *mut std::ffi::c_void = std::ptr::null_mut();
        let mut error_blob: *mut std::ffi::c_void = std::ptr::null_mut();
        // SAFETY: the description and the ranges it points to outlive the
        // call, which stores owned blobs (or NULL) in the outputs.
        let res = unsafe {
            (renderer.serialize_root_signature)(
                &root_signature_desc,
                D3D_ROOT_SIGNATURE_VERSION_1,
                &mut serialized_root_signature,
                &mut error_blob,
            )
        };
        // SAFETY: the outputs are NULL or owned ID3DBlobs.
        let (serialized_root_signature, error_blob) = unsafe {
            (
                D3dBlob::from_raw(serialized_root_signature.cast()),
                D3dBlob::from_raw(error_blob.cast()),
            )
        };

        if res < 0 {
            if let Some(error_blob) = &error_blob {
                return Err(renderer.set_string_error(&format!(
                    "Failed to serialize RootSignature: {}",
                    blob_text(error_blob)
                )));
            }
            // Note (upstream): C fails without setting an error here.
            return Err(renderer.set_error("Failed to serialize RootSignature", res));
        }
        let Some(serialized_root_signature) = serialized_root_signature else {
            return Err(renderer.set_string_error("Failed to serialize RootSignature"));
        };

        // Create the root signature
        // Note (upstream): C never releases the serialized root signature;
        // the blob is released here.
        renderer
            .device
            .create_root_signature(serialized_root_signature.bytes())
            .map_err(|res| {
                if let Some(error_blob) = &error_blob {
                    renderer.set_string_error(&format!(
                        "Failed to create RootSignature: {}",
                        blob_text(error_blob)
                    ))
                } else {
                    // Note (upstream): C fails without setting an error here.
                    renderer.set_error("Failed to create RootSignature", res)
                }
            })
    }
}

/// The NUL-terminated text of an error blob.
fn blob_text(blob: &D3dBlob) -> String {
    let bytes = blob.bytes();
    let end = bytes.iter().position(|&b| b == 0).unwrap_or(bytes.len());
    String::from_utf8_lossy(&bytes[..end]).into_owned()
}

impl D3D12Renderer {
    /// The root signature of a graphics pipeline. Translation of
    /// `D3D12_INTERNAL_CreateGraphicsRootSignature()`.
    pub(super) fn create_graphics_root_signature(
        &self,
        vertex_shader: &D3D12Shader,
        fragment_shader: &D3D12Shader,
    ) -> Result<GraphicsRootSignature> {
        let mut builder = RootSignatureBuilder::default();

        let mut vertex_sampler_root_index = -1;
        let mut vertex_sampler_texture_root_index = -1;
        let mut vertex_storage_texture_root_index = -1;
        let mut vertex_storage_buffer_root_index = -1;
        let mut vertex_uniform_buffer_root_index = [-1; MAX_UNIFORM_BUFFERS_PER_STAGE as usize];

        let mut fragment_sampler_root_index = -1;
        let mut fragment_sampler_texture_root_index = -1;
        let mut fragment_storage_texture_root_index = -1;
        let mut fragment_storage_buffer_root_index = -1;
        let mut fragment_uniform_buffer_root_index = [-1; MAX_UNIFORM_BUFFERS_PER_STAGE as usize];

        if vertex_shader.num_samplers > 0 {
            // Vertex Samplers
            vertex_sampler_root_index = builder.table(
                D3D12_DESCRIPTOR_RANGE_TYPE_SAMPLER,
                vertex_shader.num_samplers,
                0,
                0,
                D3D12_SHADER_VISIBILITY_VERTEX,
            );

            vertex_sampler_texture_root_index = builder.table(
                D3D12_DESCRIPTOR_RANGE_TYPE_SRV,
                vertex_shader.num_samplers,
                0,
                0,
                D3D12_SHADER_VISIBILITY_VERTEX,
            );
        }

        if vertex_shader.num_storage_textures != 0 {
            // Vertex storage textures
            vertex_storage_texture_root_index = builder.table(
                D3D12_DESCRIPTOR_RANGE_TYPE_SRV,
                vertex_shader.num_storage_textures,
                vertex_shader.num_samplers,
                0,
                D3D12_SHADER_VISIBILITY_VERTEX,
            );
        }

        if vertex_shader.num_storage_buffers != 0 {
            // Vertex storage buffers
            vertex_storage_buffer_root_index = builder.table(
                D3D12_DESCRIPTOR_RANGE_TYPE_SRV,
                vertex_shader.num_storage_buffers,
                vertex_shader.num_samplers + vertex_shader.num_storage_textures,
                0,
                D3D12_SHADER_VISIBILITY_VERTEX,
            );
        }

        // Vertex Uniforms
        // Note (upstream): C writes past the root index array for more
        // uniform buffers than a stage has (the front end rejects them in
        // debug mode); they get no index here.
        for i in 0..vertex_shader.num_uniform_buffers {
            let index = builder.cbv(i, 1, D3D12_SHADER_VISIBILITY_VERTEX);
            if let Some(slot) = vertex_uniform_buffer_root_index.get_mut(i as usize) {
                *slot = index;
            }
        }

        if fragment_shader.num_samplers != 0 {
            // Fragment Samplers
            fragment_sampler_root_index = builder.table(
                D3D12_DESCRIPTOR_RANGE_TYPE_SAMPLER,
                fragment_shader.num_samplers,
                0,
                2,
                D3D12_SHADER_VISIBILITY_PIXEL,
            );

            fragment_sampler_texture_root_index = builder.table(
                D3D12_DESCRIPTOR_RANGE_TYPE_SRV,
                fragment_shader.num_samplers,
                0,
                2,
                D3D12_SHADER_VISIBILITY_PIXEL,
            );
        }

        if fragment_shader.num_storage_textures != 0 {
            // Fragment Storage Textures
            fragment_storage_texture_root_index = builder.table(
                D3D12_DESCRIPTOR_RANGE_TYPE_SRV,
                fragment_shader.num_storage_textures,
                fragment_shader.num_samplers,
                2,
                D3D12_SHADER_VISIBILITY_PIXEL,
            );
        }

        if fragment_shader.num_storage_buffers != 0 {
            // Fragment Storage Buffers
            fragment_storage_buffer_root_index = builder.table(
                D3D12_DESCRIPTOR_RANGE_TYPE_SRV,
                fragment_shader.num_storage_buffers,
                fragment_shader.num_samplers + fragment_shader.num_storage_textures,
                2,
                D3D12_SHADER_VISIBILITY_PIXEL,
            );
        }

        // Fragment Uniforms
        for i in 0..fragment_shader.num_uniform_buffers {
            let index = builder.cbv(i, 3, D3D12_SHADER_VISIBILITY_PIXEL);
            if let Some(slot) = fragment_uniform_buffer_root_index.get_mut(i as usize) {
                *slot = index;
            }
        }

        let handle = builder.create(
            self,
            D3D12_ROOT_SIGNATURE_FLAG_ALLOW_INPUT_ASSEMBLER_INPUT_LAYOUT,
        )?;

        Ok(GraphicsRootSignature {
            handle,
            vertex_sampler_root_index,
            vertex_sampler_texture_root_index,
            vertex_storage_texture_root_index,
            vertex_storage_buffer_root_index,
            vertex_uniform_buffer_root_index,
            fragment_sampler_root_index,
            fragment_sampler_texture_root_index,
            fragment_storage_texture_root_index,
            fragment_storage_buffer_root_index,
            fragment_uniform_buffer_root_index,
        })
    }

    /// The root signature of a compute pipeline. Translation of
    /// `D3D12_INTERNAL_CreateComputeRootSignature()`.
    pub(super) fn create_compute_root_signature(
        &self,
        createinfo: &ComputePipelineCreateInfo<'_>,
    ) -> Result<ComputeRootSignature> {
        let mut builder = RootSignatureBuilder::default();

        let mut sampler_root_index = -1;
        let mut sampler_texture_root_index = -1;
        let mut read_only_storage_texture_root_index = -1;
        let mut read_only_storage_buffer_root_index = -1;
        let mut read_write_storage_texture_root_index = -1;
        let mut read_write_storage_buffer_root_index = -1;
        let mut uniform_buffer_root_index = [-1; MAX_UNIFORM_BUFFERS_PER_STAGE as usize];

        // (ALL is used for compute)
        if createinfo.num_samplers != 0 {
            sampler_root_index = builder.table(
                D3D12_DESCRIPTOR_RANGE_TYPE_SAMPLER,
                createinfo.num_samplers,
                0,
                0,
                D3D12_SHADER_VISIBILITY_ALL,
            );

            sampler_texture_root_index = builder.table(
                D3D12_DESCRIPTOR_RANGE_TYPE_SRV,
                createinfo.num_samplers,
                0,
                0,
                D3D12_SHADER_VISIBILITY_ALL,
            );
        }

        if createinfo.num_readonly_storage_textures != 0 {
            read_only_storage_texture_root_index = builder.table(
                D3D12_DESCRIPTOR_RANGE_TYPE_SRV,
                createinfo.num_readonly_storage_textures,
                createinfo.num_samplers,
                0,
                D3D12_SHADER_VISIBILITY_ALL,
            );
        }

        if createinfo.num_readonly_storage_buffers != 0 {
            read_only_storage_buffer_root_index = builder.table(
                D3D12_DESCRIPTOR_RANGE_TYPE_SRV,
                createinfo.num_readonly_storage_buffers,
                createinfo.num_samplers + createinfo.num_readonly_storage_textures,
                0,
                D3D12_SHADER_VISIBILITY_ALL,
            );
        }

        if createinfo.num_readwrite_storage_textures != 0 {
            read_write_storage_texture_root_index = builder.table(
                D3D12_DESCRIPTOR_RANGE_TYPE_UAV,
                createinfo.num_readwrite_storage_textures,
                0,
                1,
                D3D12_SHADER_VISIBILITY_ALL,
            );
        }

        if createinfo.num_readwrite_storage_buffers != 0 {
            read_write_storage_buffer_root_index = builder.table(
                D3D12_DESCRIPTOR_RANGE_TYPE_UAV,
                createinfo.num_readwrite_storage_buffers,
                createinfo.num_readwrite_storage_textures,
                1,
                D3D12_SHADER_VISIBILITY_ALL,
            );
        }

        for i in 0..createinfo.num_uniform_buffers {
            let index = builder.cbv(i, 2, D3D12_SHADER_VISIBILITY_ALL);
            if let Some(slot) = uniform_buffer_root_index.get_mut(i as usize) {
                *slot = index;
            }
        }

        let handle = builder.create(self, D3D12_ROOT_SIGNATURE_FLAG_NONE)?;

        Ok(ComputeRootSignature {
            handle,
            sampler_root_index,
            sampler_texture_root_index,
            read_only_storage_texture_root_index,
            read_only_storage_buffer_root_index,
            read_write_storage_texture_root_index,
            read_write_storage_buffer_root_index,
            uniform_buffer_root_index,
        })
    }

    /// Translation of `D3D12_CreateComputePipeline()`.
    pub(super) fn create_compute_pipeline_internal(
        &self,
        createinfo: &ComputePipelineCreateInfo<'_>,
    ) -> Result<Arc<D3D12ComputePipeline>> {
        self.check_compute_bytecode(createinfo.code, createinfo.format)?;

        let root_signature = self
            .create_compute_root_signature(createinfo)
            .map_err(|_| self.set_string_error("Could not create root signature!"))?;

        let pipeline_desc = ComputePipelineStateDesc {
            cs: ShaderBytecode::new(createinfo.code),
            root_signature: root_signature.handle.as_ptr().cast(),
            cached_pso: CachedPipelineState::default(),
            flags: D3D12_PIPELINE_STATE_FLAG_NONE,
            node_mask: 0,
        };

        // Note (upstream): C leaks the root signature when this fails; it
        // is released here.
        // SAFETY: the bytecode and the root signature outlive the call.
        let pipeline_state =
            unsafe { self.device.create_compute_pipeline_state(&pipeline_desc) }
                .map_err(|res| self.set_error("Could not create compute pipeline state", res))?;

        let header = ComputePipelineHeader {
            num_samplers: createinfo.num_samplers,
            num_readonly_storage_textures: createinfo.num_readonly_storage_textures,
            num_readonly_storage_buffers: createinfo.num_readonly_storage_buffers,
            num_readwrite_storage_textures: createinfo.num_readwrite_storage_textures,
            num_readwrite_storage_buffers: createinfo.num_readwrite_storage_buffers,
            num_uniform_buffers: createinfo.num_uniform_buffers,
        };

        if self.debug_mode {
            if let Some(name) = createinfo
                .props
                .as_ref()
                .and_then(|p| p.get_string(crate::gpu::PROP_GPU_COMPUTEPIPELINE_CREATE_NAME_STRING))
            {
                self.set_pipeline_state_name(&pipeline_state, &name);
            }
        }

        Ok(Arc::new(D3D12ComputePipeline {
            header,
            pipeline_state,
            root_signature,
            reference_count: AtomicI32::new(0),
        }))
    }

    /// Translation of `D3D12_CreateGraphicsPipeline()`.
    pub(super) fn create_graphics_pipeline_internal(
        &self,
        createinfo: &GraphicsPipelineCreateInfo<'_>,
        vert_shader: &D3D12Shader,
        frag_shader: &D3D12Shader,
    ) -> Result<Arc<D3D12GraphicsPipeline>> {
        if self.debug_mode {
            if vert_shader.stage != ShaderStage::Vertex {
                crate::sdl_assert_release!(
                    !"CreateGraphicsPipeline was passed a fragment shader for the vertex stage"
                );
            }
            if frag_shader.stage != ShaderStage::Fragment {
                crate::sdl_assert_release!(
                    !"CreateGraphicsPipeline was passed a vertex shader for the fragment stage"
                );
            }
        }

        let mut pso_desc = GraphicsPipelineStateDesc {
            vs: ShaderBytecode::new(&vert_shader.bytecode),
            ps: ShaderBytecode::new(&frag_shader.bytecode),
            ..Default::default()
        };

        let input_element_descs =
            convert_vertex_input_state(&createinfo.vertex_input_state, self.semantic.as_ptr());
        if !input_element_descs.is_empty() {
            pso_desc.input_layout = InputLayoutDesc {
                input_element_descs: input_element_descs.as_ptr(),
                num_elements: input_element_descs.len() as u32,
            };
        }

        pso_desc.primitive_topology_type = primitive_topology_type(createinfo.primitive_type);

        pso_desc.rasterizer_state = convert_rasterizer_state(&createinfo.rasterizer_state);
        pso_desc.blend_state = convert_blend_state(createinfo);
        pso_desc.depth_stencil_state = convert_depth_stencil_state(&createinfo.depth_stencil_state);

        let multisample = createinfo.multisample_state.sample_count > SampleCount::One;
        pso_desc.sample_mask = 0xFFFFFFFF;
        pso_desc.sample_desc = SampleDesc {
            count: sample_count(createinfo.multisample_state.sample_count),
            quality: if multisample {
                D3D12_STANDARD_MULTISAMPLE_PATTERN
            } else {
                0
            },
        };
        if multisample {
            pso_desc.rasterizer_state.multisample_enable = 1;
        }

        if createinfo.target_info.has_depth_stencil_target {
            pso_desc.dsv_format =
                sdl_to_d3d12_depth_format(createinfo.target_info.depth_stencil_format);
        }
        let color_targets = createinfo.target_info.color_target_descriptions;
        pso_desc.num_render_targets = color_targets.len() as u32;
        // Note (upstream): C writes past the eight formats for more color
        // targets (the front end rejects them in debug mode); only eight
        // are converted here.
        for (rtv_format, target) in pso_desc.rtv_formats.iter_mut().zip(color_targets) {
            *rtv_format = sdl_to_d3d12_texture_format(target.format);
        }

        // Assuming some default values or further initialization
        pso_desc.flags = D3D12_PIPELINE_STATE_FLAG_NONE;
        pso_desc.cached_pso = CachedPipelineState::default();

        pso_desc.node_mask = 0;

        let root_signature = self.create_graphics_root_signature(vert_shader, frag_shader)?;

        pso_desc.root_signature = root_signature.handle.as_ptr().cast();

        // SAFETY: the bytecode, input layout, semantic name and root
        // signature outlive the call.
        let pipeline_state = unsafe { self.device.create_graphics_pipeline_state(&pso_desc) }
            .map_err(|res| self.set_error("Could not create graphics pipeline state", res))?;

        let mut vertex_strides = [0; MAX_VERTEX_BUFFERS as usize];
        for description in createinfo.vertex_input_state.vertex_buffer_descriptions {
            // Note (upstream): C writes past the strides for a slot out of
            // their range (the front end rejects it in debug mode).
            if let Some(stride) = vertex_strides.get_mut(description.slot as usize) {
                *stride = description.pitch;
            }
        }

        let header = GraphicsPipelineHeader {
            num_vertex_samplers: vert_shader.num_samplers,
            num_vertex_storage_textures: vert_shader.num_storage_textures,
            num_vertex_storage_buffers: vert_shader.num_storage_buffers,
            num_vertex_uniform_buffers: vert_shader.num_uniform_buffers,

            num_fragment_samplers: frag_shader.num_samplers,
            num_fragment_storage_textures: frag_shader.num_storage_textures,
            num_fragment_storage_buffers: frag_shader.num_storage_buffers,
            num_fragment_uniform_buffers: frag_shader.num_uniform_buffers,
        };

        if self.debug_mode {
            if let Some(name) = createinfo.props.as_ref().and_then(|p| {
                p.get_string(crate::gpu::PROP_GPU_GRAPHICSPIPELINE_CREATE_NAME_STRING)
            }) {
                self.set_pipeline_state_name(&pipeline_state, &name);
            }
        }

        Ok(Arc::new(D3D12GraphicsPipeline {
            header,
            pipeline_state,
            root_signature,
            primitive_type: createinfo.primitive_type,
            vertex_strides,
            reference_count: AtomicI32::new(0),
        }))
    }
}

/// Translation of `D3D12_INTERNAL_ConvertRasterizerState()`.
pub(super) fn convert_rasterizer_state(rasterizer_state: &RasterizerState) -> RasterizerDesc {
    let (depth_bias, depth_bias_clamp, slope_scaled_depth_bias) =
        if rasterizer_state.enable_depth_bias {
            (
                // (SDL_lroundf(): half away from zero, as f32::round)
                rasterizer_state.depth_bias_constant_factor.round() as i32,
                rasterizer_state.depth_bias_clamp,
                rasterizer_state.depth_bias_slope_factor,
            )
        } else {
            (0, 0.0, 0.0)
        };

    RasterizerDesc {
        fill_mode: fill_mode(rasterizer_state.fill_mode),
        cull_mode: cull_mode(rasterizer_state.cull_mode),
        front_counter_clockwise: (rasterizer_state.front_face == FrontFace::CounterClockwise)
            as BOOL,
        depth_bias,
        depth_bias_clamp,
        slope_scaled_depth_bias,
        depth_clip_enable: rasterizer_state.enable_depth_clip as BOOL,
        multisample_enable: 0,
        antialiased_line_enable: 0,
        forced_sample_count: 0,
        conservative_raster: D3D12_CONSERVATIVE_RASTERIZATION_MODE_OFF,
    }
}

/// Translation of `D3D12_INTERNAL_ConvertBlendState()`.
pub(super) fn convert_blend_state(pipeline_info: &GraphicsPipelineCreateInfo<'_>) -> BlendDesc {
    let mut blend_desc = BlendDesc {
        alpha_to_coverage_enable: pipeline_info.multisample_state.enable_alpha_to_coverage as BOOL,
        independent_blend_enable: 1,
        ..Default::default()
    };

    for (rt_blend_desc, target) in blend_desc
        .render_target
        .iter_mut()
        .zip(pipeline_info.target_info.color_target_descriptions)
    {
        let sdl_blend_state = &target.blend_state;
        let color_write_mask = if sdl_blend_state.enable_color_write_mask {
            sdl_blend_state.color_write_mask.0
        } else {
            0xF
        };

        *rt_blend_desc = RenderTargetBlendDesc {
            blend_enable: sdl_blend_state.enable_blend as BOOL,
            logic_op_enable: 0,
            src_blend: blend_factor(sdl_blend_state.src_color_blendfactor),
            dest_blend: blend_factor(sdl_blend_state.dst_color_blendfactor),
            blend_op: blend_op(sdl_blend_state.color_blend_op),
            src_blend_alpha: blend_factor_alpha(sdl_blend_state.src_alpha_blendfactor),
            dest_blend_alpha: blend_factor_alpha(sdl_blend_state.dst_alpha_blendfactor),
            blend_op_alpha: blend_op(sdl_blend_state.alpha_blend_op),
            logic_op: D3D12_LOGIC_OP_NOOP,
            render_target_write_mask: color_write_mask,
        };
    }

    blend_desc
}

/// Translation of `D3D12_INTERNAL_ConvertDepthStencilState()`.
pub(super) fn convert_depth_stencil_state(
    depth_stencil_state: &DepthStencilState,
) -> DepthStencilDesc {
    let front = &depth_stencil_state.front_stencil_state;
    let back = &depth_stencil_state.back_stencil_state;
    DepthStencilDesc {
        depth_enable: depth_stencil_state.enable_depth_test as BOOL,
        depth_write_mask: if depth_stencil_state.enable_depth_write {
            D3D12_DEPTH_WRITE_MASK_ALL
        } else {
            D3D12_DEPTH_WRITE_MASK_ZERO
        },
        depth_func: compare_op(depth_stencil_state.compare_op),
        stencil_enable: depth_stencil_state.enable_stencil_test as BOOL,
        stencil_read_mask: depth_stencil_state.compare_mask,
        stencil_write_mask: depth_stencil_state.write_mask,

        front_face: DepthStencilOpDesc {
            stencil_fail_op: stencil_op(front.fail_op),
            stencil_depth_fail_op: stencil_op(front.depth_fail_op),
            stencil_pass_op: stencil_op(front.pass_op),
            stencil_func: compare_op(front.compare_op),
        },

        back_face: DepthStencilOpDesc {
            stencil_fail_op: stencil_op(back.fail_op),
            stencil_depth_fail_op: stencil_op(back.depth_fail_op),
            stencil_pass_op: stencil_op(back.pass_op),
            stencil_func: compare_op(back.compare_op),
        },
    }
}

/// The input elements of the vertex attributes (none for none).
/// Translation of `D3D12_INTERNAL_ConvertVertexInputState()`.
///
/// FIXME (upstream): the input rate is the one of the buffer description
/// at the attribute's `buffer_slot` *position*, not of the description
/// whose `slot` that is, so descriptions out of slot order get the wrong
/// rates. Note (upstream): C reads past the descriptions (and the 32
/// elements) for values out of their range, which the front end rejects in
/// debug mode; a missing description reads as per-vertex here.
pub(super) fn convert_vertex_input_state(
    vertex_input_state: &VertexInputState<'_>,
    semantic: *const std::ffi::c_char,
) -> Vec<InputElementDesc> {
    vertex_input_state
        .vertex_attributes
        .iter()
        .take(D3D12_IA_VERTEX_INPUT_STRUCTURE_ELEMENT_COUNT)
        .map(|attribute| {
            let rate = vertex_input_state
                .vertex_buffer_descriptions
                .get(attribute.buffer_slot as usize)
                .map_or(VertexInputRate::Vertex, |d| d.input_rate);

            InputElementDesc {
                semantic_name: semantic,
                semantic_index: attribute.location,
                format: vertex_format(attribute.format),
                input_slot: attribute.buffer_slot,
                aligned_byte_offset: attribute.offset,
                input_slot_class: input_rate(rate),
                instance_data_step_rate: (rate == VertexInputRate::Instance) as u32,
            }
        })
        .collect()
}
