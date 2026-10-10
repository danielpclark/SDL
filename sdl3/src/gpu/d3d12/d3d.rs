// The Direct3D 12 declarations of src/gpu/d3d12/SDL_gpu_d3d12.c (from the
// <d3d12.h> and <d3d12sdklayers.h> of src/video/directx), for the Rust
// translation of Simple DirectMedia Layer.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! The Direct3D 12 declarations the backend uses, written by hand as the
//! Direct3D 11 renderer's are ([`crate::render::direct3d11::d3d`], whose
//! DXGI declarations this module reuses): the enumerants, the structures
//! and GUIDs, the entry points `d3d12.dll` and the PIX runtime export, and
//! the vtables of the COM interfaces SDL calls (only the slots it uses are
//! typed, the rest are padding), with thin wrappers that turn the out
//! parameters into [`ComPtr`]s and the `HRESULT`s into `Result`s.
//!
//! The structures implement `Default` as all zeroes, which is what the C
//! code's `SDL_zero()` gives. The methods that return a structure
//! (`GetCPUDescriptorHandleForHeapStart()`...) take a pointer to it after
//! `this` and return that pointer, as the vendored headers declare them
//! (`D3D_CALL_RET`): that is how MSVC returns structures from methods.

#![allow(clippy::upper_case_acronyms)]

use std::ffi::{c_char, c_void};
use std::ptr::{null, null_mut};

use windows_sys::core::{BOOL, GUID, HRESULT};
use windows_sys::Win32::Foundation::{HANDLE, HWND};

use crate::core::windows::com::{ComPtr, IUnknownVtbl};
use crate::render::direct3d11::d3d::{
    check, out, raw, D3dFeatureLevel, DxgiAdapter1, DxgiColorSpaceType, DxgiFactory1, DxgiFactory4,
    DxgiFactory6, DxgiFormat, DxgiGpuPreference, IDXGIFactory2Vtbl, SampleDesc, Slot,
    IID_IDXGIFACTORY1,
};

// --- enumerants ---

// D3D12_DESCRIPTOR_HEAP_TYPE
pub(crate) const D3D12_DESCRIPTOR_HEAP_TYPE_CBV_SRV_UAV: u32 = 0;
pub(crate) const D3D12_DESCRIPTOR_HEAP_TYPE_SAMPLER: u32 = 1;
pub(crate) const D3D12_DESCRIPTOR_HEAP_TYPE_RTV: u32 = 2;
pub(crate) const D3D12_DESCRIPTOR_HEAP_TYPE_DSV: u32 = 3;
pub(crate) const D3D12_DESCRIPTOR_HEAP_TYPE_NUM_TYPES: u32 = 4;

// D3D12_DESCRIPTOR_HEAP_FLAGS
pub(crate) const D3D12_DESCRIPTOR_HEAP_FLAG_NONE: u32 = 0;
pub(crate) const D3D12_DESCRIPTOR_HEAP_FLAG_SHADER_VISIBLE: u32 = 0x1;

// D3D12_BLEND
pub(crate) const D3D12_BLEND_ZERO: u32 = 1;
pub(crate) const D3D12_BLEND_ONE: u32 = 2;
pub(crate) const D3D12_BLEND_SRC_COLOR: u32 = 3;
pub(crate) const D3D12_BLEND_INV_SRC_COLOR: u32 = 4;
pub(crate) const D3D12_BLEND_SRC_ALPHA: u32 = 5;
pub(crate) const D3D12_BLEND_INV_SRC_ALPHA: u32 = 6;
pub(crate) const D3D12_BLEND_DEST_ALPHA: u32 = 7;
pub(crate) const D3D12_BLEND_INV_DEST_ALPHA: u32 = 8;
pub(crate) const D3D12_BLEND_DEST_COLOR: u32 = 9;
pub(crate) const D3D12_BLEND_INV_DEST_COLOR: u32 = 10;
pub(crate) const D3D12_BLEND_SRC_ALPHA_SAT: u32 = 11;
pub(crate) const D3D12_BLEND_BLEND_FACTOR: u32 = 14;
pub(crate) const D3D12_BLEND_INV_BLEND_FACTOR: u32 = 15;

// D3D12_BLEND_OP
pub(crate) const D3D12_BLEND_OP_ADD: u32 = 1;
pub(crate) const D3D12_BLEND_OP_SUBTRACT: u32 = 2;
pub(crate) const D3D12_BLEND_OP_REV_SUBTRACT: u32 = 3;
pub(crate) const D3D12_BLEND_OP_MIN: u32 = 4;
pub(crate) const D3D12_BLEND_OP_MAX: u32 = 5;

// D3D12_LOGIC_OP
pub(crate) const D3D12_LOGIC_OP_NOOP: u32 = 4;

// D3D12_COMPARISON_FUNC
pub(crate) const D3D12_COMPARISON_FUNC_NEVER: u32 = 1;
pub(crate) const D3D12_COMPARISON_FUNC_LESS: u32 = 2;
pub(crate) const D3D12_COMPARISON_FUNC_EQUAL: u32 = 3;
pub(crate) const D3D12_COMPARISON_FUNC_LESS_EQUAL: u32 = 4;
pub(crate) const D3D12_COMPARISON_FUNC_GREATER: u32 = 5;
pub(crate) const D3D12_COMPARISON_FUNC_NOT_EQUAL: u32 = 6;
pub(crate) const D3D12_COMPARISON_FUNC_GREATER_EQUAL: u32 = 7;
pub(crate) const D3D12_COMPARISON_FUNC_ALWAYS: u32 = 8;

// D3D12_STENCIL_OP
pub(crate) const D3D12_STENCIL_OP_KEEP: u32 = 1;
pub(crate) const D3D12_STENCIL_OP_ZERO: u32 = 2;
pub(crate) const D3D12_STENCIL_OP_REPLACE: u32 = 3;
pub(crate) const D3D12_STENCIL_OP_INCR_SAT: u32 = 4;
pub(crate) const D3D12_STENCIL_OP_DECR_SAT: u32 = 5;
pub(crate) const D3D12_STENCIL_OP_INVERT: u32 = 6;
pub(crate) const D3D12_STENCIL_OP_INCR: u32 = 7;
pub(crate) const D3D12_STENCIL_OP_DECR: u32 = 8;

// D3D12_CULL_MODE, D3D12_FILL_MODE
pub(crate) const D3D12_CULL_MODE_NONE: u32 = 1;
pub(crate) const D3D12_CULL_MODE_FRONT: u32 = 2;
pub(crate) const D3D12_CULL_MODE_BACK: u32 = 3;
pub(crate) const D3D12_FILL_MODE_WIREFRAME: u32 = 2;
pub(crate) const D3D12_FILL_MODE_SOLID: u32 = 3;

// D3D12_DEPTH_WRITE_MASK, D3D12_CONSERVATIVE_RASTERIZATION_MODE
pub(crate) const D3D12_DEPTH_WRITE_MASK_ZERO: u32 = 0;
pub(crate) const D3D12_DEPTH_WRITE_MASK_ALL: u32 = 1;
pub(crate) const D3D12_CONSERVATIVE_RASTERIZATION_MODE_OFF: u32 = 0;

// D3D12_INPUT_CLASSIFICATION
pub(crate) const D3D12_INPUT_CLASSIFICATION_PER_VERTEX_DATA: u32 = 0;
pub(crate) const D3D12_INPUT_CLASSIFICATION_PER_INSTANCE_DATA: u32 = 1;

// D3D_PRIMITIVE_TOPOLOGY
pub(crate) const D3D_PRIMITIVE_TOPOLOGY_POINTLIST: u32 = 1;
pub(crate) const D3D_PRIMITIVE_TOPOLOGY_LINELIST: u32 = 2;
pub(crate) const D3D_PRIMITIVE_TOPOLOGY_LINESTRIP: u32 = 3;
pub(crate) const D3D_PRIMITIVE_TOPOLOGY_TRIANGLELIST: u32 = 4;
pub(crate) const D3D_PRIMITIVE_TOPOLOGY_TRIANGLESTRIP: u32 = 5;

// D3D12_PRIMITIVE_TOPOLOGY_TYPE
pub(crate) const D3D12_PRIMITIVE_TOPOLOGY_TYPE_POINT: u32 = 1;
pub(crate) const D3D12_PRIMITIVE_TOPOLOGY_TYPE_LINE: u32 = 2;
pub(crate) const D3D12_PRIMITIVE_TOPOLOGY_TYPE_TRIANGLE: u32 = 3;
pub(crate) const D3D12_PRIMITIVE_TOPOLOGY_TYPE_PATCH: u32 = 4;

// D3D12_TEXTURE_ADDRESS_MODE
pub(crate) const D3D12_TEXTURE_ADDRESS_MODE_WRAP: u32 = 1;
pub(crate) const D3D12_TEXTURE_ADDRESS_MODE_MIRROR: u32 = 2;
pub(crate) const D3D12_TEXTURE_ADDRESS_MODE_CLAMP: u32 = 3;

// D3D12_FILTER (D3D12_ENCODE_BASIC_FILTER)
pub(crate) const D3D12_FILTER_REDUCTION_TYPE_MASK: u32 = 0x3;
pub(crate) const D3D12_FILTER_REDUCTION_TYPE_SHIFT: u32 = 7;
pub(crate) const D3D12_FILTER_TYPE_MASK: u32 = 0x3;
pub(crate) const D3D12_MIN_FILTER_SHIFT: u32 = 4;
pub(crate) const D3D12_MAG_FILTER_SHIFT: u32 = 2;
pub(crate) const D3D12_MIP_FILTER_SHIFT: u32 = 0;
pub(crate) const D3D12_ANISOTROPIC_FILTERING_BIT: u32 = 0x40;
// (the Direct3D 12 renderer's)
pub(crate) const D3D12_FILTER_MIN_MAG_MIP_POINT: u32 = 0;
pub(crate) const D3D12_FILTER_MIN_MAG_MIP_LINEAR: u32 = 0x15;
pub(crate) const D3D12_COMPARISON_FUNC_NONE: u32 = 0;
pub(crate) const D3D12_FLOAT32_MAX: f32 = f32::MAX; // (3.402823466e+38f)
pub(crate) const D3D12_COLOR_WRITE_ENABLE_ALL: u8 = 15;

// D3D12_RESOURCE_STATES
pub(crate) const D3D12_RESOURCE_STATE_COMMON: u32 = 0;
pub(crate) const D3D12_RESOURCE_STATE_VERTEX_AND_CONSTANT_BUFFER: u32 = 0x1;
pub(crate) const D3D12_RESOURCE_STATE_INDEX_BUFFER: u32 = 0x2;
pub(crate) const D3D12_RESOURCE_STATE_RENDER_TARGET: u32 = 0x4;
pub(crate) const D3D12_RESOURCE_STATE_UNORDERED_ACCESS: u32 = 0x8;
pub(crate) const D3D12_RESOURCE_STATE_DEPTH_WRITE: u32 = 0x10;
pub(crate) const D3D12_RESOURCE_STATE_NON_PIXEL_SHADER_RESOURCE: u32 = 0x40;
pub(crate) const D3D12_RESOURCE_STATE_PIXEL_SHADER_RESOURCE: u32 = 0x80;
pub(crate) const D3D12_RESOURCE_STATE_INDIRECT_ARGUMENT: u32 = 0x200;
pub(crate) const D3D12_RESOURCE_STATE_COPY_DEST: u32 = 0x400;
pub(crate) const D3D12_RESOURCE_STATE_COPY_SOURCE: u32 = 0x800;
pub(crate) const D3D12_RESOURCE_STATE_RESOLVE_DEST: u32 = 0x1000;
pub(crate) const D3D12_RESOURCE_STATE_RESOLVE_SOURCE: u32 = 0x2000;
pub(crate) const D3D12_RESOURCE_STATE_GENERIC_READ: u32 = 0xac3;
pub(crate) const D3D12_RESOURCE_STATE_ALL_SHADER_RESOURCE: u32 = 0xc0;
pub(crate) const D3D12_RESOURCE_STATE_PRESENT: u32 = 0;

// D3D12_HEAP_TYPE, D3D12_CPU_PAGE_PROPERTY, D3D12_MEMORY_POOL
pub(crate) const D3D12_HEAP_TYPE_DEFAULT: u32 = 1;
pub(crate) const D3D12_HEAP_TYPE_UPLOAD: u32 = 2;
pub(crate) const D3D12_HEAP_TYPE_READBACK: u32 = 3;
pub(crate) const D3D12_HEAP_TYPE_GPU_UPLOAD: u32 = 5;
pub(crate) const D3D12_CPU_PAGE_PROPERTY_UNKNOWN: u32 = 0;
pub(crate) const D3D12_MEMORY_POOL_UNKNOWN: u32 = 0;

// D3D12_HEAP_FLAGS
pub(crate) const D3D12_HEAP_FLAG_NONE: u32 = 0;
pub(crate) const D3D12_HEAP_FLAG_ALLOW_DISPLAY: u32 = 0x8;

// D3D12_RESOURCE_DIMENSION, D3D12_TEXTURE_LAYOUT
pub(crate) const D3D12_RESOURCE_DIMENSION_BUFFER: u32 = 1;
pub(crate) const D3D12_RESOURCE_DIMENSION_TEXTURE2D: u32 = 3;
pub(crate) const D3D12_RESOURCE_DIMENSION_TEXTURE3D: u32 = 4;
pub(crate) const D3D12_TEXTURE_LAYOUT_UNKNOWN: u32 = 0;
pub(crate) const D3D12_TEXTURE_LAYOUT_ROW_MAJOR: u32 = 1;

// D3D12_RESOURCE_FLAGS
pub(crate) const D3D12_RESOURCE_FLAG_NONE: u32 = 0;
pub(crate) const D3D12_RESOURCE_FLAG_ALLOW_RENDER_TARGET: u32 = 0x1;
pub(crate) const D3D12_RESOURCE_FLAG_ALLOW_DEPTH_STENCIL: u32 = 0x2;
pub(crate) const D3D12_RESOURCE_FLAG_ALLOW_UNORDERED_ACCESS: u32 = 0x4;

pub(crate) const D3D12_DEFAULT_RESOURCE_PLACEMENT_ALIGNMENT: u64 = 65536;
pub(crate) const D3D12_DEFAULT_MSAA_RESOURCE_PLACEMENT_ALIGNMENT: u64 = 4194304;
pub(crate) const D3D12_STANDARD_MULTISAMPLE_PATTERN: u32 = 0xffffffff;
pub(crate) const D3D12_DEFAULT_SHADER_4_COMPONENT_MAPPING: u32 = 0x1688;
pub(crate) const D3D12_IA_VERTEX_INPUT_STRUCTURE_ELEMENT_COUNT: usize = 32;

// D3D12_SRV_DIMENSION
pub(crate) const D3D12_SRV_DIMENSION_BUFFER: u32 = 1;
pub(crate) const D3D12_SRV_DIMENSION_TEXTURE2D: u32 = 4;
pub(crate) const D3D12_SRV_DIMENSION_TEXTURE2DARRAY: u32 = 5;
pub(crate) const D3D12_SRV_DIMENSION_TEXTURE2DMS: u32 = 6;
pub(crate) const D3D12_SRV_DIMENSION_TEXTURE3D: u32 = 8;
pub(crate) const D3D12_SRV_DIMENSION_TEXTURECUBE: u32 = 9;
pub(crate) const D3D12_SRV_DIMENSION_TEXTURECUBEARRAY: u32 = 10;

// D3D12_RTV_DIMENSION, D3D12_DSV_DIMENSION, D3D12_UAV_DIMENSION
pub(crate) const D3D12_RTV_DIMENSION_TEXTURE2D: u32 = 4;
pub(crate) const D3D12_RTV_DIMENSION_TEXTURE2DARRAY: u32 = 5;
pub(crate) const D3D12_RTV_DIMENSION_TEXTURE2DMS: u32 = 6;
pub(crate) const D3D12_RTV_DIMENSION_TEXTURE3D: u32 = 8;
pub(crate) const D3D12_DSV_DIMENSION_TEXTURE2D: u32 = 3;
pub(crate) const D3D12_DSV_DIMENSION_TEXTURE2DARRAY: u32 = 4;
pub(crate) const D3D12_DSV_DIMENSION_TEXTURE2DMS: u32 = 5;
pub(crate) const D3D12_UAV_DIMENSION_BUFFER: u32 = 1;
pub(crate) const D3D12_UAV_DIMENSION_TEXTURE2D: u32 = 4;
pub(crate) const D3D12_UAV_DIMENSION_TEXTURE2DARRAY: u32 = 5;
pub(crate) const D3D12_UAV_DIMENSION_TEXTURE3D: u32 = 8;

// D3D12_BUFFER_UAV_FLAGS, D3D12_BUFFER_SRV_FLAGS
pub(crate) const D3D12_BUFFER_UAV_FLAG_RAW: u32 = 0x1;
pub(crate) const D3D12_BUFFER_SRV_FLAG_RAW: u32 = 0x1;

// D3D12_DESCRIPTOR_RANGE_TYPE
pub(crate) const D3D12_DESCRIPTOR_RANGE_TYPE_SRV: u32 = 0;
pub(crate) const D3D12_DESCRIPTOR_RANGE_TYPE_UAV: u32 = 1;
pub(crate) const D3D12_DESCRIPTOR_RANGE_TYPE_SAMPLER: u32 = 3;
pub(crate) const D3D12_DESCRIPTOR_RANGE_OFFSET_APPEND: u32 = 0xffffffff;

// D3D12_ROOT_PARAMETER_TYPE, D3D12_SHADER_VISIBILITY
pub(crate) const D3D12_ROOT_PARAMETER_TYPE_DESCRIPTOR_TABLE: u32 = 0;
pub(crate) const D3D12_ROOT_PARAMETER_TYPE_CBV: u32 = 2;
pub(crate) const D3D12_SHADER_VISIBILITY_ALL: u32 = 0;
pub(crate) const D3D12_SHADER_VISIBILITY_VERTEX: u32 = 1;
pub(crate) const D3D12_SHADER_VISIBILITY_PIXEL: u32 = 5;

// D3D12_ROOT_SIGNATURE_FLAGS, D3D_ROOT_SIGNATURE_VERSION
pub(crate) const D3D12_ROOT_SIGNATURE_FLAG_NONE: u32 = 0;
pub(crate) const D3D12_ROOT_SIGNATURE_FLAG_ALLOW_INPUT_ASSEMBLER_INPUT_LAYOUT: u32 = 0x1;
pub(crate) const D3D_ROOT_SIGNATURE_VERSION_1: u32 = 0x1;

// D3D12_PIPELINE_STATE_FLAGS
pub(crate) const D3D12_PIPELINE_STATE_FLAG_NONE: u32 = 0;

// D3D12_FEATURE
pub(crate) const D3D12_FEATURE_D3D12_OPTIONS: u32 = 0;
pub(crate) const D3D12_FEATURE_FORMAT_SUPPORT: u32 = 3;
pub(crate) const D3D12_FEATURE_MULTISAMPLE_QUALITY_LEVELS: u32 = 4;
pub(crate) const D3D12_FEATURE_SHADER_MODEL: u32 = 7;
pub(crate) const D3D12_FEATURE_D3D12_OPTIONS13: u32 = 42;
pub(crate) const D3D12_FEATURE_D3D12_OPTIONS16: u32 = 45;

// D3D12_FORMAT_SUPPORT1, D3D12_FORMAT_SUPPORT2
pub(crate) const D3D12_FORMAT_SUPPORT1_NONE: u32 = 0;
pub(crate) const D3D12_FORMAT_SUPPORT1_TEXTURE2D: u32 = 0x20;
pub(crate) const D3D12_FORMAT_SUPPORT1_TEXTURE3D: u32 = 0x40;
pub(crate) const D3D12_FORMAT_SUPPORT1_TEXTURECUBE: u32 = 0x80;
pub(crate) const D3D12_FORMAT_SUPPORT1_SHADER_LOAD: u32 = 0x100;
pub(crate) const D3D12_FORMAT_SUPPORT1_SHADER_SAMPLE: u32 = 0x200;
pub(crate) const D3D12_FORMAT_SUPPORT1_RENDER_TARGET: u32 = 0x4000;
pub(crate) const D3D12_FORMAT_SUPPORT1_DEPTH_STENCIL: u32 = 0x10000;
pub(crate) const D3D12_FORMAT_SUPPORT2_NONE: u32 = 0;
pub(crate) const D3D12_FORMAT_SUPPORT2_UAV_TYPED_LOAD: u32 = 0x40;
pub(crate) const D3D12_FORMAT_SUPPORT2_UAV_TYPED_STORE: u32 = 0x80;

// D3D_SHADER_MODEL, D3D12_RESOURCE_BINDING_TIER
pub(crate) const D3D_SHADER_MODEL_6_0: u32 = 0x60;
pub(crate) const D3D12_RESOURCE_BINDING_TIER_2: u32 = 2;

// D3D12_COMMAND_LIST_TYPE, D3D12_COMMAND_QUEUE_FLAGS
pub(crate) const D3D12_COMMAND_LIST_TYPE_DIRECT: u32 = 0;
pub(crate) const D3D12_COMMAND_QUEUE_FLAG_NONE: u32 = 0;

// D3D12_INDIRECT_ARGUMENT_TYPE
pub(crate) const D3D12_INDIRECT_ARGUMENT_TYPE_DRAW: u32 = 0;
pub(crate) const D3D12_INDIRECT_ARGUMENT_TYPE_DRAW_INDEXED: u32 = 1;
pub(crate) const D3D12_INDIRECT_ARGUMENT_TYPE_DISPATCH: u32 = 2;

// D3D12_MESSAGE_CATEGORY
pub(crate) const D3D12_MESSAGE_CATEGORY_APPLICATION_DEFINED: u32 = 0;
pub(crate) const D3D12_MESSAGE_CATEGORY_MISCELLANEOUS: u32 = 1;
pub(crate) const D3D12_MESSAGE_CATEGORY_INITIALIZATION: u32 = 2;
pub(crate) const D3D12_MESSAGE_CATEGORY_CLEANUP: u32 = 3;
pub(crate) const D3D12_MESSAGE_CATEGORY_COMPILATION: u32 = 4;
pub(crate) const D3D12_MESSAGE_CATEGORY_STATE_CREATION: u32 = 5;
pub(crate) const D3D12_MESSAGE_CATEGORY_STATE_SETTING: u32 = 6;
pub(crate) const D3D12_MESSAGE_CATEGORY_STATE_GETTING: u32 = 7;
pub(crate) const D3D12_MESSAGE_CATEGORY_RESOURCE_MANIPULATION: u32 = 8;
pub(crate) const D3D12_MESSAGE_CATEGORY_EXECUTION: u32 = 9;
pub(crate) const D3D12_MESSAGE_CATEGORY_SHADER: u32 = 10;

// D3D12_MESSAGE_SEVERITY
pub(crate) const D3D12_MESSAGE_SEVERITY_CORRUPTION: u32 = 0;
pub(crate) const D3D12_MESSAGE_SEVERITY_ERROR: u32 = 1;
pub(crate) const D3D12_MESSAGE_SEVERITY_WARNING: u32 = 2;
pub(crate) const D3D12_MESSAGE_SEVERITY_INFO: u32 = 3;
pub(crate) const D3D12_MESSAGE_SEVERITY_MESSAGE: u32 = 4;

// D3D12_MESSAGE_CALLBACK_FLAGS, D3D12_DEVICE_FACTORY_FLAGS
pub(crate) const D3D12_MESSAGE_CALLBACK_FLAG_NONE: u32 = 0;
pub(crate) const D3D12_DEVICE_FACTORY_FLAG_ALLOW_RETURNING_EXISTING_DEVICE: u32 = 0x1;

// D3D12_RESOURCE_BARRIER_TYPE, D3D12_RESOURCE_BARRIER_FLAGS
pub(crate) const D3D12_RESOURCE_BARRIER_TYPE_TRANSITION: u32 = 0;
pub(crate) const D3D12_RESOURCE_BARRIER_TYPE_UAV: u32 = 2;
pub(crate) const D3D12_RESOURCE_BARRIER_FLAG_NONE: u32 = 0;
pub(crate) const D3D12_RESOURCE_BARRIER_ALL_SUBRESOURCES: u32 = 0xffffffff;

// D3D12_TEXTURE_COPY_TYPE, D3D12_CLEAR_FLAGS, D3D12_FENCE_FLAGS
pub(crate) const D3D12_TEXTURE_COPY_TYPE_SUBRESOURCE_INDEX: u32 = 0;
pub(crate) const D3D12_TEXTURE_COPY_TYPE_PLACED_FOOTPRINT: u32 = 1;
pub(crate) const D3D12_CLEAR_FLAG_DEPTH: u32 = 0x1;
pub(crate) const D3D12_CLEAR_FLAG_STENCIL: u32 = 0x2;
pub(crate) const D3D12_FENCE_FLAG_NONE: u32 = 0;

// The copy alignments
pub(crate) const D3D12_TEXTURE_DATA_PITCH_ALIGNMENT: u32 = 256;
pub(crate) const D3D12_TEXTURE_DATA_PLACEMENT_ALIGNMENT: u32 = 512;

// D3D12_FORMAT_SUPPORT1 (for swapchains)
pub(crate) const D3D12_FORMAT_SUPPORT1_DISPLAY: u32 = 0x80000;

// DXGI (the swapchains')
pub(crate) const DXGI_SWAP_EFFECT_FLIP_DISCARD: u32 = 4;
pub(crate) const DXGI_SCALING_NONE: u32 = 1;
pub(crate) const DXGI_ALPHA_MODE_UNSPECIFIED: u32 = 0;
pub(crate) const DXGI_USAGE_RENDER_TARGET_OUTPUT: u32 = 0x20;
pub(crate) const DXGI_SWAP_CHAIN_FLAG_ALLOW_TEARING: u32 = 2048;
pub(crate) const DXGI_SWAP_CHAIN_FLAG_FRAME_LATENCY_WAITABLE_OBJECT: u32 = 64;
pub(crate) const DXGI_PRESENT_ALLOW_TEARING: u32 = 0x200;
pub(crate) const DXGI_MWA_NO_WINDOW_CHANGES: u32 = 1;
pub(crate) const DXGI_SWAP_CHAIN_COLOR_SPACE_SUPPORT_FLAG_PRESENT: u32 = 1;
pub(crate) const DXGI_MODE_SCANLINE_ORDER_UNSPECIFIED: u32 = 0;
pub(crate) const DXGI_MODE_SCALING_UNSPECIFIED: u32 = 0;

// --- GUIDs (upstream defines them so as not to need uuid.lib) ---

pub(crate) const IID_ID3D12DEVICE: GUID = GUID::from_u128(0x189819f1_1db6_4b57_be54_1821339b85f7);
pub(crate) const IID_ID3D12COMMANDQUEUE: GUID =
    GUID::from_u128(0x0ec870a6_5d7e_4c22_8cfc_5baae07616ed);
pub(crate) const IID_ID3D12DESCRIPTORHEAP: GUID =
    GUID::from_u128(0x8efb471d_616c_4f49_90f7_127bb763fa51);
pub(crate) const IID_ID3D12RESOURCE: GUID = GUID::from_u128(0x696442be_a72e_4059_bc79_5b5c98040fad);
pub(crate) const IID_ID3D12ROOTSIGNATURE: GUID =
    GUID::from_u128(0xc54a6b66_72df_4ee8_8be5_a946a1429214);
pub(crate) const IID_ID3D12COMMANDSIGNATURE: GUID =
    GUID::from_u128(0xc36a797c_ec80_4f0a_8985_a7b2475082d1);
pub(crate) const IID_ID3D12PIPELINESTATE: GUID =
    GUID::from_u128(0x765a30f3_f624_4c6f_a828_ace948622445);
pub(crate) const IID_ID3D12DEBUG: GUID = GUID::from_u128(0x344488b7_6846_474b_b989_f027448245e0);
pub(crate) const IID_ID3D12INFOQUEUE: GUID =
    GUID::from_u128(0x0742a90b_c387_483f_b946_30a7e4e61458);
pub(crate) const IID_ID3D12INFOQUEUE1: GUID =
    GUID::from_u128(0x2852dd88_b484_4c0c_b6b1_67168500e600);

pub(crate) const CLSID_ID3D12SDKCONFIGURATION: GUID =
    GUID::from_u128(0x7cda6aca_a03e_49c8_9458_0334d20e07ce);
pub(crate) const CLSID_ID3D12DEBUG: GUID = GUID::from_u128(0xf2352aeb_dd84_49fe_b97b_a9dcfdcc1b4f);
pub(crate) const IID_ID3D12SDKCONFIGURATION: GUID =
    GUID::from_u128(0xe9eb5314_33aa_42b2_a718_d77f58b1f1c7);
pub(crate) const IID_ID3D12SDKCONFIGURATION1: GUID =
    GUID::from_u128(0x8aaf9303_ad25_48b9_9a57_d9c37e009d9f);
pub(crate) const IID_ID3D12DEVICEFACTORY: GUID =
    GUID::from_u128(0x61f307d3_d34e_4e7c_8374_3ba4de23cccb);
pub(crate) const IID_ID3D12COMMANDALLOCATOR: GUID =
    GUID::from_u128(0x6102dee4_af59_4b09_b999_b44d73f09b24);
pub(crate) const IID_ID3D12COMMANDLIST: GUID =
    GUID::from_u128(0x7116d91c_e7e4_47ce_b8c6_ec8168f437e5);
pub(crate) const IID_ID3D12GRAPHICSCOMMANDLIST: GUID =
    GUID::from_u128(0x5b160d0f_ac1b_4185_8ba8_b3ae42a5a455);
pub(crate) const IID_ID3D12FENCE: GUID = GUID::from_u128(0x0a753dcf_c4d8_4b91_adf6_be5a60d95a76);
pub(crate) const IID_IDXGISWAPCHAIN3: GUID =
    GUID::from_u128(0x94d99bdb_f1f8_4ab0_b236_7da0170edab1);
// (the Direct3D 12 renderer's)
pub(crate) const IID_ID3D12DEVICE1: GUID = GUID::from_u128(0x77acce80_638e_4e65_8895_c1f23386863e);
pub(crate) const IID_ID3D12GRAPHICSCOMMANDLIST2: GUID =
    GUID::from_u128(0x38c3e585_ff17_412c_9150_4fc6f9d72a28);
pub(crate) const IID_IDXGISWAPCHAIN4: GUID =
    GUID::from_u128(0x3d585d5a_bd4a_489e_b1f4_3dbcb6452ffb);
pub(crate) const IID_IDXGIADAPTER4: GUID = GUID::from_u128(0x3c8d99d1_4fbf_4181_a82c_af66bf7bd24e);

// --- structures ---

/// `Default` as all zeroes (`SDL_zero()`), for structures with pointers or
/// unions.
macro_rules! zeroed_default {
    ($($ty:ty),* $(,)?) => {
        $(
            impl Default for $ty {
                fn default() -> $ty {
                    // SAFETY: the structure is plain data (integers,
                    // floats, raw pointers and unions of them), for which
                    // all zeroes is a valid value.
                    unsafe { std::mem::zeroed() }
                }
            }
        )*
    };
}

/// `D3D12_CPU_DESCRIPTOR_HANDLE`
#[repr(C)]
#[derive(Clone, Copy, Default, Debug, PartialEq, Eq)]
pub(crate) struct CpuDescriptorHandle {
    pub(crate) ptr: usize,
}

/// `D3D12_GPU_DESCRIPTOR_HANDLE`
#[repr(C)]
#[derive(Clone, Copy, Default, Debug, PartialEq, Eq)]
pub(crate) struct GpuDescriptorHandle {
    pub(crate) ptr: u64,
}

/// `D3D12_DESCRIPTOR_HEAP_DESC`
#[repr(C)]
#[derive(Clone, Copy, Default, Debug)]
pub(crate) struct DescriptorHeapDesc {
    pub(crate) ty: u32,
    pub(crate) num_descriptors: u32,
    pub(crate) flags: u32,
    pub(crate) node_mask: u32,
}

/// `D3D12_COMMAND_QUEUE_DESC`
#[repr(C)]
#[derive(Clone, Copy, Default, Debug)]
pub(crate) struct CommandQueueDesc {
    pub(crate) ty: u32,
    pub(crate) priority: i32,
    pub(crate) flags: u32,
    pub(crate) node_mask: u32,
}

/// `D3D12_HEAP_PROPERTIES`
#[repr(C)]
#[derive(Clone, Copy, Default, Debug)]
pub(crate) struct HeapProperties {
    pub(crate) ty: u32,
    pub(crate) cpu_page_property: u32,
    pub(crate) memory_pool_preference: u32,
    pub(crate) creation_node_mask: u32,
    pub(crate) visible_node_mask: u32,
}

/// `D3D12_RESOURCE_DESC`
#[repr(C)]
#[derive(Clone, Copy, Default, Debug)]
pub(crate) struct ResourceDesc {
    pub(crate) dimension: u32,
    pub(crate) alignment: u64,
    pub(crate) width: u64,
    pub(crate) height: u32,
    pub(crate) depth_or_array_size: u16,
    pub(crate) mip_levels: u16,
    pub(crate) format: DxgiFormat,
    pub(crate) sample_desc: SampleDesc,
    pub(crate) layout: u32,
    pub(crate) flags: u32,
}

/// `D3D12_DEPTH_STENCIL_VALUE`
#[repr(C)]
#[derive(Clone, Copy, Default, Debug)]
pub(crate) struct DepthStencilValue {
    pub(crate) depth: f32,
    pub(crate) stencil: u8,
}

/// The union of `D3D12_CLEAR_VALUE`.
#[repr(C)]
#[derive(Clone, Copy)]
pub(crate) union ClearValueUnion {
    pub(crate) color: [f32; 4],
    pub(crate) depth_stencil: DepthStencilValue,
}

/// `D3D12_CLEAR_VALUE`
#[repr(C)]
#[derive(Clone, Copy)]
pub(crate) struct ClearValue {
    pub(crate) format: DxgiFormat,
    pub(crate) u: ClearValueUnion,
}

/// `D3D12_BUFFER_SRV`
#[repr(C)]
#[derive(Clone, Copy, Default, Debug)]
pub(crate) struct BufferSrv {
    pub(crate) first_element: u64,
    pub(crate) num_elements: u32,
    pub(crate) structure_byte_stride: u32,
    pub(crate) flags: u32,
}

/// `D3D12_TEX2D_SRV`
#[repr(C)]
#[derive(Clone, Copy, Default, Debug)]
pub(crate) struct Tex2dSrv {
    pub(crate) most_detailed_mip: u32,
    pub(crate) mip_levels: u32,
    pub(crate) plane_slice: u32,
    pub(crate) resource_min_lod_clamp: f32,
}

/// `D3D12_TEX2D_ARRAY_SRV`
#[repr(C)]
#[derive(Clone, Copy, Default, Debug)]
pub(crate) struct Tex2dArraySrv {
    pub(crate) most_detailed_mip: u32,
    pub(crate) mip_levels: u32,
    pub(crate) first_array_slice: u32,
    pub(crate) array_size: u32,
    pub(crate) plane_slice: u32,
    pub(crate) resource_min_lod_clamp: f32,
}

/// `D3D12_TEX2DMS_SRV`
#[repr(C)]
#[derive(Clone, Copy, Default, Debug)]
pub(crate) struct Tex2dmsSrv {
    pub(crate) unused_field_nothing_to_define: u32,
}

/// `D3D12_TEX3D_SRV`, `D3D12_TEXCUBE_SRV` (the same members)
#[repr(C)]
#[derive(Clone, Copy, Default, Debug)]
pub(crate) struct Tex3dSrv {
    pub(crate) most_detailed_mip: u32,
    pub(crate) mip_levels: u32,
    pub(crate) resource_min_lod_clamp: f32,
}

/// `D3D12_TEXCUBE_ARRAY_SRV`
#[repr(C)]
#[derive(Clone, Copy, Default, Debug)]
pub(crate) struct TexCubeArraySrv {
    pub(crate) most_detailed_mip: u32,
    pub(crate) mip_levels: u32,
    pub(crate) first_2d_array_face: u32,
    pub(crate) num_cubes: u32,
    pub(crate) resource_min_lod_clamp: f32,
}

/// The union of `D3D12_SHADER_RESOURCE_VIEW_DESC` (the members SDL uses).
#[repr(C)]
#[derive(Clone, Copy)]
pub(crate) union SrvUnion {
    pub(crate) buffer: BufferSrv,
    pub(crate) texture_2d: Tex2dSrv,
    pub(crate) texture_2d_array: Tex2dArraySrv,
    pub(crate) texture_2dms: Tex2dmsSrv,
    pub(crate) texture_3d: Tex3dSrv,
    pub(crate) texture_cube: Tex3dSrv,
    pub(crate) texture_cube_array: TexCubeArraySrv,
}

/// `D3D12_SHADER_RESOURCE_VIEW_DESC`
#[repr(C)]
#[derive(Clone, Copy)]
pub(crate) struct ShaderResourceViewDesc {
    pub(crate) format: DxgiFormat,
    pub(crate) view_dimension: u32,
    pub(crate) shader_4_component_mapping: u32,
    pub(crate) u: SrvUnion,
}

/// `D3D12_TEX2D_RTV`, `D3D12_TEX2D_UAV` (the same members)
#[repr(C)]
#[derive(Clone, Copy, Default, Debug)]
pub(crate) struct Tex2dRtv {
    pub(crate) mip_slice: u32,
    pub(crate) plane_slice: u32,
}

/// `D3D12_TEX2D_ARRAY_RTV`, `D3D12_TEX2D_ARRAY_UAV` (the same members)
#[repr(C)]
#[derive(Clone, Copy, Default, Debug)]
pub(crate) struct Tex2dArrayRtv {
    pub(crate) mip_slice: u32,
    pub(crate) first_array_slice: u32,
    pub(crate) array_size: u32,
    pub(crate) plane_slice: u32,
}

/// `D3D12_TEX3D_RTV`, `D3D12_TEX3D_UAV` (the same members)
#[repr(C)]
#[derive(Clone, Copy, Default, Debug)]
pub(crate) struct Tex3dRtv {
    pub(crate) mip_slice: u32,
    pub(crate) first_w_slice: u32,
    pub(crate) w_size: u32,
}

/// `D3D12_BUFFER_RTV` (the largest member of the union)
#[repr(C)]
#[derive(Clone, Copy, Default, Debug)]
pub(crate) struct BufferRtv {
    pub(crate) first_element: u64,
    pub(crate) num_elements: u32,
}

/// The union of `D3D12_RENDER_TARGET_VIEW_DESC` (the members SDL uses).
#[repr(C)]
#[derive(Clone, Copy)]
pub(crate) union RtvUnion {
    pub(crate) buffer: BufferRtv,
    pub(crate) texture_2d: Tex2dRtv,
    pub(crate) texture_2d_array: Tex2dArrayRtv,
    pub(crate) texture_3d: Tex3dRtv,
}

/// `D3D12_RENDER_TARGET_VIEW_DESC`
#[repr(C)]
#[derive(Clone, Copy)]
pub(crate) struct RenderTargetViewDesc {
    pub(crate) format: DxgiFormat,
    pub(crate) view_dimension: u32,
    pub(crate) u: RtvUnion,
}

/// `D3D12_TEX2D_DSV`
#[repr(C)]
#[derive(Clone, Copy, Default, Debug)]
pub(crate) struct Tex2dDsv {
    pub(crate) mip_slice: u32,
}

/// `D3D12_TEX2D_ARRAY_DSV`
#[repr(C)]
#[derive(Clone, Copy, Default, Debug)]
pub(crate) struct Tex2dArrayDsv {
    pub(crate) mip_slice: u32,
    pub(crate) first_array_slice: u32,
    pub(crate) array_size: u32,
}

/// The union of `D3D12_DEPTH_STENCIL_VIEW_DESC` (the members SDL uses;
/// the others are no larger).
#[repr(C)]
#[derive(Clone, Copy)]
pub(crate) union DsvUnion {
    pub(crate) texture_2d: Tex2dDsv,
    pub(crate) texture_2d_array: Tex2dArrayDsv,
}

/// `D3D12_DEPTH_STENCIL_VIEW_DESC`
#[repr(C)]
#[derive(Clone, Copy)]
pub(crate) struct DepthStencilViewDesc {
    pub(crate) format: DxgiFormat,
    pub(crate) view_dimension: u32,
    pub(crate) flags: u32,
    pub(crate) u: DsvUnion,
}

/// `D3D12_BUFFER_UAV`
#[repr(C)]
#[derive(Clone, Copy, Default, Debug)]
pub(crate) struct BufferUav {
    pub(crate) first_element: u64,
    pub(crate) num_elements: u32,
    pub(crate) structure_byte_stride: u32,
    pub(crate) counter_offset_in_bytes: u64,
    pub(crate) flags: u32,
}

/// The union of `D3D12_UNORDERED_ACCESS_VIEW_DESC` (the members SDL uses).
#[repr(C)]
#[derive(Clone, Copy)]
pub(crate) union UavUnion {
    pub(crate) buffer: BufferUav,
    pub(crate) texture_2d: Tex2dRtv,
    pub(crate) texture_2d_array: Tex2dArrayRtv,
    pub(crate) texture_3d: Tex3dRtv,
}

/// `D3D12_UNORDERED_ACCESS_VIEW_DESC`
#[repr(C)]
#[derive(Clone, Copy)]
pub(crate) struct UnorderedAccessViewDesc {
    pub(crate) format: DxgiFormat,
    pub(crate) view_dimension: u32,
    pub(crate) u: UavUnion,
}

/// `D3D12_SAMPLER_DESC`
#[repr(C)]
#[derive(Clone, Copy, Default, Debug, PartialEq)]
pub(crate) struct SamplerDesc {
    pub(crate) filter: u32,
    pub(crate) address_u: u32,
    pub(crate) address_v: u32,
    pub(crate) address_w: u32,
    pub(crate) mip_lod_bias: f32,
    pub(crate) max_anisotropy: u32,
    pub(crate) comparison_func: u32,
    pub(crate) border_color: [f32; 4],
    pub(crate) min_lod: f32,
    pub(crate) max_lod: f32,
}

/// `D3D12_DESCRIPTOR_RANGE`
#[repr(C)]
#[derive(Clone, Copy, Default, Debug, PartialEq, Eq)]
pub(crate) struct DescriptorRange {
    pub(crate) range_type: u32,
    pub(crate) num_descriptors: u32,
    pub(crate) base_shader_register: u32,
    pub(crate) register_space: u32,
    pub(crate) offset_in_descriptors_from_table_start: u32,
}

/// `D3D12_ROOT_DESCRIPTOR_TABLE`
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub(crate) struct RootDescriptorTable {
    pub(crate) num_descriptor_ranges: u32,
    pub(crate) descriptor_ranges: *const DescriptorRange,
}

/// `D3D12_ROOT_DESCRIPTOR`
#[repr(C)]
#[derive(Clone, Copy, Default, Debug, PartialEq, Eq)]
pub(crate) struct RootDescriptor {
    pub(crate) shader_register: u32,
    pub(crate) register_space: u32,
}

/// The union of `D3D12_ROOT_PARAMETER` (the members SDL uses; the
/// `Constants` member is no larger).
#[repr(C)]
#[derive(Clone, Copy)]
pub(crate) union RootParameterUnion {
    pub(crate) descriptor_table: RootDescriptorTable,
    pub(crate) descriptor: RootDescriptor,
}

/// `D3D12_ROOT_PARAMETER`
#[repr(C)]
#[derive(Clone, Copy)]
pub(crate) struct RootParameter {
    pub(crate) parameter_type: u32,
    pub(crate) u: RootParameterUnion,
    pub(crate) shader_visibility: u32,
}

/// `D3D12_ROOT_SIGNATURE_DESC` (without static samplers: SDL has none)
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub(crate) struct RootSignatureDesc {
    pub(crate) num_parameters: u32,
    pub(crate) parameters: *const RootParameter,
    pub(crate) num_static_samplers: u32,
    pub(crate) static_samplers: *const c_void,
    pub(crate) flags: u32,
}

/// `D3D12_SHADER_BYTECODE`
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub(crate) struct ShaderBytecode {
    pub(crate) shader_bytecode: *const c_void,
    pub(crate) bytecode_length: usize,
}

impl ShaderBytecode {
    /// The bytecode of a shader, borrowed for the call it is passed to.
    pub(crate) fn new(code: &[u8]) -> ShaderBytecode {
        ShaderBytecode {
            shader_bytecode: code.as_ptr().cast(),
            bytecode_length: code.len(),
        }
    }
}

/// `D3D12_STREAM_OUTPUT_DESC` (SDL leaves it zeroed)
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub(crate) struct StreamOutputDesc {
    pub(crate) so_declaration: *const c_void,
    pub(crate) num_entries: u32,
    pub(crate) buffer_strides: *const u32,
    pub(crate) num_strides: u32,
    pub(crate) rasterized_stream: u32,
}

/// `D3D12_RENDER_TARGET_BLEND_DESC`
#[repr(C)]
#[derive(Clone, Copy, Default, Debug, PartialEq, Eq)]
pub(crate) struct RenderTargetBlendDesc {
    pub(crate) blend_enable: BOOL,
    pub(crate) logic_op_enable: BOOL,
    pub(crate) src_blend: u32,
    pub(crate) dest_blend: u32,
    pub(crate) blend_op: u32,
    pub(crate) src_blend_alpha: u32,
    pub(crate) dest_blend_alpha: u32,
    pub(crate) blend_op_alpha: u32,
    pub(crate) logic_op: u32,
    pub(crate) render_target_write_mask: u8,
}

/// `D3D12_BLEND_DESC`
#[repr(C)]
#[derive(Clone, Copy, Default, Debug, PartialEq, Eq)]
pub(crate) struct BlendDesc {
    pub(crate) alpha_to_coverage_enable: BOOL,
    pub(crate) independent_blend_enable: BOOL,
    pub(crate) render_target: [RenderTargetBlendDesc; 8],
}

/// `D3D12_RASTERIZER_DESC`
#[repr(C)]
#[derive(Clone, Copy, Default, Debug, PartialEq)]
pub(crate) struct RasterizerDesc {
    pub(crate) fill_mode: u32,
    pub(crate) cull_mode: u32,
    pub(crate) front_counter_clockwise: BOOL,
    pub(crate) depth_bias: i32,
    pub(crate) depth_bias_clamp: f32,
    pub(crate) slope_scaled_depth_bias: f32,
    pub(crate) depth_clip_enable: BOOL,
    pub(crate) multisample_enable: BOOL,
    pub(crate) antialiased_line_enable: BOOL,
    pub(crate) forced_sample_count: u32,
    pub(crate) conservative_raster: u32,
}

/// `D3D12_DEPTH_STENCILOP_DESC`
#[repr(C)]
#[derive(Clone, Copy, Default, Debug, PartialEq, Eq)]
pub(crate) struct DepthStencilOpDesc {
    pub(crate) stencil_fail_op: u32,
    pub(crate) stencil_depth_fail_op: u32,
    pub(crate) stencil_pass_op: u32,
    pub(crate) stencil_func: u32,
}

/// `D3D12_DEPTH_STENCIL_DESC`
#[repr(C)]
#[derive(Clone, Copy, Default, Debug, PartialEq, Eq)]
pub(crate) struct DepthStencilDesc {
    pub(crate) depth_enable: BOOL,
    pub(crate) depth_write_mask: u32,
    pub(crate) depth_func: u32,
    pub(crate) stencil_enable: BOOL,
    pub(crate) stencil_read_mask: u8,
    pub(crate) stencil_write_mask: u8,
    pub(crate) front_face: DepthStencilOpDesc,
    pub(crate) back_face: DepthStencilOpDesc,
}

/// `D3D12_INPUT_ELEMENT_DESC`
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub(crate) struct InputElementDesc {
    pub(crate) semantic_name: *const c_char,
    pub(crate) semantic_index: u32,
    pub(crate) format: DxgiFormat,
    pub(crate) input_slot: u32,
    pub(crate) aligned_byte_offset: u32,
    pub(crate) input_slot_class: u32,
    pub(crate) instance_data_step_rate: u32,
}

/// `D3D12_INPUT_LAYOUT_DESC`
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub(crate) struct InputLayoutDesc {
    pub(crate) input_element_descs: *const InputElementDesc,
    pub(crate) num_elements: u32,
}

/// `D3D12_CACHED_PIPELINE_STATE` (SDL leaves it zeroed)
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub(crate) struct CachedPipelineState {
    pub(crate) cached_blob: *const c_void,
    pub(crate) cached_blob_size_in_bytes: usize,
}

/// `D3D12_GRAPHICS_PIPELINE_STATE_DESC`
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub(crate) struct GraphicsPipelineStateDesc {
    pub(crate) root_signature: *mut c_void,
    pub(crate) vs: ShaderBytecode,
    pub(crate) ps: ShaderBytecode,
    pub(crate) ds: ShaderBytecode,
    pub(crate) hs: ShaderBytecode,
    pub(crate) gs: ShaderBytecode,
    pub(crate) stream_output: StreamOutputDesc,
    pub(crate) blend_state: BlendDesc,
    pub(crate) sample_mask: u32,
    pub(crate) rasterizer_state: RasterizerDesc,
    pub(crate) depth_stencil_state: DepthStencilDesc,
    pub(crate) input_layout: InputLayoutDesc,
    pub(crate) ib_strip_cut_value: u32,
    pub(crate) primitive_topology_type: u32,
    pub(crate) num_render_targets: u32,
    pub(crate) rtv_formats: [DxgiFormat; 8],
    pub(crate) dsv_format: DxgiFormat,
    pub(crate) sample_desc: SampleDesc,
    pub(crate) node_mask: u32,
    pub(crate) cached_pso: CachedPipelineState,
    pub(crate) flags: u32,
}

/// `D3D12_COMPUTE_PIPELINE_STATE_DESC`
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub(crate) struct ComputePipelineStateDesc {
    pub(crate) root_signature: *mut c_void,
    pub(crate) cs: ShaderBytecode,
    pub(crate) node_mask: u32,
    pub(crate) cached_pso: CachedPipelineState,
    pub(crate) flags: u32,
}

/// `D3D12_INDIRECT_ARGUMENT_DESC` (the union is three `UINT`s at most; SDL
/// only uses argument types without parameters)
#[repr(C)]
#[derive(Clone, Copy, Default, Debug)]
pub(crate) struct IndirectArgumentDesc {
    pub(crate) ty: u32,
    _union: [u32; 3],
}

impl IndirectArgumentDesc {
    /// An argument of a type without parameters (`Draw`, `DrawIndexed`,
    /// `Dispatch`).
    pub(crate) fn new(ty: u32) -> IndirectArgumentDesc {
        IndirectArgumentDesc { ty, _union: [0; 3] }
    }
}

/// `D3D12_COMMAND_SIGNATURE_DESC`
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub(crate) struct CommandSignatureDesc {
    pub(crate) byte_stride: u32,
    pub(crate) num_argument_descs: u32,
    pub(crate) argument_descs: *const IndirectArgumentDesc,
    pub(crate) node_mask: u32,
}

/// `D3D12_FEATURE_DATA_FORMAT_SUPPORT`
#[repr(C)]
#[derive(Clone, Copy, Default, Debug)]
pub(crate) struct FeatureDataFormatSupport {
    pub(crate) format: DxgiFormat,
    pub(crate) support1: u32,
    pub(crate) support2: u32,
}

/// `D3D12_FEATURE_DATA_MULTISAMPLE_QUALITY_LEVELS`
#[repr(C)]
#[derive(Clone, Copy, Default, Debug)]
pub(crate) struct FeatureDataMultisampleQualityLevels {
    pub(crate) format: DxgiFormat,
    pub(crate) sample_count: u32,
    pub(crate) flags: u32,
    pub(crate) num_quality_levels: u32,
}

/// `D3D12_FEATURE_DATA_D3D12_OPTIONS`
#[repr(C)]
#[derive(Clone, Copy, Default, Debug)]
pub(crate) struct FeatureDataD3d12Options {
    pub(crate) double_precision_float_shader_ops: BOOL,
    pub(crate) output_merger_logic_op: BOOL,
    pub(crate) min_precision_support: u32,
    pub(crate) tiled_resources_tier: u32,
    pub(crate) resource_binding_tier: u32,
    pub(crate) ps_specified_stencil_ref_supported: BOOL,
    pub(crate) typed_uav_load_additional_formats: BOOL,
    pub(crate) rovs_supported: BOOL,
    pub(crate) conservative_rasterization_tier: u32,
    pub(crate) max_gpu_virtual_address_bits_per_resource: u32,
    pub(crate) standard_swizzle_64kb_supported: BOOL,
    pub(crate) cross_node_sharing_tier: u32,
    pub(crate) cross_adapter_row_major_texture_supported: BOOL,
    pub(crate) vp_and_rt_array_index_from_any_shader_feeding_rasterizer_supported_without_gs_emulation:
        BOOL,
    pub(crate) resource_heap_tier: u32,
}

/// `D3D12_FEATURE_DATA_SHADER_MODEL`
#[repr(C)]
#[derive(Clone, Copy, Default, Debug)]
pub(crate) struct FeatureDataShaderModel {
    pub(crate) highest_shader_model: u32,
}

/// `D3D12_FEATURE_DATA_D3D12_OPTIONS13`
#[repr(C)]
#[derive(Clone, Copy, Default, Debug)]
pub(crate) struct FeatureDataD3d12Options13 {
    pub(crate) unrestricted_buffer_texture_copy_pitch_supported: BOOL,
    pub(crate) unrestricted_vertex_element_alignment_supported: BOOL,
    pub(crate) inverted_viewport_height_flips_y_supported: BOOL,
    pub(crate) inverted_viewport_depth_flips_z_supported: BOOL,
    pub(crate) texture_copy_between_dimensions_supported: BOOL,
    pub(crate) alpha_blend_factor_supported: BOOL,
}

/// `D3D12_FEATURE_DATA_D3D12_OPTIONS16`
#[repr(C)]
#[derive(Clone, Copy, Default, Debug)]
pub(crate) struct FeatureDataD3d12Options16 {
    pub(crate) dynamic_depth_bias_supported: BOOL,
    pub(crate) gpu_upload_heap_supported: BOOL,
}

/// `D3D12_MESSAGE`
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub(crate) struct Message {
    pub(crate) category: u32,
    pub(crate) severity: u32,
    pub(crate) id: i32,
    pub(crate) description: *const c_char,
    pub(crate) description_byte_length: usize,
}

/// `D3D12_INFO_QUEUE_FILTER_DESC`
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub(crate) struct InfoQueueFilterDesc {
    pub(crate) num_categories: u32,
    pub(crate) category_list: *const u32,
    pub(crate) num_severities: u32,
    pub(crate) severity_list: *const u32,
    pub(crate) num_ids: u32,
    pub(crate) id_list: *const i32,
}

/// `D3D12_INFO_QUEUE_FILTER`
#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct InfoQueueFilter {
    pub(crate) allow_list: InfoQueueFilterDesc,
    pub(crate) deny_list: InfoQueueFilterDesc,
}

/// `D3D12_RESOURCE_TRANSITION_BARRIER`
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub(crate) struct ResourceTransitionBarrier {
    pub(crate) resource: *mut c_void,
    pub(crate) subresource: u32,
    pub(crate) state_before: u32,
    pub(crate) state_after: u32,
}

/// `D3D12_RESOURCE_ALIASING_BARRIER`
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub(crate) struct ResourceAliasingBarrier {
    pub(crate) resource_before: *mut c_void,
    pub(crate) resource_after: *mut c_void,
}

/// `D3D12_RESOURCE_UAV_BARRIER`
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub(crate) struct ResourceUavBarrier {
    pub(crate) resource: *mut c_void,
}

/// The union of `D3D12_RESOURCE_BARRIER`.
#[repr(C)]
#[derive(Clone, Copy)]
pub(crate) union ResourceBarrierUnion {
    pub(crate) transition: ResourceTransitionBarrier,
    pub(crate) aliasing: ResourceAliasingBarrier,
    pub(crate) uav: ResourceUavBarrier,
}

/// `D3D12_RESOURCE_BARRIER`
#[repr(C)]
#[derive(Clone, Copy)]
pub(crate) struct ResourceBarrier {
    pub(crate) ty: u32,
    pub(crate) flags: u32,
    pub(crate) u: ResourceBarrierUnion,
}

/// `D3D12_SUBRESOURCE_FOOTPRINT`
#[repr(C)]
#[derive(Clone, Copy, Default, Debug)]
pub(crate) struct SubresourceFootprint {
    pub(crate) format: DxgiFormat,
    pub(crate) width: u32,
    pub(crate) height: u32,
    pub(crate) depth: u32,
    pub(crate) row_pitch: u32,
}

/// `D3D12_PLACED_SUBRESOURCE_FOOTPRINT`
#[repr(C)]
#[derive(Clone, Copy, Default, Debug)]
pub(crate) struct PlacedSubresourceFootprint {
    pub(crate) offset: u64,
    pub(crate) footprint: SubresourceFootprint,
}

/// The union of `D3D12_TEXTURE_COPY_LOCATION`.
#[repr(C)]
#[derive(Clone, Copy)]
pub(crate) union TextureCopyLocationUnion {
    pub(crate) placed_footprint: PlacedSubresourceFootprint,
    pub(crate) subresource_index: u32,
}

/// `D3D12_TEXTURE_COPY_LOCATION`
#[repr(C)]
#[derive(Clone, Copy)]
pub(crate) struct TextureCopyLocation {
    pub(crate) resource: *mut c_void,
    pub(crate) ty: u32,
    pub(crate) u: TextureCopyLocationUnion,
}

/// `D3D12_RANGE`
#[repr(C)]
#[derive(Clone, Copy, Default, Debug)]
pub(crate) struct D3d12Range {
    pub(crate) begin: usize,
    pub(crate) end: usize,
}

/// `D3D12_BOX`
#[repr(C)]
#[derive(Clone, Copy, Default, Debug)]
pub(crate) struct D3d12Box {
    pub(crate) left: u32,
    pub(crate) top: u32,
    pub(crate) front: u32,
    pub(crate) right: u32,
    pub(crate) bottom: u32,
    pub(crate) back: u32,
}

/// `D3D12_VIEWPORT`
#[repr(C)]
#[derive(Clone, Copy, Default, Debug)]
pub(crate) struct D3d12Viewport {
    pub(crate) top_left_x: f32,
    pub(crate) top_left_y: f32,
    pub(crate) width: f32,
    pub(crate) height: f32,
    pub(crate) min_depth: f32,
    pub(crate) max_depth: f32,
}

/// `D3D12_RECT` (`RECT`)
#[repr(C)]
#[derive(Clone, Copy, Default, Debug)]
pub(crate) struct D3d12Rect {
    pub(crate) left: i32,
    pub(crate) top: i32,
    pub(crate) right: i32,
    pub(crate) bottom: i32,
}

/// `D3D12_VERTEX_BUFFER_VIEW`
#[repr(C)]
#[derive(Clone, Copy, Default, Debug)]
pub(crate) struct VertexBufferView {
    pub(crate) buffer_location: u64,
    pub(crate) size_in_bytes: u32,
    pub(crate) stride_in_bytes: u32,
}

/// `D3D12_INDEX_BUFFER_VIEW`
#[repr(C)]
#[derive(Clone, Copy, Default, Debug)]
pub(crate) struct IndexBufferView {
    pub(crate) buffer_location: u64,
    pub(crate) size_in_bytes: u32,
    pub(crate) format: DxgiFormat,
}

/// `DXGI_SWAP_CHAIN_DESC1`
#[repr(C)]
#[derive(Clone, Copy, Default, Debug)]
pub(crate) struct SwapChainDesc1 {
    pub(crate) width: u32,
    pub(crate) height: u32,
    pub(crate) format: DxgiFormat,
    pub(crate) stereo: BOOL,
    pub(crate) sample_desc: SampleDesc,
    pub(crate) buffer_usage: u32,
    pub(crate) buffer_count: u32,
    pub(crate) scaling: u32,
    pub(crate) swap_effect: u32,
    pub(crate) alpha_mode: u32,
    pub(crate) flags: u32,
}

/// `DXGI_RATIONAL`
#[repr(C)]
#[derive(Clone, Copy, Default, Debug)]
pub(crate) struct DxgiRational {
    pub(crate) numerator: u32,
    pub(crate) denominator: u32,
}

/// `DXGI_SWAP_CHAIN_FULLSCREEN_DESC`
#[repr(C)]
#[derive(Clone, Copy, Default, Debug)]
pub(crate) struct SwapChainFullscreenDesc {
    pub(crate) refresh_rate: DxgiRational,
    pub(crate) scanline_ordering: u32,
    pub(crate) scaling: u32,
    pub(crate) windowed: BOOL,
}

zeroed_default!(
    ResourceBarrier,
    TextureCopyLocation,
    ClearValue,
    ShaderResourceViewDesc,
    RenderTargetViewDesc,
    DepthStencilViewDesc,
    UnorderedAccessViewDesc,
    RootDescriptorTable,
    RootParameter,
    RootSignatureDesc,
    ShaderBytecode,
    StreamOutputDesc,
    InputLayoutDesc,
    CachedPipelineState,
    GraphicsPipelineStateDesc,
    ComputePipelineStateDesc,
    CommandSignatureDesc,
    InfoQueueFilterDesc,
);

// --- entry points ---

/// `PFN_D3D12_CREATE_DEVICE`
pub(crate) type PfnD3d12CreateDevice = unsafe extern "system" fn(
    *mut c_void,
    D3dFeatureLevel,
    *const GUID,
    *mut *mut c_void,
) -> HRESULT;
/// `PFN_D3D12_SERIALIZE_ROOT_SIGNATURE`
pub(crate) type PfnD3d12SerializeRootSignature = unsafe extern "system" fn(
    *const RootSignatureDesc,
    u32,
    *mut *mut c_void,
    *mut *mut c_void,
) -> HRESULT;
/// `PFN_D3D12_GET_DEBUG_INTERFACE` (and `pfnDXGIGetDebugInterface`, the
/// same signature)
pub(crate) type PfnD3d12GetDebugInterface =
    unsafe extern "system" fn(*const GUID, *mut *mut c_void) -> HRESULT;
/// `PFN_D3D12_GET_INTERFACE`
pub(crate) type PfnD3d12GetInterface =
    unsafe extern "system" fn(*const GUID, *const GUID, *mut *mut c_void) -> HRESULT;
/// `pfnBeginEventOnCommandList`, `pfnSetMarkerOnCommandList`
pub(crate) type PfnBeginEventOnCommandList =
    unsafe extern "system" fn(*mut c_void, u64, *const c_char);
/// `pfnEndEventOnCommandList`
pub(crate) type PfnEndEventOnCommandList = unsafe extern "system" fn(*mut c_void);
/// `D3D12MessageFunc`
pub(crate) type D3d12MessageFunc =
    unsafe extern "system" fn(u32, u32, i32, *const c_char, *mut c_void);

// --- the COM interfaces SDL calls ---

/// `ID3D12Object`'s methods but `SetName()`: GetPrivateData,
/// SetPrivateData, SetPrivateDataInterface.
type PrivateDataSlots = [Slot; 3];

/// `ID3D12ObjectVtbl`, which every Direct3D 12 object's vtable starts with.
#[repr(C)]
pub(crate) struct ID3D12ObjectVtbl {
    pub(crate) base: IUnknownVtbl,
    _private_data: PrivateDataSlots,
    pub(crate) set_name: unsafe extern "system" fn(*mut c_void, *const u16) -> HRESULT,
}

/// `ID3D12DeviceVtbl` (the part SDL uses).
#[repr(C)]
pub(crate) struct ID3D12DeviceVtbl {
    pub(crate) object: ID3D12ObjectVtbl,
    _get_node_count: Slot,
    pub(crate) create_command_queue: unsafe extern "system" fn(
        *mut c_void,
        *const CommandQueueDesc,
        *const GUID,
        *mut *mut c_void,
    ) -> HRESULT,
    pub(crate) create_command_allocator:
        unsafe extern "system" fn(*mut c_void, u32, *const GUID, *mut *mut c_void) -> HRESULT,
    pub(crate) create_graphics_pipeline_state: unsafe extern "system" fn(
        *mut c_void,
        *const GraphicsPipelineStateDesc,
        *const GUID,
        *mut *mut c_void,
    ) -> HRESULT,
    pub(crate) create_compute_pipeline_state: unsafe extern "system" fn(
        *mut c_void,
        *const ComputePipelineStateDesc,
        *const GUID,
        *mut *mut c_void,
    ) -> HRESULT,
    pub(crate) create_command_list: unsafe extern "system" fn(
        *mut c_void,
        u32,
        u32,
        *mut c_void,
        *mut c_void,
        *const GUID,
        *mut *mut c_void,
    ) -> HRESULT,
    pub(crate) check_feature_support:
        unsafe extern "system" fn(*mut c_void, u32, *mut c_void, u32) -> HRESULT,
    pub(crate) create_descriptor_heap: unsafe extern "system" fn(
        *mut c_void,
        *const DescriptorHeapDesc,
        *const GUID,
        *mut *mut c_void,
    ) -> HRESULT,
    pub(crate) get_descriptor_handle_increment_size:
        unsafe extern "system" fn(*mut c_void, u32) -> u32,
    pub(crate) create_root_signature: unsafe extern "system" fn(
        *mut c_void,
        u32,
        *const c_void,
        usize,
        *const GUID,
        *mut *mut c_void,
    ) -> HRESULT,
    _create_constant_buffer_view: Slot,
    pub(crate) create_shader_resource_view: unsafe extern "system" fn(
        *mut c_void,
        *mut c_void,
        *const ShaderResourceViewDesc,
        CpuDescriptorHandle,
    ),
    pub(crate) create_unordered_access_view: unsafe extern "system" fn(
        *mut c_void,
        *mut c_void,
        *mut c_void,
        *const UnorderedAccessViewDesc,
        CpuDescriptorHandle,
    ),
    pub(crate) create_render_target_view: unsafe extern "system" fn(
        *mut c_void,
        *mut c_void,
        *const RenderTargetViewDesc,
        CpuDescriptorHandle,
    ),
    pub(crate) create_depth_stencil_view: unsafe extern "system" fn(
        *mut c_void,
        *mut c_void,
        *const DepthStencilViewDesc,
        CpuDescriptorHandle,
    ),
    pub(crate) create_sampler:
        unsafe extern "system" fn(*mut c_void, *const SamplerDesc, CpuDescriptorHandle),
    _copy_descriptors: Slot,
    pub(crate) copy_descriptors_simple:
        unsafe extern "system" fn(*mut c_void, u32, CpuDescriptorHandle, CpuDescriptorHandle, u32),
    /// GetResourceAllocationInfo, GetCustomHeapProperties
    _get_resource_allocation_info: [Slot; 2],
    pub(crate) create_committed_resource: unsafe extern "system" fn(
        *mut c_void,
        *const HeapProperties,
        u32,
        *const ResourceDesc,
        u32,
        *const ClearValue,
        *const GUID,
        *mut *mut c_void,
    ) -> HRESULT,
    /// CreateHeap, CreatePlacedResource, CreateReservedResource,
    /// CreateSharedHandle, OpenSharedHandle, OpenSharedHandleByName,
    /// MakeResident, Evict
    _create_heap: [Slot; 8],
    pub(crate) create_fence:
        unsafe extern "system" fn(*mut c_void, u64, u32, *const GUID, *mut *mut c_void) -> HRESULT,
    pub(crate) get_device_removed_reason: unsafe extern "system" fn(*mut c_void) -> HRESULT,
    pub(crate) get_copyable_footprints: unsafe extern "system" fn(
        *mut c_void,
        *const ResourceDesc,
        u32,
        u32,
        u64,
        *mut PlacedSubresourceFootprint,
        *mut u32,
        *mut u64,
        *mut u64,
    ),
    /// CreateQueryHeap, SetStablePowerState
    _create_query_heap: [Slot; 2],
    pub(crate) create_command_signature: unsafe extern "system" fn(
        *mut c_void,
        *const CommandSignatureDesc,
        *mut c_void,
        *const GUID,
        *mut *mut c_void,
    ) -> HRESULT,
    /// GetResourceTiling, GetAdapterLuid
    _get_resource_tiling: [Slot; 2],
}

/// `ID3D12DescriptorHeapVtbl` (the part SDL uses).
#[repr(C)]
pub(crate) struct ID3D12DescriptorHeapVtbl {
    pub(crate) object: ID3D12ObjectVtbl,
    /// ID3D12DeviceChild's GetDevice; GetDesc
    _get_device: [Slot; 2],
    pub(crate) get_cpu_descriptor_handle_for_heap_start:
        unsafe extern "system" fn(
            *mut c_void,
            *mut CpuDescriptorHandle,
        ) -> *mut CpuDescriptorHandle,
    pub(crate) get_gpu_descriptor_handle_for_heap_start:
        unsafe extern "system" fn(
            *mut c_void,
            *mut GpuDescriptorHandle,
        ) -> *mut GpuDescriptorHandle,
}

/// `ID3D12ResourceVtbl` (the part SDL uses).
#[repr(C)]
pub(crate) struct ID3D12ResourceVtbl {
    pub(crate) object: ID3D12ObjectVtbl,
    /// ID3D12DeviceChild's GetDevice
    _get_device: Slot,
    pub(crate) map:
        unsafe extern "system" fn(*mut c_void, u32, *const D3d12Range, *mut *mut c_void) -> HRESULT,
    pub(crate) unmap: unsafe extern "system" fn(*mut c_void, u32, *const c_void),
    pub(crate) get_desc:
        unsafe extern "system" fn(*mut c_void, *mut ResourceDesc) -> *mut ResourceDesc,
    pub(crate) get_gpu_virtual_address: unsafe extern "system" fn(*mut c_void) -> u64,
    /// WriteToSubresource, ReadFromSubresource, GetHeapProperties
    _write_to_subresource: [Slot; 3],
}

/// `ID3D12PipelineStateVtbl` (`ID3D12DeviceChild`'s GetDevice and
/// GetCachedBlob follow).
#[repr(C)]
pub(crate) struct ID3D12PipelineStateVtbl {
    pub(crate) object: ID3D12ObjectVtbl,
}

/// `ID3DBlobVtbl` (`ID3D10Blob`).
#[repr(C)]
pub(crate) struct ID3DBlobVtbl {
    pub(crate) base: IUnknownVtbl,
    pub(crate) get_buffer_pointer: unsafe extern "system" fn(*mut c_void) -> *mut c_void,
    pub(crate) get_buffer_size: unsafe extern "system" fn(*mut c_void) -> usize,
}

/// `ID3D12DebugVtbl`.
#[repr(C)]
pub(crate) struct ID3D12DebugVtbl {
    pub(crate) base: IUnknownVtbl,
    pub(crate) enable_debug_layer: unsafe extern "system" fn(*mut c_void),
}

/// `ID3D12InfoQueueVtbl` (the part SDL uses).
#[repr(C)]
pub(crate) struct ID3D12InfoQueueVtbl {
    pub(crate) base: IUnknownVtbl,
    _set_message_count_limit: Slot,
    pub(crate) clear_stored_messages: unsafe extern "system" fn(*mut c_void),
    pub(crate) get_message:
        unsafe extern "system" fn(*mut c_void, u64, *mut Message, *mut usize) -> HRESULT,
    /// GetNumMessagesAllowedByStorageFilter,
    /// GetNumMessagesDeniedByStorageFilter
    _get_num_messages_allowed_by_storage_filter: [Slot; 2],
    pub(crate) get_num_stored_messages: unsafe extern "system" fn(*mut c_void) -> u64,
    /// GetNumStoredMessagesAllowedByRetrievalFilter,
    /// GetNumMessagesDiscardedByMessageCountLimit, GetMessageCountLimit,
    /// AddStorageFilterEntries, GetStorageFilter, ClearStorageFilter,
    /// PushEmptyStorageFilter, PushCopyOfStorageFilter
    _get_num_stored_messages_allowed_by_retrieval_filter: [Slot; 8],
    pub(crate) push_storage_filter:
        unsafe extern "system" fn(*mut c_void, *const InfoQueueFilter) -> HRESULT,
    /// PopStorageFilter ... SetBreakOnCategory (13 methods)
    _pop_storage_filter: [Slot; 13],
    pub(crate) set_break_on_severity: unsafe extern "system" fn(*mut c_void, u32, BOOL) -> HRESULT,
    /// SetBreakOnID, GetBreakOnCategory, GetBreakOnSeverity, GetBreakOnID,
    /// SetMuteDebugOutput, GetMuteDebugOutput
    _set_break_on_id: [Slot; 6],
}

/// `ID3D12InfoQueue1Vtbl` (the part SDL uses).
#[repr(C)]
pub(crate) struct ID3D12InfoQueue1Vtbl {
    pub(crate) info_queue: ID3D12InfoQueueVtbl,
    pub(crate) register_message_callback: unsafe extern "system" fn(
        *mut c_void,
        D3d12MessageFunc,
        u32,
        *mut c_void,
        *mut u32,
    ) -> HRESULT,
    _unregister_message_callback: Slot,
}

/// `ID3D12SDKConfigurationVtbl`.
#[repr(C)]
pub(crate) struct ID3D12SDKConfigurationVtbl {
    pub(crate) base: IUnknownVtbl,
    _set_sdk_version: Slot,
}

/// `ID3D12SDKConfiguration1Vtbl` (the part SDL uses).
#[repr(C)]
pub(crate) struct ID3D12SDKConfiguration1Vtbl {
    pub(crate) configuration: ID3D12SDKConfigurationVtbl,
    pub(crate) create_device_factory: unsafe extern "system" fn(
        *mut c_void,
        u32,
        *const c_char,
        *const GUID,
        *mut *mut c_void,
    ) -> HRESULT,
    _free_unused_sdks: Slot,
}

/// `ID3D12DeviceFactoryVtbl` (the part SDL uses).
#[repr(C)]
pub(crate) struct ID3D12DeviceFactoryVtbl {
    pub(crate) base: IUnknownVtbl,
    /// InitializeFromGlobalState, ApplyToGlobalState
    _initialize_from_global_state: [Slot; 2],
    pub(crate) set_flags: unsafe extern "system" fn(*mut c_void, u32) -> HRESULT,
    _get_flags: Slot,
    pub(crate) get_configuration_interface: unsafe extern "system" fn(
        *mut c_void,
        *const GUID,
        *const GUID,
        *mut *mut c_void,
    ) -> HRESULT,
    _enable_experimental_features: Slot,
    pub(crate) create_device: unsafe extern "system" fn(
        *mut c_void,
        *mut c_void,
        D3dFeatureLevel,
        *const GUID,
        *mut *mut c_void,
    ) -> HRESULT,
}

/// `ID3D12CommandQueueVtbl` (the part SDL uses).
#[repr(C)]
pub(crate) struct ID3D12CommandQueueVtbl {
    pub(crate) object: ID3D12ObjectVtbl,
    /// ID3D12DeviceChild's GetDevice; UpdateTileMappings, CopyTileMappings
    _get_device: [Slot; 3],
    pub(crate) execute_command_lists:
        unsafe extern "system" fn(*mut c_void, u32, *const *mut c_void),
    /// SetMarker, BeginEvent, EndEvent
    _set_marker: [Slot; 3],
    pub(crate) signal: unsafe extern "system" fn(*mut c_void, *mut c_void, u64) -> HRESULT,
    /// Wait, GetTimestampFrequency, GetClockCalibration, GetDesc
    _wait: [Slot; 4],
}

/// `ID3D12CommandAllocatorVtbl`.
#[repr(C)]
pub(crate) struct ID3D12CommandAllocatorVtbl {
    pub(crate) object: ID3D12ObjectVtbl,
    /// ID3D12DeviceChild's GetDevice
    _get_device: Slot,
    pub(crate) reset: unsafe extern "system" fn(*mut c_void) -> HRESULT,
}

/// `ID3D12FenceVtbl`.
#[repr(C)]
pub(crate) struct ID3D12FenceVtbl {
    pub(crate) object: ID3D12ObjectVtbl,
    /// ID3D12DeviceChild's GetDevice
    _get_device: Slot,
    pub(crate) get_completed_value: unsafe extern "system" fn(*mut c_void) -> u64,
    pub(crate) set_event_on_completion:
        unsafe extern "system" fn(*mut c_void, u64, HANDLE) -> HRESULT,
    pub(crate) signal: unsafe extern "system" fn(*mut c_void, u64) -> HRESULT,
}

/// `ID3D12GraphicsCommandListVtbl` (the part SDL uses).
#[repr(C)]
pub(crate) struct ID3D12GraphicsCommandListVtbl {
    pub(crate) object: ID3D12ObjectVtbl,
    /// ID3D12DeviceChild's GetDevice; ID3D12CommandList's GetType
    _get_device: [Slot; 2],
    pub(crate) close: unsafe extern "system" fn(*mut c_void) -> HRESULT,
    pub(crate) reset: unsafe extern "system" fn(*mut c_void, *mut c_void, *mut c_void) -> HRESULT,
    _clear_state: Slot,
    pub(crate) draw_instanced: unsafe extern "system" fn(*mut c_void, u32, u32, u32, u32),
    pub(crate) draw_indexed_instanced:
        unsafe extern "system" fn(*mut c_void, u32, u32, u32, i32, u32),
    pub(crate) dispatch: unsafe extern "system" fn(*mut c_void, u32, u32, u32),
    pub(crate) copy_buffer_region:
        unsafe extern "system" fn(*mut c_void, *mut c_void, u64, *mut c_void, u64, u64),
    pub(crate) copy_texture_region: unsafe extern "system" fn(
        *mut c_void,
        *const TextureCopyLocation,
        u32,
        u32,
        u32,
        *const TextureCopyLocation,
        *const D3d12Box,
    ),
    /// CopyResource, CopyTiles
    _copy_resource: [Slot; 2],
    pub(crate) resolve_subresource:
        unsafe extern "system" fn(*mut c_void, *mut c_void, u32, *mut c_void, u32, DxgiFormat),
    pub(crate) ia_set_primitive_topology: unsafe extern "system" fn(*mut c_void, u32),
    pub(crate) rs_set_viewports: unsafe extern "system" fn(*mut c_void, u32, *const D3d12Viewport),
    pub(crate) rs_set_scissor_rects: unsafe extern "system" fn(*mut c_void, u32, *const D3d12Rect),
    pub(crate) om_set_blend_factor: unsafe extern "system" fn(*mut c_void, *const f32),
    pub(crate) om_set_stencil_ref: unsafe extern "system" fn(*mut c_void, u32),
    pub(crate) set_pipeline_state: unsafe extern "system" fn(*mut c_void, *mut c_void),
    pub(crate) resource_barrier:
        unsafe extern "system" fn(*mut c_void, u32, *const ResourceBarrier),
    _execute_bundle: Slot,
    pub(crate) set_descriptor_heaps:
        unsafe extern "system" fn(*mut c_void, u32, *const *mut c_void),
    pub(crate) set_compute_root_signature: unsafe extern "system" fn(*mut c_void, *mut c_void),
    pub(crate) set_graphics_root_signature: unsafe extern "system" fn(*mut c_void, *mut c_void),
    pub(crate) set_compute_root_descriptor_table:
        unsafe extern "system" fn(*mut c_void, u32, GpuDescriptorHandle),
    pub(crate) set_graphics_root_descriptor_table:
        unsafe extern "system" fn(*mut c_void, u32, GpuDescriptorHandle),
    /// SetComputeRoot32BitConstant, SetGraphicsRoot32BitConstant,
    /// SetComputeRoot32BitConstants
    _set_compute_root_32bit_constant: [Slot; 3],
    pub(crate) set_graphics_root_32bit_constants:
        unsafe extern "system" fn(*mut c_void, u32, u32, *const c_void, u32),
    pub(crate) set_compute_root_constant_buffer_view:
        unsafe extern "system" fn(*mut c_void, u32, u64),
    pub(crate) set_graphics_root_constant_buffer_view:
        unsafe extern "system" fn(*mut c_void, u32, u64),
    /// SetComputeRootShaderResourceView, SetGraphicsRootShaderResourceView,
    /// SetComputeRootUnorderedAccessView, SetGraphicsRootUnorderedAccessView
    _set_compute_root_shader_resource_view: [Slot; 4],
    pub(crate) ia_set_index_buffer: unsafe extern "system" fn(*mut c_void, *const IndexBufferView),
    pub(crate) ia_set_vertex_buffers:
        unsafe extern "system" fn(*mut c_void, u32, u32, *const VertexBufferView),
    _so_set_targets: Slot,
    pub(crate) om_set_render_targets: unsafe extern "system" fn(
        *mut c_void,
        u32,
        *const CpuDescriptorHandle,
        BOOL,
        *const CpuDescriptorHandle,
    ),
    pub(crate) clear_depth_stencil_view: unsafe extern "system" fn(
        *mut c_void,
        CpuDescriptorHandle,
        u32,
        f32,
        u8,
        u32,
        *const D3d12Rect,
    ),
    pub(crate) clear_render_target_view: unsafe extern "system" fn(
        *mut c_void,
        CpuDescriptorHandle,
        *const f32,
        u32,
        *const D3d12Rect,
    ),
    /// ClearUnorderedAccessViewUint, ClearUnorderedAccessViewFloat,
    /// DiscardResource, BeginQuery, EndQuery, ResolveQueryData,
    /// SetPredication, SetMarker, BeginEvent, EndEvent
    _clear_unordered_access_view_uint: [Slot; 10],
    pub(crate) execute_indirect: unsafe extern "system" fn(
        *mut c_void,
        *mut c_void,
        u32,
        *mut c_void,
        u64,
        *mut c_void,
        u64,
    ),
}

/// `IDXGISwapChain3Vtbl` (the part SDL uses; the Direct3D 11 renderer's
/// declaration names fewer methods).
#[repr(C)]
pub(crate) struct IDXGISwapChain3Vtbl {
    pub(crate) base: IUnknownVtbl,
    /// IDXGIObject's SetPrivateData, SetPrivateDataInterface,
    /// GetPrivateData
    _set_private_data: [Slot; 3],
    pub(crate) get_parent:
        unsafe extern "system" fn(*mut c_void, *const GUID, *mut *mut c_void) -> HRESULT,
    /// IDXGIDeviceSubObject's GetDevice
    _get_device: Slot,
    pub(crate) present: unsafe extern "system" fn(*mut c_void, u32, u32) -> HRESULT,
    pub(crate) get_buffer:
        unsafe extern "system" fn(*mut c_void, u32, *const GUID, *mut *mut c_void) -> HRESULT,
    /// SetFullscreenState, GetFullscreenState, GetDesc
    _set_fullscreen_state: [Slot; 3],
    pub(crate) resize_buffers:
        unsafe extern "system" fn(*mut c_void, u32, u32, u32, DxgiFormat, u32) -> HRESULT,
    /// ResizeTarget, GetContainingOutput, GetFrameStatistics,
    /// GetLastPresentCount
    _resize_target: [Slot; 4],
    pub(crate) get_desc1: unsafe extern "system" fn(*mut c_void, *mut SwapChainDesc1) -> HRESULT,
    /// GetFullscreenDesc, GetHwnd, GetCoreWindow, Present1,
    /// IsTemporaryMonoSupported, GetRestrictToOutput, SetBackgroundColor,
    /// GetBackgroundColor
    _get_fullscreen_desc: [Slot; 8],
    pub(crate) set_rotation: unsafe extern "system" fn(*mut c_void, u32) -> HRESULT,
    /// GetRotation; IDXGISwapChain2's SetSourceSize, GetSourceSize
    _get_rotation: [Slot; 3],
    pub(crate) set_maximum_frame_latency: unsafe extern "system" fn(*mut c_void, u32) -> HRESULT,
    /// GetMaximumFrameLatency, GetFrameLatencyWaitableObject,
    /// SetMatrixTransform, GetMatrixTransform
    _get_maximum_frame_latency: [Slot; 4],
    pub(crate) get_current_back_buffer_index: unsafe extern "system" fn(*mut c_void) -> u32,
    pub(crate) check_color_space_support:
        unsafe extern "system" fn(*mut c_void, DxgiColorSpaceType, *mut u32) -> HRESULT,
    pub(crate) set_color_space1:
        unsafe extern "system" fn(*mut c_void, DxgiColorSpaceType) -> HRESULT,
    _resize_buffers1: Slot,
}

/// The vtable of an interface SDL calls none of the methods of (root
/// signatures, command signatures and lists): just its `IUnknown` part.
#[repr(C)]
pub(crate) struct OpaqueVtbl {
    pub(crate) base: IUnknownVtbl,
}

pub(crate) type D3d12Device = ComPtr<ID3D12DeviceVtbl>;
pub(crate) type D3d12DescriptorHeap = ComPtr<ID3D12DescriptorHeapVtbl>;
pub(crate) type D3d12Resource = ComPtr<ID3D12ResourceVtbl>;
pub(crate) type D3d12PipelineState = ComPtr<ID3D12PipelineStateVtbl>;
pub(crate) type D3d12RootSignature = ComPtr<OpaqueVtbl>;
pub(crate) type D3d12CommandQueue = ComPtr<ID3D12CommandQueueVtbl>;
pub(crate) type D3d12CommandSignature = ComPtr<OpaqueVtbl>;
pub(crate) type D3d12CommandAllocator = ComPtr<ID3D12CommandAllocatorVtbl>;
pub(crate) type D3d12GraphicsCommandList = ComPtr<ID3D12GraphicsCommandListVtbl>;
pub(crate) type D3d12CommandList = ComPtr<OpaqueVtbl>;
pub(crate) type D3d12Fence = ComPtr<ID3D12FenceVtbl>;
pub(crate) type DxgiSwapChain3 = ComPtr<IDXGISwapChain3Vtbl>;
pub(crate) type D3dBlob = ComPtr<ID3DBlobVtbl>;
pub(crate) type D3d12Debug = ComPtr<ID3D12DebugVtbl>;
pub(crate) type D3d12InfoQueue = ComPtr<ID3D12InfoQueueVtbl>;
pub(crate) type D3d12InfoQueue1 = ComPtr<ID3D12InfoQueue1Vtbl>;
pub(crate) type D3d12SdkConfiguration1 = ComPtr<ID3D12SDKConfiguration1Vtbl>;
pub(crate) type D3d12DeviceFactory = ComPtr<ID3D12DeviceFactoryVtbl>;

/// `ID3D12Object::SetName()`, for the objects SDL names.
pub(crate) trait D3d12Object {
    /// The object's `ID3D12Object` vtable and interface pointer.
    fn object(&self) -> (&ID3D12ObjectVtbl, *mut c_void);

    /// `ID3D12Object::SetName()`: `name` is a NUL-terminated UTF-16 string.
    fn set_name(&self, name: &[u16]) {
        debug_assert_eq!(name.last(), Some(&0));
        let (vtbl, this) = self.object();
        // SAFETY: a live object and a NUL-terminated wide string.
        unsafe { (vtbl.set_name)(this, name.as_ptr()) };
    }
}

impl D3d12Object for D3d12Resource {
    fn object(&self) -> (&ID3D12ObjectVtbl, *mut c_void) {
        (&self.vtbl().object, raw(self))
    }
}

impl D3d12Object for D3d12PipelineState {
    fn object(&self) -> (&ID3D12ObjectVtbl, *mut c_void) {
        (&self.vtbl().object, raw(self))
    }
}

impl D3d12Device {
    /// `ID3D12Device::CreateCommandQueue()`
    pub(crate) fn create_command_queue(
        &self,
        desc: &CommandQueueDesc,
    ) -> Result<D3d12CommandQueue, HRESULT> {
        let f = self.vtbl().create_command_queue;
        // SAFETY: a live device and description; the call stores an owned
        // ID3D12CommandQueue, the interface asked for.
        unsafe { out(|o| f(raw(self), desc, &IID_ID3D12COMMANDQUEUE, o)) }
    }

    /// `ID3D12Device::CreateGraphicsPipelineState()`
    ///
    /// # Safety
    ///
    /// The description's pointers (bytecode, input layout, root signature)
    /// must be valid.
    pub(crate) unsafe fn create_graphics_pipeline_state(
        &self,
        desc: &GraphicsPipelineStateDesc,
    ) -> Result<D3d12PipelineState, HRESULT> {
        let f = self.vtbl().create_graphics_pipeline_state;
        // SAFETY: the caller's contract; the call stores an owned
        // ID3D12PipelineState.
        unsafe { out(|o| f(raw(self), desc, &IID_ID3D12PIPELINESTATE, o)) }
    }

    /// `ID3D12Device::CreateComputePipelineState()`
    ///
    /// # Safety
    ///
    /// The description's pointers (bytecode, root signature) must be valid.
    pub(crate) unsafe fn create_compute_pipeline_state(
        &self,
        desc: &ComputePipelineStateDesc,
    ) -> Result<D3d12PipelineState, HRESULT> {
        let f = self.vtbl().create_compute_pipeline_state;
        // SAFETY: the caller's contract; the call stores an owned
        // ID3D12PipelineState.
        unsafe { out(|o| f(raw(self), desc, &IID_ID3D12PIPELINESTATE, o)) }
    }

    /// `ID3D12Device::CheckFeatureSupport()` of `feature`, whose data
    /// structure is a `T`.
    pub(crate) fn check_feature_support<T: Copy>(&self, feature: u32, data: &mut T) -> HRESULT {
        // SAFETY: a live device; `data` is the feature's structure, of the
        // size passed.
        unsafe {
            (self.vtbl().check_feature_support)(
                raw(self),
                feature,
                (data as *mut T).cast(),
                size_of::<T>() as u32,
            )
        }
    }

    /// `ID3D12Device::CreateDescriptorHeap()`
    pub(crate) fn create_descriptor_heap(
        &self,
        desc: &DescriptorHeapDesc,
    ) -> Result<D3d12DescriptorHeap, HRESULT> {
        let f = self.vtbl().create_descriptor_heap;
        // SAFETY: a live device and description; the call stores an owned
        // ID3D12DescriptorHeap.
        unsafe { out(|o| f(raw(self), desc, &IID_ID3D12DESCRIPTORHEAP, o)) }
    }

    /// `ID3D12Device::GetDescriptorHandleIncrementSize()`
    pub(crate) fn descriptor_handle_increment_size(&self, heap_type: u32) -> u32 {
        // SAFETY: a live device.
        unsafe { (self.vtbl().get_descriptor_handle_increment_size)(raw(self), heap_type) }
    }

    /// `ID3D12Device::CreateRootSignature()` from a serialized root
    /// signature.
    pub(crate) fn create_root_signature(&self, blob: &[u8]) -> Result<D3d12RootSignature, HRESULT> {
        let f = self.vtbl().create_root_signature;
        // SAFETY: a live device and the blob's bytes; the call stores an
        // owned ID3D12RootSignature.
        unsafe {
            out(|o| {
                f(
                    raw(self),
                    0,
                    blob.as_ptr().cast(),
                    blob.len(),
                    &IID_ID3D12ROOTSIGNATURE,
                    o,
                )
            })
        }
    }

    /// `ID3D12Device::CreateShaderResourceView()`
    pub(crate) fn create_shader_resource_view(
        &self,
        resource: &D3d12Resource,
        desc: &ShaderResourceViewDesc,
        dest: CpuDescriptorHandle,
    ) {
        // SAFETY: a live device and resource, a description, and a
        // descriptor of a live heap.
        unsafe { (self.vtbl().create_shader_resource_view)(raw(self), raw(resource), desc, dest) }
    }

    /// `ID3D12Device::CreateUnorderedAccessView()`, without a counter.
    pub(crate) fn create_unordered_access_view(
        &self,
        resource: &D3d12Resource,
        desc: &UnorderedAccessViewDesc,
        dest: CpuDescriptorHandle,
    ) {
        // SAFETY: as for create_shader_resource_view; no counter resource.
        unsafe {
            (self.vtbl().create_unordered_access_view)(
                raw(self),
                raw(resource),
                null_mut(),
                desc,
                dest,
            )
        }
    }

    /// `ID3D12Device::CreateRenderTargetView()`
    pub(crate) fn create_render_target_view(
        &self,
        resource: &D3d12Resource,
        desc: &RenderTargetViewDesc,
        dest: CpuDescriptorHandle,
    ) {
        // SAFETY: as for create_shader_resource_view.
        unsafe { (self.vtbl().create_render_target_view)(raw(self), raw(resource), desc, dest) }
    }

    /// `ID3D12Device::CreateDepthStencilView()`
    pub(crate) fn create_depth_stencil_view(
        &self,
        resource: &D3d12Resource,
        desc: &DepthStencilViewDesc,
        dest: CpuDescriptorHandle,
    ) {
        // SAFETY: as for create_shader_resource_view.
        unsafe { (self.vtbl().create_depth_stencil_view)(raw(self), raw(resource), desc, dest) }
    }

    /// `ID3D12Device::CreateSampler()`
    pub(crate) fn create_sampler(&self, desc: &SamplerDesc, dest: CpuDescriptorHandle) {
        // SAFETY: a live device, a description and a descriptor of a live
        // heap.
        unsafe { (self.vtbl().create_sampler)(raw(self), desc, dest) }
    }

    /// `ID3D12Device::CreateCommittedResource()`
    pub(crate) fn create_committed_resource(
        &self,
        heap_properties: &HeapProperties,
        heap_flags: u32,
        desc: &ResourceDesc,
        initial_resource_state: u32,
        optimized_clear_value: Option<&ClearValue>,
    ) -> Result<D3d12Resource, HRESULT> {
        let f = self.vtbl().create_committed_resource;
        let clear_value = optimized_clear_value.map_or(null(), |c| c as *const ClearValue);
        // SAFETY: a live device and descriptions; the call stores an owned
        // ID3D12Resource.
        unsafe {
            out(|o| {
                f(
                    raw(self),
                    heap_properties,
                    heap_flags,
                    desc,
                    initial_resource_state,
                    clear_value,
                    &IID_ID3D12RESOURCE,
                    o,
                )
            })
        }
    }

    /// `ID3D12Device::GetDeviceRemovedReason()`
    pub(crate) fn device_removed_reason(&self) -> HRESULT {
        // SAFETY: a live device.
        unsafe { (self.vtbl().get_device_removed_reason)(raw(self)) }
    }

    /// `ID3D12Device::CreateCommandSignature()`, without a root signature.
    pub(crate) fn create_command_signature(
        &self,
        desc: &CommandSignatureDesc,
    ) -> Result<D3d12CommandSignature, HRESULT> {
        let f = self.vtbl().create_command_signature;
        // SAFETY: a live device and description (whose arguments the
        // caller keeps alive); the call stores an owned
        // ID3D12CommandSignature.
        unsafe { out(|o| f(raw(self), desc, null_mut(), &IID_ID3D12COMMANDSIGNATURE, o)) }
    }
}

impl D3d12DescriptorHeap {
    /// `ID3D12DescriptorHeap::GetCPUDescriptorHandleForHeapStart()`
    pub(crate) fn cpu_descriptor_handle_for_heap_start(&self) -> CpuDescriptorHandle {
        let mut handle = CpuDescriptorHandle::default();
        // SAFETY: a live heap; the method writes the handle it returns.
        unsafe { (self.vtbl().get_cpu_descriptor_handle_for_heap_start)(raw(self), &mut handle) };
        handle
    }

    /// `ID3D12DescriptorHeap::GetGPUDescriptorHandleForHeapStart()`
    pub(crate) fn gpu_descriptor_handle_for_heap_start(&self) -> GpuDescriptorHandle {
        let mut handle = GpuDescriptorHandle::default();
        // SAFETY: as above.
        unsafe { (self.vtbl().get_gpu_descriptor_handle_for_heap_start)(raw(self), &mut handle) };
        handle
    }
}

impl D3d12Resource {
    /// `ID3D12Resource::Map()` of the whole first subresource.
    pub(crate) fn map(&self) -> Result<*mut u8, HRESULT> {
        let mut data: *mut c_void = null_mut();
        // SAFETY: a live resource; no read range (the whole resource).
        check(unsafe { (self.vtbl().map)(raw(self), 0, null(), &mut data) })?;
        Ok(data.cast())
    }

    /// `ID3D12Resource::Map()` of the first subresource with an empty read
    /// range (the CPU doesn't read it).
    pub(crate) fn map_write_only(&self) -> Result<*mut u8, HRESULT> {
        let mut data: *mut c_void = null_mut();
        let range = D3d12Range { begin: 0, end: 0 };
        // SAFETY: a live resource and a read range.
        check(unsafe { (self.vtbl().map)(raw(self), 0, &range, &mut data) })?;
        Ok(data.cast())
    }

    /// `ID3D12Resource::Unmap()` of the first subresource, with the whole
    /// resource written.
    pub(crate) fn unmap(&self) {
        // SAFETY: a live, mapped resource.
        unsafe { (self.vtbl().unmap)(raw(self), 0, null()) }
    }

    /// `ID3D12Resource::GetGPUVirtualAddress()`
    pub(crate) fn gpu_virtual_address(&self) -> u64 {
        // SAFETY: a live resource.
        unsafe { (self.vtbl().get_gpu_virtual_address)(raw(self)) }
    }
}

impl D3dBlob {
    /// The blob's bytes (`GetBufferPointer()`, `GetBufferSize()`).
    pub(crate) fn bytes(&self) -> &[u8] {
        // SAFETY: a live blob owns its buffer of the size it reports for
        // as long as it lives.
        unsafe {
            let ptr = (self.vtbl().get_buffer_pointer)(raw(self));
            let size = (self.vtbl().get_buffer_size)(raw(self));
            if ptr.is_null() {
                &[]
            } else {
                std::slice::from_raw_parts(ptr.cast(), size)
            }
        }
    }
}

impl D3d12Debug {
    /// `ID3D12Debug::EnableDebugLayer()`
    pub(crate) fn enable_debug_layer(&self) {
        // SAFETY: a live debug interface.
        unsafe { (self.vtbl().enable_debug_layer)(raw(self)) }
    }
}

impl D3d12InfoQueue {
    /// `ID3D12InfoQueue::PushStorageFilter()`
    pub(crate) fn push_storage_filter(&self, filter: &InfoQueueFilter) -> HRESULT {
        // SAFETY: a live queue; the filter's lists outlive the call.
        unsafe { (self.vtbl().push_storage_filter)(raw(self), filter) }
    }

    /// `ID3D12InfoQueue::SetBreakOnSeverity()`
    pub(crate) fn set_break_on_severity(&self, severity: u32, enable: bool) -> HRESULT {
        // SAFETY: a live queue.
        unsafe { (self.vtbl().set_break_on_severity)(raw(self), severity, enable as BOOL) }
    }

    /// `ID3D12InfoQueue::GetNumStoredMessages()`
    pub(crate) fn num_stored_messages(&self) -> u64 {
        // SAFETY: a live queue.
        unsafe { (self.vtbl().get_num_stored_messages)(raw(self)) }
    }

    /// Call `f` with the stored message `index` (`GetMessage()` twice: for
    /// its size, then into a buffer of that size). Nothing happens if
    /// getting it fails.
    pub(crate) fn with_message(&self, index: u64, f: impl FnOnce(&Message)) {
        let get_message = self.vtbl().get_message;
        let mut size = 0usize;
        // SAFETY: a live queue; a NULL message asks for its size.
        unsafe { get_message(raw(self), index, null_mut(), &mut size) };
        if size < size_of::<Message>() {
            return;
        }
        // A buffer aligned for the message, whose text follows it.
        let mut buffer = vec![0u64; size.div_ceil(size_of::<u64>())];
        let message = buffer.as_mut_ptr().cast::<Message>();
        // SAFETY: the buffer holds `size` bytes, aligned for a Message.
        if check(unsafe { get_message(raw(self), index, message, &mut size) }).is_ok() {
            // SAFETY: GetMessage filled the message in, and its description
            // points into the buffer.
            f(unsafe { &*message });
        }
    }

    /// `ID3D12InfoQueue::ClearStoredMessages()`
    pub(crate) fn clear_stored_messages(&self) {
        // SAFETY: a live queue.
        unsafe { (self.vtbl().clear_stored_messages)(raw(self)) }
    }
}

impl D3d12InfoQueue1 {
    /// `ID3D12InfoQueue1::RegisterMessageCallback()`, without a context:
    /// the callback's cookie.
    pub(crate) fn register_message_callback(
        &self,
        callback: D3d12MessageFunc,
        flags: u32,
    ) -> Result<u32, HRESULT> {
        let mut cookie = 0u32;
        // SAFETY: a live queue and a callback that lives forever.
        check(unsafe {
            (self.vtbl().register_message_callback)(
                raw(self),
                callback,
                flags,
                null_mut(),
                &mut cookie,
            )
        })?;
        Ok(cookie)
    }
}

impl D3d12SdkConfiguration1 {
    /// `ID3D12SDKConfiguration1::CreateDeviceFactory()`
    pub(crate) fn create_device_factory(
        &self,
        sdk_version: u32,
        sdk_path: &std::ffi::CStr,
    ) -> Result<D3d12DeviceFactory, HRESULT> {
        let f = self.vtbl().create_device_factory;
        // SAFETY: a live configuration and a NUL-terminated path; the call
        // stores an owned ID3D12DeviceFactory.
        unsafe {
            out(|o| {
                f(
                    raw(self),
                    sdk_version,
                    sdk_path.as_ptr(),
                    &IID_ID3D12DEVICEFACTORY,
                    o,
                )
            })
        }
    }
}

impl D3d12DeviceFactory {
    /// `ID3D12DeviceFactory::SetFlags()`
    pub(crate) fn set_flags(&self, flags: u32) -> HRESULT {
        // SAFETY: a live factory.
        unsafe { (self.vtbl().set_flags)(raw(self), flags) }
    }

    /// `ID3D12DeviceFactory::GetConfigurationInterface()` of the debug
    /// layer.
    pub(crate) fn debug_interface(&self) -> Result<D3d12Debug, HRESULT> {
        let f = self.vtbl().get_configuration_interface;
        // SAFETY: a live factory; the call stores an owned ID3D12Debug, the
        // interface asked for.
        unsafe { out(|o| f(raw(self), &CLSID_ID3D12DEBUG, &IID_ID3D12DEBUG, o)) }
    }

    /// `ID3D12DeviceFactory::CreateDevice()`
    pub(crate) fn create_device(
        &self,
        adapter: &DxgiAdapter1,
        minimum_feature_level: D3dFeatureLevel,
    ) -> Result<D3d12Device, HRESULT> {
        let f = self.vtbl().create_device;
        // SAFETY: a live factory and adapter; the call stores an owned
        // ID3D12Device.
        unsafe {
            out(|o| {
                f(
                    raw(self),
                    raw(adapter),
                    minimum_feature_level,
                    &IID_ID3D12DEVICE,
                    o,
                )
            })
        }
    }
}

impl D3d12Device {
    /// `ID3D12Device::CreateCommandAllocator()`
    pub(crate) fn create_command_allocator(
        &self,
        ty: u32,
    ) -> Result<D3d12CommandAllocator, HRESULT> {
        let f = self.vtbl().create_command_allocator;
        // SAFETY: a live device; the call stores an owned
        // ID3D12CommandAllocator, the interface asked for.
        unsafe { out(|o| f(raw(self), ty, &IID_ID3D12COMMANDALLOCATOR, o)) }
    }

    /// `ID3D12Device::CreateCommandList()` on an allocator, without an
    /// initial pipeline state.
    pub(crate) fn create_command_list(
        &self,
        node_mask: u32,
        ty: u32,
        command_allocator: &D3d12CommandAllocator,
    ) -> Result<D3d12GraphicsCommandList, HRESULT> {
        let f = self.vtbl().create_command_list;
        // SAFETY: a live device and allocator; the call stores an owned
        // ID3D12GraphicsCommandList, the interface asked for.
        unsafe {
            out(|o| {
                f(
                    raw(self),
                    node_mask,
                    ty,
                    raw(command_allocator),
                    null_mut(),
                    &IID_ID3D12GRAPHICSCOMMANDLIST,
                    o,
                )
            })
        }
    }

    /// `ID3D12Device::CopyDescriptorsSimple()`
    ///
    /// # Safety
    ///
    /// The handles must be descriptors of live heaps of `heap_type`, with
    /// `num_descriptors` descriptors from each.
    pub(crate) unsafe fn copy_descriptors_simple(
        &self,
        num_descriptors: u32,
        dest_descriptor_range_start: CpuDescriptorHandle,
        src_descriptor_range_start: CpuDescriptorHandle,
        heap_type: u32,
    ) {
        // SAFETY: the caller's contract.
        unsafe {
            (self.vtbl().copy_descriptors_simple)(
                raw(self),
                num_descriptors,
                dest_descriptor_range_start,
                src_descriptor_range_start,
                heap_type,
            )
        }
    }

    /// `ID3D12Device::CreateCommandList()` on an allocator, without an
    /// initial pipeline state, as an `ID3D12GraphicsCommandList2` (the
    /// renderer's; its methods are `ID3D12GraphicsCommandList`'s).
    pub(crate) fn create_command_list2(
        &self,
        node_mask: u32,
        ty: u32,
        command_allocator: &D3d12CommandAllocator,
    ) -> Result<D3d12GraphicsCommandList, HRESULT> {
        let f = self.vtbl().create_command_list;
        // SAFETY: a live device and allocator; the call stores an owned
        // ID3D12GraphicsCommandList2, whose vtable starts with
        // ID3D12GraphicsCommandList's.
        unsafe {
            out(|o| {
                f(
                    raw(self),
                    node_mask,
                    ty,
                    raw(command_allocator),
                    null_mut(),
                    &IID_ID3D12GRAPHICSCOMMANDLIST2,
                    o,
                )
            })
        }
    }

    /// `ID3D12Device::GetCopyableFootprints()` of one subresource of a
    /// resource: its layout, number of rows and size of a row, and the
    /// total size.
    pub(crate) fn copyable_footprints(
        &self,
        desc: &ResourceDesc,
        subresource: u32,
        base_offset: u64,
    ) -> (PlacedSubresourceFootprint, u32, u64, u64) {
        let mut layout = PlacedSubresourceFootprint::default();
        let (mut num_rows, mut row_size_in_bytes, mut total_bytes) = (0u32, 0u64, 0u64);
        // SAFETY: a live device and description; the outputs hold one
        // subresource's values.
        unsafe {
            (self.vtbl().get_copyable_footprints)(
                raw(self),
                desc,
                subresource,
                1,
                base_offset,
                &mut layout,
                &mut num_rows,
                &mut row_size_in_bytes,
                &mut total_bytes,
            )
        };
        (layout, num_rows, row_size_in_bytes, total_bytes)
    }

    /// `ID3D12Device::CreateFence()`
    pub(crate) fn create_fence(
        &self,
        initial_value: u64,
        flags: u32,
    ) -> Result<D3d12Fence, HRESULT> {
        let f = self.vtbl().create_fence;
        // SAFETY: a live device; the call stores an owned ID3D12Fence.
        unsafe { out(|o| f(raw(self), initial_value, flags, &IID_ID3D12FENCE, o)) }
    }
}

impl D3d12Resource {
    /// `ID3D12Resource::GetDesc()`
    pub(crate) fn desc(&self) -> ResourceDesc {
        let mut desc = ResourceDesc::default();
        // SAFETY: a live resource; the method writes the description it
        // returns.
        unsafe { (self.vtbl().get_desc)(raw(self), &mut desc) };
        desc
    }
}

impl D3d12CommandQueue {
    /// `ID3D12CommandQueue::ExecuteCommandLists()` of one list.
    pub(crate) fn execute_command_list(&self, command_list: &D3d12CommandList) {
        let list = raw(command_list);
        // SAFETY: a live queue and a closed command list.
        unsafe { (self.vtbl().execute_command_lists)(raw(self), 1, &list) }
    }

    /// `ID3D12CommandQueue::ExecuteCommandLists()` of one graphics command
    /// list (which is an `ID3D12CommandList`, as the C code casts it).
    pub(crate) fn execute_graphics_command_list(&self, command_list: &D3d12GraphicsCommandList) {
        let list = raw(command_list);
        // SAFETY: a live queue and a closed command list, whose interface
        // pointer is also its ID3D12CommandList's.
        unsafe { (self.vtbl().execute_command_lists)(raw(self), 1, &list) }
    }

    /// `ID3D12CommandQueue::Signal()`
    pub(crate) fn signal(&self, fence: &D3d12Fence, value: u64) -> HRESULT {
        // SAFETY: a live queue and fence.
        unsafe { (self.vtbl().signal)(raw(self), raw(fence), value) }
    }
}

impl D3d12CommandAllocator {
    /// `ID3D12CommandAllocator::Reset()`
    pub(crate) fn reset(&self) -> HRESULT {
        // SAFETY: a live allocator whose command lists the GPU is done with.
        unsafe { (self.vtbl().reset)(raw(self)) }
    }
}

impl D3d12Fence {
    /// `ID3D12Fence::GetCompletedValue()`
    pub(crate) fn completed_value(&self) -> u64 {
        // SAFETY: a live fence.
        unsafe { (self.vtbl().get_completed_value)(raw(self)) }
    }

    /// `ID3D12Fence::SetEventOnCompletion()`
    pub(crate) fn set_event_on_completion(&self, value: u64, event: HANDLE) -> HRESULT {
        // SAFETY: a live fence and an event handle (or NULL).
        unsafe { (self.vtbl().set_event_on_completion)(raw(self), value, event) }
    }

    /// `ID3D12Fence::Signal()`
    pub(crate) fn signal(&self, value: u64) -> HRESULT {
        // SAFETY: a live fence.
        unsafe { (self.vtbl().signal)(raw(self), value) }
    }
}

/// The methods of `ID3D12GraphicsCommandList` SDL records with. The
/// resources, descriptors and objects they name must be live, which the
/// command buffers' tracking ensures until the GPU is done with them.
impl D3d12GraphicsCommandList {
    /// `Close()`
    pub(crate) fn close(&self) -> HRESULT {
        // SAFETY: a live command list being recorded.
        unsafe { (self.vtbl().close)(raw(self)) }
    }

    /// `Reset()`, without an initial pipeline state.
    pub(crate) fn reset(&self, allocator: &D3d12CommandAllocator) -> HRESULT {
        // SAFETY: a closed command list and a reset allocator.
        unsafe { (self.vtbl().reset)(raw(self), raw(allocator), null_mut()) }
    }

    /// `DrawInstanced()`
    pub(crate) fn draw_instanced(&self, vertices: u32, instances: u32, first: u32, base: u32) {
        // SAFETY: a command list being recorded.
        unsafe { (self.vtbl().draw_instanced)(raw(self), vertices, instances, first, base) }
    }

    /// `DrawIndexedInstanced()`
    pub(crate) fn draw_indexed_instanced(
        &self,
        indices: u32,
        instances: u32,
        first_index: u32,
        base_vertex: i32,
        first_instance: u32,
    ) {
        // SAFETY: a command list being recorded.
        unsafe {
            (self.vtbl().draw_indexed_instanced)(
                raw(self),
                indices,
                instances,
                first_index,
                base_vertex,
                first_instance,
            )
        }
    }

    /// `Dispatch()`
    pub(crate) fn dispatch(&self, x: u32, y: u32, z: u32) {
        // SAFETY: a command list being recorded.
        unsafe { (self.vtbl().dispatch)(raw(self), x, y, z) }
    }

    /// `CopyBufferRegion()`
    pub(crate) fn copy_buffer_region(
        &self,
        dst: &D3d12Resource,
        dst_offset: u64,
        src: &D3d12Resource,
        src_offset: u64,
        num_bytes: u64,
    ) {
        // SAFETY: a command list being recorded and live buffers.
        unsafe {
            (self.vtbl().copy_buffer_region)(
                raw(self),
                raw(dst),
                dst_offset,
                raw(src),
                src_offset,
                num_bytes,
            )
        }
    }

    /// `CopyTextureRegion()`
    pub(crate) fn copy_texture_region(
        &self,
        dst: &TextureCopyLocation,
        dst_x: u32,
        dst_y: u32,
        dst_z: u32,
        src: &TextureCopyLocation,
        src_box: Option<&D3d12Box>,
    ) {
        let src_box = src_box.map_or(null(), |b| b as *const D3d12Box);
        // SAFETY: a command list being recorded; the locations name live
        // resources.
        unsafe {
            (self.vtbl().copy_texture_region)(raw(self), dst, dst_x, dst_y, dst_z, src, src_box)
        }
    }

    /// `ResolveSubresource()`
    pub(crate) fn resolve_subresource(
        &self,
        dst: &D3d12Resource,
        dst_subresource: u32,
        src: &D3d12Resource,
        src_subresource: u32,
        format: DxgiFormat,
    ) {
        // SAFETY: a command list being recorded and live textures.
        unsafe {
            (self.vtbl().resolve_subresource)(
                raw(self),
                raw(dst),
                dst_subresource,
                raw(src),
                src_subresource,
                format,
            )
        }
    }

    /// `IASetPrimitiveTopology()`
    pub(crate) fn ia_set_primitive_topology(&self, topology: u32) {
        // SAFETY: a command list being recorded.
        unsafe { (self.vtbl().ia_set_primitive_topology)(raw(self), topology) }
    }

    /// `RSSetViewports()` of one viewport.
    pub(crate) fn rs_set_viewport(&self, viewport: &D3d12Viewport) {
        // SAFETY: a command list being recorded.
        unsafe { (self.vtbl().rs_set_viewports)(raw(self), 1, viewport) }
    }

    /// `RSSetScissorRects()` of one rectangle.
    pub(crate) fn rs_set_scissor_rect(&self, rect: &D3d12Rect) {
        // SAFETY: a command list being recorded.
        unsafe { (self.vtbl().rs_set_scissor_rects)(raw(self), 1, rect) }
    }

    /// `OMSetBlendFactor()`
    pub(crate) fn om_set_blend_factor(&self, blend_factor: &[f32; 4]) {
        // SAFETY: a command list being recorded and four floats.
        unsafe { (self.vtbl().om_set_blend_factor)(raw(self), blend_factor.as_ptr()) }
    }

    /// `OMSetStencilRef()`
    pub(crate) fn om_set_stencil_ref(&self, stencil_ref: u32) {
        // SAFETY: a command list being recorded.
        unsafe { (self.vtbl().om_set_stencil_ref)(raw(self), stencil_ref) }
    }

    /// `SetPipelineState()`
    pub(crate) fn set_pipeline_state(&self, pipeline_state: &D3d12PipelineState) {
        // SAFETY: a command list being recorded and a live pipeline state.
        unsafe { (self.vtbl().set_pipeline_state)(raw(self), raw(pipeline_state)) }
    }

    /// `ResourceBarrier()`
    pub(crate) fn resource_barrier(&self, barriers: &[ResourceBarrier]) {
        // SAFETY: a command list being recorded; the barriers name live
        // resources.
        unsafe {
            (self.vtbl().resource_barrier)(raw(self), barriers.len() as u32, barriers.as_ptr())
        }
    }

    /// `SetDescriptorHeaps()`
    pub(crate) fn set_descriptor_heaps(&self, heaps: &[&D3d12DescriptorHeap]) {
        let heaps: Vec<*mut c_void> = heaps.iter().map(|h| raw(*h)).collect();
        // SAFETY: a command list being recorded and live shader-visible
        // heaps.
        unsafe { (self.vtbl().set_descriptor_heaps)(raw(self), heaps.len() as u32, heaps.as_ptr()) }
    }

    /// `SetComputeRootSignature()`
    pub(crate) fn set_compute_root_signature(&self, root_signature: &D3d12RootSignature) {
        // SAFETY: a command list being recorded and a live root signature.
        unsafe { (self.vtbl().set_compute_root_signature)(raw(self), raw(root_signature)) }
    }

    /// `SetGraphicsRootSignature()`
    pub(crate) fn set_graphics_root_signature(&self, root_signature: &D3d12RootSignature) {
        // SAFETY: a command list being recorded and a live root signature.
        unsafe { (self.vtbl().set_graphics_root_signature)(raw(self), raw(root_signature)) }
    }

    /// `SetComputeRootDescriptorTable()`
    pub(crate) fn set_compute_root_descriptor_table(&self, index: i32, base: GpuDescriptorHandle) {
        // SAFETY: a command list being recorded; the descriptors are in the
        // shader-visible heaps set.
        unsafe { (self.vtbl().set_compute_root_descriptor_table)(raw(self), index as u32, base) }
    }

    /// `SetGraphicsRootDescriptorTable()`
    pub(crate) fn set_graphics_root_descriptor_table(&self, index: i32, base: GpuDescriptorHandle) {
        // SAFETY: as for set_compute_root_descriptor_table().
        unsafe { (self.vtbl().set_graphics_root_descriptor_table)(raw(self), index as u32, base) }
    }

    /// `SetComputeRootConstantBufferView()`
    pub(crate) fn set_compute_root_constant_buffer_view(&self, index: i32, address: u64) {
        // SAFETY: a command list being recorded; the address is in a live
        // buffer.
        unsafe {
            (self.vtbl().set_compute_root_constant_buffer_view)(raw(self), index as u32, address)
        }
    }

    /// `SetGraphicsRootConstantBufferView()`
    pub(crate) fn set_graphics_root_constant_buffer_view(&self, index: i32, address: u64) {
        // SAFETY: as for set_compute_root_constant_buffer_view().
        unsafe {
            (self.vtbl().set_graphics_root_constant_buffer_view)(raw(self), index as u32, address)
        }
    }

    /// `SetGraphicsRoot32BitConstants()`
    pub(crate) fn set_graphics_root_32bit_constants(
        &self,
        root_parameter_index: u32,
        src_data: &[u32],
        dest_offset_in_32bit_values: u32,
    ) {
        // SAFETY: a command list being recorded and `src_data`'s values.
        unsafe {
            (self.vtbl().set_graphics_root_32bit_constants)(
                raw(self),
                root_parameter_index,
                src_data.len() as u32,
                src_data.as_ptr().cast(),
                dest_offset_in_32bit_values,
            )
        }
    }

    /// `IASetIndexBuffer()`
    pub(crate) fn ia_set_index_buffer(&self, view: &IndexBufferView) {
        // SAFETY: a command list being recorded; the view is of a live
        // buffer.
        unsafe { (self.vtbl().ia_set_index_buffer)(raw(self), view) }
    }

    /// `IASetVertexBuffers()`
    pub(crate) fn ia_set_vertex_buffers(&self, start_slot: u32, views: &[VertexBufferView]) {
        // SAFETY: a command list being recorded; the views are of live
        // buffers.
        unsafe {
            (self.vtbl().ia_set_vertex_buffers)(
                raw(self),
                start_slot,
                views.len() as u32,
                views.as_ptr(),
            )
        }
    }

    /// `OMSetRenderTargets()`, the targets not being a single range.
    pub(crate) fn om_set_render_targets(
        &self,
        render_targets: &[CpuDescriptorHandle],
        depth_stencil: Option<&CpuDescriptorHandle>,
    ) {
        let depth_stencil = depth_stencil.map_or(null(), |d| d as *const CpuDescriptorHandle);
        let targets = if render_targets.is_empty() {
            null()
        } else {
            render_targets.as_ptr()
        };
        // SAFETY: a command list being recorded and descriptors of live
        // views.
        unsafe {
            (self.vtbl().om_set_render_targets)(
                raw(self),
                render_targets.len() as u32,
                targets,
                0,
                depth_stencil,
            )
        }
    }

    /// `ClearDepthStencilView()` of the whole view.
    pub(crate) fn clear_depth_stencil_view(
        &self,
        view: CpuDescriptorHandle,
        flags: u32,
        depth: f32,
        stencil: u8,
    ) {
        // SAFETY: a command list being recorded and a descriptor of a live
        // view.
        unsafe {
            (self.vtbl().clear_depth_stencil_view)(
                raw(self),
                view,
                flags,
                depth,
                stencil,
                0,
                null(),
            )
        }
    }

    /// `ClearRenderTargetView()` of the whole view.
    pub(crate) fn clear_render_target_view(&self, view: CpuDescriptorHandle, color: &[f32; 4]) {
        // SAFETY: a command list being recorded and a descriptor of a live
        // view.
        unsafe {
            (self.vtbl().clear_render_target_view)(raw(self), view, color.as_ptr(), 0, null())
        }
    }

    /// `ExecuteIndirect()` without a count buffer.
    pub(crate) fn execute_indirect(
        &self,
        command_signature: &D3d12CommandSignature,
        max_command_count: u32,
        argument_buffer: &D3d12Resource,
        argument_buffer_offset: u64,
    ) {
        // SAFETY: a command list being recorded, a live signature and
        // buffer.
        unsafe {
            (self.vtbl().execute_indirect)(
                raw(self),
                raw(command_signature),
                max_command_count,
                raw(argument_buffer),
                argument_buffer_offset,
                null_mut(),
                0,
            )
        }
    }

    /// The command list as an `ID3D12CommandList` (`QueryInterface()`).
    pub(crate) fn command_list(&self) -> Result<D3d12CommandList, HRESULT> {
        self.query(&IID_ID3D12COMMANDLIST)
    }

    /// The interface pointer, for the PIX runtime's functions.
    pub(crate) fn as_raw(&self) -> *mut c_void {
        raw(self)
    }
}

/// `IDXGIFactory2::CreateSwapChainForHwnd()` on a command queue, as an
/// `IDXGISwapChain1` (whose `IUnknown` is all SDL needs before asking for
/// its `IDXGISwapChain3`).
pub(crate) fn create_swap_chain_for_hwnd(
    factory: &DxgiFactory4,
    command_queue: &D3d12CommandQueue,
    hwnd: HWND,
    desc: &SwapChainDesc1,
    fullscreen_desc: &SwapChainFullscreenDesc,
) -> Result<ComPtr<OpaqueVtbl>, HRESULT> {
    // SAFETY: a live factory, whose vtable starts with IDXGIFactory2's.
    unsafe {
        create_swap_chain_for_hwnd_on(
            &factory.vtbl().factory2,
            raw(factory),
            command_queue,
            hwnd,
            desc,
            Some(fullscreen_desc),
        )
    }
}

/// `IDXGIFactory2::CreateSwapChainForHwnd()` on a command queue, for all
/// outputs (no output to restrict to).
///
/// # Safety
///
/// `factory` must be a live factory whose vtable starts with `vtbl`.
unsafe fn create_swap_chain_for_hwnd_on(
    vtbl: &IDXGIFactory2Vtbl,
    factory: *mut c_void,
    command_queue: &D3d12CommandQueue,
    hwnd: HWND,
    desc: &SwapChainDesc1,
    fullscreen_desc: Option<&SwapChainFullscreenDesc>,
) -> Result<ComPtr<OpaqueVtbl>, HRESULT> {
    let f = vtbl.create_swap_chain_for_hwnd;
    let fullscreen_desc = fullscreen_desc.map_or(null(), |d| d as *const SwapChainFullscreenDesc);
    // SAFETY: the caller's contract; a live queue; the descriptions (whose
    // layouts are DXGI's) outlive the call; the swap chain is stored owned.
    unsafe {
        out(|o| {
            f(
                factory,
                raw(command_queue),
                hwnd,
                (desc as *const SwapChainDesc1).cast(),
                fullscreen_desc.cast(),
                null_mut(),
                o,
            )
        })
    }
}

impl DxgiFactory6 {
    /// `IDXGIFactory2::CreateSwapChainForHwnd()` on a command queue,
    /// without a fullscreen description (the Direct3D 12 renderer's), as an
    /// `IDXGISwapChain1`.
    pub(crate) fn create_swap_chain_for_hwnd(
        &self,
        command_queue: &D3d12CommandQueue,
        hwnd: HWND,
        desc: &SwapChainDesc1,
    ) -> Result<ComPtr<OpaqueVtbl>, HRESULT> {
        // SAFETY: a live factory, whose vtable starts with IDXGIFactory2's.
        unsafe {
            create_swap_chain_for_hwnd_on(
                &self.vtbl().factory5.factory4.factory2,
                raw(self),
                command_queue,
                hwnd,
                desc,
                None,
            )
        }
    }

    /// `IDXGIFactory::MakeWindowAssociation()`
    pub(crate) fn make_window_association(&self, hwnd: HWND, flags: u32) -> HRESULT {
        let f = self
            .vtbl()
            .factory5
            .factory4
            .factory2
            .factory1
            .make_window_association;
        // SAFETY: a live factory and a window handle.
        unsafe { f(raw(self), hwnd, flags) }
    }

    /// `IDXGIFactory6::EnumAdapterByGpuPreference()`, for an
    /// `IDXGIAdapter4` (whose methods the renderer doesn't call).
    pub(crate) fn enum_adapter4_by_gpu_preference(
        &self,
        adapter: u32,
        gpu_preference: DxgiGpuPreference,
    ) -> Result<ComPtr<OpaqueVtbl>, HRESULT> {
        let f = self.vtbl().enum_adapter_by_gpu_preference;
        // SAFETY: a live factory; the call stores an owned IDXGIAdapter4,
        // the interface asked for.
        unsafe { out(|o| f(raw(self), adapter, gpu_preference, &IID_IDXGIADAPTER4, o)) }
    }
}

impl DxgiSwapChain3 {
    /// `IDXGIObject::GetParent()` of an `IDXGIFactory1`.
    pub(crate) fn parent_factory(&self) -> Result<DxgiFactory1, HRESULT> {
        let f = self.vtbl().get_parent;
        // SAFETY: a live swap chain; the call stores an owned
        // IDXGIFactory1, the interface asked for.
        unsafe { out(|o| f(raw(self), &IID_IDXGIFACTORY1, o)) }
    }

    /// `IDXGISwapChain::Present()`
    pub(crate) fn present(&self, sync_interval: u32, flags: u32) -> HRESULT {
        // SAFETY: a live swap chain.
        unsafe { (self.vtbl().present)(raw(self), sync_interval, flags) }
    }

    /// `IDXGISwapChain::GetBuffer()` of an `ID3D12Resource`.
    pub(crate) fn buffer(&self, index: u32) -> Result<D3d12Resource, HRESULT> {
        let f = self.vtbl().get_buffer;
        // SAFETY: a live swap chain; the call stores an owned
        // ID3D12Resource, the interface asked for.
        unsafe { out(|o| f(raw(self), index, &IID_ID3D12RESOURCE, o)) }
    }

    /// `IDXGISwapChain::ResizeBuffers()`
    pub(crate) fn resize_buffers(
        &self,
        buffer_count: u32,
        width: u32,
        height: u32,
        format: DxgiFormat,
        flags: u32,
    ) -> HRESULT {
        // SAFETY: a live swap chain.
        unsafe {
            (self.vtbl().resize_buffers)(raw(self), buffer_count, width, height, format, flags)
        }
    }

    /// `IDXGISwapChain1::GetDesc1()` (whose result upstream doesn't check).
    pub(crate) fn desc1(&self) -> SwapChainDesc1 {
        let mut desc = SwapChainDesc1::default();
        // SAFETY: a live swap chain; a valid output.
        unsafe { (self.vtbl().get_desc1)(raw(self), &mut desc) };
        desc
    }

    /// `IDXGISwapChain1::SetRotation()`
    pub(crate) fn set_rotation(&self, rotation: u32) -> HRESULT {
        // SAFETY: a live swap chain.
        unsafe { (self.vtbl().set_rotation)(raw(self), rotation) }
    }

    /// `IDXGISwapChain2::SetMaximumFrameLatency()`
    pub(crate) fn set_maximum_frame_latency(&self, max_latency: u32) -> HRESULT {
        // SAFETY: a live swap chain made with
        // DXGI_SWAP_CHAIN_FLAG_FRAME_LATENCY_WAITABLE_OBJECT.
        unsafe { (self.vtbl().set_maximum_frame_latency)(raw(self), max_latency) }
    }

    /// `IDXGISwapChain3::GetCurrentBackBufferIndex()`
    pub(crate) fn current_back_buffer_index(&self) -> u32 {
        // SAFETY: a live swap chain.
        unsafe { (self.vtbl().get_current_back_buffer_index)(raw(self)) }
    }

    /// `IDXGISwapChain3::CheckColorSpaceSupport()`: the support flags (0
    /// when the call fails; upstream reads its uninitialized variable then).
    pub(crate) fn check_color_space_support(&self, color_space: DxgiColorSpaceType) -> u32 {
        let mut support = 0;
        // SAFETY: a live swap chain; a valid output.
        unsafe { (self.vtbl().check_color_space_support)(raw(self), color_space, &mut support) };
        support
    }

    /// `IDXGISwapChain3::SetColorSpace1()`
    pub(crate) fn set_color_space1(&self, color_space: DxgiColorSpaceType) -> HRESULT {
        // SAFETY: a live swap chain.
        unsafe { (self.vtbl().set_color_space1)(raw(self), color_space) }
    }
}

/// `IDXGIFactory::MakeWindowAssociation()` of a swap chain's parent.
pub(crate) fn make_window_association(factory: &DxgiFactory1, hwnd: HWND, flags: u32) -> HRESULT {
    // SAFETY: a live factory and a window handle.
    unsafe { (factory.vtbl().make_window_association)(raw(factory), hwnd, flags) }
}
