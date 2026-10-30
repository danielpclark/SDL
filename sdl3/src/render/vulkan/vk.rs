// The VULKAN_FUNCTIONS() table of src/render/vulkan/SDL_render_vulkan.c from
// Simple DirectMedia Layer.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! The function tables of the renderer, over the Vulkan declarations it
//! shares with the GPU backend ([`crate::video::vk`]).

use std::ffi::{c_char, c_void};

pub(super) use crate::video::vk::*;

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

    /// The `VULKAN_DEVICE_FUNCTION`s, and the optional ones (the sampler
    /// YCbCr conversion's, and Android's hardware buffers' (looked up
    /// everywhere, Android's only there): the Android parts use them).
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
        [optional] {
            #[cfg_attr(not(target_os = "android"), allow(dead_code))]
            create_sampler_ycbcr_conversion_khr = c"vkCreateSamplerYcbcrConversionKHR":
                fn(
                    VkDevice,
                    *const VkSamplerYcbcrConversionCreateInfo,
                    Alloc,
                    *mut VkSamplerYcbcrConversion,
                ) -> VkResult;
            destroy_sampler_ycbcr_conversion_khr = c"vkDestroySamplerYcbcrConversionKHR":
                fn(VkDevice, VkSamplerYcbcrConversion, Alloc);
            #[cfg_attr(not(target_os = "android"), allow(dead_code))]
            get_android_hardware_buffer_properties_android =
                c"vkGetAndroidHardwareBufferPropertiesANDROID":
                fn(VkDevice, *const c_void, *mut VkAndroidHardwareBufferPropertiesANDROID)
                    -> VkResult;
        }
    }
}
