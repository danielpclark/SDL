// The Direct3D 11 and DXGI declarations of src/render/direct3d11/SDL_render_d3d11.c
// (from <d3d11_1.h>, <dxgi1_5.h> and <dxgidebug.h>), for the Rust translation
// of Simple DirectMedia Layer.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! The Direct3D 11 and DXGI declarations the renderer uses, written by
//! hand (`windows-sys` has none of them): the enumerants, the structures
//! and GUIDs, the entry points `d3d11.dll` and `dxgi.dll` export, and the
//! vtables of the COM interfaces SDL calls (only the slots it uses are
//! typed, the rest are padding), with thin wrappers that turn the out
//! parameters into [`ComPtr`]s and the `HRESULT`s into `Result`s.
//!
//! The structures implement `Default` as all zeroes, which is what the C
//! code's `SDL_zero()` gives.

#![allow(clippy::upper_case_acronyms)]

use std::ffi::c_void;
use std::ptr::{null, null_mut};

use windows_sys::core::{BOOL, GUID, HRESULT};
use windows_sys::Win32::Foundation::{HMODULE, HWND, RECT};

use crate::core::windows::com::{ComObject, ComPtr, IUnknownVtbl};

pub(super) type DxgiFormat = u32;
pub(super) type D3d11Blend = u32;
pub(super) type D3d11BlendOp = u32;
pub(super) type D3d11PrimitiveTopology = u32;
pub(super) type DxgiModeRotation = u32;
pub(super) type DxgiColorSpaceType = u32;
pub(super) type D3dFeatureLevel = u32;

// --- enumerants ---

// DXGI_FORMAT
pub(super) const DXGI_FORMAT_UNKNOWN: DxgiFormat = 0;
pub(super) const DXGI_FORMAT_R32G32B32A32_FLOAT: DxgiFormat = 2;
pub(super) const DXGI_FORMAT_R16G16B16A16_FLOAT: DxgiFormat = 10;
pub(super) const DXGI_FORMAT_R32G32_FLOAT: DxgiFormat = 16;
pub(super) const DXGI_FORMAT_R10G10B10A2_UNORM: DxgiFormat = 24;
pub(super) const DXGI_FORMAT_R8G8B8A8_UNORM: DxgiFormat = 28;
pub(super) const DXGI_FORMAT_R8G8B8A8_UNORM_SRGB: DxgiFormat = 29;
pub(super) const DXGI_FORMAT_R16G16_UNORM: DxgiFormat = 35;
pub(super) const DXGI_FORMAT_R8G8_UNORM: DxgiFormat = 49;
pub(super) const DXGI_FORMAT_R16_UNORM: DxgiFormat = 56;
pub(super) const DXGI_FORMAT_R8_UNORM: DxgiFormat = 61;
pub(super) const DXGI_FORMAT_B5G6R5_UNORM: DxgiFormat = 85;
pub(super) const DXGI_FORMAT_B5G5R5A1_UNORM: DxgiFormat = 86;
pub(super) const DXGI_FORMAT_B8G8R8A8_UNORM: DxgiFormat = 87;
pub(super) const DXGI_FORMAT_B8G8R8X8_UNORM: DxgiFormat = 88;
pub(super) const DXGI_FORMAT_B8G8R8A8_UNORM_SRGB: DxgiFormat = 91;
pub(super) const DXGI_FORMAT_B8G8R8X8_UNORM_SRGB: DxgiFormat = 93;
pub(super) const DXGI_FORMAT_NV12: DxgiFormat = 103;
pub(super) const DXGI_FORMAT_P010: DxgiFormat = 104;
pub(super) const DXGI_FORMAT_B4G4R4A4_UNORM: DxgiFormat = 115;

// D3D11_USAGE
pub(super) const D3D11_USAGE_DEFAULT: u32 = 0;
pub(super) const D3D11_USAGE_DYNAMIC: u32 = 2;
pub(super) const D3D11_USAGE_STAGING: u32 = 3;

// D3D11_BIND_FLAG
pub(super) const D3D11_BIND_VERTEX_BUFFER: u32 = 0x1;
pub(super) const D3D11_BIND_CONSTANT_BUFFER: u32 = 0x4;
pub(super) const D3D11_BIND_SHADER_RESOURCE: u32 = 0x8;
pub(super) const D3D11_BIND_RENDER_TARGET: u32 = 0x20;

// D3D11_CPU_ACCESS_FLAG
pub(super) const D3D11_CPU_ACCESS_WRITE: u32 = 0x10000;
pub(super) const D3D11_CPU_ACCESS_READ: u32 = 0x20000;

// D3D11_MAP
pub(super) const D3D11_MAP_READ: u32 = 1;
pub(super) const D3D11_MAP_WRITE: u32 = 2;
pub(super) const D3D11_MAP_WRITE_DISCARD: u32 = 4;

// D3D11_SRV_DIMENSION, D3D11_RTV_DIMENSION
pub(super) const D3D11_SRV_DIMENSION_TEXTURE2D: u32 = 4;
pub(super) const D3D11_RTV_DIMENSION_TEXTURE2D: u32 = 4;

// D3D11_BLEND
pub(super) const D3D11_BLEND_ZERO: D3d11Blend = 1;
pub(super) const D3D11_BLEND_ONE: D3d11Blend = 2;
pub(super) const D3D11_BLEND_SRC_COLOR: D3d11Blend = 3;
pub(super) const D3D11_BLEND_INV_SRC_COLOR: D3d11Blend = 4;
pub(super) const D3D11_BLEND_SRC_ALPHA: D3d11Blend = 5;
pub(super) const D3D11_BLEND_INV_SRC_ALPHA: D3d11Blend = 6;
pub(super) const D3D11_BLEND_DEST_ALPHA: D3d11Blend = 7;
pub(super) const D3D11_BLEND_INV_DEST_ALPHA: D3d11Blend = 8;
pub(super) const D3D11_BLEND_DEST_COLOR: D3d11Blend = 9;
pub(super) const D3D11_BLEND_INV_DEST_COLOR: D3d11Blend = 10;

// D3D11_BLEND_OP
pub(super) const D3D11_BLEND_OP_ADD: D3d11BlendOp = 1;
pub(super) const D3D11_BLEND_OP_SUBTRACT: D3d11BlendOp = 2;
pub(super) const D3D11_BLEND_OP_REV_SUBTRACT: D3d11BlendOp = 3;
pub(super) const D3D11_BLEND_OP_MIN: D3d11BlendOp = 4;
pub(super) const D3D11_BLEND_OP_MAX: D3d11BlendOp = 5;

pub(super) const D3D11_COLOR_WRITE_ENABLE_ALL: u8 = 0xf;
pub(super) const D3D11_CULL_NONE: u32 = 1;
pub(super) const D3D11_FILL_SOLID: u32 = 3;

// D3D11_FILTER, D3D11_TEXTURE_ADDRESS_MODE, D3D11_COMPARISON_FUNC
pub(super) const D3D11_FILTER_MIN_MAG_MIP_POINT: u32 = 0;
pub(super) const D3D11_FILTER_MIN_MAG_MIP_LINEAR: u32 = 0x15;
pub(super) const D3D11_TEXTURE_ADDRESS_WRAP: u32 = 1;
pub(super) const D3D11_TEXTURE_ADDRESS_CLAMP: u32 = 3;
pub(super) const D3D11_COMPARISON_ALWAYS: u32 = 8;
pub(super) const D3D11_FLOAT32_MAX: f32 = f32::MAX;

pub(super) const D3D11_INPUT_PER_VERTEX_DATA: u32 = 0;

// D3D11_PRIMITIVE_TOPOLOGY
pub(super) const D3D11_PRIMITIVE_TOPOLOGY_POINTLIST: D3d11PrimitiveTopology = 1;
pub(super) const D3D11_PRIMITIVE_TOPOLOGY_LINELIST: D3d11PrimitiveTopology = 2;
pub(super) const D3D11_PRIMITIVE_TOPOLOGY_LINESTRIP: D3d11PrimitiveTopology = 3;
pub(super) const D3D11_PRIMITIVE_TOPOLOGY_TRIANGLELIST: D3d11PrimitiveTopology = 4;

pub(super) const D3D11_FORMAT_SUPPORT_TEXTURE2D: u32 = 0x20;

// D3D11_CREATE_DEVICE_FLAG
pub(super) const D3D11_CREATE_DEVICE_SINGLETHREADED: u32 = 0x1;
pub(super) const D3D11_CREATE_DEVICE_DEBUG: u32 = 0x2;
pub(super) const D3D11_CREATE_DEVICE_BGRA_SUPPORT: u32 = 0x20;
pub(super) const D3D11_SDK_VERSION: u32 = 7;

// D3D_DRIVER_TYPE, D3D_FEATURE_LEVEL
pub(super) const D3D_DRIVER_TYPE_UNKNOWN: u32 = 0;
pub(super) const D3D_DRIVER_TYPE_WARP: u32 = 5;
pub(super) const D3D_FEATURE_LEVEL_11_0: D3dFeatureLevel = 0xb000;
pub(super) const D3D_FEATURE_LEVEL_11_1: D3dFeatureLevel = 0xb100;

pub(super) const DXGI_CREATE_FACTORY_DEBUG: u32 = 0x1;
pub(super) const DXGI_FEATURE_PRESENT_ALLOW_TEARING: u32 = 0;
pub(super) const DXGI_SWAP_CHAIN_FLAG_ALLOW_TEARING: u32 = 2048;
pub(super) const DXGI_PRESENT_DO_NOT_WAIT: u32 = 0x8;
pub(super) const DXGI_PRESENT_ALLOW_TEARING: u32 = 0x200;
pub(super) const DXGI_USAGE_RENDER_TARGET_OUTPUT: u32 = 0x20;
pub(super) const DXGI_MWA_NO_WINDOW_CHANGES: u32 = 1;

// DXGI_SCALING, DXGI_SWAP_EFFECT
pub(super) const DXGI_SCALING_STRETCH: u32 = 0;
pub(super) const DXGI_SCALING_NONE: u32 = 1;
pub(super) const DXGI_SWAP_EFFECT_DISCARD: u32 = 0;
pub(super) const DXGI_SWAP_EFFECT_FLIP_SEQUENTIAL: u32 = 3;

// DXGI_MODE_ROTATION
pub(super) const DXGI_MODE_ROTATION_UNSPECIFIED: DxgiModeRotation = 0;
pub(super) const DXGI_MODE_ROTATION_IDENTITY: DxgiModeRotation = 1;
pub(super) const DXGI_MODE_ROTATION_ROTATE90: DxgiModeRotation = 2;
pub(super) const DXGI_MODE_ROTATION_ROTATE180: DxgiModeRotation = 3;
pub(super) const DXGI_MODE_ROTATION_ROTATE270: DxgiModeRotation = 4;

// DXGI_COLOR_SPACE_TYPE
pub(super) const DXGI_COLOR_SPACE_RGB_FULL_G22_NONE_P709: DxgiColorSpaceType = 0;
pub(super) const DXGI_COLOR_SPACE_RGB_FULL_G10_NONE_P709: DxgiColorSpaceType = 1;
pub(super) const DXGI_COLOR_SPACE_RGB_FULL_G2084_NONE_P2020: DxgiColorSpaceType = 12;
pub(super) const DXGI_SWAP_CHAIN_COLOR_SPACE_SUPPORT_FLAG_PRESENT: u32 = 1;

// DXGI_DEBUG_RLO_FLAGS, DXGI_INFO_QUEUE_MESSAGE_SEVERITY
pub(super) const DXGI_DEBUG_RLO_DETAIL: u32 = 0x2;
pub(super) const DXGI_DEBUG_RLO_IGNORE_INTERNAL: u32 = 0x4;
pub(super) const DXGI_INFO_QUEUE_MESSAGE_SEVERITY_CORRUPTION: u32 = 0;
pub(super) const DXGI_INFO_QUEUE_MESSAGE_SEVERITY_ERROR: u32 = 1;

// HRESULTs
pub(super) const E_FAIL: HRESULT = 0x8000_4005_u32 as HRESULT;
pub(super) const DXGI_ERROR_INVALID_CALL: HRESULT = 0x887a_0001_u32 as HRESULT;
pub(super) const DXGI_ERROR_DEVICE_REMOVED: HRESULT = 0x887a_0005_u32 as HRESULT;
pub(super) const DXGI_ERROR_WAS_STILL_DRAWING: HRESULT = 0x887a_000a_u32 as HRESULT;

// --- GUIDs (upstream defines them so as not to need uuid.lib) ---

pub(super) const IID_IDXGIFACTORY5: GUID = GUID::from_u128(0x7632e1f5_ee65_4dca_87fd_84cd75f8838d);
pub(super) const IID_IDXGIFACTORY2: GUID = GUID::from_u128(0x50c83a1c_e072_4c48_87b0_3630fa36a6d0);
pub(super) const IID_IDXGIDEVICE1: GUID = GUID::from_u128(0x77db970f_6276_48ba_ba28_070143b4392c);
pub(super) const IID_ID3D11TEXTURE2D: GUID =
    GUID::from_u128(0x6f15aaf2_d208_4e89_9ab4_489535d34f9c);
pub(super) const IID_ID3D11DEVICE1: GUID = GUID::from_u128(0xa04bfb29_08ef_43d6_a49c_a9bdbdcbe686);
pub(super) const IID_ID3D11DEVICECONTEXT1: GUID =
    GUID::from_u128(0xbb2c6faa_b5fb_4082_8e6b_388b8cfa90e1);
pub(super) const IID_IDXGISWAPCHAIN2: GUID =
    GUID::from_u128(0x94d99bdb_f1f8_4ab0_b236_7da0170edab1);
pub(super) const IID_IDXGIDEBUG1: GUID = GUID::from_u128(0xc5a05f0c_16f2_4adf_9f4d_a8c4d58ac550);
pub(super) const IID_IDXGIINFOQUEUE: GUID = GUID::from_u128(0xd67441c7_672a_476f_9e82_cd55b44949ce);
pub(super) const DXGI_DEBUG_ALL: GUID = GUID::from_u128(0xe48ae283_da80_490b_87e6_43e9a9cfda08);

// --- structures ---

/// `DXGI_SAMPLE_DESC`
#[repr(C)]
#[derive(Clone, Copy, Default, Debug, PartialEq)]
pub(super) struct SampleDesc {
    pub(super) count: u32,
    pub(super) quality: u32,
}

/// `D3D11_TEXTURE2D_DESC`
#[repr(C)]
#[derive(Clone, Copy, Default, Debug, PartialEq)]
pub(super) struct Texture2dDesc {
    pub(super) width: u32,
    pub(super) height: u32,
    pub(super) mip_levels: u32,
    pub(super) array_size: u32,
    pub(super) format: DxgiFormat,
    pub(super) sample_desc: SampleDesc,
    pub(super) usage: u32,
    pub(super) bind_flags: u32,
    pub(super) cpu_access_flags: u32,
    pub(super) misc_flags: u32,
}

/// `D3D11_BUFFER_DESC`
#[repr(C)]
#[derive(Clone, Copy, Default, Debug)]
pub(super) struct BufferDesc {
    pub(super) byte_width: u32,
    pub(super) usage: u32,
    pub(super) bind_flags: u32,
    pub(super) cpu_access_flags: u32,
    pub(super) misc_flags: u32,
    pub(super) structure_byte_stride: u32,
}

/// `D3D11_SUBRESOURCE_DATA`
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub(super) struct SubresourceData {
    pub(super) sys_mem: *const c_void,
    pub(super) sys_mem_pitch: u32,
    pub(super) sys_mem_slice_pitch: u32,
}

/// `D3D11_MAPPED_SUBRESOURCE`
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub(super) struct MappedSubresource {
    pub(super) data: *mut c_void,
    pub(super) row_pitch: u32,
    pub(super) depth_pitch: u32,
}

impl Default for MappedSubresource {
    fn default() -> MappedSubresource {
        MappedSubresource {
            data: null_mut(),
            row_pitch: 0,
            depth_pitch: 0,
        }
    }
}

/// `D3D11_SHADER_RESOURCE_VIEW_DESC`, with the `Texture2D` member
/// (`D3D11_TEX2D_SRV`) of its union; the other members are no larger
/// than four `UINT`s.
#[repr(C)]
#[derive(Clone, Copy, Default, Debug)]
pub(super) struct ShaderResourceViewDesc {
    pub(super) format: DxgiFormat,
    pub(super) view_dimension: u32,
    /// `Texture2D.MostDetailedMip`
    pub(super) most_detailed_mip: u32,
    /// `Texture2D.MipLevels`
    pub(super) mip_levels: u32,
    _rest_of_union: [u32; 2],
}

impl ShaderResourceViewDesc {
    /// A view of a 2D texture (`D3D11_SRV_DIMENSION_TEXTURE2D`).
    pub(super) fn texture_2d(
        format: DxgiFormat,
        most_detailed_mip: u32,
        mip_levels: u32,
    ) -> ShaderResourceViewDesc {
        ShaderResourceViewDesc {
            format,
            view_dimension: D3D11_SRV_DIMENSION_TEXTURE2D,
            most_detailed_mip,
            mip_levels,
            _rest_of_union: [0; 2],
        }
    }
}

/// `D3D11_RENDER_TARGET_VIEW_DESC`, with the `Texture2D` member
/// (`D3D11_TEX2D_RTV`) of its union.
#[repr(C)]
#[derive(Clone, Copy, Default, Debug)]
pub(super) struct RenderTargetViewDesc {
    pub(super) format: DxgiFormat,
    pub(super) view_dimension: u32,
    /// `Texture2D.MipSlice`
    pub(super) mip_slice: u32,
    _rest_of_union: [u32; 2],
}

impl RenderTargetViewDesc {
    /// A view of a 2D texture (`D3D11_RTV_DIMENSION_TEXTURE2D`).
    pub(super) fn texture_2d(format: DxgiFormat, mip_slice: u32) -> RenderTargetViewDesc {
        RenderTargetViewDesc {
            format,
            view_dimension: D3D11_RTV_DIMENSION_TEXTURE2D,
            mip_slice,
            _rest_of_union: [0; 2],
        }
    }
}

/// `D3D11_RENDER_TARGET_BLEND_DESC`
#[repr(C)]
#[derive(Clone, Copy, Default, Debug)]
pub(super) struct RenderTargetBlendDesc {
    pub(super) blend_enable: BOOL,
    pub(super) src_blend: D3d11Blend,
    pub(super) dest_blend: D3d11Blend,
    pub(super) blend_op: D3d11BlendOp,
    pub(super) src_blend_alpha: D3d11Blend,
    pub(super) dest_blend_alpha: D3d11Blend,
    pub(super) blend_op_alpha: D3d11BlendOp,
    pub(super) render_target_write_mask: u8,
}

/// `D3D11_BLEND_DESC`
#[repr(C)]
#[derive(Clone, Copy, Default, Debug)]
pub(super) struct BlendDesc {
    pub(super) alpha_to_coverage_enable: BOOL,
    pub(super) independent_blend_enable: BOOL,
    pub(super) render_target: [RenderTargetBlendDesc; 8],
}

/// `D3D11_RASTERIZER_DESC`
#[repr(C)]
#[derive(Clone, Copy, Default, Debug)]
pub(super) struct RasterizerDesc {
    pub(super) fill_mode: u32,
    pub(super) cull_mode: u32,
    pub(super) front_counter_clockwise: BOOL,
    pub(super) depth_bias: i32,
    pub(super) depth_bias_clamp: f32,
    pub(super) slope_scaled_depth_bias: f32,
    pub(super) depth_clip_enable: BOOL,
    pub(super) scissor_enable: BOOL,
    pub(super) multisample_enable: BOOL,
    pub(super) antialiased_line_enable: BOOL,
}

/// `D3D11_SAMPLER_DESC`
#[repr(C)]
#[derive(Clone, Copy, Default, Debug)]
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

/// `D3D11_VIEWPORT`
#[repr(C)]
#[derive(Clone, Copy, Default, Debug)]
pub(super) struct Viewport {
    pub(super) top_left_x: f32,
    pub(super) top_left_y: f32,
    pub(super) width: f32,
    pub(super) height: f32,
    pub(super) min_depth: f32,
    pub(super) max_depth: f32,
}

/// `D3D11_RECT` (a `RECT`)
pub(super) type D3d11Rect = RECT;

/// `D3D11_BOX`
#[repr(C)]
#[derive(Clone, Copy, Default, Debug)]
pub(super) struct D3d11Box {
    pub(super) left: u32,
    pub(super) top: u32,
    pub(super) front: u32,
    pub(super) right: u32,
    pub(super) bottom: u32,
    pub(super) back: u32,
}

/// `D3D11_INPUT_ELEMENT_DESC`
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub(super) struct InputElementDesc {
    pub(super) semantic_name: *const u8,
    pub(super) semantic_index: u32,
    pub(super) format: DxgiFormat,
    pub(super) input_slot: u32,
    pub(super) aligned_byte_offset: u32,
    pub(super) input_slot_class: u32,
    pub(super) instance_data_step_rate: u32,
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

/// `DXGI_PRESENT_PARAMETERS`
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub(super) struct PresentParameters {
    pub(super) dirty_rects_count: u32,
    pub(super) dirty_rects: *mut RECT,
    pub(super) scroll_rect: *mut RECT,
    pub(super) scroll_offset: *mut c_void,
}

impl Default for PresentParameters {
    fn default() -> PresentParameters {
        PresentParameters {
            dirty_rects_count: 0,
            dirty_rects: null_mut(),
            scroll_rect: null_mut(),
            scroll_offset: null_mut(),
        }
    }
}

/// `DXGI_ADAPTER_DESC`
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub(super) struct AdapterDesc {
    pub(super) description: [u16; 128],
    pub(super) vendor_id: u32,
    pub(super) device_id: u32,
    pub(super) sub_sys_id: u32,
    pub(super) revision: u32,
    pub(super) dedicated_video_memory: usize,
    pub(super) dedicated_system_memory: usize,
    pub(super) shared_system_memory: usize,
    /// `AdapterLuid` (a `LUID`: low part, high part)
    pub(super) adapter_luid: [u32; 2],
}

// --- entry points ---

/// `CreateDXGIFactory()`
pub(super) type PfnCreateDxgiFactory =
    unsafe extern "system" fn(*const GUID, *mut *mut c_void) -> HRESULT;
/// `CreateDXGIFactory2()` (and `DXGIGetDebugInterface1()`, which upstream
/// calls through the same type)
pub(super) type PfnCreateDxgiFactory2 =
    unsafe extern "system" fn(u32, *const GUID, *mut *mut c_void) -> HRESULT;
/// `PFN_D3D11_CREATE_DEVICE`
pub(super) type PfnD3d11CreateDevice = unsafe extern "system" fn(
    *mut c_void,
    u32,
    HMODULE,
    u32,
    *const D3dFeatureLevel,
    u32,
    u32,
    *mut *mut c_void,
    *mut D3dFeatureLevel,
    *mut *mut c_void,
) -> HRESULT;

// --- the COM interfaces SDL calls ---

/// An unused vtable slot.
type Slot = usize;

/// `IDXGIObject`'s methods: SetPrivateData, SetPrivateDataInterface,
/// GetPrivateData, GetParent.
type DxgiObjectSlots = [Slot; 4];

/// `ID3D11DeviceChild`'s methods: GetDevice, GetPrivateData,
/// SetPrivateData, SetPrivateDataInterface.
type DeviceChildSlots = [Slot; 4];

/// `IDXGIFactory2Vtbl` (the part SDL uses).
#[repr(C)]
pub(super) struct IDXGIFactory2Vtbl {
    pub(super) base: IUnknownVtbl,
    _object: DxgiObjectSlots,
    pub(super) enum_adapters:
        unsafe extern "system" fn(*mut c_void, u32, *mut *mut c_void) -> HRESULT,
    pub(super) make_window_association:
        unsafe extern "system" fn(*mut c_void, HWND, u32) -> HRESULT,
    /// GetWindowAssociation, CreateSwapChain, CreateSoftwareAdapter;
    /// IDXGIFactory1's EnumAdapters1, IsCurrent; IsWindowedStereoEnabled
    _get_window_association: [Slot; 6],
    pub(super) create_swap_chain_for_hwnd: unsafe extern "system" fn(
        *mut c_void,
        *mut c_void,
        HWND,
        *const SwapChainDesc1,
        *const c_void,
        *mut c_void,
        *mut *mut c_void,
    ) -> HRESULT,
    /// CreateSwapChainForCoreWindow, GetSharedResourceAdapterLuid,
    /// RegisterStereoStatusWindow, RegisterStereoStatusEvent,
    /// UnregisterStereoStatus, RegisterOcclusionStatusWindow,
    /// RegisterOcclusionStatusEvent, UnregisterOcclusionStatus,
    /// CreateSwapChainForComposition
    _create_swap_chain_for_core_window: [Slot; 9],
}

/// `IDXGIFactory5Vtbl` (the part SDL uses).
#[repr(C)]
pub(super) struct IDXGIFactory5Vtbl {
    pub(super) factory2: IDXGIFactory2Vtbl,
    /// IDXGIFactory3's GetCreationFlags; IDXGIFactory4's
    /// EnumAdapterByLuid, EnumWarpAdapter
    _get_creation_flags: [Slot; 3],
    pub(super) check_feature_support:
        unsafe extern "system" fn(*mut c_void, u32, *mut c_void, u32) -> HRESULT,
}

/// `IDXGIAdapterVtbl` (the part SDL uses).
#[repr(C)]
pub(super) struct IDXGIAdapterVtbl {
    pub(super) base: IUnknownVtbl,
    _object: DxgiObjectSlots,
    _enum_outputs: Slot,
    pub(super) get_desc: unsafe extern "system" fn(*mut c_void, *mut AdapterDesc) -> HRESULT,
}

/// `IDXGIDevice1Vtbl` (the part SDL uses).
#[repr(C)]
pub(super) struct IDXGIDevice1Vtbl {
    pub(super) base: IUnknownVtbl,
    _object: DxgiObjectSlots,
    pub(super) get_adapter: unsafe extern "system" fn(*mut c_void, *mut *mut c_void) -> HRESULT,
    /// CreateSurface, QueryResourceResidency, SetGPUThreadPriority,
    /// GetGPUThreadPriority
    _create_surface: [Slot; 4],
    pub(super) set_maximum_frame_latency: unsafe extern "system" fn(*mut c_void, u32) -> HRESULT,
}

/// `IDXGISwapChain1Vtbl` (the part SDL uses).
#[repr(C)]
pub(super) struct IDXGISwapChain1Vtbl {
    pub(super) base: IUnknownVtbl,
    _object: DxgiObjectSlots,
    /// IDXGIDeviceSubObject's GetDevice; Present
    _get_device: [Slot; 2],
    pub(super) get_buffer:
        unsafe extern "system" fn(*mut c_void, u32, *const GUID, *mut *mut c_void) -> HRESULT,
    _set_fullscreen_state: Slot,
    pub(super) get_fullscreen_state:
        unsafe extern "system" fn(*mut c_void, *mut BOOL, *mut *mut c_void) -> HRESULT,
    _get_desc: Slot,
    pub(super) resize_buffers:
        unsafe extern "system" fn(*mut c_void, u32, u32, u32, DxgiFormat, u32) -> HRESULT,
    /// ResizeTarget, GetContainingOutput, GetFrameStatistics,
    /// GetLastPresentCount, GetDesc1, GetFullscreenDesc, GetHwnd,
    /// GetCoreWindow
    _resize_target: [Slot; 8],
    pub(super) present1:
        unsafe extern "system" fn(*mut c_void, u32, u32, *const PresentParameters) -> HRESULT,
    /// IsTemporaryMonoSupported, GetRestrictToOutput, SetBackgroundColor,
    /// GetBackgroundColor
    _is_temporary_mono_supported: [Slot; 4],
    pub(super) set_rotation: unsafe extern "system" fn(*mut c_void, DxgiModeRotation) -> HRESULT,
    _get_rotation: Slot,
}

/// `IDXGISwapChain3Vtbl` (the part SDL uses).
#[repr(C)]
pub(super) struct IDXGISwapChain3Vtbl {
    pub(super) swap_chain1: IDXGISwapChain1Vtbl,
    /// IDXGISwapChain2's SetSourceSize, GetSourceSize,
    /// SetMaximumFrameLatency, GetMaximumFrameLatency,
    /// GetFrameLatencyWaitableObject, SetMatrixTransform,
    /// GetMatrixTransform; GetCurrentBackBufferIndex
    _set_source_size: [Slot; 8],
    pub(super) check_color_space_support:
        unsafe extern "system" fn(*mut c_void, DxgiColorSpaceType, *mut u32) -> HRESULT,
    pub(super) set_color_space1:
        unsafe extern "system" fn(*mut c_void, DxgiColorSpaceType) -> HRESULT,
    _resize_buffers1: Slot,
}

/// `IDXGIDebugVtbl`.
#[repr(C)]
pub(super) struct IDXGIDebugVtbl {
    pub(super) base: IUnknownVtbl,
    pub(super) report_live_objects: unsafe extern "system" fn(*mut c_void, GUID, u32) -> HRESULT,
}

/// `IDXGIInfoQueueVtbl` (the part SDL uses).
#[repr(C)]
pub(super) struct IDXGIInfoQueueVtbl {
    pub(super) base: IUnknownVtbl,
    /// SetMessageCountLimit ... SetBreakOnCategory (30 methods)
    _set_message_count_limit: [Slot; 30],
    pub(super) set_break_on_severity:
        unsafe extern "system" fn(*mut c_void, GUID, u32, BOOL) -> HRESULT,
}

/// `ID3D11DeviceVtbl` (the part SDL uses; `ID3D11Device1`'s starts with it).
#[repr(C)]
pub(super) struct ID3D11DeviceVtbl {
    pub(super) base: IUnknownVtbl,
    pub(super) create_buffer: unsafe extern "system" fn(
        *mut c_void,
        *const BufferDesc,
        *const SubresourceData,
        *mut *mut c_void,
    ) -> HRESULT,
    _create_texture_1d: Slot,
    pub(super) create_texture_2d: unsafe extern "system" fn(
        *mut c_void,
        *const Texture2dDesc,
        *const SubresourceData,
        *mut *mut c_void,
    ) -> HRESULT,
    _create_texture_3d: Slot,
    pub(super) create_shader_resource_view: unsafe extern "system" fn(
        *mut c_void,
        *mut c_void,
        *const ShaderResourceViewDesc,
        *mut *mut c_void,
    ) -> HRESULT,
    _create_unordered_access_view: Slot,
    pub(super) create_render_target_view: unsafe extern "system" fn(
        *mut c_void,
        *mut c_void,
        *const RenderTargetViewDesc,
        *mut *mut c_void,
    ) -> HRESULT,
    _create_depth_stencil_view: Slot,
    pub(super) create_input_layout: unsafe extern "system" fn(
        *mut c_void,
        *const InputElementDesc,
        u32,
        *const c_void,
        usize,
        *mut *mut c_void,
    ) -> HRESULT,
    pub(super) create_vertex_shader: unsafe extern "system" fn(
        *mut c_void,
        *const c_void,
        usize,
        *mut c_void,
        *mut *mut c_void,
    ) -> HRESULT,
    /// CreateGeometryShader, CreateGeometryShaderWithStreamOutput
    _create_geometry_shader: [Slot; 2],
    pub(super) create_pixel_shader: unsafe extern "system" fn(
        *mut c_void,
        *const c_void,
        usize,
        *mut c_void,
        *mut *mut c_void,
    ) -> HRESULT,
    /// CreateHullShader, CreateDomainShader, CreateComputeShader,
    /// CreateClassLinkage
    _create_hull_shader: [Slot; 4],
    pub(super) create_blend_state:
        unsafe extern "system" fn(*mut c_void, *const BlendDesc, *mut *mut c_void) -> HRESULT,
    _create_depth_stencil_state: Slot,
    pub(super) create_rasterizer_state:
        unsafe extern "system" fn(*mut c_void, *const RasterizerDesc, *mut *mut c_void) -> HRESULT,
    pub(super) create_sampler_state:
        unsafe extern "system" fn(*mut c_void, *const SamplerDesc, *mut *mut c_void) -> HRESULT,
    /// CreateQuery, CreatePredicate, CreateCounter, CreateDeferredContext,
    /// OpenSharedResource
    _create_query: [Slot; 5],
    pub(super) check_format_support:
        unsafe extern "system" fn(*mut c_void, DxgiFormat, *mut u32) -> HRESULT,
}

/// `ID3D11DeviceContextVtbl` (the part SDL uses; `ID3D11DeviceContext1`'s
/// starts with it).
#[repr(C)]
pub(super) struct ID3D11DeviceContextVtbl {
    pub(super) base: IUnknownVtbl,
    _device_child: DeviceChildSlots,
    pub(super) vs_set_constant_buffers:
        unsafe extern "system" fn(*mut c_void, u32, u32, *const *mut c_void),
    pub(super) ps_set_shader_resources:
        unsafe extern "system" fn(*mut c_void, u32, u32, *const *mut c_void),
    pub(super) ps_set_shader:
        unsafe extern "system" fn(*mut c_void, *mut c_void, *const *mut c_void, u32),
    pub(super) ps_set_samplers:
        unsafe extern "system" fn(*mut c_void, u32, u32, *const *mut c_void),
    pub(super) vs_set_shader:
        unsafe extern "system" fn(*mut c_void, *mut c_void, *const *mut c_void, u32),
    _draw_indexed: Slot,
    pub(super) draw: unsafe extern "system" fn(*mut c_void, u32, u32),
    pub(super) map: unsafe extern "system" fn(
        *mut c_void,
        *mut c_void,
        u32,
        u32,
        u32,
        *mut MappedSubresource,
    ) -> HRESULT,
    pub(super) unmap: unsafe extern "system" fn(*mut c_void, *mut c_void, u32),
    pub(super) ps_set_constant_buffers:
        unsafe extern "system" fn(*mut c_void, u32, u32, *const *mut c_void),
    pub(super) ia_set_input_layout: unsafe extern "system" fn(*mut c_void, *mut c_void),
    pub(super) ia_set_vertex_buffers: unsafe extern "system" fn(
        *mut c_void,
        u32,
        u32,
        *const *mut c_void,
        *const u32,
        *const u32,
    ),
    /// IASetIndexBuffer, DrawIndexedInstanced, DrawInstanced,
    /// GSSetConstantBuffers, GSSetShader
    _ia_set_index_buffer: [Slot; 5],
    pub(super) ia_set_primitive_topology:
        unsafe extern "system" fn(*mut c_void, D3d11PrimitiveTopology),
    /// VSSetShaderResources, VSSetSamplers, Begin, End, GetData,
    /// SetPredication, GSSetShaderResources, GSSetSamplers
    _vs_set_shader_resources: [Slot; 8],
    pub(super) om_set_render_targets:
        unsafe extern "system" fn(*mut c_void, u32, *const *mut c_void, *mut c_void),
    _om_set_render_targets_and_unordered_access_views: Slot,
    pub(super) om_set_blend_state:
        unsafe extern "system" fn(*mut c_void, *mut c_void, *const f32, u32),
    /// OMSetDepthStencilState, SOSetTargets, DrawAuto,
    /// DrawIndexedInstancedIndirect, DrawInstancedIndirect, Dispatch,
    /// DispatchIndirect
    _om_set_depth_stencil_state: [Slot; 7],
    pub(super) rs_set_state: unsafe extern "system" fn(*mut c_void, *mut c_void),
    pub(super) rs_set_viewports: unsafe extern "system" fn(*mut c_void, u32, *const Viewport),
    pub(super) rs_set_scissor_rects: unsafe extern "system" fn(*mut c_void, u32, *const D3d11Rect),
    pub(super) copy_subresource_region: unsafe extern "system" fn(
        *mut c_void,
        *mut c_void,
        u32,
        u32,
        u32,
        u32,
        *mut c_void,
        u32,
        *const D3d11Box,
    ),
    _copy_resource: Slot,
    pub(super) update_subresource: unsafe extern "system" fn(
        *mut c_void,
        *mut c_void,
        u32,
        *const D3d11Box,
        *const c_void,
        u32,
        u32,
    ),
    _copy_structure_count: Slot,
    pub(super) clear_render_target_view:
        unsafe extern "system" fn(*mut c_void, *mut c_void, *const f32),
    /// ClearUnorderedAccessViewUint ... GetContextFlags (59 methods)
    _clear_unordered_access_view_uint: [Slot; 59],
    pub(super) clear_state: unsafe extern "system" fn(*mut c_void),
    pub(super) flush: unsafe extern "system" fn(*mut c_void),
}

/// `ID3D11Texture2DVtbl`.
#[repr(C)]
pub(super) struct ID3D11Texture2DVtbl {
    pub(super) base: IUnknownVtbl,
    _device_child: DeviceChildSlots,
    /// ID3D11Resource's GetType, SetEvictionPriority, GetEvictionPriority
    _get_type: [Slot; 3],
    pub(super) get_desc: unsafe extern "system" fn(*mut c_void, *mut Texture2dDesc),
}

/// `ID3D11ViewVtbl`, which `ID3D11RenderTargetView`'s starts with (its
/// GetDesc follows).
#[repr(C)]
pub(super) struct ID3D11ViewVtbl {
    pub(super) base: IUnknownVtbl,
    _device_child: DeviceChildSlots,
    pub(super) get_resource: unsafe extern "system" fn(*mut c_void, *mut *mut c_void),
}

/// The vtable of an interface SDL calls none of the methods of (states,
/// shaders, views, buffers, layouts): just its `IUnknown` part.
#[repr(C)]
pub(super) struct OpaqueVtbl {
    pub(super) base: IUnknownVtbl,
}

pub(super) type DxgiFactory2 = ComPtr<IDXGIFactory2Vtbl>;
pub(super) type DxgiFactory5 = ComPtr<IDXGIFactory5Vtbl>;
pub(super) type DxgiAdapter = ComPtr<IDXGIAdapterVtbl>;
pub(super) type DxgiDevice1 = ComPtr<IDXGIDevice1Vtbl>;
pub(super) type DxgiSwapChain1 = ComPtr<IDXGISwapChain1Vtbl>;
pub(super) type DxgiSwapChain3 = ComPtr<IDXGISwapChain3Vtbl>;
pub(super) type DxgiDebug = ComPtr<IDXGIDebugVtbl>;
pub(super) type DxgiInfoQueue = ComPtr<IDXGIInfoQueueVtbl>;
pub(super) type Device = ComPtr<ID3D11DeviceVtbl>;
pub(super) type DeviceContext = ComPtr<ID3D11DeviceContextVtbl>;
pub(super) type Texture2d = ComPtr<ID3D11Texture2DVtbl>;
pub(super) type RenderTargetView = ComPtr<ID3D11ViewVtbl>;
pub(super) type Buffer = ComPtr<OpaqueVtbl>;
pub(super) type ShaderResourceView = ComPtr<OpaqueVtbl>;
pub(super) type InputLayout = ComPtr<OpaqueVtbl>;
pub(super) type VertexShader = ComPtr<OpaqueVtbl>;
pub(super) type PixelShader = ComPtr<OpaqueVtbl>;
pub(super) type BlendState = ComPtr<OpaqueVtbl>;
pub(super) type RasterizerState = ComPtr<OpaqueVtbl>;
pub(super) type SamplerState = ComPtr<OpaqueVtbl>;

/// `HRESULT` to `Result` (`SUCCEEDED()`).
pub(super) fn check(hr: HRESULT) -> Result<(), HRESULT> {
    if hr < 0 {
        Err(hr)
    } else {
        Ok(())
    }
}

/// The interface pointer of `object`, as the C code passes it.
pub(super) fn raw<V>(object: &ComPtr<V>) -> *mut c_void {
    object.as_ptr().cast()
}

/// The interface pointer of an optional object (NULL for `None`).
pub(super) fn raw_or_null<V>(object: Option<&ComPtr<V>>) -> *mut c_void {
    object.map_or(null_mut(), raw)
}

/// Take a new reference from an out parameter.
///
/// # Safety
///
/// `f` must store NULL or an owned reference to an object whose vtable is
/// a `V` on success.
pub(super) unsafe fn out<V>(
    f: impl FnOnce(*mut *mut c_void) -> HRESULT,
) -> Result<ComPtr<V>, HRESULT> {
    // SAFETY: the caller's contract.
    unsafe { ComPtr::from_out(|p: *mut *mut ComObject<V>| f(p.cast())) }
}

impl DxgiFactory2 {
    /// `IDXGIFactory::EnumAdapters()`
    pub(super) fn enum_adapters(&self, adapter: u32) -> Result<DxgiAdapter, HRESULT> {
        let f = self.vtbl().enum_adapters;
        // SAFETY: a live factory; EnumAdapters stores an owned adapter.
        unsafe { out(|o| f(raw(self), adapter, o)) }
    }

    /// `IDXGIFactory::MakeWindowAssociation()`
    pub(super) fn make_window_association(&self, hwnd: HWND, flags: u32) -> HRESULT {
        // SAFETY: a live factory and a window handle.
        unsafe { (self.vtbl().make_window_association)(raw(self), hwnd, flags) }
    }

    /// `IDXGIFactory2::CreateSwapChainForHwnd()`, for all displays and
    /// without a fullscreen description.
    pub(super) fn create_swap_chain_for_hwnd(
        &self,
        device: &Device,
        hwnd: HWND,
        desc: &SwapChainDesc1,
    ) -> Result<DxgiSwapChain1, HRESULT> {
        let f = self.vtbl().create_swap_chain_for_hwnd;
        // SAFETY: a live factory and device; the description outlives the
        // call; the swap chain is stored owned.
        unsafe { out(|o| f(raw(self), raw(device), hwnd, desc, null(), null_mut(), o)) }
    }
}

impl DxgiFactory5 {
    /// `IDXGIFactory5::CheckFeatureSupport()` for a `BOOL` feature.
    pub(super) fn check_feature_support_bool(&self, feature: u32) -> Result<bool, HRESULT> {
        let mut value: BOOL = 0;
        // SAFETY: a live factory; the BOOL is the feature's data, of its size.
        check(unsafe {
            (self.vtbl().check_feature_support)(
                raw(self),
                feature,
                (&mut value as *mut BOOL).cast(),
                size_of::<BOOL>() as u32,
            )
        })?;
        Ok(value != 0)
    }
}

impl DxgiAdapter {
    /// `IDXGIAdapter::GetDesc()` (for the hardware check: SDL doesn't call it)
    #[cfg_attr(not(test), allow(dead_code))]
    pub(super) fn desc(&self) -> Result<AdapterDesc, HRESULT> {
        let mut desc = AdapterDesc {
            description: [0; 128],
            vendor_id: 0,
            device_id: 0,
            sub_sys_id: 0,
            revision: 0,
            dedicated_video_memory: 0,
            dedicated_system_memory: 0,
            shared_system_memory: 0,
            adapter_luid: [0; 2],
        };
        // SAFETY: a live adapter; a valid output.
        check(unsafe { (self.vtbl().get_desc)(raw(self), &mut desc) })?;
        Ok(desc)
    }
}

impl DxgiDevice1 {
    /// `IDXGIDevice::GetAdapter()` (for the hardware check: SDL doesn't call it)
    #[cfg_attr(not(test), allow(dead_code))]
    pub(super) fn adapter(&self) -> Result<DxgiAdapter, HRESULT> {
        let f = self.vtbl().get_adapter;
        // SAFETY: a live device; GetAdapter stores an owned adapter.
        unsafe { out(|o| f(raw(self), o)) }
    }

    /// `IDXGIDevice1::SetMaximumFrameLatency()`
    pub(super) fn set_maximum_frame_latency(&self, max_latency: u32) -> HRESULT {
        // SAFETY: a live device.
        unsafe { (self.vtbl().set_maximum_frame_latency)(raw(self), max_latency) }
    }
}

impl DxgiSwapChain1 {
    /// `IDXGISwapChain::GetBuffer()` of a texture.
    pub(super) fn buffer(&self, index: u32) -> Result<Texture2d, HRESULT> {
        let f = self.vtbl().get_buffer;
        // SAFETY: a live swap chain; GetBuffer stores an owned
        // ID3D11Texture2D, the interface asked for.
        unsafe { out(|o| f(raw(self), index, &IID_ID3D11TEXTURE2D, o)) }
    }

    /// `IDXGISwapChain::GetFullscreenState()` (without the output).
    pub(super) fn fullscreen_state(&self) -> Result<bool, HRESULT> {
        let mut state: BOOL = 0;
        // SAFETY: a live swap chain; a NULL output pointer is allowed.
        check(unsafe { (self.vtbl().get_fullscreen_state)(raw(self), &mut state, null_mut()) })?;
        Ok(state != 0)
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

    /// `IDXGISwapChain1::Present1()`
    pub(super) fn present1(
        &self,
        sync_interval: u32,
        flags: u32,
        parameters: &PresentParameters,
    ) -> HRESULT {
        // SAFETY: a live swap chain; the parameters outlive the call.
        unsafe { (self.vtbl().present1)(raw(self), sync_interval, flags, parameters) }
    }

    /// `IDXGISwapChain1::SetRotation()`
    pub(super) fn set_rotation(&self, rotation: DxgiModeRotation) -> HRESULT {
        // SAFETY: a live swap chain.
        unsafe { (self.vtbl().set_rotation)(raw(self), rotation) }
    }
}

impl DxgiSwapChain3 {
    /// `IDXGISwapChain3::CheckColorSpaceSupport()`
    pub(super) fn check_color_space_support(
        &self,
        color_space: DxgiColorSpaceType,
    ) -> Result<u32, HRESULT> {
        let mut support = 0;
        // SAFETY: the vtable has the slot (see where the interface is
        // queried); a valid output.
        check(unsafe {
            (self.vtbl().check_color_space_support)(raw(self), color_space, &mut support)
        })?;
        Ok(support)
    }

    /// `IDXGISwapChain3::SetColorSpace1()`
    pub(super) fn set_color_space1(&self, color_space: DxgiColorSpaceType) -> HRESULT {
        // SAFETY: as for check_color_space_support().
        unsafe { (self.vtbl().set_color_space1)(raw(self), color_space) }
    }
}

impl DxgiDebug {
    /// `IDXGIDebug::ReportLiveObjects()`
    pub(super) fn report_live_objects(&self, apiid: GUID, flags: u32) -> HRESULT {
        // SAFETY: a live debug interface.
        unsafe { (self.vtbl().report_live_objects)(raw(self), apiid, flags) }
    }
}

impl DxgiInfoQueue {
    /// `IDXGIInfoQueue::SetBreakOnSeverity()`
    pub(super) fn set_break_on_severity(&self, producer: GUID, severity: u32, enable: bool) {
        // SAFETY: a live info queue.
        unsafe {
            (self.vtbl().set_break_on_severity)(raw(self), producer, severity, enable as BOOL)
        };
    }
}

/// An `ID3D11Resource`: a texture or a buffer.
pub(super) trait Resource {
    /// The `ID3D11Resource` pointer of the object.
    fn resource(&self) -> *mut c_void;
}

impl Resource for Texture2d {
    fn resource(&self) -> *mut c_void {
        raw(self)
    }
}

impl Resource for Buffer {
    fn resource(&self) -> *mut c_void {
        raw(self)
    }
}

impl Device {
    /// `ID3D11Device::CreateBuffer()`
    pub(super) fn create_buffer(
        &self,
        desc: &BufferDesc,
        initial: Option<&SubresourceData>,
    ) -> Result<Buffer, HRESULT> {
        let f = self.vtbl().create_buffer;
        let initial = initial.map_or(null(), |d| d as *const SubresourceData);
        // SAFETY: a live device; the description and the initial data
        // (ByteWidth bytes, which the callers provide) outlive the call.
        unsafe { out(|o| f(raw(self), desc, initial, o)) }
    }

    /// `ID3D11Device::CreateTexture2D()` (without initial data).
    pub(super) fn create_texture_2d(&self, desc: &Texture2dDesc) -> Result<Texture2d, HRESULT> {
        let f = self.vtbl().create_texture_2d;
        // SAFETY: a live device; the description outlives the call.
        unsafe { out(|o| f(raw(self), desc, null(), o)) }
    }

    /// `ID3D11Device::CreateShaderResourceView()`
    pub(super) fn create_shader_resource_view(
        &self,
        resource: &impl Resource,
        desc: &ShaderResourceViewDesc,
    ) -> Result<ShaderResourceView, HRESULT> {
        let f = self.vtbl().create_shader_resource_view;
        // SAFETY: a live device and resource; the description outlives the
        // call.
        unsafe { out(|o| f(raw(self), resource.resource(), desc, o)) }
    }

    /// `ID3D11Device::CreateRenderTargetView()`
    pub(super) fn create_render_target_view(
        &self,
        resource: &impl Resource,
        desc: Option<&RenderTargetViewDesc>,
    ) -> Result<RenderTargetView, HRESULT> {
        let f = self.vtbl().create_render_target_view;
        let desc = desc.map_or(null(), |d| d as *const RenderTargetViewDesc);
        // SAFETY: a live device and resource; the description, if any,
        // outlives the call.
        unsafe { out(|o| f(raw(self), resource.resource(), desc, o)) }
    }

    /// `ID3D11Device::CreateInputLayout()`
    pub(super) fn create_input_layout(
        &self,
        elements: &[InputElementDesc],
        bytecode: &[u8],
    ) -> Result<InputLayout, HRESULT> {
        let f = self.vtbl().create_input_layout;
        // SAFETY: a live device; the elements (and their NUL-terminated
        // semantic names) and the bytecode outlive the call.
        unsafe {
            out(|o| {
                f(
                    raw(self),
                    elements.as_ptr(),
                    elements.len() as u32,
                    bytecode.as_ptr().cast(),
                    bytecode.len(),
                    o,
                )
            })
        }
    }

    /// `ID3D11Device::CreateVertexShader()` (without a class linkage).
    pub(super) fn create_vertex_shader(&self, bytecode: &[u8]) -> Result<VertexShader, HRESULT> {
        let f = self.vtbl().create_vertex_shader;
        // SAFETY: a live device; the bytecode outlives the call.
        unsafe {
            out(|o| {
                f(
                    raw(self),
                    bytecode.as_ptr().cast(),
                    bytecode.len(),
                    null_mut(),
                    o,
                )
            })
        }
    }

    /// `ID3D11Device::CreatePixelShader()` (without a class linkage).
    pub(super) fn create_pixel_shader(&self, bytecode: &[u8]) -> Result<PixelShader, HRESULT> {
        let f = self.vtbl().create_pixel_shader;
        // SAFETY: a live device; the bytecode outlives the call.
        unsafe {
            out(|o| {
                f(
                    raw(self),
                    bytecode.as_ptr().cast(),
                    bytecode.len(),
                    null_mut(),
                    o,
                )
            })
        }
    }

    /// `ID3D11Device::CreateBlendState()`
    pub(super) fn create_blend_state(&self, desc: &BlendDesc) -> Result<BlendState, HRESULT> {
        let f = self.vtbl().create_blend_state;
        // SAFETY: a live device; the description outlives the call.
        unsafe { out(|o| f(raw(self), desc, o)) }
    }

    /// `ID3D11Device::CreateRasterizerState()`
    pub(super) fn create_rasterizer_state(
        &self,
        desc: &RasterizerDesc,
    ) -> Result<RasterizerState, HRESULT> {
        let f = self.vtbl().create_rasterizer_state;
        // SAFETY: a live device; the description outlives the call.
        unsafe { out(|o| f(raw(self), desc, o)) }
    }

    /// `ID3D11Device::CreateSamplerState()`
    pub(super) fn create_sampler_state(&self, desc: &SamplerDesc) -> Result<SamplerState, HRESULT> {
        let f = self.vtbl().create_sampler_state;
        // SAFETY: a live device; the description outlives the call.
        unsafe { out(|o| f(raw(self), desc, o)) }
    }

    /// `ID3D11Device::CheckFormatSupport()`
    pub(super) fn check_format_support(&self, format: DxgiFormat) -> Result<u32, HRESULT> {
        let mut support = 0;
        // SAFETY: a live device; a valid output.
        check(unsafe { (self.vtbl().check_format_support)(raw(self), format, &mut support) })?;
        Ok(support)
    }
}

impl DeviceContext {
    /// `ID3D11DeviceContext::VSSetConstantBuffers()`
    pub(super) fn vs_set_constant_buffers(&self, start: u32, buffers: &[*mut c_void]) {
        // SAFETY: a live context; the pointers are live buffers (or NULL).
        unsafe {
            (self.vtbl().vs_set_constant_buffers)(
                raw(self),
                start,
                buffers.len() as u32,
                buffers.as_ptr(),
            )
        }
    }

    /// `ID3D11DeviceContext::PSSetShaderResources()`
    pub(super) fn ps_set_shader_resources(&self, start: u32, views: &[*mut c_void]) {
        // SAFETY: a live context; the pointers are live views (or NULL).
        unsafe {
            (self.vtbl().ps_set_shader_resources)(
                raw(self),
                start,
                views.len() as u32,
                views.as_ptr(),
            )
        }
    }

    /// `ID3D11DeviceContext::PSSetShader()` (without class instances).
    pub(super) fn ps_set_shader(&self, shader: &PixelShader) {
        // SAFETY: a live context and shader.
        unsafe { (self.vtbl().ps_set_shader)(raw(self), raw(shader), null(), 0) }
    }

    /// `ID3D11DeviceContext::PSSetSamplers()`
    pub(super) fn ps_set_samplers(&self, start: u32, samplers: &[*mut c_void]) {
        // SAFETY: a live context; the pointers are live samplers (or NULL).
        unsafe {
            (self.vtbl().ps_set_samplers)(
                raw(self),
                start,
                samplers.len() as u32,
                samplers.as_ptr(),
            )
        }
    }

    /// `ID3D11DeviceContext::VSSetShader()` (without class instances).
    pub(super) fn vs_set_shader(&self, shader: &VertexShader) {
        // SAFETY: a live context and shader.
        unsafe { (self.vtbl().vs_set_shader)(raw(self), raw(shader), null(), 0) }
    }

    /// `ID3D11DeviceContext::Draw()`
    pub(super) fn draw(&self, vertex_count: u32, start_vertex_location: u32) {
        // SAFETY: a live context.
        unsafe { (self.vtbl().draw)(raw(self), vertex_count, start_vertex_location) }
    }

    /// `ID3D11DeviceContext::Map()` of subresource 0.
    pub(super) fn map(
        &self,
        resource: &impl Resource,
        map_type: u32,
    ) -> Result<MappedSubresource, HRESULT> {
        let mut mapped = MappedSubresource::default();
        // SAFETY: a live context and resource; a valid output.
        check(unsafe {
            (self.vtbl().map)(raw(self), resource.resource(), 0, map_type, 0, &mut mapped)
        })?;
        Ok(mapped)
    }

    /// `ID3D11DeviceContext::Unmap()` of subresource 0.
    pub(super) fn unmap(&self, resource: &impl Resource) {
        // SAFETY: a live context and resource.
        unsafe { (self.vtbl().unmap)(raw(self), resource.resource(), 0) }
    }

    /// `ID3D11DeviceContext::PSSetConstantBuffers()`
    pub(super) fn ps_set_constant_buffers(&self, start: u32, buffers: &[*mut c_void]) {
        // SAFETY: a live context; the pointers are live buffers (or NULL).
        unsafe {
            (self.vtbl().ps_set_constant_buffers)(
                raw(self),
                start,
                buffers.len() as u32,
                buffers.as_ptr(),
            )
        }
    }

    /// `ID3D11DeviceContext::IASetInputLayout()`
    pub(super) fn ia_set_input_layout(&self, layout: &InputLayout) {
        // SAFETY: a live context and layout.
        unsafe { (self.vtbl().ia_set_input_layout)(raw(self), raw(layout)) }
    }

    /// `ID3D11DeviceContext::IASetVertexBuffers()` of one buffer.
    pub(super) fn ia_set_vertex_buffer(
        &self,
        slot: u32,
        buffer: &Buffer,
        stride: u32,
        offset: u32,
    ) {
        let buffers = [raw(buffer)];
        // SAFETY: a live context and buffer; one stride and one offset.
        unsafe {
            (self.vtbl().ia_set_vertex_buffers)(
                raw(self),
                slot,
                1,
                buffers.as_ptr(),
                &stride,
                &offset,
            )
        }
    }

    /// `ID3D11DeviceContext::IASetPrimitiveTopology()`
    pub(super) fn ia_set_primitive_topology(&self, topology: D3d11PrimitiveTopology) {
        // SAFETY: a live context.
        unsafe { (self.vtbl().ia_set_primitive_topology)(raw(self), topology) }
    }

    /// `ID3D11DeviceContext::OMSetRenderTargets()` (without a depth
    /// stencil view).
    pub(super) fn om_set_render_targets(&self, views: &[*mut c_void]) {
        let views_ptr = if views.is_empty() {
            null()
        } else {
            views.as_ptr()
        };
        // SAFETY: a live context; the pointers are live views (or NULL).
        unsafe {
            (self.vtbl().om_set_render_targets)(
                raw(self),
                views.len() as u32,
                views_ptr,
                null_mut(),
            )
        }
    }

    /// `ID3D11DeviceContext::OMSetBlendState()` (blend factor NULL).
    pub(super) fn om_set_blend_state(&self, state: Option<&BlendState>, sample_mask: u32) {
        // SAFETY: a live context; the state, if any, is live.
        unsafe {
            (self.vtbl().om_set_blend_state)(raw(self), raw_or_null(state), null(), sample_mask)
        }
    }

    /// `ID3D11DeviceContext::RSSetState()`
    pub(super) fn rs_set_state(&self, state: &RasterizerState) {
        // SAFETY: a live context and state.
        unsafe { (self.vtbl().rs_set_state)(raw(self), raw(state)) }
    }

    /// `ID3D11DeviceContext::RSSetViewports()`
    pub(super) fn rs_set_viewports(&self, viewports: &[Viewport]) {
        // SAFETY: a live context; the viewports outlive the call.
        unsafe {
            (self.vtbl().rs_set_viewports)(raw(self), viewports.len() as u32, viewports.as_ptr())
        }
    }

    /// `ID3D11DeviceContext::RSSetScissorRects()`
    pub(super) fn rs_set_scissor_rects(&self, rects: &[D3d11Rect]) {
        let rects_ptr = if rects.is_empty() {
            null()
        } else {
            rects.as_ptr()
        };
        // SAFETY: a live context; the rectangles outlive the call.
        unsafe { (self.vtbl().rs_set_scissor_rects)(raw(self), rects.len() as u32, rects_ptr) }
    }

    /// `ID3D11DeviceContext::CopySubresourceRegion()` of subresources 0.
    pub(super) fn copy_subresource_region(
        &self,
        dst: &impl Resource,
        (dst_x, dst_y): (u32, u32),
        src: &impl Resource,
        src_box: Option<&D3d11Box>,
    ) {
        let src_box = src_box.map_or(null(), |b| b as *const D3d11Box);
        // SAFETY: a live context and resources; the box, if any, outlives
        // the call.
        unsafe {
            (self.vtbl().copy_subresource_region)(
                raw(self),
                dst.resource(),
                0,
                dst_x,
                dst_y,
                0,
                src.resource(),
                0,
                src_box,
            )
        }
    }

    /// `ID3D11DeviceContext::UpdateSubresource()` of a whole buffer.
    pub(super) fn update_subresource<T: Copy>(&self, dst: &Buffer, data: &T) {
        // SAFETY: a live context and buffer of `size_of::<T>()` bytes; the
        // data outlives the call.
        unsafe {
            (self.vtbl().update_subresource)(
                raw(self),
                raw(dst),
                0,
                null(),
                (data as *const T).cast(),
                0,
                0,
            )
        }
    }

    /// `ID3D11DeviceContext::ClearRenderTargetView()`
    pub(super) fn clear_render_target_view(&self, view: &RenderTargetView, color: &[f32; 4]) {
        // SAFETY: a live context and view; four floats.
        unsafe { (self.vtbl().clear_render_target_view)(raw(self), raw(view), color.as_ptr()) }
    }

    /// `ID3D11DeviceContext::ClearState()`
    pub(super) fn clear_state(&self) {
        // SAFETY: a live context.
        unsafe { (self.vtbl().clear_state)(raw(self)) }
    }

    /// `ID3D11DeviceContext::Flush()`
    pub(super) fn flush(&self) {
        // SAFETY: a live context.
        unsafe { (self.vtbl().flush)(raw(self)) }
    }
}

impl Texture2d {
    /// `ID3D11Texture2D::GetDesc()`
    pub(super) fn desc(&self) -> Texture2dDesc {
        let mut desc = Texture2dDesc::default();
        // SAFETY: a live texture; a valid output.
        unsafe { (self.vtbl().get_desc)(raw(self), &mut desc) };
        desc
    }
}

impl RenderTargetView {
    /// `ID3D11View::GetResource()` of a texture's view (`None` for NULL).
    pub(super) fn resource_texture(&self) -> Option<Texture2d> {
        let f = self.vtbl().get_resource;
        let mut resource: *mut c_void = null_mut();
        // SAFETY: a live view; GetResource stores an owned reference to the
        // view's resource, which is a texture for render target views.
        unsafe {
            f(raw(self), &mut resource);
            ComPtr::from_raw(resource.cast())
        }
    }
}
