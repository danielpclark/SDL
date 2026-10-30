// Rust translation of src/video/android/SDL_androidvulkan.c and
// SDL_androidvulkan.h from Simple DirectMedia Layer.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! Vulkan on the android driver's window: Android's Vulkan loader
//! (`libvulkan.so`), loaded at run time, which must have
//! `VK_KHR_surface` and `VK_KHR_android_surface`, and window surfaces on
//! the window's `ANativeWindow` (`vkCreateAndroidSurfaceKHR()`), which
//! must not have an EGL surface: a Vulkan window has none.
//!
//! (Based on Jacob Lifshay's SDL_x11vulkan.c, by Mark Callow,
//! www.edgewise-consulting.com.)

use std::ffi::c_void;
use std::sync::Mutex;

use crate::error::{Error, Result};
use crate::events::WindowID;
use crate::hints;
use crate::loadso::SharedObject;
use crate::video::vulkan_utils::{
    instance_function, vulkan_create_instance_extensions_list, vulkan_destroy_surface_internal,
    vulkan_get_result_string, PfnVkEnumerateInstanceExtensionProperties, PfnVkGetInstanceProcAddr,
    VkResult, VK_KHR_SURFACE_EXTENSION_NAME, VK_SUCCESS,
};

/// `VK_KHR_ANDROID_SURFACE_EXTENSION_NAME`.
const VK_KHR_ANDROID_SURFACE_EXTENSION_NAME: &str = "VK_KHR_android_surface";
/// `VK_STRUCTURE_TYPE_ANDROID_SURFACE_CREATE_INFO_KHR`.
const VK_STRUCTURE_TYPE_ANDROID_SURFACE_CREATE_INFO_KHR: i32 = 1000008000;
/// `VK_ERROR_NATIVE_WINDOW_IN_USE_KHR`.
const VK_ERROR_NATIVE_WINDOW_IN_USE_KHR: VkResult = -1000000001;

/// `VkAndroidSurfaceCreateInfoKHR`.
#[repr(C)]
struct VkAndroidSurfaceCreateInfoKHR {
    s_type: i32,
    p_next: *const c_void,
    flags: u32,
    window: *mut c_void,
}

type PfnVkCreateAndroidSurfaceKHR = unsafe extern "system" fn(
    *mut c_void,
    *const VkAndroidSurfaceCreateInfoKHR,
    *const c_void,
    *mut u64,
) -> VkResult;

/// The loader (`_this->vulkan_config`: `loader_handle`,
/// `vkGetInstanceProcAddr` and `vkEnumerateInstanceExtensionProperties`).
#[derive(Default)]
pub(crate) struct VulkanConfig {
    loader_handle: Option<SharedObject>,
    vk_get_instance_proc_addr: Option<PfnVkGetInstanceProcAddr>,
    vk_enumerate_instance_extension_properties: Option<PfnVkEnumerateInstanceExtensionProperties>,
}

fn lock(config: &Mutex<VulkanConfig>) -> std::sync::MutexGuard<'_, VulkanConfig> {
    config.lock().unwrap_or_else(|e| e.into_inner())
}

/// Translation of `Android_Vulkan_LoadLibrary()`.
pub(crate) fn vulkan_load_library(config: &Mutex<VulkanConfig>, path: Option<&str>) -> Result<()> {
    let mut has_surface_extension = false;
    let mut has_android_surface_extension = false;
    if lock(config).loader_handle.is_some() {
        return Err(Error::new("Vulkan already loaded"));
    }

    // Load the Vulkan loader library
    let hint = match path {
        Some(_) => None,
        None => hints::get(hints::VULKAN_LIBRARY),
    };
    let path = path.or(hint.as_deref()).unwrap_or("libvulkan.so");
    let loader_handle = SharedObject::load(path)?;
    // (the video core keeps vulkan_config.loader_path)
    // SAFETY: vkGetInstanceProcAddr has this signature.
    let vk_get_instance_proc_addr =
        unsafe { loader_handle.function::<PfnVkGetInstanceProcAddr>("vkGetInstanceProcAddr") }?;
    // (fail: dropping the handle unloads the library)

    // SAFETY: the loader's vkGetInstanceProcAddr; the global function has
    // this signature.
    let vk_enumerate_instance_extension_properties: Option<
        PfnVkEnumerateInstanceExtensionProperties,
    > = unsafe {
        instance_function(
            vk_get_instance_proc_addr,
            std::ptr::null_mut(),
            c"vkEnumerateInstanceExtensionProperties",
        )
    };
    let Some(vk_enumerate_instance_extension_properties) =
        vk_enumerate_instance_extension_properties
    else {
        // (upstream fails without setting an error here)
        return Err(Error::new(
            "vkEnumerateInstanceExtensionProperties not found",
        ));
    };
    let extensions =
        vulkan_create_instance_extensions_list(vk_enumerate_instance_extension_properties)?;
    for extension in &extensions {
        let name = extension.name();
        if name == VK_KHR_SURFACE_EXTENSION_NAME {
            has_surface_extension = true;
        } else if name == VK_KHR_ANDROID_SURFACE_EXTENSION_NAME {
            has_android_surface_extension = true;
        }
    }
    if !has_surface_extension {
        return Err(Error::new(format!(
            "Installed Vulkan doesn't implement the {VK_KHR_SURFACE_EXTENSION_NAME} extension"
        )));
    } else if !has_android_surface_extension {
        return Err(Error::new(format!(
            "Installed Vulkan doesn't implement the {VK_KHR_ANDROID_SURFACE_EXTENSION_NAME} extension"
        )));
    }

    let mut config = lock(config);
    config.loader_handle = Some(loader_handle);
    config.vk_get_instance_proc_addr = Some(vk_get_instance_proc_addr);
    config.vk_enumerate_instance_extension_properties =
        Some(vk_enumerate_instance_extension_properties);
    Ok(())
}

/// Translation of `Android_Vulkan_UnloadLibrary()`. (Note (upstream): only
/// the loader handle is cleared there, the function pointers of the
/// unloaded library kept; here they go with it.)
pub(crate) fn vulkan_unload_library(config: &Mutex<VulkanConfig>) {
    let old = std::mem::take(&mut *lock(config));
    // (dropping the handle unloads the library)
    drop(old);
}

/// The loader's `vkGetInstanceProcAddr` (`vulkan_config.vkGetInstanceProcAddr`).
pub(crate) fn vulkan_get_instance_proc_addr(config: &Mutex<VulkanConfig>) -> Option<usize> {
    lock(config).vk_get_instance_proc_addr.map(|f| f as usize)
}

/// Translation of `Android_Vulkan_GetInstanceExtensions()`.
pub(crate) fn vulkan_get_instance_extensions() -> Vec<&'static str> {
    vec![
        VK_KHR_SURFACE_EXTENSION_NAME,
        VK_KHR_ANDROID_SURFACE_EXTENSION_NAME,
    ]
}

/// Translation of `Android_Vulkan_CreateSurface()`.
pub(crate) fn vulkan_create_surface(
    config: &Mutex<VulkanConfig>,
    window: WindowID,
    instance: usize,
    allocator: usize,
) -> Result<u64> {
    let (loaded, get) = {
        let config = lock(config);
        (
            config.loader_handle.is_some(),
            config.vk_get_instance_proc_addr,
        )
    };
    let instance = instance as *mut c_void;
    // (upstream looks the function up before checking that Vulkan is
    // loaded)
    // SAFETY: the loader's vkGetInstanceProcAddr and the caller's
    // instance; the function's type.
    let vk_create_android_surface_khr: Option<PfnVkCreateAndroidSurfaceKHR> = get
        .and_then(|get| unsafe { instance_function(get, instance, c"vkCreateAndroidSurfaceKHR") });

    if !loaded {
        return Err(Error::new("Vulkan is not loaded"));
    }

    let Some(vk_create_android_surface_khr) = vk_create_android_surface_khr else {
        return Err(Error::new(format!(
            "{VK_KHR_ANDROID_SURFACE_EXTENSION_NAME} extension is not enabled in the Vulkan instance."
        )));
    };

    let native_window = super::window::native_window(window);
    if native_window.is_null() {
        // passing a NULL window violates Vulkan spec
        return Err(Error::new(
            "Android native window is not available for surface creation, usually because of backgrounding",
        ));
    }

    let create_info = VkAndroidSurfaceCreateInfoKHR {
        s_type: VK_STRUCTURE_TYPE_ANDROID_SURFACE_CREATE_INFO_KHR,
        p_next: std::ptr::null(),
        flags: 0,
        window: native_window.cast(),
    };
    let mut surface: u64 = 0;
    // SAFETY: the caller's instance and allocator; the create info is
    // valid, its window the window's.
    let result = unsafe {
        vk_create_android_surface_khr(
            instance,
            &create_info,
            allocator as *const c_void,
            &mut surface,
        )
    };
    if result != VK_SUCCESS {
        if result == VK_ERROR_NATIVE_WINDOW_IN_USE_KHR {
            return Err(Error::new(format!(
                "vkCreateAndroidSurfaceKHR failed: {}, was the window created with SDL_WINDOW_VULKAN?",
                vulkan_get_result_string(result)
            )));
        } else {
            return Err(Error::new(format!(
                "vkCreateAndroidSurfaceKHR failed: {}",
                vulkan_get_result_string(result)
            )));
        }
    }
    Ok(surface)
}

/// Translation of `Android_Vulkan_DestroySurface()`.
pub(crate) fn vulkan_destroy_surface(
    config: &Mutex<VulkanConfig>,
    instance: usize,
    surface: u64,
    allocator: usize,
) {
    let get = {
        let config = lock(config);
        config
            .loader_handle
            .as_ref()
            .and(config.vk_get_instance_proc_addr)
    };
    if let Some(vk_get_instance_proc_addr) = get {
        // SAFETY: the caller's instance, surface and allocator.
        unsafe {
            vulkan_destroy_surface_internal(
                vk_get_instance_proc_addr,
                instance as *mut c_void,
                surface,
                allocator as *const c_void,
            )
        };
    }
}
