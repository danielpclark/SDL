// The parts of the Vulkan headers (src/video/khronos/vulkan/vulkan_core.h)
// the Vulkan renderer of Simple DirectMedia Layer uses, with its
// VULKAN_FUNCTIONS() table from src/render/vulkan/SDL_render_vulkan.c.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! Vulkan declarations: handles, enumerants, structures and the function
//! tables of the renderer, looked up at run time through the loader's
//! `vkGetInstanceProcAddr` (`VK_NO_PROTOTYPES`).
//!
//! The structures implement `Default` as all zeroes, which is what the C
//! code's `= { 0 }` initializers give.

use std::ffi::{c_char, c_void, CStr};

pub(super) use crate::video::vulkan_utils::{
    PfnVkGetInstanceProcAddr, PfnVkVoidFunction, VkExtensionProperties, VkResult, VK_SUCCESS,
};

pub(super) type VkBool32 = u32;
pub(super) type VkDeviceSize = u64;
pub(super) type VkFlags = u32;

// Dispatchable handles
pub(super) type VkInstance = *mut c_void;
pub(super) type VkPhysicalDevice = *mut c_void;
pub(super) type VkDevice = *mut c_void;
pub(super) type VkQueue = *mut c_void;
pub(super) type VkCommandBuffer = *mut c_void;

// Non-dispatchable handles
pub(super) type VkSemaphore = u64;
pub(super) type VkFence = u64;
pub(super) type VkDeviceMemory = u64;
pub(super) type VkBuffer = u64;
pub(super) type VkImage = u64;
pub(super) type VkImageView = u64;
pub(super) type VkShaderModule = u64;
pub(super) type VkPipelineCache = u64;
pub(super) type VkPipelineLayout = u64;
pub(super) type VkRenderPass = u64;
pub(super) type VkPipeline = u64;
pub(super) type VkDescriptorSetLayout = u64;
pub(super) type VkSampler = u64;
pub(super) type VkDescriptorPool = u64;
pub(super) type VkDescriptorSet = u64;
pub(super) type VkFramebuffer = u64;
pub(super) type VkCommandPool = u64;
pub(super) type VkSurfaceKHR = u64;
pub(super) type VkSwapchainKHR = u64;

/// `VK_NULL_HANDLE` (of a non-dispatchable handle).
pub(super) const VK_NULL_HANDLE: u64 = 0;

pub(super) const VK_TRUE: VkBool32 = 1;
pub(super) const VK_FALSE: VkBool32 = 0;
pub(super) const VK_QUEUE_FAMILY_IGNORED: u32 = !0;
pub(super) const VK_SUBPASS_EXTERNAL: u32 = !0;
pub(super) const VK_MAX_PHYSICAL_DEVICE_NAME_SIZE: usize = 256;
pub(super) const VK_UUID_SIZE: usize = 16;
pub(super) const VK_MAX_MEMORY_TYPES: usize = 32;
pub(super) const VK_MAX_MEMORY_HEAPS: usize = 16;
pub(super) const VK_MAX_EXTENSION_NAME_SIZE: usize = 256;
pub(super) const VK_MAX_DESCRIPTION_SIZE: usize = 256;

/// `VK_MAKE_API_VERSION(0, 1, 0, 0)`
pub(super) const VK_API_VERSION_1_0: u32 = 1 << 22;

/// `VK_VERSION_MAJOR()`
pub(super) const fn vk_version_major(version: u32) -> u32 {
    version >> 22
}

// VkResult
pub(super) const VK_SUBOPTIMAL_KHR: VkResult = 1000001003;
pub(super) const VK_ERROR_OUT_OF_DEVICE_MEMORY: VkResult = -2;
pub(super) const VK_ERROR_DEVICE_LOST: VkResult = -4;
pub(super) const VK_ERROR_UNKNOWN: VkResult = -13;
pub(super) const VK_ERROR_SURFACE_LOST_KHR: VkResult = -1000000000;
pub(super) const VK_ERROR_OUT_OF_DATE_KHR: VkResult = -1000001004;

// VkStructureType
pub(super) type VkStructureType = i32;
pub(super) const VK_STRUCTURE_TYPE_APPLICATION_INFO: VkStructureType = 0;
pub(super) const VK_STRUCTURE_TYPE_INSTANCE_CREATE_INFO: VkStructureType = 1;
pub(super) const VK_STRUCTURE_TYPE_DEVICE_QUEUE_CREATE_INFO: VkStructureType = 2;
pub(super) const VK_STRUCTURE_TYPE_DEVICE_CREATE_INFO: VkStructureType = 3;
pub(super) const VK_STRUCTURE_TYPE_SUBMIT_INFO: VkStructureType = 4;
pub(super) const VK_STRUCTURE_TYPE_MEMORY_ALLOCATE_INFO: VkStructureType = 5;
pub(super) const VK_STRUCTURE_TYPE_FENCE_CREATE_INFO: VkStructureType = 8;
pub(super) const VK_STRUCTURE_TYPE_SEMAPHORE_CREATE_INFO: VkStructureType = 9;
pub(super) const VK_STRUCTURE_TYPE_BUFFER_CREATE_INFO: VkStructureType = 12;
pub(super) const VK_STRUCTURE_TYPE_IMAGE_CREATE_INFO: VkStructureType = 14;
pub(super) const VK_STRUCTURE_TYPE_IMAGE_VIEW_CREATE_INFO: VkStructureType = 15;
pub(super) const VK_STRUCTURE_TYPE_SHADER_MODULE_CREATE_INFO: VkStructureType = 16;
pub(super) const VK_STRUCTURE_TYPE_PIPELINE_SHADER_STAGE_CREATE_INFO: VkStructureType = 18;
pub(super) const VK_STRUCTURE_TYPE_PIPELINE_VERTEX_INPUT_STATE_CREATE_INFO: VkStructureType = 19;
pub(super) const VK_STRUCTURE_TYPE_PIPELINE_INPUT_ASSEMBLY_STATE_CREATE_INFO: VkStructureType = 20;
pub(super) const VK_STRUCTURE_TYPE_PIPELINE_VIEWPORT_STATE_CREATE_INFO: VkStructureType = 22;
pub(super) const VK_STRUCTURE_TYPE_PIPELINE_RASTERIZATION_STATE_CREATE_INFO: VkStructureType = 23;
pub(super) const VK_STRUCTURE_TYPE_PIPELINE_MULTISAMPLE_STATE_CREATE_INFO: VkStructureType = 24;
pub(super) const VK_STRUCTURE_TYPE_PIPELINE_DEPTH_STENCIL_STATE_CREATE_INFO: VkStructureType = 25;
pub(super) const VK_STRUCTURE_TYPE_PIPELINE_COLOR_BLEND_STATE_CREATE_INFO: VkStructureType = 26;
pub(super) const VK_STRUCTURE_TYPE_PIPELINE_DYNAMIC_STATE_CREATE_INFO: VkStructureType = 27;
pub(super) const VK_STRUCTURE_TYPE_GRAPHICS_PIPELINE_CREATE_INFO: VkStructureType = 28;
pub(super) const VK_STRUCTURE_TYPE_PIPELINE_LAYOUT_CREATE_INFO: VkStructureType = 30;
pub(super) const VK_STRUCTURE_TYPE_SAMPLER_CREATE_INFO: VkStructureType = 31;
pub(super) const VK_STRUCTURE_TYPE_DESCRIPTOR_SET_LAYOUT_CREATE_INFO: VkStructureType = 32;
pub(super) const VK_STRUCTURE_TYPE_DESCRIPTOR_POOL_CREATE_INFO: VkStructureType = 33;
pub(super) const VK_STRUCTURE_TYPE_DESCRIPTOR_SET_ALLOCATE_INFO: VkStructureType = 34;
pub(super) const VK_STRUCTURE_TYPE_WRITE_DESCRIPTOR_SET: VkStructureType = 35;
pub(super) const VK_STRUCTURE_TYPE_FRAMEBUFFER_CREATE_INFO: VkStructureType = 37;
pub(super) const VK_STRUCTURE_TYPE_RENDER_PASS_CREATE_INFO: VkStructureType = 38;
pub(super) const VK_STRUCTURE_TYPE_COMMAND_POOL_CREATE_INFO: VkStructureType = 39;
pub(super) const VK_STRUCTURE_TYPE_COMMAND_BUFFER_ALLOCATE_INFO: VkStructureType = 40;
pub(super) const VK_STRUCTURE_TYPE_COMMAND_BUFFER_BEGIN_INFO: VkStructureType = 42;
pub(super) const VK_STRUCTURE_TYPE_RENDER_PASS_BEGIN_INFO: VkStructureType = 43;
pub(super) const VK_STRUCTURE_TYPE_IMAGE_MEMORY_BARRIER: VkStructureType = 45;
pub(super) const VK_STRUCTURE_TYPE_SWAPCHAIN_CREATE_INFO_KHR: VkStructureType = 1000001000;
pub(super) const VK_STRUCTURE_TYPE_PRESENT_INFO_KHR: VkStructureType = 1000001001;
pub(super) const VK_STRUCTURE_TYPE_IMAGE_VIEW_USAGE_CREATE_INFO: VkStructureType = 1000117002;
pub(super) const VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_SAMPLER_YCBCR_CONVERSION_FEATURES:
    VkStructureType = 1000156004;

// VkFormat
pub(super) type VkFormat = i32;
pub(super) const VK_FORMAT_UNDEFINED: VkFormat = 0;
pub(super) const VK_FORMAT_R4G4B4A4_UNORM_PACK16: VkFormat = 2;
pub(super) const VK_FORMAT_B4G4R4A4_UNORM_PACK16: VkFormat = 3;
pub(super) const VK_FORMAT_R5G6B5_UNORM_PACK16: VkFormat = 4;
pub(super) const VK_FORMAT_B5G6R5_UNORM_PACK16: VkFormat = 5;
pub(super) const VK_FORMAT_R5G5B5A1_UNORM_PACK16: VkFormat = 6;
pub(super) const VK_FORMAT_B5G5R5A1_UNORM_PACK16: VkFormat = 7;
pub(super) const VK_FORMAT_A1R5G5B5_UNORM_PACK16: VkFormat = 8;
pub(super) const VK_FORMAT_R8_UNORM: VkFormat = 9;
pub(super) const VK_FORMAT_R8G8_UNORM: VkFormat = 16;
pub(super) const VK_FORMAT_R8G8B8A8_UNORM: VkFormat = 37;
pub(super) const VK_FORMAT_R8G8B8A8_SRGB: VkFormat = 43;
pub(super) const VK_FORMAT_B8G8R8A8_UNORM: VkFormat = 44;
pub(super) const VK_FORMAT_B8G8R8A8_SRGB: VkFormat = 50;
#[cfg(target_endian = "big")]
pub(super) const VK_FORMAT_A8B8G8R8_UNORM_PACK32: VkFormat = 51;
#[cfg(target_endian = "big")]
pub(super) const VK_FORMAT_A8B8G8R8_SRGB_PACK32: VkFormat = 57;
pub(super) const VK_FORMAT_A2B10G10R10_UNORM_PACK32: VkFormat = 64;
pub(super) const VK_FORMAT_R16_UNORM: VkFormat = 70;
pub(super) const VK_FORMAT_R16G16_UNORM: VkFormat = 77;
pub(super) const VK_FORMAT_R16G16B16A16_SFLOAT: VkFormat = 97;
pub(super) const VK_FORMAT_R32G32_SFLOAT: VkFormat = 103;
pub(super) const VK_FORMAT_R32G32B32A32_SFLOAT: VkFormat = 109;
pub(super) const VK_FORMAT_G8_B8_R8_3PLANE_420_UNORM: VkFormat = 1000156002;
pub(super) const VK_FORMAT_G8_B8R8_2PLANE_420_UNORM: VkFormat = 1000156003;
pub(super) const VK_FORMAT_G8_B8_R8_3PLANE_444_UNORM: VkFormat = 1000156006;
pub(super) const VK_FORMAT_G10X6_B10X6R10X6_2PLANE_420_UNORM_3PACK16: VkFormat = 1000156013;
pub(super) const VK_FORMAT_G16_B16_R16_3PLANE_420_UNORM: VkFormat = 1000156029;
pub(super) const VK_FORMAT_G16_B16_R16_3PLANE_444_UNORM: VkFormat = 1000156033;
pub(super) const VK_FORMAT_A4R4G4B4_UNORM_PACK16_EXT: VkFormat = 1000340000;
pub(super) const VK_FORMAT_A4B4G4R4_UNORM_PACK16_EXT: VkFormat = 1000340001;

// VkColorSpaceKHR
pub(super) type VkColorSpaceKHR = i32;
pub(super) const VK_COLOR_SPACE_SRGB_NONLINEAR_KHR: VkColorSpaceKHR = 0;
pub(super) const VK_COLOR_SPACE_EXTENDED_SRGB_LINEAR_EXT: VkColorSpaceKHR = 1000104002;
pub(super) const VK_COLOR_SPACE_HDR10_ST2084_EXT: VkColorSpaceKHR = 1000104008;

// VkImageLayout
pub(super) type VkImageLayout = i32;
pub(super) const VK_IMAGE_LAYOUT_UNDEFINED: VkImageLayout = 0;
pub(super) const VK_IMAGE_LAYOUT_COLOR_ATTACHMENT_OPTIMAL: VkImageLayout = 2;
pub(super) const VK_IMAGE_LAYOUT_SHADER_READ_ONLY_OPTIMAL: VkImageLayout = 5;
pub(super) const VK_IMAGE_LAYOUT_TRANSFER_SRC_OPTIMAL: VkImageLayout = 6;
pub(super) const VK_IMAGE_LAYOUT_TRANSFER_DST_OPTIMAL: VkImageLayout = 7;
pub(super) const VK_IMAGE_LAYOUT_PRESENT_SRC_KHR: VkImageLayout = 1000001002;

// VkAccessFlagBits
pub(super) type VkAccessFlags = VkFlags;
pub(super) const VK_ACCESS_NONE: VkAccessFlags = 0;
pub(super) const VK_ACCESS_SHADER_READ_BIT: VkAccessFlags = 0x00000020;
pub(super) const VK_ACCESS_COLOR_ATTACHMENT_READ_BIT: VkAccessFlags = 0x00000080;
pub(super) const VK_ACCESS_COLOR_ATTACHMENT_WRITE_BIT: VkAccessFlags = 0x00000100;
pub(super) const VK_ACCESS_TRANSFER_READ_BIT: VkAccessFlags = 0x00000800;
pub(super) const VK_ACCESS_TRANSFER_WRITE_BIT: VkAccessFlags = 0x00001000;

// VkPipelineStageFlagBits
pub(super) type VkPipelineStageFlags = VkFlags;
pub(super) const VK_PIPELINE_STAGE_TOP_OF_PIPE_BIT: VkPipelineStageFlags = 0x00000001;
pub(super) const VK_PIPELINE_STAGE_FRAGMENT_SHADER_BIT: VkPipelineStageFlags = 0x00000080;
pub(super) const VK_PIPELINE_STAGE_COLOR_ATTACHMENT_OUTPUT_BIT: VkPipelineStageFlags = 0x00000400;
pub(super) const VK_PIPELINE_STAGE_TRANSFER_BIT: VkPipelineStageFlags = 0x00001000;
pub(super) const VK_PIPELINE_STAGE_ALL_COMMANDS_BIT: VkPipelineStageFlags = 0x00010000;

// VkImageAspectFlagBits
pub(super) type VkImageAspectFlags = VkFlags;
pub(super) const VK_IMAGE_ASPECT_COLOR_BIT: VkImageAspectFlags = 0x00000001;
pub(super) const VK_IMAGE_ASPECT_PLANE_0_BIT: VkImageAspectFlags = 0x00000010;

// VkImageUsageFlagBits
pub(super) type VkImageUsageFlags = VkFlags;
pub(super) const VK_IMAGE_USAGE_TRANSFER_SRC_BIT: VkImageUsageFlags = 0x00000001;
pub(super) const VK_IMAGE_USAGE_TRANSFER_DST_BIT: VkImageUsageFlags = 0x00000002;
pub(super) const VK_IMAGE_USAGE_SAMPLED_BIT: VkImageUsageFlags = 0x00000004;
pub(super) const VK_IMAGE_USAGE_COLOR_ATTACHMENT_BIT: VkImageUsageFlags = 0x00000010;

pub(super) const VK_IMAGE_CREATE_MUTABLE_FORMAT_BIT: VkFlags = 0x00000008;
pub(super) const VK_IMAGE_TYPE_2D: i32 = 1;
pub(super) const VK_IMAGE_TILING_OPTIMAL: i32 = 0;
pub(super) const VK_IMAGE_VIEW_TYPE_2D: i32 = 1;
pub(super) const VK_SAMPLE_COUNT_1_BIT: VkFlags = 0x00000001;
pub(super) const VK_SHARING_MODE_EXCLUSIVE: i32 = 0;
pub(super) const VK_COMPONENT_SWIZZLE_IDENTITY: i32 = 0;

// VkBufferUsageFlagBits
pub(super) type VkBufferUsageFlags = VkFlags;
pub(super) const VK_BUFFER_USAGE_TRANSFER_SRC_BIT: VkBufferUsageFlags = 0x00000001;
pub(super) const VK_BUFFER_USAGE_TRANSFER_DST_BIT: VkBufferUsageFlags = 0x00000002;
pub(super) const VK_BUFFER_USAGE_UNIFORM_BUFFER_BIT: VkBufferUsageFlags = 0x00000010;
pub(super) const VK_BUFFER_USAGE_VERTEX_BUFFER_BIT: VkBufferUsageFlags = 0x00000080;

// VkMemoryPropertyFlagBits
pub(super) type VkMemoryPropertyFlags = VkFlags;
pub(super) const VK_MEMORY_PROPERTY_DEVICE_LOCAL_BIT: VkMemoryPropertyFlags = 0x00000001;
pub(super) const VK_MEMORY_PROPERTY_HOST_VISIBLE_BIT: VkMemoryPropertyFlags = 0x00000002;
pub(super) const VK_MEMORY_PROPERTY_HOST_COHERENT_BIT: VkMemoryPropertyFlags = 0x00000004;

pub(super) const VK_QUEUE_GRAPHICS_BIT: VkFlags = 0x00000001;
pub(super) const VK_COMMAND_POOL_CREATE_RESET_COMMAND_BUFFER_BIT: VkFlags = 0x00000002;
pub(super) const VK_COMMAND_BUFFER_LEVEL_PRIMARY: i32 = 0;
pub(super) const VK_FENCE_CREATE_SIGNALED_BIT: VkFlags = 0x00000001;
pub(super) const VK_DEPENDENCY_BY_REGION_BIT: VkFlags = 0x00000001;

// Render passes
pub(super) type VkAttachmentLoadOp = i32;
pub(super) const VK_ATTACHMENT_LOAD_OP_LOAD: VkAttachmentLoadOp = 0;
pub(super) const VK_ATTACHMENT_LOAD_OP_CLEAR: VkAttachmentLoadOp = 1;
pub(super) const VK_ATTACHMENT_LOAD_OP_DONT_CARE: VkAttachmentLoadOp = 2;
pub(super) const VK_ATTACHMENT_STORE_OP_STORE: i32 = 0;
pub(super) const VK_ATTACHMENT_STORE_OP_DONT_CARE: i32 = 1;
pub(super) const VK_PIPELINE_BIND_POINT_GRAPHICS: i32 = 0;
pub(super) const VK_SUBPASS_CONTENTS_INLINE: i32 = 0;

// Pipelines
pub(super) type VkPrimitiveTopology = i32;
pub(super) const VK_PRIMITIVE_TOPOLOGY_POINT_LIST: VkPrimitiveTopology = 0;
pub(super) const VK_PRIMITIVE_TOPOLOGY_LINE_LIST: VkPrimitiveTopology = 1;
pub(super) const VK_PRIMITIVE_TOPOLOGY_LINE_STRIP: VkPrimitiveTopology = 2;
pub(super) const VK_PRIMITIVE_TOPOLOGY_TRIANGLE_LIST: VkPrimitiveTopology = 3;
pub(super) const VK_SHADER_STAGE_VERTEX_BIT: VkFlags = 0x00000001;
pub(super) const VK_SHADER_STAGE_FRAGMENT_BIT: VkFlags = 0x00000010;
pub(super) const VK_VERTEX_INPUT_RATE_VERTEX: i32 = 0;
pub(super) const VK_DYNAMIC_STATE_VIEWPORT: i32 = 0;
pub(super) const VK_DYNAMIC_STATE_SCISSOR: i32 = 1;
pub(super) const VK_CULL_MODE_NONE: VkFlags = 0;
pub(super) const VK_POLYGON_MODE_FILL: i32 = 0;
pub(super) const VK_FRONT_FACE_COUNTER_CLOCKWISE: i32 = 0;
pub(super) const VK_COLOR_COMPONENT_R_BIT: VkFlags = 0x00000001;
pub(super) const VK_COLOR_COMPONENT_G_BIT: VkFlags = 0x00000002;
pub(super) const VK_COLOR_COMPONENT_B_BIT: VkFlags = 0x00000004;
pub(super) const VK_COLOR_COMPONENT_A_BIT: VkFlags = 0x00000008;

// VkBlendFactor
pub(super) type VkBlendFactor = i32;
pub(super) const VK_BLEND_FACTOR_ZERO: VkBlendFactor = 0;
pub(super) const VK_BLEND_FACTOR_ONE: VkBlendFactor = 1;
pub(super) const VK_BLEND_FACTOR_SRC_COLOR: VkBlendFactor = 2;
pub(super) const VK_BLEND_FACTOR_ONE_MINUS_SRC_COLOR: VkBlendFactor = 3;
pub(super) const VK_BLEND_FACTOR_DST_COLOR: VkBlendFactor = 4;
pub(super) const VK_BLEND_FACTOR_ONE_MINUS_DST_COLOR: VkBlendFactor = 5;
pub(super) const VK_BLEND_FACTOR_SRC_ALPHA: VkBlendFactor = 6;
pub(super) const VK_BLEND_FACTOR_ONE_MINUS_SRC_ALPHA: VkBlendFactor = 7;
pub(super) const VK_BLEND_FACTOR_DST_ALPHA: VkBlendFactor = 8;
pub(super) const VK_BLEND_FACTOR_ONE_MINUS_DST_ALPHA: VkBlendFactor = 9;
pub(super) const VK_BLEND_FACTOR_MAX_ENUM: VkBlendFactor = 0x7FFFFFFF;

// VkBlendOp
pub(super) type VkBlendOp = i32;
pub(super) const VK_BLEND_OP_ADD: VkBlendOp = 0;
pub(super) const VK_BLEND_OP_SUBTRACT: VkBlendOp = 1;
pub(super) const VK_BLEND_OP_REVERSE_SUBTRACT: VkBlendOp = 2;
pub(super) const VK_BLEND_OP_MIN: VkBlendOp = 3;
pub(super) const VK_BLEND_OP_MAX: VkBlendOp = 4;
pub(super) const VK_BLEND_OP_MAX_ENUM: VkBlendOp = 0x7FFFFFFF;

// Descriptors and samplers
pub(super) const VK_DESCRIPTOR_TYPE_SAMPLER: i32 = 0;
pub(super) const VK_DESCRIPTOR_TYPE_COMBINED_IMAGE_SAMPLER: i32 = 1;
pub(super) const VK_DESCRIPTOR_TYPE_UNIFORM_BUFFER: i32 = 6;
pub(super) const VK_FILTER_NEAREST: i32 = 0;
pub(super) const VK_FILTER_LINEAR: i32 = 1;
pub(super) const VK_SAMPLER_MIPMAP_MODE_NEAREST: i32 = 0;
pub(super) const VK_SAMPLER_ADDRESS_MODE_REPEAT: i32 = 0;
pub(super) const VK_SAMPLER_ADDRESS_MODE_CLAMP_TO_EDGE: i32 = 2;

// Surfaces and swapchains
pub(super) type VkSurfaceTransformFlagBitsKHR = VkFlags;
pub(super) const VK_SURFACE_TRANSFORM_IDENTITY_BIT_KHR: VkSurfaceTransformFlagBitsKHR = 0x00000001;
pub(super) const VK_SURFACE_TRANSFORM_ROTATE_90_BIT_KHR: VkSurfaceTransformFlagBitsKHR = 0x00000002;
pub(super) const VK_SURFACE_TRANSFORM_ROTATE_180_BIT_KHR: VkSurfaceTransformFlagBitsKHR =
    0x00000004;
pub(super) const VK_SURFACE_TRANSFORM_ROTATE_270_BIT_KHR: VkSurfaceTransformFlagBitsKHR =
    0x00000008;
pub(super) const VK_COMPOSITE_ALPHA_OPAQUE_BIT_KHR: VkFlags = 0x00000001;
pub(super) const VK_COMPOSITE_ALPHA_INHERIT_BIT_KHR: VkFlags = 0x00000008;
pub(super) type VkPresentModeKHR = i32;
pub(super) const VK_PRESENT_MODE_IMMEDIATE_KHR: VkPresentModeKHR = 0;
pub(super) const VK_PRESENT_MODE_MAILBOX_KHR: VkPresentModeKHR = 1;
pub(super) const VK_PRESENT_MODE_FIFO_KHR: VkPresentModeKHR = 2;
pub(super) const VK_PRESENT_MODE_FIFO_RELAXED_KHR: VkPresentModeKHR = 3;

/// Structures with an all-zero `Default` (the C `= { 0 }`).
macro_rules! vk_structs {
    ($(
        $(#[$m:meta])*
        struct $name:ident { $($field:ident: $ty:ty,)* }
    )*) => {$(
        $(#[$m])*
        #[repr(C)]
        #[derive(Clone, Copy)]
        pub(super) struct $name { $(pub(super) $field: $ty,)* }

        impl Default for $name {
            fn default() -> $name {
                // SAFETY: the fields are integers, floats, raw pointers and
                // arrays and structures of those, for which all zeroes is
                // a valid value.
                unsafe { std::mem::zeroed() }
            }
        }
    )*};
}

vk_structs! {
    /// `VkExtent2D`
    struct VkExtent2D { width: u32, height: u32, }
    /// `VkExtent3D`
    struct VkExtent3D { width: u32, height: u32, depth: u32, }
    /// `VkOffset2D`
    struct VkOffset2D { x: i32, y: i32, }
    /// `VkOffset3D`
    struct VkOffset3D { x: i32, y: i32, z: i32, }
    /// `VkRect2D`
    struct VkRect2D { offset: VkOffset2D, extent: VkExtent2D, }
    /// `VkViewport`
    struct VkViewport {
        x: f32,
        y: f32,
        width: f32,
        height: f32,
        min_depth: f32,
        max_depth: f32,
    }
    /// `VkClearValue` (its `color.float32`)
    struct VkClearValue { float32: [f32; 4], }

    /// `VkLayerProperties`
    struct VkLayerProperties {
        layer_name: [c_char; VK_MAX_EXTENSION_NAME_SIZE],
        spec_version: u32,
        implementation_version: u32,
        description: [c_char; VK_MAX_DESCRIPTION_SIZE],
    }
    /// `VkApplicationInfo`
    struct VkApplicationInfo {
        s_type: VkStructureType,
        p_next: *const c_void,
        p_application_name: *const c_char,
        application_version: u32,
        p_engine_name: *const c_char,
        engine_version: u32,
        api_version: u32,
    }
    /// `VkInstanceCreateInfo`
    struct VkInstanceCreateInfo {
        s_type: VkStructureType,
        p_next: *const c_void,
        flags: VkFlags,
        p_application_info: *const VkApplicationInfo,
        enabled_layer_count: u32,
        pp_enabled_layer_names: *const *const c_char,
        enabled_extension_count: u32,
        pp_enabled_extension_names: *const *const c_char,
    }

    /// `VkPhysicalDeviceLimits`
    struct VkPhysicalDeviceLimits {
        max_image_dimension_1d: u32,
        max_image_dimension_2d: u32,
        max_image_dimension_3d: u32,
        max_image_dimension_cube: u32,
        max_image_array_layers: u32,
        max_texel_buffer_elements: u32,
        max_uniform_buffer_range: u32,
        max_storage_buffer_range: u32,
        max_push_constants_size: u32,
        max_memory_allocation_count: u32,
        max_sampler_allocation_count: u32,
        buffer_image_granularity: VkDeviceSize,
        sparse_address_space_size: VkDeviceSize,
        max_bound_descriptor_sets: u32,
        max_per_stage_descriptor_samplers: u32,
        max_per_stage_descriptor_uniform_buffers: u32,
        max_per_stage_descriptor_storage_buffers: u32,
        max_per_stage_descriptor_sampled_images: u32,
        max_per_stage_descriptor_storage_images: u32,
        max_per_stage_descriptor_input_attachments: u32,
        max_per_stage_resources: u32,
        max_descriptor_set_samplers: u32,
        max_descriptor_set_uniform_buffers: u32,
        max_descriptor_set_uniform_buffers_dynamic: u32,
        max_descriptor_set_storage_buffers: u32,
        max_descriptor_set_storage_buffers_dynamic: u32,
        max_descriptor_set_sampled_images: u32,
        max_descriptor_set_storage_images: u32,
        max_descriptor_set_input_attachments: u32,
        max_vertex_input_attributes: u32,
        max_vertex_input_bindings: u32,
        max_vertex_input_attribute_offset: u32,
        max_vertex_input_binding_stride: u32,
        max_vertex_output_components: u32,
        max_tessellation_generation_level: u32,
        max_tessellation_patch_size: u32,
        max_tessellation_control_per_vertex_input_components: u32,
        max_tessellation_control_per_vertex_output_components: u32,
        max_tessellation_control_per_patch_output_components: u32,
        max_tessellation_control_total_output_components: u32,
        max_tessellation_evaluation_input_components: u32,
        max_tessellation_evaluation_output_components: u32,
        max_geometry_shader_invocations: u32,
        max_geometry_input_components: u32,
        max_geometry_output_components: u32,
        max_geometry_output_vertices: u32,
        max_geometry_total_output_components: u32,
        max_fragment_input_components: u32,
        max_fragment_output_attachments: u32,
        max_fragment_dual_src_attachments: u32,
        max_fragment_combined_output_resources: u32,
        max_compute_shared_memory_size: u32,
        max_compute_work_group_count: [u32; 3],
        max_compute_work_group_invocations: u32,
        max_compute_work_group_size: [u32; 3],
        sub_pixel_precision_bits: u32,
        sub_texel_precision_bits: u32,
        mipmap_precision_bits: u32,
        max_draw_indexed_index_value: u32,
        max_draw_indirect_count: u32,
        max_sampler_lod_bias: f32,
        max_sampler_anisotropy: f32,
        max_viewports: u32,
        max_viewport_dimensions: [u32; 2],
        viewport_bounds_range: [f32; 2],
        viewport_sub_pixel_bits: u32,
        min_memory_map_alignment: usize,
        min_texel_buffer_offset_alignment: VkDeviceSize,
        min_uniform_buffer_offset_alignment: VkDeviceSize,
        min_storage_buffer_offset_alignment: VkDeviceSize,
        min_texel_offset: i32,
        max_texel_offset: u32,
        min_texel_gather_offset: i32,
        max_texel_gather_offset: u32,
        min_interpolation_offset: f32,
        max_interpolation_offset: f32,
        sub_pixel_interpolation_offset_bits: u32,
        max_framebuffer_width: u32,
        max_framebuffer_height: u32,
        max_framebuffer_layers: u32,
        framebuffer_color_sample_counts: VkFlags,
        framebuffer_depth_sample_counts: VkFlags,
        framebuffer_stencil_sample_counts: VkFlags,
        framebuffer_no_attachments_sample_counts: VkFlags,
        max_color_attachments: u32,
        sampled_image_color_sample_counts: VkFlags,
        sampled_image_integer_sample_counts: VkFlags,
        sampled_image_depth_sample_counts: VkFlags,
        sampled_image_stencil_sample_counts: VkFlags,
        storage_image_sample_counts: VkFlags,
        max_sample_mask_words: u32,
        timestamp_compute_and_graphics: VkBool32,
        timestamp_period: f32,
        max_clip_distances: u32,
        max_cull_distances: u32,
        max_combined_clip_and_cull_distances: u32,
        discrete_queue_priorities: u32,
        point_size_range: [f32; 2],
        line_width_range: [f32; 2],
        point_size_granularity: f32,
        line_width_granularity: f32,
        strict_lines: VkBool32,
        standard_sample_locations: VkBool32,
        optimal_buffer_copy_offset_alignment: VkDeviceSize,
        optimal_buffer_copy_row_pitch_alignment: VkDeviceSize,
        non_coherent_atom_size: VkDeviceSize,
    }
    /// `VkPhysicalDeviceSparseProperties`
    struct VkPhysicalDeviceSparseProperties {
        residency_standard_2d_block_shape: VkBool32,
        residency_standard_2d_multisample_block_shape: VkBool32,
        residency_standard_3d_block_shape: VkBool32,
        residency_aligned_mip_size: VkBool32,
        residency_non_resident_strict: VkBool32,
    }
    /// `VkPhysicalDeviceProperties`
    struct VkPhysicalDeviceProperties {
        api_version: u32,
        driver_version: u32,
        vendor_id: u32,
        device_id: u32,
        device_type: i32,
        device_name: [c_char; VK_MAX_PHYSICAL_DEVICE_NAME_SIZE],
        pipeline_cache_uuid: [u8; VK_UUID_SIZE],
        limits: VkPhysicalDeviceLimits,
        sparse_properties: VkPhysicalDeviceSparseProperties,
    }
    /// `VkMemoryType`
    struct VkMemoryType { property_flags: VkMemoryPropertyFlags, heap_index: u32, }
    /// `VkMemoryHeap`
    struct VkMemoryHeap { size: VkDeviceSize, flags: VkFlags, }
    /// `VkPhysicalDeviceMemoryProperties`
    struct VkPhysicalDeviceMemoryProperties {
        memory_type_count: u32,
        memory_types: [VkMemoryType; VK_MAX_MEMORY_TYPES],
        memory_heap_count: u32,
        memory_heaps: [VkMemoryHeap; VK_MAX_MEMORY_HEAPS],
    }
    /// `VkPhysicalDeviceFeatures` (55 `VkBool32`s, which the renderer
    /// queries and doesn't look at)
    struct VkPhysicalDeviceFeatures { features: [VkBool32; 55], }
    /// `VkQueueFamilyProperties`
    struct VkQueueFamilyProperties {
        queue_flags: VkFlags,
        queue_count: u32,
        timestamp_valid_bits: u32,
        min_image_transfer_granularity: VkExtent3D,
    }
    /// `VkImageFormatProperties`
    struct VkImageFormatProperties {
        max_extent: VkExtent3D,
        max_mip_levels: u32,
        max_array_layers: u32,
        sample_counts: VkFlags,
        max_resource_size: VkDeviceSize,
    }

    /// `VkDeviceQueueCreateInfo`
    struct VkDeviceQueueCreateInfo {
        s_type: VkStructureType,
        p_next: *const c_void,
        flags: VkFlags,
        queue_family_index: u32,
        queue_count: u32,
        p_queue_priorities: *const f32,
    }
    /// `VkDeviceCreateInfo`
    struct VkDeviceCreateInfo {
        s_type: VkStructureType,
        p_next: *const c_void,
        flags: VkFlags,
        queue_create_info_count: u32,
        p_queue_create_infos: *const VkDeviceQueueCreateInfo,
        enabled_layer_count: u32,
        pp_enabled_layer_names: *const *const c_char,
        enabled_extension_count: u32,
        pp_enabled_extension_names: *const *const c_char,
        p_enabled_features: *const VkPhysicalDeviceFeatures,
    }
    /// `VkPhysicalDeviceSamplerYcbcrConversionFeatures`
    struct VkPhysicalDeviceSamplerYcbcrConversionFeatures {
        s_type: VkStructureType,
        p_next: *mut c_void,
        sampler_ycbcr_conversion: VkBool32,
    }

    /// `VkSubmitInfo`
    struct VkSubmitInfo {
        s_type: VkStructureType,
        p_next: *const c_void,
        wait_semaphore_count: u32,
        p_wait_semaphores: *const VkSemaphore,
        p_wait_dst_stage_mask: *const VkPipelineStageFlags,
        command_buffer_count: u32,
        p_command_buffers: *const VkCommandBuffer,
        signal_semaphore_count: u32,
        p_signal_semaphores: *const VkSemaphore,
    }
    /// `VkPresentInfoKHR`
    struct VkPresentInfoKHR {
        s_type: VkStructureType,
        p_next: *const c_void,
        wait_semaphore_count: u32,
        p_wait_semaphores: *const VkSemaphore,
        swapchain_count: u32,
        p_swapchains: *const VkSwapchainKHR,
        p_image_indices: *const u32,
        p_results: *mut VkResult,
    }
    /// `VkFenceCreateInfo`
    struct VkFenceCreateInfo { s_type: VkStructureType, p_next: *const c_void, flags: VkFlags, }
    /// `VkSemaphoreCreateInfo`
    struct VkSemaphoreCreateInfo {
        s_type: VkStructureType,
        p_next: *const c_void,
        flags: VkFlags,
    }

    /// `VkMemoryRequirements`
    struct VkMemoryRequirements {
        size: VkDeviceSize,
        alignment: VkDeviceSize,
        memory_type_bits: u32,
    }
    /// `VkMemoryAllocateInfo`
    struct VkMemoryAllocateInfo {
        s_type: VkStructureType,
        p_next: *const c_void,
        allocation_size: VkDeviceSize,
        memory_type_index: u32,
    }
    /// `VkBufferCreateInfo`
    struct VkBufferCreateInfo {
        s_type: VkStructureType,
        p_next: *const c_void,
        flags: VkFlags,
        size: VkDeviceSize,
        usage: VkBufferUsageFlags,
        sharing_mode: i32,
        queue_family_index_count: u32,
        p_queue_family_indices: *const u32,
    }
    /// `VkImageCreateInfo`
    struct VkImageCreateInfo {
        s_type: VkStructureType,
        p_next: *const c_void,
        flags: VkFlags,
        image_type: i32,
        format: VkFormat,
        extent: VkExtent3D,
        mip_levels: u32,
        array_layers: u32,
        samples: VkFlags,
        tiling: i32,
        usage: VkImageUsageFlags,
        sharing_mode: i32,
        queue_family_index_count: u32,
        p_queue_family_indices: *const u32,
        initial_layout: VkImageLayout,
    }
    /// `VkComponentMapping`
    struct VkComponentMapping { r: i32, g: i32, b: i32, a: i32, }
    /// `VkImageSubresourceRange`
    struct VkImageSubresourceRange {
        aspect_mask: VkImageAspectFlags,
        base_mip_level: u32,
        level_count: u32,
        base_array_layer: u32,
        layer_count: u32,
    }
    /// `VkImageSubresourceLayers`
    struct VkImageSubresourceLayers {
        aspect_mask: VkImageAspectFlags,
        mip_level: u32,
        base_array_layer: u32,
        layer_count: u32,
    }
    /// `VkImageViewCreateInfo`
    struct VkImageViewCreateInfo {
        s_type: VkStructureType,
        p_next: *const c_void,
        flags: VkFlags,
        image: VkImage,
        view_type: i32,
        format: VkFormat,
        components: VkComponentMapping,
        subresource_range: VkImageSubresourceRange,
    }
    /// `VkImageViewUsageCreateInfo`
    struct VkImageViewUsageCreateInfo {
        s_type: VkStructureType,
        p_next: *const c_void,
        usage: VkImageUsageFlags,
    }
    /// `VkImageMemoryBarrier`
    struct VkImageMemoryBarrier {
        s_type: VkStructureType,
        p_next: *const c_void,
        src_access_mask: VkAccessFlags,
        dst_access_mask: VkAccessFlags,
        old_layout: VkImageLayout,
        new_layout: VkImageLayout,
        src_queue_family_index: u32,
        dst_queue_family_index: u32,
        image: VkImage,
        subresource_range: VkImageSubresourceRange,
    }
    /// `VkBufferImageCopy`
    struct VkBufferImageCopy {
        buffer_offset: VkDeviceSize,
        buffer_row_length: u32,
        buffer_image_height: u32,
        image_subresource: VkImageSubresourceLayers,
        image_offset: VkOffset3D,
        image_extent: VkExtent3D,
    }

    /// `VkShaderModuleCreateInfo`
    struct VkShaderModuleCreateInfo {
        s_type: VkStructureType,
        p_next: *const c_void,
        flags: VkFlags,
        code_size: usize,
        p_code: *const u32,
    }
    /// `VkPipelineShaderStageCreateInfo`
    struct VkPipelineShaderStageCreateInfo {
        s_type: VkStructureType,
        p_next: *const c_void,
        flags: VkFlags,
        stage: VkFlags,
        module: VkShaderModule,
        p_name: *const c_char,
        p_specialization_info: *const c_void,
    }
    /// `VkVertexInputBindingDescription`
    struct VkVertexInputBindingDescription { binding: u32, stride: u32, input_rate: i32, }
    /// `VkVertexInputAttributeDescription`
    struct VkVertexInputAttributeDescription {
        location: u32,
        binding: u32,
        format: VkFormat,
        offset: u32,
    }
    /// `VkPipelineVertexInputStateCreateInfo`
    struct VkPipelineVertexInputStateCreateInfo {
        s_type: VkStructureType,
        p_next: *const c_void,
        flags: VkFlags,
        vertex_binding_description_count: u32,
        p_vertex_binding_descriptions: *const VkVertexInputBindingDescription,
        vertex_attribute_description_count: u32,
        p_vertex_attribute_descriptions: *const VkVertexInputAttributeDescription,
    }
    /// `VkPipelineInputAssemblyStateCreateInfo`
    struct VkPipelineInputAssemblyStateCreateInfo {
        s_type: VkStructureType,
        p_next: *const c_void,
        flags: VkFlags,
        topology: VkPrimitiveTopology,
        primitive_restart_enable: VkBool32,
    }
    /// `VkPipelineViewportStateCreateInfo`
    struct VkPipelineViewportStateCreateInfo {
        s_type: VkStructureType,
        p_next: *const c_void,
        flags: VkFlags,
        viewport_count: u32,
        p_viewports: *const VkViewport,
        scissor_count: u32,
        p_scissors: *const VkRect2D,
    }
    /// `VkPipelineRasterizationStateCreateInfo`
    struct VkPipelineRasterizationStateCreateInfo {
        s_type: VkStructureType,
        p_next: *const c_void,
        flags: VkFlags,
        depth_clamp_enable: VkBool32,
        rasterizer_discard_enable: VkBool32,
        polygon_mode: i32,
        cull_mode: VkFlags,
        front_face: i32,
        depth_bias_enable: VkBool32,
        depth_bias_constant_factor: f32,
        depth_bias_clamp: f32,
        depth_bias_slope_factor: f32,
        line_width: f32,
    }
    /// `VkPipelineMultisampleStateCreateInfo`
    struct VkPipelineMultisampleStateCreateInfo {
        s_type: VkStructureType,
        p_next: *const c_void,
        flags: VkFlags,
        rasterization_samples: VkFlags,
        sample_shading_enable: VkBool32,
        min_sample_shading: f32,
        p_sample_mask: *const u32,
        alpha_to_coverage_enable: VkBool32,
        alpha_to_one_enable: VkBool32,
    }
    /// `VkStencilOpState`
    struct VkStencilOpState {
        fail_op: i32,
        pass_op: i32,
        depth_fail_op: i32,
        compare_op: i32,
        compare_mask: u32,
        write_mask: u32,
        reference: u32,
    }
    /// `VkPipelineDepthStencilStateCreateInfo`
    struct VkPipelineDepthStencilStateCreateInfo {
        s_type: VkStructureType,
        p_next: *const c_void,
        flags: VkFlags,
        depth_test_enable: VkBool32,
        depth_write_enable: VkBool32,
        depth_compare_op: i32,
        depth_bounds_test_enable: VkBool32,
        stencil_test_enable: VkBool32,
        front: VkStencilOpState,
        back: VkStencilOpState,
        min_depth_bounds: f32,
        max_depth_bounds: f32,
    }
    /// `VkPipelineColorBlendAttachmentState`
    struct VkPipelineColorBlendAttachmentState {
        blend_enable: VkBool32,
        src_color_blend_factor: VkBlendFactor,
        dst_color_blend_factor: VkBlendFactor,
        color_blend_op: VkBlendOp,
        src_alpha_blend_factor: VkBlendFactor,
        dst_alpha_blend_factor: VkBlendFactor,
        alpha_blend_op: VkBlendOp,
        color_write_mask: VkFlags,
    }
    /// `VkPipelineColorBlendStateCreateInfo`
    struct VkPipelineColorBlendStateCreateInfo {
        s_type: VkStructureType,
        p_next: *const c_void,
        flags: VkFlags,
        logic_op_enable: VkBool32,
        logic_op: i32,
        attachment_count: u32,
        p_attachments: *const VkPipelineColorBlendAttachmentState,
        blend_constants: [f32; 4],
    }
    /// `VkPipelineDynamicStateCreateInfo`
    struct VkPipelineDynamicStateCreateInfo {
        s_type: VkStructureType,
        p_next: *const c_void,
        flags: VkFlags,
        dynamic_state_count: u32,
        p_dynamic_states: *const i32,
    }
    /// `VkGraphicsPipelineCreateInfo`
    struct VkGraphicsPipelineCreateInfo {
        s_type: VkStructureType,
        p_next: *const c_void,
        flags: VkFlags,
        stage_count: u32,
        p_stages: *const VkPipelineShaderStageCreateInfo,
        p_vertex_input_state: *const VkPipelineVertexInputStateCreateInfo,
        p_input_assembly_state: *const VkPipelineInputAssemblyStateCreateInfo,
        p_tessellation_state: *const c_void,
        p_viewport_state: *const VkPipelineViewportStateCreateInfo,
        p_rasterization_state: *const VkPipelineRasterizationStateCreateInfo,
        p_multisample_state: *const VkPipelineMultisampleStateCreateInfo,
        p_depth_stencil_state: *const VkPipelineDepthStencilStateCreateInfo,
        p_color_blend_state: *const VkPipelineColorBlendStateCreateInfo,
        p_dynamic_state: *const VkPipelineDynamicStateCreateInfo,
        layout: VkPipelineLayout,
        render_pass: VkRenderPass,
        subpass: u32,
        base_pipeline_handle: VkPipeline,
        base_pipeline_index: i32,
    }
    /// `VkPushConstantRange`
    struct VkPushConstantRange { stage_flags: VkFlags, offset: u32, size: u32, }
    /// `VkPipelineLayoutCreateInfo`
    struct VkPipelineLayoutCreateInfo {
        s_type: VkStructureType,
        p_next: *const c_void,
        flags: VkFlags,
        set_layout_count: u32,
        p_set_layouts: *const VkDescriptorSetLayout,
        push_constant_range_count: u32,
        p_push_constant_ranges: *const VkPushConstantRange,
    }

    /// `VkSamplerCreateInfo`
    struct VkSamplerCreateInfo {
        s_type: VkStructureType,
        p_next: *const c_void,
        flags: VkFlags,
        mag_filter: i32,
        min_filter: i32,
        mipmap_mode: i32,
        address_mode_u: i32,
        address_mode_v: i32,
        address_mode_w: i32,
        mip_lod_bias: f32,
        anisotropy_enable: VkBool32,
        max_anisotropy: f32,
        compare_enable: VkBool32,
        compare_op: i32,
        min_lod: f32,
        max_lod: f32,
        border_color: i32,
        unnormalized_coordinates: VkBool32,
    }
    /// `VkDescriptorSetLayoutBinding`
    struct VkDescriptorSetLayoutBinding {
        binding: u32,
        descriptor_type: i32,
        descriptor_count: u32,
        stage_flags: VkFlags,
        p_immutable_samplers: *const VkSampler,
    }
    /// `VkDescriptorSetLayoutCreateInfo`
    struct VkDescriptorSetLayoutCreateInfo {
        s_type: VkStructureType,
        p_next: *const c_void,
        flags: VkFlags,
        binding_count: u32,
        p_bindings: *const VkDescriptorSetLayoutBinding,
    }
    /// `VkDescriptorPoolSize`
    struct VkDescriptorPoolSize { ty: i32, descriptor_count: u32, }
    /// `VkDescriptorPoolCreateInfo`
    struct VkDescriptorPoolCreateInfo {
        s_type: VkStructureType,
        p_next: *const c_void,
        flags: VkFlags,
        max_sets: u32,
        pool_size_count: u32,
        p_pool_sizes: *const VkDescriptorPoolSize,
    }
    /// `VkDescriptorSetAllocateInfo`
    struct VkDescriptorSetAllocateInfo {
        s_type: VkStructureType,
        p_next: *const c_void,
        descriptor_pool: VkDescriptorPool,
        descriptor_set_count: u32,
        p_set_layouts: *const VkDescriptorSetLayout,
    }
    /// `VkDescriptorImageInfo`
    struct VkDescriptorImageInfo {
        sampler: VkSampler,
        image_view: VkImageView,
        image_layout: VkImageLayout,
    }
    /// `VkDescriptorBufferInfo`
    struct VkDescriptorBufferInfo {
        buffer: VkBuffer,
        offset: VkDeviceSize,
        range: VkDeviceSize,
    }
    /// `VkWriteDescriptorSet`
    struct VkWriteDescriptorSet {
        s_type: VkStructureType,
        p_next: *const c_void,
        dst_set: VkDescriptorSet,
        dst_binding: u32,
        dst_array_element: u32,
        descriptor_count: u32,
        descriptor_type: i32,
        p_image_info: *const VkDescriptorImageInfo,
        p_buffer_info: *const VkDescriptorBufferInfo,
        p_texel_buffer_view: *const u64,
    }

    /// `VkAttachmentDescription`
    struct VkAttachmentDescription {
        flags: VkFlags,
        format: VkFormat,
        samples: VkFlags,
        load_op: VkAttachmentLoadOp,
        store_op: i32,
        stencil_load_op: VkAttachmentLoadOp,
        stencil_store_op: i32,
        initial_layout: VkImageLayout,
        final_layout: VkImageLayout,
    }
    /// `VkAttachmentReference`
    struct VkAttachmentReference { attachment: u32, layout: VkImageLayout, }
    /// `VkSubpassDescription`
    struct VkSubpassDescription {
        flags: VkFlags,
        pipeline_bind_point: i32,
        input_attachment_count: u32,
        p_input_attachments: *const VkAttachmentReference,
        color_attachment_count: u32,
        p_color_attachments: *const VkAttachmentReference,
        p_resolve_attachments: *const VkAttachmentReference,
        p_depth_stencil_attachment: *const VkAttachmentReference,
        preserve_attachment_count: u32,
        p_preserve_attachments: *const u32,
    }
    /// `VkSubpassDependency`
    struct VkSubpassDependency {
        src_subpass: u32,
        dst_subpass: u32,
        src_stage_mask: VkPipelineStageFlags,
        dst_stage_mask: VkPipelineStageFlags,
        src_access_mask: VkAccessFlags,
        dst_access_mask: VkAccessFlags,
        dependency_flags: VkFlags,
    }
    /// `VkRenderPassCreateInfo`
    struct VkRenderPassCreateInfo {
        s_type: VkStructureType,
        p_next: *const c_void,
        flags: VkFlags,
        attachment_count: u32,
        p_attachments: *const VkAttachmentDescription,
        subpass_count: u32,
        p_subpasses: *const VkSubpassDescription,
        dependency_count: u32,
        p_dependencies: *const VkSubpassDependency,
    }
    /// `VkFramebufferCreateInfo`
    struct VkFramebufferCreateInfo {
        s_type: VkStructureType,
        p_next: *const c_void,
        flags: VkFlags,
        render_pass: VkRenderPass,
        attachment_count: u32,
        p_attachments: *const VkImageView,
        width: u32,
        height: u32,
        layers: u32,
    }
    /// `VkRenderPassBeginInfo`
    struct VkRenderPassBeginInfo {
        s_type: VkStructureType,
        p_next: *const c_void,
        render_pass: VkRenderPass,
        framebuffer: VkFramebuffer,
        render_area: VkRect2D,
        clear_value_count: u32,
        p_clear_values: *const VkClearValue,
    }

    /// `VkCommandPoolCreateInfo`
    struct VkCommandPoolCreateInfo {
        s_type: VkStructureType,
        p_next: *const c_void,
        flags: VkFlags,
        queue_family_index: u32,
    }
    /// `VkCommandBufferAllocateInfo`
    struct VkCommandBufferAllocateInfo {
        s_type: VkStructureType,
        p_next: *const c_void,
        command_pool: VkCommandPool,
        level: i32,
        command_buffer_count: u32,
    }
    /// `VkCommandBufferBeginInfo`
    struct VkCommandBufferBeginInfo {
        s_type: VkStructureType,
        p_next: *const c_void,
        flags: VkFlags,
        p_inheritance_info: *const c_void,
    }

    /// `VkSurfaceCapabilitiesKHR`
    struct VkSurfaceCapabilitiesKHR {
        min_image_count: u32,
        max_image_count: u32,
        current_extent: VkExtent2D,
        min_image_extent: VkExtent2D,
        max_image_extent: VkExtent2D,
        max_image_array_layers: u32,
        supported_transforms: VkFlags,
        current_transform: VkSurfaceTransformFlagBitsKHR,
        supported_composite_alpha: VkFlags,
        supported_usage_flags: VkImageUsageFlags,
    }
    /// `VkSurfaceFormatKHR`
    struct VkSurfaceFormatKHR { format: VkFormat, color_space: VkColorSpaceKHR, }
    /// `VkSwapchainCreateInfoKHR`
    struct VkSwapchainCreateInfoKHR {
        s_type: VkStructureType,
        p_next: *const c_void,
        flags: VkFlags,
        surface: VkSurfaceKHR,
        min_image_count: u32,
        image_format: VkFormat,
        image_color_space: VkColorSpaceKHR,
        image_extent: VkExtent2D,
        image_array_layers: u32,
        image_usage: VkImageUsageFlags,
        image_sharing_mode: i32,
        queue_family_index_count: u32,
        p_queue_family_indices: *const u32,
        pre_transform: VkSurfaceTransformFlagBitsKHR,
        composite_alpha: VkFlags,
        present_mode: VkPresentModeKHR,
        clipped: VkBool32,
        old_swapchain: VkSwapchainKHR,
    }
}

impl std::fmt::Debug for VkPhysicalDeviceProperties {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("VkPhysicalDeviceProperties")
            .field("api_version", &self.api_version)
            .finish_non_exhaustive()
    }
}

/// The size of each structure on 64-bit targets, as the C compiler lays
/// out vulkan_core.h's (checked with `sizeof` there).
#[cfg(target_pointer_width = "64")]
const _: () = {
    assert!(size_of::<VkLayerProperties>() == 520);
    assert!(size_of::<VkApplicationInfo>() == 48);
    assert!(size_of::<VkInstanceCreateInfo>() == 64);
    assert!(size_of::<VkPhysicalDeviceLimits>() == 504);
    assert!(size_of::<VkPhysicalDeviceProperties>() == 824);
    assert!(size_of::<VkPhysicalDeviceMemoryProperties>() == 520);
    assert!(size_of::<VkPhysicalDeviceFeatures>() == 220);
    assert!(size_of::<VkQueueFamilyProperties>() == 24);
    assert!(size_of::<VkImageFormatProperties>() == 32);
    assert!(size_of::<VkDeviceQueueCreateInfo>() == 40);
    assert!(size_of::<VkDeviceCreateInfo>() == 72);
    assert!(size_of::<VkPhysicalDeviceSamplerYcbcrConversionFeatures>() == 24);
    assert!(size_of::<VkSubmitInfo>() == 72);
    assert!(size_of::<VkPresentInfoKHR>() == 64);
    assert!(size_of::<VkFenceCreateInfo>() == 24);
    assert!(size_of::<VkSemaphoreCreateInfo>() == 24);
    assert!(size_of::<VkMemoryRequirements>() == 24);
    assert!(size_of::<VkMemoryAllocateInfo>() == 32);
    assert!(size_of::<VkBufferCreateInfo>() == 56);
    assert!(size_of::<VkImageCreateInfo>() == 88);
    assert!(size_of::<VkImageViewCreateInfo>() == 80);
    assert!(size_of::<VkImageViewUsageCreateInfo>() == 24);
    assert!(size_of::<VkImageMemoryBarrier>() == 72);
    assert!(size_of::<VkBufferImageCopy>() == 56);
    assert!(size_of::<VkShaderModuleCreateInfo>() == 40);
    assert!(size_of::<VkPipelineShaderStageCreateInfo>() == 48);
    assert!(size_of::<VkPipelineVertexInputStateCreateInfo>() == 48);
    assert!(size_of::<VkPipelineInputAssemblyStateCreateInfo>() == 32);
    assert!(size_of::<VkPipelineViewportStateCreateInfo>() == 48);
    assert!(size_of::<VkPipelineRasterizationStateCreateInfo>() == 64);
    assert!(size_of::<VkPipelineMultisampleStateCreateInfo>() == 48);
    assert!(size_of::<VkPipelineDepthStencilStateCreateInfo>() == 104);
    assert!(size_of::<VkPipelineColorBlendAttachmentState>() == 32);
    assert!(size_of::<VkPipelineColorBlendStateCreateInfo>() == 56);
    assert!(size_of::<VkPipelineDynamicStateCreateInfo>() == 32);
    assert!(size_of::<VkGraphicsPipelineCreateInfo>() == 144);
    assert!(size_of::<VkPipelineLayoutCreateInfo>() == 48);
    assert!(size_of::<VkSamplerCreateInfo>() == 80);
    assert!(size_of::<VkDescriptorSetLayoutBinding>() == 24);
    assert!(size_of::<VkDescriptorSetLayoutCreateInfo>() == 32);
    assert!(size_of::<VkDescriptorPoolCreateInfo>() == 40);
    assert!(size_of::<VkDescriptorSetAllocateInfo>() == 40);
    assert!(size_of::<VkDescriptorImageInfo>() == 24);
    assert!(size_of::<VkDescriptorBufferInfo>() == 24);
    assert!(size_of::<VkWriteDescriptorSet>() == 64);
    assert!(size_of::<VkAttachmentDescription>() == 36);
    assert!(size_of::<VkSubpassDescription>() == 72);
    assert!(size_of::<VkSubpassDependency>() == 28);
    assert!(size_of::<VkRenderPassCreateInfo>() == 64);
    assert!(size_of::<VkFramebufferCreateInfo>() == 64);
    assert!(size_of::<VkRenderPassBeginInfo>() == 64);
    assert!(size_of::<VkCommandPoolCreateInfo>() == 24);
    assert!(size_of::<VkCommandBufferAllocateInfo>() == 32);
    assert!(size_of::<VkCommandBufferBeginInfo>() == 32);
    assert!(size_of::<VkSurfaceCapabilitiesKHR>() == 52);
    assert!(size_of::<VkSwapchainCreateInfoKHR>() == 104);
    assert!(std::mem::offset_of!(VkPhysicalDeviceProperties, limits) == 296);
    assert!(
        std::mem::offset_of!(VkPhysicalDeviceLimits, min_uniform_buffer_offset_alignment) == 320
    );
};

/// The function tables: a structure of function pointers for each kind of
/// function in `VULKAN_FUNCTIONS()`, looked up all at once.
macro_rules! vulkan_functions {
    ($(
        $(#[$m:meta])*
        struct $table:ident {
            $(
                $(#[$fm:meta])*
                $field:ident = $sym:literal: fn($($arg:ty),* $(,)?) $(-> $ret:ty)?;
            )*
        }
    )*) => {$(
        $(#[$m])*
        pub(super) struct $table {
            $(
                $(#[$fm])*
                pub(super) $field: unsafe extern "system" fn($($arg),*) $(-> $ret)?,
            )*
        }

        impl $table {
            /// Look up every function with `lookup`; the name of the first
            /// one that isn't there as the error.
            ///
            /// # Safety
            ///
            /// `lookup` returns the functions of the names it's given.
            pub(super) unsafe fn load(
                mut lookup: impl FnMut(&CStr) -> Option<PfnVkVoidFunction>,
            ) -> std::result::Result<$table, &'static str> {
                Ok($table {
                    $($field: {
                        let f = lookup($sym).ok_or($sym.to_str().unwrap_or(""))?;
                        // SAFETY: the caller's contract: the function of
                        // that name has this type.
                        unsafe {
                            std::mem::transmute::<
                                PfnVkVoidFunction,
                                unsafe extern "system" fn($($arg),*) $(-> $ret)?,
                            >(f)
                        }
                    },)*
                })
            }
        }
    )*};
}

/// `const VkAllocationCallbacks *` (always null here).
type Alloc = *const c_void;

vulkan_functions! {
    /// The `VULKAN_GLOBAL_FUNCTION`s.
    struct GlobalFunctions {
        create_instance = c"vkCreateInstance":
            fn(*const VkInstanceCreateInfo, Alloc, *mut VkInstance) -> VkResult;
        enumerate_instance_extension_properties = c"vkEnumerateInstanceExtensionProperties":
            fn(*const c_char, *mut u32, *mut VkExtensionProperties) -> VkResult;
        enumerate_instance_layer_properties = c"vkEnumerateInstanceLayerProperties":
            fn(*mut u32, *mut VkLayerProperties) -> VkResult;
    }

    /// The `VULKAN_INSTANCE_FUNCTION`s. (The optional
    /// `vkGetPhysicalDevice*2KHR` are looked up but never called upstream,
    /// so they aren't here.)
    struct InstanceFunctions {
        create_device = c"vkCreateDevice":
            fn(VkPhysicalDevice, *const VkDeviceCreateInfo, Alloc, *mut VkDevice) -> VkResult;
        destroy_instance = c"vkDestroyInstance": fn(VkInstance, Alloc);
        destroy_surface_khr = c"vkDestroySurfaceKHR": fn(VkInstance, VkSurfaceKHR, Alloc);
        enumerate_device_extension_properties = c"vkEnumerateDeviceExtensionProperties":
            fn(VkPhysicalDevice, *const c_char, *mut u32, *mut VkExtensionProperties) -> VkResult;
        enumerate_physical_devices = c"vkEnumeratePhysicalDevices":
            fn(VkInstance, *mut u32, *mut VkPhysicalDevice) -> VkResult;
        get_device_proc_addr = c"vkGetDeviceProcAddr":
            fn(VkDevice, *const c_char) -> Option<PfnVkVoidFunction>;
        get_physical_device_features = c"vkGetPhysicalDeviceFeatures":
            fn(VkPhysicalDevice, *mut VkPhysicalDeviceFeatures);
        get_physical_device_image_format_properties =
            c"vkGetPhysicalDeviceImageFormatProperties":
            fn(
                VkPhysicalDevice,
                VkFormat,
                i32,
                i32,
                VkImageUsageFlags,
                VkFlags,
                *mut VkImageFormatProperties,
            ) -> VkResult;
        get_physical_device_properties = c"vkGetPhysicalDeviceProperties":
            fn(VkPhysicalDevice, *mut VkPhysicalDeviceProperties);
        get_physical_device_memory_properties = c"vkGetPhysicalDeviceMemoryProperties":
            fn(VkPhysicalDevice, *mut VkPhysicalDeviceMemoryProperties);
        get_physical_device_queue_family_properties =
            c"vkGetPhysicalDeviceQueueFamilyProperties":
            fn(VkPhysicalDevice, *mut u32, *mut VkQueueFamilyProperties);
        get_physical_device_surface_capabilities_khr =
            c"vkGetPhysicalDeviceSurfaceCapabilitiesKHR":
            fn(VkPhysicalDevice, VkSurfaceKHR, *mut VkSurfaceCapabilitiesKHR) -> VkResult;
        get_physical_device_surface_formats_khr = c"vkGetPhysicalDeviceSurfaceFormatsKHR":
            fn(VkPhysicalDevice, VkSurfaceKHR, *mut u32, *mut VkSurfaceFormatKHR) -> VkResult;
        get_physical_device_surface_present_modes_khr =
            c"vkGetPhysicalDeviceSurfacePresentModesKHR":
            fn(VkPhysicalDevice, VkSurfaceKHR, *mut u32, *mut VkPresentModeKHR) -> VkResult;
        get_physical_device_surface_support_khr = c"vkGetPhysicalDeviceSurfaceSupportKHR":
            fn(VkPhysicalDevice, u32, VkSurfaceKHR, *mut VkBool32) -> VkResult;
        queue_wait_idle = c"vkQueueWaitIdle": fn(VkQueue) -> VkResult;
    }

    /// The `VULKAN_DEVICE_FUNCTION`s. (The optional sampler YCbCr
    /// conversion functions are only used by the Android parts.)
    struct DeviceFunctions {
        acquire_next_image_khr = c"vkAcquireNextImageKHR":
            fn(VkDevice, VkSwapchainKHR, u64, VkSemaphore, VkFence, *mut u32) -> VkResult;
        allocate_command_buffers = c"vkAllocateCommandBuffers":
            fn(VkDevice, *const VkCommandBufferAllocateInfo, *mut VkCommandBuffer) -> VkResult;
        allocate_descriptor_sets = c"vkAllocateDescriptorSets":
            fn(VkDevice, *const VkDescriptorSetAllocateInfo, *mut VkDescriptorSet) -> VkResult;
        allocate_memory = c"vkAllocateMemory":
            fn(VkDevice, *const VkMemoryAllocateInfo, Alloc, *mut VkDeviceMemory) -> VkResult;
        begin_command_buffer = c"vkBeginCommandBuffer":
            fn(VkCommandBuffer, *const VkCommandBufferBeginInfo) -> VkResult;
        bind_buffer_memory = c"vkBindBufferMemory":
            fn(VkDevice, VkBuffer, VkDeviceMemory, VkDeviceSize) -> VkResult;
        bind_image_memory = c"vkBindImageMemory":
            fn(VkDevice, VkImage, VkDeviceMemory, VkDeviceSize) -> VkResult;
        cmd_begin_render_pass = c"vkCmdBeginRenderPass":
            fn(VkCommandBuffer, *const VkRenderPassBeginInfo, i32);
        cmd_bind_descriptor_sets = c"vkCmdBindDescriptorSets":
            fn(
                VkCommandBuffer,
                i32,
                VkPipelineLayout,
                u32,
                u32,
                *const VkDescriptorSet,
                u32,
                *const u32,
            );
        cmd_bind_pipeline = c"vkCmdBindPipeline": fn(VkCommandBuffer, i32, VkPipeline);
        cmd_bind_vertex_buffers = c"vkCmdBindVertexBuffers":
            fn(VkCommandBuffer, u32, u32, *const VkBuffer, *const VkDeviceSize);
        #[allow(dead_code)] // (looked up, never called)
        cmd_clear_color_image = c"vkCmdClearColorImage":
            fn(
                VkCommandBuffer,
                VkImage,
                VkImageLayout,
                *const VkClearValue,
                u32,
                *const VkImageSubresourceRange,
            );
        cmd_copy_buffer_to_image = c"vkCmdCopyBufferToImage":
            fn(VkCommandBuffer, VkBuffer, VkImage, VkImageLayout, u32, *const VkBufferImageCopy);
        cmd_copy_image_to_buffer = c"vkCmdCopyImageToBuffer":
            fn(VkCommandBuffer, VkImage, VkImageLayout, VkBuffer, u32, *const VkBufferImageCopy);
        cmd_draw = c"vkCmdDraw": fn(VkCommandBuffer, u32, u32, u32, u32);
        cmd_end_render_pass = c"vkCmdEndRenderPass": fn(VkCommandBuffer);
        cmd_pipeline_barrier = c"vkCmdPipelineBarrier":
            fn(
                VkCommandBuffer,
                VkPipelineStageFlags,
                VkPipelineStageFlags,
                VkFlags,
                u32,
                *const c_void,
                u32,
                *const c_void,
                u32,
                *const VkImageMemoryBarrier,
            );
        cmd_push_constants = c"vkCmdPushConstants":
            fn(VkCommandBuffer, VkPipelineLayout, VkFlags, u32, u32, *const c_void);
        cmd_set_scissor = c"vkCmdSetScissor": fn(VkCommandBuffer, u32, u32, *const VkRect2D);
        cmd_set_viewport = c"vkCmdSetViewport": fn(VkCommandBuffer, u32, u32, *const VkViewport);
        create_buffer = c"vkCreateBuffer":
            fn(VkDevice, *const VkBufferCreateInfo, Alloc, *mut VkBuffer) -> VkResult;
        create_command_pool = c"vkCreateCommandPool":
            fn(VkDevice, *const VkCommandPoolCreateInfo, Alloc, *mut VkCommandPool) -> VkResult;
        create_descriptor_pool = c"vkCreateDescriptorPool":
            fn(
                VkDevice,
                *const VkDescriptorPoolCreateInfo,
                Alloc,
                *mut VkDescriptorPool,
            ) -> VkResult;
        create_descriptor_set_layout = c"vkCreateDescriptorSetLayout":
            fn(
                VkDevice,
                *const VkDescriptorSetLayoutCreateInfo,
                Alloc,
                *mut VkDescriptorSetLayout,
            ) -> VkResult;
        create_fence = c"vkCreateFence":
            fn(VkDevice, *const VkFenceCreateInfo, Alloc, *mut VkFence) -> VkResult;
        create_framebuffer = c"vkCreateFramebuffer":
            fn(VkDevice, *const VkFramebufferCreateInfo, Alloc, *mut VkFramebuffer) -> VkResult;
        create_graphics_pipelines = c"vkCreateGraphicsPipelines":
            fn(
                VkDevice,
                VkPipelineCache,
                u32,
                *const VkGraphicsPipelineCreateInfo,
                Alloc,
                *mut VkPipeline,
            ) -> VkResult;
        create_image = c"vkCreateImage":
            fn(VkDevice, *const VkImageCreateInfo, Alloc, *mut VkImage) -> VkResult;
        create_image_view = c"vkCreateImageView":
            fn(VkDevice, *const VkImageViewCreateInfo, Alloc, *mut VkImageView) -> VkResult;
        create_pipeline_layout = c"vkCreatePipelineLayout":
            fn(
                VkDevice,
                *const VkPipelineLayoutCreateInfo,
                Alloc,
                *mut VkPipelineLayout,
            ) -> VkResult;
        create_render_pass = c"vkCreateRenderPass":
            fn(VkDevice, *const VkRenderPassCreateInfo, Alloc, *mut VkRenderPass) -> VkResult;
        create_sampler = c"vkCreateSampler":
            fn(VkDevice, *const VkSamplerCreateInfo, Alloc, *mut VkSampler) -> VkResult;
        create_semaphore = c"vkCreateSemaphore":
            fn(VkDevice, *const VkSemaphoreCreateInfo, Alloc, *mut VkSemaphore) -> VkResult;
        create_shader_module = c"vkCreateShaderModule":
            fn(VkDevice, *const VkShaderModuleCreateInfo, Alloc, *mut VkShaderModule) -> VkResult;
        create_swapchain_khr = c"vkCreateSwapchainKHR":
            fn(VkDevice, *const VkSwapchainCreateInfoKHR, Alloc, *mut VkSwapchainKHR) -> VkResult;
        destroy_buffer = c"vkDestroyBuffer": fn(VkDevice, VkBuffer, Alloc);
        destroy_command_pool = c"vkDestroyCommandPool": fn(VkDevice, VkCommandPool, Alloc);
        destroy_device = c"vkDestroyDevice": fn(VkDevice, Alloc);
        destroy_descriptor_pool = c"vkDestroyDescriptorPool": fn(VkDevice, VkDescriptorPool, Alloc);
        destroy_descriptor_set_layout = c"vkDestroyDescriptorSetLayout":
            fn(VkDevice, VkDescriptorSetLayout, Alloc);
        destroy_fence = c"vkDestroyFence": fn(VkDevice, VkFence, Alloc);
        destroy_framebuffer = c"vkDestroyFramebuffer": fn(VkDevice, VkFramebuffer, Alloc);
        destroy_image = c"vkDestroyImage": fn(VkDevice, VkImage, Alloc);
        destroy_image_view = c"vkDestroyImageView": fn(VkDevice, VkImageView, Alloc);
        destroy_pipeline = c"vkDestroyPipeline": fn(VkDevice, VkPipeline, Alloc);
        destroy_pipeline_layout = c"vkDestroyPipelineLayout": fn(VkDevice, VkPipelineLayout, Alloc);
        destroy_render_pass = c"vkDestroyRenderPass": fn(VkDevice, VkRenderPass, Alloc);
        destroy_sampler = c"vkDestroySampler": fn(VkDevice, VkSampler, Alloc);
        destroy_semaphore = c"vkDestroySemaphore": fn(VkDevice, VkSemaphore, Alloc);
        destroy_shader_module = c"vkDestroyShaderModule": fn(VkDevice, VkShaderModule, Alloc);
        destroy_swapchain_khr = c"vkDestroySwapchainKHR": fn(VkDevice, VkSwapchainKHR, Alloc);
        device_wait_idle = c"vkDeviceWaitIdle": fn(VkDevice) -> VkResult;
        end_command_buffer = c"vkEndCommandBuffer": fn(VkCommandBuffer) -> VkResult;
        free_command_buffers = c"vkFreeCommandBuffers":
            fn(VkDevice, VkCommandPool, u32, *const VkCommandBuffer);
        free_memory = c"vkFreeMemory": fn(VkDevice, VkDeviceMemory, Alloc);
        get_buffer_memory_requirements = c"vkGetBufferMemoryRequirements":
            fn(VkDevice, VkBuffer, *mut VkMemoryRequirements);
        get_image_memory_requirements = c"vkGetImageMemoryRequirements":
            fn(VkDevice, VkImage, *mut VkMemoryRequirements);
        get_device_queue = c"vkGetDeviceQueue": fn(VkDevice, u32, u32, *mut VkQueue);
        #[allow(dead_code)] // (looked up, never called)
        get_fence_status = c"vkGetFenceStatus": fn(VkDevice, VkFence) -> VkResult;
        get_swapchain_images_khr = c"vkGetSwapchainImagesKHR":
            fn(VkDevice, VkSwapchainKHR, *mut u32, *mut VkImage) -> VkResult;
        map_memory = c"vkMapMemory":
            fn(VkDevice, VkDeviceMemory, VkDeviceSize, VkDeviceSize, VkFlags, *mut *mut c_void)
                -> VkResult;
        queue_present_khr = c"vkQueuePresentKHR":
            fn(VkQueue, *const VkPresentInfoKHR) -> VkResult;
        queue_submit = c"vkQueueSubmit":
            fn(VkQueue, u32, *const VkSubmitInfo, VkFence) -> VkResult;
        reset_command_buffer = c"vkResetCommandBuffer": fn(VkCommandBuffer, VkFlags) -> VkResult;
        #[allow(dead_code)] // (looked up, never called)
        reset_command_pool = c"vkResetCommandPool":
            fn(VkDevice, VkCommandPool, VkFlags) -> VkResult;
        reset_descriptor_pool = c"vkResetDescriptorPool":
            fn(VkDevice, VkDescriptorPool, VkFlags) -> VkResult;
        reset_fences = c"vkResetFences": fn(VkDevice, u32, *const VkFence) -> VkResult;
        #[allow(dead_code)] // (looked up, never called)
        unmap_memory = c"vkUnmapMemory": fn(VkDevice, VkDeviceMemory);
        update_descriptor_sets = c"vkUpdateDescriptorSets":
            fn(VkDevice, u32, *const VkWriteDescriptorSet, u32, *const c_void);
        wait_for_fences = c"vkWaitForFences":
            fn(VkDevice, u32, *const VkFence, VkBool32, u64) -> VkResult;
    }
}
