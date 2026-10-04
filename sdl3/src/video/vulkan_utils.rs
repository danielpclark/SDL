// Rust translation of the platform-independent parts of
// src/video/SDL_vulkan_utils.c and SDL_vulkan_internal.h from Simple
// DirectMedia Layer.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! The Vulkan helpers the video drivers and the Vulkan renderer share:
//! the names of `VkResult`s, the instance extension list, destroying a
//! surface, and looking up instance functions through the loader's
//! `vkGetInstanceProcAddr`. The Vulkan calling convention is the system's
//! (`VKAPI_CALL` is `__stdcall` on 32-bit Windows).
//!
//! (The X11, Wayland and Windows drivers still have their own copies.)

use std::ffi::{c_char, c_void, CStr};

use crate::error::{Error, Result};

/// `VkResult`.
pub(crate) type VkResult = i32;
pub(crate) const VK_SUCCESS: VkResult = 0;
pub(crate) const VK_ERROR_INCOMPATIBLE_DRIVER: VkResult = -9;

/// `VK_MAX_EXTENSION_NAME_SIZE`
pub(crate) const VK_MAX_EXTENSION_NAME_SIZE: usize = 256;

/// `VK_KHR_SURFACE_EXTENSION_NAME`
pub(crate) const VK_KHR_SURFACE_EXTENSION_NAME: &str = "VK_KHR_surface";

/// `VkExtensionProperties`.
#[repr(C)]
#[derive(Clone, Copy)]
pub(crate) struct VkExtensionProperties {
    pub(crate) extension_name: [c_char; VK_MAX_EXTENSION_NAME_SIZE],
    pub(crate) spec_version: u32,
}

impl Default for VkExtensionProperties {
    fn default() -> VkExtensionProperties {
        VkExtensionProperties {
            extension_name: [0; VK_MAX_EXTENSION_NAME_SIZE],
            spec_version: 0,
        }
    }
}

impl VkExtensionProperties {
    /// The extension's name (`extensionName`), lossily as UTF-8.
    pub(crate) fn name(&self) -> String {
        let bytes: Vec<u8> = self
            .extension_name
            .iter()
            .take_while(|&&c| c != 0)
            .map(|&c| c as u8)
            .collect();
        String::from_utf8_lossy(&bytes).into_owned()
    }
}

/// `PFN_vkVoidFunction`.
pub(crate) type PfnVkVoidFunction = unsafe extern "system" fn();
/// `PFN_vkGetInstanceProcAddr`.
pub(crate) type PfnVkGetInstanceProcAddr =
    unsafe extern "system" fn(*mut c_void, *const c_char) -> Option<PfnVkVoidFunction>;
/// `PFN_vkEnumerateInstanceExtensionProperties`.
pub(crate) type PfnVkEnumerateInstanceExtensionProperties =
    unsafe extern "system" fn(*const c_char, *mut u32, *mut VkExtensionProperties) -> VkResult;
/// `PFN_vkDestroySurfaceKHR`.
type PfnVkDestroySurfaceKHR = unsafe extern "system" fn(*mut c_void, u64, *const c_void);

/// The name of a `VkResult`. Translation of `SDL_Vulkan_GetResultString()`.
pub(crate) fn vulkan_get_result_string(result: VkResult) -> &'static str {
    match result {
        0 => "VK_SUCCESS",
        1 => "VK_NOT_READY",
        2 => "VK_TIMEOUT",
        3 => "VK_EVENT_SET",
        4 => "VK_EVENT_RESET",
        5 => "VK_INCOMPLETE",
        -1 => "VK_ERROR_OUT_OF_HOST_MEMORY",
        -2 => "VK_ERROR_OUT_OF_DEVICE_MEMORY",
        -3 => "VK_ERROR_INITIALIZATION_FAILED",
        -4 => "VK_ERROR_DEVICE_LOST",
        -5 => "VK_ERROR_MEMORY_MAP_FAILED",
        -6 => "VK_ERROR_LAYER_NOT_PRESENT",
        -7 => "VK_ERROR_EXTENSION_NOT_PRESENT",
        -8 => "VK_ERROR_FEATURE_NOT_PRESENT",
        -9 => "VK_ERROR_INCOMPATIBLE_DRIVER",
        -10 => "VK_ERROR_TOO_MANY_OBJECTS",
        -11 => "VK_ERROR_FORMAT_NOT_SUPPORTED",
        -12 => "VK_ERROR_FRAGMENTED_POOL",
        -13 => "VK_ERROR_UNKNOWN",
        -1000069000 => "VK_ERROR_OUT_OF_POOL_MEMORY",
        -1000072003 => "VK_ERROR_INVALID_EXTERNAL_HANDLE",
        -1000161000 => "VK_ERROR_FRAGMENTATION",
        -1000257000 => "VK_ERROR_INVALID_OPAQUE_CAPTURE_ADDRESS",
        -1000000000 => "VK_ERROR_SURFACE_LOST_KHR",
        -1000000001 => "VK_ERROR_NATIVE_WINDOW_IN_USE_KHR",
        1000001003 => "VK_SUBOPTIMAL_KHR",
        -1000001004 => "VK_ERROR_OUT_OF_DATE_KHR",
        -1000003001 => "VK_ERROR_INCOMPATIBLE_DISPLAY_KHR",
        -1000011001 => "VK_ERROR_VALIDATION_FAILED_EXT",
        -1000012000 => "VK_ERROR_INVALID_SHADER_NV",
        // (VK_ERROR_INCOMPATIBLE_VERSION_KHR only exists in Vulkan headers
        // 135 to 161)
        -1000158000 => "VK_ERROR_INVALID_DRM_FORMAT_MODIFIER_PLANE_LAYOUT_EXT",
        -1000174001 => "VK_ERROR_NOT_PERMITTED_EXT",
        -1000255000 => "VK_ERROR_FULL_SCREEN_EXCLUSIVE_MODE_LOST_EXT",
        1000268000 => "VK_THREAD_IDLE_KHR",
        1000268001 => "VK_THREAD_DONE_KHR",
        1000268002 => "VK_OPERATION_DEFERRED_KHR",
        1000268003 => "VK_OPERATION_NOT_DEFERRED_KHR",
        1000297000 => "VK_PIPELINE_COMPILE_REQUIRED_EXT",
        r if r < 0 => "VK_ERROR_<Unknown>",
        _ => "VK_<Unknown>",
    }
}

/// The error of a failed `vkEnumerateInstanceExtensionProperties()`.
fn enumerate_error(rc: VkResult) -> Error {
    Error::new(format!(
        "Getting Vulkan extensions failed: vkEnumerateInstanceExtensionProperties returned {}({})",
        vulkan_get_result_string(rc),
        rc
    ))
}

/// The instance extensions. Translation of
/// `SDL_Vulkan_CreateInstanceExtensionsList()`.
pub(crate) fn vulkan_create_instance_extensions_list(
    vk_enumerate_instance_extension_properties: PfnVkEnumerateInstanceExtensionProperties,
) -> Result<Vec<VkExtensionProperties>> {
    let mut count: u32 = 0;
    // SAFETY: a NULL properties array queries the count.
    let rc = unsafe {
        vk_enumerate_instance_extension_properties(
            std::ptr::null(),
            &mut count,
            std::ptr::null_mut(),
        )
    };

    if rc == VK_ERROR_INCOMPATIBLE_DRIVER {
        // Avoid the ERR_MAX_STRLEN limit by passing part of the message as a string argument.
        return Err(Error::new(format!(
            "You probably don't have a working Vulkan driver installed. {} {} {}({})",
            "Getting Vulkan extensions failed:",
            "vkEnumerateInstanceExtensionProperties returned",
            vulkan_get_result_string(rc),
            rc
        )));
    } else if rc != VK_SUCCESS {
        return Err(enumerate_error(rc));
    }

    let mut result = vec![VkExtensionProperties::default(); count as usize];

    // SAFETY: the array has room for `count` properties.
    let rc = unsafe {
        vk_enumerate_instance_extension_properties(
            std::ptr::null(),
            &mut count,
            result.as_mut_ptr(),
        )
    };
    if rc != VK_SUCCESS {
        return Err(enumerate_error(rc));
    }
    result.truncate(count as usize);
    Ok(result)
}

/// Look up a function (`vkGetInstanceProcAddr(instance, name)`; a null
/// instance for the global functions).
///
/// # Safety
///
/// `get` is a loader's `vkGetInstanceProcAddr`, `instance` null or one of
/// its instances, and `F` the function's pointer type.
pub(crate) unsafe fn instance_function<F: Copy>(
    get: PfnVkGetInstanceProcAddr,
    instance: *mut c_void,
    name: &CStr,
) -> Option<F> {
    const { assert!(size_of::<F>() == size_of::<PfnVkVoidFunction>()) };
    // SAFETY: the caller's loader and instance; the name is NUL-terminated.
    let f = unsafe { get(instance, name.as_ptr()) }?;
    // SAFETY: the caller's contract: F is the function's pointer type.
    Some(unsafe { std::mem::transmute_copy::<PfnVkVoidFunction, F>(&f) })
}

/// Destroy a surface. Translation of `SDL_Vulkan_DestroySurface_Internal()`.
///
/// # Safety
///
/// `get` is a loader's `vkGetInstanceProcAddr`, `instance` one of its
/// instances, `surface` a surface of the instance and `allocator` null or
/// the allocation callbacks the surface was created with.
pub(crate) unsafe fn vulkan_destroy_surface_internal(
    get: PfnVkGetInstanceProcAddr,
    instance: *mut c_void,
    surface: u64,
    allocator: *const c_void,
) {
    // SAFETY: the caller's loader and instance; the function's type.
    let vk_destroy_surface_khr: Option<PfnVkDestroySurfaceKHR> =
        unsafe { instance_function(get, instance, c"vkDestroySurfaceKHR") };

    if let Some(vk_destroy_surface_khr) = vk_destroy_surface_khr {
        // SAFETY: the caller's instance, surface and allocator.
        unsafe { vk_destroy_surface_khr(instance, surface, allocator) };
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn result_strings() {
        assert_eq!(vulkan_get_result_string(0), "VK_SUCCESS");
        assert_eq!(vulkan_get_result_string(-4), "VK_ERROR_DEVICE_LOST");
        assert_eq!(
            vulkan_get_result_string(-1000001004),
            "VK_ERROR_OUT_OF_DATE_KHR"
        );
        assert_eq!(vulkan_get_result_string(1000001003), "VK_SUBOPTIMAL_KHR");
        assert_eq!(vulkan_get_result_string(-424242), "VK_ERROR_<Unknown>");
        assert_eq!(vulkan_get_result_string(424242), "VK_<Unknown>");
    }

    #[test]
    fn extension_names() {
        let mut e = VkExtensionProperties::default();
        assert_eq!(e.name(), "");
        for (d, s) in e.extension_name.iter_mut().zip(b"VK_KHR_surface") {
            *d = *s as c_char;
        }
        assert_eq!(e.name(), VK_KHR_SURFACE_EXTENSION_NAME);
    }
}
