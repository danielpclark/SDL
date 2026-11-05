// The OpenXR and Vulkan declarations of the test runtime.
// Part of the repository's own test code (not a translation); see LICENSE.txt.

//! Interface facts only (names, values and layouts), declared by hand for
//! what the runtime implements: the loader's negotiation structures, the
//! core structures of the functions it serves, `XR_KHR_vulkan_enable2`'s
//! and `XR_KHR_D3D12_enable`'s, the few Vulkan structures and functions it
//! calls through the application's `vkGetInstanceProcAddr`, and the few
//! DXGI and Direct3D 12 ones it calls (on Windows: `dxgi.dll`'s factory,
//! and the application's device). The layouts are pinned by the tests
//! below to what a C program built against the OpenXR 1.1.63 and Vulkan
//! headers upstream SDL vendors printed on x86-64 Linux (and, for the
//! Direct3D 12 ones, a mingw-w64 one against the Windows headers on
//! x86-64 Windows).
//!
//! Handles are `u64` (`XR_DEFINE_HANDLE`, `VK_DEFINE_NON_DISPATCHABLE_HANDLE`)
//! or pointers (Vulkan's dispatchable handles); the function types are
//! `extern "system"` (`XRAPI_PTR` and `VKAPI_PTR` are `__stdcall` on 32-bit
//! Windows).

#![allow(non_camel_case_types, dead_code)] // (the headers' names; some for the tests only)

use std::ffi::{c_char, c_void};

pub type XrResult = i32;
pub type XrVersion = u64;
pub type XrFlags64 = u64;
pub type XrBool32 = u32;
pub type XrTime = i64;
pub type XrDuration = i64;
pub type XrStructureType = i32;
pub type XrInstance = u64;
pub type XrSession = u64;
pub type XrSwapchain = u64;
pub type XrSpace = u64;
pub type XrSystemId = u64;

/// `XR_MAKE_VERSION(major, minor, patch)`.
pub const fn xr_make_version(major: u64, minor: u64, patch: u64) -> XrVersion {
    ((major & 0xffff) << 48) | ((minor & 0xffff) << 32) | (patch & 0xffff_ffff)
}

/// A version without its patch component (major and minor).
pub const fn xr_major_minor(version: XrVersion) -> XrVersion {
    version & !0xffff_ffff
}

// XrResult

pub const XR_SUCCESS: XrResult = 0;
pub const XR_EVENT_UNAVAILABLE: XrResult = 4;
pub const XR_FRAME_DISCARDED: XrResult = 9;
pub const XR_ERROR_VALIDATION_FAILURE: XrResult = -1;
pub const XR_ERROR_RUNTIME_FAILURE: XrResult = -2;
pub const XR_ERROR_API_VERSION_UNSUPPORTED: XrResult = -4;
pub const XR_ERROR_INITIALIZATION_FAILED: XrResult = -6;
pub const XR_ERROR_FUNCTION_UNSUPPORTED: XrResult = -7;
pub const XR_ERROR_FEATURE_UNSUPPORTED: XrResult = -8;
pub const XR_ERROR_EXTENSION_NOT_PRESENT: XrResult = -9;
pub const XR_ERROR_LIMIT_REACHED: XrResult = -10;
pub const XR_ERROR_SIZE_INSUFFICIENT: XrResult = -11;
pub const XR_ERROR_HANDLE_INVALID: XrResult = -12;
pub const XR_ERROR_SESSION_RUNNING: XrResult = -14;
pub const XR_ERROR_SESSION_NOT_RUNNING: XrResult = -16;
pub const XR_ERROR_SYSTEM_INVALID: XrResult = -18;
pub const XR_ERROR_LAYER_INVALID: XrResult = -23;
pub const XR_ERROR_LAYER_LIMIT_EXCEEDED: XrResult = -24;
pub const XR_ERROR_SWAPCHAIN_FORMAT_UNSUPPORTED: XrResult = -26;
pub const XR_ERROR_SESSION_NOT_READY: XrResult = -28;
pub const XR_ERROR_SESSION_NOT_STOPPING: XrResult = -29;
pub const XR_ERROR_REFERENCE_SPACE_UNSUPPORTED: XrResult = -31;
pub const XR_ERROR_FORM_FACTOR_UNSUPPORTED: XrResult = -34;
pub const XR_ERROR_API_LAYER_NOT_PRESENT: XrResult = -36;
pub const XR_ERROR_CALL_ORDER_INVALID: XrResult = -37;
pub const XR_ERROR_GRAPHICS_DEVICE_INVALID: XrResult = -38;
pub const XR_ERROR_POSE_INVALID: XrResult = -39;
pub const XR_ERROR_VIEW_CONFIGURATION_TYPE_UNSUPPORTED: XrResult = -41;
pub const XR_ERROR_ENVIRONMENT_BLEND_MODE_UNSUPPORTED: XrResult = -42;
pub const XR_ERROR_NAME_INVALID: XrResult = -45;
pub const XR_ERROR_GRAPHICS_REQUIREMENTS_CALL_MISSING: XrResult = -50;

// XrStructureType

pub const XR_TYPE_API_LAYER_PROPERTIES: XrStructureType = 1;
pub const XR_TYPE_EXTENSION_PROPERTIES: XrStructureType = 2;
pub const XR_TYPE_INSTANCE_CREATE_INFO: XrStructureType = 3;
pub const XR_TYPE_SYSTEM_GET_INFO: XrStructureType = 4;
pub const XR_TYPE_VIEW_LOCATE_INFO: XrStructureType = 6;
pub const XR_TYPE_VIEW: XrStructureType = 7;
pub const XR_TYPE_SESSION_CREATE_INFO: XrStructureType = 8;
pub const XR_TYPE_SWAPCHAIN_CREATE_INFO: XrStructureType = 9;
pub const XR_TYPE_SESSION_BEGIN_INFO: XrStructureType = 10;
pub const XR_TYPE_VIEW_STATE: XrStructureType = 11;
pub const XR_TYPE_FRAME_END_INFO: XrStructureType = 12;
pub const XR_TYPE_EVENT_DATA_BUFFER: XrStructureType = 16;
pub const XR_TYPE_EVENT_DATA_SESSION_STATE_CHANGED: XrStructureType = 18;
pub const XR_TYPE_FRAME_WAIT_INFO: XrStructureType = 33;
pub const XR_TYPE_COMPOSITION_LAYER_PROJECTION: XrStructureType = 35;
pub const XR_TYPE_COMPOSITION_LAYER_QUAD: XrStructureType = 36;
pub const XR_TYPE_REFERENCE_SPACE_CREATE_INFO: XrStructureType = 37;
pub const XR_TYPE_FRAME_STATE: XrStructureType = 44;
pub const XR_TYPE_FRAME_BEGIN_INFO: XrStructureType = 46;
pub const XR_TYPE_COMPOSITION_LAYER_PROJECTION_VIEW: XrStructureType = 48;
pub const XR_TYPE_SWAPCHAIN_IMAGE_ACQUIRE_INFO: XrStructureType = 55;
pub const XR_TYPE_SWAPCHAIN_IMAGE_WAIT_INFO: XrStructureType = 56;
pub const XR_TYPE_SWAPCHAIN_IMAGE_RELEASE_INFO: XrStructureType = 57;
pub const XR_TYPE_GRAPHICS_BINDING_VULKAN2_KHR: XrStructureType = 1000025000;
pub const XR_TYPE_SWAPCHAIN_IMAGE_VULKAN2_KHR: XrStructureType = 1000025001;
pub const XR_TYPE_GRAPHICS_REQUIREMENTS_VULKAN2_KHR: XrStructureType = 1000025002;
pub const XR_TYPE_VULKAN_INSTANCE_CREATE_INFO_KHR: XrStructureType = 1000090000;
pub const XR_TYPE_VULKAN_DEVICE_CREATE_INFO_KHR: XrStructureType = 1000090001;
pub const XR_TYPE_VULKAN_GRAPHICS_DEVICE_GET_INFO_KHR: XrStructureType = 1000090003;

// Enumerations and flags

pub const XR_FORM_FACTOR_HEAD_MOUNTED_DISPLAY: i32 = 1;
pub const XR_FORM_FACTOR_HANDHELD_DISPLAY: i32 = 2;
pub const XR_VIEW_CONFIGURATION_TYPE_PRIMARY_STEREO: i32 = 2;
pub const XR_ENVIRONMENT_BLEND_MODE_OPAQUE: i32 = 1;
pub const XR_REFERENCE_SPACE_TYPE_VIEW: i32 = 1;
pub const XR_REFERENCE_SPACE_TYPE_LOCAL: i32 = 2;
pub const XR_REFERENCE_SPACE_TYPE_STAGE: i32 = 3;

pub const XR_SESSION_STATE_IDLE: i32 = 1;
pub const XR_SESSION_STATE_READY: i32 = 2;
pub const XR_SESSION_STATE_SYNCHRONIZED: i32 = 3;
pub const XR_SESSION_STATE_VISIBLE: i32 = 4;
pub const XR_SESSION_STATE_FOCUSED: i32 = 5;
pub const XR_SESSION_STATE_STOPPING: i32 = 6;
pub const XR_SESSION_STATE_EXITING: i32 = 8;

pub const XR_SWAPCHAIN_CREATE_PROTECTED_CONTENT_BIT: XrFlags64 = 1;
pub const XR_SWAPCHAIN_CREATE_STATIC_IMAGE_BIT: XrFlags64 = 2;

pub const XR_SWAPCHAIN_USAGE_COLOR_ATTACHMENT_BIT: XrFlags64 = 0x01;
pub const XR_SWAPCHAIN_USAGE_DEPTH_STENCIL_ATTACHMENT_BIT: XrFlags64 = 0x02;
pub const XR_SWAPCHAIN_USAGE_UNORDERED_ACCESS_BIT: XrFlags64 = 0x04;
pub const XR_SWAPCHAIN_USAGE_TRANSFER_SRC_BIT: XrFlags64 = 0x08;
pub const XR_SWAPCHAIN_USAGE_TRANSFER_DST_BIT: XrFlags64 = 0x10;
pub const XR_SWAPCHAIN_USAGE_SAMPLED_BIT: XrFlags64 = 0x20;
pub const XR_SWAPCHAIN_USAGE_MUTABLE_FORMAT_BIT: XrFlags64 = 0x40;

/// `XR_VIEW_STATE_{ORIENTATION,POSITION}_{VALID,TRACKED}_BIT`.
pub const XR_VIEW_STATE_ALL_VALID_AND_TRACKED: XrFlags64 = 0xf;

pub const XR_MAX_EXTENSION_NAME_SIZE: usize = 128;
pub const XR_MAX_APPLICATION_NAME_SIZE: usize = 128;
pub const XR_MAX_ENGINE_NAME_SIZE: usize = 128;
pub const XR_MAX_API_LAYER_NAME_SIZE: usize = 256;
pub const XR_MAX_API_LAYER_DESCRIPTION_SIZE: usize = 256;

// The loader's negotiation (openxr_loader_negotiation.h)

pub const XR_LOADER_INTERFACE_STRUCT_LOADER_INFO: i32 = 1;
pub const XR_LOADER_INTERFACE_STRUCT_RUNTIME_REQUEST: i32 = 3;
pub const XR_LOADER_INFO_STRUCT_VERSION: u32 = 1;
pub const XR_RUNTIME_INFO_STRUCT_VERSION: u32 = 1;
pub const XR_CURRENT_LOADER_RUNTIME_VERSION: u32 = 1;

/// `XrNegotiateLoaderInfo`.
#[repr(C)]
pub struct XrNegotiateLoaderInfo {
    pub struct_type: i32,
    pub struct_version: u32,
    pub struct_size: usize,
    pub min_interface_version: u32,
    pub max_interface_version: u32,
    pub min_api_version: XrVersion,
    pub max_api_version: XrVersion,
}

/// `XrNegotiateRuntimeRequest`.
#[repr(C)]
pub struct XrNegotiateRuntimeRequest {
    pub struct_type: i32,
    pub struct_version: u32,
    pub struct_size: usize,
    pub runtime_interface_version: u32,
    pub runtime_api_version: XrVersion,
    pub get_instance_proc_addr: Option<PfnXrGetInstanceProcAddr>,
}

// Core structures

/// A structure's header (`XrBaseInStructure`, `XrBaseOutStructure`): its
/// type and chain.
#[repr(C)]
pub struct XrBaseHeader {
    pub ty: XrStructureType,
    pub next: *const XrBaseHeader,
}

/// `XrApiLayerProperties`.
#[repr(C)]
pub struct XrApiLayerProperties {
    pub ty: XrStructureType,
    pub next: *mut c_void,
    pub layer_name: [c_char; XR_MAX_API_LAYER_NAME_SIZE],
    pub spec_version: XrVersion,
    pub layer_version: u32,
    pub description: [c_char; XR_MAX_API_LAYER_DESCRIPTION_SIZE],
}

/// `XrExtensionProperties`.
#[repr(C)]
pub struct XrExtensionProperties {
    pub ty: XrStructureType,
    pub next: *mut c_void,
    pub extension_name: [c_char; XR_MAX_EXTENSION_NAME_SIZE],
    pub extension_version: u32,
}

/// `XrApplicationInfo`.
#[repr(C)]
pub struct XrApplicationInfo {
    pub application_name: [c_char; XR_MAX_APPLICATION_NAME_SIZE],
    pub application_version: u32,
    pub engine_name: [c_char; XR_MAX_ENGINE_NAME_SIZE],
    pub engine_version: u32,
    pub api_version: XrVersion,
}

/// `XrInstanceCreateInfo`.
#[repr(C)]
pub struct XrInstanceCreateInfo {
    pub ty: XrStructureType,
    pub next: *const c_void,
    pub create_flags: XrFlags64,
    pub application_info: XrApplicationInfo,
    pub enabled_api_layer_count: u32,
    pub enabled_api_layer_names: *const *const c_char,
    pub enabled_extension_count: u32,
    pub enabled_extension_names: *const *const c_char,
}

/// `XrSystemGetInfo`.
#[repr(C)]
pub struct XrSystemGetInfo {
    pub ty: XrStructureType,
    pub next: *const c_void,
    pub form_factor: i32,
}

/// `XrSessionCreateInfo`.
#[repr(C)]
pub struct XrSessionCreateInfo {
    pub ty: XrStructureType,
    pub next: *const XrBaseHeader,
    pub create_flags: XrFlags64,
    pub system_id: XrSystemId,
}

/// `XrSessionBeginInfo`.
#[repr(C)]
pub struct XrSessionBeginInfo {
    pub ty: XrStructureType,
    pub next: *const c_void,
    pub primary_view_configuration_type: i32,
}

/// `XrEventDataBuffer`.
#[repr(C)]
pub struct XrEventDataBuffer {
    pub ty: XrStructureType,
    pub next: *const c_void,
    pub varying: [u8; 4000],
}

/// `XrEventDataSessionStateChanged`.
#[repr(C)]
pub struct XrEventDataSessionStateChanged {
    pub ty: XrStructureType,
    pub next: *const c_void,
    pub session: XrSession,
    pub state: i32,
    pub time: XrTime,
}

/// `XrSwapchainCreateInfo`.
#[repr(C)]
pub struct XrSwapchainCreateInfo {
    pub ty: XrStructureType,
    pub next: *const c_void,
    pub create_flags: XrFlags64,
    pub usage_flags: XrFlags64,
    pub format: i64,
    pub sample_count: u32,
    pub width: u32,
    pub height: u32,
    pub face_count: u32,
    pub array_size: u32,
    pub mip_count: u32,
}

/// `XrSwapchainImageWaitInfo`.
#[repr(C)]
pub struct XrSwapchainImageWaitInfo {
    pub ty: XrStructureType,
    pub next: *const c_void,
    pub timeout: XrDuration,
}

/// `XrFrameState`.
#[repr(C)]
pub struct XrFrameState {
    pub ty: XrStructureType,
    pub next: *mut c_void,
    pub predicted_display_time: XrTime,
    pub predicted_display_period: XrDuration,
    pub should_render: XrBool32,
}

/// `XrFrameEndInfo`.
#[repr(C)]
pub struct XrFrameEndInfo {
    pub ty: XrStructureType,
    pub next: *const c_void,
    pub display_time: XrTime,
    pub environment_blend_mode: i32,
    pub layer_count: u32,
    pub layers: *const *const XrCompositionLayerBaseHeader,
}

/// `XrCompositionLayerBaseHeader`.
#[repr(C)]
pub struct XrCompositionLayerBaseHeader {
    pub ty: XrStructureType,
    pub next: *const c_void,
    pub layer_flags: XrFlags64,
    pub space: XrSpace,
}

/// `XrPosef` (an `XrQuaternionf` and an `XrVector3f`).
#[repr(C)]
#[derive(Clone, Copy)]
pub struct XrPosef {
    pub orientation: [f32; 4],
    pub position: [f32; 3],
}

/// `XrFovf`.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct XrFovf {
    pub angle_left: f32,
    pub angle_right: f32,
    pub angle_up: f32,
    pub angle_down: f32,
}

/// `XrSwapchainSubImage` (an `XrRect2Di`'s offset and extent).
#[repr(C)]
pub struct XrSwapchainSubImage {
    pub swapchain: XrSwapchain,
    pub image_rect: [i32; 4],
    pub image_array_index: u32,
}

/// `XrCompositionLayerProjectionView`.
#[repr(C)]
pub struct XrCompositionLayerProjectionView {
    pub ty: XrStructureType,
    pub next: *const c_void,
    pub pose: XrPosef,
    pub fov: XrFovf,
    pub sub_image: XrSwapchainSubImage,
}

/// `XrCompositionLayerProjection`.
#[repr(C)]
pub struct XrCompositionLayerProjection {
    pub ty: XrStructureType,
    pub next: *const c_void,
    pub layer_flags: XrFlags64,
    pub space: XrSpace,
    pub view_count: u32,
    pub views: *const XrCompositionLayerProjectionView,
}

/// `XrViewLocateInfo`.
#[repr(C)]
pub struct XrViewLocateInfo {
    pub ty: XrStructureType,
    pub next: *const c_void,
    pub view_configuration_type: i32,
    pub display_time: XrTime,
    pub space: XrSpace,
}

/// `XrViewState`.
#[repr(C)]
pub struct XrViewState {
    pub ty: XrStructureType,
    pub next: *mut c_void,
    pub view_state_flags: XrFlags64,
}

/// `XrView`.
#[repr(C)]
pub struct XrView {
    pub ty: XrStructureType,
    pub next: *mut c_void,
    pub pose: XrPosef,
    pub fov: XrFovf,
}

/// `XrReferenceSpaceCreateInfo`.
#[repr(C)]
pub struct XrReferenceSpaceCreateInfo {
    pub ty: XrStructureType,
    pub next: *const c_void,
    pub reference_space_type: i32,
    pub pose_in_reference_space: XrPosef,
}

// XR_KHR_vulkan_enable2

pub const XR_KHR_VULKAN_ENABLE2_EXTENSION_NAME: &str = "XR_KHR_vulkan_enable2";
/// The extension's version this runtime implements.
pub const XR_KHR_VULKAN_ENABLE2_SPEC_VERSION: u32 = 2;

/// `XrGraphicsRequirementsVulkan2KHR`.
#[repr(C)]
pub struct XrGraphicsRequirementsVulkan2KHR {
    pub ty: XrStructureType,
    pub next: *mut c_void,
    pub min_api_version_supported: XrVersion,
    pub max_api_version_supported: XrVersion,
}

/// `XrVulkanInstanceCreateInfoKHR`.
#[repr(C)]
pub struct XrVulkanInstanceCreateInfoKHR {
    pub ty: XrStructureType,
    pub next: *const c_void,
    pub system_id: XrSystemId,
    pub create_flags: XrFlags64,
    pub pfn_get_instance_proc_addr: Option<PfnVkGetInstanceProcAddr>,
    pub vulkan_create_info: *const c_void,
    pub vulkan_allocator: *const c_void,
}

/// `XrVulkanDeviceCreateInfoKHR`.
#[repr(C)]
pub struct XrVulkanDeviceCreateInfoKHR {
    pub ty: XrStructureType,
    pub next: *const c_void,
    pub system_id: XrSystemId,
    pub create_flags: XrFlags64,
    pub pfn_get_instance_proc_addr: Option<PfnVkGetInstanceProcAddr>,
    pub vulkan_physical_device: VkPhysicalDevice,
    pub vulkan_create_info: *const c_void,
    pub vulkan_allocator: *const c_void,
}

/// `XrVulkanGraphicsDeviceGetInfoKHR`.
#[repr(C)]
pub struct XrVulkanGraphicsDeviceGetInfoKHR {
    pub ty: XrStructureType,
    pub next: *const c_void,
    pub system_id: XrSystemId,
    pub vulkan_instance: VkInstance,
}

/// `XrGraphicsBindingVulkan2KHR`.
#[repr(C)]
pub struct XrGraphicsBindingVulkan2KHR {
    pub ty: XrStructureType,
    pub next: *const c_void,
    pub instance: VkInstance,
    pub physical_device: VkPhysicalDevice,
    pub device: VkDevice,
    pub queue_family_index: u32,
    pub queue_index: u32,
}

/// `XrSwapchainImageVulkan2KHR`.
#[repr(C)]
pub struct XrSwapchainImageVulkan2KHR {
    pub ty: XrStructureType,
    pub next: *mut c_void,
    pub image: VkImage,
}

/// `XR_KHR_D3D12_ENABLE_EXTENSION_NAME`.
pub const XR_KHR_D3D12_ENABLE_EXTENSION_NAME: &str = "XR_KHR_D3D12_enable";
/// The extension's version this runtime implements.
pub const XR_KHR_D3D12_ENABLE_SPEC_VERSION: u32 = 11;

pub const XR_TYPE_GRAPHICS_BINDING_D3D12_KHR: XrStructureType = 1000028000;
pub const XR_TYPE_SWAPCHAIN_IMAGE_D3D12_KHR: XrStructureType = 1000028001;
pub const XR_TYPE_GRAPHICS_REQUIREMENTS_D3D12_KHR: XrStructureType = 1000028002;

/// `D3D_FEATURE_LEVEL_11_0` (d3dcommon.h).
pub const D3D_FEATURE_LEVEL_11_0: i32 = 0xb000;

/// `LUID` (winnt.h).
#[repr(C)]
#[derive(Clone, Copy, Default, PartialEq, Eq, Debug)]
pub struct Luid {
    pub low_part: u32,
    pub high_part: i32,
}

/// `XrGraphicsRequirementsD3D12KHR`.
#[repr(C)]
pub struct XrGraphicsRequirementsD3D12KHR {
    pub ty: XrStructureType,
    pub next: *mut c_void,
    pub adapter_luid: Luid,
    /// `D3D_FEATURE_LEVEL`.
    pub min_feature_level: i32,
}

/// `XrGraphicsBindingD3D12KHR`.
#[repr(C)]
pub struct XrGraphicsBindingD3D12KHR {
    pub ty: XrStructureType,
    pub next: *const c_void,
    /// `ID3D12Device *`.
    pub device: *mut c_void,
    /// `ID3D12CommandQueue *`.
    pub queue: *mut c_void,
}

/// `XrSwapchainImageD3D12KHR`.
#[repr(C)]
pub struct XrSwapchainImageD3D12KHR {
    pub ty: XrStructureType,
    pub next: *mut c_void,
    /// `ID3D12Resource *`.
    pub texture: *mut c_void,
}

// The function types

/// `PFN_xrVoidFunction`.
pub type PfnXrVoidFunction = unsafe extern "system" fn();
/// `PFN_xrGetInstanceProcAddr`.
pub type PfnXrGetInstanceProcAddr = unsafe extern "system" fn(
    instance: XrInstance,
    name: *const c_char,
    function: *mut Option<PfnXrVoidFunction>,
) -> XrResult;

// Vulkan

pub type VkResult = i32;
pub type VkInstance = *mut c_void;
pub type VkPhysicalDevice = *mut c_void;
pub type VkDevice = *mut c_void;
pub type VkImage = u64;
pub type VkDeviceMemory = u64;

pub const VK_SUCCESS: VkResult = 0;
pub const VK_STRUCTURE_TYPE_MEMORY_ALLOCATE_INFO: i32 = 5;
pub const VK_STRUCTURE_TYPE_IMAGE_CREATE_INFO: i32 = 14;
pub const VK_IMAGE_CREATE_MUTABLE_FORMAT_BIT: u32 = 0x08;
pub const VK_IMAGE_TYPE_2D: i32 = 1;
pub const VK_SAMPLE_COUNT_1_BIT: u32 = 1;
pub const VK_IMAGE_TILING_OPTIMAL: i32 = 0;
pub const VK_SHARING_MODE_EXCLUSIVE: i32 = 0;
pub const VK_IMAGE_LAYOUT_UNDEFINED: i32 = 0;
pub const VK_IMAGE_USAGE_TRANSFER_SRC_BIT: u32 = 0x01;
pub const VK_IMAGE_USAGE_TRANSFER_DST_BIT: u32 = 0x02;
pub const VK_IMAGE_USAGE_SAMPLED_BIT: u32 = 0x04;
pub const VK_IMAGE_USAGE_STORAGE_BIT: u32 = 0x08;
pub const VK_IMAGE_USAGE_COLOR_ATTACHMENT_BIT: u32 = 0x10;
pub const VK_IMAGE_USAGE_DEPTH_STENCIL_ATTACHMENT_BIT: u32 = 0x20;
pub const VK_MEMORY_PROPERTY_DEVICE_LOCAL_BIT: u32 = 0x01;

/// `VkExtent3D`.
#[repr(C)]
pub struct VkExtent3D {
    pub width: u32,
    pub height: u32,
    pub depth: u32,
}

/// `VkImageCreateInfo`.
#[repr(C)]
pub struct VkImageCreateInfo {
    pub s_type: i32,
    pub p_next: *const c_void,
    pub flags: u32,
    pub image_type: i32,
    pub format: i32,
    pub extent: VkExtent3D,
    pub mip_levels: u32,
    pub array_layers: u32,
    pub samples: u32,
    pub tiling: i32,
    pub usage: u32,
    pub sharing_mode: i32,
    pub queue_family_index_count: u32,
    pub p_queue_family_indices: *const u32,
    pub initial_layout: i32,
}

/// `VkMemoryRequirements`.
#[repr(C)]
pub struct VkMemoryRequirements {
    pub size: u64,
    pub alignment: u64,
    pub memory_type_bits: u32,
}

/// `VkMemoryAllocateInfo`.
#[repr(C)]
pub struct VkMemoryAllocateInfo {
    pub s_type: i32,
    pub p_next: *const c_void,
    pub allocation_size: u64,
    pub memory_type_index: u32,
}

/// `VkMemoryType`.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct VkMemoryType {
    pub property_flags: u32,
    pub heap_index: u32,
}

/// `VkMemoryHeap`.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct VkMemoryHeap {
    pub size: u64,
    pub flags: u32,
}

/// `VkPhysicalDeviceMemoryProperties`.
#[repr(C)]
pub struct VkPhysicalDeviceMemoryProperties {
    pub memory_type_count: u32,
    pub memory_types: [VkMemoryType; 32],
    pub memory_heap_count: u32,
    pub memory_heaps: [VkMemoryHeap; 16],
}

/// `PFN_vkVoidFunction`.
pub type PfnVkVoidFunction = unsafe extern "system" fn();
/// `PFN_vkGetInstanceProcAddr`.
pub type PfnVkGetInstanceProcAddr = unsafe extern "system" fn(
    instance: VkInstance,
    name: *const c_char,
) -> Option<PfnVkVoidFunction>;
/// `PFN_vkGetDeviceProcAddr`.
pub type PfnVkGetDeviceProcAddr =
    unsafe extern "system" fn(device: VkDevice, name: *const c_char) -> Option<PfnVkVoidFunction>;
/// `PFN_vkCreateInstance` (`VkInstanceCreateInfo` and
/// `VkAllocationCallbacks` are the application's, passed on).
pub type PfnVkCreateInstance = unsafe extern "system" fn(
    create_info: *const c_void,
    allocator: *const c_void,
    instance: *mut VkInstance,
) -> VkResult;
/// `PFN_vkEnumeratePhysicalDevices`.
pub type PfnVkEnumeratePhysicalDevices = unsafe extern "system" fn(
    instance: VkInstance,
    count: *mut u32,
    devices: *mut VkPhysicalDevice,
) -> VkResult;
/// `PFN_vkCreateDevice` (the application's `VkDeviceCreateInfo` and
/// allocator, passed on).
pub type PfnVkCreateDevice = unsafe extern "system" fn(
    physical_device: VkPhysicalDevice,
    create_info: *const c_void,
    allocator: *const c_void,
    device: *mut VkDevice,
) -> VkResult;
/// `PFN_vkGetPhysicalDeviceMemoryProperties`.
pub type PfnVkGetPhysicalDeviceMemoryProperties = unsafe extern "system" fn(
    physical_device: VkPhysicalDevice,
    properties: *mut VkPhysicalDeviceMemoryProperties,
);
/// `PFN_vkCreateImage`.
pub type PfnVkCreateImage = unsafe extern "system" fn(
    device: VkDevice,
    create_info: *const VkImageCreateInfo,
    allocator: *const c_void,
    image: *mut VkImage,
) -> VkResult;
/// `PFN_vkDestroyImage`.
pub type PfnVkDestroyImage =
    unsafe extern "system" fn(device: VkDevice, image: VkImage, allocator: *const c_void);
/// `PFN_vkGetImageMemoryRequirements`.
pub type PfnVkGetImageMemoryRequirements = unsafe extern "system" fn(
    device: VkDevice,
    image: VkImage,
    requirements: *mut VkMemoryRequirements,
);
/// `PFN_vkAllocateMemory`.
pub type PfnVkAllocateMemory = unsafe extern "system" fn(
    device: VkDevice,
    allocate_info: *const VkMemoryAllocateInfo,
    allocator: *const c_void,
    memory: *mut VkDeviceMemory,
) -> VkResult;
/// `PFN_vkFreeMemory`.
pub type PfnVkFreeMemory =
    unsafe extern "system" fn(device: VkDevice, memory: VkDeviceMemory, allocator: *const c_void);
/// `PFN_vkBindImageMemory`.
pub type PfnVkBindImageMemory = unsafe extern "system" fn(
    device: VkDevice,
    image: VkImage,
    memory: VkDeviceMemory,
    offset: u64,
) -> VkResult;

#[cfg(test)]
mod tests {
    use super::*;

    /// The sizes and offsets the scratchpad harness printed (layout.c,
    /// against the OpenXR 1.1.63 and Vulkan headers upstream SDL vendors at
    /// ea7a2da, gcc on x86-64 Linux; the OpenXR ones are the same on
    /// x86-64 Windows and Android, as the sdl3 crate's `gpu::xr::sys`
    /// tests say).
    #[cfg(target_pointer_width = "64")]
    #[test]
    fn layouts_match_c() {
        use std::mem::{offset_of, size_of};
        macro_rules! size {
            ($t:ty, $size:expr) => {
                assert_eq!(size_of::<$t>(), $size, stringify!($t));
            };
        }
        macro_rules! off {
            ($t:ty, $f:ident, $off:expr) => {
                assert_eq!(
                    offset_of!($t, $f),
                    $off,
                    concat!(stringify!($t), ".", stringify!($f))
                );
            };
        }
        size!(XrNegotiateLoaderInfo, 40);
        off!(XrNegotiateLoaderInfo, struct_size, 8);
        off!(XrNegotiateLoaderInfo, min_interface_version, 16);
        off!(XrNegotiateLoaderInfo, max_interface_version, 20);
        off!(XrNegotiateLoaderInfo, min_api_version, 24);
        off!(XrNegotiateLoaderInfo, max_api_version, 32);
        size!(XrNegotiateRuntimeRequest, 40);
        off!(XrNegotiateRuntimeRequest, struct_size, 8);
        off!(XrNegotiateRuntimeRequest, runtime_interface_version, 16);
        off!(XrNegotiateRuntimeRequest, runtime_api_version, 24);
        off!(XrNegotiateRuntimeRequest, get_instance_proc_addr, 32);
        size!(XrApiLayerProperties, 544);
        off!(XrApiLayerProperties, spec_version, 272);
        off!(XrApiLayerProperties, description, 284);
        size!(XrExtensionProperties, 152);
        off!(XrExtensionProperties, extension_version, 144);
        size!(XrInstanceCreateInfo, 328);
        off!(XrInstanceCreateInfo, application_info, 24);
        off!(XrInstanceCreateInfo, enabled_api_layer_count, 296);
        off!(XrInstanceCreateInfo, enabled_extension_names, 320);
        off!(XrApplicationInfo, api_version, 264);
        size!(XrSystemGetInfo, 24);
        size!(XrSessionCreateInfo, 32);
        off!(XrSessionCreateInfo, system_id, 24);
        size!(XrSessionBeginInfo, 24);
        off!(XrSessionBeginInfo, primary_view_configuration_type, 16);
        size!(XrEventDataBuffer, 4016);
        size!(XrEventDataSessionStateChanged, 40);
        off!(XrEventDataSessionStateChanged, session, 16);
        off!(XrEventDataSessionStateChanged, state, 24);
        off!(XrEventDataSessionStateChanged, time, 32);
        size!(XrSwapchainCreateInfo, 64);
        off!(XrSwapchainCreateInfo, format, 32);
        off!(XrSwapchainCreateInfo, mip_count, 60);
        size!(XrSwapchainImageWaitInfo, 24);
        off!(XrSwapchainImageWaitInfo, timeout, 16);
        size!(XrFrameState, 40);
        off!(XrFrameState, predicted_display_time, 16);
        off!(XrFrameState, predicted_display_period, 24);
        off!(XrFrameState, should_render, 32);
        size!(XrFrameEndInfo, 40);
        off!(XrFrameEndInfo, display_time, 16);
        off!(XrFrameEndInfo, environment_blend_mode, 24);
        off!(XrFrameEndInfo, layer_count, 28);
        off!(XrFrameEndInfo, layers, 32);
        size!(XrCompositionLayerBaseHeader, 32);
        off!(XrCompositionLayerBaseHeader, layer_flags, 16);
        off!(XrCompositionLayerBaseHeader, space, 24);
        size!(XrCompositionLayerProjection, 48);
        off!(XrCompositionLayerProjection, view_count, 32);
        off!(XrCompositionLayerProjection, views, 40);
        size!(XrCompositionLayerProjectionView, 96);
        off!(XrCompositionLayerProjectionView, sub_image, 64);
        size!(XrSwapchainSubImage, 32);
        off!(XrSwapchainSubImage, image_rect, 8);
        off!(XrSwapchainSubImage, image_array_index, 24);
        size!(XrViewLocateInfo, 40);
        off!(XrViewLocateInfo, view_configuration_type, 16);
        off!(XrViewLocateInfo, display_time, 24);
        off!(XrViewLocateInfo, space, 32);
        size!(XrViewState, 24);
        off!(XrViewState, view_state_flags, 16);
        size!(XrView, 64);
        off!(XrView, pose, 16);
        off!(XrView, fov, 44);
        size!(XrPosef, 28);
        size!(XrFovf, 16);
        size!(XrReferenceSpaceCreateInfo, 48);
        off!(XrReferenceSpaceCreateInfo, reference_space_type, 16);
        off!(XrReferenceSpaceCreateInfo, pose_in_reference_space, 20);
        size!(XrGraphicsRequirementsVulkan2KHR, 32);
        off!(
            XrGraphicsRequirementsVulkan2KHR,
            max_api_version_supported,
            24
        );
        size!(XrVulkanInstanceCreateInfoKHR, 56);
        off!(
            XrVulkanInstanceCreateInfoKHR,
            pfn_get_instance_proc_addr,
            32
        );
        off!(XrVulkanInstanceCreateInfoKHR, vulkan_allocator, 48);
        size!(XrVulkanDeviceCreateInfoKHR, 64);
        off!(XrVulkanDeviceCreateInfoKHR, vulkan_physical_device, 40);
        off!(XrVulkanDeviceCreateInfoKHR, vulkan_allocator, 56);
        size!(XrVulkanGraphicsDeviceGetInfoKHR, 32);
        off!(XrVulkanGraphicsDeviceGetInfoKHR, vulkan_instance, 24);
        size!(XrGraphicsBindingVulkan2KHR, 48);
        off!(XrGraphicsBindingVulkan2KHR, device, 32);
        off!(XrGraphicsBindingVulkan2KHR, queue_index, 44);
        size!(XrSwapchainImageVulkan2KHR, 24);
        off!(XrSwapchainImageVulkan2KHR, image, 16);
        assert_eq!((size_of::<Luid>(), std::mem::align_of::<Luid>()), (8, 4));
        size!(XrGraphicsRequirementsD3D12KHR, 32);
        off!(XrGraphicsRequirementsD3D12KHR, adapter_luid, 16);
        off!(XrGraphicsRequirementsD3D12KHR, min_feature_level, 24);
        size!(XrGraphicsBindingD3D12KHR, 32);
        off!(XrGraphicsBindingD3D12KHR, device, 16);
        off!(XrGraphicsBindingD3D12KHR, queue, 24);
        size!(XrSwapchainImageD3D12KHR, 24);
        off!(XrSwapchainImageD3D12KHR, texture, 16);
        size!(VkImageCreateInfo, 88);
        off!(VkImageCreateInfo, flags, 16);
        off!(VkImageCreateInfo, image_type, 20);
        off!(VkImageCreateInfo, format, 24);
        off!(VkImageCreateInfo, extent, 28);
        off!(VkImageCreateInfo, mip_levels, 40);
        off!(VkImageCreateInfo, array_layers, 44);
        off!(VkImageCreateInfo, samples, 48);
        off!(VkImageCreateInfo, tiling, 52);
        off!(VkImageCreateInfo, usage, 56);
        off!(VkImageCreateInfo, sharing_mode, 60);
        off!(VkImageCreateInfo, queue_family_index_count, 64);
        off!(VkImageCreateInfo, p_queue_family_indices, 72);
        off!(VkImageCreateInfo, initial_layout, 80);
        size!(VkMemoryRequirements, 24);
        off!(VkMemoryRequirements, alignment, 8);
        off!(VkMemoryRequirements, memory_type_bits, 16);
        size!(VkMemoryAllocateInfo, 32);
        off!(VkMemoryAllocateInfo, allocation_size, 16);
        off!(VkMemoryAllocateInfo, memory_type_index, 24);
        size!(VkPhysicalDeviceMemoryProperties, 520);
        off!(VkPhysicalDeviceMemoryProperties, memory_types, 4);
        off!(VkPhysicalDeviceMemoryProperties, memory_heap_count, 260);
        off!(VkPhysicalDeviceMemoryProperties, memory_heaps, 264);
    }

    /// The values the harness printed.
    #[test]
    fn values_match_c() {
        assert_eq!(
            [
                XR_TYPE_EVENT_DATA_BUFFER,
                XR_TYPE_EVENT_DATA_SESSION_STATE_CHANGED,
                XR_TYPE_FRAME_WAIT_INFO,
                XR_TYPE_FRAME_STATE,
                XR_TYPE_FRAME_BEGIN_INFO,
                XR_TYPE_FRAME_END_INFO,
                XR_TYPE_COMPOSITION_LAYER_PROJECTION,
                XR_TYPE_COMPOSITION_LAYER_QUAD,
                XR_TYPE_COMPOSITION_LAYER_PROJECTION_VIEW,
                XR_TYPE_VIEW_LOCATE_INFO,
                XR_TYPE_VIEW_STATE,
                XR_TYPE_VIEW,
                XR_TYPE_REFERENCE_SPACE_CREATE_INFO,
                XR_TYPE_SESSION_BEGIN_INFO,
                XR_TYPE_SWAPCHAIN_IMAGE_ACQUIRE_INFO,
                XR_TYPE_SWAPCHAIN_IMAGE_WAIT_INFO,
                XR_TYPE_SWAPCHAIN_IMAGE_RELEASE_INFO,
            ],
            [16, 18, 33, 44, 46, 12, 35, 36, 48, 6, 11, 7, 37, 10, 55, 56, 57]
        );
        assert_eq!(XR_CURRENT_LOADER_RUNTIME_VERSION, 1);
        assert_eq!(
            (
                VK_STRUCTURE_TYPE_IMAGE_CREATE_INFO,
                VK_STRUCTURE_TYPE_MEMORY_ALLOCATE_INFO,
                VK_IMAGE_CREATE_MUTABLE_FORMAT_BIT,
                VK_IMAGE_USAGE_STORAGE_BIT,
            ),
            (14, 5, 8, 8)
        );
        assert_eq!(
            (
                XR_SWAPCHAIN_CREATE_PROTECTED_CONTENT_BIT,
                XR_SWAPCHAIN_CREATE_STATIC_IMAGE_BIT,
                XR_REFERENCE_SPACE_TYPE_STAGE,
            ),
            (1, 2, 3)
        );
        assert_eq!(xr_make_version(1, 0, 63), 0x0001_0000_0000_003f);
        assert_eq!(
            [
                XR_TYPE_GRAPHICS_BINDING_D3D12_KHR,
                XR_TYPE_SWAPCHAIN_IMAGE_D3D12_KHR,
                XR_TYPE_GRAPHICS_REQUIREMENTS_D3D12_KHR,
            ],
            [1000028000, 1000028001, 1000028002]
        );
    }
}
