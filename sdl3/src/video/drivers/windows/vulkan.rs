// Rust translation of src/video/windows/SDL_windowsvulkan.c (and the three
// helpers it uses from src/video/SDL_vulkan_utils.c) from Simple DirectMedia Layer.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! Vulkan surfaces for Windows windows (`VK_KHR_win32_surface`); the
//! loader (`vulkan-1.dll`) is loaded at run time.
//
// @author Mark Callow, www.edgewise-consulting.com. Based on Jacob Lifshay's
// SDL_x11vulkan.c.

use std::ffi::{c_char, c_void, CStr};
use std::sync::Mutex;

use windows_sys::Win32::Foundation::{HINSTANCE, HWND};

use super::window::window_data;
use crate::error::{Error, Result};
use crate::events::WindowID;
use crate::hints;
use crate::loadso::SharedObject;

type VkResult = i32;
type PfnVkVoidFunction = Option<unsafe extern "system" fn()>;
type PfnVkGetInstanceProcAddr =
    unsafe extern "system" fn(instance: usize, name: *const c_char) -> PfnVkVoidFunction;
type PfnVkEnumerateInstanceExtensionProperties = unsafe extern "system" fn(
    layer_name: *const c_char,
    count: *mut u32,
    properties: *mut VkExtensionProperties,
) -> VkResult;
type PfnVkCreateWin32SurfaceKHR = unsafe extern "system" fn(
    instance: usize,
    create_info: *const VkWin32SurfaceCreateInfoKHR,
    allocator: *const c_void,
    surface: *mut u64,
) -> VkResult;
type PfnVkDestroySurfaceKHR =
    unsafe extern "system" fn(instance: usize, surface: u64, allocator: *const c_void);
type PfnVkGetPhysicalDeviceWin32PresentationSupportKHR =
    unsafe extern "system" fn(physical_device: usize, queue_family_index: u32) -> u32;

const VK_SUCCESS: VkResult = 0;
const VK_ERROR_INCOMPATIBLE_DRIVER: VkResult = -9;
const VK_STRUCTURE_TYPE_WIN32_SURFACE_CREATE_INFO_KHR: i32 = 1000009000;
const VK_MAX_EXTENSION_NAME_SIZE: usize = 256;
const VK_KHR_SURFACE_EXTENSION_NAME: &str = "VK_KHR_surface";
const VK_KHR_WIN32_SURFACE_EXTENSION_NAME: &str = "VK_KHR_win32_surface";

/// `VkExtensionProperties`
#[repr(C)]
#[derive(Clone, Copy)]
struct VkExtensionProperties {
    extension_name: [c_char; VK_MAX_EXTENSION_NAME_SIZE],
    spec_version: u32,
}

/// `VkWin32SurfaceCreateInfoKHR`
#[repr(C)]
struct VkWin32SurfaceCreateInfoKHR {
    s_type: i32,
    p_next: *const c_void,
    flags: u32,
    hinstance: HINSTANCE,
    hwnd: HWND,
}

/// The driver's `_this->vulkan_config`: the loader and its
/// `vkGetInstanceProcAddr`.
struct VulkanConfig {
    loader_handle: SharedObject,
    vk_get_instance_proc_addr: PfnVkGetInstanceProcAddr,
}

static VULKAN_CONFIG: Mutex<Option<VulkanConfig>> = Mutex::new(None);

fn config() -> std::sync::MutexGuard<'static, Option<VulkanConfig>> {
    VULKAN_CONFIG.lock().unwrap_or_else(|e| e.into_inner())
}

/// Translation of `SDL_Vulkan_GetResultString()`.
fn vulkan_get_result_string(result: VkResult) -> &'static str {
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
        -1000158000 => "VK_ERROR_INVALID_DRM_FORMAT_MODIFIER_PLANE_LAYOUT_EXT",
        -1000174001 => "VK_ERROR_NOT_PERMITTED_EXT",
        -1000255000 => "VK_ERROR_FULL_SCREEN_EXCLUSIVE_MODE_LOST_EXT",
        1000268000 => "VK_THREAD_IDLE_KHR",
        1000268001 => "VK_THREAD_DONE_KHR",
        1000268002 => "VK_OPERATION_DEFERRED_KHR",
        1000268003 => "VK_OPERATION_NOT_DEFERRED_KHR",
        1000297000 => "VK_PIPELINE_COMPILE_REQUIRED_EXT",
        _ if result < 0 => "VK_ERROR_<Unknown>",
        _ => "VK_<Unknown>",
    }
}

/// The names of the instance extensions. Translation of
/// `SDL_Vulkan_CreateInstanceExtensionsList()`.
fn vulkan_create_instance_extensions_list(
    vk_enumerate_instance_extension_properties: PfnVkEnumerateInstanceExtensionProperties,
) -> Result<Vec<String>> {
    let mut count: u32 = 0;
    // SAFETY: the loader's function, asked for the count only.
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
        return Err(Error::new(format!(
            "Getting Vulkan extensions failed: vkEnumerateInstanceExtensionProperties returned {}({})",
            vulkan_get_result_string(rc),
            rc
        )));
    }

    let empty = VkExtensionProperties {
        extension_name: [0; VK_MAX_EXTENSION_NAME_SIZE],
        spec_version: 0,
    };
    // (at least one entry, so we can return non-null)
    let mut result = vec![empty; count.max(1) as usize];

    // SAFETY: the buffer holds `count` entries.
    let rc = unsafe {
        vk_enumerate_instance_extension_properties(
            std::ptr::null(),
            &mut count,
            result.as_mut_ptr(),
        )
    };
    if rc != VK_SUCCESS {
        return Err(Error::new(format!(
            "Getting Vulkan extensions failed: vkEnumerateInstanceExtensionProperties returned {}({})",
            vulkan_get_result_string(rc),
            rc
        )));
    }
    result.truncate(count as usize);
    Ok(result
        .iter()
        .map(|e| {
            // SAFETY: the name is a NUL-terminated array.
            unsafe { CStr::from_ptr(e.extension_name.as_ptr()) }
                .to_string_lossy()
                .into_owned()
        })
        .collect())
}

/// Translation of `SDL_Vulkan_DestroySurface_Internal()`.
fn vulkan_destroy_surface_internal(
    vk_get_instance_proc_addr: PfnVkGetInstanceProcAddr,
    instance: usize,
    surface: u64,
    allocator: usize,
) {
    // SAFETY: the loader's vkGetInstanceProcAddr; the result has the
    // signature of vkDestroySurfaceKHR.
    unsafe {
        if let Some(f) = vk_get_instance_proc_addr(instance, c"vkDestroySurfaceKHR".as_ptr()) {
            let vk_destroy_surface_khr: PfnVkDestroySurfaceKHR = std::mem::transmute(f);
            vk_destroy_surface_khr(instance, surface, allocator as *const c_void);
        }
    }
}

/// Translation of `WIN_Vulkan_LoadLibrary()`.
pub(crate) fn vulkan_load_library(path: Option<&str>) -> Result<()> {
    let mut config = config();
    if config.is_some() {
        return Err(Error::new("Vulkan already loaded"));
    }

    // Load the Vulkan loader library
    let hint = hints::get(hints::VULKAN_LIBRARY);
    let path = path.or(hint.as_deref()).unwrap_or("vulkan-1.dll");
    let loader_handle = SharedObject::load(path)?;
    // SAFETY: vkGetInstanceProcAddr has this signature.
    let vk_get_instance_proc_addr: PfnVkGetInstanceProcAddr =
        unsafe { loader_handle.function("vkGetInstanceProcAddr")? };
    // SAFETY: a global command, queried with a null instance.
    let vk_enumerate_instance_extension_properties: PfnVkEnumerateInstanceExtensionProperties = unsafe {
        match vk_get_instance_proc_addr(0, c"vkEnumerateInstanceExtensionProperties".as_ptr()) {
            Some(f) => std::mem::transmute::<
                unsafe extern "system" fn(),
                PfnVkEnumerateInstanceExtensionProperties,
            >(f),
            None => {
                return Err(Error::new(
                    "Couldn't load vkEnumerateInstanceExtensionProperties",
                ))
            }
        }
    };
    let extensions =
        vulkan_create_instance_extensions_list(vk_enumerate_instance_extension_properties)?;
    let mut has_surface_extension = false;
    let mut has_win32_surface_extension = false;
    for extension in &extensions {
        if extension == VK_KHR_SURFACE_EXTENSION_NAME {
            has_surface_extension = true;
        } else if extension == VK_KHR_WIN32_SURFACE_EXTENSION_NAME {
            has_win32_surface_extension = true;
        }
    }
    if !has_surface_extension {
        return Err(Error::new(format!(
            "Installed Vulkan doesn't implement the {VK_KHR_SURFACE_EXTENSION_NAME} extension"
        )));
    } else if !has_win32_surface_extension {
        return Err(Error::new(format!(
            "Installed Vulkan doesn't implement the {VK_KHR_WIN32_SURFACE_EXTENSION_NAME} extension"
        )));
    }
    // (on failure above, dropping loader_handle unloads it)
    *config = Some(VulkanConfig {
        loader_handle,
        vk_get_instance_proc_addr,
    });
    Ok(())
}

/// Translation of `WIN_Vulkan_UnloadLibrary()`.
pub(crate) fn vulkan_unload_library() {
    let config = config().take();
    if let Some(config) = config {
        drop(config.loader_handle);
    }
}

/// `_this->vulkan_config.vkGetInstanceProcAddr`.
pub(crate) fn vk_get_instance_proc_addr() -> Option<usize> {
    config()
        .as_ref()
        .map(|c| c.vk_get_instance_proc_addr as usize)
}

/// Translation of `WIN_Vulkan_GetInstanceExtensions()`.
pub(crate) fn vulkan_get_instance_extensions() -> Vec<&'static str> {
    vec![
        VK_KHR_SURFACE_EXTENSION_NAME,
        VK_KHR_WIN32_SURFACE_EXTENSION_NAME,
    ]
}

/// Translation of `WIN_Vulkan_CreateSurface()`.
pub(crate) fn vulkan_create_surface(
    window: WindowID,
    instance: usize,
    allocator: usize,
) -> Result<u64> {
    let window_data = window_data(window).ok_or_else(|| Error::new("Invalid window"))?;
    let Some(get_proc) = config().as_ref().map(|c| c.vk_get_instance_proc_addr) else {
        return Err(Error::new("Vulkan is not loaded"));
    };
    // SAFETY: the loader's vkGetInstanceProcAddr; the result has the
    // signature of vkCreateWin32SurfaceKHR.
    let vk_create_win32_surface_khr: Option<PfnVkCreateWin32SurfaceKHR> = unsafe {
        get_proc(instance, c"vkCreateWin32SurfaceKHR".as_ptr())
            .map(|f| std::mem::transmute::<unsafe extern "system" fn(), _>(f))
    };

    let Some(vk_create_win32_surface_khr) = vk_create_win32_surface_khr else {
        return Err(Error::new(format!(
            "{VK_KHR_WIN32_SURFACE_EXTENSION_NAME} extension is not enabled in the Vulkan instance."
        )));
    };
    let create_info = VkWin32SurfaceCreateInfoKHR {
        s_type: VK_STRUCTURE_TYPE_WIN32_SURFACE_CREATE_INFO_KHR,
        p_next: std::ptr::null(),
        flags: 0,
        hinstance: window_data.hinstance,
        hwnd: window_data.hwnd,
    };
    let mut surface: u64 = 0;
    // SAFETY: the create info and out pointer are valid for the call.
    let result = unsafe {
        vk_create_win32_surface_khr(
            instance,
            &create_info,
            allocator as *const c_void,
            &mut surface,
        )
    };
    if result != VK_SUCCESS {
        return Err(Error::new(format!(
            "vkCreateWin32SurfaceKHR failed: {}",
            vulkan_get_result_string(result)
        )));
    }
    Ok(surface)
}

/// Translation of `WIN_Vulkan_DestroySurface()`.
pub(crate) fn vulkan_destroy_surface(instance: usize, surface: u64, allocator: usize) {
    let get_proc = config().as_ref().map(|c| c.vk_get_instance_proc_addr);
    if let Some(get_proc) = get_proc {
        vulkan_destroy_surface_internal(get_proc, instance, surface, allocator);
    }
}

/// Translation of `WIN_Vulkan_GetPresentationSupport()`.
pub(crate) fn vulkan_get_presentation_support(
    instance: usize,
    physical_device: usize,
    queue_family_index: u32,
) -> bool {
    let Some(get_proc) = config().as_ref().map(|c| c.vk_get_instance_proc_addr) else {
        // ("Vulkan is not loaded")
        return false;
    };
    // SAFETY: the loader's vkGetInstanceProcAddr; the result has this signature.
    let f: Option<PfnVkGetPhysicalDeviceWin32PresentationSupportKHR> = unsafe {
        get_proc(
            instance,
            c"vkGetPhysicalDeviceWin32PresentationSupportKHR".as_ptr(),
        )
        .map(|f| std::mem::transmute::<unsafe extern "system" fn(), _>(f))
    };

    let Some(f) = f else {
        // ("VK_KHR_win32_surface extension is not enabled in the Vulkan instance.")
        return false;
    };

    // SAFETY: a physical device of the instance.
    unsafe { f(physical_device, queue_family_index) != 0 }
}
