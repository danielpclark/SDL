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
    DxgiFormat, SampleDesc, Slot, IID_IDXGIFACTORY1,
};

// --- enumerants ---

// D3D12_DESCRIPTOR_HEAP_TYPE
pub(super) const D3D12_DESCRIPTOR_HEAP_TYPE_CBV_SRV_UAV: u32 = 0;
pub(super) const D3D12_DESCRIPTOR_HEAP_TYPE_SAMPLER: u32 = 1;
pub(super) const D3D12_DESCRIPTOR_HEAP_TYPE_RTV: u32 = 2;
pub(super) const D3D12_DESCRIPTOR_HEAP_TYPE_DSV: u32 = 3;
pub(super) const D3D12_DESCRIPTOR_HEAP_TYPE_NUM_TYPES: u32 = 4;

// D3D12_DESCRIPTOR_HEAP_FLAGS
pub(super) const D3D12_DESCRIPTOR_HEAP_FLAG_NONE: u32 = 0;
pub(super) const D3D12_DESCRIPTOR_HEAP_FLAG_SHADER_VISIBLE: u32 = 0x1;

// D3D12_BLEND
pub(super) const D3D12_BLEND_ZERO: u32 = 1;
pub(super) const D3D12_BLEND_ONE: u32 = 2;
pub(super) const D3D12_BLEND_SRC_COLOR: u32 = 3;
pub(super) const D3D12_BLEND_INV_SRC_COLOR: u32 = 4;
pub(super) const D3D12_BLEND_SRC_ALPHA: u32 = 5;
pub(super) const D3D12_BLEND_INV_SRC_ALPHA: u32 = 6;
pub(super) const D3D12_BLEND_DEST_ALPHA: u32 = 7;
pub(super) const D3D12_BLEND_INV_DEST_ALPHA: u32 = 8;
pub(super) const D3D12_BLEND_DEST_COLOR: u32 = 9;
pub(super) const D3D12_BLEND_INV_DEST_COLOR: u32 = 10;
pub(super) const D3D12_BLEND_SRC_ALPHA_SAT: u32 = 11;
pub(super) const D3D12_BLEND_BLEND_FACTOR: u32 = 14;
pub(super) const D3D12_BLEND_INV_BLEND_FACTOR: u32 = 15;

// D3D12_BLEND_OP
pub(super) const D3D12_BLEND_OP_ADD: u32 = 1;
pub(super) const D3D12_BLEND_OP_SUBTRACT: u32 = 2;
pub(super) const D3D12_BLEND_OP_REV_SUBTRACT: u32 = 3;
pub(super) const D3D12_BLEND_OP_MIN: u32 = 4;
pub(super) const D3D12_BLEND_OP_MAX: u32 = 5;

// D3D12_LOGIC_OP
pub(super) const D3D12_LOGIC_OP_NOOP: u32 = 4;

// D3D12_COMPARISON_FUNC
pub(super) const D3D12_COMPARISON_FUNC_NEVER: u32 = 1;
pub(super) const D3D12_COMPARISON_FUNC_LESS: u32 = 2;
pub(super) const D3D12_COMPARISON_FUNC_EQUAL: u32 = 3;
pub(super) const D3D12_COMPARISON_FUNC_LESS_EQUAL: u32 = 4;
pub(super) const D3D12_COMPARISON_FUNC_GREATER: u32 = 5;
pub(super) const D3D12_COMPARISON_FUNC_NOT_EQUAL: u32 = 6;
pub(super) const D3D12_COMPARISON_FUNC_GREATER_EQUAL: u32 = 7;
pub(super) const D3D12_COMPARISON_FUNC_ALWAYS: u32 = 8;

// D3D12_STENCIL_OP
pub(super) const D3D12_STENCIL_OP_KEEP: u32 = 1;
pub(super) const D3D12_STENCIL_OP_ZERO: u32 = 2;
pub(super) const D3D12_STENCIL_OP_REPLACE: u32 = 3;
pub(super) const D3D12_STENCIL_OP_INCR_SAT: u32 = 4;
pub(super) const D3D12_STENCIL_OP_DECR_SAT: u32 = 5;
pub(super) const D3D12_STENCIL_OP_INVERT: u32 = 6;
pub(super) const D3D12_STENCIL_OP_INCR: u32 = 7;
pub(super) const D3D12_STENCIL_OP_DECR: u32 = 8;

// D3D12_CULL_MODE, D3D12_FILL_MODE
pub(super) const D3D12_CULL_MODE_NONE: u32 = 1;
pub(super) const D3D12_CULL_MODE_FRONT: u32 = 2;
pub(super) const D3D12_CULL_MODE_BACK: u32 = 3;
pub(super) const D3D12_FILL_MODE_WIREFRAME: u32 = 2;
pub(super) const D3D12_FILL_MODE_SOLID: u32 = 3;

// D3D12_DEPTH_WRITE_MASK, D3D12_CONSERVATIVE_RASTERIZATION_MODE
pub(super) const D3D12_DEPTH_WRITE_MASK_ZERO: u32 = 0;
pub(super) const D3D12_DEPTH_WRITE_MASK_ALL: u32 = 1;
pub(super) const D3D12_CONSERVATIVE_RASTERIZATION_MODE_OFF: u32 = 0;

// D3D12_INPUT_CLASSIFICATION
pub(super) const D3D12_INPUT_CLASSIFICATION_PER_VERTEX_DATA: u32 = 0;
pub(super) const D3D12_INPUT_CLASSIFICATION_PER_INSTANCE_DATA: u32 = 1;

// D3D_PRIMITIVE_TOPOLOGY
pub(super) const D3D_PRIMITIVE_TOPOLOGY_POINTLIST: u32 = 1;
pub(super) const D3D_PRIMITIVE_TOPOLOGY_LINELIST: u32 = 2;
pub(super) const D3D_PRIMITIVE_TOPOLOGY_LINESTRIP: u32 = 3;
pub(super) const D3D_PRIMITIVE_TOPOLOGY_TRIANGLELIST: u32 = 4;
pub(super) const D3D_PRIMITIVE_TOPOLOGY_TRIANGLESTRIP: u32 = 5;

// D3D12_PRIMITIVE_TOPOLOGY_TYPE
pub(super) const D3D12_PRIMITIVE_TOPOLOGY_TYPE_POINT: u32 = 1;
pub(super) const D3D12_PRIMITIVE_TOPOLOGY_TYPE_LINE: u32 = 2;
pub(super) const D3D12_PRIMITIVE_TOPOLOGY_TYPE_TRIANGLE: u32 = 3;

// D3D12_TEXTURE_ADDRESS_MODE
pub(super) const D3D12_TEXTURE_ADDRESS_MODE_WRAP: u32 = 1;
pub(super) const D3D12_TEXTURE_ADDRESS_MODE_MIRROR: u32 = 2;
pub(super) const D3D12_TEXTURE_ADDRESS_MODE_CLAMP: u32 = 3;

// D3D12_FILTER (D3D12_ENCODE_BASIC_FILTER)
pub(super) const D3D12_FILTER_REDUCTION_TYPE_MASK: u32 = 0x3;
pub(super) const D3D12_FILTER_REDUCTION_TYPE_SHIFT: u32 = 7;
pub(super) const D3D12_FILTER_TYPE_MASK: u32 = 0x3;
pub(super) const D3D12_MIN_FILTER_SHIFT: u32 = 4;
pub(super) const D3D12_MAG_FILTER_SHIFT: u32 = 2;
pub(super) const D3D12_MIP_FILTER_SHIFT: u32 = 0;
pub(super) const D3D12_ANISOTROPIC_FILTERING_BIT: u32 = 0x40;

// D3D12_RESOURCE_STATES
pub(super) const D3D12_RESOURCE_STATE_COMMON: u32 = 0;
pub(super) const D3D12_RESOURCE_STATE_VERTEX_AND_CONSTANT_BUFFER: u32 = 0x1;
pub(super) const D3D12_RESOURCE_STATE_INDEX_BUFFER: u32 = 0x2;
pub(super) const D3D12_RESOURCE_STATE_RENDER_TARGET: u32 = 0x4;
pub(super) const D3D12_RESOURCE_STATE_UNORDERED_ACCESS: u32 = 0x8;
pub(super) const D3D12_RESOURCE_STATE_DEPTH_WRITE: u32 = 0x10;
pub(super) const D3D12_RESOURCE_STATE_NON_PIXEL_SHADER_RESOURCE: u32 = 0x40;
pub(super) const D3D12_RESOURCE_STATE_INDIRECT_ARGUMENT: u32 = 0x200;
pub(super) const D3D12_RESOURCE_STATE_COPY_DEST: u32 = 0x400;
pub(super) const D3D12_RESOURCE_STATE_COPY_SOURCE: u32 = 0x800;
pub(super) const D3D12_RESOURCE_STATE_RESOLVE_DEST: u32 = 0x1000;
pub(super) const D3D12_RESOURCE_STATE_RESOLVE_SOURCE: u32 = 0x2000;
pub(super) const D3D12_RESOURCE_STATE_GENERIC_READ: u32 = 0xac3;
pub(super) const D3D12_RESOURCE_STATE_ALL_SHADER_RESOURCE: u32 = 0xc0;
pub(super) const D3D12_RESOURCE_STATE_PRESENT: u32 = 0;

// D3D12_HEAP_TYPE, D3D12_CPU_PAGE_PROPERTY, D3D12_MEMORY_POOL
pub(super) const D3D12_HEAP_TYPE_DEFAULT: u32 = 1;
pub(super) const D3D12_HEAP_TYPE_UPLOAD: u32 = 2;
pub(super) const D3D12_HEAP_TYPE_READBACK: u32 = 3;
pub(super) const D3D12_HEAP_TYPE_GPU_UPLOAD: u32 = 5;
pub(super) const D3D12_CPU_PAGE_PROPERTY_UNKNOWN: u32 = 0;
pub(super) const D3D12_MEMORY_POOL_UNKNOWN: u32 = 0;

// D3D12_HEAP_FLAGS
pub(super) const D3D12_HEAP_FLAG_NONE: u32 = 0;
pub(super) const D3D12_HEAP_FLAG_ALLOW_DISPLAY: u32 = 0x8;

// D3D12_RESOURCE_DIMENSION, D3D12_TEXTURE_LAYOUT
pub(super) const D3D12_RESOURCE_DIMENSION_BUFFER: u32 = 1;
pub(super) const D3D12_RESOURCE_DIMENSION_TEXTURE2D: u32 = 3;
pub(super) const D3D12_RESOURCE_DIMENSION_TEXTURE3D: u32 = 4;
pub(super) const D3D12_TEXTURE_LAYOUT_UNKNOWN: u32 = 0;
pub(super) const D3D12_TEXTURE_LAYOUT_ROW_MAJOR: u32 = 1;

// D3D12_RESOURCE_FLAGS
pub(super) const D3D12_RESOURCE_FLAG_ALLOW_RENDER_TARGET: u32 = 0x1;
pub(super) const D3D12_RESOURCE_FLAG_ALLOW_DEPTH_STENCIL: u32 = 0x2;
pub(super) const D3D12_RESOURCE_FLAG_ALLOW_UNORDERED_ACCESS: u32 = 0x4;

pub(super) const D3D12_DEFAULT_RESOURCE_PLACEMENT_ALIGNMENT: u64 = 65536;
pub(super) const D3D12_DEFAULT_MSAA_RESOURCE_PLACEMENT_ALIGNMENT: u64 = 4194304;
pub(super) const D3D12_STANDARD_MULTISAMPLE_PATTERN: u32 = 0xffffffff;
pub(super) const D3D12_DEFAULT_SHADER_4_COMPONENT_MAPPING: u32 = 0x1688;
pub(super) const D3D12_IA_VERTEX_INPUT_STRUCTURE_ELEMENT_COUNT: usize = 32;

// D3D12_SRV_DIMENSION
pub(super) const D3D12_SRV_DIMENSION_BUFFER: u32 = 1;
pub(super) const D3D12_SRV_DIMENSION_TEXTURE2D: u32 = 4;
pub(super) const D3D12_SRV_DIMENSION_TEXTURE2DARRAY: u32 = 5;
pub(super) const D3D12_SRV_DIMENSION_TEXTURE2DMS: u32 = 6;
pub(super) const D3D12_SRV_DIMENSION_TEXTURE3D: u32 = 8;
pub(super) const D3D12_SRV_DIMENSION_TEXTURECUBE: u32 = 9;
pub(super) const D3D12_SRV_DIMENSION_TEXTURECUBEARRAY: u32 = 10;

// D3D12_RTV_DIMENSION, D3D12_DSV_DIMENSION, D3D12_UAV_DIMENSION
pub(super) const D3D12_RTV_DIMENSION_TEXTURE2D: u32 = 4;
pub(super) const D3D12_RTV_DIMENSION_TEXTURE2DARRAY: u32 = 5;
pub(super) const D3D12_RTV_DIMENSION_TEXTURE2DMS: u32 = 6;
pub(super) const D3D12_RTV_DIMENSION_TEXTURE3D: u32 = 8;
pub(super) const D3D12_DSV_DIMENSION_TEXTURE2D: u32 = 3;
pub(super) const D3D12_DSV_DIMENSION_TEXTURE2DARRAY: u32 = 4;
pub(super) const D3D12_DSV_DIMENSION_TEXTURE2DMS: u32 = 5;
pub(super) const D3D12_UAV_DIMENSION_BUFFER: u32 = 1;
pub(super) const D3D12_UAV_DIMENSION_TEXTURE2D: u32 = 4;
pub(super) const D3D12_UAV_DIMENSION_TEXTURE2DARRAY: u32 = 5;
pub(super) const D3D12_UAV_DIMENSION_TEXTURE3D: u32 = 8;

// D3D12_BUFFER_UAV_FLAGS, D3D12_BUFFER_SRV_FLAGS
pub(super) const D3D12_BUFFER_UAV_FLAG_RAW: u32 = 0x1;
pub(super) const D3D12_BUFFER_SRV_FLAG_RAW: u32 = 0x1;

// D3D12_DESCRIPTOR_RANGE_TYPE
pub(super) const D3D12_DESCRIPTOR_RANGE_TYPE_SRV: u32 = 0;
pub(super) const D3D12_DESCRIPTOR_RANGE_TYPE_UAV: u32 = 1;
pub(super) const D3D12_DESCRIPTOR_RANGE_TYPE_SAMPLER: u32 = 3;
pub(super) const D3D12_DESCRIPTOR_RANGE_OFFSET_APPEND: u32 = 0xffffffff;

// D3D12_ROOT_PARAMETER_TYPE, D3D12_SHADER_VISIBILITY
pub(super) const D3D12_ROOT_PARAMETER_TYPE_DESCRIPTOR_TABLE: u32 = 0;
pub(super) const D3D12_ROOT_PARAMETER_TYPE_CBV: u32 = 2;
pub(super) const D3D12_SHADER_VISIBILITY_ALL: u32 = 0;
pub(super) const D3D12_SHADER_VISIBILITY_VERTEX: u32 = 1;
pub(super) const D3D12_SHADER_VISIBILITY_PIXEL: u32 = 5;

// D3D12_ROOT_SIGNATURE_FLAGS, D3D_ROOT_SIGNATURE_VERSION
pub(super) const D3D12_ROOT_SIGNATURE_FLAG_NONE: u32 = 0;
pub(super) const D3D12_ROOT_SIGNATURE_FLAG_ALLOW_INPUT_ASSEMBLER_INPUT_LAYOUT: u32 = 0x1;
pub(super) const D3D_ROOT_SIGNATURE_VERSION_1: u32 = 0x1;

// D3D12_PIPELINE_STATE_FLAGS
pub(super) const D3D12_PIPELINE_STATE_FLAG_NONE: u32 = 0;

// D3D12_FEATURE
pub(super) const D3D12_FEATURE_D3D12_OPTIONS: u32 = 0;
pub(super) const D3D12_FEATURE_FORMAT_SUPPORT: u32 = 3;
pub(super) const D3D12_FEATURE_MULTISAMPLE_QUALITY_LEVELS: u32 = 4;
pub(super) const D3D12_FEATURE_SHADER_MODEL: u32 = 7;
pub(super) const D3D12_FEATURE_D3D12_OPTIONS13: u32 = 42;
pub(super) const D3D12_FEATURE_D3D12_OPTIONS16: u32 = 45;

// D3D12_FORMAT_SUPPORT1, D3D12_FORMAT_SUPPORT2
pub(super) const D3D12_FORMAT_SUPPORT1_NONE: u32 = 0;
pub(super) const D3D12_FORMAT_SUPPORT1_TEXTURE2D: u32 = 0x20;
pub(super) const D3D12_FORMAT_SUPPORT1_TEXTURE3D: u32 = 0x40;
pub(super) const D3D12_FORMAT_SUPPORT1_TEXTURECUBE: u32 = 0x80;
pub(super) const D3D12_FORMAT_SUPPORT1_SHADER_LOAD: u32 = 0x100;
pub(super) const D3D12_FORMAT_SUPPORT1_SHADER_SAMPLE: u32 = 0x200;
pub(super) const D3D12_FORMAT_SUPPORT1_RENDER_TARGET: u32 = 0x4000;
pub(super) const D3D12_FORMAT_SUPPORT1_DEPTH_STENCIL: u32 = 0x10000;
pub(super) const D3D12_FORMAT_SUPPORT2_NONE: u32 = 0;
pub(super) const D3D12_FORMAT_SUPPORT2_UAV_TYPED_LOAD: u32 = 0x40;
pub(super) const D3D12_FORMAT_SUPPORT2_UAV_TYPED_STORE: u32 = 0x80;

// D3D_SHADER_MODEL, D3D12_RESOURCE_BINDING_TIER
pub(super) const D3D_SHADER_MODEL_6_0: u32 = 0x60;
pub(super) const D3D12_RESOURCE_BINDING_TIER_2: u32 = 2;

// D3D12_COMMAND_LIST_TYPE, D3D12_COMMAND_QUEUE_FLAGS
pub(super) const D3D12_COMMAND_LIST_TYPE_DIRECT: u32 = 0;
pub(super) const D3D12_COMMAND_QUEUE_FLAG_NONE: u32 = 0;

// D3D12_INDIRECT_ARGUMENT_TYPE
pub(super) const D3D12_INDIRECT_ARGUMENT_TYPE_DRAW: u32 = 0;
pub(super) const D3D12_INDIRECT_ARGUMENT_TYPE_DRAW_INDEXED: u32 = 1;
pub(super) const D3D12_INDIRECT_ARGUMENT_TYPE_DISPATCH: u32 = 2;

// D3D12_MESSAGE_CATEGORY
pub(super) const D3D12_MESSAGE_CATEGORY_APPLICATION_DEFINED: u32 = 0;
pub(super) const D3D12_MESSAGE_CATEGORY_MISCELLANEOUS: u32 = 1;
pub(super) const D3D12_MESSAGE_CATEGORY_INITIALIZATION: u32 = 2;
pub(super) const D3D12_MESSAGE_CATEGORY_CLEANUP: u32 = 3;
pub(super) const D3D12_MESSAGE_CATEGORY_COMPILATION: u32 = 4;
pub(super) const D3D12_MESSAGE_CATEGORY_STATE_CREATION: u32 = 5;
pub(super) const D3D12_MESSAGE_CATEGORY_STATE_SETTING: u32 = 6;
pub(super) const D3D12_MESSAGE_CATEGORY_STATE_GETTING: u32 = 7;
pub(super) const D3D12_MESSAGE_CATEGORY_RESOURCE_MANIPULATION: u32 = 8;
pub(super) const D3D12_MESSAGE_CATEGORY_EXECUTION: u32 = 9;
pub(super) const D3D12_MESSAGE_CATEGORY_SHADER: u32 = 10;

// D3D12_MESSAGE_SEVERITY
pub(super) const D3D12_MESSAGE_SEVERITY_CORRUPTION: u32 = 0;
pub(super) const D3D12_MESSAGE_SEVERITY_ERROR: u32 = 1;
pub(super) const D3D12_MESSAGE_SEVERITY_WARNING: u32 = 2;
pub(super) const D3D12_MESSAGE_SEVERITY_INFO: u32 = 3;
pub(super) const D3D12_MESSAGE_SEVERITY_MESSAGE: u32 = 4;

// D3D12_MESSAGE_CALLBACK_FLAGS, D3D12_DEVICE_FACTORY_FLAGS
pub(super) const D3D12_MESSAGE_CALLBACK_FLAG_NONE: u32 = 0;
pub(super) const D3D12_DEVICE_FACTORY_FLAG_ALLOW_RETURNING_EXISTING_DEVICE: u32 = 0x1;

// D3D12_RESOURCE_BARRIER_TYPE, D3D12_RESOURCE_BARRIER_FLAGS
pub(super) const D3D12_RESOURCE_BARRIER_TYPE_TRANSITION: u32 = 0;
pub(super) const D3D12_RESOURCE_BARRIER_TYPE_UAV: u32 = 2;
pub(super) const D3D12_RESOURCE_BARRIER_FLAG_NONE: u32 = 0;

// D3D12_TEXTURE_COPY_TYPE, D3D12_CLEAR_FLAGS, D3D12_FENCE_FLAGS
pub(super) const D3D12_TEXTURE_COPY_TYPE_SUBRESOURCE_INDEX: u32 = 0;
pub(super) const D3D12_TEXTURE_COPY_TYPE_PLACED_FOOTPRINT: u32 = 1;
pub(super) const D3D12_CLEAR_FLAG_DEPTH: u32 = 0x1;
pub(super) const D3D12_CLEAR_FLAG_STENCIL: u32 = 0x2;
pub(super) const D3D12_FENCE_FLAG_NONE: u32 = 0;

// The copy alignments
pub(super) const D3D12_TEXTURE_DATA_PITCH_ALIGNMENT: u32 = 256;
pub(super) const D3D12_TEXTURE_DATA_PLACEMENT_ALIGNMENT: u32 = 512;

// D3D12_FORMAT_SUPPORT1 (for swapchains)
pub(super) const D3D12_FORMAT_SUPPORT1_DISPLAY: u32 = 0x80000;

// DXGI (the swapchains')
pub(super) const DXGI_SWAP_EFFECT_FLIP_DISCARD: u32 = 4;
pub(super) const DXGI_SCALING_NONE: u32 = 1;
pub(super) const DXGI_ALPHA_MODE_UNSPECIFIED: u32 = 0;
pub(super) const DXGI_USAGE_RENDER_TARGET_OUTPUT: u32 = 0x20;
pub(super) const DXGI_SWAP_CHAIN_FLAG_ALLOW_TEARING: u32 = 2048;
pub(super) const DXGI_PRESENT_ALLOW_TEARING: u32 = 0x200;
pub(super) const DXGI_MWA_NO_WINDOW_CHANGES: u32 = 1;
pub(super) const DXGI_SWAP_CHAIN_COLOR_SPACE_SUPPORT_FLAG_PRESENT: u32 = 1;
pub(super) const DXGI_MODE_SCANLINE_ORDER_UNSPECIFIED: u32 = 0;
pub(super) const DXGI_MODE_SCALING_UNSPECIFIED: u32 = 0;

// --- GUIDs (upstream defines them so as not to need uuid.lib) ---

pub(super) const IID_ID3D12DEVICE: GUID = GUID::from_u128(0x189819f1_1db6_4b57_be54_1821339b85f7);
pub(super) const IID_ID3D12COMMANDQUEUE: GUID =
    GUID::from_u128(0x0ec870a6_5d7e_4c22_8cfc_5baae07616ed);
pub(super) const IID_ID3D12DESCRIPTORHEAP: GUID =
    GUID::from_u128(0x8efb471d_616c_4f49_90f7_127bb763fa51);
pub(super) const IID_ID3D12RESOURCE: GUID = GUID::from_u128(0x696442be_a72e_4059_bc79_5b5c98040fad);
pub(super) const IID_ID3D12ROOTSIGNATURE: GUID =
    GUID::from_u128(0xc54a6b66_72df_4ee8_8be5_a946a1429214);
pub(super) const IID_ID3D12COMMANDSIGNATURE: GUID =
    GUID::from_u128(0xc36a797c_ec80_4f0a_8985_a7b2475082d1);
pub(super) const IID_ID3D12PIPELINESTATE: GUID =
    GUID::from_u128(0x765a30f3_f624_4c6f_a828_ace948622445);
pub(super) const IID_ID3D12DEBUG: GUID = GUID::from_u128(0x344488b7_6846_474b_b989_f027448245e0);
pub(super) const IID_ID3D12INFOQUEUE: GUID =
    GUID::from_u128(0x0742a90b_c387_483f_b946_30a7e4e61458);
pub(super) const IID_ID3D12INFOQUEUE1: GUID =
    GUID::from_u128(0x2852dd88_b484_4c0c_b6b1_67168500e600);

pub(super) const CLSID_ID3D12SDKCONFIGURATION: GUID =
    GUID::from_u128(0x7cda6aca_a03e_49c8_9458_0334d20e07ce);
pub(super) const CLSID_ID3D12DEBUG: GUID = GUID::from_u128(0xf2352aeb_dd84_49fe_b97b_a9dcfdcc1b4f);
pub(super) const IID_ID3D12SDKCONFIGURATION: GUID =
    GUID::from_u128(0xe9eb5314_33aa_42b2_a718_d77f58b1f1c7);
pub(super) const IID_ID3D12SDKCONFIGURATION1: GUID =
    GUID::from_u128(0x8aaf9303_ad25_48b9_9a57_d9c37e009d9f);
pub(super) const IID_ID3D12DEVICEFACTORY: GUID =
    GUID::from_u128(0x61f307d3_d34e_4e7c_8374_3ba4de23cccb);
pub(super) const IID_ID3D12COMMANDALLOCATOR: GUID =
    GUID::from_u128(0x6102dee4_af59_4b09_b999_b44d73f09b24);
pub(super) const IID_ID3D12COMMANDLIST: GUID =
    GUID::from_u128(0x7116d91c_e7e4_47ce_b8c6_ec8168f437e5);
pub(super) const IID_ID3D12GRAPHICSCOMMANDLIST: GUID =
    GUID::from_u128(0x5b160d0f_ac1b_4185_8ba8_b3ae42a5a455);
pub(super) const IID_ID3D12FENCE: GUID = GUID::from_u128(0x0a753dcf_c4d8_4b91_adf6_be5a60d95a76);
pub(super) const IID_IDXGISWAPCHAIN3: GUID =
    GUID::from_u128(0x94d99bdb_f1f8_4ab0_b236_7da0170edab1);

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
pub(super) struct CpuDescriptorHandle {
    pub(super) ptr: usize,
}

/// `D3D12_GPU_DESCRIPTOR_HANDLE`
#[repr(C)]
#[derive(Clone, Copy, Default, Debug, PartialEq, Eq)]
pub(super) struct GpuDescriptorHandle {
    pub(super) ptr: u64,
}

/// `D3D12_DESCRIPTOR_HEAP_DESC`
#[repr(C)]
#[derive(Clone, Copy, Default, Debug)]
pub(super) struct DescriptorHeapDesc {
    pub(super) ty: u32,
    pub(super) num_descriptors: u32,
    pub(super) flags: u32,
    pub(super) node_mask: u32,
}

/// `D3D12_COMMAND_QUEUE_DESC`
#[repr(C)]
#[derive(Clone, Copy, Default, Debug)]
pub(super) struct CommandQueueDesc {
    pub(super) ty: u32,
    pub(super) priority: i32,
    pub(super) flags: u32,
    pub(super) node_mask: u32,
}

/// `D3D12_HEAP_PROPERTIES`
#[repr(C)]
#[derive(Clone, Copy, Default, Debug)]
pub(super) struct HeapProperties {
    pub(super) ty: u32,
    pub(super) cpu_page_property: u32,
    pub(super) memory_pool_preference: u32,
    pub(super) creation_node_mask: u32,
    pub(super) visible_node_mask: u32,
}

/// `D3D12_RESOURCE_DESC`
#[repr(C)]
#[derive(Clone, Copy, Default, Debug)]
pub(super) struct ResourceDesc {
    pub(super) dimension: u32,
    pub(super) alignment: u64,
    pub(super) width: u64,
    pub(super) height: u32,
    pub(super) depth_or_array_size: u16,
    pub(super) mip_levels: u16,
    pub(super) format: DxgiFormat,
    pub(super) sample_desc: SampleDesc,
    pub(super) layout: u32,
    pub(super) flags: u32,
}

/// `D3D12_DEPTH_STENCIL_VALUE`
#[repr(C)]
#[derive(Clone, Copy, Default, Debug)]
pub(super) struct DepthStencilValue {
    pub(super) depth: f32,
    pub(super) stencil: u8,
}

/// The union of `D3D12_CLEAR_VALUE`.
#[repr(C)]
#[derive(Clone, Copy)]
pub(super) union ClearValueUnion {
    pub(super) color: [f32; 4],
    pub(super) depth_stencil: DepthStencilValue,
}

/// `D3D12_CLEAR_VALUE`
#[repr(C)]
#[derive(Clone, Copy)]
pub(super) struct ClearValue {
    pub(super) format: DxgiFormat,
    pub(super) u: ClearValueUnion,
}

/// `D3D12_BUFFER_SRV`
#[repr(C)]
#[derive(Clone, Copy, Default, Debug)]
pub(super) struct BufferSrv {
    pub(super) first_element: u64,
    pub(super) num_elements: u32,
    pub(super) structure_byte_stride: u32,
    pub(super) flags: u32,
}

/// `D3D12_TEX2D_SRV`
#[repr(C)]
#[derive(Clone, Copy, Default, Debug)]
pub(super) struct Tex2dSrv {
    pub(super) most_detailed_mip: u32,
    pub(super) mip_levels: u32,
    pub(super) plane_slice: u32,
    pub(super) resource_min_lod_clamp: f32,
}

/// `D3D12_TEX2D_ARRAY_SRV`
#[repr(C)]
#[derive(Clone, Copy, Default, Debug)]
pub(super) struct Tex2dArraySrv {
    pub(super) most_detailed_mip: u32,
    pub(super) mip_levels: u32,
    pub(super) first_array_slice: u32,
    pub(super) array_size: u32,
    pub(super) plane_slice: u32,
    pub(super) resource_min_lod_clamp: f32,
}

/// `D3D12_TEX2DMS_SRV`
#[repr(C)]
#[derive(Clone, Copy, Default, Debug)]
pub(super) struct Tex2dmsSrv {
    pub(super) unused_field_nothing_to_define: u32,
}

/// `D3D12_TEX3D_SRV`, `D3D12_TEXCUBE_SRV` (the same members)
#[repr(C)]
#[derive(Clone, Copy, Default, Debug)]
pub(super) struct Tex3dSrv {
    pub(super) most_detailed_mip: u32,
    pub(super) mip_levels: u32,
    pub(super) resource_min_lod_clamp: f32,
}

/// `D3D12_TEXCUBE_ARRAY_SRV`
#[repr(C)]
#[derive(Clone, Copy, Default, Debug)]
pub(super) struct TexCubeArraySrv {
    pub(super) most_detailed_mip: u32,
    pub(super) mip_levels: u32,
    pub(super) first_2d_array_face: u32,
    pub(super) num_cubes: u32,
    pub(super) resource_min_lod_clamp: f32,
}

/// The union of `D3D12_SHADER_RESOURCE_VIEW_DESC` (the members SDL uses).
#[repr(C)]
#[derive(Clone, Copy)]
pub(super) union SrvUnion {
    pub(super) buffer: BufferSrv,
    pub(super) texture_2d: Tex2dSrv,
    pub(super) texture_2d_array: Tex2dArraySrv,
    pub(super) texture_2dms: Tex2dmsSrv,
    pub(super) texture_3d: Tex3dSrv,
    pub(super) texture_cube: Tex3dSrv,
    pub(super) texture_cube_array: TexCubeArraySrv,
}

/// `D3D12_SHADER_RESOURCE_VIEW_DESC`
#[repr(C)]
#[derive(Clone, Copy)]
pub(super) struct ShaderResourceViewDesc {
    pub(super) format: DxgiFormat,
    pub(super) view_dimension: u32,
    pub(super) shader_4_component_mapping: u32,
    pub(super) u: SrvUnion,
}

/// `D3D12_TEX2D_RTV`, `D3D12_TEX2D_UAV` (the same members)
#[repr(C)]
#[derive(Clone, Copy, Default, Debug)]
pub(super) struct Tex2dRtv {
    pub(super) mip_slice: u32,
    pub(super) plane_slice: u32,
}

/// `D3D12_TEX2D_ARRAY_RTV`, `D3D12_TEX2D_ARRAY_UAV` (the same members)
#[repr(C)]
#[derive(Clone, Copy, Default, Debug)]
pub(super) struct Tex2dArrayRtv {
    pub(super) mip_slice: u32,
    pub(super) first_array_slice: u32,
    pub(super) array_size: u32,
    pub(super) plane_slice: u32,
}

/// `D3D12_TEX3D_RTV`, `D3D12_TEX3D_UAV` (the same members)
#[repr(C)]
#[derive(Clone, Copy, Default, Debug)]
pub(super) struct Tex3dRtv {
    pub(super) mip_slice: u32,
    pub(super) first_w_slice: u32,
    pub(super) w_size: u32,
}

/// `D3D12_BUFFER_RTV` (the largest member of the union)
#[repr(C)]
#[derive(Clone, Copy, Default, Debug)]
pub(super) struct BufferRtv {
    pub(super) first_element: u64,
    pub(super) num_elements: u32,
}

/// The union of `D3D12_RENDER_TARGET_VIEW_DESC` (the members SDL uses).
#[repr(C)]
#[derive(Clone, Copy)]
pub(super) union RtvUnion {
    pub(super) buffer: BufferRtv,
    pub(super) texture_2d: Tex2dRtv,
    pub(super) texture_2d_array: Tex2dArrayRtv,
    pub(super) texture_3d: Tex3dRtv,
}

/// `D3D12_RENDER_TARGET_VIEW_DESC`
#[repr(C)]
#[derive(Clone, Copy)]
pub(super) struct RenderTargetViewDesc {
    pub(super) format: DxgiFormat,
    pub(super) view_dimension: u32,
    pub(super) u: RtvUnion,
}

/// `D3D12_TEX2D_DSV`
#[repr(C)]
#[derive(Clone, Copy, Default, Debug)]
pub(super) struct Tex2dDsv {
    pub(super) mip_slice: u32,
}

/// `D3D12_TEX2D_ARRAY_DSV`
#[repr(C)]
#[derive(Clone, Copy, Default, Debug)]
pub(super) struct Tex2dArrayDsv {
    pub(super) mip_slice: u32,
    pub(super) first_array_slice: u32,
    pub(super) array_size: u32,
}

/// The union of `D3D12_DEPTH_STENCIL_VIEW_DESC` (the members SDL uses;
/// the others are no larger).
#[repr(C)]
#[derive(Clone, Copy)]
pub(super) union DsvUnion {
    pub(super) texture_2d: Tex2dDsv,
    pub(super) texture_2d_array: Tex2dArrayDsv,
}

/// `D3D12_DEPTH_STENCIL_VIEW_DESC`
#[repr(C)]
#[derive(Clone, Copy)]
pub(super) struct DepthStencilViewDesc {
    pub(super) format: DxgiFormat,
    pub(super) view_dimension: u32,
    pub(super) flags: u32,
    pub(super) u: DsvUnion,
}

/// `D3D12_BUFFER_UAV`
#[repr(C)]
#[derive(Clone, Copy, Default, Debug)]
pub(super) struct BufferUav {
    pub(super) first_element: u64,
    pub(super) num_elements: u32,
    pub(super) structure_byte_stride: u32,
    pub(super) counter_offset_in_bytes: u64,
    pub(super) flags: u32,
}

/// The union of `D3D12_UNORDERED_ACCESS_VIEW_DESC` (the members SDL uses).
#[repr(C)]
#[derive(Clone, Copy)]
pub(super) union UavUnion {
    pub(super) buffer: BufferUav,
    pub(super) texture_2d: Tex2dRtv,
    pub(super) texture_2d_array: Tex2dArrayRtv,
    pub(super) texture_3d: Tex3dRtv,
}

/// `D3D12_UNORDERED_ACCESS_VIEW_DESC`
#[repr(C)]
#[derive(Clone, Copy)]
pub(super) struct UnorderedAccessViewDesc {
    pub(super) format: DxgiFormat,
    pub(super) view_dimension: u32,
    pub(super) u: UavUnion,
}

/// `D3D12_SAMPLER_DESC`
#[repr(C)]
#[derive(Clone, Copy, Default, Debug, PartialEq)]
pub(super) struct SamplerDesc {
    pub(super) filter: u32,
    pub(super) address_u: u32,
    pub(super) address_v: u32,
    pub(super) address_w: u32,
    pub(super) mip_lod_bias: f32,
    pub(super) max_anisotropy: u32,
    pub(super) comparison_func: u32,
    pub(super) border_color: [f32; 4],
    pub(super) min_lod: f32,
    pub(super) max_lod: f32,
}

/// `D3D12_DESCRIPTOR_RANGE`
#[repr(C)]
#[derive(Clone, Copy, Default, Debug, PartialEq, Eq)]
pub(super) struct DescriptorRange {
    pub(super) range_type: u32,
    pub(super) num_descriptors: u32,
    pub(super) base_shader_register: u32,
    pub(super) register_space: u32,
    pub(super) offset_in_descriptors_from_table_start: u32,
}

/// `D3D12_ROOT_DESCRIPTOR_TABLE`
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub(super) struct RootDescriptorTable {
    pub(super) num_descriptor_ranges: u32,
    pub(super) descriptor_ranges: *const DescriptorRange,
}

/// `D3D12_ROOT_DESCRIPTOR`
#[repr(C)]
#[derive(Clone, Copy, Default, Debug, PartialEq, Eq)]
pub(super) struct RootDescriptor {
    pub(super) shader_register: u32,
    pub(super) register_space: u32,
}

/// The union of `D3D12_ROOT_PARAMETER` (the members SDL uses; the
/// `Constants` member is no larger).
#[repr(C)]
#[derive(Clone, Copy)]
pub(super) union RootParameterUnion {
    pub(super) descriptor_table: RootDescriptorTable,
    pub(super) descriptor: RootDescriptor,
}

/// `D3D12_ROOT_PARAMETER`
#[repr(C)]
#[derive(Clone, Copy)]
pub(super) struct RootParameter {
    pub(super) parameter_type: u32,
    pub(super) u: RootParameterUnion,
    pub(super) shader_visibility: u32,
}

/// `D3D12_ROOT_SIGNATURE_DESC` (without static samplers: SDL has none)
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub(super) struct RootSignatureDesc {
    pub(super) num_parameters: u32,
    pub(super) parameters: *const RootParameter,
    pub(super) num_static_samplers: u32,
    pub(super) static_samplers: *const c_void,
    pub(super) flags: u32,
}

/// `D3D12_SHADER_BYTECODE`
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub(super) struct ShaderBytecode {
    pub(super) shader_bytecode: *const c_void,
    pub(super) bytecode_length: usize,
}

impl ShaderBytecode {
    /// The bytecode of a shader, borrowed for the call it is passed to.
    pub(super) fn new(code: &[u8]) -> ShaderBytecode {
        ShaderBytecode {
            shader_bytecode: code.as_ptr().cast(),
            bytecode_length: code.len(),
        }
    }
}

/// `D3D12_STREAM_OUTPUT_DESC` (SDL leaves it zeroed)
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub(super) struct StreamOutputDesc {
    pub(super) so_declaration: *const c_void,
    pub(super) num_entries: u32,
    pub(super) buffer_strides: *const u32,
    pub(super) num_strides: u32,
    pub(super) rasterized_stream: u32,
}

/// `D3D12_RENDER_TARGET_BLEND_DESC`
#[repr(C)]
#[derive(Clone, Copy, Default, Debug, PartialEq, Eq)]
pub(super) struct RenderTargetBlendDesc {
    pub(super) blend_enable: BOOL,
    pub(super) logic_op_enable: BOOL,
    pub(super) src_blend: u32,
    pub(super) dest_blend: u32,
    pub(super) blend_op: u32,
    pub(super) src_blend_alpha: u32,
    pub(super) dest_blend_alpha: u32,
    pub(super) blend_op_alpha: u32,
    pub(super) logic_op: u32,
    pub(super) render_target_write_mask: u8,
}

/// `D3D12_BLEND_DESC`
#[repr(C)]
#[derive(Clone, Copy, Default, Debug, PartialEq, Eq)]
pub(super) struct BlendDesc {
    pub(super) alpha_to_coverage_enable: BOOL,
    pub(super) independent_blend_enable: BOOL,
    pub(super) render_target: [RenderTargetBlendDesc; 8],
}

/// `D3D12_RASTERIZER_DESC`
#[repr(C)]
#[derive(Clone, Copy, Default, Debug, PartialEq)]
pub(super) struct RasterizerDesc {
    pub(super) fill_mode: u32,
    pub(super) cull_mode: u32,
    pub(super) front_counter_clockwise: BOOL,
    pub(super) depth_bias: i32,
    pub(super) depth_bias_clamp: f32,
    pub(super) slope_scaled_depth_bias: f32,
    pub(super) depth_clip_enable: BOOL,
    pub(super) multisample_enable: BOOL,
    pub(super) antialiased_line_enable: BOOL,
    pub(super) forced_sample_count: u32,
    pub(super) conservative_raster: u32,
}

/// `D3D12_DEPTH_STENCILOP_DESC`
#[repr(C)]
#[derive(Clone, Copy, Default, Debug, PartialEq, Eq)]
pub(super) struct DepthStencilOpDesc {
    pub(super) stencil_fail_op: u32,
    pub(super) stencil_depth_fail_op: u32,
    pub(super) stencil_pass_op: u32,
    pub(super) stencil_func: u32,
}

/// `D3D12_DEPTH_STENCIL_DESC`
#[repr(C)]
#[derive(Clone, Copy, Default, Debug, PartialEq, Eq)]
pub(super) struct DepthStencilDesc {
    pub(super) depth_enable: BOOL,
    pub(super) depth_write_mask: u32,
    pub(super) depth_func: u32,
    pub(super) stencil_enable: BOOL,
    pub(super) stencil_read_mask: u8,
    pub(super) stencil_write_mask: u8,
    pub(super) front_face: DepthStencilOpDesc,
    pub(super) back_face: DepthStencilOpDesc,
}

/// `D3D12_INPUT_ELEMENT_DESC`
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub(super) struct InputElementDesc {
    pub(super) semantic_name: *const c_char,
    pub(super) semantic_index: u32,
    pub(super) format: DxgiFormat,
    pub(super) input_slot: u32,
    pub(super) aligned_byte_offset: u32,
    pub(super) input_slot_class: u32,
    pub(super) instance_data_step_rate: u32,
}

/// `D3D12_INPUT_LAYOUT_DESC`
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub(super) struct InputLayoutDesc {
    pub(super) input_element_descs: *const InputElementDesc,
    pub(super) num_elements: u32,
}

/// `D3D12_CACHED_PIPELINE_STATE` (SDL leaves it zeroed)
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub(super) struct CachedPipelineState {
    pub(super) cached_blob: *const c_void,
    pub(super) cached_blob_size_in_bytes: usize,
}

/// `D3D12_GRAPHICS_PIPELINE_STATE_DESC`
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub(super) struct GraphicsPipelineStateDesc {
    pub(super) root_signature: *mut c_void,
    pub(super) vs: ShaderBytecode,
    pub(super) ps: ShaderBytecode,
    pub(super) ds: ShaderBytecode,
    pub(super) hs: ShaderBytecode,
    pub(super) gs: ShaderBytecode,
    pub(super) stream_output: StreamOutputDesc,
    pub(super) blend_state: BlendDesc,
    pub(super) sample_mask: u32,
    pub(super) rasterizer_state: RasterizerDesc,
    pub(super) depth_stencil_state: DepthStencilDesc,
    pub(super) input_layout: InputLayoutDesc,
    pub(super) ib_strip_cut_value: u32,
    pub(super) primitive_topology_type: u32,
    pub(super) num_render_targets: u32,
    pub(super) rtv_formats: [DxgiFormat; 8],
    pub(super) dsv_format: DxgiFormat,
    pub(super) sample_desc: SampleDesc,
    pub(super) node_mask: u32,
    pub(super) cached_pso: CachedPipelineState,
    pub(super) flags: u32,
}

/// `D3D12_COMPUTE_PIPELINE_STATE_DESC`
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub(super) struct ComputePipelineStateDesc {
    pub(super) root_signature: *mut c_void,
    pub(super) cs: ShaderBytecode,
    pub(super) node_mask: u32,
    pub(super) cached_pso: CachedPipelineState,
    pub(super) flags: u32,
}

/// `D3D12_INDIRECT_ARGUMENT_DESC` (the union is three `UINT`s at most; SDL
/// only uses argument types without parameters)
#[repr(C)]
#[derive(Clone, Copy, Default, Debug)]
pub(super) struct IndirectArgumentDesc {
    pub(super) ty: u32,
    _union: [u32; 3],
}

impl IndirectArgumentDesc {
    /// An argument of a type without parameters (`Draw`, `DrawIndexed`,
    /// `Dispatch`).
    pub(super) fn new(ty: u32) -> IndirectArgumentDesc {
        IndirectArgumentDesc { ty, _union: [0; 3] }
    }
}

/// `D3D12_COMMAND_SIGNATURE_DESC`
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub(super) struct CommandSignatureDesc {
    pub(super) byte_stride: u32,
    pub(super) num_argument_descs: u32,
    pub(super) argument_descs: *const IndirectArgumentDesc,
    pub(super) node_mask: u32,
}

/// `D3D12_FEATURE_DATA_FORMAT_SUPPORT`
#[repr(C)]
#[derive(Clone, Copy, Default, Debug)]
pub(super) struct FeatureDataFormatSupport {
    pub(super) format: DxgiFormat,
    pub(super) support1: u32,
    pub(super) support2: u32,
}

/// `D3D12_FEATURE_DATA_MULTISAMPLE_QUALITY_LEVELS`
#[repr(C)]
#[derive(Clone, Copy, Default, Debug)]
pub(super) struct FeatureDataMultisampleQualityLevels {
    pub(super) format: DxgiFormat,
    pub(super) sample_count: u32,
    pub(super) flags: u32,
    pub(super) num_quality_levels: u32,
}

/// `D3D12_FEATURE_DATA_D3D12_OPTIONS`
#[repr(C)]
#[derive(Clone, Copy, Default, Debug)]
pub(super) struct FeatureDataD3d12Options {
    pub(super) double_precision_float_shader_ops: BOOL,
    pub(super) output_merger_logic_op: BOOL,
    pub(super) min_precision_support: u32,
    pub(super) tiled_resources_tier: u32,
    pub(super) resource_binding_tier: u32,
    pub(super) ps_specified_stencil_ref_supported: BOOL,
    pub(super) typed_uav_load_additional_formats: BOOL,
    pub(super) rovs_supported: BOOL,
    pub(super) conservative_rasterization_tier: u32,
    pub(super) max_gpu_virtual_address_bits_per_resource: u32,
    pub(super) standard_swizzle_64kb_supported: BOOL,
    pub(super) cross_node_sharing_tier: u32,
    pub(super) cross_adapter_row_major_texture_supported: BOOL,
    pub(super) vp_and_rt_array_index_from_any_shader_feeding_rasterizer_supported_without_gs_emulation:
        BOOL,
    pub(super) resource_heap_tier: u32,
}

/// `D3D12_FEATURE_DATA_SHADER_MODEL`
#[repr(C)]
#[derive(Clone, Copy, Default, Debug)]
pub(super) struct FeatureDataShaderModel {
    pub(super) highest_shader_model: u32,
}

/// `D3D12_FEATURE_DATA_D3D12_OPTIONS13`
#[repr(C)]
#[derive(Clone, Copy, Default, Debug)]
pub(super) struct FeatureDataD3d12Options13 {
    pub(super) unrestricted_buffer_texture_copy_pitch_supported: BOOL,
    pub(super) unrestricted_vertex_element_alignment_supported: BOOL,
    pub(super) inverted_viewport_height_flips_y_supported: BOOL,
    pub(super) inverted_viewport_depth_flips_z_supported: BOOL,
    pub(super) texture_copy_between_dimensions_supported: BOOL,
    pub(super) alpha_blend_factor_supported: BOOL,
}

/// `D3D12_FEATURE_DATA_D3D12_OPTIONS16`
#[repr(C)]
#[derive(Clone, Copy, Default, Debug)]
pub(super) struct FeatureDataD3d12Options16 {
    pub(super) dynamic_depth_bias_supported: BOOL,
    pub(super) gpu_upload_heap_supported: BOOL,
}

/// `D3D12_MESSAGE`
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub(super) struct Message {
    pub(super) category: u32,
    pub(super) severity: u32,
    pub(super) id: i32,
    pub(super) description: *const c_char,
    pub(super) description_byte_length: usize,
}

/// `D3D12_INFO_QUEUE_FILTER_DESC`
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub(super) struct InfoQueueFilterDesc {
    pub(super) num_categories: u32,
    pub(super) category_list: *const u32,
    pub(super) num_severities: u32,
    pub(super) severity_list: *const u32,
    pub(super) num_ids: u32,
    pub(super) id_list: *const i32,
}

/// `D3D12_INFO_QUEUE_FILTER`
#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
pub(super) struct InfoQueueFilter {
    pub(super) allow_list: InfoQueueFilterDesc,
    pub(super) deny_list: InfoQueueFilterDesc,
}

/// `D3D12_RESOURCE_TRANSITION_BARRIER`
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub(super) struct ResourceTransitionBarrier {
    pub(super) resource: *mut c_void,
    pub(super) subresource: u32,
    pub(super) state_before: u32,
    pub(super) state_after: u32,
}

/// `D3D12_RESOURCE_ALIASING_BARRIER`
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub(super) struct ResourceAliasingBarrier {
    pub(super) resource_before: *mut c_void,
    pub(super) resource_after: *mut c_void,
}

/// `D3D12_RESOURCE_UAV_BARRIER`
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub(super) struct ResourceUavBarrier {
    pub(super) resource: *mut c_void,
}

/// The union of `D3D12_RESOURCE_BARRIER`.
#[repr(C)]
#[derive(Clone, Copy)]
pub(super) union ResourceBarrierUnion {
    pub(super) transition: ResourceTransitionBarrier,
    pub(super) aliasing: ResourceAliasingBarrier,
    pub(super) uav: ResourceUavBarrier,
}

/// `D3D12_RESOURCE_BARRIER`
#[repr(C)]
#[derive(Clone, Copy)]
pub(super) struct ResourceBarrier {
    pub(super) ty: u32,
    pub(super) flags: u32,
    pub(super) u: ResourceBarrierUnion,
}

/// `D3D12_SUBRESOURCE_FOOTPRINT`
#[repr(C)]
#[derive(Clone, Copy, Default, Debug)]
pub(super) struct SubresourceFootprint {
    pub(super) format: DxgiFormat,
    pub(super) width: u32,
    pub(super) height: u32,
    pub(super) depth: u32,
    pub(super) row_pitch: u32,
}

/// `D3D12_PLACED_SUBRESOURCE_FOOTPRINT`
#[repr(C)]
#[derive(Clone, Copy, Default, Debug)]
pub(super) struct PlacedSubresourceFootprint {
    pub(super) offset: u64,
    pub(super) footprint: SubresourceFootprint,
}

/// The union of `D3D12_TEXTURE_COPY_LOCATION`.
#[repr(C)]
#[derive(Clone, Copy)]
pub(super) union TextureCopyLocationUnion {
    pub(super) placed_footprint: PlacedSubresourceFootprint,
    pub(super) subresource_index: u32,
}

/// `D3D12_TEXTURE_COPY_LOCATION`
#[repr(C)]
#[derive(Clone, Copy)]
pub(super) struct TextureCopyLocation {
    pub(super) resource: *mut c_void,
    pub(super) ty: u32,
    pub(super) u: TextureCopyLocationUnion,
}

/// `D3D12_BOX`
#[repr(C)]
#[derive(Clone, Copy, Default, Debug)]
pub(super) struct D3d12Box {
    pub(super) left: u32,
    pub(super) top: u32,
    pub(super) front: u32,
    pub(super) right: u32,
    pub(super) bottom: u32,
    pub(super) back: u32,
}

/// `D3D12_VIEWPORT`
#[repr(C)]
#[derive(Clone, Copy, Default, Debug)]
pub(super) struct D3d12Viewport {
    pub(super) top_left_x: f32,
    pub(super) top_left_y: f32,
    pub(super) width: f32,
    pub(super) height: f32,
    pub(super) min_depth: f32,
    pub(super) max_depth: f32,
}

/// `D3D12_RECT` (`RECT`)
#[repr(C)]
#[derive(Clone, Copy, Default, Debug)]
pub(super) struct D3d12Rect {
    pub(super) left: i32,
    pub(super) top: i32,
    pub(super) right: i32,
    pub(super) bottom: i32,
}

/// `D3D12_VERTEX_BUFFER_VIEW`
#[repr(C)]
#[derive(Clone, Copy, Default, Debug)]
pub(super) struct VertexBufferView {
    pub(super) buffer_location: u64,
    pub(super) size_in_bytes: u32,
    pub(super) stride_in_bytes: u32,
}

/// `D3D12_INDEX_BUFFER_VIEW`
#[repr(C)]
#[derive(Clone, Copy, Default, Debug)]
pub(super) struct IndexBufferView {
    pub(super) buffer_location: u64,
    pub(super) size_in_bytes: u32,
    pub(super) format: DxgiFormat,
}

/// `DXGI_SWAP_CHAIN_DESC1`
#[repr(C)]
#[derive(Clone, Copy, Default, Debug)]
pub(super) struct SwapChainDesc1 {
    pub(super) width: u32,
    pub(super) height: u32,
    pub(super) format: DxgiFormat,
    pub(super) stereo: BOOL,
    pub(super) sample_desc: SampleDesc,
    pub(super) buffer_usage: u32,
    pub(super) buffer_count: u32,
    pub(super) scaling: u32,
    pub(super) swap_effect: u32,
    pub(super) alpha_mode: u32,
    pub(super) flags: u32,
}

/// `DXGI_RATIONAL`
#[repr(C)]
#[derive(Clone, Copy, Default, Debug)]
pub(super) struct DxgiRational {
    pub(super) numerator: u32,
    pub(super) denominator: u32,
}

/// `DXGI_SWAP_CHAIN_FULLSCREEN_DESC`
#[repr(C)]
#[derive(Clone, Copy, Default, Debug)]
pub(super) struct SwapChainFullscreenDesc {
    pub(super) refresh_rate: DxgiRational,
    pub(super) scanline_ordering: u32,
    pub(super) scaling: u32,
    pub(super) windowed: BOOL,
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
pub(super) type PfnD3d12CreateDevice = unsafe extern "system" fn(
    *mut c_void,
    D3dFeatureLevel,
    *const GUID,
    *mut *mut c_void,
) -> HRESULT;
/// `PFN_D3D12_SERIALIZE_ROOT_SIGNATURE`
pub(super) type PfnD3d12SerializeRootSignature = unsafe extern "system" fn(
    *const RootSignatureDesc,
    u32,
    *mut *mut c_void,
    *mut *mut c_void,
) -> HRESULT;
/// `PFN_D3D12_GET_DEBUG_INTERFACE` (and `pfnDXGIGetDebugInterface`, the
/// same signature)
pub(super) type PfnD3d12GetDebugInterface =
    unsafe extern "system" fn(*const GUID, *mut *mut c_void) -> HRESULT;
/// `PFN_D3D12_GET_INTERFACE`
pub(super) type PfnD3d12GetInterface =
    unsafe extern "system" fn(*const GUID, *const GUID, *mut *mut c_void) -> HRESULT;
/// `pfnBeginEventOnCommandList`, `pfnSetMarkerOnCommandList`
pub(super) type PfnBeginEventOnCommandList =
    unsafe extern "system" fn(*mut c_void, u64, *const c_char);
/// `pfnEndEventOnCommandList`
pub(super) type PfnEndEventOnCommandList = unsafe extern "system" fn(*mut c_void);
/// `D3D12MessageFunc`
pub(super) type D3d12MessageFunc =
    unsafe extern "system" fn(u32, u32, i32, *const c_char, *mut c_void);

// --- the COM interfaces SDL calls ---

/// `ID3D12Object`'s methods but `SetName()`: GetPrivateData,
/// SetPrivateData, SetPrivateDataInterface.
type PrivateDataSlots = [Slot; 3];

/// `ID3D12ObjectVtbl`, which every Direct3D 12 object's vtable starts with.
#[repr(C)]
pub(super) struct ID3D12ObjectVtbl {
    pub(super) base: IUnknownVtbl,
    _private_data: PrivateDataSlots,
    pub(super) set_name: unsafe extern "system" fn(*mut c_void, *const u16) -> HRESULT,
}

/// `ID3D12DeviceVtbl` (the part SDL uses).
#[repr(C)]
pub(super) struct ID3D12DeviceVtbl {
    pub(super) object: ID3D12ObjectVtbl,
    _get_node_count: Slot,
    pub(super) create_command_queue: unsafe extern "system" fn(
        *mut c_void,
        *const CommandQueueDesc,
        *const GUID,
        *mut *mut c_void,
    ) -> HRESULT,
    pub(super) create_command_allocator:
        unsafe extern "system" fn(*mut c_void, u32, *const GUID, *mut *mut c_void) -> HRESULT,
    pub(super) create_graphics_pipeline_state: unsafe extern "system" fn(
        *mut c_void,
        *const GraphicsPipelineStateDesc,
        *const GUID,
        *mut *mut c_void,
    ) -> HRESULT,
    pub(super) create_compute_pipeline_state: unsafe extern "system" fn(
        *mut c_void,
        *const ComputePipelineStateDesc,
        *const GUID,
        *mut *mut c_void,
    ) -> HRESULT,
    pub(super) create_command_list: unsafe extern "system" fn(
        *mut c_void,
        u32,
        u32,
        *mut c_void,
        *mut c_void,
        *const GUID,
        *mut *mut c_void,
    ) -> HRESULT,
    pub(super) check_feature_support:
        unsafe extern "system" fn(*mut c_void, u32, *mut c_void, u32) -> HRESULT,
    pub(super) create_descriptor_heap: unsafe extern "system" fn(
        *mut c_void,
        *const DescriptorHeapDesc,
        *const GUID,
        *mut *mut c_void,
    ) -> HRESULT,
    pub(super) get_descriptor_handle_increment_size:
        unsafe extern "system" fn(*mut c_void, u32) -> u32,
    pub(super) create_root_signature: unsafe extern "system" fn(
        *mut c_void,
        u32,
        *const c_void,
        usize,
        *const GUID,
        *mut *mut c_void,
    ) -> HRESULT,
    _create_constant_buffer_view: Slot,
    pub(super) create_shader_resource_view: unsafe extern "system" fn(
        *mut c_void,
        *mut c_void,
        *const ShaderResourceViewDesc,
        CpuDescriptorHandle,
    ),
    pub(super) create_unordered_access_view: unsafe extern "system" fn(
        *mut c_void,
        *mut c_void,
        *mut c_void,
        *const UnorderedAccessViewDesc,
        CpuDescriptorHandle,
    ),
    pub(super) create_render_target_view: unsafe extern "system" fn(
        *mut c_void,
        *mut c_void,
        *const RenderTargetViewDesc,
        CpuDescriptorHandle,
    ),
    pub(super) create_depth_stencil_view: unsafe extern "system" fn(
        *mut c_void,
        *mut c_void,
        *const DepthStencilViewDesc,
        CpuDescriptorHandle,
    ),
    pub(super) create_sampler:
        unsafe extern "system" fn(*mut c_void, *const SamplerDesc, CpuDescriptorHandle),
    _copy_descriptors: Slot,
    pub(super) copy_descriptors_simple:
        unsafe extern "system" fn(*mut c_void, u32, CpuDescriptorHandle, CpuDescriptorHandle, u32),
    /// GetResourceAllocationInfo, GetCustomHeapProperties
    _get_resource_allocation_info: [Slot; 2],
    pub(super) create_committed_resource: unsafe extern "system" fn(
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
    pub(super) create_fence:
        unsafe extern "system" fn(*mut c_void, u64, u32, *const GUID, *mut *mut c_void) -> HRESULT,
    pub(super) get_device_removed_reason: unsafe extern "system" fn(*mut c_void) -> HRESULT,
    /// GetCopyableFootprints, CreateQueryHeap, SetStablePowerState
    _get_copyable_footprints: [Slot; 3],
    pub(super) create_command_signature: unsafe extern "system" fn(
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
pub(super) struct ID3D12DescriptorHeapVtbl {
    pub(super) object: ID3D12ObjectVtbl,
    /// ID3D12DeviceChild's GetDevice; GetDesc
    _get_device: [Slot; 2],
    pub(super) get_cpu_descriptor_handle_for_heap_start:
        unsafe extern "system" fn(
            *mut c_void,
            *mut CpuDescriptorHandle,
        ) -> *mut CpuDescriptorHandle,
    pub(super) get_gpu_descriptor_handle_for_heap_start:
        unsafe extern "system" fn(
            *mut c_void,
            *mut GpuDescriptorHandle,
        ) -> *mut GpuDescriptorHandle,
}

/// `ID3D12ResourceVtbl` (the part SDL uses).
#[repr(C)]
pub(super) struct ID3D12ResourceVtbl {
    pub(super) object: ID3D12ObjectVtbl,
    /// ID3D12DeviceChild's GetDevice
    _get_device: Slot,
    pub(super) map:
        unsafe extern "system" fn(*mut c_void, u32, *const c_void, *mut *mut c_void) -> HRESULT,
    pub(super) unmap: unsafe extern "system" fn(*mut c_void, u32, *const c_void),
    pub(super) get_desc:
        unsafe extern "system" fn(*mut c_void, *mut ResourceDesc) -> *mut ResourceDesc,
    pub(super) get_gpu_virtual_address: unsafe extern "system" fn(*mut c_void) -> u64,
    /// WriteToSubresource, ReadFromSubresource, GetHeapProperties
    _write_to_subresource: [Slot; 3],
}

/// `ID3D12PipelineStateVtbl` (`ID3D12DeviceChild`'s GetDevice and
/// GetCachedBlob follow).
#[repr(C)]
pub(super) struct ID3D12PipelineStateVtbl {
    pub(super) object: ID3D12ObjectVtbl,
}

/// `ID3DBlobVtbl` (`ID3D10Blob`).
#[repr(C)]
pub(super) struct ID3DBlobVtbl {
    pub(super) base: IUnknownVtbl,
    pub(super) get_buffer_pointer: unsafe extern "system" fn(*mut c_void) -> *mut c_void,
    pub(super) get_buffer_size: unsafe extern "system" fn(*mut c_void) -> usize,
}

/// `ID3D12DebugVtbl`.
#[repr(C)]
pub(super) struct ID3D12DebugVtbl {
    pub(super) base: IUnknownVtbl,
    pub(super) enable_debug_layer: unsafe extern "system" fn(*mut c_void),
}

/// `ID3D12InfoQueueVtbl` (the part SDL uses).
#[repr(C)]
pub(super) struct ID3D12InfoQueueVtbl {
    pub(super) base: IUnknownVtbl,
    _set_message_count_limit: Slot,
    pub(super) clear_stored_messages: unsafe extern "system" fn(*mut c_void),
    pub(super) get_message:
        unsafe extern "system" fn(*mut c_void, u64, *mut Message, *mut usize) -> HRESULT,
    /// GetNumMessagesAllowedByStorageFilter,
    /// GetNumMessagesDeniedByStorageFilter
    _get_num_messages_allowed_by_storage_filter: [Slot; 2],
    pub(super) get_num_stored_messages: unsafe extern "system" fn(*mut c_void) -> u64,
    /// GetNumStoredMessagesAllowedByRetrievalFilter,
    /// GetNumMessagesDiscardedByMessageCountLimit, GetMessageCountLimit,
    /// AddStorageFilterEntries, GetStorageFilter, ClearStorageFilter,
    /// PushEmptyStorageFilter, PushCopyOfStorageFilter
    _get_num_stored_messages_allowed_by_retrieval_filter: [Slot; 8],
    pub(super) push_storage_filter:
        unsafe extern "system" fn(*mut c_void, *const InfoQueueFilter) -> HRESULT,
    /// PopStorageFilter ... SetBreakOnCategory (13 methods)
    _pop_storage_filter: [Slot; 13],
    pub(super) set_break_on_severity: unsafe extern "system" fn(*mut c_void, u32, BOOL) -> HRESULT,
    /// SetBreakOnID, GetBreakOnCategory, GetBreakOnSeverity, GetBreakOnID,
    /// SetMuteDebugOutput, GetMuteDebugOutput
    _set_break_on_id: [Slot; 6],
}

/// `ID3D12InfoQueue1Vtbl` (the part SDL uses).
#[repr(C)]
pub(super) struct ID3D12InfoQueue1Vtbl {
    pub(super) info_queue: ID3D12InfoQueueVtbl,
    pub(super) register_message_callback: unsafe extern "system" fn(
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
pub(super) struct ID3D12SDKConfigurationVtbl {
    pub(super) base: IUnknownVtbl,
    _set_sdk_version: Slot,
}

/// `ID3D12SDKConfiguration1Vtbl` (the part SDL uses).
#[repr(C)]
pub(super) struct ID3D12SDKConfiguration1Vtbl {
    pub(super) configuration: ID3D12SDKConfigurationVtbl,
    pub(super) create_device_factory: unsafe extern "system" fn(
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
pub(super) struct ID3D12DeviceFactoryVtbl {
    pub(super) base: IUnknownVtbl,
    /// InitializeFromGlobalState, ApplyToGlobalState
    _initialize_from_global_state: [Slot; 2],
    pub(super) set_flags: unsafe extern "system" fn(*mut c_void, u32) -> HRESULT,
    _get_flags: Slot,
    pub(super) get_configuration_interface: unsafe extern "system" fn(
        *mut c_void,
        *const GUID,
        *const GUID,
        *mut *mut c_void,
    ) -> HRESULT,
    _enable_experimental_features: Slot,
    pub(super) create_device: unsafe extern "system" fn(
        *mut c_void,
        *mut c_void,
        D3dFeatureLevel,
        *const GUID,
        *mut *mut c_void,
    ) -> HRESULT,
}

/// `ID3D12CommandQueueVtbl` (the part SDL uses).
#[repr(C)]
pub(super) struct ID3D12CommandQueueVtbl {
    pub(super) object: ID3D12ObjectVtbl,
    /// ID3D12DeviceChild's GetDevice; UpdateTileMappings, CopyTileMappings
    _get_device: [Slot; 3],
    pub(super) execute_command_lists:
        unsafe extern "system" fn(*mut c_void, u32, *const *mut c_void),
    /// SetMarker, BeginEvent, EndEvent
    _set_marker: [Slot; 3],
    pub(super) signal: unsafe extern "system" fn(*mut c_void, *mut c_void, u64) -> HRESULT,
    /// Wait, GetTimestampFrequency, GetClockCalibration, GetDesc
    _wait: [Slot; 4],
}

/// `ID3D12CommandAllocatorVtbl`.
#[repr(C)]
pub(super) struct ID3D12CommandAllocatorVtbl {
    pub(super) object: ID3D12ObjectVtbl,
    /// ID3D12DeviceChild's GetDevice
    _get_device: Slot,
    pub(super) reset: unsafe extern "system" fn(*mut c_void) -> HRESULT,
}

/// `ID3D12FenceVtbl`.
#[repr(C)]
pub(super) struct ID3D12FenceVtbl {
    pub(super) object: ID3D12ObjectVtbl,
    /// ID3D12DeviceChild's GetDevice
    _get_device: Slot,
    pub(super) get_completed_value: unsafe extern "system" fn(*mut c_void) -> u64,
    pub(super) set_event_on_completion:
        unsafe extern "system" fn(*mut c_void, u64, HANDLE) -> HRESULT,
    pub(super) signal: unsafe extern "system" fn(*mut c_void, u64) -> HRESULT,
}

/// `ID3D12GraphicsCommandListVtbl` (the part SDL uses).
#[repr(C)]
pub(super) struct ID3D12GraphicsCommandListVtbl {
    pub(super) object: ID3D12ObjectVtbl,
    /// ID3D12DeviceChild's GetDevice; ID3D12CommandList's GetType
    _get_device: [Slot; 2],
    pub(super) close: unsafe extern "system" fn(*mut c_void) -> HRESULT,
    pub(super) reset: unsafe extern "system" fn(*mut c_void, *mut c_void, *mut c_void) -> HRESULT,
    _clear_state: Slot,
    pub(super) draw_instanced: unsafe extern "system" fn(*mut c_void, u32, u32, u32, u32),
    pub(super) draw_indexed_instanced:
        unsafe extern "system" fn(*mut c_void, u32, u32, u32, i32, u32),
    pub(super) dispatch: unsafe extern "system" fn(*mut c_void, u32, u32, u32),
    pub(super) copy_buffer_region:
        unsafe extern "system" fn(*mut c_void, *mut c_void, u64, *mut c_void, u64, u64),
    pub(super) copy_texture_region: unsafe extern "system" fn(
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
    pub(super) resolve_subresource:
        unsafe extern "system" fn(*mut c_void, *mut c_void, u32, *mut c_void, u32, DxgiFormat),
    pub(super) ia_set_primitive_topology: unsafe extern "system" fn(*mut c_void, u32),
    pub(super) rs_set_viewports: unsafe extern "system" fn(*mut c_void, u32, *const D3d12Viewport),
    pub(super) rs_set_scissor_rects: unsafe extern "system" fn(*mut c_void, u32, *const D3d12Rect),
    pub(super) om_set_blend_factor: unsafe extern "system" fn(*mut c_void, *const f32),
    pub(super) om_set_stencil_ref: unsafe extern "system" fn(*mut c_void, u32),
    pub(super) set_pipeline_state: unsafe extern "system" fn(*mut c_void, *mut c_void),
    pub(super) resource_barrier:
        unsafe extern "system" fn(*mut c_void, u32, *const ResourceBarrier),
    _execute_bundle: Slot,
    pub(super) set_descriptor_heaps:
        unsafe extern "system" fn(*mut c_void, u32, *const *mut c_void),
    pub(super) set_compute_root_signature: unsafe extern "system" fn(*mut c_void, *mut c_void),
    pub(super) set_graphics_root_signature: unsafe extern "system" fn(*mut c_void, *mut c_void),
    pub(super) set_compute_root_descriptor_table:
        unsafe extern "system" fn(*mut c_void, u32, GpuDescriptorHandle),
    pub(super) set_graphics_root_descriptor_table:
        unsafe extern "system" fn(*mut c_void, u32, GpuDescriptorHandle),
    /// SetComputeRoot32BitConstant, SetGraphicsRoot32BitConstant,
    /// SetComputeRoot32BitConstants, SetGraphicsRoot32BitConstants
    _set_compute_root_32bit_constant: [Slot; 4],
    pub(super) set_compute_root_constant_buffer_view:
        unsafe extern "system" fn(*mut c_void, u32, u64),
    pub(super) set_graphics_root_constant_buffer_view:
        unsafe extern "system" fn(*mut c_void, u32, u64),
    /// SetComputeRootShaderResourceView, SetGraphicsRootShaderResourceView,
    /// SetComputeRootUnorderedAccessView, SetGraphicsRootUnorderedAccessView
    _set_compute_root_shader_resource_view: [Slot; 4],
    pub(super) ia_set_index_buffer: unsafe extern "system" fn(*mut c_void, *const IndexBufferView),
    pub(super) ia_set_vertex_buffers:
        unsafe extern "system" fn(*mut c_void, u32, u32, *const VertexBufferView),
    _so_set_targets: Slot,
    pub(super) om_set_render_targets: unsafe extern "system" fn(
        *mut c_void,
        u32,
        *const CpuDescriptorHandle,
        BOOL,
        *const CpuDescriptorHandle,
    ),
    pub(super) clear_depth_stencil_view: unsafe extern "system" fn(
        *mut c_void,
        CpuDescriptorHandle,
        u32,
        f32,
        u8,
        u32,
        *const D3d12Rect,
    ),
    pub(super) clear_render_target_view: unsafe extern "system" fn(
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
    pub(super) execute_indirect: unsafe extern "system" fn(
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
pub(super) struct IDXGISwapChain3Vtbl {
    pub(super) base: IUnknownVtbl,
    /// IDXGIObject's SetPrivateData, SetPrivateDataInterface,
    /// GetPrivateData
    _set_private_data: [Slot; 3],
    pub(super) get_parent:
        unsafe extern "system" fn(*mut c_void, *const GUID, *mut *mut c_void) -> HRESULT,
    /// IDXGIDeviceSubObject's GetDevice
    _get_device: Slot,
    pub(super) present: unsafe extern "system" fn(*mut c_void, u32, u32) -> HRESULT,
    pub(super) get_buffer:
        unsafe extern "system" fn(*mut c_void, u32, *const GUID, *mut *mut c_void) -> HRESULT,
    /// SetFullscreenState, GetFullscreenState, GetDesc
    _set_fullscreen_state: [Slot; 3],
    pub(super) resize_buffers:
        unsafe extern "system" fn(*mut c_void, u32, u32, u32, DxgiFormat, u32) -> HRESULT,
    /// ResizeTarget, GetContainingOutput, GetFrameStatistics,
    /// GetLastPresentCount
    _resize_target: [Slot; 4],
    pub(super) get_desc1: unsafe extern "system" fn(*mut c_void, *mut SwapChainDesc1) -> HRESULT,
    /// GetFullscreenDesc ... GetRotation (IDXGISwapChain1), SetSourceSize ...
    /// GetMatrixTransform (IDXGISwapChain2)
    _get_fullscreen_desc: [Slot; 17],
    pub(super) get_current_back_buffer_index: unsafe extern "system" fn(*mut c_void) -> u32,
    pub(super) check_color_space_support:
        unsafe extern "system" fn(*mut c_void, DxgiColorSpaceType, *mut u32) -> HRESULT,
    pub(super) set_color_space1:
        unsafe extern "system" fn(*mut c_void, DxgiColorSpaceType) -> HRESULT,
    _resize_buffers1: Slot,
}

/// The vtable of an interface SDL calls none of the methods of (root
/// signatures, command signatures and lists): just its `IUnknown` part.
#[repr(C)]
pub(super) struct OpaqueVtbl {
    pub(super) base: IUnknownVtbl,
}

pub(super) type D3d12Device = ComPtr<ID3D12DeviceVtbl>;
pub(super) type D3d12DescriptorHeap = ComPtr<ID3D12DescriptorHeapVtbl>;
pub(super) type D3d12Resource = ComPtr<ID3D12ResourceVtbl>;
pub(super) type D3d12PipelineState = ComPtr<ID3D12PipelineStateVtbl>;
pub(super) type D3d12RootSignature = ComPtr<OpaqueVtbl>;
pub(super) type D3d12CommandQueue = ComPtr<ID3D12CommandQueueVtbl>;
pub(super) type D3d12CommandSignature = ComPtr<OpaqueVtbl>;
pub(super) type D3d12CommandAllocator = ComPtr<ID3D12CommandAllocatorVtbl>;
pub(super) type D3d12GraphicsCommandList = ComPtr<ID3D12GraphicsCommandListVtbl>;
pub(super) type D3d12CommandList = ComPtr<OpaqueVtbl>;
pub(super) type D3d12Fence = ComPtr<ID3D12FenceVtbl>;
pub(super) type DxgiSwapChain3 = ComPtr<IDXGISwapChain3Vtbl>;
pub(super) type D3dBlob = ComPtr<ID3DBlobVtbl>;
pub(super) type D3d12Debug = ComPtr<ID3D12DebugVtbl>;
pub(super) type D3d12InfoQueue = ComPtr<ID3D12InfoQueueVtbl>;
pub(super) type D3d12InfoQueue1 = ComPtr<ID3D12InfoQueue1Vtbl>;
pub(super) type D3d12SdkConfiguration1 = ComPtr<ID3D12SDKConfiguration1Vtbl>;
pub(super) type D3d12DeviceFactory = ComPtr<ID3D12DeviceFactoryVtbl>;

/// `ID3D12Object::SetName()`, for the objects SDL names.
pub(super) trait D3d12Object {
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
    pub(super) fn create_command_queue(
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
    pub(super) unsafe fn create_graphics_pipeline_state(
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
    pub(super) unsafe fn create_compute_pipeline_state(
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
    pub(super) fn check_feature_support<T: Copy>(&self, feature: u32, data: &mut T) -> HRESULT {
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
    pub(super) fn create_descriptor_heap(
        &self,
        desc: &DescriptorHeapDesc,
    ) -> Result<D3d12DescriptorHeap, HRESULT> {
        let f = self.vtbl().create_descriptor_heap;
        // SAFETY: a live device and description; the call stores an owned
        // ID3D12DescriptorHeap.
        unsafe { out(|o| f(raw(self), desc, &IID_ID3D12DESCRIPTORHEAP, o)) }
    }

    /// `ID3D12Device::GetDescriptorHandleIncrementSize()`
    pub(super) fn descriptor_handle_increment_size(&self, heap_type: u32) -> u32 {
        // SAFETY: a live device.
        unsafe { (self.vtbl().get_descriptor_handle_increment_size)(raw(self), heap_type) }
    }

    /// `ID3D12Device::CreateRootSignature()` from a serialized root
    /// signature.
    pub(super) fn create_root_signature(&self, blob: &[u8]) -> Result<D3d12RootSignature, HRESULT> {
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
    pub(super) fn create_shader_resource_view(
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
    pub(super) fn create_unordered_access_view(
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
    pub(super) fn create_render_target_view(
        &self,
        resource: &D3d12Resource,
        desc: &RenderTargetViewDesc,
        dest: CpuDescriptorHandle,
    ) {
        // SAFETY: as for create_shader_resource_view.
        unsafe { (self.vtbl().create_render_target_view)(raw(self), raw(resource), desc, dest) }
    }

    /// `ID3D12Device::CreateDepthStencilView()`
    pub(super) fn create_depth_stencil_view(
        &self,
        resource: &D3d12Resource,
        desc: &DepthStencilViewDesc,
        dest: CpuDescriptorHandle,
    ) {
        // SAFETY: as for create_shader_resource_view.
        unsafe { (self.vtbl().create_depth_stencil_view)(raw(self), raw(resource), desc, dest) }
    }

    /// `ID3D12Device::CreateSampler()`
    pub(super) fn create_sampler(&self, desc: &SamplerDesc, dest: CpuDescriptorHandle) {
        // SAFETY: a live device, a description and a descriptor of a live
        // heap.
        unsafe { (self.vtbl().create_sampler)(raw(self), desc, dest) }
    }

    /// `ID3D12Device::CreateCommittedResource()`
    pub(super) fn create_committed_resource(
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
    pub(super) fn device_removed_reason(&self) -> HRESULT {
        // SAFETY: a live device.
        unsafe { (self.vtbl().get_device_removed_reason)(raw(self)) }
    }

    /// `ID3D12Device::CreateCommandSignature()`, without a root signature.
    pub(super) fn create_command_signature(
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
    pub(super) fn cpu_descriptor_handle_for_heap_start(&self) -> CpuDescriptorHandle {
        let mut handle = CpuDescriptorHandle::default();
        // SAFETY: a live heap; the method writes the handle it returns.
        unsafe { (self.vtbl().get_cpu_descriptor_handle_for_heap_start)(raw(self), &mut handle) };
        handle
    }

    /// `ID3D12DescriptorHeap::GetGPUDescriptorHandleForHeapStart()`
    pub(super) fn gpu_descriptor_handle_for_heap_start(&self) -> GpuDescriptorHandle {
        let mut handle = GpuDescriptorHandle::default();
        // SAFETY: as above.
        unsafe { (self.vtbl().get_gpu_descriptor_handle_for_heap_start)(raw(self), &mut handle) };
        handle
    }
}

impl D3d12Resource {
    /// `ID3D12Resource::Map()` of the whole first subresource.
    pub(super) fn map(&self) -> Result<*mut u8, HRESULT> {
        let mut data: *mut c_void = null_mut();
        // SAFETY: a live resource; no read range (the whole resource).
        check(unsafe { (self.vtbl().map)(raw(self), 0, null(), &mut data) })?;
        Ok(data.cast())
    }

    /// `ID3D12Resource::Unmap()` of the first subresource, with the whole
    /// resource written.
    pub(super) fn unmap(&self) {
        // SAFETY: a live, mapped resource.
        unsafe { (self.vtbl().unmap)(raw(self), 0, null()) }
    }

    /// `ID3D12Resource::GetGPUVirtualAddress()`
    pub(super) fn gpu_virtual_address(&self) -> u64 {
        // SAFETY: a live resource.
        unsafe { (self.vtbl().get_gpu_virtual_address)(raw(self)) }
    }
}

impl D3dBlob {
    /// The blob's bytes (`GetBufferPointer()`, `GetBufferSize()`).
    pub(super) fn bytes(&self) -> &[u8] {
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
    pub(super) fn enable_debug_layer(&self) {
        // SAFETY: a live debug interface.
        unsafe { (self.vtbl().enable_debug_layer)(raw(self)) }
    }
}

impl D3d12InfoQueue {
    /// `ID3D12InfoQueue::PushStorageFilter()`
    pub(super) fn push_storage_filter(&self, filter: &InfoQueueFilter) -> HRESULT {
        // SAFETY: a live queue; the filter's lists outlive the call.
        unsafe { (self.vtbl().push_storage_filter)(raw(self), filter) }
    }

    /// `ID3D12InfoQueue::SetBreakOnSeverity()`
    pub(super) fn set_break_on_severity(&self, severity: u32, enable: bool) -> HRESULT {
        // SAFETY: a live queue.
        unsafe { (self.vtbl().set_break_on_severity)(raw(self), severity, enable as BOOL) }
    }

    /// `ID3D12InfoQueue::GetNumStoredMessages()`
    pub(super) fn num_stored_messages(&self) -> u64 {
        // SAFETY: a live queue.
        unsafe { (self.vtbl().get_num_stored_messages)(raw(self)) }
    }

    /// Call `f` with the stored message `index` (`GetMessage()` twice: for
    /// its size, then into a buffer of that size). Nothing happens if
    /// getting it fails.
    pub(super) fn with_message(&self, index: u64, f: impl FnOnce(&Message)) {
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
    pub(super) fn clear_stored_messages(&self) {
        // SAFETY: a live queue.
        unsafe { (self.vtbl().clear_stored_messages)(raw(self)) }
    }
}

impl D3d12InfoQueue1 {
    /// `ID3D12InfoQueue1::RegisterMessageCallback()`, without a context:
    /// the callback's cookie.
    pub(super) fn register_message_callback(
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
    pub(super) fn create_device_factory(
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
    pub(super) fn set_flags(&self, flags: u32) -> HRESULT {
        // SAFETY: a live factory.
        unsafe { (self.vtbl().set_flags)(raw(self), flags) }
    }

    /// `ID3D12DeviceFactory::GetConfigurationInterface()` of the debug
    /// layer.
    pub(super) fn debug_interface(&self) -> Result<D3d12Debug, HRESULT> {
        let f = self.vtbl().get_configuration_interface;
        // SAFETY: a live factory; the call stores an owned ID3D12Debug, the
        // interface asked for.
        unsafe { out(|o| f(raw(self), &CLSID_ID3D12DEBUG, &IID_ID3D12DEBUG, o)) }
    }

    /// `ID3D12DeviceFactory::CreateDevice()`
    pub(super) fn create_device(
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
    pub(super) fn create_command_allocator(
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
    pub(super) fn create_command_list(
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
    pub(super) unsafe fn copy_descriptors_simple(
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

    /// `ID3D12Device::CreateFence()`
    pub(super) fn create_fence(
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
    pub(super) fn desc(&self) -> ResourceDesc {
        let mut desc = ResourceDesc::default();
        // SAFETY: a live resource; the method writes the description it
        // returns.
        unsafe { (self.vtbl().get_desc)(raw(self), &mut desc) };
        desc
    }
}

impl D3d12CommandQueue {
    /// `ID3D12CommandQueue::ExecuteCommandLists()` of one list.
    pub(super) fn execute_command_list(&self, command_list: &D3d12CommandList) {
        let list = raw(command_list);
        // SAFETY: a live queue and a closed command list.
        unsafe { (self.vtbl().execute_command_lists)(raw(self), 1, &list) }
    }

    /// `ID3D12CommandQueue::Signal()`
    pub(super) fn signal(&self, fence: &D3d12Fence, value: u64) -> HRESULT {
        // SAFETY: a live queue and fence.
        unsafe { (self.vtbl().signal)(raw(self), raw(fence), value) }
    }
}

impl D3d12CommandAllocator {
    /// `ID3D12CommandAllocator::Reset()`
    pub(super) fn reset(&self) -> HRESULT {
        // SAFETY: a live allocator whose command lists the GPU is done with.
        unsafe { (self.vtbl().reset)(raw(self)) }
    }
}

impl D3d12Fence {
    /// `ID3D12Fence::GetCompletedValue()`
    pub(super) fn completed_value(&self) -> u64 {
        // SAFETY: a live fence.
        unsafe { (self.vtbl().get_completed_value)(raw(self)) }
    }

    /// `ID3D12Fence::SetEventOnCompletion()`
    pub(super) fn set_event_on_completion(&self, value: u64, event: HANDLE) -> HRESULT {
        // SAFETY: a live fence and an event handle (or NULL).
        unsafe { (self.vtbl().set_event_on_completion)(raw(self), value, event) }
    }

    /// `ID3D12Fence::Signal()`
    pub(super) fn signal(&self, value: u64) -> HRESULT {
        // SAFETY: a live fence.
        unsafe { (self.vtbl().signal)(raw(self), value) }
    }
}

/// The methods of `ID3D12GraphicsCommandList` SDL records with. The
/// resources, descriptors and objects they name must be live, which the
/// command buffers' tracking ensures until the GPU is done with them.
impl D3d12GraphicsCommandList {
    /// `Close()`
    pub(super) fn close(&self) -> HRESULT {
        // SAFETY: a live command list being recorded.
        unsafe { (self.vtbl().close)(raw(self)) }
    }

    /// `Reset()`, without an initial pipeline state.
    pub(super) fn reset(&self, allocator: &D3d12CommandAllocator) -> HRESULT {
        // SAFETY: a closed command list and a reset allocator.
        unsafe { (self.vtbl().reset)(raw(self), raw(allocator), null_mut()) }
    }

    /// `DrawInstanced()`
    pub(super) fn draw_instanced(&self, vertices: u32, instances: u32, first: u32, base: u32) {
        // SAFETY: a command list being recorded.
        unsafe { (self.vtbl().draw_instanced)(raw(self), vertices, instances, first, base) }
    }

    /// `DrawIndexedInstanced()`
    pub(super) fn draw_indexed_instanced(
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
    pub(super) fn dispatch(&self, x: u32, y: u32, z: u32) {
        // SAFETY: a command list being recorded.
        unsafe { (self.vtbl().dispatch)(raw(self), x, y, z) }
    }

    /// `CopyBufferRegion()`
    pub(super) fn copy_buffer_region(
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
    pub(super) fn copy_texture_region(
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
    pub(super) fn resolve_subresource(
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
    pub(super) fn ia_set_primitive_topology(&self, topology: u32) {
        // SAFETY: a command list being recorded.
        unsafe { (self.vtbl().ia_set_primitive_topology)(raw(self), topology) }
    }

    /// `RSSetViewports()` of one viewport.
    pub(super) fn rs_set_viewport(&self, viewport: &D3d12Viewport) {
        // SAFETY: a command list being recorded.
        unsafe { (self.vtbl().rs_set_viewports)(raw(self), 1, viewport) }
    }

    /// `RSSetScissorRects()` of one rectangle.
    pub(super) fn rs_set_scissor_rect(&self, rect: &D3d12Rect) {
        // SAFETY: a command list being recorded.
        unsafe { (self.vtbl().rs_set_scissor_rects)(raw(self), 1, rect) }
    }

    /// `OMSetBlendFactor()`
    pub(super) fn om_set_blend_factor(&self, blend_factor: &[f32; 4]) {
        // SAFETY: a command list being recorded and four floats.
        unsafe { (self.vtbl().om_set_blend_factor)(raw(self), blend_factor.as_ptr()) }
    }

    /// `OMSetStencilRef()`
    pub(super) fn om_set_stencil_ref(&self, stencil_ref: u32) {
        // SAFETY: a command list being recorded.
        unsafe { (self.vtbl().om_set_stencil_ref)(raw(self), stencil_ref) }
    }

    /// `SetPipelineState()`
    pub(super) fn set_pipeline_state(&self, pipeline_state: &D3d12PipelineState) {
        // SAFETY: a command list being recorded and a live pipeline state.
        unsafe { (self.vtbl().set_pipeline_state)(raw(self), raw(pipeline_state)) }
    }

    /// `ResourceBarrier()`
    pub(super) fn resource_barrier(&self, barriers: &[ResourceBarrier]) {
        // SAFETY: a command list being recorded; the barriers name live
        // resources.
        unsafe {
            (self.vtbl().resource_barrier)(raw(self), barriers.len() as u32, barriers.as_ptr())
        }
    }

    /// `SetDescriptorHeaps()`
    pub(super) fn set_descriptor_heaps(&self, heaps: &[&D3d12DescriptorHeap]) {
        let heaps: Vec<*mut c_void> = heaps.iter().map(|h| raw(*h)).collect();
        // SAFETY: a command list being recorded and live shader-visible
        // heaps.
        unsafe { (self.vtbl().set_descriptor_heaps)(raw(self), heaps.len() as u32, heaps.as_ptr()) }
    }

    /// `SetComputeRootSignature()`
    pub(super) fn set_compute_root_signature(&self, root_signature: &D3d12RootSignature) {
        // SAFETY: a command list being recorded and a live root signature.
        unsafe { (self.vtbl().set_compute_root_signature)(raw(self), raw(root_signature)) }
    }

    /// `SetGraphicsRootSignature()`
    pub(super) fn set_graphics_root_signature(&self, root_signature: &D3d12RootSignature) {
        // SAFETY: a command list being recorded and a live root signature.
        unsafe { (self.vtbl().set_graphics_root_signature)(raw(self), raw(root_signature)) }
    }

    /// `SetComputeRootDescriptorTable()`
    pub(super) fn set_compute_root_descriptor_table(&self, index: i32, base: GpuDescriptorHandle) {
        // SAFETY: a command list being recorded; the descriptors are in the
        // shader-visible heaps set.
        unsafe { (self.vtbl().set_compute_root_descriptor_table)(raw(self), index as u32, base) }
    }

    /// `SetGraphicsRootDescriptorTable()`
    pub(super) fn set_graphics_root_descriptor_table(&self, index: i32, base: GpuDescriptorHandle) {
        // SAFETY: as for set_compute_root_descriptor_table().
        unsafe { (self.vtbl().set_graphics_root_descriptor_table)(raw(self), index as u32, base) }
    }

    /// `SetComputeRootConstantBufferView()`
    pub(super) fn set_compute_root_constant_buffer_view(&self, index: i32, address: u64) {
        // SAFETY: a command list being recorded; the address is in a live
        // buffer.
        unsafe {
            (self.vtbl().set_compute_root_constant_buffer_view)(raw(self), index as u32, address)
        }
    }

    /// `SetGraphicsRootConstantBufferView()`
    pub(super) fn set_graphics_root_constant_buffer_view(&self, index: i32, address: u64) {
        // SAFETY: as for set_compute_root_constant_buffer_view().
        unsafe {
            (self.vtbl().set_graphics_root_constant_buffer_view)(raw(self), index as u32, address)
        }
    }

    /// `IASetIndexBuffer()`
    pub(super) fn ia_set_index_buffer(&self, view: &IndexBufferView) {
        // SAFETY: a command list being recorded; the view is of a live
        // buffer.
        unsafe { (self.vtbl().ia_set_index_buffer)(raw(self), view) }
    }

    /// `IASetVertexBuffers()`
    pub(super) fn ia_set_vertex_buffers(&self, start_slot: u32, views: &[VertexBufferView]) {
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
    pub(super) fn om_set_render_targets(
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
    pub(super) fn clear_depth_stencil_view(
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
    pub(super) fn clear_render_target_view(&self, view: CpuDescriptorHandle, color: &[f32; 4]) {
        // SAFETY: a command list being recorded and a descriptor of a live
        // view.
        unsafe {
            (self.vtbl().clear_render_target_view)(raw(self), view, color.as_ptr(), 0, null())
        }
    }

    /// `ExecuteIndirect()` without a count buffer.
    pub(super) fn execute_indirect(
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
    pub(super) fn command_list(&self) -> Result<D3d12CommandList, HRESULT> {
        self.query(&IID_ID3D12COMMANDLIST)
    }

    /// The interface pointer, for the PIX runtime's functions.
    pub(super) fn as_raw(&self) -> *mut c_void {
        raw(self)
    }
}

/// `IDXGIFactory2::CreateSwapChainForHwnd()` on a command queue, as an
/// `IDXGISwapChain1` (whose `IUnknown` is all SDL needs before asking for
/// its `IDXGISwapChain3`).
pub(super) fn create_swap_chain_for_hwnd(
    factory: &DxgiFactory4,
    command_queue: &D3d12CommandQueue,
    hwnd: HWND,
    desc: &SwapChainDesc1,
    fullscreen_desc: &SwapChainFullscreenDesc,
) -> Result<ComPtr<OpaqueVtbl>, HRESULT> {
    let f = factory.vtbl().factory2.create_swap_chain_for_hwnd;
    // SAFETY: a live factory and queue; the descriptions (whose layouts are
    // DXGI's) outlive the call; the swap chain is stored owned.
    unsafe {
        out(|o| {
            f(
                raw(factory),
                raw(command_queue),
                hwnd,
                (desc as *const SwapChainDesc1).cast(),
                (fullscreen_desc as *const SwapChainFullscreenDesc).cast(),
                null_mut(),
                o,
            )
        })
    }
}

impl DxgiSwapChain3 {
    /// `IDXGIObject::GetParent()` of an `IDXGIFactory1`.
    pub(super) fn parent_factory(&self) -> Result<DxgiFactory1, HRESULT> {
        let f = self.vtbl().get_parent;
        // SAFETY: a live swap chain; the call stores an owned
        // IDXGIFactory1, the interface asked for.
        unsafe { out(|o| f(raw(self), &IID_IDXGIFACTORY1, o)) }
    }

    /// `IDXGISwapChain::Present()`
    pub(super) fn present(&self, sync_interval: u32, flags: u32) -> HRESULT {
        // SAFETY: a live swap chain.
        unsafe { (self.vtbl().present)(raw(self), sync_interval, flags) }
    }

    /// `IDXGISwapChain::GetBuffer()` of an `ID3D12Resource`.
    pub(super) fn buffer(&self, index: u32) -> Result<D3d12Resource, HRESULT> {
        let f = self.vtbl().get_buffer;
        // SAFETY: a live swap chain; the call stores an owned
        // ID3D12Resource, the interface asked for.
        unsafe { out(|o| f(raw(self), index, &IID_ID3D12RESOURCE, o)) }
    }

    /// `IDXGISwapChain::ResizeBuffers()`
    pub(super) fn resize_buffers(
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
    pub(super) fn desc1(&self) -> SwapChainDesc1 {
        let mut desc = SwapChainDesc1::default();
        // SAFETY: a live swap chain; a valid output.
        unsafe { (self.vtbl().get_desc1)(raw(self), &mut desc) };
        desc
    }

    /// `IDXGISwapChain3::GetCurrentBackBufferIndex()`
    pub(super) fn current_back_buffer_index(&self) -> u32 {
        // SAFETY: a live swap chain.
        unsafe { (self.vtbl().get_current_back_buffer_index)(raw(self)) }
    }

    /// `IDXGISwapChain3::CheckColorSpaceSupport()`: the support flags (0
    /// when the call fails; upstream reads its uninitialized variable then).
    pub(super) fn check_color_space_support(&self, color_space: DxgiColorSpaceType) -> u32 {
        let mut support = 0;
        // SAFETY: a live swap chain; a valid output.
        unsafe { (self.vtbl().check_color_space_support)(raw(self), color_space, &mut support) };
        support
    }

    /// `IDXGISwapChain3::SetColorSpace1()`
    pub(super) fn set_color_space1(&self, color_space: DxgiColorSpaceType) -> HRESULT {
        // SAFETY: a live swap chain.
        unsafe { (self.vtbl().set_color_space1)(raw(self), color_space) }
    }
}

/// `IDXGIFactory::MakeWindowAssociation()` of a swap chain's parent.
pub(super) fn make_window_association(factory: &DxgiFactory1, hwnd: HWND, flags: u32) -> HRESULT {
    // SAFETY: a live factory and a window handle.
    unsafe { (factory.vtbl().make_window_association)(raw(factory), hwnd, flags) }
}
