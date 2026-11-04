// Rust declarations of the parts of the OpenXR headers (openxr.h and
// openxr_platform.h, which upstream vendors in src/video/khronos/openxr) used
// by the GPU API's OpenXR support of Simple DirectMedia Layer.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! The OpenXR types, values and function types SDL uses, declared by hand
//! (interface facts: names, values and layouts; the loader is loaded at run
//! time by [`super::openxrdyn`], nothing is linked). The layouts are those of
//! the headers upstream vendors (OpenXR 1.1.63); the tests below pin them to
//! the sizes and offsets a C program built against those headers printed on
//! x86-64 Linux, x86-64 Windows (mingw-w64) and x86-64 Android (the NDK),
//! which are the same. On 32-bit targets the pointers are 4 bytes and the
//! structures holding them are smaller: the declarations are written for
//! both (pointer fields as pointers), the pinned numbers are for 64-bit.
//!
//! The handles (`XR_DEFINE_HANDLE`: a pointer to an opaque structure on
//! 64-bit targets, a `uint64_t` elsewhere) are `u64`, as SDL_openxr.h's own
//! fallback typedefs declare them; the function types are `XRAPI_PTR`
//! (`__stdcall` on 32-bit Windows: `extern "system"`). The structures
//! implement `Default` as all zeroes (C's `SDL_zero()`), with their
//! `XrStructureType` to be set by the caller as in C.

#![allow(dead_code)] // (the declarations follow the headers; PR by PR the backends use the rest)

use std::ffi::{c_char, c_void};

use crate::video::vk::{VkDevice, VkImage, VkInstance, VkPhysicalDevice};

pub(crate) use super::XrResult;

// Basic types

/// `XrVersion`.
pub(crate) type XrVersion = u64;
/// `XrFlags64`.
pub(crate) type XrFlags64 = u64;
/// `XrBool32`.
pub(crate) type XrBool32 = u32;
/// `XrSystemId` (an atom).
pub(crate) type XrSystemId = u64;
/// `XrInstance`.
pub(crate) type XrInstance = u64;
/// `XrSession`.
pub(crate) type XrSession = u64;
/// `XrSwapchain`.
pub(crate) type XrSwapchain = u64;
/// `XrStructureType`.
pub(crate) type XrStructureType = i32;
/// `XrFormFactor`.
pub(crate) type XrFormFactor = i32;
/// `XrInstanceCreateFlags`.
pub(crate) type XrInstanceCreateFlags = XrFlags64;
/// `XrSessionCreateFlags`.
pub(crate) type XrSessionCreateFlags = XrFlags64;
/// `XrSwapchainCreateFlags`.
pub(crate) type XrSwapchainCreateFlags = XrFlags64;
/// `XrSwapchainUsageFlags`.
pub(crate) type XrSwapchainUsageFlags = XrFlags64;
/// `XrVulkanInstanceCreateFlagsKHR`.
pub(crate) type XrVulkanInstanceCreateFlagsKHR = XrFlags64;
/// `XrVulkanDeviceCreateFlagsKHR`.
pub(crate) type XrVulkanDeviceCreateFlagsKHR = XrFlags64;

/// `XR_NULL_HANDLE`.
pub(crate) const XR_NULL_HANDLE: u64 = 0;
/// `XR_NULL_SYSTEM_ID`.
pub(crate) const XR_NULL_SYSTEM_ID: XrSystemId = 0;

/// `XR_MAKE_VERSION(major, minor, patch)`.
pub(crate) const fn xr_make_version(major: u64, minor: u64, patch: u64) -> XrVersion {
    ((major & 0xffff) << 48) | ((minor & 0xffff) << 32) | (patch & 0xffff_ffff)
}
/// `XR_VERSION_MAJOR(version)`.
pub(crate) const fn xr_version_major(version: XrVersion) -> u64 {
    (version >> 48) & 0xffff
}
/// `XR_VERSION_MINOR(version)`.
pub(crate) const fn xr_version_minor(version: XrVersion) -> u64 {
    (version >> 32) & 0xffff
}
/// `XR_VERSION_PATCH(version)`.
pub(crate) const fn xr_version_patch(version: XrVersion) -> u64 {
    version & 0xffff_ffff
}
/// `XR_CURRENT_API_VERSION` of the vendored headers (1.1.63).
pub(crate) const XR_CURRENT_API_VERSION: XrVersion = xr_make_version(1, 1, 63);
/// `XR_API_VERSION_1_0`: 1.0 with the headers' patch version
/// (`XR_MAKE_VERSION(1, 0, XR_VERSION_PATCH(XR_CURRENT_API_VERSION))`).
pub(crate) const XR_API_VERSION_1_0: XrVersion =
    xr_make_version(1, 0, xr_version_patch(XR_CURRENT_API_VERSION));

// Sizes

/// `XR_MAX_API_LAYER_NAME_SIZE`.
pub(crate) const XR_MAX_API_LAYER_NAME_SIZE: usize = 256;
/// `XR_MAX_API_LAYER_DESCRIPTION_SIZE`.
pub(crate) const XR_MAX_API_LAYER_DESCRIPTION_SIZE: usize = 256;
/// `XR_MAX_EXTENSION_NAME_SIZE`.
pub(crate) const XR_MAX_EXTENSION_NAME_SIZE: usize = 128;
/// `XR_MAX_APPLICATION_NAME_SIZE`.
pub(crate) const XR_MAX_APPLICATION_NAME_SIZE: usize = 128;
/// `XR_MAX_ENGINE_NAME_SIZE`.
pub(crate) const XR_MAX_ENGINE_NAME_SIZE: usize = 128;

// XrStructureType

pub(crate) const XR_TYPE_API_LAYER_PROPERTIES: XrStructureType = 1;
pub(crate) const XR_TYPE_EXTENSION_PROPERTIES: XrStructureType = 2;
pub(crate) const XR_TYPE_INSTANCE_CREATE_INFO: XrStructureType = 3;
pub(crate) const XR_TYPE_SYSTEM_GET_INFO: XrStructureType = 4;
pub(crate) const XR_TYPE_SESSION_CREATE_INFO: XrStructureType = 8;
pub(crate) const XR_TYPE_SWAPCHAIN_CREATE_INFO: XrStructureType = 9;
pub(crate) const XR_TYPE_INSTANCE_CREATE_INFO_ANDROID_KHR: XrStructureType = 1000008000;
pub(crate) const XR_TYPE_GRAPHICS_BINDING_VULKAN_KHR: XrStructureType = 1000025000;
pub(crate) const XR_TYPE_SWAPCHAIN_IMAGE_VULKAN_KHR: XrStructureType = 1000025001;
pub(crate) const XR_TYPE_GRAPHICS_REQUIREMENTS_VULKAN_KHR: XrStructureType = 1000025002;
pub(crate) const XR_TYPE_GRAPHICS_BINDING_D3D12_KHR: XrStructureType = 1000028000;
pub(crate) const XR_TYPE_SWAPCHAIN_IMAGE_D3D12_KHR: XrStructureType = 1000028001;
pub(crate) const XR_TYPE_GRAPHICS_REQUIREMENTS_D3D12_KHR: XrStructureType = 1000028002;
pub(crate) const XR_TYPE_LOADER_INIT_INFO_ANDROID_KHR: XrStructureType = 1000089000;
pub(crate) const XR_TYPE_VULKAN_INSTANCE_CREATE_INFO_KHR: XrStructureType = 1000090000;
pub(crate) const XR_TYPE_VULKAN_DEVICE_CREATE_INFO_KHR: XrStructureType = 1000090001;
pub(crate) const XR_TYPE_VULKAN_GRAPHICS_DEVICE_GET_INFO_KHR: XrStructureType = 1000090003;
/// `XR_TYPE_GRAPHICS_BINDING_VULKAN2_KHR` (an alias).
pub(crate) const XR_TYPE_GRAPHICS_BINDING_VULKAN2_KHR: XrStructureType =
    XR_TYPE_GRAPHICS_BINDING_VULKAN_KHR;
/// `XR_TYPE_SWAPCHAIN_IMAGE_VULKAN2_KHR` (an alias).
pub(crate) const XR_TYPE_SWAPCHAIN_IMAGE_VULKAN2_KHR: XrStructureType =
    XR_TYPE_SWAPCHAIN_IMAGE_VULKAN_KHR;
/// `XR_TYPE_GRAPHICS_REQUIREMENTS_VULKAN2_KHR` (an alias).
pub(crate) const XR_TYPE_GRAPHICS_REQUIREMENTS_VULKAN2_KHR: XrStructureType =
    XR_TYPE_GRAPHICS_REQUIREMENTS_VULKAN_KHR;

// XrFormFactor

pub(crate) const XR_FORM_FACTOR_HEAD_MOUNTED_DISPLAY: XrFormFactor = 1;
pub(crate) const XR_FORM_FACTOR_HANDHELD_DISPLAY: XrFormFactor = 2;

// XrSwapchainUsageFlags

pub(crate) const XR_SWAPCHAIN_USAGE_COLOR_ATTACHMENT_BIT: XrSwapchainUsageFlags = 0x0000_0001;
pub(crate) const XR_SWAPCHAIN_USAGE_DEPTH_STENCIL_ATTACHMENT_BIT: XrSwapchainUsageFlags =
    0x0000_0002;
pub(crate) const XR_SWAPCHAIN_USAGE_UNORDERED_ACCESS_BIT: XrSwapchainUsageFlags = 0x0000_0004;
pub(crate) const XR_SWAPCHAIN_USAGE_TRANSFER_SRC_BIT: XrSwapchainUsageFlags = 0x0000_0008;
pub(crate) const XR_SWAPCHAIN_USAGE_TRANSFER_DST_BIT: XrSwapchainUsageFlags = 0x0000_0010;
pub(crate) const XR_SWAPCHAIN_USAGE_SAMPLED_BIT: XrSwapchainUsageFlags = 0x0000_0020;
pub(crate) const XR_SWAPCHAIN_USAGE_MUTABLE_FORMAT_BIT: XrSwapchainUsageFlags = 0x0000_0040;

// Extension names

/// `XR_KHR_VULKAN_ENABLE2_EXTENSION_NAME`.
pub(crate) const XR_KHR_VULKAN_ENABLE2_EXTENSION_NAME: &str = "XR_KHR_vulkan_enable2";
/// `XR_KHR_D3D12_ENABLE_EXTENSION_NAME`.
pub(crate) const XR_KHR_D3D12_ENABLE_EXTENSION_NAME: &str = "XR_KHR_D3D12_enable";
/// `XR_KHR_ANDROID_CREATE_INSTANCE_EXTENSION_NAME`.
pub(crate) const XR_KHR_ANDROID_CREATE_INSTANCE_EXTENSION_NAME: &str =
    "XR_KHR_android_create_instance";

// Structures

/// `XrApiLayerProperties`.
#[repr(C)]
#[derive(Clone, Copy)]
pub(crate) struct XrApiLayerProperties {
    pub(crate) ty: XrStructureType,
    pub(crate) next: *mut c_void,
    pub(crate) layer_name: [c_char; XR_MAX_API_LAYER_NAME_SIZE],
    pub(crate) spec_version: XrVersion,
    pub(crate) layer_version: u32,
    pub(crate) description: [c_char; XR_MAX_API_LAYER_DESCRIPTION_SIZE],
}

/// `XrExtensionProperties`.
#[repr(C)]
#[derive(Clone, Copy)]
pub(crate) struct XrExtensionProperties {
    pub(crate) ty: XrStructureType,
    pub(crate) next: *mut c_void,
    pub(crate) extension_name: [c_char; XR_MAX_EXTENSION_NAME_SIZE],
    pub(crate) extension_version: u32,
}

/// `XrApplicationInfo`.
#[repr(C)]
#[derive(Clone, Copy)]
pub(crate) struct XrApplicationInfo {
    pub(crate) application_name: [c_char; XR_MAX_APPLICATION_NAME_SIZE],
    pub(crate) application_version: u32,
    pub(crate) engine_name: [c_char; XR_MAX_ENGINE_NAME_SIZE],
    pub(crate) engine_version: u32,
    pub(crate) api_version: XrVersion,
}

/// `XrInstanceCreateInfo`.
#[repr(C)]
#[derive(Clone, Copy)]
pub(crate) struct XrInstanceCreateInfo {
    pub(crate) ty: XrStructureType,
    pub(crate) next: *const c_void,
    pub(crate) create_flags: XrInstanceCreateFlags,
    pub(crate) application_info: XrApplicationInfo,
    pub(crate) enabled_api_layer_count: u32,
    pub(crate) enabled_api_layer_names: *const *const c_char,
    pub(crate) enabled_extension_count: u32,
    pub(crate) enabled_extension_names: *const *const c_char,
}

/// `XrSystemGetInfo`.
#[repr(C)]
#[derive(Clone, Copy)]
pub(crate) struct XrSystemGetInfo {
    pub(crate) ty: XrStructureType,
    pub(crate) next: *const c_void,
    pub(crate) form_factor: XrFormFactor,
}

/// `XrSessionCreateInfo`.
#[repr(C)]
#[derive(Clone, Copy)]
pub(crate) struct XrSessionCreateInfo {
    pub(crate) ty: XrStructureType,
    pub(crate) next: *const c_void,
    pub(crate) create_flags: XrSessionCreateFlags,
    pub(crate) system_id: XrSystemId,
}

/// `XrSwapchainCreateInfo`.
#[repr(C)]
#[derive(Clone, Copy)]
pub(crate) struct XrSwapchainCreateInfo {
    pub(crate) ty: XrStructureType,
    pub(crate) next: *const c_void,
    pub(crate) create_flags: XrSwapchainCreateFlags,
    pub(crate) usage_flags: XrSwapchainUsageFlags,
    pub(crate) format: i64,
    pub(crate) sample_count: u32,
    pub(crate) width: u32,
    pub(crate) height: u32,
    pub(crate) face_count: u32,
    pub(crate) array_size: u32,
    pub(crate) mip_count: u32,
}

/// `XrSwapchainImageBaseHeader`.
#[repr(C)]
#[derive(Clone, Copy)]
pub(crate) struct XrSwapchainImageBaseHeader {
    pub(crate) ty: XrStructureType,
    pub(crate) next: *mut c_void,
}

/// `XrLoaderInitInfoBaseHeaderKHR`.
#[repr(C)]
#[derive(Clone, Copy)]
pub(crate) struct XrLoaderInitInfoBaseHeaderKHR {
    pub(crate) ty: XrStructureType,
    pub(crate) next: *const c_void,
}

/// `XrGraphicsBindingVulkanKHR` (`XrGraphicsBindingVulkan2KHR`).
#[repr(C)]
#[derive(Clone, Copy)]
pub(crate) struct XrGraphicsBindingVulkanKHR {
    pub(crate) ty: XrStructureType,
    pub(crate) next: *const c_void,
    pub(crate) instance: VkInstance,
    pub(crate) physical_device: VkPhysicalDevice,
    pub(crate) device: VkDevice,
    pub(crate) queue_family_index: u32,
    pub(crate) queue_index: u32,
}

/// `XrSwapchainImageVulkanKHR` (`XrSwapchainImageVulkan2KHR`).
#[repr(C)]
#[derive(Clone, Copy)]
pub(crate) struct XrSwapchainImageVulkanKHR {
    pub(crate) ty: XrStructureType,
    pub(crate) next: *mut c_void,
    pub(crate) image: VkImage,
}

/// `XrGraphicsRequirementsVulkanKHR` (`XrGraphicsRequirementsVulkan2KHR`).
#[repr(C)]
#[derive(Clone, Copy)]
pub(crate) struct XrGraphicsRequirementsVulkanKHR {
    pub(crate) ty: XrStructureType,
    pub(crate) next: *mut c_void,
    pub(crate) min_api_version_supported: XrVersion,
    pub(crate) max_api_version_supported: XrVersion,
}

/// `XrVulkanInstanceCreateInfoKHR`.
#[repr(C)]
#[derive(Clone, Copy)]
pub(crate) struct XrVulkanInstanceCreateInfoKHR {
    pub(crate) ty: XrStructureType,
    pub(crate) next: *const c_void,
    pub(crate) system_id: XrSystemId,
    pub(crate) create_flags: XrVulkanInstanceCreateFlagsKHR,
    pub(crate) pfn_get_instance_proc_addr: Option<crate::video::vk::PfnVkGetInstanceProcAddr>,
    /// `const VkInstanceCreateInfo *`.
    pub(crate) vulkan_create_info: *const c_void,
    /// `const VkAllocationCallbacks *`.
    pub(crate) vulkan_allocator: *const c_void,
}

/// `XrVulkanDeviceCreateInfoKHR`.
#[repr(C)]
#[derive(Clone, Copy)]
pub(crate) struct XrVulkanDeviceCreateInfoKHR {
    pub(crate) ty: XrStructureType,
    pub(crate) next: *const c_void,
    pub(crate) system_id: XrSystemId,
    pub(crate) create_flags: XrVulkanDeviceCreateFlagsKHR,
    pub(crate) pfn_get_instance_proc_addr: Option<crate::video::vk::PfnVkGetInstanceProcAddr>,
    pub(crate) vulkan_physical_device: VkPhysicalDevice,
    /// `const VkDeviceCreateInfo *`.
    pub(crate) vulkan_create_info: *const c_void,
    /// `const VkAllocationCallbacks *`.
    pub(crate) vulkan_allocator: *const c_void,
}

/// `XrVulkanGraphicsDeviceGetInfoKHR`.
#[repr(C)]
#[derive(Clone, Copy)]
pub(crate) struct XrVulkanGraphicsDeviceGetInfoKHR {
    pub(crate) ty: XrStructureType,
    pub(crate) next: *const c_void,
    pub(crate) system_id: XrSystemId,
    pub(crate) vulkan_instance: VkInstance,
}

/// `LUID` (winnt.h).
#[repr(C)]
#[derive(Clone, Copy, Default, PartialEq, Eq, Debug)]
pub(crate) struct Luid {
    pub(crate) low_part: u32,
    pub(crate) high_part: i32,
}

/// `XrGraphicsBindingD3D12KHR`.
#[repr(C)]
#[derive(Clone, Copy)]
pub(crate) struct XrGraphicsBindingD3D12KHR {
    pub(crate) ty: XrStructureType,
    pub(crate) next: *const c_void,
    /// `ID3D12Device *`.
    pub(crate) device: *mut c_void,
    /// `ID3D12CommandQueue *`.
    pub(crate) queue: *mut c_void,
}

/// `XrSwapchainImageD3D12KHR`.
#[repr(C)]
#[derive(Clone, Copy)]
pub(crate) struct XrSwapchainImageD3D12KHR {
    pub(crate) ty: XrStructureType,
    pub(crate) next: *mut c_void,
    /// `ID3D12Resource *`.
    pub(crate) texture: *mut c_void,
}

/// `XrGraphicsRequirementsD3D12KHR`.
#[repr(C)]
#[derive(Clone, Copy)]
pub(crate) struct XrGraphicsRequirementsD3D12KHR {
    pub(crate) ty: XrStructureType,
    pub(crate) next: *mut c_void,
    pub(crate) adapter_luid: Luid,
    /// `D3D_FEATURE_LEVEL`.
    pub(crate) min_feature_level: i32,
}

/// `XrLoaderInitInfoAndroidKHR`.
#[repr(C)]
#[derive(Clone, Copy)]
pub(crate) struct XrLoaderInitInfoAndroidKHR {
    pub(crate) ty: XrStructureType,
    pub(crate) next: *const c_void,
    /// `void *` (the `JavaVM *`).
    pub(crate) application_vm: *mut c_void,
    /// `void *` (a `jobject`: the activity or another context).
    pub(crate) application_context: *mut c_void,
}

/// `XrInstanceCreateInfoAndroidKHR`.
#[repr(C)]
#[derive(Clone, Copy)]
pub(crate) struct XrInstanceCreateInfoAndroidKHR {
    pub(crate) ty: XrStructureType,
    pub(crate) next: *const c_void,
    /// `void *` (the `JavaVM *`).
    pub(crate) application_vm: *mut c_void,
    /// `void *` (the activity, a `jobject`).
    pub(crate) application_activity: *mut c_void,
}

/// All-zero `Default`s (C's `SDL_zero()`); every field is an integer, an
/// array of `c_char`, a raw pointer or an `Option` of a function pointer.
macro_rules! zeroed_default {
    ($($t:ty),* $(,)?) => {$(
        impl Default for $t {
            fn default() -> Self {
                // SAFETY: every field is valid as all zeroes (above).
                unsafe { std::mem::zeroed() }
            }
        }
    )*};
}
zeroed_default!(
    XrApiLayerProperties,
    XrExtensionProperties,
    XrApplicationInfo,
    XrInstanceCreateInfo,
    XrSystemGetInfo,
    XrSessionCreateInfo,
    XrSwapchainCreateInfo,
    XrSwapchainImageBaseHeader,
    XrLoaderInitInfoBaseHeaderKHR,
    XrGraphicsBindingVulkanKHR,
    XrSwapchainImageVulkanKHR,
    XrGraphicsRequirementsVulkanKHR,
    XrVulkanInstanceCreateInfoKHR,
    XrVulkanDeviceCreateInfoKHR,
    XrVulkanGraphicsDeviceGetInfoKHR,
    XrGraphicsBindingD3D12KHR,
    XrSwapchainImageD3D12KHR,
    XrGraphicsRequirementsD3D12KHR,
    XrLoaderInitInfoAndroidKHR,
    XrInstanceCreateInfoAndroidKHR,
);

// Function types

/// `PFN_xrVoidFunction`.
pub(crate) type PfnXrVoidFunction = unsafe extern "system" fn();
/// `PFN_xrGetInstanceProcAddr`.
pub(crate) type PfnXrGetInstanceProcAddr = unsafe extern "system" fn(
    instance: XrInstance,
    name: *const c_char,
    function: *mut Option<PfnXrVoidFunction>,
) -> XrResult;
/// `PFN_xrEnumerateApiLayerProperties`.
pub(crate) type PfnXrEnumerateApiLayerProperties = unsafe extern "system" fn(
    property_capacity_input: u32,
    property_count_output: *mut u32,
    properties: *mut XrApiLayerProperties,
) -> XrResult;
/// `PFN_xrEnumerateInstanceExtensionProperties`.
pub(crate) type PfnXrEnumerateInstanceExtensionProperties = unsafe extern "system" fn(
    layer_name: *const c_char,
    property_capacity_input: u32,
    property_count_output: *mut u32,
    properties: *mut XrExtensionProperties,
) -> XrResult;
/// `PFN_xrCreateInstance`.
pub(crate) type PfnXrCreateInstance = unsafe extern "system" fn(
    create_info: *const XrInstanceCreateInfo,
    instance: *mut XrInstance,
) -> XrResult;
/// `PFN_xrDestroyInstance`.
pub(crate) type PfnXrDestroyInstance = unsafe extern "system" fn(instance: XrInstance) -> XrResult;
/// `PFN_xrGetSystem`.
pub(crate) type PfnXrGetSystem = unsafe extern "system" fn(
    instance: XrInstance,
    get_info: *const XrSystemGetInfo,
    system_id: *mut XrSystemId,
) -> XrResult;
/// `PFN_xrCreateSession`.
pub(crate) type PfnXrCreateSession = unsafe extern "system" fn(
    instance: XrInstance,
    create_info: *const XrSessionCreateInfo,
    session: *mut XrSession,
) -> XrResult;
/// `PFN_xrEnumerateSwapchainFormats`.
pub(crate) type PfnXrEnumerateSwapchainFormats = unsafe extern "system" fn(
    session: XrSession,
    format_capacity_input: u32,
    format_count_output: *mut u32,
    formats: *mut i64,
) -> XrResult;
/// `PFN_xrCreateSwapchain`.
pub(crate) type PfnXrCreateSwapchain = unsafe extern "system" fn(
    session: XrSession,
    create_info: *const XrSwapchainCreateInfo,
    swapchain: *mut XrSwapchain,
) -> XrResult;
/// `PFN_xrDestroySwapchain`.
pub(crate) type PfnXrDestroySwapchain =
    unsafe extern "system" fn(swapchain: XrSwapchain) -> XrResult;
/// `PFN_xrEnumerateSwapchainImages`.
pub(crate) type PfnXrEnumerateSwapchainImages = unsafe extern "system" fn(
    swapchain: XrSwapchain,
    image_capacity_input: u32,
    image_count_output: *mut u32,
    images: *mut XrSwapchainImageBaseHeader,
) -> XrResult;
/// `PFN_xrInitializeLoaderKHR`.
pub(crate) type PfnXrInitializeLoaderKHR =
    unsafe extern "system" fn(loader_init_info: *const XrLoaderInitInfoBaseHeaderKHR) -> XrResult;
/// `PFN_xrGetVulkanGraphicsRequirements2KHR`.
pub(crate) type PfnXrGetVulkanGraphicsRequirements2KHR = unsafe extern "system" fn(
    instance: XrInstance,
    system_id: XrSystemId,
    graphics_requirements: *mut XrGraphicsRequirementsVulkanKHR,
) -> XrResult;
/// `PFN_xrCreateVulkanInstanceKHR`.
pub(crate) type PfnXrCreateVulkanInstanceKHR = unsafe extern "system" fn(
    instance: XrInstance,
    create_info: *const XrVulkanInstanceCreateInfoKHR,
    vulkan_instance: *mut VkInstance,
    vulkan_result: *mut i32,
) -> XrResult;
/// `PFN_xrCreateVulkanDeviceKHR`.
pub(crate) type PfnXrCreateVulkanDeviceKHR = unsafe extern "system" fn(
    instance: XrInstance,
    create_info: *const XrVulkanDeviceCreateInfoKHR,
    vulkan_device: *mut VkDevice,
    vulkan_result: *mut i32,
) -> XrResult;
/// `PFN_xrGetVulkanGraphicsDevice2KHR`.
pub(crate) type PfnXrGetVulkanGraphicsDevice2KHR = unsafe extern "system" fn(
    instance: XrInstance,
    get_info: *const XrVulkanGraphicsDeviceGetInfoKHR,
    vulkan_physical_device: *mut VkPhysicalDevice,
) -> XrResult;
/// `PFN_xrGetD3D12GraphicsRequirementsKHR`.
pub(crate) type PfnXrGetD3D12GraphicsRequirementsKHR = unsafe extern "system" fn(
    instance: XrInstance,
    system_id: XrSystemId,
    graphics_requirements: *mut XrGraphicsRequirementsD3D12KHR,
) -> XrResult;

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(target_pointer_width = "64")]
    use std::mem::{align_of, offset_of, size_of};

    /// The sizes, alignments and offsets the scratchpad harness printed
    /// (layout.c, built against upstream's vendored headers at ea7a2da) on
    /// x86-64 Linux (gcc), x86-64 Windows (mingw-w64 gcc, on the runner)
    /// and x86-64 Android (the NDK's clang, on the emulator).
    #[cfg(target_pointer_width = "64")]
    #[test]
    fn layouts_match_c() {
        macro_rules! size {
            ($t:ty, $size:expr) => {
                assert_eq!(
                    (size_of::<$t>(), align_of::<$t>()),
                    ($size, 8),
                    stringify!($t)
                );
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
        size!(XrApiLayerProperties, 544);
        off!(XrApiLayerProperties, next, 8);
        off!(XrApiLayerProperties, layer_name, 16);
        off!(XrApiLayerProperties, spec_version, 272);
        off!(XrApiLayerProperties, layer_version, 280);
        off!(XrApiLayerProperties, description, 284);
        size!(XrExtensionProperties, 152);
        off!(XrExtensionProperties, extension_name, 16);
        off!(XrExtensionProperties, extension_version, 144);
        size!(XrApplicationInfo, 272);
        off!(XrApplicationInfo, application_version, 128);
        off!(XrApplicationInfo, engine_name, 132);
        off!(XrApplicationInfo, engine_version, 260);
        off!(XrApplicationInfo, api_version, 264);
        size!(XrInstanceCreateInfo, 328);
        off!(XrInstanceCreateInfo, create_flags, 16);
        off!(XrInstanceCreateInfo, application_info, 24);
        off!(XrInstanceCreateInfo, enabled_api_layer_count, 296);
        off!(XrInstanceCreateInfo, enabled_api_layer_names, 304);
        off!(XrInstanceCreateInfo, enabled_extension_count, 312);
        off!(XrInstanceCreateInfo, enabled_extension_names, 320);
        size!(XrSystemGetInfo, 24);
        off!(XrSystemGetInfo, form_factor, 16);
        size!(XrSessionCreateInfo, 32);
        off!(XrSessionCreateInfo, create_flags, 16);
        off!(XrSessionCreateInfo, system_id, 24);
        size!(XrSwapchainCreateInfo, 64);
        off!(XrSwapchainCreateInfo, create_flags, 16);
        off!(XrSwapchainCreateInfo, usage_flags, 24);
        off!(XrSwapchainCreateInfo, format, 32);
        off!(XrSwapchainCreateInfo, sample_count, 40);
        off!(XrSwapchainCreateInfo, width, 44);
        off!(XrSwapchainCreateInfo, height, 48);
        off!(XrSwapchainCreateInfo, face_count, 52);
        off!(XrSwapchainCreateInfo, array_size, 56);
        off!(XrSwapchainCreateInfo, mip_count, 60);
        size!(XrSwapchainImageBaseHeader, 16);
        off!(XrSwapchainImageBaseHeader, next, 8);
        size!(XrGraphicsBindingVulkanKHR, 48);
        off!(XrGraphicsBindingVulkanKHR, instance, 16);
        off!(XrGraphicsBindingVulkanKHR, physical_device, 24);
        off!(XrGraphicsBindingVulkanKHR, device, 32);
        off!(XrGraphicsBindingVulkanKHR, queue_family_index, 40);
        off!(XrGraphicsBindingVulkanKHR, queue_index, 44);
        size!(XrSwapchainImageVulkanKHR, 24);
        off!(XrSwapchainImageVulkanKHR, image, 16);
        size!(XrGraphicsRequirementsVulkanKHR, 32);
        off!(
            XrGraphicsRequirementsVulkanKHR,
            min_api_version_supported,
            16
        );
        off!(
            XrGraphicsRequirementsVulkanKHR,
            max_api_version_supported,
            24
        );
        size!(XrVulkanInstanceCreateInfoKHR, 56);
        off!(XrVulkanInstanceCreateInfoKHR, system_id, 16);
        off!(XrVulkanInstanceCreateInfoKHR, create_flags, 24);
        off!(
            XrVulkanInstanceCreateInfoKHR,
            pfn_get_instance_proc_addr,
            32
        );
        off!(XrVulkanInstanceCreateInfoKHR, vulkan_create_info, 40);
        off!(XrVulkanInstanceCreateInfoKHR, vulkan_allocator, 48);
        size!(XrVulkanDeviceCreateInfoKHR, 64);
        off!(XrVulkanDeviceCreateInfoKHR, system_id, 16);
        off!(XrVulkanDeviceCreateInfoKHR, create_flags, 24);
        off!(XrVulkanDeviceCreateInfoKHR, pfn_get_instance_proc_addr, 32);
        off!(XrVulkanDeviceCreateInfoKHR, vulkan_physical_device, 40);
        off!(XrVulkanDeviceCreateInfoKHR, vulkan_create_info, 48);
        off!(XrVulkanDeviceCreateInfoKHR, vulkan_allocator, 56);
        size!(XrVulkanGraphicsDeviceGetInfoKHR, 32);
        off!(XrVulkanGraphicsDeviceGetInfoKHR, system_id, 16);
        off!(XrVulkanGraphicsDeviceGetInfoKHR, vulkan_instance, 24);
        // (Windows's)
        assert_eq!((size_of::<Luid>(), align_of::<Luid>()), (8, 4));
        size!(XrGraphicsBindingD3D12KHR, 32);
        off!(XrGraphicsBindingD3D12KHR, device, 16);
        off!(XrGraphicsBindingD3D12KHR, queue, 24);
        size!(XrSwapchainImageD3D12KHR, 24);
        off!(XrSwapchainImageD3D12KHR, texture, 16);
        size!(XrGraphicsRequirementsD3D12KHR, 32);
        off!(XrGraphicsRequirementsD3D12KHR, adapter_luid, 16);
        off!(XrGraphicsRequirementsD3D12KHR, min_feature_level, 24);
        // (Android's)
        size!(XrLoaderInitInfoAndroidKHR, 32);
        off!(XrLoaderInitInfoAndroidKHR, application_vm, 16);
        off!(XrLoaderInitInfoAndroidKHR, application_context, 24);
        size!(XrInstanceCreateInfoAndroidKHR, 32);
        off!(XrInstanceCreateInfoAndroidKHR, application_vm, 16);
        off!(XrInstanceCreateInfoAndroidKHR, application_activity, 24);
    }

    /// The values the harness printed.
    #[test]
    fn values_match_c() {
        assert_eq!(XR_TYPE_API_LAYER_PROPERTIES, 1);
        assert_eq!(XR_TYPE_EXTENSION_PROPERTIES, 2);
        assert_eq!(XR_TYPE_INSTANCE_CREATE_INFO, 3);
        assert_eq!(XR_TYPE_SYSTEM_GET_INFO, 4);
        assert_eq!(XR_TYPE_SESSION_CREATE_INFO, 8);
        assert_eq!(XR_TYPE_SWAPCHAIN_CREATE_INFO, 9);
        assert_eq!(XR_TYPE_GRAPHICS_BINDING_VULKAN_KHR, 1000025000);
        assert_eq!(XR_TYPE_SWAPCHAIN_IMAGE_VULKAN_KHR, 1000025001);
        assert_eq!(XR_TYPE_GRAPHICS_REQUIREMENTS_VULKAN_KHR, 1000025002);
        assert_eq!(XR_TYPE_GRAPHICS_BINDING_D3D12_KHR, 1000028000);
        assert_eq!(XR_TYPE_SWAPCHAIN_IMAGE_D3D12_KHR, 1000028001);
        assert_eq!(XR_TYPE_GRAPHICS_REQUIREMENTS_D3D12_KHR, 1000028002);
        assert_eq!(XR_TYPE_VULKAN_INSTANCE_CREATE_INFO_KHR, 1000090000);
        assert_eq!(XR_TYPE_VULKAN_DEVICE_CREATE_INFO_KHR, 1000090001);
        assert_eq!(XR_TYPE_VULKAN_GRAPHICS_DEVICE_GET_INFO_KHR, 1000090003);
        assert_eq!(XR_TYPE_LOADER_INIT_INFO_ANDROID_KHR, 1000089000);
        assert_eq!(XR_TYPE_INSTANCE_CREATE_INFO_ANDROID_KHR, 1000008000);
        assert_eq!(XR_FORM_FACTOR_HEAD_MOUNTED_DISPLAY, 1);
        assert_eq!(XR_FORM_FACTOR_HANDHELD_DISPLAY, 2);
        assert_eq!(XR_SWAPCHAIN_USAGE_COLOR_ATTACHMENT_BIT, 1);
        assert_eq!(XR_SWAPCHAIN_USAGE_DEPTH_STENCIL_ATTACHMENT_BIT, 2);
        assert_eq!(XR_SWAPCHAIN_USAGE_UNORDERED_ACCESS_BIT, 4);
        assert_eq!(XR_SWAPCHAIN_USAGE_TRANSFER_SRC_BIT, 8);
        assert_eq!(XR_SWAPCHAIN_USAGE_SAMPLED_BIT, 32);
        assert_eq!(XrResult::SUCCESS.0, 0);
        assert_eq!(XrResult::ERROR_VALIDATION_FAILURE.0, -1);
        assert_eq!(XrResult::ERROR_RUNTIME_FAILURE.0, -2);
        assert_eq!(XrResult::ERROR_INITIALIZATION_FAILED.0, -6);
        assert_eq!(XrResult::ERROR_FUNCTION_UNSUPPORTED.0, -7);
        assert_eq!(XrResult::ERROR_EXTENSION_NOT_PRESENT.0, -9);
        assert_eq!(XrResult::ERROR_HANDLE_INVALID.0, -12);
        assert_eq!(XrResult::ERROR_FORM_FACTOR_UNSUPPORTED.0, -34);
        assert_eq!(XrResult::ERROR_FORM_FACTOR_UNAVAILABLE.0, -35);
        // 1.0 with the headers' patch: 1.0.63
        assert_eq!(XR_API_VERSION_1_0, 0x0001_0000_0000_003f);
        assert_eq!(xr_version_major(XR_CURRENT_API_VERSION), 1);
        assert_eq!(xr_version_minor(XR_CURRENT_API_VERSION), 1);
    }
}
