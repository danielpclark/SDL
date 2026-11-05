// The test runtime's Vulkan objects: the instance and device it makes for
// the application, and its swapchains' images.
// Part of the repository's own test code (not a translation); see LICENSE.txt.

//! Every Vulkan function comes through the application's
//! `vkGetInstanceProcAddr` (`XrVulkanInstanceCreateInfoKHR::pfnGetInstanceProcAddr`,
//! `XrVulkanDeviceCreateInfoKHR::pfnGetInstanceProcAddr`) and the
//! device's `vkGetDeviceProcAddr` found through it: the runtime loads no
//! library.

use std::ffi::{c_void, CStr};
use std::ptr::{null, null_mut};

use crate::sys::*;

/// A function pointer as another function pointer type.
///
/// # Safety
///
/// `F` is a function pointer type, the function's.
unsafe fn cast<F: Copy>(f: PfnVkVoidFunction) -> F {
    const { assert!(std::mem::size_of::<F>() == std::mem::size_of::<PfnVkVoidFunction>()) };
    // SAFETY: the caller's.
    unsafe { std::mem::transmute_copy(&f) }
}

/// `vkGetInstanceProcAddr(instance, name)` as `F`.
///
/// # Safety
///
/// `gipa` is a `vkGetInstanceProcAddr`, `instance` null or one of its
/// instances, `F` the function's type.
pub unsafe fn instance_fn<F: Copy>(
    gipa: PfnVkGetInstanceProcAddr,
    instance: VkInstance,
    name: &CStr,
) -> Option<F> {
    // SAFETY: the caller's; a NUL-terminated name.
    unsafe { gipa(instance, name.as_ptr()).map(|f| cast(f)) }
}

/// `vkCreateInstance()` with the application's create info and allocator.
///
/// # Safety
///
/// The arguments are valid for `vkCreateInstance()`.
pub unsafe fn create_instance(
    gipa: PfnVkGetInstanceProcAddr,
    create_info: *const c_void,
    allocator: *const c_void,
    instance: &mut VkInstance,
) -> Option<VkResult> {
    // SAFETY: the caller's.
    unsafe {
        let create: PfnVkCreateInstance = instance_fn(gipa, null_mut(), c"vkCreateInstance")?;
        Some(create(create_info, allocator, instance))
    }
}

/// The first physical device of an instance (the runtime's choice:
/// `vkEnumeratePhysicalDevices()`'s order, so a single driver, as
/// `VK_ICD_FILENAMES` can make it, decides).
///
/// # Safety
///
/// `instance` is an instance of `gipa`'s.
pub unsafe fn first_physical_device(
    gipa: PfnVkGetInstanceProcAddr,
    instance: VkInstance,
) -> Option<VkPhysicalDevice> {
    // SAFETY: the caller's; the two-call idiom with room for the count.
    unsafe {
        let enumerate: PfnVkEnumeratePhysicalDevices =
            instance_fn(gipa, instance, c"vkEnumeratePhysicalDevices")?;
        let mut count = 0;
        if enumerate(instance, &mut count, null_mut()) != VK_SUCCESS || count == 0 {
            return None;
        }
        let mut devices = vec![null_mut(); count as usize];
        let result = enumerate(instance, &mut count, devices.as_mut_ptr());
        // (VK_INCOMPLETE is fine: the first is enough)
        if result < VK_SUCCESS || count == 0 {
            return None;
        }
        Some(devices[0])
    }
}

/// `vkCreateDevice()` with the application's create info and allocator.
///
/// # Safety
///
/// The arguments are valid for `vkCreateDevice()`, `physical_device` one
/// of `instance`'s.
pub unsafe fn create_device(
    gipa: PfnVkGetInstanceProcAddr,
    instance: VkInstance,
    physical_device: VkPhysicalDevice,
    create_info: *const c_void,
    allocator: *const c_void,
    device: &mut VkDevice,
) -> Option<VkResult> {
    // SAFETY: the caller's.
    unsafe {
        let create: PfnVkCreateDevice = instance_fn(gipa, instance, c"vkCreateDevice")?;
        Some(create(physical_device, create_info, allocator, device))
    }
}

/// The device functions a session's swapchains need.
#[derive(Clone, Copy)]
pub struct DeviceFns {
    /// (`VkDevice`, as an address: the state is shared between threads)
    device: usize,
    memory_types: [VkMemoryType; 32],
    memory_type_count: u32,
    create_image: PfnVkCreateImage,
    destroy_image: PfnVkDestroyImage,
    get_image_memory_requirements: PfnVkGetImageMemoryRequirements,
    allocate_memory: PfnVkAllocateMemory,
    free_memory: PfnVkFreeMemory,
    bind_image_memory: PfnVkBindImageMemory,
}

impl DeviceFns {
    /// The functions of `device` (through `vkGetDeviceProcAddr`) and the
    /// memory types of its physical device.
    ///
    /// # Safety
    ///
    /// `device` is a device of `physical_device`, an instance's of `gipa`'s.
    pub unsafe fn load(
        gipa: PfnVkGetInstanceProcAddr,
        instance: VkInstance,
        physical_device: VkPhysicalDevice,
        device: VkDevice,
    ) -> Option<DeviceFns> {
        // SAFETY: the caller's; the functions of their names' types.
        unsafe {
            let gdpa: PfnVkGetDeviceProcAddr = instance_fn(gipa, instance, c"vkGetDeviceProcAddr")?;
            let memory_properties: PfnVkGetPhysicalDeviceMemoryProperties =
                instance_fn(gipa, instance, c"vkGetPhysicalDeviceMemoryProperties")?;
            let mut properties: VkPhysicalDeviceMemoryProperties = std::mem::zeroed();
            memory_properties(physical_device, &mut properties);
            macro_rules! device_fn {
                ($name:expr) => {
                    cast(gdpa(device, $name.as_ptr())?)
                };
            }
            Some(DeviceFns {
                device: device as usize,
                memory_types: properties.memory_types,
                memory_type_count: properties.memory_type_count.min(32),
                create_image: device_fn!(c"vkCreateImage"),
                destroy_image: device_fn!(c"vkDestroyImage"),
                get_image_memory_requirements: device_fn!(c"vkGetImageMemoryRequirements"),
                allocate_memory: device_fn!(c"vkAllocateMemory"),
                free_memory: device_fn!(c"vkFreeMemory"),
                bind_image_memory: device_fn!(c"vkBindImageMemory"),
            })
        }
    }

    fn device(&self) -> VkDevice {
        self.device as VkDevice
    }

    /// A memory type for `bits`: the first device-local one, else the first.
    fn memory_type(&self, bits: u32) -> Option<u32> {
        let allowed = |i: u32| bits & (1 << i) != 0;
        let types = &self.memory_types[..self.memory_type_count as usize];
        (0..self.memory_type_count)
            .find(|&i| {
                allowed(i)
                    && types[i as usize].property_flags & VK_MEMORY_PROPERTY_DEVICE_LOCAL_BIT != 0
            })
            .or_else(|| (0..self.memory_type_count).find(|&i| allowed(i)))
    }

    /// An image (2D, optimal tiling, exclusive, undefined layout) with its
    /// own memory bound, or `None` (with nothing left).
    ///
    /// # Safety
    ///
    /// The device is alive; the create info's values are valid for it.
    pub unsafe fn create_image(
        &self,
        info: &VkImageCreateInfo,
    ) -> Option<(VkImage, VkDeviceMemory)> {
        let device = self.device();
        let mut image = 0;
        // SAFETY: the caller's; the image is destroyed on the failures after.
        unsafe {
            if (self.create_image)(device, info, null(), &mut image) != VK_SUCCESS {
                return None;
            }
            let mut requirements: VkMemoryRequirements = std::mem::zeroed();
            (self.get_image_memory_requirements)(device, image, &mut requirements);
            let Some(memory_type_index) = self.memory_type(requirements.memory_type_bits) else {
                (self.destroy_image)(device, image, null());
                return None;
            };
            let allocate_info = VkMemoryAllocateInfo {
                s_type: VK_STRUCTURE_TYPE_MEMORY_ALLOCATE_INFO,
                p_next: null(),
                allocation_size: requirements.size,
                memory_type_index,
            };
            let mut memory = 0;
            if (self.allocate_memory)(device, &allocate_info, null(), &mut memory) != VK_SUCCESS {
                (self.destroy_image)(device, image, null());
                return None;
            }
            if (self.bind_image_memory)(device, image, memory, 0) != VK_SUCCESS {
                (self.destroy_image)(device, image, null());
                (self.free_memory)(device, memory, null());
                return None;
            }
            Some((image, memory))
        }
    }

    /// Destroy an image of [`DeviceFns::create_image`] and free its memory.
    ///
    /// # Safety
    ///
    /// The device is alive, the image one of its, unused.
    pub unsafe fn destroy_image(&self, (image, memory): (VkImage, VkDeviceMemory)) {
        // SAFETY: the caller's.
        unsafe {
            (self.destroy_image)(self.device(), image, null());
            (self.free_memory)(self.device(), memory, null());
        }
    }
}

/// The Vulkan usage and create flags of OpenXR swapchain usage flags
/// (`XR_KHR_vulkan_enable2`'s table): exactly those asked for.
pub fn image_usage(usage: XrFlags64) -> (u32, u32) {
    let table = [
        (
            XR_SWAPCHAIN_USAGE_COLOR_ATTACHMENT_BIT,
            VK_IMAGE_USAGE_COLOR_ATTACHMENT_BIT,
        ),
        (
            XR_SWAPCHAIN_USAGE_DEPTH_STENCIL_ATTACHMENT_BIT,
            VK_IMAGE_USAGE_DEPTH_STENCIL_ATTACHMENT_BIT,
        ),
        (
            XR_SWAPCHAIN_USAGE_UNORDERED_ACCESS_BIT,
            VK_IMAGE_USAGE_STORAGE_BIT,
        ),
        (
            XR_SWAPCHAIN_USAGE_TRANSFER_SRC_BIT,
            VK_IMAGE_USAGE_TRANSFER_SRC_BIT,
        ),
        (
            XR_SWAPCHAIN_USAGE_TRANSFER_DST_BIT,
            VK_IMAGE_USAGE_TRANSFER_DST_BIT,
        ),
        (XR_SWAPCHAIN_USAGE_SAMPLED_BIT, VK_IMAGE_USAGE_SAMPLED_BIT),
    ];
    let vk_usage = table
        .iter()
        .filter(|(xr, _)| usage & xr != 0)
        .fold(0, |acc, (_, vk)| acc | vk);
    let flags = if usage & XR_SWAPCHAIN_USAGE_MUTABLE_FORMAT_BIT != 0 {
        VK_IMAGE_CREATE_MUTABLE_FORMAT_BIT
    } else {
        0
    };
    (vk_usage, flags)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn usage_is_exactly_the_asked_for() {
        assert_eq!(image_usage(0), (0, 0));
        assert_eq!(
            image_usage(
                XR_SWAPCHAIN_USAGE_COLOR_ATTACHMENT_BIT | XR_SWAPCHAIN_USAGE_TRANSFER_SRC_BIT
            ),
            (
                VK_IMAGE_USAGE_COLOR_ATTACHMENT_BIT | VK_IMAGE_USAGE_TRANSFER_SRC_BIT,
                0
            )
        );
        assert_eq!(
            image_usage(XR_SWAPCHAIN_USAGE_SAMPLED_BIT | XR_SWAPCHAIN_USAGE_MUTABLE_FORMAT_BIT),
            (
                VK_IMAGE_USAGE_SAMPLED_BIT,
                VK_IMAGE_CREATE_MUTABLE_FORMAT_BIT
            )
        );
        assert_eq!(
            image_usage(0x7f).0,
            VK_IMAGE_USAGE_COLOR_ATTACHMENT_BIT
                | VK_IMAGE_USAGE_DEPTH_STENCIL_ATTACHMENT_BIT
                | VK_IMAGE_USAGE_STORAGE_BIT
                | VK_IMAGE_USAGE_TRANSFER_SRC_BIT
                | VK_IMAGE_USAGE_TRANSFER_DST_BIT
                | VK_IMAGE_USAGE_SAMPLED_BIT
        );
    }
}
