// The parts of the Vulkan headers (src/video/khronos/vulkan/vulkan_core.h and
// vulkan_beta.h) that the Vulkan renderer and the Vulkan GPU backend of
// Simple DirectMedia Layer use.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! Vulkan declarations shared by the Vulkan renderer
//! (`render/vulkan`) and the Vulkan GPU backend (`gpu/vulkan`): handles,
//! enumerants, structures, and the [`vulkan_functions!`] macro their
//! function tables are declared with. Every function is looked up at run
//! time through the loader's `vkGetInstanceProcAddr` (`VK_NO_PROTOTYPES`).
//!
//! The structures implement `Default` as all zeroes, which is what the C
//! code's `= { 0 }` initializers give.

// (a header: the GPU backend's part 2 uses the rest)
#![allow(dead_code)]

use std::ffi::{c_char, c_void};

pub(crate) use crate::video::vulkan_utils::{
    PfnVkGetInstanceProcAddr, PfnVkVoidFunction, VkExtensionProperties, VkResult, VK_SUCCESS,
};

pub(crate) type VkBool32 = u32;
pub(crate) type VkDeviceSize = u64;
pub(crate) type VkFlags = u32;

// Dispatchable handles
pub(crate) type VkInstance = *mut c_void;
pub(crate) type VkPhysicalDevice = *mut c_void;
pub(crate) type VkDevice = *mut c_void;
pub(crate) type VkQueue = *mut c_void;
pub(crate) type VkCommandBuffer = *mut c_void;

// Non-dispatchable handles
pub(crate) type VkSemaphore = u64;
pub(crate) type VkFence = u64;
pub(crate) type VkDeviceMemory = u64;
pub(crate) type VkBuffer = u64;
pub(crate) type VkImage = u64;
pub(crate) type VkImageView = u64;
pub(crate) type VkShaderModule = u64;
pub(crate) type VkPipelineCache = u64;
pub(crate) type VkPipelineLayout = u64;
pub(crate) type VkRenderPass = u64;
pub(crate) type VkPipeline = u64;
pub(crate) type VkDescriptorSetLayout = u64;
pub(crate) type VkSampler = u64;
pub(crate) type VkDescriptorPool = u64;
pub(crate) type VkDescriptorSet = u64;
pub(crate) type VkFramebuffer = u64;
pub(crate) type VkCommandPool = u64;
pub(crate) type VkSurfaceKHR = u64;
pub(crate) type VkSwapchainKHR = u64;

/// `VK_NULL_HANDLE` (of a non-dispatchable handle).
pub(crate) const VK_NULL_HANDLE: u64 = 0;

pub(crate) const VK_TRUE: VkBool32 = 1;
pub(crate) const VK_FALSE: VkBool32 = 0;
pub(crate) const VK_QUEUE_FAMILY_IGNORED: u32 = !0;
pub(crate) const VK_SUBPASS_EXTERNAL: u32 = !0;
pub(crate) const VK_MAX_PHYSICAL_DEVICE_NAME_SIZE: usize = 256;
pub(crate) const VK_UUID_SIZE: usize = 16;
pub(crate) const VK_MAX_MEMORY_TYPES: usize = 32;
pub(crate) const VK_MAX_MEMORY_HEAPS: usize = 16;
pub(crate) const VK_MAX_EXTENSION_NAME_SIZE: usize = 256;
pub(crate) const VK_MAX_DESCRIPTION_SIZE: usize = 256;

/// `VK_MAKE_API_VERSION(0, 1, 0, 0)`
pub(crate) const VK_API_VERSION_1_0: u32 = 1 << 22;

/// `VK_VERSION_MAJOR()`
pub(crate) const fn vk_version_major(version: u32) -> u32 {
    version >> 22
}

// VkResult
pub(crate) const VK_SUBOPTIMAL_KHR: VkResult = 1000001003;
pub(crate) const VK_ERROR_OUT_OF_DEVICE_MEMORY: VkResult = -2;
pub(crate) const VK_ERROR_DEVICE_LOST: VkResult = -4;
pub(crate) const VK_ERROR_UNKNOWN: VkResult = -13;
pub(crate) const VK_ERROR_SURFACE_LOST_KHR: VkResult = -1000000000;
pub(crate) const VK_ERROR_OUT_OF_DATE_KHR: VkResult = -1000001004;

// VkStructureType
pub(crate) type VkStructureType = i32;
pub(crate) const VK_STRUCTURE_TYPE_APPLICATION_INFO: VkStructureType = 0;
pub(crate) const VK_STRUCTURE_TYPE_INSTANCE_CREATE_INFO: VkStructureType = 1;
pub(crate) const VK_STRUCTURE_TYPE_DEVICE_QUEUE_CREATE_INFO: VkStructureType = 2;
pub(crate) const VK_STRUCTURE_TYPE_DEVICE_CREATE_INFO: VkStructureType = 3;
pub(crate) const VK_STRUCTURE_TYPE_SUBMIT_INFO: VkStructureType = 4;
pub(crate) const VK_STRUCTURE_TYPE_MEMORY_ALLOCATE_INFO: VkStructureType = 5;
pub(crate) const VK_STRUCTURE_TYPE_FENCE_CREATE_INFO: VkStructureType = 8;
pub(crate) const VK_STRUCTURE_TYPE_SEMAPHORE_CREATE_INFO: VkStructureType = 9;
pub(crate) const VK_STRUCTURE_TYPE_BUFFER_CREATE_INFO: VkStructureType = 12;
pub(crate) const VK_STRUCTURE_TYPE_IMAGE_CREATE_INFO: VkStructureType = 14;
pub(crate) const VK_STRUCTURE_TYPE_IMAGE_VIEW_CREATE_INFO: VkStructureType = 15;
pub(crate) const VK_STRUCTURE_TYPE_SHADER_MODULE_CREATE_INFO: VkStructureType = 16;
pub(crate) const VK_STRUCTURE_TYPE_PIPELINE_SHADER_STAGE_CREATE_INFO: VkStructureType = 18;
pub(crate) const VK_STRUCTURE_TYPE_PIPELINE_VERTEX_INPUT_STATE_CREATE_INFO: VkStructureType = 19;
pub(crate) const VK_STRUCTURE_TYPE_PIPELINE_INPUT_ASSEMBLY_STATE_CREATE_INFO: VkStructureType = 20;
pub(crate) const VK_STRUCTURE_TYPE_PIPELINE_VIEWPORT_STATE_CREATE_INFO: VkStructureType = 22;
pub(crate) const VK_STRUCTURE_TYPE_PIPELINE_RASTERIZATION_STATE_CREATE_INFO: VkStructureType = 23;
pub(crate) const VK_STRUCTURE_TYPE_PIPELINE_MULTISAMPLE_STATE_CREATE_INFO: VkStructureType = 24;
pub(crate) const VK_STRUCTURE_TYPE_PIPELINE_DEPTH_STENCIL_STATE_CREATE_INFO: VkStructureType = 25;
pub(crate) const VK_STRUCTURE_TYPE_PIPELINE_COLOR_BLEND_STATE_CREATE_INFO: VkStructureType = 26;
pub(crate) const VK_STRUCTURE_TYPE_PIPELINE_DYNAMIC_STATE_CREATE_INFO: VkStructureType = 27;
pub(crate) const VK_STRUCTURE_TYPE_GRAPHICS_PIPELINE_CREATE_INFO: VkStructureType = 28;
pub(crate) const VK_STRUCTURE_TYPE_PIPELINE_LAYOUT_CREATE_INFO: VkStructureType = 30;
pub(crate) const VK_STRUCTURE_TYPE_SAMPLER_CREATE_INFO: VkStructureType = 31;
pub(crate) const VK_STRUCTURE_TYPE_DESCRIPTOR_SET_LAYOUT_CREATE_INFO: VkStructureType = 32;
pub(crate) const VK_STRUCTURE_TYPE_DESCRIPTOR_POOL_CREATE_INFO: VkStructureType = 33;
pub(crate) const VK_STRUCTURE_TYPE_DESCRIPTOR_SET_ALLOCATE_INFO: VkStructureType = 34;
pub(crate) const VK_STRUCTURE_TYPE_WRITE_DESCRIPTOR_SET: VkStructureType = 35;
pub(crate) const VK_STRUCTURE_TYPE_FRAMEBUFFER_CREATE_INFO: VkStructureType = 37;
pub(crate) const VK_STRUCTURE_TYPE_RENDER_PASS_CREATE_INFO: VkStructureType = 38;
pub(crate) const VK_STRUCTURE_TYPE_COMMAND_POOL_CREATE_INFO: VkStructureType = 39;
pub(crate) const VK_STRUCTURE_TYPE_COMMAND_BUFFER_ALLOCATE_INFO: VkStructureType = 40;
pub(crate) const VK_STRUCTURE_TYPE_COMMAND_BUFFER_BEGIN_INFO: VkStructureType = 42;
pub(crate) const VK_STRUCTURE_TYPE_RENDER_PASS_BEGIN_INFO: VkStructureType = 43;
pub(crate) const VK_STRUCTURE_TYPE_IMAGE_MEMORY_BARRIER: VkStructureType = 45;
pub(crate) const VK_STRUCTURE_TYPE_SWAPCHAIN_CREATE_INFO_KHR: VkStructureType = 1000001000;
pub(crate) const VK_STRUCTURE_TYPE_PRESENT_INFO_KHR: VkStructureType = 1000001001;
pub(crate) const VK_STRUCTURE_TYPE_IMAGE_VIEW_USAGE_CREATE_INFO: VkStructureType = 1000117002;
pub(crate) const VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_SAMPLER_YCBCR_CONVERSION_FEATURES:
    VkStructureType = 1000156004;

// VkFormat
pub(crate) type VkFormat = i32;
pub(crate) const VK_FORMAT_UNDEFINED: VkFormat = 0;
pub(crate) const VK_FORMAT_R4G4B4A4_UNORM_PACK16: VkFormat = 2;
pub(crate) const VK_FORMAT_B4G4R4A4_UNORM_PACK16: VkFormat = 3;
pub(crate) const VK_FORMAT_R5G6B5_UNORM_PACK16: VkFormat = 4;
pub(crate) const VK_FORMAT_B5G6R5_UNORM_PACK16: VkFormat = 5;
pub(crate) const VK_FORMAT_R5G5B5A1_UNORM_PACK16: VkFormat = 6;
pub(crate) const VK_FORMAT_B5G5R5A1_UNORM_PACK16: VkFormat = 7;
pub(crate) const VK_FORMAT_A1R5G5B5_UNORM_PACK16: VkFormat = 8;
pub(crate) const VK_FORMAT_R8_UNORM: VkFormat = 9;
pub(crate) const VK_FORMAT_R8G8_UNORM: VkFormat = 16;
pub(crate) const VK_FORMAT_R8G8B8A8_UNORM: VkFormat = 37;
pub(crate) const VK_FORMAT_R8G8B8A8_SRGB: VkFormat = 43;
pub(crate) const VK_FORMAT_B8G8R8A8_UNORM: VkFormat = 44;
pub(crate) const VK_FORMAT_B8G8R8A8_SRGB: VkFormat = 50;
#[cfg(target_endian = "big")]
pub(crate) const VK_FORMAT_A8B8G8R8_UNORM_PACK32: VkFormat = 51;
#[cfg(target_endian = "big")]
pub(crate) const VK_FORMAT_A8B8G8R8_SRGB_PACK32: VkFormat = 57;
pub(crate) const VK_FORMAT_A2B10G10R10_UNORM_PACK32: VkFormat = 64;
pub(crate) const VK_FORMAT_R16_UNORM: VkFormat = 70;
pub(crate) const VK_FORMAT_R16G16_UNORM: VkFormat = 77;
pub(crate) const VK_FORMAT_R16G16B16A16_SFLOAT: VkFormat = 97;
pub(crate) const VK_FORMAT_R32G32_SFLOAT: VkFormat = 103;
pub(crate) const VK_FORMAT_R32G32B32A32_SFLOAT: VkFormat = 109;
pub(crate) const VK_FORMAT_G8_B8_R8_3PLANE_420_UNORM: VkFormat = 1000156002;
pub(crate) const VK_FORMAT_G8_B8R8_2PLANE_420_UNORM: VkFormat = 1000156003;
pub(crate) const VK_FORMAT_G8_B8_R8_3PLANE_444_UNORM: VkFormat = 1000156006;
pub(crate) const VK_FORMAT_G10X6_B10X6R10X6_2PLANE_420_UNORM_3PACK16: VkFormat = 1000156013;
pub(crate) const VK_FORMAT_G16_B16_R16_3PLANE_420_UNORM: VkFormat = 1000156029;
pub(crate) const VK_FORMAT_G16_B16_R16_3PLANE_444_UNORM: VkFormat = 1000156033;
pub(crate) const VK_FORMAT_A4R4G4B4_UNORM_PACK16_EXT: VkFormat = 1000340000;
pub(crate) const VK_FORMAT_A4B4G4R4_UNORM_PACK16_EXT: VkFormat = 1000340001;

// VkColorSpaceKHR
pub(crate) type VkColorSpaceKHR = i32;
pub(crate) const VK_COLOR_SPACE_SRGB_NONLINEAR_KHR: VkColorSpaceKHR = 0;
pub(crate) const VK_COLOR_SPACE_EXTENDED_SRGB_LINEAR_EXT: VkColorSpaceKHR = 1000104002;
pub(crate) const VK_COLOR_SPACE_HDR10_ST2084_EXT: VkColorSpaceKHR = 1000104008;

// VkImageLayout
pub(crate) type VkImageLayout = i32;
pub(crate) const VK_IMAGE_LAYOUT_UNDEFINED: VkImageLayout = 0;
pub(crate) const VK_IMAGE_LAYOUT_COLOR_ATTACHMENT_OPTIMAL: VkImageLayout = 2;
pub(crate) const VK_IMAGE_LAYOUT_SHADER_READ_ONLY_OPTIMAL: VkImageLayout = 5;
pub(crate) const VK_IMAGE_LAYOUT_TRANSFER_SRC_OPTIMAL: VkImageLayout = 6;
pub(crate) const VK_IMAGE_LAYOUT_TRANSFER_DST_OPTIMAL: VkImageLayout = 7;
pub(crate) const VK_IMAGE_LAYOUT_PRESENT_SRC_KHR: VkImageLayout = 1000001002;

// VkAccessFlagBits
pub(crate) type VkAccessFlags = VkFlags;
pub(crate) const VK_ACCESS_NONE: VkAccessFlags = 0;
pub(crate) const VK_ACCESS_SHADER_READ_BIT: VkAccessFlags = 0x00000020;
pub(crate) const VK_ACCESS_COLOR_ATTACHMENT_READ_BIT: VkAccessFlags = 0x00000080;
pub(crate) const VK_ACCESS_COLOR_ATTACHMENT_WRITE_BIT: VkAccessFlags = 0x00000100;
pub(crate) const VK_ACCESS_TRANSFER_READ_BIT: VkAccessFlags = 0x00000800;
pub(crate) const VK_ACCESS_TRANSFER_WRITE_BIT: VkAccessFlags = 0x00001000;

// VkPipelineStageFlagBits
pub(crate) type VkPipelineStageFlags = VkFlags;
pub(crate) const VK_PIPELINE_STAGE_TOP_OF_PIPE_BIT: VkPipelineStageFlags = 0x00000001;
pub(crate) const VK_PIPELINE_STAGE_FRAGMENT_SHADER_BIT: VkPipelineStageFlags = 0x00000080;
pub(crate) const VK_PIPELINE_STAGE_COLOR_ATTACHMENT_OUTPUT_BIT: VkPipelineStageFlags = 0x00000400;
pub(crate) const VK_PIPELINE_STAGE_TRANSFER_BIT: VkPipelineStageFlags = 0x00001000;
pub(crate) const VK_PIPELINE_STAGE_ALL_COMMANDS_BIT: VkPipelineStageFlags = 0x00010000;

// VkImageAspectFlagBits
pub(crate) type VkImageAspectFlags = VkFlags;
pub(crate) const VK_IMAGE_ASPECT_COLOR_BIT: VkImageAspectFlags = 0x00000001;
pub(crate) const VK_IMAGE_ASPECT_PLANE_0_BIT: VkImageAspectFlags = 0x00000010;

// VkImageUsageFlagBits
pub(crate) type VkImageUsageFlags = VkFlags;
pub(crate) const VK_IMAGE_USAGE_TRANSFER_SRC_BIT: VkImageUsageFlags = 0x00000001;
pub(crate) const VK_IMAGE_USAGE_TRANSFER_DST_BIT: VkImageUsageFlags = 0x00000002;
pub(crate) const VK_IMAGE_USAGE_SAMPLED_BIT: VkImageUsageFlags = 0x00000004;
pub(crate) const VK_IMAGE_USAGE_COLOR_ATTACHMENT_BIT: VkImageUsageFlags = 0x00000010;

pub(crate) const VK_IMAGE_CREATE_MUTABLE_FORMAT_BIT: VkFlags = 0x00000008;
pub(crate) const VK_IMAGE_TYPE_2D: i32 = 1;
pub(crate) const VK_IMAGE_TILING_OPTIMAL: i32 = 0;
pub(crate) const VK_IMAGE_VIEW_TYPE_2D: i32 = 1;
pub(crate) const VK_SAMPLE_COUNT_1_BIT: VkFlags = 0x00000001;
pub(crate) const VK_SHARING_MODE_EXCLUSIVE: i32 = 0;
pub(crate) const VK_COMPONENT_SWIZZLE_IDENTITY: i32 = 0;

// VkBufferUsageFlagBits
pub(crate) type VkBufferUsageFlags = VkFlags;
pub(crate) const VK_BUFFER_USAGE_TRANSFER_SRC_BIT: VkBufferUsageFlags = 0x00000001;
pub(crate) const VK_BUFFER_USAGE_TRANSFER_DST_BIT: VkBufferUsageFlags = 0x00000002;
pub(crate) const VK_BUFFER_USAGE_UNIFORM_BUFFER_BIT: VkBufferUsageFlags = 0x00000010;
pub(crate) const VK_BUFFER_USAGE_VERTEX_BUFFER_BIT: VkBufferUsageFlags = 0x00000080;

// VkMemoryPropertyFlagBits
pub(crate) type VkMemoryPropertyFlags = VkFlags;
pub(crate) const VK_MEMORY_PROPERTY_DEVICE_LOCAL_BIT: VkMemoryPropertyFlags = 0x00000001;
pub(crate) const VK_MEMORY_PROPERTY_HOST_VISIBLE_BIT: VkMemoryPropertyFlags = 0x00000002;
pub(crate) const VK_MEMORY_PROPERTY_HOST_COHERENT_BIT: VkMemoryPropertyFlags = 0x00000004;

pub(crate) const VK_QUEUE_GRAPHICS_BIT: VkFlags = 0x00000001;
pub(crate) const VK_COMMAND_POOL_CREATE_RESET_COMMAND_BUFFER_BIT: VkFlags = 0x00000002;
pub(crate) const VK_COMMAND_BUFFER_LEVEL_PRIMARY: i32 = 0;
pub(crate) const VK_FENCE_CREATE_SIGNALED_BIT: VkFlags = 0x00000001;
pub(crate) const VK_DEPENDENCY_BY_REGION_BIT: VkFlags = 0x00000001;

// Render passes
pub(crate) type VkAttachmentLoadOp = i32;
pub(crate) const VK_ATTACHMENT_LOAD_OP_LOAD: VkAttachmentLoadOp = 0;
pub(crate) const VK_ATTACHMENT_LOAD_OP_CLEAR: VkAttachmentLoadOp = 1;
pub(crate) const VK_ATTACHMENT_LOAD_OP_DONT_CARE: VkAttachmentLoadOp = 2;
pub(crate) const VK_ATTACHMENT_STORE_OP_STORE: i32 = 0;
pub(crate) const VK_ATTACHMENT_STORE_OP_DONT_CARE: i32 = 1;
pub(crate) const VK_PIPELINE_BIND_POINT_GRAPHICS: i32 = 0;
pub(crate) const VK_SUBPASS_CONTENTS_INLINE: i32 = 0;

// Pipelines
pub(crate) type VkPrimitiveTopology = i32;
pub(crate) const VK_PRIMITIVE_TOPOLOGY_POINT_LIST: VkPrimitiveTopology = 0;
pub(crate) const VK_PRIMITIVE_TOPOLOGY_LINE_LIST: VkPrimitiveTopology = 1;
pub(crate) const VK_PRIMITIVE_TOPOLOGY_LINE_STRIP: VkPrimitiveTopology = 2;
pub(crate) const VK_PRIMITIVE_TOPOLOGY_TRIANGLE_LIST: VkPrimitiveTopology = 3;
pub(crate) const VK_SHADER_STAGE_VERTEX_BIT: VkFlags = 0x00000001;
pub(crate) const VK_SHADER_STAGE_FRAGMENT_BIT: VkFlags = 0x00000010;
pub(crate) const VK_VERTEX_INPUT_RATE_VERTEX: i32 = 0;
pub(crate) const VK_DYNAMIC_STATE_VIEWPORT: i32 = 0;
pub(crate) const VK_DYNAMIC_STATE_SCISSOR: i32 = 1;
pub(crate) const VK_CULL_MODE_NONE: VkFlags = 0;
pub(crate) const VK_POLYGON_MODE_FILL: i32 = 0;
pub(crate) const VK_FRONT_FACE_COUNTER_CLOCKWISE: i32 = 0;
pub(crate) const VK_COLOR_COMPONENT_R_BIT: VkFlags = 0x00000001;
pub(crate) const VK_COLOR_COMPONENT_G_BIT: VkFlags = 0x00000002;
pub(crate) const VK_COLOR_COMPONENT_B_BIT: VkFlags = 0x00000004;
pub(crate) const VK_COLOR_COMPONENT_A_BIT: VkFlags = 0x00000008;

// VkBlendFactor
pub(crate) type VkBlendFactor = i32;
pub(crate) const VK_BLEND_FACTOR_ZERO: VkBlendFactor = 0;
pub(crate) const VK_BLEND_FACTOR_ONE: VkBlendFactor = 1;
pub(crate) const VK_BLEND_FACTOR_SRC_COLOR: VkBlendFactor = 2;
pub(crate) const VK_BLEND_FACTOR_ONE_MINUS_SRC_COLOR: VkBlendFactor = 3;
pub(crate) const VK_BLEND_FACTOR_DST_COLOR: VkBlendFactor = 4;
pub(crate) const VK_BLEND_FACTOR_ONE_MINUS_DST_COLOR: VkBlendFactor = 5;
pub(crate) const VK_BLEND_FACTOR_SRC_ALPHA: VkBlendFactor = 6;
pub(crate) const VK_BLEND_FACTOR_ONE_MINUS_SRC_ALPHA: VkBlendFactor = 7;
pub(crate) const VK_BLEND_FACTOR_DST_ALPHA: VkBlendFactor = 8;
pub(crate) const VK_BLEND_FACTOR_ONE_MINUS_DST_ALPHA: VkBlendFactor = 9;
pub(crate) const VK_BLEND_FACTOR_MAX_ENUM: VkBlendFactor = 0x7FFFFFFF;

// VkBlendOp
pub(crate) type VkBlendOp = i32;
pub(crate) const VK_BLEND_OP_ADD: VkBlendOp = 0;
pub(crate) const VK_BLEND_OP_SUBTRACT: VkBlendOp = 1;
pub(crate) const VK_BLEND_OP_REVERSE_SUBTRACT: VkBlendOp = 2;
pub(crate) const VK_BLEND_OP_MIN: VkBlendOp = 3;
pub(crate) const VK_BLEND_OP_MAX: VkBlendOp = 4;
pub(crate) const VK_BLEND_OP_MAX_ENUM: VkBlendOp = 0x7FFFFFFF;

// Descriptors and samplers
pub(crate) const VK_DESCRIPTOR_TYPE_SAMPLER: i32 = 0;
pub(crate) const VK_DESCRIPTOR_TYPE_COMBINED_IMAGE_SAMPLER: i32 = 1;
pub(crate) const VK_DESCRIPTOR_TYPE_UNIFORM_BUFFER: i32 = 6;
pub(crate) const VK_FILTER_NEAREST: i32 = 0;
pub(crate) const VK_FILTER_LINEAR: i32 = 1;
pub(crate) const VK_SAMPLER_MIPMAP_MODE_NEAREST: i32 = 0;
pub(crate) const VK_SAMPLER_ADDRESS_MODE_REPEAT: i32 = 0;
pub(crate) const VK_SAMPLER_ADDRESS_MODE_CLAMP_TO_EDGE: i32 = 2;

// Surfaces and swapchains
pub(crate) type VkSurfaceTransformFlagBitsKHR = VkFlags;
pub(crate) const VK_SURFACE_TRANSFORM_IDENTITY_BIT_KHR: VkSurfaceTransformFlagBitsKHR = 0x00000001;
pub(crate) const VK_SURFACE_TRANSFORM_ROTATE_90_BIT_KHR: VkSurfaceTransformFlagBitsKHR = 0x00000002;
pub(crate) const VK_SURFACE_TRANSFORM_ROTATE_180_BIT_KHR: VkSurfaceTransformFlagBitsKHR =
    0x00000004;
pub(crate) const VK_SURFACE_TRANSFORM_ROTATE_270_BIT_KHR: VkSurfaceTransformFlagBitsKHR =
    0x00000008;
pub(crate) const VK_COMPOSITE_ALPHA_OPAQUE_BIT_KHR: VkFlags = 0x00000001;
pub(crate) const VK_COMPOSITE_ALPHA_INHERIT_BIT_KHR: VkFlags = 0x00000008;
pub(crate) type VkPresentModeKHR = i32;
pub(crate) const VK_PRESENT_MODE_IMMEDIATE_KHR: VkPresentModeKHR = 0;
pub(crate) const VK_PRESENT_MODE_MAILBOX_KHR: VkPresentModeKHR = 1;
pub(crate) const VK_PRESENT_MODE_FIFO_KHR: VkPresentModeKHR = 2;
pub(crate) const VK_PRESENT_MODE_FIFO_RELAXED_KHR: VkPresentModeKHR = 3;

/// Structures with an all-zero `Default` (the C `= { 0 }`).
macro_rules! vk_structs {
    ($(
        $(#[$m:meta])*
        struct $name:ident { $($field:ident: $ty:ty,)* }
    )*) => {$(
        $(#[$m])*
        #[repr(C)]
        #[derive(Clone, Copy)]
        pub(crate) struct $name { $(pub(crate) $field: $ty,)* }

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

/// The feature structures: `VkBool32` members (after `sType` and `pNext`
/// for the extensible ones), with the C names (for upstream's messages) and
/// the members as an array, for the code that walks them like C's pointer
/// arithmetic over the members does.
macro_rules! vk_feature_structs {
    ($(
        $(#[$m:meta])*
        struct $name:ident $(with $header:ident)? { $($field:ident = $cname:literal,)* }
    )*) => {$(
        vk_feature_structs!(@struct $(#[$m])* $name [$($header)?] { $($field,)* });

        #[allow(dead_code)] // (not every structure's members are walked)
        impl $name {
            /// The C names of the features, in order.
            pub(crate) const NAMES: &'static [&'static str] = &[$($cname,)*];

            /// The features, in order.
            pub(crate) fn bools(&self) -> Vec<VkBool32> {
                vec![$(self.$field,)*]
            }

            /// The features, in order, to change them.
            pub(crate) fn bools_mut(&mut self) -> Vec<&mut VkBool32> {
                vec![$(&mut self.$field,)*]
            }
        }

        impl Default for $name {
            fn default() -> $name {
                // SAFETY: the fields are integers and raw pointers, for
                // which all zeroes is a valid value.
                unsafe { std::mem::zeroed() }
            }
        }
    )*};
    (@struct $(#[$m:meta])* $name:ident [] { $($field:ident,)* }) => {
        $(#[$m])*
        #[repr(C)]
        #[derive(Clone, Copy)]
        pub(crate) struct $name { $(pub(crate) $field: VkBool32,)* }
    };
    (@struct $(#[$m:meta])* $name:ident [header] { $($field:ident,)* }) => {
        $(#[$m])*
        #[repr(C)]
        #[derive(Clone, Copy)]
        pub(crate) struct $name {
            pub(crate) s_type: VkStructureType,
            pub(crate) p_next: *mut c_void,
            $(pub(crate) $field: VkBool32,)*
        }
    };
}

/// The function tables: a structure of function pointers for each kind of
/// function (`VULKAN_GLOBAL_FUNCTION`, `VULKAN_INSTANCE_FUNCTION`,
/// `VULKAN_DEVICE_FUNCTION`), looked up all at once. The functions of an
/// `[optional] { ... }` block may be missing (they're `Option`s).
macro_rules! vulkan_functions {
    ($(
        $(#[$m:meta])*
        struct $table:ident {
            $(
                $(#[$fm:meta])*
                $field:ident = $sym:literal: fn($($arg:ty),* $(,)?) $(-> $ret:ty)?;
            )*
            $(
                [optional] {
                    $(
                        $(#[$ofm:meta])*
                        $ofield:ident = $osym:literal: fn($($oarg:ty),* $(,)?) $(-> $oret:ty)?;
                    )*
                }
            )?
        }
    )*) => {$(
        $(#[$m])*
        pub(super) struct $table {
            $(
                $(#[$fm])*
                pub(super) $field: unsafe extern "system" fn($($arg),*) $(-> $ret)?,
            )*
            $($(
                $(#[$ofm])*
                pub(super) $ofield: Option<unsafe extern "system" fn($($oarg),*) $(-> $oret)?>,
            )*)?
        }

        impl $table {
            /// Look up every function with `lookup`; the name of the first
            /// required one that isn't there as the error.
            ///
            /// # Safety
            ///
            /// `lookup` returns the functions of the names it's given.
            pub(super) unsafe fn load(
                mut lookup: impl FnMut(&::std::ffi::CStr) -> Option<$crate::video::vk::PfnVkVoidFunction>,
            ) -> std::result::Result<$table, &'static str> {
                Ok($table {
                    $($field: {
                        let f = lookup($sym).ok_or($sym.to_str().unwrap_or(""))?;
                        // SAFETY: the caller's contract: the function of
                        // that name has this type.
                        unsafe {
                            std::mem::transmute::<
                                $crate::video::vk::PfnVkVoidFunction,
                                unsafe extern "system" fn($($arg),*) $(-> $ret)?,
                            >(f)
                        }
                    },)*
                    $($($ofield: lookup($osym).map(|f| {
                        // SAFETY: the caller's contract: the function of
                        // that name has this type.
                        unsafe {
                            std::mem::transmute::<
                                $crate::video::vk::PfnVkVoidFunction,
                                unsafe extern "system" fn($($oarg),*) $(-> $oret)?,
                            >(f)
                        }
                    }),)*)?
                })
            }
        }
    )*};
}
pub(crate) use vulkan_functions;

/// `const VkAllocationCallbacks *` (always null here).
pub(crate) type Alloc = *const c_void;

vk_feature_structs! {
    /// `VkPhysicalDeviceFeatures` (55 features)
    struct VkPhysicalDeviceFeatures {
        robust_buffer_access = "robustBufferAccess",
        full_draw_index_uint32 = "fullDrawIndexUint32",
        image_cube_array = "imageCubeArray",
        independent_blend = "independentBlend",
        geometry_shader = "geometryShader",
        tessellation_shader = "tessellationShader",
        sample_rate_shading = "sampleRateShading",
        dual_src_blend = "dualSrcBlend",
        logic_op = "logicOp",
        multi_draw_indirect = "multiDrawIndirect",
        draw_indirect_first_instance = "drawIndirectFirstInstance",
        depth_clamp = "depthClamp",
        depth_bias_clamp = "depthBiasClamp",
        fill_mode_non_solid = "fillModeNonSolid",
        depth_bounds = "depthBounds",
        wide_lines = "wideLines",
        large_points = "largePoints",
        alpha_to_one = "alphaToOne",
        multi_viewport = "multiViewport",
        sampler_anisotropy = "samplerAnisotropy",
        texture_compression_etc2 = "textureCompressionETC2",
        texture_compression_astc_ldr = "textureCompressionASTC_LDR",
        texture_compression_bc = "textureCompressionBC",
        occlusion_query_precise = "occlusionQueryPrecise",
        pipeline_statistics_query = "pipelineStatisticsQuery",
        vertex_pipeline_stores_and_atomics = "vertexPipelineStoresAndAtomics",
        fragment_stores_and_atomics = "fragmentStoresAndAtomics",
        shader_tessellation_and_geometry_point_size = "shaderTessellationAndGeometryPointSize",
        shader_image_gather_extended = "shaderImageGatherExtended",
        shader_storage_image_extended_formats = "shaderStorageImageExtendedFormats",
        shader_storage_image_multisample = "shaderStorageImageMultisample",
        shader_storage_image_read_without_format = "shaderStorageImageReadWithoutFormat",
        shader_storage_image_write_without_format = "shaderStorageImageWriteWithoutFormat",
        shader_uniform_buffer_array_dynamic_indexing = "shaderUniformBufferArrayDynamicIndexing",
        shader_sampled_image_array_dynamic_indexing = "shaderSampledImageArrayDynamicIndexing",
        shader_storage_buffer_array_dynamic_indexing = "shaderStorageBufferArrayDynamicIndexing",
        shader_storage_image_array_dynamic_indexing = "shaderStorageImageArrayDynamicIndexing",
        shader_clip_distance = "shaderClipDistance",
        shader_cull_distance = "shaderCullDistance",
        shader_float64 = "shaderFloat64",
        shader_int64 = "shaderInt64",
        shader_int16 = "shaderInt16",
        shader_resource_residency = "shaderResourceResidency",
        shader_resource_min_lod = "shaderResourceMinLod",
        sparse_binding = "sparseBinding",
        sparse_residency_buffer = "sparseResidencyBuffer",
        sparse_residency_image2d = "sparseResidencyImage2D",
        sparse_residency_image3d = "sparseResidencyImage3D",
        sparse_residency2_samples = "sparseResidency2Samples",
        sparse_residency4_samples = "sparseResidency4Samples",
        sparse_residency8_samples = "sparseResidency8Samples",
        sparse_residency16_samples = "sparseResidency16Samples",
        sparse_residency_aliased = "sparseResidencyAliased",
        variable_multisample_rate = "variableMultisampleRate",
        inherited_queries = "inheritedQueries",
    }
    /// `VkPhysicalDeviceVulkan11Features` (12 features)
    struct VkPhysicalDeviceVulkan11Features with header {
        storage_buffer16_bit_access = "storageBuffer16BitAccess",
        uniform_and_storage_buffer16_bit_access = "uniformAndStorageBuffer16BitAccess",
        storage_push_constant16 = "storagePushConstant16",
        storage_input_output16 = "storageInputOutput16",
        multiview = "multiview",
        multiview_geometry_shader = "multiviewGeometryShader",
        multiview_tessellation_shader = "multiviewTessellationShader",
        variable_pointers_storage_buffer = "variablePointersStorageBuffer",
        variable_pointers = "variablePointers",
        protected_memory = "protectedMemory",
        sampler_ycbcr_conversion = "samplerYcbcrConversion",
        shader_draw_parameters = "shaderDrawParameters",
    }
    /// `VkPhysicalDeviceVulkan12Features` (47 features)
    struct VkPhysicalDeviceVulkan12Features with header {
        sampler_mirror_clamp_to_edge = "samplerMirrorClampToEdge",
        draw_indirect_count = "drawIndirectCount",
        storage_buffer8_bit_access = "storageBuffer8BitAccess",
        uniform_and_storage_buffer8_bit_access = "uniformAndStorageBuffer8BitAccess",
        storage_push_constant8 = "storagePushConstant8",
        shader_buffer_int64_atomics = "shaderBufferInt64Atomics",
        shader_shared_int64_atomics = "shaderSharedInt64Atomics",
        shader_float16 = "shaderFloat16",
        shader_int8 = "shaderInt8",
        descriptor_indexing = "descriptorIndexing",
        shader_input_attachment_array_dynamic_indexing = "shaderInputAttachmentArrayDynamicIndexing",
        shader_uniform_texel_buffer_array_dynamic_indexing = "shaderUniformTexelBufferArrayDynamicIndexing",
        shader_storage_texel_buffer_array_dynamic_indexing = "shaderStorageTexelBufferArrayDynamicIndexing",
        shader_uniform_buffer_array_non_uniform_indexing = "shaderUniformBufferArrayNonUniformIndexing",
        shader_sampled_image_array_non_uniform_indexing = "shaderSampledImageArrayNonUniformIndexing",
        shader_storage_buffer_array_non_uniform_indexing = "shaderStorageBufferArrayNonUniformIndexing",
        shader_storage_image_array_non_uniform_indexing = "shaderStorageImageArrayNonUniformIndexing",
        shader_input_attachment_array_non_uniform_indexing = "shaderInputAttachmentArrayNonUniformIndexing",
        shader_uniform_texel_buffer_array_non_uniform_indexing = "shaderUniformTexelBufferArrayNonUniformIndexing",
        shader_storage_texel_buffer_array_non_uniform_indexing = "shaderStorageTexelBufferArrayNonUniformIndexing",
        descriptor_binding_uniform_buffer_update_after_bind = "descriptorBindingUniformBufferUpdateAfterBind",
        descriptor_binding_sampled_image_update_after_bind = "descriptorBindingSampledImageUpdateAfterBind",
        descriptor_binding_storage_image_update_after_bind = "descriptorBindingStorageImageUpdateAfterBind",
        descriptor_binding_storage_buffer_update_after_bind = "descriptorBindingStorageBufferUpdateAfterBind",
        descriptor_binding_uniform_texel_buffer_update_after_bind = "descriptorBindingUniformTexelBufferUpdateAfterBind",
        descriptor_binding_storage_texel_buffer_update_after_bind = "descriptorBindingStorageTexelBufferUpdateAfterBind",
        descriptor_binding_update_unused_while_pending = "descriptorBindingUpdateUnusedWhilePending",
        descriptor_binding_partially_bound = "descriptorBindingPartiallyBound",
        descriptor_binding_variable_descriptor_count = "descriptorBindingVariableDescriptorCount",
        runtime_descriptor_array = "runtimeDescriptorArray",
        sampler_filter_minmax = "samplerFilterMinmax",
        scalar_block_layout = "scalarBlockLayout",
        imageless_framebuffer = "imagelessFramebuffer",
        uniform_buffer_standard_layout = "uniformBufferStandardLayout",
        shader_subgroup_extended_types = "shaderSubgroupExtendedTypes",
        separate_depth_stencil_layouts = "separateDepthStencilLayouts",
        host_query_reset = "hostQueryReset",
        timeline_semaphore = "timelineSemaphore",
        buffer_device_address = "bufferDeviceAddress",
        buffer_device_address_capture_replay = "bufferDeviceAddressCaptureReplay",
        buffer_device_address_multi_device = "bufferDeviceAddressMultiDevice",
        vulkan_memory_model = "vulkanMemoryModel",
        vulkan_memory_model_device_scope = "vulkanMemoryModelDeviceScope",
        vulkan_memory_model_availability_visibility_chains = "vulkanMemoryModelAvailabilityVisibilityChains",
        shader_output_viewport_index = "shaderOutputViewportIndex",
        shader_output_layer = "shaderOutputLayer",
        subgroup_broadcast_dynamic_id = "subgroupBroadcastDynamicId",
    }
    /// `VkPhysicalDeviceVulkan13Features` (15 features)
    struct VkPhysicalDeviceVulkan13Features with header {
        robust_image_access = "robustImageAccess",
        inline_uniform_block = "inlineUniformBlock",
        descriptor_binding_inline_uniform_block_update_after_bind = "descriptorBindingInlineUniformBlockUpdateAfterBind",
        pipeline_creation_cache_control = "pipelineCreationCacheControl",
        private_data = "privateData",
        shader_demote_to_helper_invocation = "shaderDemoteToHelperInvocation",
        shader_terminate_invocation = "shaderTerminateInvocation",
        subgroup_size_control = "subgroupSizeControl",
        compute_full_subgroups = "computeFullSubgroups",
        synchronization2 = "synchronization2",
        texture_compression_astc_hdr = "textureCompressionASTC_HDR",
        shader_zero_initialize_workgroup_memory = "shaderZeroInitializeWorkgroupMemory",
        dynamic_rendering = "dynamicRendering",
        shader_integer_dot_product = "shaderIntegerDotProduct",
        maintenance4 = "maintenance4",
    }
    /// `VkPhysicalDevice16BitStorageFeatures` (4 features)
    struct VkPhysicalDevice16BitStorageFeatures with header {
        storage_buffer16_bit_access = "storageBuffer16BitAccess",
        uniform_and_storage_buffer16_bit_access = "uniformAndStorageBuffer16BitAccess",
        storage_push_constant16 = "storagePushConstant16",
        storage_input_output16 = "storageInputOutput16",
    }
    /// `VkPhysicalDeviceMultiviewFeatures` (3 features)
    struct VkPhysicalDeviceMultiviewFeatures with header {
        multiview = "multiview",
        multiview_geometry_shader = "multiviewGeometryShader",
        multiview_tessellation_shader = "multiviewTessellationShader",
    }
    /// `VkPhysicalDeviceProtectedMemoryFeatures` (1 features)
    struct VkPhysicalDeviceProtectedMemoryFeatures with header {
        protected_memory = "protectedMemory",
    }
    /// `VkPhysicalDeviceSamplerYcbcrConversionFeatures` (1 features)
    struct VkPhysicalDeviceSamplerYcbcrConversionFeatures with header {
        sampler_ycbcr_conversion = "samplerYcbcrConversion",
    }
    /// `VkPhysicalDeviceShaderDrawParametersFeatures` (1 features)
    struct VkPhysicalDeviceShaderDrawParametersFeatures with header {
        shader_draw_parameters = "shaderDrawParameters",
    }
    /// `VkPhysicalDeviceVariablePointersFeatures` (2 features)
    struct VkPhysicalDeviceVariablePointersFeatures with header {
        variable_pointers_storage_buffer = "variablePointersStorageBuffer",
        variable_pointers = "variablePointers",
    }
    /// `VkPhysicalDevicePortabilitySubsetFeaturesKHR` (15 features)
    struct VkPhysicalDevicePortabilitySubsetFeaturesKHR with header {
        constant_alpha_color_blend_factors = "constantAlphaColorBlendFactors",
        events = "events",
        image_view_format_reinterpretation = "imageViewFormatReinterpretation",
        image_view_format_swizzle = "imageViewFormatSwizzle",
        image_view2d_on3d_image = "imageView2DOn3DImage",
        multisample_array_image = "multisampleArrayImage",
        mutable_comparison_samplers = "mutableComparisonSamplers",
        point_polygons = "pointPolygons",
        sampler_mip_lod_bias = "samplerMipLodBias",
        separate_stencil_mask_ref = "separateStencilMaskRef",
        shader_sample_rate_interpolation_functions = "shaderSampleRateInterpolationFunctions",
        tessellation_isolines = "tessellationIsolines",
        tessellation_point_mode = "tessellationPointMode",
        triangle_fans = "triangleFans",
        vertex_attribute_access_beyond_stride = "vertexAttributeAccessBeyondStride",
    }
}

// The structures the GPU backend uses besides the renderer's.
vk_structs! {
    /// `VkBaseOutStructure`
    struct VkBaseOutStructure { s_type: VkStructureType, p_next: *mut VkBaseOutStructure, }
    /// `VkFormatProperties`
    struct VkFormatProperties {
        linear_tiling_features: VkFlags,
        optimal_tiling_features: VkFlags,
        buffer_features: VkFlags,
    }
    /// `VkPhysicalDeviceFeatures2`
    struct VkPhysicalDeviceFeatures2 {
        s_type: VkStructureType,
        p_next: *mut c_void,
        features: VkPhysicalDeviceFeatures,
    }
    /// `VkPhysicalDeviceProperties2`
    struct VkPhysicalDeviceProperties2 {
        s_type: VkStructureType,
        p_next: *mut c_void,
        properties: VkPhysicalDeviceProperties,
    }
    /// `VkConformanceVersion`
    struct VkConformanceVersion { major: u8, minor: u8, subminor: u8, patch: u8, }
    /// `VkPhysicalDeviceDriverProperties`
    struct VkPhysicalDeviceDriverProperties {
        s_type: VkStructureType,
        p_next: *mut c_void,
        driver_id: i32,
        driver_name: [c_char; VK_MAX_DRIVER_NAME_SIZE],
        driver_info: [c_char; VK_MAX_DRIVER_INFO_SIZE],
        conformance_version: VkConformanceVersion,
    }
    /// `VkPhysicalDeviceLayeredDriverPropertiesMSFT`
    struct VkPhysicalDeviceLayeredDriverPropertiesMSFT {
        s_type: VkStructureType,
        p_next: *mut c_void,
        underlying_api: i32,
    }
    /// `VkDebugUtilsLabelEXT`
    struct VkDebugUtilsLabelEXT {
        s_type: VkStructureType,
        p_next: *const c_void,
        p_label_name: *const c_char,
        color: [f32; 4],
    }
    /// `VkDebugUtilsObjectNameInfoEXT`
    struct VkDebugUtilsObjectNameInfoEXT {
        s_type: VkStructureType,
        p_next: *const c_void,
        object_type: i32,
        object_handle: u64,
        p_object_name: *const c_char,
    }
    /// `VkBufferCopy`
    struct VkBufferCopy { src_offset: VkDeviceSize, dst_offset: VkDeviceSize, size: VkDeviceSize, }
    /// `VkImageCopy`
    struct VkImageCopy {
        src_subresource: VkImageSubresourceLayers,
        src_offset: VkOffset3D,
        dst_subresource: VkImageSubresourceLayers,
        dst_offset: VkOffset3D,
        extent: VkExtent3D,
    }
    /// `VkImageBlit`
    struct VkImageBlit {
        src_subresource: VkImageSubresourceLayers,
        src_offsets: [VkOffset3D; 2],
        dst_subresource: VkImageSubresourceLayers,
        dst_offsets: [VkOffset3D; 2],
    }
    /// `VkImageResolve`
    struct VkImageResolve {
        src_subresource: VkImageSubresourceLayers,
        src_offset: VkOffset3D,
        dst_subresource: VkImageSubresourceLayers,
        dst_offset: VkOffset3D,
        extent: VkExtent3D,
    }
    /// `VkClearDepthStencilValue`
    struct VkClearDepthStencilValue { depth: f32, stencil: u32, }
    /// `VkClearAttachment`
    struct VkClearAttachment {
        aspect_mask: VkImageAspectFlags,
        color_attachment: u32,
        clear_value: VkClearValue,
    }
    /// `VkClearRect`
    struct VkClearRect { rect: VkRect2D, base_array_layer: u32, layer_count: u32, }
    /// `VkMemoryBarrier`
    struct VkMemoryBarrier {
        s_type: VkStructureType,
        p_next: *const c_void,
        src_access_mask: VkAccessFlags,
        dst_access_mask: VkAccessFlags,
    }
    /// `VkBufferMemoryBarrier`
    struct VkBufferMemoryBarrier {
        s_type: VkStructureType,
        p_next: *const c_void,
        src_access_mask: VkAccessFlags,
        dst_access_mask: VkAccessFlags,
        src_queue_family_index: u32,
        dst_queue_family_index: u32,
        buffer: VkBuffer,
        offset: VkDeviceSize,
        size: VkDeviceSize,
    }
    /// `VkComputePipelineCreateInfo`
    struct VkComputePipelineCreateInfo {
        s_type: VkStructureType,
        p_next: *const c_void,
        flags: VkFlags,
        stage: VkPipelineShaderStageCreateInfo,
        layout: VkPipelineLayout,
        base_pipeline_handle: VkPipeline,
        base_pipeline_index: i32,
    }
    /// `VkPipelineCacheCreateInfo`
    struct VkPipelineCacheCreateInfo {
        s_type: VkStructureType,
        p_next: *const c_void,
        flags: VkFlags,
        initial_data_size: usize,
        p_initial_data: *const c_void,
    }
}

/// The size of the structures the GPU backend adds on 64-bit targets, as
/// the C compiler lays out vulkan_core.h's (checked with `sizeof` there).
#[cfg(target_pointer_width = "64")]
const _: () = {
    assert!(size_of::<VkBaseOutStructure>() == 16);
    assert!(size_of::<VkFormatProperties>() == 12);
    assert!(size_of::<VkPhysicalDeviceFeatures2>() == 240);
    assert!(size_of::<VkPhysicalDeviceProperties2>() == 840);
    assert!(size_of::<VkPhysicalDeviceDriverProperties>() == 536);
    assert!(size_of::<VkPhysicalDeviceLayeredDriverPropertiesMSFT>() == 24);
    assert!(size_of::<VkDebugUtilsLabelEXT>() == 40);
    assert!(size_of::<VkDebugUtilsObjectNameInfoEXT>() == 40);
    assert!(size_of::<VkBufferCopy>() == 24);
    assert!(size_of::<VkImageCopy>() == 68);
    assert!(size_of::<VkImageBlit>() == 80);
    assert!(size_of::<VkImageResolve>() == 68);
    assert!(size_of::<VkClearAttachment>() == 24);
    assert!(size_of::<VkClearRect>() == 24);
    assert!(size_of::<VkMemoryBarrier>() == 24);
    assert!(size_of::<VkBufferMemoryBarrier>() == 56);
    assert!(size_of::<VkComputePipelineCreateInfo>() == 96);
    assert!(size_of::<VkPipelineCacheCreateInfo>() == 40);
    assert!(size_of::<VkPhysicalDeviceFeatures>() == 220);
    assert!(size_of::<VkPhysicalDeviceVulkan11Features>() == 64);
    assert!(size_of::<VkPhysicalDeviceVulkan12Features>() == 208);
    assert!(size_of::<VkPhysicalDeviceVulkan13Features>() == 80);
    assert!(size_of::<VkPhysicalDevice16BitStorageFeatures>() == 32);
    assert!(size_of::<VkPhysicalDeviceMultiviewFeatures>() == 32);
    assert!(size_of::<VkPhysicalDeviceProtectedMemoryFeatures>() == 24);
    assert!(size_of::<VkPhysicalDeviceSamplerYcbcrConversionFeatures>() == 24);
    assert!(size_of::<VkPhysicalDeviceShaderDrawParametersFeatures>() == 24);
    assert!(size_of::<VkPhysicalDeviceVariablePointersFeatures>() == 24);
    assert!(size_of::<VkPhysicalDevicePortabilitySubsetFeaturesKHR>() == 80);
    assert!(std::mem::offset_of!(VkPhysicalDeviceDriverProperties, conformance_version) == 532);
    assert!(std::mem::offset_of!(VkDebugUtilsObjectNameInfoEXT, p_object_name) == 32);
};

impl std::fmt::Debug for VkComponentMapping {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "VkComponentMapping({}, {}, {}, {})",
            self.r, self.g, self.b, self.a
        )
    }
}

impl PartialEq for VkComponentMapping {
    fn eq(&self, other: &VkComponentMapping) -> bool {
        (self.r, self.g, self.b, self.a) == (other.r, other.g, other.b, other.a)
    }
}

/// `VK_MAKE_API_VERSION(variant, major, minor, patch)`
pub(crate) const fn vk_make_api_version(variant: u32, major: u32, minor: u32, patch: u32) -> u32 {
    (variant << 29) | (major << 22) | (minor << 12) | patch
}

/// `VK_MAKE_VERSION(major, minor, patch)`
pub(crate) const fn vk_make_version(major: u32, minor: u32, patch: u32) -> u32 {
    (major << 22) | (minor << 12) | patch
}

/// `VK_API_VERSION_MINOR()` (also `VK_VERSION_MINOR()`, which has the same
/// value for the versions in use)
pub(crate) const fn vk_api_version_minor(version: u32) -> u32 {
    (version >> 12) & 0x3FF
}

/// A fixed-size, NUL-terminated `char` array of a Vulkan structure as text
/// (lossily as UTF-8).
pub(crate) fn c_chars_to_string(chars: &[c_char]) -> String {
    let bytes: Vec<u8> = chars
        .iter()
        .take_while(|&&c| c != 0)
        .map(|&c| c as u8)
        .collect();
    String::from_utf8_lossy(&bytes).into_owned()
}

// The constants the GPU backend uses besides the renderer's (generated
// from vulkan_core.h).
pub(crate) const VK_ACCESS_DEPTH_STENCIL_ATTACHMENT_READ_BIT: VkAccessFlags = 0x00000200;
pub(crate) const VK_ACCESS_DEPTH_STENCIL_ATTACHMENT_WRITE_BIT: VkAccessFlags = 0x00000400;
pub(crate) const VK_ACCESS_INDEX_READ_BIT: VkAccessFlags = 0x00000002;
pub(crate) const VK_ACCESS_INDIRECT_COMMAND_READ_BIT: VkAccessFlags = 0x00000001;
pub(crate) const VK_ACCESS_SHADER_WRITE_BIT: VkAccessFlags = 0x00000040;
pub(crate) const VK_ACCESS_VERTEX_ATTRIBUTE_READ_BIT: VkAccessFlags = 0x00000004;
pub(crate) const VK_BLEND_FACTOR_CONSTANT_COLOR: VkBlendFactor = 10;
pub(crate) const VK_BLEND_FACTOR_ONE_MINUS_CONSTANT_COLOR: VkBlendFactor = 11;
pub(crate) const VK_BLEND_FACTOR_SRC_ALPHA_SATURATE: VkBlendFactor = 14;
pub(crate) const VK_BORDER_COLOR_FLOAT_TRANSPARENT_BLACK: i32 = 0;
pub(crate) const VK_BUFFER_USAGE_INDEX_BUFFER_BIT: VkBufferUsageFlags = 0x00000040;
pub(crate) const VK_BUFFER_USAGE_INDIRECT_BUFFER_BIT: VkBufferUsageFlags = 0x00000100;
pub(crate) const VK_BUFFER_USAGE_STORAGE_BUFFER_BIT: VkBufferUsageFlags = 0x00000020;
pub(crate) const VK_COMMAND_BUFFER_USAGE_ONE_TIME_SUBMIT_BIT: VkFlags = 0x00000001;
pub(crate) const VK_COMPARE_OP_ALWAYS: i32 = 7;
pub(crate) const VK_COMPARE_OP_EQUAL: i32 = 2;
pub(crate) const VK_COMPARE_OP_GREATER: i32 = 4;
pub(crate) const VK_COMPARE_OP_GREATER_OR_EQUAL: i32 = 6;
pub(crate) const VK_COMPARE_OP_LESS: i32 = 1;
pub(crate) const VK_COMPARE_OP_LESS_OR_EQUAL: i32 = 3;
pub(crate) const VK_COMPARE_OP_NEVER: i32 = 0;
pub(crate) const VK_COMPARE_OP_NOT_EQUAL: i32 = 5;
pub(crate) const VK_COMPONENT_SWIZZLE_A: i32 = 6;
pub(crate) const VK_COMPONENT_SWIZZLE_B: i32 = 5;
pub(crate) const VK_COMPONENT_SWIZZLE_G: i32 = 4;
pub(crate) const VK_COMPONENT_SWIZZLE_R: i32 = 3;
pub(crate) const VK_COMPONENT_SWIZZLE_ZERO: i32 = 1;
pub(crate) const VK_CULL_MODE_BACK_BIT: VkFlags = 0x00000002;
pub(crate) const VK_CULL_MODE_FRONT_AND_BACK: VkFlags = 0x00000003;
pub(crate) const VK_CULL_MODE_FRONT_BIT: VkFlags = 0x00000001;
pub(crate) const VK_DESCRIPTOR_TYPE_SAMPLED_IMAGE: i32 = 2;
pub(crate) const VK_DESCRIPTOR_TYPE_STORAGE_BUFFER: i32 = 7;
pub(crate) const VK_DESCRIPTOR_TYPE_STORAGE_IMAGE: i32 = 3;
pub(crate) const VK_DESCRIPTOR_TYPE_UNIFORM_BUFFER_DYNAMIC: i32 = 8;
pub(crate) const VK_DYNAMIC_STATE_BLEND_CONSTANTS: i32 = 4;
pub(crate) const VK_DYNAMIC_STATE_STENCIL_REFERENCE: i32 = 8;
pub(crate) const VK_ERROR_EXTENSION_NOT_PRESENT: VkResult = -7;
pub(crate) const VK_ERROR_FEATURE_NOT_PRESENT: VkResult = -8;
pub(crate) const VK_ERROR_FRAGMENTED_POOL: VkResult = -12;
pub(crate) const VK_ERROR_FULL_SCREEN_EXCLUSIVE_MODE_LOST_EXT: VkResult = -1000255000;
pub(crate) const VK_ERROR_INCOMPATIBLE_DRIVER: VkResult = -9;
pub(crate) const VK_ERROR_INITIALIZATION_FAILED: VkResult = -3;
pub(crate) const VK_ERROR_INVALID_SHADER_NV: VkResult = -1000012000;
pub(crate) const VK_ERROR_LAYER_NOT_PRESENT: VkResult = -6;
pub(crate) const VK_ERROR_NATIVE_WINDOW_IN_USE_KHR: VkResult = -1000000001;
pub(crate) const VK_ERROR_OUT_OF_HOST_MEMORY: VkResult = -1;
pub(crate) const VK_ERROR_OUT_OF_POOL_MEMORY: VkResult = -1000069000;
pub(crate) const VK_ERROR_TOO_MANY_OBJECTS: VkResult = -10;
pub(crate) const VK_FORMAT_A8_UNORM_KHR: VkFormat = 1000470001;
#[allow(non_upper_case_globals)]
pub(crate) const VK_FORMAT_ASTC_10x10_SFLOAT_BLOCK_EXT: VkFormat = 1000066011;
#[allow(non_upper_case_globals)]
pub(crate) const VK_FORMAT_ASTC_10x10_SRGB_BLOCK: VkFormat = 180;
#[allow(non_upper_case_globals)]
pub(crate) const VK_FORMAT_ASTC_10x10_UNORM_BLOCK: VkFormat = 179;
#[allow(non_upper_case_globals)]
pub(crate) const VK_FORMAT_ASTC_10x5_SFLOAT_BLOCK_EXT: VkFormat = 1000066008;
#[allow(non_upper_case_globals)]
pub(crate) const VK_FORMAT_ASTC_10x5_SRGB_BLOCK: VkFormat = 174;
#[allow(non_upper_case_globals)]
pub(crate) const VK_FORMAT_ASTC_10x5_UNORM_BLOCK: VkFormat = 173;
#[allow(non_upper_case_globals)]
pub(crate) const VK_FORMAT_ASTC_10x6_SFLOAT_BLOCK_EXT: VkFormat = 1000066009;
#[allow(non_upper_case_globals)]
pub(crate) const VK_FORMAT_ASTC_10x6_SRGB_BLOCK: VkFormat = 176;
#[allow(non_upper_case_globals)]
pub(crate) const VK_FORMAT_ASTC_10x6_UNORM_BLOCK: VkFormat = 175;
#[allow(non_upper_case_globals)]
pub(crate) const VK_FORMAT_ASTC_10x8_SFLOAT_BLOCK_EXT: VkFormat = 1000066010;
#[allow(non_upper_case_globals)]
pub(crate) const VK_FORMAT_ASTC_10x8_SRGB_BLOCK: VkFormat = 178;
#[allow(non_upper_case_globals)]
pub(crate) const VK_FORMAT_ASTC_10x8_UNORM_BLOCK: VkFormat = 177;
#[allow(non_upper_case_globals)]
pub(crate) const VK_FORMAT_ASTC_12x10_SFLOAT_BLOCK_EXT: VkFormat = 1000066012;
#[allow(non_upper_case_globals)]
pub(crate) const VK_FORMAT_ASTC_12x10_SRGB_BLOCK: VkFormat = 182;
#[allow(non_upper_case_globals)]
pub(crate) const VK_FORMAT_ASTC_12x10_UNORM_BLOCK: VkFormat = 181;
#[allow(non_upper_case_globals)]
pub(crate) const VK_FORMAT_ASTC_12x12_SFLOAT_BLOCK: VkFormat = 1000066013;
#[allow(non_upper_case_globals)]
pub(crate) const VK_FORMAT_ASTC_12x12_SRGB_BLOCK: VkFormat = 184;
#[allow(non_upper_case_globals)]
pub(crate) const VK_FORMAT_ASTC_12x12_UNORM_BLOCK: VkFormat = 183;
#[allow(non_upper_case_globals)]
pub(crate) const VK_FORMAT_ASTC_4x4_SFLOAT_BLOCK_EXT: VkFormat = 1000066000;
#[allow(non_upper_case_globals)]
pub(crate) const VK_FORMAT_ASTC_4x4_SRGB_BLOCK: VkFormat = 158;
#[allow(non_upper_case_globals)]
pub(crate) const VK_FORMAT_ASTC_4x4_UNORM_BLOCK: VkFormat = 157;
#[allow(non_upper_case_globals)]
pub(crate) const VK_FORMAT_ASTC_5x4_SFLOAT_BLOCK_EXT: VkFormat = 1000066001;
#[allow(non_upper_case_globals)]
pub(crate) const VK_FORMAT_ASTC_5x4_SRGB_BLOCK: VkFormat = 160;
#[allow(non_upper_case_globals)]
pub(crate) const VK_FORMAT_ASTC_5x4_UNORM_BLOCK: VkFormat = 159;
#[allow(non_upper_case_globals)]
pub(crate) const VK_FORMAT_ASTC_5x5_SFLOAT_BLOCK_EXT: VkFormat = 1000066002;
#[allow(non_upper_case_globals)]
pub(crate) const VK_FORMAT_ASTC_5x5_SRGB_BLOCK: VkFormat = 162;
#[allow(non_upper_case_globals)]
pub(crate) const VK_FORMAT_ASTC_5x5_UNORM_BLOCK: VkFormat = 161;
#[allow(non_upper_case_globals)]
pub(crate) const VK_FORMAT_ASTC_6x5_SFLOAT_BLOCK_EXT: VkFormat = 1000066003;
#[allow(non_upper_case_globals)]
pub(crate) const VK_FORMAT_ASTC_6x5_SRGB_BLOCK: VkFormat = 164;
#[allow(non_upper_case_globals)]
pub(crate) const VK_FORMAT_ASTC_6x5_UNORM_BLOCK: VkFormat = 163;
#[allow(non_upper_case_globals)]
pub(crate) const VK_FORMAT_ASTC_6x6_SFLOAT_BLOCK_EXT: VkFormat = 1000066004;
#[allow(non_upper_case_globals)]
pub(crate) const VK_FORMAT_ASTC_6x6_SRGB_BLOCK: VkFormat = 166;
#[allow(non_upper_case_globals)]
pub(crate) const VK_FORMAT_ASTC_6x6_UNORM_BLOCK: VkFormat = 165;
#[allow(non_upper_case_globals)]
pub(crate) const VK_FORMAT_ASTC_8x5_SFLOAT_BLOCK_EXT: VkFormat = 1000066005;
#[allow(non_upper_case_globals)]
pub(crate) const VK_FORMAT_ASTC_8x5_SRGB_BLOCK: VkFormat = 168;
#[allow(non_upper_case_globals)]
pub(crate) const VK_FORMAT_ASTC_8x5_UNORM_BLOCK: VkFormat = 167;
#[allow(non_upper_case_globals)]
pub(crate) const VK_FORMAT_ASTC_8x6_SFLOAT_BLOCK_EXT: VkFormat = 1000066006;
#[allow(non_upper_case_globals)]
pub(crate) const VK_FORMAT_ASTC_8x6_SRGB_BLOCK: VkFormat = 170;
#[allow(non_upper_case_globals)]
pub(crate) const VK_FORMAT_ASTC_8x6_UNORM_BLOCK: VkFormat = 169;
#[allow(non_upper_case_globals)]
pub(crate) const VK_FORMAT_ASTC_8x8_SFLOAT_BLOCK_EXT: VkFormat = 1000066007;
#[allow(non_upper_case_globals)]
pub(crate) const VK_FORMAT_ASTC_8x8_SRGB_BLOCK: VkFormat = 172;
#[allow(non_upper_case_globals)]
pub(crate) const VK_FORMAT_ASTC_8x8_UNORM_BLOCK: VkFormat = 171;
pub(crate) const VK_FORMAT_B10G11R11_UFLOAT_PACK32: VkFormat = 122;
pub(crate) const VK_FORMAT_BC1_RGBA_SRGB_BLOCK: VkFormat = 134;
pub(crate) const VK_FORMAT_BC1_RGBA_UNORM_BLOCK: VkFormat = 133;
pub(crate) const VK_FORMAT_BC2_SRGB_BLOCK: VkFormat = 136;
pub(crate) const VK_FORMAT_BC2_UNORM_BLOCK: VkFormat = 135;
pub(crate) const VK_FORMAT_BC3_SRGB_BLOCK: VkFormat = 138;
pub(crate) const VK_FORMAT_BC3_UNORM_BLOCK: VkFormat = 137;
pub(crate) const VK_FORMAT_BC4_UNORM_BLOCK: VkFormat = 139;
pub(crate) const VK_FORMAT_BC5_UNORM_BLOCK: VkFormat = 141;
pub(crate) const VK_FORMAT_BC6H_SFLOAT_BLOCK: VkFormat = 144;
pub(crate) const VK_FORMAT_BC6H_UFLOAT_BLOCK: VkFormat = 143;
pub(crate) const VK_FORMAT_BC7_SRGB_BLOCK: VkFormat = 146;
pub(crate) const VK_FORMAT_BC7_UNORM_BLOCK: VkFormat = 145;
pub(crate) const VK_FORMAT_D16_UNORM: VkFormat = 124;
pub(crate) const VK_FORMAT_D24_UNORM_S8_UINT: VkFormat = 129;
pub(crate) const VK_FORMAT_D32_SFLOAT: VkFormat = 126;
pub(crate) const VK_FORMAT_D32_SFLOAT_S8_UINT: VkFormat = 130;
pub(crate) const VK_FORMAT_R16G16B16A16_SINT: VkFormat = 96;
pub(crate) const VK_FORMAT_R16G16B16A16_SNORM: VkFormat = 92;
pub(crate) const VK_FORMAT_R16G16B16A16_UINT: VkFormat = 95;
pub(crate) const VK_FORMAT_R16G16B16A16_UNORM: VkFormat = 91;
pub(crate) const VK_FORMAT_R16G16_SFLOAT: VkFormat = 83;
pub(crate) const VK_FORMAT_R16G16_SINT: VkFormat = 82;
pub(crate) const VK_FORMAT_R16G16_SNORM: VkFormat = 78;
pub(crate) const VK_FORMAT_R16G16_UINT: VkFormat = 81;
pub(crate) const VK_FORMAT_R16_SFLOAT: VkFormat = 76;
pub(crate) const VK_FORMAT_R16_SINT: VkFormat = 75;
pub(crate) const VK_FORMAT_R16_SNORM: VkFormat = 71;
pub(crate) const VK_FORMAT_R16_UINT: VkFormat = 74;
pub(crate) const VK_FORMAT_R32G32B32A32_SINT: VkFormat = 108;
pub(crate) const VK_FORMAT_R32G32B32A32_UINT: VkFormat = 107;
pub(crate) const VK_FORMAT_R32G32B32_SFLOAT: VkFormat = 106;
pub(crate) const VK_FORMAT_R32G32B32_SINT: VkFormat = 105;
pub(crate) const VK_FORMAT_R32G32B32_UINT: VkFormat = 104;
pub(crate) const VK_FORMAT_R32G32_SINT: VkFormat = 102;
pub(crate) const VK_FORMAT_R32G32_UINT: VkFormat = 101;
pub(crate) const VK_FORMAT_R32_SFLOAT: VkFormat = 100;
pub(crate) const VK_FORMAT_R32_SINT: VkFormat = 99;
pub(crate) const VK_FORMAT_R32_UINT: VkFormat = 98;
pub(crate) const VK_FORMAT_R8G8B8A8_SINT: VkFormat = 42;
pub(crate) const VK_FORMAT_R8G8B8A8_SNORM: VkFormat = 38;
pub(crate) const VK_FORMAT_R8G8B8A8_UINT: VkFormat = 41;
pub(crate) const VK_FORMAT_R8G8_SINT: VkFormat = 21;
pub(crate) const VK_FORMAT_R8G8_SNORM: VkFormat = 17;
pub(crate) const VK_FORMAT_R8G8_UINT: VkFormat = 20;
pub(crate) const VK_FORMAT_R8_SINT: VkFormat = 14;
pub(crate) const VK_FORMAT_R8_SNORM: VkFormat = 10;
pub(crate) const VK_FORMAT_R8_UINT: VkFormat = 13;
pub(crate) const VK_FORMAT_X8_D24_UNORM_PACK32: VkFormat = 125;
pub(crate) const VK_FRONT_FACE_CLOCKWISE: i32 = 1;
pub(crate) const VK_IMAGE_ASPECT_DEPTH_BIT: VkImageAspectFlags = 0x00000002;
pub(crate) const VK_IMAGE_ASPECT_STENCIL_BIT: VkImageAspectFlags = 0x00000004;
pub(crate) const VK_IMAGE_CREATE_2D_ARRAY_COMPATIBLE_BIT: VkFlags = 0x00000020;
pub(crate) const VK_IMAGE_CREATE_CUBE_COMPATIBLE_BIT: VkFlags = 0x00000010;
pub(crate) const VK_IMAGE_LAYOUT_DEPTH_STENCIL_ATTACHMENT_OPTIMAL: VkImageLayout = 3;
pub(crate) const VK_IMAGE_LAYOUT_GENERAL: VkImageLayout = 1;
pub(crate) const VK_IMAGE_TYPE_3D: i32 = 2;
pub(crate) const VK_IMAGE_USAGE_DEPTH_STENCIL_ATTACHMENT_BIT: VkImageUsageFlags = 0x00000020;
pub(crate) const VK_IMAGE_USAGE_STORAGE_BIT: VkImageUsageFlags = 0x00000008;
pub(crate) const VK_IMAGE_VIEW_TYPE_2D_ARRAY: i32 = 5;
pub(crate) const VK_IMAGE_VIEW_TYPE_3D: i32 = 2;
pub(crate) const VK_IMAGE_VIEW_TYPE_CUBE: i32 = 3;
pub(crate) const VK_IMAGE_VIEW_TYPE_CUBE_ARRAY: i32 = 6;
pub(crate) const VK_INCOMPLETE: VkResult = 5;
pub(crate) const VK_INDEX_TYPE_UINT16: i32 = 0;
pub(crate) const VK_INDEX_TYPE_UINT32: i32 = 1;
pub(crate) const VK_INSTANCE_CREATE_ENUMERATE_PORTABILITY_BIT_KHR: VkFlags = 0x00000001;
pub(crate) const VK_LAYERED_DRIVER_UNDERLYING_API_NONE_MSFT: i32 = 0;
pub(crate) const VK_MAX_DRIVER_INFO_SIZE: usize = 256;
pub(crate) const VK_MAX_DRIVER_NAME_SIZE: usize = 256;
pub(crate) const VK_MEMORY_HEAP_DEVICE_LOCAL_BIT: VkFlags = 0x00000001;
pub(crate) const VK_MEMORY_PROPERTY_HOST_CACHED_BIT: VkMemoryPropertyFlags = 0x00000008;
pub(crate) const VK_OBJECT_TYPE_BUFFER: i32 = 9;
pub(crate) const VK_OBJECT_TYPE_IMAGE: i32 = 10;
pub(crate) const VK_OBJECT_TYPE_PIPELINE: i32 = 19;
pub(crate) const VK_OBJECT_TYPE_SAMPLER: i32 = 21;
pub(crate) const VK_OBJECT_TYPE_SHADER_MODULE: i32 = 15;
pub(crate) const VK_PHYSICAL_DEVICE_TYPE_CPU: i32 = 4;
pub(crate) const VK_PHYSICAL_DEVICE_TYPE_DISCRETE_GPU: i32 = 2;
pub(crate) const VK_PHYSICAL_DEVICE_TYPE_INTEGRATED_GPU: i32 = 1;
pub(crate) const VK_PHYSICAL_DEVICE_TYPE_OTHER: i32 = 0;
pub(crate) const VK_PHYSICAL_DEVICE_TYPE_VIRTUAL_GPU: i32 = 3;
pub(crate) const VK_PIPELINE_STAGE_BOTTOM_OF_PIPE_BIT: VkPipelineStageFlags = 0x00002000;
pub(crate) const VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT: VkPipelineStageFlags = 0x00000800;
pub(crate) const VK_PIPELINE_STAGE_DRAW_INDIRECT_BIT: VkPipelineStageFlags = 0x00000002;
pub(crate) const VK_PIPELINE_STAGE_EARLY_FRAGMENT_TESTS_BIT: VkPipelineStageFlags = 0x00000100;
pub(crate) const VK_PIPELINE_STAGE_LATE_FRAGMENT_TESTS_BIT: VkPipelineStageFlags = 0x00000200;
pub(crate) const VK_PIPELINE_STAGE_VERTEX_INPUT_BIT: VkPipelineStageFlags = 0x00000004;
pub(crate) const VK_PIPELINE_STAGE_VERTEX_SHADER_BIT: VkPipelineStageFlags = 0x00000008;
pub(crate) const VK_POLYGON_MODE_LINE: i32 = 1;
pub(crate) const VK_PRIMITIVE_TOPOLOGY_TRIANGLE_STRIP: VkPrimitiveTopology = 4;
pub(crate) const VK_QUEUE_COMPUTE_BIT: VkFlags = 0x00000002;
pub(crate) const VK_QUEUE_TRANSFER_BIT: VkFlags = 0x00000004;
pub(crate) const VK_REMAINING_ARRAY_LAYERS: u32 = !0;
pub(crate) const VK_SAMPLER_ADDRESS_MODE_MIRRORED_REPEAT: i32 = 1;
pub(crate) const VK_SAMPLER_MIPMAP_MODE_LINEAR: i32 = 1;
pub(crate) const VK_SAMPLE_COUNT_2_BIT: VkFlags = 0x00000002;
pub(crate) const VK_SAMPLE_COUNT_4_BIT: VkFlags = 0x00000004;
pub(crate) const VK_SAMPLE_COUNT_8_BIT: VkFlags = 0x00000008;
pub(crate) const VK_SHADER_STAGE_COMPUTE_BIT: VkFlags = 0x00000020;
pub(crate) const VK_STENCIL_OP_DECREMENT_AND_CLAMP: i32 = 4;
pub(crate) const VK_STENCIL_OP_DECREMENT_AND_WRAP: i32 = 7;
pub(crate) const VK_STENCIL_OP_INCREMENT_AND_CLAMP: i32 = 3;
pub(crate) const VK_STENCIL_OP_INCREMENT_AND_WRAP: i32 = 6;
pub(crate) const VK_STENCIL_OP_INVERT: i32 = 5;
pub(crate) const VK_STENCIL_OP_KEEP: i32 = 0;
pub(crate) const VK_STENCIL_OP_REPLACE: i32 = 2;
pub(crate) const VK_STENCIL_OP_ZERO: i32 = 1;
pub(crate) const VK_STRUCTURE_TYPE_BUFFER_MEMORY_BARRIER: VkStructureType = 44;
pub(crate) const VK_STRUCTURE_TYPE_COMPUTE_PIPELINE_CREATE_INFO: VkStructureType = 29;
pub(crate) const VK_STRUCTURE_TYPE_DEBUG_UTILS_LABEL_EXT: VkStructureType = 1000128002;
pub(crate) const VK_STRUCTURE_TYPE_DEBUG_UTILS_OBJECT_NAME_INFO_EXT: VkStructureType = 1000128000;
pub(crate) const VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_16BIT_STORAGE_FEATURES: VkStructureType =
    1000083000;
pub(crate) const VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_DRIVER_PROPERTIES: VkStructureType = 1000196000;
pub(crate) const VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_FEATURES_2: VkStructureType = 1000059000;
pub(crate) const VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_LAYERED_DRIVER_PROPERTIES_MSFT: VkStructureType =
    1000530000;
pub(crate) const VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_MULTIVIEW_FEATURES: VkStructureType = 1000053001;
pub(crate) const VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_PORTABILITY_SUBSET_FEATURES_KHR:
    VkStructureType = 1000163000;
pub(crate) const VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_PROPERTIES_2: VkStructureType = 1000059001;
pub(crate) const VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_PROTECTED_MEMORY_FEATURES: VkStructureType =
    1000145001;
pub(crate) const VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_SHADER_DRAW_PARAMETERS_FEATURES:
    VkStructureType = 1000063000;
pub(crate) const VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_VARIABLE_POINTERS_FEATURES: VkStructureType =
    1000120000;
pub(crate) const VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_VULKAN_1_1_FEATURES: VkStructureType = 49;
pub(crate) const VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_VULKAN_1_2_FEATURES: VkStructureType = 51;
pub(crate) const VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_VULKAN_1_3_FEATURES: VkStructureType = 53;
pub(crate) const VK_VERTEX_INPUT_RATE_INSTANCE: i32 = 1;
pub(crate) const VK_WHOLE_SIZE: VkDeviceSize = !0;
pub(crate) const VK_NOT_READY: VkResult = 1;
pub(crate) const VK_ATTACHMENT_UNUSED: u32 = !0;
pub(crate) const VK_STENCIL_FACE_FRONT_AND_BACK: VkFlags = 0x00000003;
pub(crate) const VK_PIPELINE_BIND_POINT_COMPUTE: i32 = 1;
pub(crate) const VK_COMMAND_BUFFER_RESET_RELEASE_RESOURCES_BIT: VkFlags = 0x00000001;
pub(crate) const VK_COMPOSITE_ALPHA_PRE_MULTIPLIED_BIT_KHR: VkFlags = 0x00000002;
pub(crate) const VK_COMPOSITE_ALPHA_POST_MULTIPLIED_BIT_KHR: VkFlags = 0x00000004;
