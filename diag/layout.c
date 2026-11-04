/* diag only (never committed to a PR branch): the sizes, alignments and
 * offsets of the OpenXR structures SDL uses, from Khronos's headers, for
 * sdl3/src/gpu/xr/sys.rs's layout tests. Built with -I<openxr include>. */
#include <stdio.h>
#include <stddef.h>
#ifdef _WIN32
#include <windows.h>
#include <d3d12.h>
#define XR_USE_GRAPHICS_API_D3D12 1
#endif
#ifdef __ANDROID__
#include <jni.h>
#define XR_USE_PLATFORM_ANDROID 1
#endif
#include <vulkan/vulkan.h>
#define XR_USE_GRAPHICS_API_VULKAN 1
#include <openxr/openxr.h>
#include <openxr/openxr_platform.h>
#define S(t) printf("size %s %zu align %zu\n", #t, sizeof(t), _Alignof(t))
#define O(t, f) printf("off %s.%s %zu\n", #t, #f, offsetof(t, f))
#define V(x) printf("value %s %lld\n", #x, (long long)(x))
int main(void) {
  S(XrApiLayerProperties); O(XrApiLayerProperties, next); O(XrApiLayerProperties, layerName); O(XrApiLayerProperties, specVersion); O(XrApiLayerProperties, layerVersion); O(XrApiLayerProperties, description);
  S(XrExtensionProperties); O(XrExtensionProperties, extensionName); O(XrExtensionProperties, extensionVersion);
  S(XrApplicationInfo); O(XrApplicationInfo, applicationVersion); O(XrApplicationInfo, engineName); O(XrApplicationInfo, engineVersion); O(XrApplicationInfo, apiVersion);
  S(XrInstanceCreateInfo); O(XrInstanceCreateInfo, createFlags); O(XrInstanceCreateInfo, applicationInfo); O(XrInstanceCreateInfo, enabledApiLayerCount); O(XrInstanceCreateInfo, enabledApiLayerNames); O(XrInstanceCreateInfo, enabledExtensionCount); O(XrInstanceCreateInfo, enabledExtensionNames);
  S(XrSystemGetInfo); O(XrSystemGetInfo, formFactor);
  S(XrSessionCreateInfo); O(XrSessionCreateInfo, createFlags); O(XrSessionCreateInfo, systemId);
  S(XrSwapchainCreateInfo); O(XrSwapchainCreateInfo, createFlags); O(XrSwapchainCreateInfo, usageFlags); O(XrSwapchainCreateInfo, format); O(XrSwapchainCreateInfo, sampleCount); O(XrSwapchainCreateInfo, width); O(XrSwapchainCreateInfo, height); O(XrSwapchainCreateInfo, faceCount); O(XrSwapchainCreateInfo, arraySize); O(XrSwapchainCreateInfo, mipCount);
  S(XrSwapchainImageBaseHeader); O(XrSwapchainImageBaseHeader, next);
  S(XrGraphicsBindingVulkanKHR); O(XrGraphicsBindingVulkanKHR, instance); O(XrGraphicsBindingVulkanKHR, physicalDevice); O(XrGraphicsBindingVulkanKHR, device); O(XrGraphicsBindingVulkanKHR, queueFamilyIndex); O(XrGraphicsBindingVulkanKHR, queueIndex);
  S(XrSwapchainImageVulkanKHR); O(XrSwapchainImageVulkanKHR, image);
  S(XrGraphicsRequirementsVulkanKHR); O(XrGraphicsRequirementsVulkanKHR, minApiVersionSupported); O(XrGraphicsRequirementsVulkanKHR, maxApiVersionSupported);
  S(XrVulkanInstanceCreateInfoKHR); O(XrVulkanInstanceCreateInfoKHR, systemId); O(XrVulkanInstanceCreateInfoKHR, createFlags); O(XrVulkanInstanceCreateInfoKHR, pfnGetInstanceProcAddr); O(XrVulkanInstanceCreateInfoKHR, vulkanCreateInfo); O(XrVulkanInstanceCreateInfoKHR, vulkanAllocator);
  S(XrVulkanDeviceCreateInfoKHR); O(XrVulkanDeviceCreateInfoKHR, systemId); O(XrVulkanDeviceCreateInfoKHR, createFlags); O(XrVulkanDeviceCreateInfoKHR, pfnGetInstanceProcAddr); O(XrVulkanDeviceCreateInfoKHR, vulkanPhysicalDevice); O(XrVulkanDeviceCreateInfoKHR, vulkanCreateInfo); O(XrVulkanDeviceCreateInfoKHR, vulkanAllocator);
  S(XrVulkanGraphicsDeviceGetInfoKHR); O(XrVulkanGraphicsDeviceGetInfoKHR, systemId); O(XrVulkanGraphicsDeviceGetInfoKHR, vulkanInstance);
#ifdef _WIN32
  S(LUID); S(XrGraphicsBindingD3D12KHR); O(XrGraphicsBindingD3D12KHR, device); O(XrGraphicsBindingD3D12KHR, queue);
  S(XrSwapchainImageD3D12KHR); O(XrSwapchainImageD3D12KHR, texture);
  S(XrGraphicsRequirementsD3D12KHR); O(XrGraphicsRequirementsD3D12KHR, adapterLuid); O(XrGraphicsRequirementsD3D12KHR, minFeatureLevel);
  V(XR_TYPE_GRAPHICS_BINDING_D3D12_KHR); V(XR_TYPE_SWAPCHAIN_IMAGE_D3D12_KHR); V(XR_TYPE_GRAPHICS_REQUIREMENTS_D3D12_KHR);
#endif
#ifdef __ANDROID__
  S(XrLoaderInitInfoAndroidKHR); O(XrLoaderInitInfoAndroidKHR, applicationVM); O(XrLoaderInitInfoAndroidKHR, applicationContext);
  S(XrInstanceCreateInfoAndroidKHR); O(XrInstanceCreateInfoAndroidKHR, applicationVM); O(XrInstanceCreateInfoAndroidKHR, applicationActivity);
#endif
  V(XR_TYPE_API_LAYER_PROPERTIES); V(XR_TYPE_EXTENSION_PROPERTIES); V(XR_TYPE_INSTANCE_CREATE_INFO); V(XR_TYPE_SYSTEM_GET_INFO);
  V(XR_TYPE_SESSION_CREATE_INFO); V(XR_TYPE_SWAPCHAIN_CREATE_INFO);
  V(XR_TYPE_GRAPHICS_BINDING_VULKAN_KHR); V(XR_TYPE_SWAPCHAIN_IMAGE_VULKAN_KHR); V(XR_TYPE_GRAPHICS_REQUIREMENTS_VULKAN_KHR);
  V(XR_TYPE_VULKAN_INSTANCE_CREATE_INFO_KHR); V(XR_TYPE_VULKAN_DEVICE_CREATE_INFO_KHR); V(XR_TYPE_VULKAN_GRAPHICS_DEVICE_GET_INFO_KHR);
  V(XR_TYPE_LOADER_INIT_INFO_ANDROID_KHR); V(XR_TYPE_INSTANCE_CREATE_INFO_ANDROID_KHR);
  V(XR_MAX_API_LAYER_NAME_SIZE); V(XR_MAX_API_LAYER_DESCRIPTION_SIZE); V(XR_MAX_EXTENSION_NAME_SIZE); V(XR_MAX_APPLICATION_NAME_SIZE); V(XR_MAX_ENGINE_NAME_SIZE);
  V(XR_FORM_FACTOR_HEAD_MOUNTED_DISPLAY); V(XR_FORM_FACTOR_HANDHELD_DISPLAY);
  V(XR_SWAPCHAIN_USAGE_COLOR_ATTACHMENT_BIT); V(XR_SWAPCHAIN_USAGE_DEPTH_STENCIL_ATTACHMENT_BIT); V(XR_SWAPCHAIN_USAGE_TRANSFER_SRC_BIT); V(XR_SWAPCHAIN_USAGE_SAMPLED_BIT); V(XR_SWAPCHAIN_USAGE_UNORDERED_ACCESS_BIT);
  V(XR_SUCCESS); V(XR_ERROR_VALIDATION_FAILURE); V(XR_ERROR_RUNTIME_FAILURE); V(XR_ERROR_INITIALIZATION_FAILED); V(XR_ERROR_FUNCTION_UNSUPPORTED); V(XR_ERROR_EXTENSION_NOT_PRESENT); V(XR_ERROR_HANDLE_INVALID); V(XR_ERROR_FORM_FACTOR_UNSUPPORTED); V(XR_ERROR_FORM_FACTOR_UNAVAILABLE);
  return 0;
}
