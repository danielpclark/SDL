// Rust translation of src/video/wayland/SDL_waylandvulkan.c and
// SDL_waylandvulkan.h from Simple DirectMedia Layer.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! Vulkan surfaces for Wayland windows (`VK_KHR_wayland_surface`). The
//! parts of SDL_vulkan_utils.c are shared with the X11 driver.

use std::ffi::c_void;

use super::video::WaylandVideo;
use crate::error::{Error, Result};
use crate::events::WindowID;
use crate::hints;
use crate::loadso::SharedObject;
use crate::video::drivers::x11::vulkan::{
    instance_function, vulkan_create_instance_extensions_list, vulkan_get_result_string,
    PfnVkDestroySurfaceKHR, PfnVkEnumerateInstanceExtensionProperties, PfnVkGetInstanceProcAddr,
    VkResult, VK_KHR_SURFACE_EXTENSION_NAME, VK_SUCCESS,
};

#[cfg(target_os = "openbsd")]
const DEFAULT_VULKAN: &str = "libvulkan.so";
#[cfg(not(target_os = "openbsd"))]
const DEFAULT_VULKAN: &str = "libvulkan.so.1";

const VK_KHR_WAYLAND_SURFACE_EXTENSION_NAME: &str = "VK_KHR_wayland_surface";

const VK_STRUCTURE_TYPE_WAYLAND_SURFACE_CREATE_INFO_KHR: i32 = 1000006000;

/// `VkWaylandSurfaceCreateInfoKHR`.
#[repr(C)]
struct VkWaylandSurfaceCreateInfoKHR {
    s_type: i32,
    p_next: *const c_void,
    flags: u32,
    display: *mut c_void,
    surface: *mut c_void,
}

type PfnVkCreateWaylandSurfaceKHR = unsafe extern "C" fn(
    *mut c_void,
    *const VkWaylandSurfaceCreateInfoKHR,
    *const c_void,
    *mut u64,
) -> VkResult;
type PfnVkGetPhysicalDeviceWaylandPresentationSupportKHR =
    unsafe extern "C" fn(*mut c_void, u32, *mut c_void) -> u32;

/// The Vulkan loader of the device (`vulkan_config.loader_handle` and its
/// `vkGetInstanceProcAddr`).
#[derive(Default)]
pub(crate) struct VulkanData {
    loader_handle: Option<SharedObject>,
    vk_get_instance_proc_addr: Option<PfnVkGetInstanceProcAddr>,
}

impl VulkanData {
    /// Whether the loader is loaded (`vulkan_config.loader_handle`).
    pub(crate) fn loader_loaded(&self) -> bool {
        self.loader_handle.is_some()
    }
}

impl WaylandVideo {
    /// Translation of `Wayland_Vulkan_LoadLibrary()`.
    pub(crate) fn wayland_vulkan_load_library(&self, path: Option<&str>) -> Result<()> {
        let mut has_surface_extension = false;
        let mut has_wayland_surface_extension = false;
        if self.with_data(|d| d.vulkan.loader_handle.is_some()) {
            return Err(Error::new("Vulkan already loaded"));
        }

        // Load the Vulkan loader library
        let path = path
            .map(str::to_owned)
            .or_else(|| hints::get(hints::VULKAN_LIBRARY))
            .unwrap_or_else(|| DEFAULT_VULKAN.to_owned());
        let loader_handle = SharedObject::load(&path)?;
        // (vulkan_config.loader_path is kept by the video core; on failure,
        // dropping the loader unloads it)
        // SAFETY: vkGetInstanceProcAddr has this signature.
        let vk_get_instance_proc_addr: PfnVkGetInstanceProcAddr =
            unsafe { loader_handle.function::<PfnVkGetInstanceProcAddr>("vkGetInstanceProcAddr")? };
        // SAFETY: the global function has this signature.
        let vk_enumerate_instance_extension_properties: Option<
            PfnVkEnumerateInstanceExtensionProperties,
        > = unsafe {
            instance_function(
                vk_get_instance_proc_addr,
                0,
                "vkEnumerateInstanceExtensionProperties",
            )
        };
        let Some(vk_enumerate_instance_extension_properties) =
            vk_enumerate_instance_extension_properties
        else {
            return Err(Error::new(
                "vkEnumerateInstanceExtensionProperties not found",
            ));
        };
        let extensions =
            vulkan_create_instance_extensions_list(vk_enumerate_instance_extension_properties)?;
        for extension in &extensions {
            if extension == VK_KHR_SURFACE_EXTENSION_NAME {
                has_surface_extension = true;
            } else if extension == VK_KHR_WAYLAND_SURFACE_EXTENSION_NAME {
                has_wayland_surface_extension = true;
            }
        }
        if !has_surface_extension {
            return Err(Error::new(format!(
                "Installed Vulkan doesn't implement the {VK_KHR_SURFACE_EXTENSION_NAME} extension"
            )));
        } else if !has_wayland_surface_extension {
            return Err(Error::new(format!(
                "Installed Vulkan doesn't implement the {VK_KHR_WAYLAND_SURFACE_EXTENSION_NAME} extension"
            )));
        }
        self.with_data(|d| {
            d.vulkan.loader_handle = Some(loader_handle);
            d.vulkan.vk_get_instance_proc_addr = Some(vk_get_instance_proc_addr);
        });
        Ok(())
    }

    /// Translation of `Wayland_Vulkan_UnloadLibrary()`.
    pub(crate) fn wayland_vulkan_unload_library(&self) {
        let loader = self.with_data(|d| {
            d.vulkan.vk_get_instance_proc_addr = None;
            d.vulkan.loader_handle.take()
        });
        // (dropping the library unloads it)
        drop(loader);
    }

    /// The loader's `vkGetInstanceProcAddr` (`vulkan_config.vkGetInstanceProcAddr`).
    pub(crate) fn wayland_vulkan_get_instance_proc_addr(&self) -> Option<usize> {
        self.with_data(|d| d.vulkan.vk_get_instance_proc_addr.map(|f| f as usize))
    }

    /// Translation of `Wayland_Vulkan_GetInstanceExtensions()`.
    pub(crate) fn wayland_vulkan_get_instance_extensions(&self) -> Vec<&'static str> {
        vec![
            VK_KHR_SURFACE_EXTENSION_NAME,
            VK_KHR_WAYLAND_SURFACE_EXTENSION_NAME,
        ]
    }

    /// Translation of `Wayland_Vulkan_CreateSurface()`.
    pub(crate) fn wayland_vulkan_create_surface(
        &self,
        window: WindowID,
        instance: usize,
        allocator: usize,
    ) -> Result<u64> {
        // FIXME (upstream): vkGetInstanceProcAddr is called before checking
        // that the loader is loaded (calling NULL when it isn't); the check
        // comes first here, as calling a missing function can't be expressed.
        let get = self.with_data(|d| d.vulkan.vk_get_instance_proc_addr);
        let Some(vk_get_instance_proc_addr) = get else {
            return Err(Error::new("Vulkan is not loaded"));
        };
        // SAFETY: the function's type.
        let vk_create_wayland_surface_khr: Option<PfnVkCreateWaylandSurfaceKHR> = unsafe {
            instance_function(
                vk_get_instance_proc_addr,
                instance,
                "vkCreateWaylandSurfaceKHR",
            )
        };

        let Some(vk_create_wayland_surface_khr) = vk_create_wayland_surface_khr else {
            return Err(Error::new(format!(
                "{VK_KHR_WAYLAND_SURFACE_EXTENSION_NAME} extension is not enabled in the Vulkan instance."
            )));
        };
        let surface_ptr = self
            .with_data(|d| d.window(window).map(|w| w.surface_id()))
            .ok_or_else(|| Error::new("Invalid window"))?;
        let create_info = VkWaylandSurfaceCreateInfoKHR {
            s_type: VK_STRUCTURE_TYPE_WAYLAND_SURFACE_CREATE_INFO_KHR,
            p_next: std::ptr::null(),
            flags: 0,
            display: self.conn.raw().cast(),
            surface: surface_ptr as *mut c_void,
        };
        let mut surface: u64 = 0;
        // SAFETY: the instance and allocator are the caller's; the display
        // and surface are alive.
        let result = unsafe {
            vk_create_wayland_surface_khr(
                instance as *mut c_void,
                &create_info,
                allocator as *const c_void,
                &mut surface,
            )
        };
        if result != VK_SUCCESS {
            return Err(Error::new(format!(
                "vkCreateWaylandSurfaceKHR failed: {}",
                vulkan_get_result_string(result)
            )));
        }
        Ok(surface)
    }

    /// Translation of `Wayland_Vulkan_DestroySurface()` (and
    /// `SDL_Vulkan_DestroySurface_Internal()`).
    pub(crate) fn wayland_vulkan_destroy_surface(
        &self,
        instance: usize,
        surface: u64,
        allocator: usize,
    ) {
        if let Some(vk_get_instance_proc_addr) =
            self.with_data(|d| d.vulkan.vk_get_instance_proc_addr)
        {
            // SAFETY: the function's type.
            let vk_destroy_surface_khr: Option<PfnVkDestroySurfaceKHR> = unsafe {
                instance_function(vk_get_instance_proc_addr, instance, "vkDestroySurfaceKHR")
            };

            if let Some(vk_destroy_surface_khr) = vk_destroy_surface_khr {
                // SAFETY: the caller's instance, surface and allocator.
                unsafe {
                    vk_destroy_surface_khr(
                        instance as *mut c_void,
                        surface,
                        allocator as *const c_void,
                    )
                };
            }
        }
    }

    /// Translation of `Wayland_Vulkan_GetPresentationSupport()`.
    pub(crate) fn wayland_vulkan_get_presentation_support(
        &self,
        instance: usize,
        physical_device: usize,
        queue_family_index: u32,
    ) -> bool {
        // FIXME (upstream): as in Wayland_Vulkan_CreateSurface(), the loader
        // is checked after calling its vkGetInstanceProcAddr; first here.
        let Some(vk_get_instance_proc_addr) =
            self.with_data(|d| d.vulkan.vk_get_instance_proc_addr)
        else {
            // (SDL_SetError("Vulkan is not loaded"))
            return false;
        };
        // SAFETY: the function's type.
        let f: Option<PfnVkGetPhysicalDeviceWaylandPresentationSupportKHR> = unsafe {
            instance_function(
                vk_get_instance_proc_addr,
                instance,
                "vkGetPhysicalDeviceWaylandPresentationSupportKHR",
            )
        };

        let Some(f) = f else {
            // (SDL_SetError: the Wayland surface extension is not enabled)
            return false;
        };

        // SAFETY: the caller's physical device; the display is connected.
        unsafe {
            f(
                physical_device as *mut c_void,
                queue_family_index,
                self.conn.raw().cast(),
            ) != 0
        }
    }
}
